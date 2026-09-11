# Contributing

## Workflow

Never commit to `main`. Branch off main as `feature/<slug>`, make one commit per
feature, push, open a PR with `gh`, watch CI (`gh pr checks <n> --watch`), and
squash-merge only when green (`gh pr merge --squash --delete-branch`). Never
merge red; do not sit on a finished branch. "Did I adapt CI for this change?" is
part of done.

## Style

American English. No em or en dashes in code, UI, or docs. No version numbers in
filenames. Concise code, few comments; durable rationale goes in
docs/DESIGN-NOTES.md, not inline. Commit subjects are short imperative with a
trailing `(#PR)` and reference a ROADMAP item.

## AI transparency

This project is developed with Claude Code. Commits Claude co-authors end with
`Co-Authored-By: Claude <noreply@anthropic.com>`, and its PRs carry a
"Generated with Claude Code" footer.

## Testing

Tests are pure functions only. CI has no Wayland session, no PipeWire, and no
display, so anything that spawns wf-recorder or reads audio is local-only:
document it, do not fake it in CI.
