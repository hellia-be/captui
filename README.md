# captui

Terminal UI to record the screen, a window, or a region with sound on wlroots
Wayland, with a live status view (audio meter, elapsed time, file size) and
one-key start/stop. Recordings feed a Whisper transcription flow. Rust +
ratatui, packaged as a Nix flake.

Status: working recorder (source/region picker, output+input audio with mixing,
live meters, timer/size, video and audio-only modes). See docs/ROADMAP.md.

## What it does

- Pick a source: a full display, a drag-selected region (slurp), or a window
  (captured as a region from the compositor's geometry).
- Record with sound via wf-recorder, choosing a PipeWire audio source.
- Live status: elapsed timer, growing file size, and an audio level meter so you
  can see sound is being captured. No live video (that would need a GUI).
- Two modes: screen + audio (mkv), or audio-only (flac) for a lean transcript.
- Stop cleanly with SIGINT so the file is finalized, then optionally hand the
  recording to a Whisper transcription step.

## Documentation

- docs/DESIGN-NOTES.md - why each subsystem is built the way it is
- docs/ROADMAP.md - priority-ordered backlog
- docs/CONTRIBUTING.md - workflow, style, and the CI gate
- docs/CHANGELOG.md - what has shipped
- CLAUDE.md - agent-facing brief (conventions summary + pointers)

## Requirements

A wlroots Wayland compositor (Umbriel, Niri, LabWC, Sway, ...) and PipeWire.
Runtime tools: wf-recorder (or wl-screenrec), slurp, wlr-randr, the PipeWire CLIs
(pw-record, pw-dump, wpctl), and pactl/parec (pulseaudio) for the audio mix and
level meters. The packaged binary and the dev shell provide them.

## Configuration

Optional `~/.config/captui/config.toml`, all keys optional:

    output_dir = "~/Videos/captures"   # where recordings are written
    container = "mkv"                   # A/V file extension (audio-only is flac)
    audio_output = "alsa_output.pci-0000_01_00.1.hdmi-stereo.monitor"
    audio_input = "alsa_input.usb-Razer_..."

`audio_output` / `audio_input` are PipeWire node names (as shown by
`pactl list short sources`) to preselect in the pickers. A missing or malformed
file falls back to built-in defaults.

## Getting started

Run without installing (needs Nix with flakes):

    nix run github:hellia-be/captui

Development shell (Rust toolchain + the runtime tools above):

    nix develop
    cargo run

## Development and tests

CI is the gate: rustfmt, clippy (warnings are errors), cargo-audit, cargo-deny,
gitleaks, cargo test, and a Nix build. Run the cheap checks locally before
pushing (`cargo fmt`, `cargo clippy`, `cargo test`). Tests are pure functions
only; anything needing a live Wayland session or PipeWire is local-only.
Contributions go via a feature branch and a PR that must be green before a
squash-merge.

This project is developed with the assistance of Claude Code (Anthropic). We are
transparent about that: commits Claude co-authors carry a
`Co-Authored-By: Claude <noreply@anthropic.com>` trailer, and its pull requests
carry a "Generated with Claude Code" footer.

## License

GPL-3.0-or-later. See LICENSE. Any distributed fork or derivative must also be
GPL and publish its source.
