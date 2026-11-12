# scripts

## OVERVIEW
Python gates and generators for CI and PRs. Own file because each script is a gate with exact flags and
limits that must not drift, and one script writes to GitHub.

## WHERE TO LOOK
| Task | File | Symbol / notes |
|---|---|---|
| Terminal smoke test | `smoke_terminal.py` | `PICKER_VISITS`, `VIEW_KEYS`, `ENDINGS`; arg `binary` (default `target/release/isotop`) |
| CPU/GPU parity | `compare_png.py` | `first second`, `--max-mean`, `--max-share`, `--levels`, `--report FILE`, `--self-test` |
| Metrics report | `code_report.py` | `--check`, `--self-test`; writes `docs/METRICS.md` and the README metrics block |
| Wiki check | `check_wiki.py` | prints `N pages, M problems`, exit 1 on any problem |
| Wiki render media | `render_wiki_media.py` | `--only NAME`, `--check-tools`, `--output DIR`, `--jobs N` |
| GitHub Project loader | `setup_project.py` | `--dry-run`; one-off, writes to GitHub |

## CONVENTIONS
- Standard library only (Python 3.11+ for `code_report.py`); `code_report.py` also needs `tokei`, pinned to 14.0.0 in CI.
- `smoke_terminal.py` drives the release binary through a pty, once demo over shared memory, once live inline. It skips
  names `--list-views` does not print here. A new view is added to `PICKER_VISITS`; a view with local keys also gets a
  `VIEW_KEYS` entry whose expected text is a legend or status string.
- `smoke_terminal.py` also runs `exercise_ending` in each mode for the endings in `ENDINGS`: a panic on the main thread
  (`ISOTOP_FORCE_PANIC=main`, exit 101), SIGINT, SIGTERM and SIGHUP (killed by the signal), and a panic on another
  thread (`ISOTOP_FORCE_PANIC=thread`, the app keeps running and `q` quits it with status 0). Each checks one keyboard
  pop and one alternate-screen exit in order, the cursor shown, the mouse off, the images deleted, raw mode off, no frame
  after the restore, the message once (after the screen was left for a main-thread panic) and no shared-memory object
  left behind. That check does not read the frames, so it tests the app's own cleanup; a new ending is one `Ending`.
- `compare_png.py` exits 0 within limits, 1 on size mismatch or exceeded limit, 2 on decode error. Defaults (the
  `docs/VIEW_CONVENTIONS.md` tolerance): mean 0.25, share 0.25 percent, levels 2.
- `check_wiki.py` checks page prefixes, first heading, page links, and `docs/` media links; fenced blocks are skipped
  but link syntax in inline code is scanned. It does not check maths, code facts or spelling.
- `render_wiki_media.py` needs a release build, ImageMagick (`convert`, `montage`) and ffmpeg with libwebp. A loop
  frame k is the run with `--benchmark k+1`, never a series of `--time` values.
- Linux CI runs `compare_png.py` over every view on lavapipe (`ISOTOP_ALLOW_SOFTWARE_GPU=1`, step "CPU/GPU parity for every view (lavapipe, blocking)") and fails on a miss; only the `known_debt` views in that step (tracked in #174) are reported without failing. Do not extend that list for a new view.
- CI runs `code_report.py --self-test` and `compare_png.py --self-test`; keep those fixtures passing.

## ANTI-PATTERNS
- Never loosen the parity limits. `compare_png.py` has no per-view selection: pass the line-dominant limit by hand,
  `--max-mean 0.5 --max-share 1.0`, for the views `docs/VIEW_CONVENTIONS.md` lists as line-dominant (city, ripple,
  cores, cells, globe). No new view may use it.
- Do not weaken existing smoke-test assertions.
- Do not add a pip or crate dependency to a script.
- Do not run `setup_project.py` unprompted: it writes to GitHub (use `--dry-run` to inspect).
- Do not hand-edit what `code_report.py` generates.

## COMMANDS
- `python3 scripts/smoke_terminal.py [binary]`
- `python3 scripts/compare_png.py cpu.png gpu.png`
- `python3 scripts/compare_png.py --max-mean 0.5 --max-share 1.0 cpu.png gpu.png` (line-dominant views only)
- `python3 scripts/compare_png.py --self-test`
- `python3 scripts/code_report.py [--check|--self-test]`
- `python3 scripts/check_wiki.py`
- `python3 scripts/render_wiki_media.py --check-tools`
