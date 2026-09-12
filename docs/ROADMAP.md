# Roadmap

Priority-ordered backlog. Commit subjects reference these item numbers.
Completed items are removed here and recorded in docs/CHANGELOG.md.

## P1

All P1 items are done. See docs/CHANGELOG.md.

## P2

1. Window source (compositor IPC geometry captured as a region). Blocked: needs
   a compositor that reports window geometry over IPC; Umbriel's `msg` is
   action-only (no query), so it needs an Umbriel query action first. The captui
   side is a pluggable WindowProvider once a backend can supply geometry.
2. wl-screenrec backend option (hardware encode).
