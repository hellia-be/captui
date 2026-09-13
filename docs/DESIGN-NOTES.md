# Design notes

Per-subsystem rationale. Keep each fact in one place; code carries short
pointers back here rather than long comments.

## Selection UI (src/main.rs)

The picker is a single screen of three side-by-side panes — Display, Audio, Mic —
that TAB (or left/right) cycles between; up/down selects within the focused pane
and Enter starts recording, at which point the panes are replaced by the recording
view. This replaced the earlier source -> output -> input wizard.

The Display pane lists each enabled display, then Window (only when
`window_geometry_command` is configured), Region, and Audio only. The
Audio pane lists the outputs to capture — System audio (all), each sink's
monitor, each playing app, and None. The Mic pane lists the mics plus None. So region and
audio-only are choices in the Display pane rather than separate keys/screens.

(There is no "all screens" option: wf-recorder captures one output per instance,
and a region spanning multiple outputs is rejected with "Failed to select
output". Capturing every screen would need parallel recorders and one file per
screen.)

## Sources (src/sources.rs)

wlroots screencopy captures outputs and regions, not surfaces. So captui offers
a display (`wf-recorder -o <output>`) or a region (`-g "X,Y WxH"`). A window is
just a region whose rectangle came from the compositor rather than a drag: it is
a fixed region taken once at start and does not follow the window if it moves.

