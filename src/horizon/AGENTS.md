# src/horizon

## OVERVIEW
Submodules of the Horizon view (the OOM killer as a Schwarzschild black hole), declared from `../horizon.rs`.
Own file because it holds relativistic physics ported from a Python prototype and the heaviest triangle generator.

## WHERE TO LOOK
| Task | File | Symbol / notes |
|---|---|---|
| Lensing and photon rays | `lens.rs` | `LensTable` (RK4, built on first use), `Observer` |
| Emission and orbital physics | `lens.rs` | `flux` (Page-Thorne), `one_plus_z`, `disk_emission`, `angular_velocity`, `period` |
| Plunge of the victim | `lens.rs` | `PlungeTrack`, `plunge_brightness` |
| Orbit ranking and ghosts | `orbits.rs` | `Ranking`, `Orbits`, `Ghost::pose`, `rank_radius`, `band_radius` |
| Lensed disk mesh | `disk.rs` | primary and secondary image, `Turbulence`, `SkyField`, `draw_shadow`, `draw_ring`, `draw_glare`, `layer` |
| View glue, roster, kill counter | `../horizon.rs` | `Horizon`, `Roster`, `kill_counter`, `ViewModule` impl |
| The prototype being ported | `../../docs/mockups/code/horizon.py` | read, never run as truth |

## CONVENTIONS
- Units are G = c = M = 1: horizon at r = 2, photon sphere at 3, ISCO at 6, shadow radius 3 sqrt(3).
- `lens.rs` ports functions of `docs/mockups/code/horizon.py`; each is tested against analytic values. A changed constant
  changes the prototype comment, the analytic test and `docs/theory/horizon.md` together.
- Only `../horizon.rs` touches `Stage`. `disk.rs` draws into a `Frame` it is handed (imports `Frame`, `Item`, `Sky`);
  `lens.rs` and `orbits.rs` are pure and import no `Frame` or `Stage`.
- Rank 1 sits on the ISCO; `oom_score_adj` -1000 sits in a band far out; memory footprint ranks where no score exists.
- A kill is a rise of the kill counter together with a vanished process; the best-ranked ghost then plunges.
- The demo kill cycle is the row's `DemoOptions::oom_cycle`.

## ANTI-PATTERNS
- Do not change `lens.rs` numbers without rechecking the prototype and the analytic tests.
- Lensed disk mesh is capped at 120k triangles with cells of about 4 px; do not raise either to buy detail.
- Do not re-lens every frame: lens again only when inclination or scale changes, recolour every frame.
- Do not add drawing to `lens.rs` or `orbits.rs`, and no `Stage` use outside `../horizon.rs`.
- Do not draw a second temperature-to-colour table; use `blackbody::color`.

## COMMANDS
- `cargo test horizon`
- `./target/release/isotop --demo --view horizon --renderer cpu --width 960 --time 12 --output /tmp/horizon.png`
