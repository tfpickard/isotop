# src/views

## OVERVIEW
Orbit, Flow, Ripple and City modules plus the shared `systems.rs`, declared privately from `views.rs` (which holds
`REGISTRY`). Own file because it departs from the "new view lives in `src/<name>.rs`" rule and shares layout code.

## WHERE TO LOOK
| Task | File | Symbol / notes |
|---|---|---|
| Add or edit a registry row | `../views.rs` | `REGISTRY`, `ViewInfo`, `ViewInfo::new` |
| The module trait | `../views.rs` | `ViewModule` (defaults: `configure`, `record`, `key`, `inspector_line`, `card`, `state`, `anchors`) |
| Harness contract | `../views.rs` | `Contract`, `Property`, `Unstable` |
| Warm-up phases | `../views.rs` | `Warmup`, `Phase`, `When`, `SETTLE` |
| View-local keys | `../keys.rs`, `../views.rs` | `LOCAL_KEYS` in `keys.rs`, `ViewKey` in `views.rs` |
| Test fixtures for modules | `../views.rs` | `pub mod testing` (`Scene::with_rows` tables) |
| Nested Keplerian systems | `systems.rs` | `layout`, `Galaxy`, `Tree`, `hubs`; shared by Orbit and Flow |
| Orbit, Flow | `orbit.rs`, `flow.rs` | `Orbits`, `Wells` (each owns its own `Galaxy`) |
| Ripple | `ripple.rs` | `Ripple`, `Cluster`, `territories` |
| City | `city.rs` | `City`, `Plot`, `district_origin` |

## CONVENTIONS
- Modules here are declared `mod city; mod flow; mod orbit; mod ripple; pub(crate) mod systems;` in `views.rs`, not `main.rs`.
  `systems.rs` is layout code, not a view: no row, no `ViewModule`.
- Rows use struct-update style and name only the options they turn on:
  `ViewInfo { links: LinkRule::All, ..ViewInfo::new(name, title, backdrop, || Box::new(..)) }`.
- `keys::LOCAL_KEYS` is `"bijkmopuxyz,.;"`; every other key is global and none reaches a view while a search is open.
- A new view still gets a root file-table row; `src/views/` files are listed there as `views/<name>.rs`.
- Orbit and Flow share layout code but never state: switching views must not show wells where Orbit turned its systems.
- Body placement inside a system is not yet stable under memory and population changes (issue 142).
- `testing` holds one non-ASCII string (`GUIDE_SPARKLINE`, braille); do not copy that habit into prose.

## ANTI-PATTERNS
- "Never reorder the first ten rows: the digits pick them."
- "Never add a `known_unstable` entry for a new view."
- A view whose bodies move on their own must return its places from `ViewModule::anchors`.
- No per-view field, branch or arm on `Scene` (`scene_has_no_view_specific_fields`).
- A view reads only `Stage`, the process slice and `&Snapshot`; no `/proc`, files, sockets or terminal.

## COMMANDS
- `cargo test every_view_satisfies_the_view_contract`
- `cargo test views::`
- `ISOTOP_BLESS=1 cargo test every_view_matches` after a deliberate change to a render (say so in the PR).
