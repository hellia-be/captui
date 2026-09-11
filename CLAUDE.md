# captui

Terminal UI to record the screen (or a window, or a region) with sound on
wlroots Wayland, with a live status view (audio meter, elapsed time, file size)
and one-key start/stop, feeding recordings to Whisper via the existing
`transcribe-remote` flow. Rust + ratatui. Personal tool for hellia's NixOS
machines; consumed by `nixos-config` as a flake input. Public, open-source (GPL-3.0-or-later).

**Precedence:** where this file or any doc disagrees with the code, the code
wins. Update the doc.

## What it does

- **Declare a source** (three kinds):
  - **Full display:** pick an output enumerated from `wlr-randr` (or compositor
    IPC); `wf-recorder -o <output>`.
  - **Region/area:** `slurp` drag-select gives geometry `X,Y WxH`;
    `wf-recorder -g "<geom>"`.
  - **Window/app:** get the target window's geometry from the compositor IPC
    (Umbriel / Niri) and record that region on its output. wlroots screencopy
    captures outputs and regions, not surfaces, so this is region-follows-window,
    not true per-surface capture.
- **Start / stop:** spawn the recorder, track its PID, stop with **SIGINT** so it
  finalizes the file cleanly. Never SIGKILL or hard-kill (corrupts the file).
- **Sound + live meter:** record a PipeWire source via `-a <source>` (a monitor
  of an output for system audio, or a mic). Show a live **audio level meter** so
  you can see sound is being captured (the key confirmation for a transcription
  workflow). Enumerate sources live (`wpctl status` / `pw-dump`); WirePlumber
  node names are runtime state, do not hardcode them.
- **Live status panel:** source description, elapsed timer, growing output file
  size, audio meter. No live video preview (a TUI cannot show it; that would need
  a GUI or OBS).
- **Two modes:**
  - **A/V** (default): screen+audio to `.mkv` via `wf-recorder` (or
    `wl-screenrec` for hardware encode), for a screencast or archive.
  - **Audio-only:** straight to `.wav`/`.flac` via `pw-record` or
    `ffmpeg -f pulse`, the lean path when only a Whisper transcript is wanted.
- **Whisper handoff:** after stop, offer to run the existing `transcribe-remote`
  (laptop tool: rsync to the desktop, whisper large-v3 on the GPU, `.txt` back)
  on the recording.

## Stack

Rust + `ratatui` + `crossterm`. `tokio` for async subprocess management +
timers. The `nix` crate for sending SIGINT to the child. `serde` for config.
`directories` for XDG paths. Audio metering: parse a PipeWire CLI stream
(`pw-mon` / `pw-dump`) first; link libpipewire only if that proves insufficient.

## Design notes and gotchas

- **Wayland/wlroots only.** Needs Umbriel / Niri / LabWC; there is no X11 path.
  Targets: laptop + desktop (Umbriel/Niri), laptop-laura (LabWC).
- **Multiple displays into one file is not native** to wf-recorder (one output
  per instance). Multi-output means parallel recorders or picking one output;
  document the limit, do not silently record just one.
- **SIGINT to stop, always.** wf-recorder finalizes the container on SIGINT; a
  hard kill leaves a corrupt file.
- **Window capture is a fixed region** derived from compositor IPC geometry, on
  the window's output. It does not follow the window if it moves mid-recording
  unless you actively track it (out of scope for v1).
- **Audio is all Whisper needs.** The audio-only mode exists so a transcript job
  does not carry a pointless screen video; offer it explicitly.
- **Defaults** (output dir e.g. `~/Videos/captures`, container, default audio
  source) come from `~/.config/captui/config.toml`. No credentials are involved,
  but keep host-specific runtime choices in config, not the repo.

## Conventions (follow these)

- **Style:** American English; no em or en dashes in code, UI, or docs; no
  version numbers in filenames. Concise code, few comments; durable rationale
  goes in `docs/DESIGN-NOTES.md`, not inline.
