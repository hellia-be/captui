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

After a source is chosen, captui asks for the audio in two steps: an **output**
(system audio) and then an **input** (mic), each independently choosable or None.
Both are enumerated from `pw-dump`'s JSON (parsed with serde_json in the pure
`parse_pw_dump`, so CI can test it) rather than by scraping `wpctl status`'s tree.
Each `Audio/Source` node is a real input, labeled "Mic: <name>"; each `Audio/Sink`
becomes a "Monitor of <sink>" option whose value is the sink's node name plus
`.monitor`, the PulseAudio name for a sink's monitor. The two metadata defaults
are surfaced first: the default sink (`default.audio.sink`) as "System audio
(all)" and the default source (`default.audio.source`) as "Microphone (default)",
each excluded from the per-device lists to avoid a duplicate. The app screens
split the flat list by `is_monitor`: monitors are the output options, mics the
input options. Output defaults to System audio (preselected); input defaults to
None. Running pw-dump is IO in the app layer.

`audio_target(output, input)` (pure) turns the two choices into one of: Silent
(neither), Single (exactly one, recorded directly), or Mix (both). For Mix,
captui builds a temporary PipeWire graph via pactl: a `module-null-sink` named
`captui_mix`, plus a `module-loopback` from each chosen source into it, then
records `captui_mix.monitor`. The loaded module ids are tracked and unloaded in
reverse on stop or drop (an `AudioMix` `Drop`), so the graph never leaks; wf-
recorder is finalized first, then the mix is torn down. This is why pactl
(pulseaudio) is a runtime dependency. A short loopback latency (20ms) keeps the
mixed audio close to video.

The value carried forward is the node name passed to wf-recorder. It must be
given as `--audio=<node>` (the attached form): wf-recorder's `-a`/`--audio` takes
an optional argument, so getopt only binds a value when attached. A space-
separated `-a <node>` silently records the default source (the mic) instead,
which is the "muted" bug we hit. Node names are runtime state, never hardcoded.

## Recorder (src/recorder.rs)

Pure argv builders and output naming, kept IO-free so they are testable in CI.
The A/V mode records screen + a PipeWire source to mkv; the audio-only mode
writes flac for a lean Whisper transcript. `wf_recorder_argv` takes an optional
audio node (omitting `--audio` when the user picked "No audio").

The recording screen shows a live status panel: an elapsed timer and the growing
output file size. The timer runs off an `Instant` captured at spawn and freezes
at the value sampled on stop; the size is read from the file's metadata each
draw (the loop redraws ~10x/s). Duration and byte formatting are pure helpers in
src/format.rs, unit-tested in CI.

Video quality is set explicitly instead of relying on wf-recorder's defaults,
which look soft (especially screen text): software libx264 at `crf=18` (visually
near-lossless, sharper than the ~23 default) with `preset=fast` to stay
realtime. These live as constants in recorder.rs; a future config item can
expose them, and hardware (VAAPI) encoding is the separate wl-screenrec backend
on the roadmap. `timestamped_name`
formats a UTC `captui-YYYYMMDD-HHMMSS.<ext>` name from a Unix timestamp using the
days-from-civil algorithm, so no date crate is pulled in.

The process spawn and stop live in the IO app layer (src/main.rs). After the
audio pick, captui resolves the output directory (the XDG videos dir via the
directories crate, else `$HOME/Videos`, then a `captures` subdir), spawns
wf-recorder as a child with its stdio nulled so it does not corrupt the TUI, and
tracks the child. Stopping sends SIGINT via the nix crate (never a hard kill, so
wf-recorder finalizes the container) and waits for the child; quitting while
recording stops first, so a capture is never left unfinalized.

## Audio metering (src/meter.rs)

The "is sound coming in" confirmation is a live level bar on the recording
screen. While recording (and only when an audio source was chosen), a second
`pw-record --raw --format=f32 --channels=1 --target=<node> -` streams headerless
mono float samples to stdout; a background thread computes a decaying peak from
each chunk and publishes it in an atomic. The draw loop reads that atomic and
renders a bar. `--raw` matters: without it pw-cat wraps stdout in an `.au`
container (big-endian), which would garble the little-endian float parse. The
sample-to-peak and level-to-bar helpers are pure and unit-tested; the process and
reader thread are IO in src/main.rs, torn down (child killed, thread joined) when
the meter is dropped on stop or quit. Linking libpipewire stays a fallback only
if the CLI stream proves too thin.

## Whisper handoff (planned)

After stop, captui offers to run the recording through a Whisper transcription
step. On the author's setup that is transcribe-remote (rsync to a GPU host, run
whisper, bring back the text); the handoff is just a configured command.

## CI pipeline (.github/workflows/ci.yml)

Three jobs: lint-and-security (fmt, clippy, cargo-audit, cargo-deny, gitleaks),
test (cargo test - pure functions only; no Wayland or PipeWire in CI), and nix
(`nix flake check` + `nix build`, the downstream consumption gate). Third-party
tools are pinned by version. A weekly schedule catches newly disclosed advisories.
