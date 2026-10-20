# src/platform

## OVERVIEW
The OS contract and its Linux and macOS implementations. Own file because it is a distinct domain: the only code
allowed to read `/proc`, `/sys` or libproc, and where nearly all `unsafe` lives.

## WHERE TO LOOK
| Task | File | Symbol / notes |
|---|---|---|
| What each platform must export | `mod.rs` | the `//!` header; both platforms provide identical names |
| Platform selection | `mod.rs` | `#[cfg(target_os = "...")] mod linux; pub use linux::*;`; any other OS hits `compile_error!` |
| New optional source | `mod.rs` | `Source`, `Source::ALL`, `Source::name`, `Sources::of`, `Sources` (u64 bit set, 64 variants at most) |
| A reading that may be gone | `mod.rs` | `Taken` |
| Life events | `mod.rs`, `linux/events.rs`, `macos/events.rs` | `LifeEvent`, `Cause::of_status`, `ExitRecord`, `Observed`, `LifeQueue` |
| Linux stat parsing | `linux/mod.rs` | `parse_stat`, `split_stat`, `timing` |
| Netlink byte offsets | `linux/events.rs`, `linux/net.rs` | `mod field` holds the uapi offsets |
| macOS FFI declarations | `macos/ffi.rs` | every extern and `#[repr(C)]` struct libc lacks, with size assertions |
| macOS pure decisions | `macos/logic.rs` | no FFI; also built into Linux tests |
| Kernel counter fixtures | `linux/fixtures/` | consumed only by `linux/counters.rs` through `include_str!` |

## CONVENTIONS
- `macos/logic.rs` and `macos/journal.rs` are compiled into Linux test builds (`macos_logic`, `macos_journal`
  in `mod.rs`, `#[cfg(all(test, not(target_os = "macos")))]` with `#[path]`). Keep them std-only: no `libc::`, no extern.
- Platform-only items silence dead code with `#[cfg_attr(all(target_os = "macos", not(test)), expect(...))]`
  or `#[cfg(target_os = "linux")]`, never `#[allow]`.
- Parsers are pure functions of `&str` so tests need no `/proc`. Each file ends in a `#[cfg(test)] mod tests`.
- Fixtures are named `<arch>_<what>[_<condition>].txt` (`x86_interrupts_cpu2_offline.txt`); real captures say so in the header.
- Unreadable Linux data is `None` or listed in `Sampler::missing`; `pressure` is the one exception (returns 0, reports the gap).
- `FileScan::sample`: a pid the scan did not reach is absent from the map; `None` means the table could not be read.
- Adding a `Source` end to end:
  1. Variant with doc comment in `Source`, appended to `Source::ALL`, arm in `Source::name` (no wildcard).
  2. Move the sentinel in `source_all_lists_every_source_in_declaration_order` to the new last variant: it asserts
     `Source::<last> as usize + 1 == Source::ALL.len()`, and today that variant is `Source::WaitChannel`.
  3. Document the function in the `//!` contract; implement it in `linux/` and `macos/` under the same name.
  4. Read it in `model.rs` through `Reads::read` or `Demand::read_if` so it lands in `Snapshot.sources` or `missing`.
  5. Declare `sources: Sources::of(&[Source::X])` on the drawing view's row in `views.rs`.
  6. Legend note through `status.rs` (`missing_note`, `sources_note`) and in the view.
  7. Demo synthesis plus a `DemoOptions` flag in `model.rs`; plain `demo` leaves it unread.
  8. A `model.rs` test of the demand path, and a root file-table row for any new `.rs` file.

## ANTI-PATTERNS
- "Every `unsafe` block is preceded by a comment that says why it is sound" (`// SAFETY:`).
- "A reading it cannot take ... is None in the `Cpu`, never 0" (`Sampler::cpus`).
- `open_events` "never escalates privilege"; it returns None and starts no thread when no source opens.
- `stop_journal` kills the whole process group, not only the child.
- Races with exiting processes are not permission gaps; `unreadable` does not count them.
- No FFI in `macos/logic.rs` or `macos/journal.rs`; no `render`, view or `terminal` imports here.
- `nvml.rs` never wakes a runtime-suspended GPU.

## COMMANDS
- `cargo test platform::` (includes `macos_logic` and `macos_journal` on Linux)
- `CARGO_TARGET_DIR=<own dir> cargo clippy --target aarch64-apple-darwin --all-targets -- -D warnings`
