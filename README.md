# captui

Terminal UI to record the screen, a window, or a region with sound on wlroots
Wayland, with a live status view (audio meter, elapsed time, file size) and
one-key start/stop. Recordings feed a Whisper transcription flow. Rust +
ratatui, packaged as a Nix flake.

Status: working recorder (source/region picker, output+input audio with mixing,
live meters, timer/size, video and audio-only modes). See docs/CHANGELOG.md.

## What it does

- One picker with three panes — Display, Audio, Mic — TAB (or ←/→) to switch
  panes, ↑/↓ to select, Enter to record. The Display pane offers each display,
  Region (drag-select via slurp), Audio only (no video), and a Window entry when
  `window_geometry_command` is configured. The Audio pane lists the outputs
  (System audio and each monitor) plus any app currently playing sound
  ("App: <name>", captured non-destructively via pw-link); Mic lists your inputs.
- Record with sound via wf-recorder (or wl-screenrec for hardware encode). Mix
  system audio and a mic together.
- Live recording view: elapsed timer, growing file size, per-source level meters,
  per-source volume, and pause/resume (`p`). No live video (that needs a GUI).
- Two modes: screen + audio (mkv), or audio-only (flac) for a lean transcript.
- Stop cleanly with SIGINT so the file is finalized, then optionally hand the
  recording to a Whisper transcription step (`t`).

## Documentation

- docs/DESIGN-NOTES.md - why each subsystem is built the way it is
- docs/CONTRIBUTING.md - workflow, style, and the CI gate
- docs/CHANGELOG.md - what has shipped
- CLAUDE.md - agent-facing brief (conventions summary + pointers)

## Requirements

A wlroots Wayland compositor (Umbriel, Niri, LabWC, Sway, ...) and PipeWire.
Runtime tools: wf-recorder (or wl-screenrec), slurp, wlr-randr, ffmpeg, the
PipeWire CLIs (pw-dump, pw-link, wpctl), and pactl/parec (pulseaudio) for the
audio mix and level meters. The packaged binary and the dev shell provide them.

## Configuration

Optional `~/.config/captui/config.toml`, all keys optional:

    output_dir = "~/Videos/captures"   # where recordings are written
    container = "mkv"                   # A/V file extension (audio-only is flac)
    audio_output = "alsa_output.pci-0000_01_00.1.hdmi-stereo.monitor"
    audio_input = "alsa_input.usb-Razer_..."
    transcribe_command = "transcribe-remote {}"   # {} = recording path
    backend = "wf-recorder"             # or "wl-screenrec" for hardware (VAAPI) encode
    no_hw = false                       # wl-screenrec only: true forces software encode
    window_geometry_command = "..."     # picks a window, prints it as "X,Y WxH"

`audio_output` / `audio_input` are PipeWire node names (as shown by
`pactl list short sources`) to preselect in the pickers. `transcribe_command`, if
set, adds a `t` action on the stopped recording screen that runs the command on
the file (`{}` is the path, else it is appended).

`window_geometry_command`, if set, adds a "Window" entry to the Display pane.
captui runs the command with `sh -c` and expects it to print one line on stdout,
the target window's geometry as `X,Y WxH` (the same format slurp emits); captui
then records that rectangle. This keeps window capture compositor-agnostic: point
it at your compositor's own window query, run through a small wrapper if needed.

Do not use a "focused window" query: captui runs in a terminal, so the focused
window is captui itself, and you would record captui. Instead pick the target
window at record time. The portable recipe pipes every window's rectangle into
`slurp`, which lets you click the one to record (it reads
`"<x>,<y> <width>x<height> [label]"` lines from stdin and prints the chosen box):

    # Sway (wrap in a script on your PATH, then set the command to its name):
    swaymsg -t get_tree \
      | jq -r '.. | objects | select(.pid and .visible)
               | "\(.rect.x),\(.rect.y) \(.rect.width)x\(.rect.height) \(.name)"' \
      | slurp

    window_geometry_command = "captui-pick-window"   # the wrapper above, on PATH

The exact query differs per compositor (`swaymsg -t get_tree`, `hyprctl clients
-j`, `niri msg --json windows`, ...) and the JSON field names are version
dependent, so verify your command at a shell first — it must print a single
`X,Y WxH` line. Capture is a fixed region taken once at start, so it does not
follow the window if it moves. A missing or malformed file
falls back to built-in defaults.

The `wl-screenrec` backend needs a working VAAPI encoder (AMD/Intel). On NVIDIA
its hardware capture-format negotiation fails, so use the default `wf-recorder`
(software libx264) or try `no_hw = true`.

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
