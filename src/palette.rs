//! Every colour that means something about a process or a CPU core, in one place: the process-kind
//! bases, the process state colours, the core-kind colours and the district colours. Views read
//! them from here instead of spelling the triples again, so a colour scheme changes in one file.
//! Colours that only one view uses stay in that view.

use crate::model::{CoreKind, Kind, Process};
use crate::render::Color;
use crate::simulation::hash;

/// Kernel threads: slate.
pub(crate) const KERNEL: Color = [104, 122, 156];
/// System services: teal.
pub(crate) const SYSTEM: Color = [72, 183, 199];
/// The user's session: amber.
pub(crate) const SESSION: Color = [236, 178, 92];
/// Containers: violet.
pub(crate) const CONTAINER: Color = [178, 136, 232];

/// A zombie process.
pub(crate) const STATE_ZOMBIE: Color = [213, 97, 126];
/// A stopped or traced process.
pub(crate) const STATE_STOPPED: Color = [225, 161, 75];
/// A process in uninterruptible sleep.
pub(crate) const STATE_BLOCKED: Color = [222, 94, 73];

/// Performance cores share the session's amber, efficiency cores the system's teal.
pub(crate) const PERFORMANCE: Color = SESSION;
pub(crate) const EFFICIENCY: Color = SYSTEM;
pub(crate) const UNKNOWN_CORE: Color = [140, 160, 196];

/// One colour per city district, cycling.
pub(crate) const DISTRICTS: [Color; 8] = [
    [72, 183, 199],
    [122, 145, 229],
    [190, 135, 215],
    [211, 159, 91],
    [98, 185, 149],
    [212, 116, 140],
    [149, 171, 213],
    [166, 185, 104],
];

/// The stone a process of each kind is remembered in (Necropolis): slate for kernel threads, green
/// granite for system services, sandstone for the session and lilac marble for containers, so
/// the kinds read alike in moonlight without the bright kind colours.
pub(crate) fn stone_color(kind: Kind) -> Color {
    match kind {
        Kind::Kernel => [104, 112, 132],
        Kind::System => [112, 138, 134],
        Kind::Session => [166, 142, 108],
        Kind::Container => [134, 118, 160],
    }
}

/// Linear mix of two colours, `a` at 0 and `b` at 1, each channel truncated to a byte.
pub(crate) fn mix(a: Color, b: Color, t: f32) -> Color {
    [0, 1, 2].map(|k| (a[k] as f32 + (b[k] as f32 - a[k] as f32) * t) as u8)
}

/// A colour scaled in brightness by `scale`, clamped to the byte range.
pub(crate) fn tint(color: Color, scale: f32) -> Color {
    color.map(|v| (v as f32 * scale).clamp(0.0, 255.0) as u8)
}

/// The colour of a process whose state overrides its kind: zombie, stopped or blocked.
fn state_color(state: char) -> Option<Color> {
    match state {
        'Z' => Some(STATE_ZOMBIE),
        'T' | 't' => Some(STATE_STOPPED),
        'D' => Some(STATE_BLOCKED),
        _ => None,
    }
}

/// City colours: a state colour, or the district's.
pub(crate) fn process_color(process: &Process, group: usize) -> Color {
    state_color(process.state).unwrap_or(DISTRICTS[group % DISTRICTS.len()])
}

/// Orbit colours by role: slate kernel, teal system, amber session, violet containers, each
/// brightened or dimmed a little per pid so neighbours differ. State colours are not jittered.
pub(crate) fn kind_color(process: &Process) -> Color {
    if let Some(color) = state_color(process.state) {
        return color;
    }
    let base = match process.kind {
        Kind::Kernel => KERNEL,
        Kind::System => SYSTEM,
        Kind::Session => SESSION,
        Kind::Container => CONTAINER,
    };
    // pid only: kept for pixel stability, see A33
    let jitter = (hash::knuth(process.id.pid) >> 24) as f32 / 255.0;
    tint(base, 0.86 + 0.24 * jitter)
}

/// The colour of a CPU core of the given kind.
pub(crate) fn core_color(kind: CoreKind) -> Color {
    match kind {
        CoreKind::Performance => PERFORMANCE,
        CoreKind::Efficiency => EFFICIENCY,
        CoreKind::Unknown => UNKNOWN_CORE,
    }
}

/// The lower-case name of a core kind, as the views spell it.
pub(crate) fn core_name(kind: CoreKind) -> &'static str {
    match kind {
        CoreKind::Performance => "performance",
        CoreKind::Efficiency => "efficiency",
        CoreKind::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::testing::process;

    #[test]
    fn palette_kind_colours_match_their_previous_literals() {
        assert_eq!(KERNEL, [104, 122, 156]);
        assert_eq!(SYSTEM, [72, 183, 199]);
        assert_eq!(SESSION, [236, 178, 92]);
        assert_eq!(CONTAINER, [178, 136, 232]);
        assert_eq!(STATE_ZOMBIE, [213, 97, 126]);
        assert_eq!(STATE_STOPPED, [225, 161, 75]);
        assert_eq!(STATE_BLOCKED, [222, 94, 73]);
        assert_eq!(core_color(CoreKind::Performance), [236, 178, 92]);
        assert_eq!(core_color(CoreKind::Efficiency), [72, 183, 199]);
        assert_eq!(core_color(CoreKind::Unknown), [140, 160, 196]);
        assert_eq!(DISTRICTS[0], [72, 183, 199]);
        assert_eq!(DISTRICTS[7], [166, 185, 104]);
        for (kind, name) in [
            (CoreKind::Performance, "performance"),
            (CoreKind::Efficiency, "efficiency"),
            (CoreKind::Unknown, "unknown"),
        ] {
            assert_eq!(core_name(kind), name);
        }
        for (state, kind, pid, expected) in [
            ('S', Kind::Kernel, 1, [104, 123, 157]),
            ('S', Kind::Kernel, 2, [95, 111, 142]),
            ('S', Kind::Kernel, 1000, [90, 105, 135]),
            ('S', Kind::System, 1, [72, 184, 200]),
            ('S', Kind::Session, 3, [251, 189, 97]),
            ('S', Kind::Container, 3, [189, 144, 247]),
            ('Z', Kind::Session, 7, [213, 97, 126]),
            ('t', Kind::System, 7, [225, 161, 75]),
            ('D', Kind::Container, 7, [222, 94, 73]),
        ] {
            let sample = Process {
                state,
                kind,
                ..process(pid, 0)
            };
            assert_eq!(kind_color(&sample), expected, "{state} {kind:?} pid {pid}");
        }
        let blocked = Process {
            state: 'D',
            ..process(5, 0)
        };
        assert_eq!(process_color(&blocked, 3), [222, 94, 73]);
        let sleeping = process(5, 0);
        assert_eq!(process_color(&sleeping, 9), [122, 145, 229]);
    }

    #[test]
    fn mix_blends_each_channel_and_truncates() {
        assert_eq!(mix([0, 100, 255], [100, 0, 255], 0.0), [0, 100, 255]);
        assert_eq!(mix([0, 100, 255], [100, 0, 255], 1.0), [100, 0, 255]);
        assert_eq!(mix([0, 100, 255], [100, 0, 255], 0.55), [55, 45, 255]);
    }
}
