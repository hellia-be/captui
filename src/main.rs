// captui - terminal UI to record the screen, a window, or a region with sound.
// Copyright (C) 2026  Kevin Andriessens
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. See the LICENSE file for details.

use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use directories::UserDirs;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph};

use captui::audio::{
    audio_target, parse_pw_dump, parse_sink_input_index, AudioSource, AudioTarget,
};
use captui::format::{format_duration, format_size};
use captui::meter::{meter_bar, samples_peak};
use captui::recorder::{extension, timestamped_name, wf_recorder_argv, Mode};
use captui::sources::{
    layout_hints, parse_geometry, parse_wlr_randr, region, sort_reading_order, Output, Source,
};

#[cfg(feature = "identify")]
mod identify;

fn enumerate_displays() -> Result<Vec<Output>> {
    let out = Command::new("wlr-randr").output().context(
        "could not run wlr-randr (is it installed and are you on a wlroots Wayland session?)",
    )?;
    if !out.status.success() {
        bail!(
            "wlr-randr failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let outputs = parse_wlr_randr(&String::from_utf8_lossy(&out.stdout));
    let mut displays: Vec<Output> = outputs.into_iter().filter(|o| o.enabled).collect();
    sort_reading_order(&mut displays);
    Ok(displays)
}

fn enumerate_audio() -> Result<Vec<AudioSource>> {
    let out = Command::new("pw-dump")
        .output()
        .context("could not run pw-dump (is PipeWire installed?)")?;
    if !out.status.success() {
        bail!(
            "pw-dump failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(parse_pw_dump(&String::from_utf8_lossy(&out.stdout)))
}

fn output_path(mode: Mode) -> Result<PathBuf> {
    let base = UserDirs::new()
        .and_then(|u| u.video_dir().map(Path::to_path_buf))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Videos")))
        .ok_or_else(|| anyhow!("could not determine a video directory"))?;
    let dir = base.join("captures");
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    Ok(dir.join(timestamped_name(secs, extension(mode))))
}

fn spawn_audio_recorder(node: &str, out: &Path) -> Result<Child> {
    Command::new("pw-record")
        .arg(format!("--target={node}"))
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not spawn pw-record")
}

fn spawn_recorder(source: &Source, audio: Option<&str>, out: &Path) -> Result<Child> {
    let argv = wf_recorder_argv(source, audio, &out.to_string_lossy());
    Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("could not spawn {} (is it installed?)", argv[0]))
}

fn stop_recorder(child: &mut Child) -> Result<()> {
    let _ = signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGINT);
    child
        .wait()
        .context("waiting for wf-recorder to finalize")?;
    Ok(())
}

const MIX_SINK: &str = "captui_mix";

struct AudioMix {
    modules: Vec<String>,
}

impl Drop for AudioMix {
    fn drop(&mut self) {
        for id in self.modules.iter().rev() {
            let _ = Command::new("pactl")
                .args(["unload-module", id])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

fn pactl_load(args: &[&str]) -> Result<String> {
    let out = Command::new("pactl")
        .arg("load-module")
        .args(args)
        .output()
        .context("could not run pactl (is it installed?)")?;
    if !out.status.success() {
        bail!(
            "pactl load-module {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

struct MixSetup {
    monitor: String,
    mix: AudioMix,
    out_sink_input: Option<u32>,
    in_sink_input: Option<u32>,
}

fn load_loopback(source: &str, mix: &mut AudioMix) -> Result<String> {
    let id = pactl_load(&[
        "module-loopback",
        &format!("source={source}"),
        &format!("sink={MIX_SINK}"),
        "latency_msec=20",
    ])?;
    mix.modules.push(id.clone());
    Ok(id)
}

fn setup_mix(output: &str, input: &str) -> Result<MixSetup> {
    let sink = pactl_load(&[
        "module-null-sink",
        &format!("sink_name={MIX_SINK}"),
        "sink_properties=device.description=captui-mix",
    ])?;
    let mut mix = AudioMix {
        modules: vec![sink],
    };
    let out_module = load_loopback(output, &mut mix)?;
    let in_module = load_loopback(input, &mut mix)?;

    let sink_inputs = Command::new("pactl")
        .args(["list", "sink-inputs"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    let idx = |module: &str| {
        sink_inputs
            .as_deref()
            .and_then(|t| parse_sink_input_index(t, module))
    };

    Ok(MixSetup {
        monitor: format!("{MIX_SINK}.monitor"),
        out_sink_input: idx(&out_module),
        in_sink_input: idx(&in_module),
        mix,
    })
}

fn set_sink_input_volume(idx: u32, pct: u16) {
    let _ = Command::new("pactl")
        .args([
            "set-sink-input-volume",
            &idx.to_string(),
            &format!("{pct}%"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

struct Meter {
    child: Child,
    level: Arc<AtomicU32>,
    handle: Option<JoinHandle<()>>,
}

impl Meter {
    fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
}

impl Drop for Meter {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn spawn_meter(node: &str) -> Option<Meter> {
    let mut child = Command::new("parec")
        .args([
            "--device",
            node,
            "--format=float32le",
            "--rate=48000",
            "--channels=1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let level = Arc::new(AtomicU32::new(0));
    let shared = level.clone();
    let handle = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut display = 0.0f32;
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 {
                break;
            }
            display *= 0.8;
            display = display.max(samples_peak(&buf[..n]));
            shared.store(display.to_bits(), Ordering::Relaxed);
        }
    });
    Some(Meter {
        child,
        level,
        handle: Some(handle),
    })
}

struct SourceControl {
    label: &'static str,
    meter: Meter,
    sink_input: Option<u32>,
    volume: u16,
}

struct Rec {
    child: Child,
    path: PathBuf,
    stopped: bool,
    started: Instant,
    final_elapsed: Option<Duration>,
    sources: Vec<SourceControl>,
    focus: usize,
    mix: Option<AudioMix>,
}

impl Rec {
    fn elapsed(&self) -> Duration {
        self.final_elapsed.unwrap_or_else(|| self.started.elapsed())
    }

    fn size_bytes(&self) -> u64 {
        std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0)
    }
}

enum Screen {
    Source,
    AudioOutput,
    AudioInput,
    Recording,
}

struct App {
    displays: Vec<Output>,
    source_list: ListState,
    pending_source: Option<Source>,
    output_options: Vec<Option<AudioSource>>,
    output_list: ListState,
    input_options: Vec<Option<AudioSource>>,
    input_list: ListState,
    chosen_output: Option<String>,
    audio_only: bool,
    screen: Screen,
    recording: Option<Rec>,
    status: Option<String>,
}

impl App {
    fn new(displays: Vec<Output>) -> Self {
        let mut source_list = ListState::default();
        if !displays.is_empty() {
            source_list.select(Some(0));
        }
        Self {
            displays,
            source_list,
            pending_source: None,
            output_options: Vec::new(),
            output_list: ListState::default(),
            input_options: Vec::new(),
            input_list: ListState::default(),
            chosen_output: None,
            audio_only: false,
            screen: Screen::Source,
            recording: None,
            status: None,
        }
    }

    fn active_list(&mut self) -> (&mut ListState, usize) {
        match self.screen {
            Screen::AudioOutput => (&mut self.output_list, self.output_options.len()),
            Screen::AudioInput => (&mut self.input_list, self.input_options.len()),
            _ => (&mut self.source_list, self.displays.len()),
        }
    }

    fn move_by(&mut self, delta: isize) {
        let (list, len) = self.active_list();
        if len == 0 {
            return;
        }
        let cur = list.selected().unwrap_or(0) as isize;
        list.select(Some((cur + delta).rem_euclid(len as isize) as usize));
    }

    fn identify(&mut self) {
        self.status = Some(run_identify(&self.displays));
    }

    fn choose_display(&mut self) {
        if let Some(o) = self
            .source_list
            .selected()
            .and_then(|i| self.displays.get(i))
        {
            self.audio_only = false;
            self.pending_source = Some(Source::Display(o.name.clone()));
            self.enter_output();
        }
    }

    fn choose_region(&mut self, src: Source) {
        self.audio_only = false;
        self.pending_source = Some(src);
        self.enter_output();
    }

    fn choose_audio_only(&mut self) {
        self.audio_only = true;
        self.pending_source = None;
        self.enter_output();
    }

    fn enter_output(&mut self) {
        self.status = None;
        let sources = match enumerate_audio() {
            Ok(s) => s,
            Err(e) => {
                self.status = Some(format!("{e:#}"));
                Vec::new()
            }
        };
        let (monitors, mics): (Vec<_>, Vec<_>) = sources.into_iter().partition(|a| a.is_monitor);
        self.output_options = monitors
            .into_iter()
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
        self.input_options = mics
            .into_iter()
            .map(Some)
            .chain(std::iter::once(None))
            .collect();
        // Default to system audio, and to the default mic if there is one.
        self.output_list.select(Some(0));
        let default_mic = self.input_options.iter().position(|o| {
            o.as_ref()
                .is_some_and(|a| a.description.ends_with("(default)"))
        });
        self.input_list.select(Some(
            default_mic.unwrap_or(self.input_options.len().saturating_sub(1)),
        ));
        self.screen = Screen::AudioOutput;
    }

    fn confirm_output(&mut self) {
        self.chosen_output = self
            .output_list
            .selected()
            .and_then(|i| self.output_options.get(i))
            .and_then(|c| c.as_ref().map(|a| a.node_name.clone()));
        self.screen = Screen::AudioInput;
    }

    fn back_to_source(&mut self) {
        self.pending_source = None;
        self.status = None;
        self.screen = Screen::Source;
    }

    fn back_to_output(&mut self) {
        self.status = None;
        self.screen = Screen::AudioOutput;
    }

    fn start_recording(&mut self) {
        let input = self
            .input_list
            .selected()
            .and_then(|i| self.input_options.get(i))
            .and_then(|c| c.as_ref().map(|a| a.node_name.clone()));
        let output = self.chosen_output.clone();
        let target = audio_target(output.as_deref(), input.as_deref());

        if self.audio_only && target == AudioTarget::Silent {
            self.status = Some("audio-only needs an output or an input".into());
            return;
        }

        let (audio_node, mix, out_input, in_input) = match target {
            AudioTarget::Silent => (None, None, None, None),
            AudioTarget::Single(node) => (Some(node), None, None, None),
            AudioTarget::Mix { output, input } => match setup_mix(&output, &input) {
                Ok(s) => (
                    Some(s.monitor),
                    Some(s.mix),
                    s.out_sink_input,
                    s.in_sink_input,
                ),
                Err(e) => {
                    self.status = Some(format!("{e:#}"));
                    return;
                }
            },
        };

        let mode = if self.audio_only {
            Mode::AudioOnly
        } else {
            Mode::AudioVideo
        };
        let path = match output_path(mode) {
            Ok(p) => p,
            Err(e) => {
                self.status = Some(format!("{e:#}"));
                return;
            }
        };

        let spawned = if self.audio_only {
            match audio_node.as_deref() {
                Some(node) => spawn_audio_recorder(node, &path),
                None => return,
            }
        } else {
            match self.pending_source.as_ref() {
                Some(source) => spawn_recorder(source, audio_node.as_deref(), &path),
                None => return,
            }
        };

        match spawned {
            Ok(child) => {
                let mut sources = Vec::new();
                if let Some(meter) = output.as_deref().and_then(spawn_meter) {
                    sources.push(SourceControl {
                        label: "output",
                        meter,
                        sink_input: out_input,
                        volume: 100,
                    });
                }
                if let Some(meter) = input.as_deref().and_then(spawn_meter) {
                    sources.push(SourceControl {
                        label: "input",
                        meter,
                        sink_input: in_input,
                        volume: 100,
                    });
                }
                self.pending_source = None;
                self.recording = Some(Rec {
                    child,
                    path,
                    stopped: false,
                    started: Instant::now(),
                    final_elapsed: None,
                    sources,
                    focus: 0,
                    mix,
                });
                self.status = None;
                self.screen = Screen::Recording;
            }
            Err(e) => {
                self.status = Some(format!("{e:#}"));
            }
        }
    }

    fn focus_source(&mut self, delta: isize) {
        if let Some(rec) = self.recording.as_mut() {
            if !rec.sources.is_empty() {
                let len = rec.sources.len() as isize;
                rec.focus = (rec.focus as isize + delta).rem_euclid(len) as usize;
            }
        }
    }

    fn adjust_volume(&mut self, delta: i16) {
        if let Some(rec) = self.recording.as_mut() {
            if let Some(src) = rec.sources.get_mut(rec.focus) {
                if let Some(idx) = src.sink_input {
                    src.volume = (src.volume as i16 + delta).clamp(0, 150) as u16;
                    set_sink_input_volume(idx, src.volume);
                }
            }
        }
    }

    fn stop(&mut self) {
        let Some(rec) = self.recording.as_mut() else {
            return;
        };
        if rec.stopped {
            return;
        }
        rec.final_elapsed = Some(rec.started.elapsed());
        rec.sources.clear();
        let msg = match stop_recorder(&mut rec.child) {
            Ok(()) => {
                rec.stopped = true;
                format!("saved: {}", rec.path.display())
            }
            Err(e) => format!("stop failed: {e:#}"),
        };
        rec.mix = None;
        self.status = Some(msg);
    }
}

fn main() -> Result<()> {
    let displays = enumerate_displays();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let res = run(&mut terminal, displays);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Some(rec) = res?.recording {
        println!("recording saved: {}", rec.path.display());
    }
    Ok(())
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    displays: Result<Vec<Output>>,
) -> Result<App> {
    let (mut app, error) = match displays {
        Ok(d) => (App::new(d), None),
        Err(e) => (App::new(Vec::new()), Some(format!("{e:#}"))),
    };

    loop {
        terminal.draw(|f| draw(f, &mut app, error.as_deref()))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind != KeyEventKind::Press {
            continue;
        }
        match app.screen {
            Screen::Source => match k.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(app),
                KeyCode::Down | KeyCode::Char('j') => app.move_by(1),
                KeyCode::Up | KeyCode::Char('k') => app.move_by(-1),
                KeyCode::Char('i') => app.identify(),
                KeyCode::Char('r') => match run_slurp() {
                    Ok(src) => app.choose_region(src),
                    Err(e) => app.status = Some(format!("{e:#}")),
                },
                KeyCode::Char('a') => app.choose_audio_only(),
                KeyCode::Enter => app.choose_display(),
                _ => {}
            },
            Screen::AudioOutput => match k.code {
                KeyCode::Char('q') => return Ok(app),
                KeyCode::Esc => app.back_to_source(),
                KeyCode::Down | KeyCode::Char('j') => app.move_by(1),
                KeyCode::Up | KeyCode::Char('k') => app.move_by(-1),
                KeyCode::Enter => app.confirm_output(),
                _ => {}
            },
            Screen::AudioInput => match k.code {
                KeyCode::Char('q') => return Ok(app),
                KeyCode::Esc => app.back_to_output(),
                KeyCode::Down | KeyCode::Char('j') => app.move_by(1),
                KeyCode::Up | KeyCode::Char('k') => app.move_by(-1),
                KeyCode::Enter => app.start_recording(),
                _ => {}
            },
            Screen::Recording => match k.code {
                KeyCode::Char('s') => app.stop(),
                KeyCode::Up | KeyCode::Char('k') => app.focus_source(-1),
                KeyCode::Down | KeyCode::Char('j') => app.focus_source(1),
                KeyCode::Left | KeyCode::Char('h') => app.adjust_volume(-5),
                KeyCode::Right | KeyCode::Char('l') => app.adjust_volume(5),
                KeyCode::Char('q') | KeyCode::Esc => {
                    app.stop();
                    return Ok(app);
                }
                _ => {}
            },
        }
    }
}

fn run_slurp() -> Result<Source> {
    let out = Command::new("slurp")
        .output()
        .context("could not run slurp (is it installed?)")?;
    if !out.status.success() {
        bail!("region selection cancelled");
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (x, y, w, h) = parse_geometry(&text)
        .ok_or_else(|| anyhow!("unexpected slurp output: {:?}", text.trim()))?;
    Ok(Source::Region(region(x, y, w, h)))
}

#[cfg(feature = "identify")]
fn run_identify(displays: &[Output]) -> String {
    use std::collections::HashMap;

    let numbers: HashMap<String, u32> = displays
        .iter()
        .enumerate()
        .map(|(i, o)| (o.name.clone(), i as u32 + 1))
        .collect();
    match identify::flash(&numbers, Duration::from_millis(1600)) {
        Ok(0) => "identify: no matching outputs".into(),
        Ok(n) => format!("flashed a number on {n} screen(s)"),
        Err(e) => format!("identify failed: {e:#}"),
    }
}

#[cfg(not(feature = "identify"))]
fn run_identify(_displays: &[Output]) -> String {
    "identify overlay not built in this binary".into()
}

fn row_label(n: usize, o: &Output, hint: &str) -> String {
    let mode = o.mode.map(|m| m.label()).unwrap_or_else(|| "?".into());
    let pos = match o.position {
        Some((x, y)) => format!("@{x},{y}"),
        None => String::new(),
    };
    format!("{n:>2}. {:<9} {:<13} {:<12} {hint}", o.name, mode, pos)
        .trim_end()
        .to_string()
}

fn audio_label(choice: &Option<AudioSource>) -> String {
    match choice {
        None => "None".into(),
        Some(a) => a.description.clone(),
    }
}

fn draw(f: &mut Frame, app: &mut App, error: Option<&str>) {
    let chunks = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(f.area());
    match app.screen {
        Screen::Source => draw_source(f, app, error, chunks[0]),
        Screen::AudioOutput => draw_audio_list(
            f,
            " captui - output (system audio) ",
            &app.output_options,
            &mut app.output_list,
            chunks[0],
        ),
        Screen::AudioInput => draw_audio_list(
            f,
            " captui - input (microphone) ",
            &app.input_options,
            &mut app.input_list,
            chunks[0],
        ),
        Screen::Recording => draw_recording(f, app, chunks[0]),
    }
    draw_footer(f, app, chunks[1]);
}

fn draw_source(f: &mut Frame, app: &mut App, error: Option<&str>, area: Rect) {
    let block = Block::default()
        .title(" captui - select a source ")
        .borders(Borders::ALL);
    if let Some(err) = error {
        f.render_widget(
            Paragraph::new(err).block(block).style(Style::new().red()),
            area,
        );
    } else if app.displays.is_empty() {
        f.render_widget(
            Paragraph::new("no enabled displays found.").block(block),
            area,
        );
    } else {
        let hints = layout_hints(&app.displays);
        let items: Vec<ListItem> = app
            .displays
            .iter()
            .zip(hints)
            .enumerate()
            .map(|(i, (o, hint))| ListItem::new(row_label(i + 1, o, &hint)))
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_symbol("> ")
            .highlight_spacing(HighlightSpacing::Always)
            .highlight_style(Style::new().reversed());
        f.render_stateful_widget(list, area, &mut app.source_list);
    }
}

fn draw_audio_list(
    f: &mut Frame,
    title: &str,
    options: &[Option<AudioSource>],
    state: &mut ListState,
    area: Rect,
) {
    let block = Block::default().title(title).borders(Borders::ALL);
    let items: Vec<ListItem> = options
        .iter()
        .map(|c| ListItem::new(audio_label(c)))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_symbol("> ")
        .highlight_spacing(HighlightSpacing::Always)
        .highlight_style(Style::new().reversed());
    f.render_stateful_widget(list, area, state);
}

fn draw_recording(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" captui - recording ")
        .borders(Borders::ALL);
    let body = match &app.recording {
        Some(rec) => {
            let timer = format_duration(rec.elapsed().as_secs());
            let size = format_size(rec.size_bytes());
            let tag = if app.audio_only { " (audio)" } else { "" };
            let head = if rec.stopped {
                Line::from(vec![format!("■ stopped{tag}  ").green(), timer.into()])
            } else {
                Line::from(vec![format!("● REC{tag}  ").red().bold(), timer.into()])
            };
            let mut lines = vec![head, Line::from(format!("size: {size}"))];
            let adjustable = !rec.stopped && rec.sources.iter().any(|s| s.sink_input.is_some());
            for (i, src) in rec.sources.iter().enumerate() {
                let marker = if adjustable && i == rec.focus {
                    ">"
                } else {
                    " "
                };
                let vol = match src.sink_input {
                    Some(_) if !rec.stopped => format!("  {:>3}%", src.volume),
                    _ => String::new(),
                };
                lines.push(Line::from(format!(
                    "{marker} {:>6}: {}{vol}",
                    src.label,
                    meter_bar(src.meter.level(), 24)
                )));
            }
            lines.push(Line::from(format!("file: {}", rec.path.display())));
            Text::from(lines)
        }
        None => Text::from("not recording"),
    };
    f.render_widget(Paragraph::new(body).block(block), area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let stopped = matches!(&app.recording, Some(r) if r.stopped);
    let adjustable = matches!(&app.recording, Some(r) if !r.stopped && r.sources.iter().any(|s| s.sink_input.is_some()));
    let hint = match app.screen {
        Screen::Source => {
            " up/down move  i identify  enter display  r region  a audio-only  q quit "
        }
        Screen::AudioOutput => " up/down move  enter next (input)  esc back  q quit ",
        Screen::AudioInput => " up/down move  enter record  esc back  q quit ",
        Screen::Recording if stopped => " q quit ",
        Screen::Recording if adjustable => {
            " s stop  up/down focus  left/right volume  q stop and quit "
        }
        Screen::Recording => " s stop  q stop and quit ",
    };
    let footer = match &app.status {
        Some(s) => Paragraph::new(format!(" {s} ")).style(Style::new().yellow()),
        None => Paragraph::new(hint).style(Style::new().dim()),
    };
    f.render_widget(footer, area);
}
