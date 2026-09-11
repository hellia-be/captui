# Changelog

Hand-written, newest first. Not tied to version numbers.

## Scaffold

- Repo scaffold: Cargo package, Nix flake (package + devShell with the wlroots
  runtime tools), CI (fmt/clippy/cargo-audit/cargo-deny/gitleaks/test + nix
  build), Dependabot, docs, and the GPL-3.0-or-later license.
- Core pure helpers with tests: source-to-args (src/sources.rs) and wf-recorder
  argv + output naming (src/recorder.rs).
- Minimal ratatui shell (placeholder status screen, q to quit).
