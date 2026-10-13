//! Stellar photometry shared by every view that draws processes as stars: memory becomes an
//! astronomical magnitude, a magnitude becomes a point-spread star (a sharp core inside a soft
//! halo), and births and exits become flash timelines. The constants follow `star` and the
//! supernova block of `docs/mockups/code/hr.py`.
//!
//! Everything here depends only on its arguments; a caller keeps its own birth and exit times
//! and passes the age.

use crate::palette::{mix, tint};
use crate::render::{Camera, Color, Frame, Item, Point};

const GIBIBYTE: u64 = 1 << 30;
const MEBIBYTE: u64 = 1 << 20;

/// A star's flux is 1 at this magnitude, the faintest the prototype draws as a visible star.
const FLUX_ONE_MAGNITUDE: f32 = 7.0;
/// Frame height at which one spread pixel is one screen pixel, the prototype's `sc = 1`.
const REFERENCE_HEIGHT: f32 = 720.0;
const CORE_LIMITS: (f32, f32) = (0.8, 6.0);
const HALO_RADIUS_LIMIT: f32 = 6.0;
const HALO_STRENGTH_LIMIT: f32 = 0.95;
/// Halos fade out between these magnitudes, so a star never pops a halo on at one magnitude.
const HALO_FADE: (f32, f32) = (5.5, 6.0);
const CORE_STRENGTH_LIMIT: f32 = 2.2;
/// How far the core colour is mixed toward white, as in the prototype.
const CORE_WHITENING: f32 = 0.6;

const NOVA_RISE_SECONDS: f32 = 0.3;
const NOVA_RISE_MAGNITUDES: f32 = 4.0;
const NOVA_DECAY_SECONDS: f32 = 2.0;
const NOVA_DURATION_SECONDS: f32 = 6.0;
const FADE_SECONDS: f32 = 1.5;
const SUPERNOVA_FLASH_SECONDS: f32 = 0.5;
const SUPERNOVA_FLASH_MAGNITUDES: f32 = 6.0;
const SHELL_EASE_SECONDS: f32 = 1.1;
const SHELL_FADE_SECONDS: f32 = 1.6;
const SHELL_WARM_SECONDS: f32 = 1.5;
const SUPERNOVA_DURATION_SECONDS: f32 = 8.0;

/// The apparent magnitude of a process holding `memory` bytes: `-2.5 log10(memory / 1 GiB)`, so
/// 1 GiB is 0 and 10 MiB is about +5. Memory is floored at 1 MiB (+7.5) so rows with none, such
/// as kernel threads, get a finite and faint magnitude. A smaller number is brighter.
pub fn magnitude(memory: u64) -> f32 {
    let ratio = memory.max(MEBIBYTE) as f64 / GIBIBYTE as f64;
    (-2.5 * ratio.log10()) as f32
}

/// The size and strength of a star's two layers, in pixels of a frame of a given height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    /// Radius of the sharp core in pixels.
    pub core_px: f32,
    /// Core brightness: the core colour is scaled by it, so above 1 it saturates toward white.
    pub core: f32,
    /// Radius of the soft halo in pixels; 0 when the star has none.
    pub halo_px: f32,
    /// Strength of the halo, 0 to 0.95; 0 when the star has none.
    pub halo: f32,
}

