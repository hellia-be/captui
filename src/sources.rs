//! Source selection and wlr-randr parsing. See docs/DESIGN-NOTES.md.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Display(String),
    Region(String),
}

impl Source {
    pub fn wf_args(&self) -> Vec<String> {
        match self {
            Source::Display(o) => vec!["-o".into(), o.clone()],
            Source::Region(g) => vec!["-g".into(), g.clone()],
        }
    }
}

pub fn region(x: i32, y: i32, w: u32, h: u32) -> String {
    format!("{x},{y} {w}x{h}")
}

pub fn parse_geometry(s: &str) -> Option<(i32, i32, u32, u32)> {
    let (pos, size) = s.trim().split_once(' ')?;
    let (x, y) = pos.split_once(',')?;
    let (w, h) = size.split_once('x')?;
    let x = x.trim().parse().ok()?;
    let y = y.trim().parse().ok()?;
    let w: u32 = w.trim().parse().ok()?;
    let h: u32 = h.trim().parse().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((x, y, w, h))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: f64,
}

impl Mode {
    pub fn label(&self) -> String {
        format!(
            "{}x{}@{}",
            self.width,
            self.height,
            self.refresh_hz.round() as i64
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub position: Option<(i32, i32)>,
    pub mode: Option<Mode>,
    pub scale: f64,
}

pub fn parse_wlr_randr(s: &str) -> Vec<Output> {
    let mut outputs: Vec<Output> = Vec::new();
    for line in s.lines() {
        if line.is_empty() {
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            let name = line
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_string();
            let description = match (line.find('"'), line.rfind('"')) {
                (Some(start), Some(end)) if end > start => line[start + 1..end].to_string(),
                _ => String::new(),
            };
            outputs.push(Output {
                name,
                description,
                enabled: false,
                position: None,
                mode: None,
                scale: 1.0,
            });
            continue;
        }
        let t = line.trim_start();
        let Some(last) = outputs.last_mut() else {
            continue;
        };
        if let Some(rest) = t.strip_prefix("Enabled:") {
            last.enabled = rest.trim().eq_ignore_ascii_case("yes");
        } else if let Some(rest) = t.strip_prefix("Position:") {
            if let Some((x, y)) = rest.trim().split_once(',') {
                if let (Ok(x), Ok(y)) = (x.trim().parse(), y.trim().parse()) {
                    last.position = Some((x, y));
                }
            }
        } else if let Some(rest) = t.strip_prefix("Scale:") {
            if let Ok(scale) = rest.trim().parse::<f64>() {
                if scale > 0.0 {
                    last.scale = scale;
                }
            }
        } else if t.contains("current") {
            if let Some(m) = parse_mode_line(t) {
                last.mode = Some(m);
            }
        }
    }
    outputs
}

fn parse_mode_line(t: &str) -> Option<Mode> {
    let toks: Vec<&str> = t.split_whitespace().collect();
    let (w, h) = toks.first()?.split_once('x')?;
    let width = w.parse().ok()?;
    let height = h.parse().ok()?;
    let hz_idx = toks.iter().position(|s| *s == "Hz")?;
    let refresh_hz = toks.get(hz_idx.checked_sub(1)?)?.parse().ok()?;
    Some(Mode {
        width,
        height,
        refresh_hz,
    })
}

pub fn sort_reading_order(outputs: &mut [Output]) {
    outputs.sort_by(|a, b| {
        let ka = a.position.map(|(x, y)| (y, x));
        let kb = b.position.map(|(x, y)| (y, x));
        match (ka, kb) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
}

pub fn layout_hints(outputs: &[Output]) -> Vec<String> {
    let xs: Vec<i32> = outputs
        .iter()
        .filter_map(|o| o.position.map(|p| p.0))
        .collect();
    let ys: Vec<i32> = outputs
        .iter()
        .filter_map(|o| o.position.map(|p| p.1))
        .collect();
    let x_lo = xs.iter().min().copied();
    let x_hi = xs.iter().max().copied();
    let y_lo = ys.iter().min().copied();
    let y_hi = ys.iter().max().copied();

    outputs
        .iter()
        .map(|o| {
            let Some((x, y)) = o.position else {
                return String::new();
            };
            let horiz = axis_word(x, x_lo, x_hi, "left", "center", "right");
            let vert = axis_word(y, y_lo, y_hi, "top", "middle", "bottom");
            match (vert, horiz) {
                (Some(v), Some(h)) => format!("{v}-{h}"),
                (Some(v), None) => v.to_string(),
                (None, Some(h)) => h.to_string(),
                (None, None) => String::new(),
            }
        })
        .collect()
}

fn axis_word(
    v: i32,
    lo: Option<i32>,
    hi: Option<i32>,
    low: &'static str,
    mid: &'static str,
    high: &'static str,
) -> Option<&'static str> {
    let (lo, hi) = (lo?, hi?);
    if lo == hi {
        return None;
    }
    if v == lo {
        Some(low)
    } else if v == hi {
        Some(high)
    } else {
        Some(mid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_args() {
        assert_eq!(Source::Display("DP-1".into()).wf_args(), vec!["-o", "DP-1"]);
    }

    #[test]
    fn region_args() {
        let s = Source::Region(region(0, 0, 1920, 1080));
        assert_eq!(s.wf_args(), vec!["-g", "0,0 1920x1080"]);
    }

    #[test]
    fn parses_slurp_geometry() {
        assert_eq!(
            parse_geometry("100,200 640x480\n"),
            Some((100, 200, 640, 480))
        );
        assert_eq!(
            parse_geometry("-5,-10 1920x1080"),
            Some((-5, -10, 1920, 1080))
        );
    }

    #[test]
    fn rejects_bad_geometry() {
        assert_eq!(parse_geometry(""), None);
        assert_eq!(parse_geometry("100,200"), None);
        assert_eq!(parse_geometry("100,200 640x0"), None);
        assert_eq!(parse_geometry("a,b cxd"), None);
    }

    #[test]
    fn parses_outputs_with_details() {
        let sample = "\
DP-1 \"Dell Inc. DELL U2415 7MT018 (DP-1)\"
  Make: Dell Inc.
  Enabled: yes
  Modes:
    1920x1200 px, 59.950001 Hz (preferred, current)
    1920x1080 px, 60.000000 Hz
  Position: 0,0
  Scale: 1.000000
HDMI-A-1 \"Samsung S22C300 (HDMI-A-1)\"
  Enabled: no
";
        let outputs = parse_wlr_randr(sample);
        assert_eq!(
            outputs,
            vec![
                Output {
                    name: "DP-1".into(),
                    description: "Dell Inc. DELL U2415 7MT018 (DP-1)".into(),
                    enabled: true,
                    position: Some((0, 0)),
                    mode: Some(Mode {
                        width: 1920,
                        height: 1200,
                        refresh_hz: 59.950001,
                    }),
                    scale: 1.0,
                },
                Output {
                    name: "HDMI-A-1".into(),
                    description: "Samsung S22C300 (HDMI-A-1)".into(),
                    enabled: false,
                    position: None,
                    mode: None,
                    scale: 1.0,
                },
            ]
        );
    }

    #[test]
    fn mode_label_rounds_refresh() {
        let m = Mode {
            width: 1920,
            height: 1200,
            refresh_hz: 59.950001,
        };
        assert_eq!(m.label(), "1920x1200@60");
    }

    #[test]
    fn header_without_description_still_parses() {
        let outputs = parse_wlr_randr("eDP-1\n  Enabled: yes\n");
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].name, "eDP-1");
        assert!(outputs[0].description.is_empty());
        assert!(outputs[0].enabled);
    }

    #[test]
    fn empty_input_is_no_outputs() {
        assert!(parse_wlr_randr("").is_empty());
    }

    fn at(x: i32, y: i32) -> Output {
        Output {
            name: "x".into(),
            description: String::new(),
            enabled: true,
            position: Some((x, y)),
            mode: None,
            scale: 1.0,
        }
    }

    #[test]
    fn parses_scale() {
        let outputs = parse_wlr_randr("eDP-1\n  Enabled: yes\n  Scale: 1.500000\n");
        assert_eq!(outputs[0].scale, 1.5);
    }

    #[test]
    fn horizontal_row_gets_left_center_right() {
        let outputs = vec![at(0, 0), at(1920, 0), at(3840, 0)];
        assert_eq!(layout_hints(&outputs), vec!["left", "center", "right"]);
    }

    #[test]
    fn vertical_stack_gets_top_bottom_only() {
        let outputs = vec![at(0, 0), at(0, 1080)];
        assert_eq!(layout_hints(&outputs), vec!["top", "bottom"]);
    }

    #[test]
    fn sorts_top_to_bottom_then_left_to_right() {
        let mut outputs = vec![at(1920, 0), at(3840, 0), at(0, 0), at(0, 1080)];
        sort_reading_order(&mut outputs);
        let positions: Vec<_> = outputs.iter().map(|o| o.position.unwrap()).collect();
        assert_eq!(positions, vec![(0, 0), (1920, 0), (3840, 0), (0, 1080)]);
    }

    #[test]
    fn sorts_unknown_position_last_stably() {
        let mut a = at(1920, 0);
        let mut b = at(0, 0);
        a.position = None;
        b.position = None;
        a.name = "first".into();
        b.name = "second".into();
        let mut outputs = vec![a, at(0, 0), b];
        sort_reading_order(&mut outputs);
        assert_eq!(outputs[0].position, Some((0, 0)));
        assert_eq!(outputs[1].name, "first");
        assert_eq!(outputs[2].name, "second");
    }

    #[test]
    fn grid_combines_axes() {
        let outputs = vec![at(0, 0), at(1920, 1080)];
        assert_eq!(layout_hints(&outputs), vec!["top-left", "bottom-right"]);
    }

    #[test]
    fn single_output_has_no_hint() {
        assert_eq!(layout_hints(&[at(0, 0)]), vec![String::new()]);
    }

    #[test]
    fn unknown_position_has_no_hint() {
        let mut o = at(0, 0);
        o.position = None;
        assert_eq!(layout_hints(&[o]), vec![String::new()]);
    }
}
