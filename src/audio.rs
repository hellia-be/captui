//! See docs/DESIGN-NOTES.md.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSource {
    pub node_name: String,
    pub description: String,
    pub is_monitor: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioTarget {
    Silent,
    Single(String),
    Mix { output: String, input: String },
}

pub fn audio_target(output: Option<&str>, input: Option<&str>) -> AudioTarget {
    match (output, input) {
        (None, None) => AudioTarget::Silent,
        (Some(o), None) => AudioTarget::Single(o.to_string()),
        (None, Some(i)) => AudioTarget::Single(i.to_string()),
        (Some(o), Some(i)) => AudioTarget::Mix {
            output: o.to_string(),
            input: i.to_string(),
        },
    }
}

pub fn parse_pw_dump(json: &str) -> Vec<AudioSource> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(objects) = value.as_array() else {
        return Vec::new();
    };

    let default_sink = default_meta(objects, "default.audio.sink");
    let default_source = default_meta(objects, "default.audio.source");
    // Mics are collected as (is_default, description, node_name) so the default
    // one can sort first and be marked, while every mic stays visible by name.
    let mut mics: Vec<(bool, String, String)> = Vec::new();
    let mut monitors = Vec::new();
    for obj in objects {
        let props = &obj["info"]["props"];
        let Some(name) = props["node.name"].as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let description = props["node.description"]
            .as_str()
            .or_else(|| props["node.nick"].as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(name);
        let class = props["media.class"].as_str().unwrap_or("");
        if class.starts_with("Audio/Source") {
            let is_default = default_source.as_deref() == Some(name);
            mics.push((is_default, description.to_string(), name.to_string()));
        } else if class == "Audio/Sink" && default_sink.as_deref() != Some(name) {
            monitors.push(AudioSource {
                node_name: format!("{name}.monitor"),
                description: format!("Monitor of {description}"),
                is_monitor: true,
            });
        }
    }

    mics.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    monitors.sort_by(|a, b| a.description.cmp(&b.description));

    let mut sources = Vec::new();
    if let Some(sink) = default_sink {
        sources.push(AudioSource {
            node_name: format!("{sink}.monitor"),
            description: "System audio (all)".to_string(),
            is_monitor: true,
        });
    }
    sources.extend(
        mics.into_iter()
            .map(|(is_default, desc, node_name)| AudioSource {
                node_name,
                description: if is_default {
                    format!("Mic: {desc} (default)")
                } else {
                    format!("Mic: {desc}")
                },
                is_monitor: false,
            }),
    );
    sources.extend(monitors);
    sources
}

pub fn parse_sink_input_index(text: &str, module_id: &str) -> Option<u32> {
    let mut current = None;
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("Sink Input #") {
            current = rest.trim().parse().ok();
        } else if let Some(rest) = t.strip_prefix("Owner Module:") {
            if rest.trim() == module_id {
                return current;
            }
        }
    }
    None
}

