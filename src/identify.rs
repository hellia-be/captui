// See docs/DESIGN-NOTES.md.

use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use captui::font::{digit_pixel, GLYPH_H, GLYPH_W};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_output, wl_shm, wl_surface::WlSurface,
};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};

const PANEL_W: u32 = 360;
const PANEL_H: u32 = 280;
const BG: [u8; 4] = [0x18, 0x14, 0x10, 0xFF];
const FG: [u8; 4] = [0xEE, 0xEE, 0xEE, 0xFF];

pub fn flash(numbers: &HashMap<String, u32>, duration: Duration) -> Result<usize> {
    let conn = Connection::connect_to_env().map_err(|e| anyhow!("connect to Wayland: {e}"))?;
    let (globals, mut event_queue) =
        registry_queue_init(&conn).map_err(|e| anyhow!("Wayland registry init: {e}"))?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)
        .map_err(|e| anyhow!("wl_compositor unavailable: {e}"))?;
    let layer_shell = LayerShell::bind(&globals, &qh)
        .map_err(|e| anyhow!("wlr-layer-shell unavailable (need a wlroots compositor): {e}"))?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| anyhow!("wl_shm unavailable: {e}"))?;

    let mut state = Identify {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        compositor,
        layer_shell,
        numbers: numbers.clone(),
        surfaces: Vec::new(),
    };

    event_queue.roundtrip(&mut state)?;
    state.create_surfaces(&qh)?;
    let shown = state.surfaces.len();
    if shown == 0 {
        return Ok(0);
    }

    let start = Instant::now();
    for _ in 0..20 {
        event_queue.roundtrip(&mut state)?;
        conn.flush()?;
        if state.surfaces.iter().all(|s| s.drawn) {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    conn.flush()?;
    if let Some(remaining) = duration.checked_sub(start.elapsed()) {
        thread::sleep(remaining);
    }

    state.surfaces.clear();
    let _ = conn.flush();
    let _ = event_queue.roundtrip(&mut state);
    Ok(shown)
}

struct Surface {
    layer: LayerSurface,
    pool: SlotPool,
    number: u32,
    width: u32,
    height: u32,
    drawn: bool,
}

struct Identify {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    compositor: CompositorState,
    layer_shell: LayerShell,
    numbers: HashMap<String, u32>,
    surfaces: Vec<Surface>,
}

impl Identify {
    fn create_surfaces(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        for output in self.output_state.outputs() {
            let Some(name) = self.output_state.info(&output).and_then(|i| i.name) else {
                continue;
            };
            let Some(&number) = self.numbers.get(&name) else {
                continue;
            };
            let surface = self.compositor.create_surface(qh);
            let layer = self.layer_shell.create_layer_surface(
                qh,
                surface,
                Layer::Overlay,
                Some("captui-identify"),
                Some(&output),
            );
            layer.set_size(PANEL_W, PANEL_H);
            layer.set_anchor(Anchor::empty());
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();
            let pool = SlotPool::new((PANEL_W * PANEL_H * 4) as usize, &self.shm)?;
            self.surfaces.push(Surface {
                layer,
                pool,
                number,
                width: PANEL_W,
                height: PANEL_H,
                drawn: false,
            });
        }
        Ok(())
    }

    fn draw(&mut self, idx: usize) {
        let s = &mut self.surfaces[idx];
        let (w, h) = (s.width, s.height);
        let stride = w as i32 * 4;
        let Ok((buffer, canvas)) =
            s.pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        paint(canvas, w, h, s.number);
        let surface = s.layer.wl_surface();
        surface.attach(Some(buffer.wl_buffer()), 0, 0);
        surface.damage_buffer(0, 0, w as i32, h as i32);
        s.layer.commit();
        s.drawn = true;
    }

    fn index_of(&self, surface: &WlSurface) -> Option<usize> {
        self.surfaces
            .iter()
            .position(|s| s.layer.wl_surface() == surface)
    }
}

fn paint(canvas: &mut [u8], w: u32, h: u32, number: u32) {
    let mut i = 0;
    while i + 4 <= canvas.len() {
        canvas[i..i + 4].copy_from_slice(&BG);
        i += 4;
    }
    let digits: Vec<u8> = number.to_string().bytes().map(|b| b - b'0').collect();
    let n = digits.len() as u32;
    if n == 0 {
        return;
    }
    let gap = 1;
    let cells_w = n * GLYPH_W + (n - 1) * gap;
    let scale = ((w * 3 / 4) / cells_w).min((h * 3 / 4) / GLYPH_H).max(1);
    let glyph_w = cells_w * scale;
    let glyph_h = GLYPH_H * scale;
    let x0 = w.saturating_sub(glyph_w) / 2;
    let y0 = h.saturating_sub(glyph_h) / 2;

    for (di, d) in digits.iter().enumerate() {
        let dx = x0 + di as u32 * (GLYPH_W + gap) * scale;
        for row in 0..GLYPH_H {
            for col in 0..GLYPH_W {
                if !digit_pixel(*d, col, row) {
                    continue;
                }
                for sy in 0..scale {
                    for sx in 0..scale {
                        put(canvas, w, h, dx + col * scale + sx, y0 + row * scale + sy);
                    }
                }
            }
        }
    }
}

fn put(canvas: &mut [u8], w: u32, h: u32, x: u32, y: u32) {
    if x >= w || y >= h {
        return;
    }
    let idx = ((y * w + x) * 4) as usize;
    if let Some(px) = canvas.get_mut(idx..idx + 4) {
        px.copy_from_slice(&FG);
    }
}

impl CompositorHandler for Identify {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Identify {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for Identify {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {}
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        if let Some(idx) = self.index_of(layer.wl_surface()) {
            let (cw, ch) = configure.new_size;
            if cw != 0 && ch != 0 {
                self.surfaces[idx].width = cw;
                self.surfaces[idx].height = ch;
            }
            self.draw(idx);
        }
    }
}

impl ShmHandler for Identify {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Identify {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_registry!(Identify);
smithay_client_toolkit::delegate_dispatch2!(Identify);
