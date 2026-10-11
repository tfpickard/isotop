---
name: View
about: A new view
title: "View: <name>, <one line>"
labels: view
---

## Goal

<!-- What the view shows and which measured quantities drive it. -->

## Depends on

<!-- Prerequisites and building blocks by ID and issue number, each with what it provides. -->

## Read first

- `AGENTS.md`: invariants, gates and the platform boundary.
- `docs/VIEW_CONVENTIONS.md`

## Scope

<!-- Module, registry row, mapping decisions, rendering and budget, headless setup (reference --time). -->

## Platforms

- **Linux:**
- **macOS:**

## Tests

<!-- Named tests in the view's module, including a same-frame-twice render test. -->

## Acceptance

- [ ] The gates pass in this order: `cargo fmt && cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release`, `python3 scripts/smoke_terminal.py`.
- [ ] CPU and GPU PNGs of `--demo --view <name>` are attached and agree.
- [ ] Other views' demo PNGs and `tests/golden.txt` are unchanged.
- [ ] The README section opens with the mapping table and lists what is decorative.
- [ ] `cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings` is clean.

## Out of scope

## Delivery

Branch `feat/<name>`, cut from an up-to-date `main`. The PR body includes `Closes #<this issue>`. Commits are attributed to the human author only. Stop after opening the PR.
