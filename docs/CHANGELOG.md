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
