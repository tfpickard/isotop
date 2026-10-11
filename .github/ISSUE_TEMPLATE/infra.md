---
name: Infra
about: Refactor, tooling, CI, tests or docs that change no pixels
title: "A#: "
labels: infra
---

## Goal

<!-- What changes and why, in a few sentences. -->

## Depends on

<!-- Issues that must land first, or "None". -->

## Scope

<!-- Concrete changes: files, functions, decisions. -->

## Tests

<!-- Named tests, each a sentence about behaviour. -->

## Acceptance

- [ ] The gates pass in this order: `cargo fmt && cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release`, `python3 scripts/smoke_terminal.py`.
- [ ] `cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings` is clean, run with its own `CARGO_TARGET_DIR`.
- [ ] Golden hashes are unchanged, or the change is explained.

## Out of scope

## Delivery

Branch `refactor/...`, `feat/...` or `docs/...`, cut from an up-to-date `main`. The PR body includes `Closes #<this issue>`. Commits are attributed to the human author only. Stop after opening the PR.