/// The point-spread of a star of `magnitude` in a frame `frame_height` pixels high. With flux
/// `L = 10^(-0.4 (magnitude - 7))`, the core and halo radii grow as about `L^0.2` and the halo
/// strength as `L^0.42`. Sizes scale with the frame height, and a brighter star is never
/// smaller or weaker than a fainter one.
pub fn spread(magnitude: f32, frame_height: u32) -> Spread {
    let scale = frame_height as f32 / REFERENCE_HEIGHT;
    let flux = 10.0_f32.powf(-0.4 * (magnitude - FLUX_ONE_MAGNITUDE));

    let core_px = ((0.92 + 0.36 * flux.powf(0.2)) * scale)
        .clamp(CORE_LIMITS.0 * scale, CORE_LIMITS.1 * scale);
    let core = (0.5 + 0.24 * flux.powf(0.3)).min(CORE_STRENGTH_LIMIT);

    let fade = ((HALO_FADE.1 - magnitude) / (HALO_FADE.1 - HALO_FADE.0)).clamp(0.0, 1.0);
    let halo = (0.07 * flux.powf(0.42)).min(HALO_STRENGTH_LIMIT) * fade;
    let halo_px = if halo > 0.0 {
        ((1.9 + 2.9 * flux.powf(0.21)) * scale).min(HALO_RADIUS_LIMIT * scale)
    } else {
        0.0
    };
    Spread {
        core_px,
        core,
        halo_px,
        halo,
    }
}

/// How the soft layer of a star blends into the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halo {
    /// An `Item::Glow`, which the CPU blends with truncation and the GPU with rounding, so
    /// halos that overlap drift apart by a level or two between the rasterizers.
    Glow,
    /// An `Item::Puff`, which rounds on both: for a sky of many stars, whose halos overlap.
    Puff,
}

impl Frame {
    /// Draws a star at `position`: one pickable sphere sized in pixels, so zoom does not change
    /// it, with a whitened and brightened `color`, and one glow in `color` when the spread has a
    /// halo.
    pub(crate) fn star(
        &mut self,
        camera: &Camera,
        position: Point,
        spread: &Spread,
        color: Color,
        pick: u32,
        selected: bool,
    ) {
        self.star_with(camera, position, spread, color, pick, selected, Halo::Glow);
    }

    /// `star` with the halo blended as `halo` says.
    #[expect(
        clippy::too_many_arguments,
        reason = "a star's place, size, colour, pick and halo"
    )]
    pub(crate) fn star_with(
        &mut self,
        camera: &Camera,
        position: Point,
        spread: &Spread,
        color: Color,
        pick: u32,
        selected: bool,
        halo: Halo,
    ) {
        let center = self.project(camera, position);
        self.items.push(Item::Sphere {
            center,
            radius: spread.core_px,
            depth: spread.core_px / camera.zoom,
            color: tint(mix(color, [255; 3], CORE_WHITENING), spread.core),
            pick,
            selected,
        });
        if spread.halo > 0.0 {
            let center = [center[0], center[1]];
            self.items.push(match halo {
                Halo::Glow => Item::Glow {
                    center,
                    radius: spread.halo_px,
                    color,
                    strength: spread.halo,
                },
                Halo::Puff => Item::Puff {
                    center,
                    radius: spread.halo_px,
                    color,
                    strength: spread.halo,
                },
            });
        }
    }
}

/// The kind of event a flash timeline plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flash {
    /// A birth: the star brightens sharply, then settles.
    Nova,
    /// An ordinary exit: the star dims out.
    Fade,
    /// The exit of a large process: a flash, then an expanding shell where the star was.
    Supernova,
}

/// The flash for a process that exits holding `memory` bytes: a supernova above 1 GiB, a fade
/// otherwise.
pub fn exit_flash(memory: u64) -> Flash {
    if memory > GIBIBYTE {
        Flash::Supernova
    } else {
        Flash::Fade
    }
}

/// One instant of a flash.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlashState {
    /// Magnitudes to add to the star's own; negative is brighter.
    pub magnitude_offset: f32,
    /// How much of the star remains, 1 to 0; a star at 0 is not drawn.
    pub opacity: f32,
    /// The supernova's expanding shell.
    pub shell: Option<Shell>,
}

/// A supernova's shell. The caller chooses the world or pixel scale for `radius`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shell {
    /// Fraction of the final radius, 0 to 1.
    pub radius: f32,
    /// Opacity, 1 down to 0.
    pub alpha: f32,
    /// 0 is blue-white and 1 is red.
    pub warmth: f32,
}

