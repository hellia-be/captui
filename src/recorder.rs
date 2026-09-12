//! wf-recorder argv and output naming. See docs/DESIGN-NOTES.md.

use crate::sources::Source;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    AudioVideo,
    AudioOnly,
}

pub fn extension(mode: Mode) -> &'static str {
    match mode {
        Mode::AudioVideo => "mkv",
        Mode::AudioOnly => "flac",
    }
}

const VIDEO_CODEC: &str = "libx264";
const VIDEO_CRF: u32 = 18;
const VIDEO_PRESET: &str = "fast";

pub fn wf_recorder_argv(source: &Source, audio: Option<&str>, out: &str) -> Vec<String> {
    let mut argv = vec!["wf-recorder".to_string()];
    argv.extend(source.wf_args());
    argv.push("-c".into());
    argv.push(VIDEO_CODEC.into());
    argv.push("-p".into());
    argv.push(format!("crf={VIDEO_CRF}"));
    argv.push("-p".into());
    argv.push(format!("preset={VIDEO_PRESET}"));
    if let Some(a) = audio {
        argv.push(format!("--audio={a}"));
    }
    argv.push("-f".into());
    argv.push(out.into());
    argv
}

pub fn wl_screenrec_argv(
    source: &Source,
    audio: Option<&str>,
    out: &str,
    no_hw: bool,
) -> Vec<String> {
    let mut argv = vec!["wl-screenrec".to_string()];
    argv.extend(source.wf_args());
    if no_hw {
        argv.push("--no-hw".into());
    }
    if let Some(a) = audio {
        argv.push("--audio".into());
        argv.push("--audio-device".into());
        argv.push(a.into());
    }
    argv.push("-f".into());
    argv.push(out.into());
    argv
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    WfRecorder,
    WlScreenrec,
}

impl Backend {
    pub fn from_config(name: Option<&str>) -> Backend {
        match name {
            Some("wl-screenrec") => Backend::WlScreenrec,
            _ => Backend::WfRecorder,
        }
    }

    pub fn argv(self, source: &Source, audio: Option<&str>, out: &str, no_hw: bool) -> Vec<String> {
        match self {
            Backend::WfRecorder => wf_recorder_argv(source, audio, out),
            Backend::WlScreenrec => wl_screenrec_argv(source, audio, out, no_hw),
        }
    }
}

pub fn transcribe_argv(template: &str, file: &str) -> Option<Vec<String>> {
    let mut argv: Vec<String> = template.split_whitespace().map(String::from).collect();
    if argv.is_empty() {
        return None;
    }
    let mut replaced = false;
    for arg in &mut argv {
        if arg.contains("{}") {
            *arg = arg.replace("{}", file);
            replaced = true;
        }
    }
    if !replaced {
        argv.push(file.to_string());
    }
    Some(argv)
}

pub fn timestamped_name(unix_secs: u64, ext: &str) -> String {
    let (y, m, d) = civil_from_days((unix_secs / 86400) as i64);
    let s = unix_secs % 86400;
    format!(
        "captui-{y:04}{m:02}{d:02}-{:02}{:02}{:02}.{ext}",
        s / 3600,
        (s % 3600) / 60,
        s % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + i64::from(m <= 2), m as u32, d)
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
    fn builds_argv_with_audio() {
        let s = Source::Display("HDMI-A-1".into());
        let argv = wf_recorder_argv(&s, Some("alsa_output.monitor"), "/tmp/cap.mkv");
        assert_eq!(
            argv,
            vec![
                "wf-recorder",
                "-o",
                "HDMI-A-1",
                "-c",
                "libx264",
                "-p",
                "crf=18",
                "-p",
                "preset=fast",
                "--audio=alsa_output.monitor",
                "-f",
                "/tmp/cap.mkv"
            ]
        );
    }

    #[test]
    fn builds_argv_without_audio() {
        let s = Source::Region("0,0 640x480".into());
        let argv = wf_recorder_argv(&s, None, "/tmp/cap.mkv");
        assert_eq!(
            argv,
            vec![
                "wf-recorder",
                "-g",
                "0,0 640x480",
                "-c",
                "libx264",
                "-p",
                "crf=18",
                "-p",
                "preset=fast",
                "-f",
                "/tmp/cap.mkv"
            ]
        );
    }

    #[test]
    fn wl_screenrec_argv_uses_audio_device_and_no_hw() {
        let s = Source::Display("DP-1".into());
        assert_eq!(
            wl_screenrec_argv(&s, Some("mon"), "/tmp/c.mkv", false),
            vec![
                "wl-screenrec",
                "-o",
                "DP-1",
                "--audio",
                "--audio-device",
                "mon",
                "-f",
                "/tmp/c.mkv"
            ]
        );
        assert_eq!(
            wl_screenrec_argv(&Source::Region("0,0 8x8".into()), None, "/tmp/c.mkv", true),
            vec![
                "wl-screenrec",
                "-g",
                "0,0 8x8",
                "--no-hw",
                "-f",
                "/tmp/c.mkv"
            ]
        );
    }

    #[test]
    fn backend_from_config_and_dispatch() {
        assert_eq!(
            Backend::from_config(Some("wl-screenrec")),
            Backend::WlScreenrec
        );
        assert_eq!(
            Backend::from_config(Some("wf-recorder")),
            Backend::WfRecorder
        );
        assert_eq!(Backend::from_config(None), Backend::WfRecorder);
        assert_eq!(Backend::from_config(Some("nonsense")), Backend::WfRecorder);
        let s = Source::Display("DP-1".into());
        assert_eq!(
            Backend::WlScreenrec.argv(&s, None, "/o.mkv", false)[0],
            "wl-screenrec"
        );
        assert_eq!(
            Backend::WfRecorder.argv(&s, None, "/o.mkv", false)[0],
            "wf-recorder"
        );
    }

    #[test]
    fn transcribe_argv_substitutes_or_appends() {
        assert_eq!(
            transcribe_argv("transcribe-remote", "/a.mkv"),
            Some(vec!["transcribe-remote".into(), "/a.mkv".into()])
        );
        assert_eq!(
            transcribe_argv("t {} --fast", "/a.mkv"),
            Some(vec!["t".into(), "/a.mkv".into(), "--fast".into()])
        );
        assert_eq!(transcribe_argv("   ", "/a.mkv"), None);
    }

    #[test]
    fn timestamps_are_utc_civil() {
        assert_eq!(timestamped_name(0, "mkv"), "captui-19700101-000000.mkv");
        assert_eq!(timestamped_name(86400, "mkv"), "captui-19700102-000000.mkv");
        assert_eq!(
            timestamped_name(1_700_000_000, "flac"),
            "captui-20231114-221320.flac"
        );
    }
}
