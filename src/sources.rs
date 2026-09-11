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
}