fn default_meta(objects: &[serde_json::Value], key: &str) -> Option<String> {
    for obj in objects {
        let Some(entries) = obj["metadata"].as_array() else {
            continue;
        };
        for entry in entries {
            if entry["key"] == key {
                if let Some(name) = entry["value"]["name"].as_str() {
                    return Some(name.to_string());
                }
                if let Some(name) = entry["value"].as_str() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      { "type": "PipeWire:Interface:Node",
        "info": { "props": {
          "media.class": "Audio/Source",
          "node.name": "alsa_input.pci-0000_0c_00.4.analog-stereo",
          "node.description": "Built-in Microphone" } } },
      { "type": "PipeWire:Interface:Node",
        "info": { "props": {
          "media.class": "Audio/Sink",
          "node.name": "alsa_output.pci-0000_0c_00.4.analog-stereo",
          "node.description": "Speakers" } } },
      { "type": "PipeWire:Interface:Node",
        "info": { "props": {
          "media.class": "Video/Source",
          "node.name": "v4l2_cam" } } }
    ]"#;

    #[test]
    fn lists_mics_then_monitors() {
        let sources = parse_pw_dump(SAMPLE);
        assert_eq!(
            sources,
            vec![
                AudioSource {
                    node_name: "alsa_input.pci-0000_0c_00.4.analog-stereo".into(),
                    description: "Mic: Built-in Microphone".into(),
                    is_monitor: false,
                },
                AudioSource {
                    node_name: "alsa_output.pci-0000_0c_00.4.analog-stereo.monitor".into(),
                    description: "Monitor of Speakers".into(),
                    is_monitor: true,
                },
            ]
        );
    }

    #[test]
    fn ignores_non_audio_and_falls_back_to_node_name() {
        let json = r#"[
          { "info": { "props": { "media.class": "Audio/Source",
            "node.name": "usb_mic" } } } ]"#;
        let sources = parse_pw_dump(json);
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].description, "Mic: usb_mic");
    }

    #[test]
    fn default_mic_is_named_marked_and_first() {
        let json = r#"[
          { "metadata": [
              { "key": "default.audio.source", "value": { "name": "mic_a" } } ] },
          { "info": { "props": {
            "media.class": "Audio/Source", "node.name": "mic_a",
            "node.description": "Headset" } } },
          { "info": { "props": {
            "media.class": "Audio/Source", "node.name": "mic_b",
            "node.description": "Webcam" } } }
        ]"#;
        let sources = parse_pw_dump(json);
        assert_eq!(
            sources,
            vec![
                AudioSource {
                    node_name: "mic_a".into(),
                    description: "Mic: Headset (default)".into(),
                    is_monitor: false,
                },
                AudioSource {
                    node_name: "mic_b".into(),
                    description: "Mic: Webcam".into(),
                    is_monitor: false,
                },
            ]
        );
    }

    #[test]
    fn bad_json_is_empty() {
        assert!(parse_pw_dump("not json").is_empty());
        assert!(parse_pw_dump("{}").is_empty());
    }

    #[test]
    fn finds_sink_input_by_owner_module() {
        let text = "\
Sink Input #10
\tDriver: PipeWire
\tOwner Module: 40
Sink Input #11
\tDriver: PipeWire
\tOwner Module: 41
";
        assert_eq!(parse_sink_input_index(text, "41"), Some(11));
        assert_eq!(parse_sink_input_index(text, "40"), Some(10));
        assert_eq!(parse_sink_input_index(text, "99"), None);
    }

    #[test]
    fn audio_target_combines_output_and_input() {
        assert_eq!(audio_target(None, None), AudioTarget::Silent);
        assert_eq!(
            audio_target(Some("mon"), None),
            AudioTarget::Single("mon".into())
        );
        assert_eq!(
            audio_target(None, Some("mic")),
            AudioTarget::Single("mic".into())
        );
        assert_eq!(
            audio_target(Some("mon"), Some("mic")),
            AudioTarget::Mix {
                output: "mon".into(),
                input: "mic".into()
            }
        );
    }

    #[test]
    fn default_sink_becomes_system_audio_all_without_duplicate() {
        let json = r#"[
          { "type": "PipeWire:Interface:Metadata",
            "props": { "metadata.name": "default" },
            "metadata": [
              { "key": "default.audio.sink", "value": { "name": "alsa_output.speakers" } }
            ] },
          { "info": { "props": {
            "media.class": "Audio/Sink",
            "node.name": "alsa_output.speakers",
            "node.description": "Speakers" } } },
          { "info": { "props": {
            "media.class": "Audio/Sink",
            "node.name": "hdmi.tv",
            "node.description": "TV" } } },
          { "info": { "props": {
            "media.class": "Audio/Source",
            "node.name": "mic",
            "node.description": "Mic" } } }
        ]"#;
        let sources = parse_pw_dump(json);
        assert_eq!(
            sources,
            vec![
                AudioSource {
                    node_name: "alsa_output.speakers.monitor".into(),
                    description: "System audio (all)".into(),
                    is_monitor: true,
                },
                AudioSource {
                    node_name: "mic".into(),
                    description: "Mic: Mic".into(),
                    is_monitor: false,
                },
                AudioSource {
                    node_name: "hdmi.tv.monitor".into(),
                    description: "Monitor of TV".into(),
                    is_monitor: true,
                },
            ]
        );
    }
}