/// The state of a flash `age_seconds` after it began, or `None` once it is over. A negative age
/// counts as 0.
///
/// A nova brightens by 4 magnitudes over 0.3 s, then decays with an e-folding time of 2 s, and
/// is over after 6 s. A fade loses opacity linearly over 1.5 s. A supernova flashes to -6
/// magnitudes in 0.5 s and then the star is gone, while its shell grows as `1 - e^(-t / 1.1)`,
/// fades as `e^(-t / 1.6)` and is fully red at 1.5 s; it is over after 8 s.
pub fn flash(kind: Flash, age_seconds: f32) -> Option<FlashState> {
    let age = age_seconds.max(0.0);
    match kind {
        Flash::Nova if age <= NOVA_DURATION_SECONDS => {
            let magnitude_offset = if age < NOVA_RISE_SECONDS {
                -NOVA_RISE_MAGNITUDES * age / NOVA_RISE_SECONDS
            } else {
                -NOVA_RISE_MAGNITUDES * (-(age - NOVA_RISE_SECONDS) / NOVA_DECAY_SECONDS).exp()
            };
            Some(FlashState {
                magnitude_offset,
                opacity: 1.0,
                shell: None,
            })
        }
        Flash::Fade if age <= FADE_SECONDS => Some(FlashState {
            magnitude_offset: 0.0,
            opacity: 1.0 - age / FADE_SECONDS,
            shell: None,
        }),
        Flash::Supernova if age <= SUPERNOVA_DURATION_SECONDS => {
            let flashing = age < SUPERNOVA_FLASH_SECONDS;
            Some(FlashState {
                magnitude_offset: if flashing {
                    -SUPERNOVA_FLASH_MAGNITUDES * age / SUPERNOVA_FLASH_SECONDS
                } else {
                    0.0
                },
                opacity: if flashing { 1.0 } else { 0.0 },
                shell: Some(Shell {
                    radius: 1.0 - (-age / SHELL_EASE_SECONDS).exp(),
                    alpha: (-age / SHELL_FADE_SECONDS).exp(),
                    warmth: (age / SHELL_WARM_SECONDS).min(1.0),
                }),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{Backdrop, Buffers, NONE, Sky};

    fn frame() -> Frame {
        let sky = Sky {
            backdrop: Backdrop::Void,
            haze: [0.0; 3],
            time: 0.0,
        };
        Frame::new(960, 540, sky, Buffers::default())
    }

    fn magnitudes() -> impl Iterator<Item = f32> {
        (-40..=40).map(|step| step as f32 * 0.25)
    }

    #[test]
    fn one_gibibyte_is_magnitude_zero_and_ten_mebibytes_is_five() {
        assert!(magnitude(GIBIBYTE).abs() < 1e-6);
        assert!((magnitude(10 * MEBIBYTE) - 5.0).abs() < 0.05);
        assert!(magnitude(16 * GIBIBYTE) < -3.0);
    }

    #[test]
    fn memory_below_one_mebibyte_is_floored() {
        let floor = magnitude(MEBIBYTE);
        assert!((floor - 7.5).abs() < 0.05);
        assert_eq!(magnitude(0), floor);
        assert_eq!(magnitude(MEBIBYTE - 1), floor);
        assert!(magnitude(MEBIBYTE + 1024 * 1024) < floor);
    }

    #[test]
    fn brighter_stars_are_never_smaller() {
        let mut previous = spread(-10.0, 720);
        for magnitude in magnitudes().skip(1) {
            let next = spread(magnitude, 720);
            assert!(next.core_px <= previous.core_px, "core_px at {magnitude}");
            assert!(next.core <= previous.core, "core at {magnitude}");
            assert!(next.halo_px <= previous.halo_px, "halo_px at {magnitude}");
            assert!(next.halo <= previous.halo, "halo at {magnitude}");
            previous = next;
        }
    }

    #[test]
    fn halos_are_capped_and_absent_for_faint_stars() {
        for height in [360, 720, 1440] {
            let scale = height as f32 / 720.0;
            for magnitude in magnitudes() {
                let star = spread(magnitude, height);
                assert!(star.halo_px <= HALO_RADIUS_LIMIT * scale + 1e-4);
                assert!(star.halo <= HALO_STRENGTH_LIMIT);
                assert!(star.core_px >= CORE_LIMITS.0 * scale - 1e-4);
                assert!(star.core_px <= CORE_LIMITS.1 * scale + 1e-4);
                if magnitude >= 6.0 {
                    assert_eq!((star.halo_px, star.halo), (0.0, 0.0), "at {magnitude}");
                }
            }
        }
        let bright = spread(0.0, 720);
        assert!(bright.halo > 0.0 && bright.halo_px > 0.0);
        assert_eq!(spread(7.5, 720).halo, 0.0);
    }

    #[test]
    fn spread_scales_with_frame_height() {
        for magnitude in [-2.0, 0.0, 4.0, 5.5, 7.5] {
            let base = spread(magnitude, 720);
            let double = spread(magnitude, 1440);
            assert!((double.core_px - 2.0 * base.core_px).abs() < 1e-4);
            assert!((double.halo_px - 2.0 * base.halo_px).abs() < 1e-4);
            assert_eq!((double.core, double.halo), (base.core, base.halo));
        }
    }

    #[test]
    fn a_star_pushes_one_pickable_sphere_and_at_most_one_glow() {
        let mut frame = frame();
        let mut camera = Camera::default();
        let position = [1.0, 2.0, 0.0];
        for (magnitude, glows) in [(0.0, 1), (7.5, 0)] {
            frame.items.clear();
            let star = spread(magnitude, 540);
            frame.star(&camera, position, &star, [200, 220, 255], 7, true);
            let spheres: Vec<_> = frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Sphere { .. }))
                .collect();
            assert_eq!(spheres.len(), 1);
            assert!(matches!(
                spheres[0],
                Item::Sphere { radius, pick: 7, selected: true, .. } if *radius == star.core_px
            ));
            let found = frame
                .items
                .iter()
                .filter(|item| matches!(item, Item::Glow { .. }))
                .count();
            assert_eq!(found, glows);
            assert_eq!(frame.items.len(), 1 + glows);
        }
        // Pixel sizes do not follow the zoom, and an unpickable star stays unpickable.
        let star = spread(0.0, 540);
        frame.items.clear();
        camera.zoom *= 4.0;
        frame.star(&camera, position, &star, [255; 3], NONE, false);
        assert!(matches!(
            frame.items[0],
            Item::Sphere { radius, pick: NONE, .. } if radius == star.core_px
        ));
    }

    #[test]
    fn a_puffed_star_draws_its_halo_as_a_puff_and_the_core_as_before() {
        let mut frame = frame();
        let camera = Camera::default();
        let star = spread(0.0, 540);
        let position = [1.0, 2.0, 0.0];
        frame.star(&camera, position, &star, [200, 220, 255], 3, false);
        let glow = frame.items.clone();
        frame.items.clear();
        frame.star_with(
            &camera,
            position,
            &star,
            [200, 220, 255],
            3,
            false,
            Halo::Puff,
        );
        assert_eq!(frame.items.len(), 2);
        match (glow[0], frame.items[0]) {
            (
                Item::Sphere {
                    center: a,
                    color: x,
                    ..
                },
                Item::Sphere {
                    center: b,
                    color: y,
                    ..
                },
            ) => {
                assert_eq!((a, x), (b, y), "the core is the same");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(glow[1], Item::Glow { .. }));
        match (glow[1], frame.items[1]) {
            (
                Item::Glow {
                    center: a,
                    radius: r,
                    color: c,
                    strength: s,
                },
                Item::Puff {
                    center: b,
                    radius: q,
                    color: d,
                    strength: t,
                },
            ) => assert_eq!(
                (a, r, c, s),
                (b, q, d, t),
                "the halo is the same, blended alike"
            ),
            other => panic!("{other:?}"),
        }
        // A faint star has no halo to blend.
        frame.items.clear();
        frame.star_with(
            &camera,
            position,
            &spread(7.5, 540),
            [255; 3],
            3,
            false,
            Halo::Puff,
        );
        assert_eq!(frame.items.len(), 1);
    }

    #[test]
    fn a_nova_peaks_early_and_is_gone_after_six_seconds() {
        let offset = |age| flash(Flash::Nova, age).unwrap().magnitude_offset;
        assert_eq!(offset(0.0), 0.0);
        assert!((offset(0.3) + 4.0).abs() < 1e-6);
        let peak = (0..=600)
            .map(|step| offset(step as f32 * 0.01))
            .fold(f32::MAX, f32::min);
        assert!((peak + 4.0).abs() < 1e-6);
        assert!(offset(0.15) > offset(0.3) && offset(2.3) > offset(0.3));
        // An e-folding time of two seconds.
        assert!((offset(2.3) / offset(0.3) - (-1.0_f32).exp()).abs() < 1e-5);
        assert!(flash(Flash::Nova, 6.0).is_some());
        assert!(flash(Flash::Nova, 6.01).is_none());
        assert_eq!(flash(Flash::Nova, -1.0), flash(Flash::Nova, 0.0));
        assert!(flash(Flash::Nova, 1.0).unwrap().shell.is_none());
    }

    #[test]
    fn a_fade_reaches_zero_opacity_at_one_and_a_half_seconds() {
        let opacity = |age| flash(Flash::Fade, age).unwrap().opacity;
        assert_eq!(opacity(0.0), 1.0);
        assert!((opacity(0.75) - 0.5).abs() < 1e-6);
        assert_eq!(opacity(1.5), 0.0);
        assert!(flash(Flash::Fade, 1.51).is_none());
    }

    #[test]
    fn only_exits_above_one_gibibyte_go_supernova() {
        assert_eq!(exit_flash(0), Flash::Fade);
        assert_eq!(exit_flash(GIBIBYTE), Flash::Fade);
        assert_eq!(exit_flash(GIBIBYTE + 1), Flash::Supernova);
        assert_eq!(exit_flash(64 * GIBIBYTE), Flash::Supernova);
    }

    #[test]
    fn a_supernova_shell_grows_monotonically_and_fades() {
        let flash_at = |age| flash(Flash::Supernova, age).unwrap();
        let flashing = flash_at(0.49);
        assert!(flashing.magnitude_offset < -5.5 && flashing.opacity == 1.0);
        assert_eq!(flash_at(0.5).opacity, 0.0);
        assert_eq!(flash_at(3.0).opacity, 0.0);

        let mut previous = flash_at(0.0).shell.unwrap();
        assert_eq!(previous.radius, 0.0);
        assert_eq!((previous.alpha, previous.warmth), (1.0, 0.0));
        for step in 1..=160 {
            let shell = flash_at(step as f32 * 0.05).shell.unwrap();
            assert!(shell.radius >= previous.radius);
            assert!(shell.alpha <= previous.alpha);
            assert!(shell.warmth >= previous.warmth);
            assert!((0.0..=1.0).contains(&shell.radius));
            previous = shell;
        }
        assert!((flash_at(1.1).shell.unwrap().radius - (1.0 - (-1.0_f32).exp())).abs() < 1e-6);
        assert_eq!(flash_at(1.5).shell.unwrap().warmth, 1.0);
        assert!(flash_at(8.0).shell.unwrap().alpha < 0.01);
        assert!(flash(Flash::Supernova, 8.01).is_none());
    }
}
