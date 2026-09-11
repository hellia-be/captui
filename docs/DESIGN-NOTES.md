# Design notes

Per-subsystem rationale. Keep each fact in one place; code carries short
pointers back here rather than long comments.

## Sources (src/sources.rs)

wlroots screencopy captures outputs and regions, not surfaces. So captui offers
a display (`wf-recorder -o <output>`) or a region (`-g "X,Y WxH"`, from slurp).
A window is captured as a fixed region derived from the compositor's reported
geometry (Umbriel/Niri IPC); it does not follow the window if it moves. Multiple
displays into one file is not native to wf-recorder (one output per instance).

A region source comes from `slurp`: pressing `r` in the picker spawns slurp for
an interactive drag-select and captures its `X,Y WxH` on stdout, validated by the
pure `parse_geometry` (rejects malformed output and zero-area rectangles) into a
`Source::Region`. Spawning slurp is IO in the app layer; slurp draws its overlay
through the compositor, so the TUI stays up underneath. Region selection works
even when no displays enumerated.

Displays are enumerated by parsing `wlr-randr`'s plain-text output
(`parse_wlr_randr`): an output header sits at column 0 as `NAME "DESCRIPTION"`,
and its indented properties follow. We read `Enabled: yes|no`, `Position: X,Y`,
and the active mode from the modes line marked as current. That marker is the
word `current` inside a parenthetical like `(preferred, current)`, so the match
is on `current`, not the substring `(current)`. The parse is pure so CI can test
it; the picker filters to enabled outputs, since a disabled output has no
framebuffer to capture. Running wlr-randr is IO and lives in the app layer
(src/main.rs), not the CI-tested lib.

Identical monitors (same make/model) share a description, so the connector name
is the only differentiator and it does not say which physical screen is which.
The picker therefore shows each output's current mode, layout position, and a
directional hint (`layout_hints`) derived from how the outputs sit relative to
each other: per axis, the min coordinate is left/top, the max is right/bottom,
anything between is center/middle, and an axis all outputs share contributes no
word. wlr-randr enumerates in connector order, which is spatially arbitrary, so
enabled displays are sorted into reading order (top to bottom, then left to
right, by position; unknown-position outputs last, stably) before numbering.
Rows are numbered so the identify overlay can flash the matching number on each
screen.

## Identify overlay (src/identify.rs, src/font.rs)

Pressing `i` in the picker flashes each display's row number on its physical
screen for ~1.6s, the reliable way to tell identical monitors apart. This is a
Wayland layer-shell client (smithay-client-toolkit): one `Overlay` layer surface
per output, centered, drawn into an shm buffer, matched to the picker rows by the
output's connector name. It is a departure from the otherwise CLI-driven design,
justified because no CLI can draw on top of a specific output.

It is gated behind the non-default `identify` Cargo feature so the CI test and
lint jobs (which have no Wayland) build pure Rust only; the nix package, the real
downstream gate, builds `--features identify`. smithay-client-toolkit is taken
with `default-features = false` to drop its xkbcommon and calloop requirements
(we use neither), which also means the pure-Rust wayland backend and thus no
libwayland or pkg-config at build or run time. The Wayland code is a binary-only
module, never in the CI-tested lib; only the digit bitmap (`font.rs`) is pure and
unit-tested. Local runs use `cargo run --features identify`.

## Audio sources (src/audio.rs)

After a source is chosen, the picker offers an audio source. They are enumerated
from `pw-dump`'s JSON (parsed with serde_json in the pure `parse_pw_dump`, so CI
can test it) rather than by scraping `wpctl status`'s tree. Each `Audio/Source`
node is a real input (a mic); each `Audio/Sink` becomes a "Monitor of <sink>"
option whose value is the sink's node name plus `.monitor`, which is how the
PulseAudio-compatible layer names a sink's monitor for capturing system audio.
The value carried forward is the node name that `wf-recorder -a` wants; node
names are runtime state, never hardcoded. Mics are listed before monitors, each
alphabetical, with a leading "No audio (silent)" option. Running pw-dump is IO in
the app layer.

## Recorder (src/recorder.rs)

Pure argv builders and output naming, kept IO-free so they are testable in CI.
The A/V mode records screen + a PipeWire source to mkv; the audio-only mode
writes flac for a lean Whisper transcript. The process spawn and stop live in
the IO app layer: wf-recorder must be stopped with SIGINT (not a hard kill) so
it finalizes the container.

## Audio metering (planned)

The "is sound coming in" confirmation is a live level meter read from a PipeWire
CLI stream (pw-mon / pw-dump); link libpipewire only if parsing proves too thin.
It meters the audio source enumerated by src/audio.rs.

## Whisper handoff (planned)

After stop, captui offers to run the recording through a Whisper transcription
step. On the author's setup that is transcribe-remote (rsync to a GPU host, run
whisper, bring back the text); the handoff is just a configured command.

## CI pipeline (.github/workflows/ci.yml)

Three jobs: lint-and-security (fmt, clippy, cargo-audit, cargo-deny, gitleaks),
test (cargo test - pure functions only; no Wayland or PipeWire in CI), and nix
(`nix flake check` + `nix build`, the downstream consumption gate). Third-party
tools are pinned by version. A weekly schedule catches newly disclosed advisories.
