//! Capture source selection: turn a chosen source into wf-recorder selection
//! arguments. Pure and IO-free so it is unit-testable in CI. Rationale in
//! docs/DESIGN-NOTES.md.

/// What to capture. A window is captured as a fixed Region derived from the
/// compositor's reported geometry (wlroots screencopy has no per-surface grab).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A whole output/display by connector name, e.g. "DP-1".
    Display(String),
    /// A rectangular region "X,Y WxH" (from slurp or a window geometry).
    Region(String),
}

impl Source {
    /// wf-recorder selection args for this source.
    pub fn wf_args(&self) -> Vec<String> {
        match self {
            Source::Display(o) => vec!["-o".into(), o.clone()],
            Source::Region(g) => vec!["-g".into(), g.clone()],
        }
    }
}

/// Format a region the way slurp and wf-recorder expect: "X,Y WxH".
pub fn region(x: i32, y: i32, w: u32, h: u32) -> String {
    format!("{x},{y} {w}x{h}")
}

/// A display enumerated from wlr-randr: its connector name (what wf-recorder's
/// `-o` wants) plus the human description and whether it is currently on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Connector name, e.g. "DP-1".
    pub name: String,
    /// Quoted description line, e.g. "Dell Inc. DELL U2415 ... (DP-1)".
    pub description: String,
    /// A disabled output has no framebuffer and cannot be recorded.
    pub enabled: bool,
}

/// Parse `wlr-randr` plain-text output into a list of outputs. An output header
/// starts at column 0 as `NAME "DESCRIPTION"`; indented lines are its
/// properties, of which we only need `Enabled: yes|no`. Pure so CI (no Wayland)
/// can test it. See docs/DESIGN-NOTES.md.
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
            });
        } else if let Some(rest) = line.trim_start().strip_prefix("Enabled:") {
            if let Some(last) = outputs.last_mut() {
                last.enabled = rest.trim().eq_ignore_ascii_case("yes");
            }
        }
    }
    outputs
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
    fn parses_outputs_with_enabled_flag() {
        let sample = "\
DP-1 \"Dell Inc. DELL U2415 7MT018 (DP-1)\"
  Make: Dell Inc.
  Enabled: yes
  Modes:
    1920x1200 px, 59.950001 Hz (preferred, current)
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
                },
                Output {
                    name: "HDMI-A-1".into(),
                    description: "Samsung S22C300 (HDMI-A-1)".into(),
                    enabled: false,
                },
            ]
        );
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
}
