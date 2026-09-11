//! See docs/DESIGN-NOTES.md.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSource {
    pub node_name: String,
    pub description: String,
    pub is_monitor: bool,
}

pub fn parse_pw_dump(json: &str) -> Vec<AudioSource> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let Some(objects) = value.as_array() else {
        return Vec::new();
    };

    let mut mics = Vec::new();
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
            mics.push(AudioSource {
                node_name: name.to_string(),
                description: description.to_string(),
                is_monitor: false,
            });
        } else if class == "Audio/Sink" {
            monitors.push(AudioSource {
                node_name: format!("{name}.monitor"),
                description: format!("Monitor of {description}"),
                is_monitor: true,
            });
        }
    }

    mics.sort_by(|a, b| a.description.cmp(&b.description));
    monitors.sort_by(|a, b| a.description.cmp(&b.description));
    mics.extend(monitors);
    mics
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
                    description: "Built-in Microphone".into(),
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
        assert_eq!(sources[0].description, "usb_mic");
    }

    #[test]
    fn bad_json_is_empty() {
        assert!(parse_pw_dump("not json").is_empty());
        assert!(parse_pw_dump("{}").is_empty());
    }
}
