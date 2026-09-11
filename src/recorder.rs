//! Recorder command construction and output naming. Pure/IO-free for CI; the
//! actual process spawn and SIGINT stop live in the (IO) app layer. wf-recorder
//! must be stopped with SIGINT so it finalizes the container.

use crate::sources::Source;

/// Capture mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Screen + audio to a container (default).
    AudioVideo,
    /// Audio only: the lean path when only a Whisper transcript is wanted.
    AudioOnly,
}

/// Default output file extension for a mode.
pub fn extension(mode: Mode) -> &'static str {
    match mode {
        Mode::AudioVideo => "mkv",
        Mode::AudioOnly => "flac",
    }
}

/// Build the wf-recorder argv for an A/V capture of `source`, recording the
/// PipeWire node `audio_src` and writing to `out`.
pub fn wf_recorder_argv(source: &Source, audio_src: &str, out: &str) -> Vec<String> {
    let mut argv = vec!["wf-recorder".to_string()];
    argv.extend(source.wf_args());
    argv.push("-a".into());
    argv.push(audio_src.into());
    argv.push("-f".into());
    argv.push(out.into());
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions() {
        assert_eq!(extension(Mode::AudioVideo), "mkv");
        assert_eq!(extension(Mode::AudioOnly), "flac");
    }

    #[test]
    fn builds_wf_recorder_argv() {
        let s = Source::Display("HDMI-A-1".into());
        let argv = wf_recorder_argv(&s, "alsa_output.monitor", "/tmp/cap.mkv");
        assert_eq!(
            argv,
            vec![
                "wf-recorder",
                "-o",
                "HDMI-A-1",
                "-a",
                "alsa_output.monitor",
                "-f",
                "/tmp/cap.mkv"
            ]
        );
    }
}