A region source comes from `slurp` (the Display pane's "Region"): on record it
spawns slurp for an interactive drag-select and captures its `X,Y WxH` on stdout,
validated by the pure `parse_geometry` (rejects malformed output and zero-area
rectangles) into a `Source::Region`. Spawning slurp is IO in the app layer.

The window source (the Display pane's "Window") is the same `Source::Region`, but
its geometry comes from the user's `window_geometry_command` instead of slurp.
captui does not speak any compositor's IPC: it runs the configured command with
`sh -c` (so a pipeline through `jq` and `slurp` works) and parses whatever it
prints with the same `parse_geometry`. This is deliberately compositor-agnostic —
niri, Sway, and Hyprland each know how to enumerate their windows, and reshaping
that to `X,Y WxH` is the user's one-line command. It also sidesteps the old
blocker that Umbriel's `msg` is action-only: any working query will do. The Window
entry is only shown when the key is set, since without a command there is nothing
to run. Because a window is represented as a region, nothing downstream (argv,
recording view, naming) needs a window-specific path.

The command must not be a "focused window" query. captui runs in a terminal, so
at record time the focused window is captui itself and such a query would capture
captui. The window has to be chosen at record time instead: the portable recipe
pipes every window's rectangle into `slurp`, which reads
`"<x>,<y> <width>x<height> [label]"` lines from stdin and lets the user click the
one to record, printing its box. That is why the option is labeled just "Window",
not "focused": captui does not assume how the command picks the window, and the
focus-independent click-to-pick flow is the one that actually works from a TUI.

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

Audio is enumerated from `pw-dump`'s JSON (parsed with serde_json in the pure
`parse_pw_dump`, so CI can test it) rather than by scraping `wpctl status`'s tree.
Each `Audio/Source` node is a real input, labeled "Mic: <name>"; each `Audio/Sink`
becomes a monitor whose value is the sink's node name plus `.monitor`, the
PulseAudio name for a sink's monitor. Every mic stays visible by its own name; the
default source (`default.audio.source`) is marked "(default)" and sorted first,
never collapsed into an opaque label that hides which physical device it is (that
hid a user's real mic).

Individual apps are enumerated separately by `parse_app_streams` (also pure):
every `Stream/Output/Audio` node is one app currently playing sound, labeled
"App: <name>" (from `application.name`, falling back to `media.name` then
`node.name`) and carrying `app: true`. Its `node_name` is the PipeWire object id,
not a pulse source name, because an app stream is not a source you can hand to a
recorder or to parec; it is a graph node you route (see below).

The UI splits these across the two audio panes: the Audio pane lists the monitors
(System audio (all) first, then each sink's monitor) followed by the apps, plus
None, preselecting `config.audio_output` if set; the Mic pane lists the mics plus
None, preselecting the configured or default mic.

Routing is decided inline from the chosen `(output, input)` (each mapped to an
`Ingredient`: a pulse `Source` name, or an `App(object id)`). Neither chosen is
silent; exactly one plain source is recorded directly; anything else — two
ingredients, or any app (which cannot be recorded directly) — builds a temporary
PipeWire graph via pactl: a `module-null-sink` named `captui_mix`, into which
each ingredient is fed — a `Source` via `module-loopback`, an `App` via a
`pw-link <object id> captui_mix` fan-out — then records `captui_mix.monitor`. The
pw-link is non-destructive: it adds a link so the app keeps playing to its normal
sink as well, and it disappears when the null sink is unloaded. The loaded module
ids are tracked and unloaded in reverse on stop or drop (an `AudioMix` `Drop`), so
the graph never leaks; wf-recorder is finalized first, then the mix is torn down.
This is why pactl (pulseaudio) and pw-link (pipewire) are runtime dependencies. A
short loopback latency (20ms) keeps the mixed audio close to video.

In the mix case each loopback's sink-input on `captui_mix` is resolved from
`pactl list sink-inputs` (matching its owner module id, via the pure
`parse_sink_input_index`), so the recording screen can adjust that source's level
with `pactl set-sink-input-volume` (left/right on the focused source) without
touching system volume. Resolution is best-effort: if an index cannot be found,
that source simply has no volume control and metering/recording still work.
Volume control exists only for loopback sources in the mix; a single directly-
recorded source has none (adjusting it would change the device's global volume),
and an app has none either: a pw-link fan-out is not a loopback sink-input, so
there is nothing to attenuate short of the app's own global volume. An app is
also metered on the combined `captui_mix.monitor` rather than individually, since
parec cannot target a bare graph node; a plain source keeps its own meter.

The value carried forward is the node name passed to wf-recorder. It must be
given as `--audio=<node>` (the attached form): wf-recorder's `-a`/`--audio` takes
an optional argument, so getopt only binds a value when attached. A space-
separated `-a <node>` silently records the default source (the mic) instead,
which is the "muted" bug we hit. Node names are runtime state, never hardcoded.

## Audio-only mode

Pressing `a` on the source screen skips video and records straight to a `.flac`
for a lean transcript. It reuses the same output/input pickers (and the mix when
both are chosen), but the recorder is `ffmpeg -f pulse -i <node> <path.flac>`
(ffmpeg picks flac from the extension) instead of wf-recorder, and the extension
comes from `Mode::AudioOnly`. ffmpeg finalizes the flac on SIGINT, so the same
stop closes the file cleanly. Audio-only with neither output nor input is refused
(nothing to record).

It records via ffmpeg's PulseAudio input, not `pw-record --target=<node>`, for
the same reason the meters use `parec` (see Audio metering): `pw-record --target`
wants a PipeWire node and cannot resolve a pulse `<sink>.monitor` name, so it
silently fell back to the default source (the mic) and every monitor or app
capture recorded the microphone instead of the intended audio. ffmpeg's pulse
input uses the same PulseAudio device namespace the recorder's `--audio=` and the
meters already rely on, so a mic, a sink monitor, and the `captui_mix.monitor`
all resolve correctly.

## Recorder (src/recorder.rs)

Pure argv builders and output naming, kept IO-free so they are testable in CI.
The A/V mode records screen + a PipeWire source to mkv; the audio-only mode
writes flac for a lean Whisper transcript. `wf_recorder_argv` takes an optional
audio node (omitting `--audio` when the user picked "No audio").

The recording screen shows a live status panel: an elapsed timer and the growing
output file size. The timer runs off an `Instant` captured at spawn and freezes
at the value sampled on stop; the size is read from the file's metadata each
draw (the loop redraws ~20x/s). Duration and byte formatting are pure helpers in
src/format.rs, unit-tested in CI.

The A/V recorder is selectable via `backend` in the config: `wf-recorder`
(default, software libx264) or `wl-screenrec` (hardware VAAPI encode by default,
better quality-per-bitrate and lower CPU where the GPU supports it). `Backend`
(pure) dispatches to the matching argv builder. Both take the same `-o`/`-g`
source and `-f` output; they differ on audio — wf-recorder wants the attached
`--audio=<node>`, wl-screenrec wants `--audio --audio-device <node>`. Audio-only
mode always uses ffmpeg's pulse input regardless of backend. wl-screenrec's hardware VAAPI
path fails to negotiate a capture format on NVIDIA (block-linear dmabuf
modifiers), so `no_hw = true` in the config adds `--no-hw` (software encode) and
NVIDIA users are better off on the default wf-recorder.

Pause (`p`) works by segments, because no wlroots recorder pauses natively and
SIGSTOP desyncs audio (the audio server buffers through the freeze). Each capture
records to numbered part files (`<name>.partN.<ext>` via `segment_path`); pausing
stops the current recorder (finalizing that part), resuming spawns a fresh
recorder for the next part, and stop concatenates the parts into the final file
with `ffmpeg -f concat -c copy` (stream copy, no re-encode, so no frozen frames or
drift). A single part (never paused) is just renamed. The pact/mix and the meters
stay up across a pause; the displayed timer excludes paused time. This is why
ffmpeg is a runtime dependency. `segment_path`, `concat_list_line`, and
`ffmpeg_concat_argv` are pure and tested; the spawn/concat IO is in the app layer.

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
wf-recorder as a child (stdout nulled). Stopping sends SIGINT via the nix crate
(never a hard kill, so wf-recorder finalizes the container) and waits for the
child; quitting while recording stops first, so a capture is never left
unfinalized.

The recorder's stderr is captured (a drain thread into a shared string), not
nulled, so failures are not silent. If the recorder exits on its own (polled with
`try_wait` each tick) or the output file is missing/empty on stop, captui marks
the recording failed and shows the last stderr line, rather than counting a fake
timer over a dead recorder. Only a recording that produced a non-empty file is
reported as saved (and eligible for the transcribe handoff).

## Audio metering (src/meter.rs)

The "is sound coming in" confirmation is a live level bar per chosen source on
the recording screen: an "output" bar and/or an "input" bar. Each meters the raw
chosen node (the sink monitor and/or the mic) directly, not the mixed
`captui_mix.monitor`, so the two levels stay separate even when both are being
mixed into the recording. The one exception is an app output: it is not a pulse
device parec can open, so its "output" bar meters `captui_mix.monitor` (the
combined mix) instead. Each bar is a `parec --device=<node> --format=float32le
--rate=48000 --channels=1 --latency-msec=30` streaming headerless mono float
samples to stdout; a background thread computes a decaying peak from each chunk
and publishes it in an atomic. The draw loop reads those atomics and renders the
bars. The low `--latency-msec` matters: parec's default buffering delivers large
fragments, which makes the meter lag; a small buffer keeps it responsive.

parec (PulseAudio), not pw-record, because the meter must accept the same source
names the recorder uses, including a sink monitor `<sink>.monitor`. pw-record's
`--target` wants a PipeWire node and does not resolve the pulse `.monitor` name,
so it silently falls back to the default source, making the output meter read the
mic (both bars then show the same level). parec `--device` takes the pulse name,
matching wf-recorder. `--raw` matters: without it pw-cat wraps stdout in an `.au`
container (big-endian), which would garble the little-endian float parse. The
sample-to-peak and level-to-bar helpers are pure and unit-tested; the process and
reader thread are IO in src/main.rs, torn down (child killed, thread joined) when
the meter is dropped on stop or quit. Linking libpipewire stays a fallback only
if the CLI stream proves too thin.

## Config (src/config.rs)

`~/.config/captui/config.toml` (via the directories crate) holds host-specific
defaults, all optional: `output_dir` (where captures are written; `~/` is
expanded; when unset, the XDG videos dir's `captures/`), `container` (the A/V
file extension, default `mkv`; audio-only stays `flac`), and `audio_output` /
`audio_input` (a node name to preselect in the pickers). Parsing is the pure
`parse_config` (serde + toml); any read or parse failure falls back to defaults,
so a missing or malformed file never blocks startup. Loading the file is IO in
the app layer. Runtime state stays in config, not the repo.

## Whisper handoff (src/main.rs, recorder.rs)

After stop, if `transcribe_command` is set in the config, the stopped recording
screen offers `t` to transcribe. On the author's setup that command is
transcribe-remote (rsync to a GPU host, run whisper, bring back the text); the
handoff is just a configured command. `transcribe_argv` (pure) turns the template
into an argv: whitespace-split, with a `{}` token replaced by the recording path,
or the path appended when there is no `{}`. It is intentionally not run through a
shell (no injection, predictable). The command runs after the TUI exits, in the
foreground with inherited stdio, so the user sees rsync/whisper progress.

## CI pipeline (.github/workflows/ci.yml)

Three jobs: lint-and-security (fmt, clippy, cargo-audit, cargo-deny, gitleaks),
test (cargo test - pure functions only; no Wayland or PipeWire in CI), and nix
(`nix flake check` + `nix build`, the downstream consumption gate). Third-party
tools are pinned by version. A weekly schedule catches newly disclosed advisories.
