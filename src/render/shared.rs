//! Constants that the CPU rasterizer (`render.rs`) and the GPU shader (`shaders.wgsl`) must agree
//! on. Each one is declared once here; `prelude` writes the same values into the WGSL source, so
//! a pair cannot drift. Never write one of these numbers into `shaders.wgsl` directly.

use crate::glyphs;

/// A shared value in the form the shader prelude emits it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wgsl {
    U32(u32),
    F32(f32),
    Vec3([f32; 3]),
}

/// A WGSL float literal. `{:?}` prints the shortest text that parses back to the same `f32`, and
/// always includes a decimal point or an exponent, as WGSL requires.
fn float(value: f32) -> String {
    assert!(value.is_finite(), "WGSL has no literal for {value}");
    format!("{value:?}")
}

impl Wgsl {
    fn declaration(self, name: &str) -> String {
        match self {
            Self::U32(value) => format!("const {name}: u32 = {value}u;"),
            Self::F32(value) => format!("const {name}: f32 = {};", float(value)),
            Self::Vec3([x, y, z]) => format!(
                "const {name}: vec3<f32> = vec3<f32>({}, {}, {});",
                float(x),
                float(y),
                float(z)
            ),
        }
    }
}

/// Declares each constant in Rust and lists it in `CONSTANTS` for the prelude.
macro_rules! shared {
    ($($(#[$meta:meta])* $kind:ident $name:ident: $type:ty = $value:expr;)*) => {
        $($(#[$meta])* pub const $name: $type = $value;)*

        /// Every shared constant by name, in declaration order.
        pub const CONSTANTS: &[(&str, Wgsl)] = &[$((stringify!($name), Wgsl::$kind($name)),)*];
    };
}

shared! {
    /// Pick-buffer value for "nothing pickable here".
    U32 NONE: u32 = u32::MAX;

    /// Backdrop discriminants; the GPU receives the active one in `globals.sky.x`.
    U32 BACKDROP_DUSK: u32 = 0;
    U32 BACKDROP_SPACE: u32 = 1;
    U32 BACKDROP_SEA: u32 = 2;
    U32 BACKDROP_VOID: u32 = 3;
    U32 BACKDROP_TWILIGHT: u32 = 4;

    /// Sky colours in 0-255 units. Dusk and sea run a gradient: base plus (dusk) or minus (sea)
    /// slope times the screen height fraction.
    Vec3 SPACE_BASE: [f32; 3] = [5.0, 8.0, 18.0];
    Vec3 DUSK_BASE: [f32; 3] = [8.0, 13.0, 25.0];
    Vec3 DUSK_SLOPE: [f32; 3] = [6.0, 7.0, 8.0];
    Vec3 SEA_BASE: [f32; 3] = [24.0, 72.0, 96.0];
    Vec3 SEA_SLOPE: [f32; 3] = [18.0, 50.0, 58.0];

    /// Twilight runs three stops down the frame, in 0-255 units: the top colour at the top row,
    /// the middle colour `TWILIGHT_KNEE` of the way down, and the horizon colour at the bottom,
    /// each stop linear to the next.
    Vec3 TWILIGHT_TOP: [f32; 3] = [28.0, 34.0, 70.0];
    Vec3 TWILIGHT_MIDDLE: [f32; 3] = [120.0, 92.0, 138.0];
    Vec3 TWILIGHT_HORIZON: [f32; 3] = [238.0, 150.0, 104.0];
    F32 TWILIGHT_KNEE: f32 = 0.6;

    /// Haze density along one axis is 0.5 + 0.5 * sin(t * A + time * A_DRIFT + phase) *
    /// cos(t * B - time * B_DRIFT + phase * B_PHASE).
    F32 HAZE_FREQUENCY_A: f32 = 7.3;
    F32 HAZE_DRIFT_A: f32 = 0.11;
    F32 HAZE_FREQUENCY_B: f32 = 2.9;
    F32 HAZE_DRIFT_B: f32 = 0.07;
    F32 HAZE_PHASE_B: f32 = 1.7;

    /// Sphere light: ambient plus the dot of the surface normal with (-X, -Y, Z), floored at
    /// zero, capped at the maximum.
    F32 SPHERE_AMBIENT: f32 = 0.25;
    F32 SPHERE_LIGHT_X: f32 = 0.4;
    F32 SPHERE_LIGHT_Y: f32 = 0.5;
    F32 SPHERE_LIGHT_Z: f32 = 0.65;
    F32 SPHERE_LIGHT_MAX: f32 = 1.2;

    /// How far past the radius a sphere's outline reaches in pixels, unselected and selected.
    F32 SPHERE_EDGE: f32 = 1.0;
    F32 SELECTION_RING_WIDTH: f32 = 3.0;
    /// Selection ring colour in 0-255 units.
    Vec3 SELECTION_COLOR: [f32; 3] = [237.0, 220.0, 154.0];

    /// Added to a line's depth so it draws over the surface it lies on.
    F32 LINE_DEPTH_BIAS: f32 = 0.02;

    /// Glyph cell size in font pixels.
    F32 GLYPH_WIDTH: f32 = glyphs::WIDTH as f32;
    F32 GLYPH_HEIGHT: f32 = glyphs::HEIGHT as f32;
    U32 GLYPH_COLUMNS: u32 = glyphs::WIDTH;
}

/// The one light every lit surface drawn on the CPU shares (tubes, and the relief of a field): a
/// unit vector in world space, z up, pointing from the surface towards the light, the normalized
/// `[-0.4, -0.5, 0.75]`. It is shaded in Rust, so the shader never sees it and it is not in
/// `CONSTANTS`; a surface is `0.35 + 0.65 * max(normal . LIGHT, 0)` bright.
pub const LIGHT: [f32; 3] = [-0.405_616_1, -0.507_020_1, 0.760_530_2];

/// The WGSL declarations of every shared constant, placed before `shaders.wgsl`.
pub fn prelude() -> String {
    let mut source = String::from("// Generated from render::shared; do not edit.\n");
    for (name, value) in CONSTANTS {
        source.push_str(&value.declaration(name));
        source.push('\n');
    }
    source
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prelude_declares_every_shared_constant_once() {
        let prelude = prelude();
        for (name, _) in CONSTANTS {
            let declarations = prelude
                .lines()
                .filter(|line| line.starts_with(&format!("const {name}:")))
                .count();
            assert_eq!(declarations, 1, "{name} must be declared exactly once");
        }
        assert_eq!(prelude.lines().count(), CONSTANTS.len() + 1);
    }

    #[test]
    fn prelude_emits_exact_literals() {
        let prelude = prelude();
        assert!(prelude.contains("const NONE: u32 = 4294967295u;\n"));
        assert!(prelude.contains("const HAZE_FREQUENCY_A: f32 = 7.3;\n"));
        assert!(prelude.contains("const SPACE_BASE: vec3<f32> = vec3<f32>(5.0, 8.0, 18.0);\n"));
        assert_eq!(float(1.0), "1.0");
        assert_eq!(float(-0.0), "-0.0");
        assert_eq!(float(1.0e-7), "1e-7");
        assert_eq!(float(3.0e10), "30000000000.0");
        assert_eq!(float(1.0e16), "1e16");
    }

    #[test]
    fn emitted_floats_parse_back_to_the_same_bits() {
        for value in [0.1_f32, 7.3, 0.02, 1.0 / 3.0, 1e-30, 16777217.0, f32::MAX] {
            assert_eq!(
                float(value).parse::<f32>().unwrap().to_bits(),
                value.to_bits()
            );
        }
    }

    #[test]
    fn shader_source_does_not_repeat_shared_literals() {
        let source = include_str!("../shaders.wgsl");
        for literal in ["0.65", "7.3", "4294967295", "0xffffffff", "237.0", "0.25"] {
            assert!(
                !source.contains(literal),
                "shaders.wgsl must use the shared constant instead of {literal}"
            );
        }
    }
}