- **CI is the authoritative gate.** Locally run the cheap checks before pushing:
  `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. "Did I
  adapt CI for this change?" is part of done.
- **Delivery:** never commit to `main`. Branch `feature/**`, one commit per
  feature, push, open a PR with `gh`, `gh pr checks <n> --watch`, squash-merge
  only on green (`gh pr merge --squash --delete-branch`). Never merge red; do not
  sit on a finished branch.
- **Commit subjects:** short imperative with a trailing `(#PR)`, referencing the
  `docs/ROADMAP.md` item, e.g. `Region source via slurp (ROADMAP P1 item 2)
  (#4)`. Not Conventional Commits.
- **Public repo, full AI transparency.** This is a public, open-source project,
  and we are 100 percent transparent about Claude usage: every commit Claude
  co-authors ends with `Co-Authored-By: Claude <noreply@anthropic.com>`, PR
  descriptions carry the "Generated with Claude Code" footer, and the README
  states the project is developed with Claude Code. (This is the opposite of
  maturity-tool's private trailer-suppression, on purpose.)
- **Runtime state** (audio source, output dir, host quirks) lives in config, not
  the repo.

## CI shape (`.github/workflows/ci.yml`)

Adapt maturity-tool's shape to Rust. Triggers: push `[main, feature/**]`,
`pull_request`, `workflow_dispatch`, weekly `schedule` (catches new RUSTSEC
advisories). Top-level `permissions: contents: read`; concurrency
`cancel-in-progress` keyed on ref + event name. Pin every third-party tool by
explicit version (release tarball or pinned action); least privilege per job.
Jobs:

1. **lint-and-security:** `cargo fmt --check`; `cargo clippy --all-targets --
   -D warnings`; `cargo-audit` (RUSTSEC CVEs); `cargo-deny check` (advisories,
   licenses, bans); `gitleaks` (pinned tarball, `--redact --exit-code 1`);
   `shellcheck` on `scripts/*.sh` if any exist.
2. **test:** `cargo test`. Pure-function tests only (argv construction, geometry
   formatting, config). Anything needing a live Wayland session, PipeWire, or
   wf-recorder is local-only: CI has no display or audio, so document it and do
   not fake it.
3. **nix:** install Nix (pinned installer action), `nix flake check`,
   `nix build .#default`. This is the real downstream gate: `nixos-config`
   consumes this flake.

**No publish job.** Consumption is by git revision: `nixos-config` adds this repo
as a flake input and pins the exact rev in its `flake.lock` (the Nix analog of
maturity-tool's digest pinning). No git tags, no GitHub Releases, no semver,
matching maturity-tool.

## Layout

```
src/                   # main.rs, ui/, capture/ (recorder, sources, audio, handoff)
flake.nix              # package (.#default) + devShell; nixos-config consumes this
Cargo.toml Cargo.lock  # Cargo.lock IS committed
docs/                  # DESIGN-NOTES.md (per-subsystem why), CHANGELOG.md
                       #   (hand-written narrative, no semver), CONTRIBUTING.md,
                       #   ROADMAP.md (priority-ordered backlog, items P1/P2)
scripts/               # bash helpers if any (set -euo pipefail, shellcheck-linted)
tests/                 # pure integration tests
.github/               # workflows/ci.yml, dependabot.yml (cargo + github-actions)
CLAUDE.md README.md .gitignore
```

- **Runtime deps** (not Rust crates): `wf-recorder`, `slurp`, `wl-screenrec`
  (optional), PipeWire CLI tools (`pw-record`, `pw-dump`, `wpctl`), `wlr-randr`.
  The flake's package should wrap the binary with these on `PATH`, and the
  devShell should provide them.
- **Dependency management:** commit `Cargo.lock`. Dependabot ecosystems `cargo` +
  `github-actions`, weekly. Optionally mirror maturity-tool's home-rolled
  auto-merge workflow (never checks out PR code; auto-merges dev/patch/minor +
  actions on green).
- **LICENSE: GPL-3.0-or-later** (public, open-source, strong copyleft: forks
  must stay GPL and publish source). The `LICENSE` file is verbatim GPL-3.0;
  source-file headers add the "or later" notice. As sole copyright holder Kevin
  can still dual-license or relicense later.
- **Docs discipline:** one fact in one place; migrate durable content out of this
  CLAUDE.md into `docs/` as the repo grows, leaving CLAUDE.md a thin pointer
  index + conventions summary.
