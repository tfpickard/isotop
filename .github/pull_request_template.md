## What changed

<!-- What and why, in a few lines. For a stacked PR, list issues as "Part of #N" (see Closes below). -->

## How it was checked

Gates, in this order (see AGENTS.md):

- [ ] `cargo fmt && cargo fmt --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test` (includes the golden hashes and the AGENTS.md file table)
- [ ] `cargo build --release`
- [ ] `python3 scripts/smoke_terminal.py`
- [ ] `cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings` when `platform/macos` or shared code changed (own `CARGO_TARGET_DIR`)

Golden hashes (`tests/golden.txt`): unchanged / changed because ...

Renders for visual changes (CPU and GPU where the rasterizer is touched), and `--benchmark` numbers when performance is the point:

<!-- Paste commands and output or attach images. -->

## Closes

<!-- `Closes #N` when this PR merges into main. When stacked on another branch, write `Part of #N` and close the issue when the stack reaches main. -->
Closes #
