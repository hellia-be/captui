# Changelog

Hand-written, newest first. Not tied to version numbers.

## Delivery

- Protected `main`: PRs required, CI (lint-and-security/test/nix) must be green,
  linear history, no force-push or deletion.
- CODEOWNERS assigns the sole maintainer for auto-requested review.
- Dependabot auto-merge workflow: enables auto-merge on green for patch, minor,
  and development bumps plus all github-actions updates; never checks out PR
  code; major production bumps stay manual.

## Features

- Window source, compositor-agnostic via config: set `window_geometry_command`
  in the config to a command that prints the focused window's geometry as
  `X,Y WxH` (slurp's format), and the Display pane gains a "Window (focused)"
  entry. Choosing it runs the command (through `sh -c`, so pipes work), parses
  the geometry, and records that rectangle like a region. The option is hidden
  when the key is unset, so captui stays portable across compositors: each
  compositor's own query (niri msg, swaymsg, hyprctl, ...) supplies the geometry
  through a small user command instead of captui hardcoding any IPC. Capture is a
  fixed region on the window's output, taken once at start; it does not follow
  the window if it moves.
- Per-application audio capture: the Audio pane now also lists each running app
  that is playing sound ("App: <name>"), enumerated from `pw-dump`
  (`Stream/Output/Audio` nodes). Choosing one records just that app: captui
  creates the `captui_mix` null sink and fans the app's output into it with
  `pw-link` (non-destructive, so the app keeps playing normally), then records
  the mix monitor; the link and sink are torn down on stop. It composes with a
  mic like any other output. Caveats: an app stream is not a pulse source, so it
  is metered on the combined mix monitor (not individually) and has no per-source
  volume; and unlike a plain monitor it can be mixed with a mic without a second
  loopback. Adds pw-link (PipeWire CLI) to the runtime tools.
- Reworked picker: one screen with three side-by-side panes — Display, Audio, Mic
  — that TAB (or ←/→) cycles, ↑/↓ selects within, Enter records; it is replaced by
  the recording view on launch. The Display pane holds each display plus Region
  and Audio only; the Audio pane lists the outputs (System audio + each monitor)
  plus None; the Mic pane lists the mics. Replaces the old source -> output ->
  input wizard.
- Pause/resume (`p`) that actually works: recording is split into segments — each
  pause finalizes the current segment, resume starts a new one, and stop
  concatenates them with `ffmpeg -c copy` (no re-encode, so no frozen frames or
  A/V drift). The timer excludes paused time. Works for wf-recorder,
  wl-screenrec, and audio-only. Adds ffmpeg as a runtime dependency.
- wl-screenrec backend: set `backend = "wl-screenrec"` in the config to record
  A/V with hardware (VAAPI) encoding instead of wf-recorder's software libx264;
  default stays `wf-recorder`. Audio-only mode is unaffected (always pw-record).
- New recording without restarting: after stopping, press `n` on the recording
  screen to return to the source picker and record again.
- Whisper handoff: with `transcribe_command` set in the config, the stopped
  recording screen offers `t` to run it on the recording (a `{}` in the command
  is replaced by the file path, else it is appended). It runs after the TUI exits
  with inherited output, so you see transcription progress.
- Config file: `~/.config/captui/config.toml` sets optional defaults — output
  directory (with `~` expansion), A/V container extension, and a preselected
  audio output/input by node name. Missing or malformed config falls back to
  built-in defaults.
- Per-source mix volume: when mixing output + input, the recording screen lets
  you focus a source (up/down) and adjust its level (left/right) with a live
  percentage, applied to that loopback's stream only, never system volume.
- Separate output and input level meters: the recording screen shows one bar per
  chosen source (metering the raw sink monitor and mic directly), so you can see
  system audio and your mic independently even when they are mixed.
- Audio-only mode: press `a` on the source screen to skip video and record
  straight to a timestamped `.flac` (via pw-record), reusing the output/input
  pickers and the mix. Lean path for a transcript.
- Select output and input independently: the audio step is now two screens, an
  output (system audio) and an input (mic), each choosable or None. When both are
  set, captui builds a temporary PipeWire mix (a `captui_mix` null sink fed by a
  loopback from each source) and records its monitor, then tears the mix down on
  stop; one source records directly; neither is silent. Adds pactl (pulseaudio)
  as a runtime dependency. Every mic is listed by its real name with the default
  marked "(default)" and preselected, instead of hiding the default behind an
  opaque label.
- Clearer microphone selection: mics are now labeled "Mic: <name>", and the
  default input is surfaced as a "Microphone (default)" option alongside "System
  audio (all)", so choosing a specific mic among several is obvious.
- Live audio level meter: while recording with an audio source, a second
  `pw-record` stream feeds a decaying peak meter shown as a bar on the recording
  screen, so you can see sound is being captured. Completes P1.
- Recording status panel: the recording screen now shows a live elapsed timer
  and the growing output file size; the timer freezes at the final duration when
  stopped.

## Fixes

- Audio-only recording captured the microphone instead of the chosen source:
  the recorder was `pw-record --target=<node>`, but `pw-record --target` wants a
  PipeWire node and cannot resolve a pulse `<sink>.monitor` name, so it silently
  fell back to the default source (the mic). Every monitor and per-app audio-only
  capture recorded the room mic. Record with `ffmpeg -f pulse -i <node>` instead
  (the same PulseAudio namespace the recorder's `--audio=` and the meters use), so
  a mic, a sink monitor, and the mix monitor all resolve correctly. (The A/V path
  via wf-recorder `--audio=` was already correct.)
- wl-screenrec `no_hw` option and shorter error display: `no_hw = true` in the
  config forces software encode (`--no-hw`), and the failure message shown in the
  UI is truncated. wl-screenrec's hardware path can't negotiate a capture format
  on NVIDIA; those GPUs should use the default wf-recorder. (Documented.)
- Recorder failures are no longer silent: captui captured and nulled the
  recorder's stderr, so a recorder that crashed or produced no file left a fake
  running timer and no saved capture. Now the recorder's stderr is captured, an
  early exit is detected, and a missing/empty output on stop is reported as
  "recording failed: <error>" rather than "saved".
- The "n new recording" hint was hidden after stopping: a "saved" status
  occupied the footer. Drop that status on a successful stop (the body already
  shows the saved path), so the `n new recording / q quit` hint shows.
- Output and input meters showed the same level: the meter used
  `pw-record --target`, which wants a PipeWire node and cannot resolve a pulse
  `<sink>.monitor`, so it fell back to the default source (the mic) and both bars
  tracked the mic. Meter with `parec --device` (same pulse names as the recorder)
  instead, so each bar tracks its own source.
- Better video quality: encode with libx264 at `crf=18`, `preset=fast` instead
  of wf-recorder's soft defaults, so screen text and detail are sharp. (Hardware
  VAAPI encoding remains the separate wl-screenrec backend on the roadmap.)
- Audio was recorded muted: the audio node was passed as `-a <node>`, but
  wf-recorder's `-a`/`--audio` takes an optional argument that getopt only binds
  when attached, so it silently fell back to the default source (the mic). Pass
  it as `--audio=<node>` instead.
- Added a "System audio (all)" option that targets the default sink's monitor,
  preselected, so capturing everything playing is one keypress.

## Features

- Start/stop recording: after the audio pick, captui spawns wf-recorder to a
  timestamped `.mkv` under the videos dir and shows a recording screen. Press `s`
  to stop (or `q` to stop and quit); stop sends SIGINT so the container is
  finalized, never a hard kill. This makes captui an actual recorder.
- Audio source picker: after choosing a source, pick an audio input on a second
  screen. Sources are enumerated from `pw-dump` (mics and sink monitors for
  system audio), with a "No audio (silent)" option; Esc goes back. Pure
  `parse_pw_dump` parser with tests.
- Region source: pressing `r` in the picker runs `slurp` for an interactive
  drag-select and records the chosen rectangle as the capture source (validated
  by the pure `parse_geometry`). Works even when no displays are enumerated.
- Displays are numbered in reading order (top to bottom, then left to right) by
  layout position, instead of wlr-randr's spatially arbitrary connector order,
  so the picker and identify numbers line up with the physical arrangement.
- Identify overlay: pressing `i` in the picker flashes each display's row number
  on its physical screen (a Wayland layer-shell client, smithay-client-toolkit),
  the reliable way to tell identical monitors apart. Gated behind the non-default
  `identify` Cargo feature; the nix package ships it built in. Pure-Rust wayland
  backend, so no libwayland or pkg-config is pulled in.
- Source picker distinguishes identical monitors: each row is numbered and
  shows the current mode, layout position, and a directional hint (left/right,
  top/bottom, or a grid combination) derived from how the outputs sit relative
  to each other. Parsing and the hint logic are pure and unit-tested.
- Source picker: enumerate displays from wlr-randr and select one in a
  navigable ratatui list. Pure `parse_wlr_randr` parser is unit-tested; the
  picker filters to enabled outputs.

## Delivery

- Wrap the packaged binary with its runtime tools (wf-recorder, wl-screenrec,
  slurp, wlr-randr, PipeWire and WirePlumber CLIs) on `PATH` via wrapProgram, so
  `nix run`/installed captui finds them without a devShell. Same list feeds the
  devShell.
- Commit `flake.lock` so `nix run github:hellia-be/captui` works from an
  immutable git rev (nix cannot write a lock into the fetched source) and
  downstream consumers pin reproducible inputs.

## Scaffold

- Repo scaffold: Cargo package, Nix flake (package + devShell with the wlroots
  runtime tools), CI (fmt/clippy/cargo-audit/cargo-deny/gitleaks/test + nix
  build), Dependabot, docs, and the GPL-3.0-or-later license.
- Core pure helpers with tests: source-to-args (src/sources.rs) and wf-recorder
  argv + output naming (src/recorder.rs).
- Minimal ratatui shell (placeholder status screen, q to quit).
