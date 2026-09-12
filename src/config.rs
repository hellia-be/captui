//! See docs/DESIGN-NOTES.md.

use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Config {
    pub output_dir: Option<String>,
    pub container: Option<String>,
    pub audio_output: Option<String>,
    pub audio_input: Option<String>,
    pub transcribe_command: Option<String>,
    pub backend: Option<String>,
    pub no_hw: bool,
}

pub fn parse_config(text: &str) -> Config {
    toml::from_str(text).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fields() {
        let cfg = parse_config(
            r#"
            output_dir = "~/rec"
            container = "mp4"
            audio_output = "alsa_output.speakers.monitor"
            audio_input = "alsa_input.mic"
            transcribe_command = "transcribe-remote {}"
            backend = "wl-screenrec"
            no_hw = true
        "#,
        );
        assert_eq!(cfg.output_dir.as_deref(), Some("~/rec"));
        assert_eq!(cfg.container.as_deref(), Some("mp4"));
        assert_eq!(
            cfg.audio_output.as_deref(),
            Some("alsa_output.speakers.monitor")
        );
        assert_eq!(cfg.audio_input.as_deref(), Some("alsa_input.mic"));
        assert_eq!(
            cfg.transcribe_command.as_deref(),
            Some("transcribe-remote {}")
        );
        assert_eq!(cfg.backend.as_deref(), Some("wl-screenrec"));
        assert!(cfg.no_hw);
    }

    #[test]
    fn missing_fields_are_none() {
        let cfg = parse_config("container = \"mkv\"");
        assert_eq!(cfg.container.as_deref(), Some("mkv"));
        assert!(cfg.output_dir.is_none());
        assert!(cfg.audio_input.is_none());
    }

    #[test]
    fn empty_and_bad_are_default() {
        assert_eq!(parse_config(""), Config::default());
        assert_eq!(
            parse_config("this is not = valid = toml"),
            Config::default()
        );
        assert_eq!(parse_config("unknown_key = 1"), Config::default());
    }
}
