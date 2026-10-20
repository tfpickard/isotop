//! Test helpers shared by the tests of the coop's modules.

use std::f32::consts::TAU;

use crate::model::Measured;
use crate::platform::DeletedFile;
use crate::render::{Camera, RenderInput, Scene};
use crate::simulation::Rng;
use crate::views::view;

use super::sim::{Chicken, Yard, speed};
use super::*;

pub(super) fn identity(pid: u32) -> Identity {
    Identity {
        pid,
        start: pid as u64,
    }
}

pub(super) fn process(pid: u32, cgroup: &str) -> Process {
    Process {
        name: format!("hen-{pid}"),
        group: cgroup.rsplit('/').next().unwrap_or("").into(),
        cpu: 5.0,
        memory: 20 << 20,
        io_rate: Some(0.0),
        read_rate: Some(0.0),
        write_rate: Some(0.0),
        written: Some(0),
        cgroup: cgroup.into(),
        files: Measured::Known(crate::platform::Files::default()),
        locks_held: Measured::Known(0),
        ..crate::model::testing::process(pid, 1)
    }
}

pub(super) fn snapshot(processes: Vec<Process>, elapsed: f64) -> Snapshot {
    Snapshot {
        processes,
        cores: 2,
        elapsed,
        ..Snapshot::default()
    }
}

pub(super) fn forager(
    pid: u32,
    flock: usize,
    position: [f32; 2],
    theta: f32,
    noise: f32,
) -> Chicken {
    let mut chicken = Chicken::new(
        identity(pid),
        flock,
        position,
        theta,
        Rng::for_identity(identity(pid), 1),
    );
    chicken.noise = noise;
    chicken.speed = speed(0.0);
    chicken.radius = 0.3;
    chicken
}

/// A flock of `count` foragers scattered over a square of side `spread` around `center`.
pub(super) fn scatter(
    count: u32,
    first: u32,
    flock: usize,
    center: [f32; 2],
    spread: f32,
    noise: f32,
) -> Vec<Chicken> {
    let mut rng = Rng::new(first as u64 * 7 + 3);
    (0..count)
        .map(|k| {
            let position = [
                center[0] + rng.signed() * spread * 0.5,
                center[1] + rng.signed() * spread * 0.5,
            ];
            forager(first + k, flock, position, rng.unit() * TAU, noise)
        })
        .collect()
}

pub(super) fn state(yard: &Yard) -> Vec<(Identity, [u32; 2], u32)> {
    let mut state: Vec<_> = yard
        .chickens
        .iter()
        .map(|c| {
            (
                c.id,
                [c.position[0].to_bits(), c.position[1].to_bits()],
                c.theta.to_bits(),
            )
        })
        .collect();
    state.sort();
    state
}

/// The coop's own state, read through the trait from the scene that draws it.
pub(super) fn coop_of(scene: &Scene) -> &Coop {
    scene
        .module::<Coop>(view("coop"))
        .expect("the coop has been built")
}

pub(super) fn legend_of(scene: &Scene) -> String {
    scene.legend(view("coop"), &Snapshot::default())
}

pub(super) fn render(scene: &mut Scene, snapshot: &Snapshot, time: f32) {
    let frame = scene.render(&RenderInput {
        width: 160,
        height: 90,
        limit: 4096,
        ..RenderInput::small(snapshot, view("coop"), &Camera::default(), time)
    });
    scene.spare = frame.release();
}

/// The notes of a drawn process, without the flock line and the nest line around them.
pub(super) fn notes(scene: &Scene, pid: u32) -> Vec<String> {
    let notes = &scene.notes[&identity(pid)];
    notes[1..notes.len() - 1].to_vec()
}

pub(super) fn nest_line(scene: &Scene, pid: u32) -> String {
    scene.notes[&identity(pid)].last().unwrap().clone()
}

/// A process holding `open` distinct regular files, `deleted` of which are the given files.
pub(super) fn holding(pid: u32, cgroup: &str, open: u32, deleted: &[DeletedFile]) -> Process {
    let mut process = process(pid, cgroup);
    process.files = Measured::Known(crate::platform::Files {
        open,
        deleted: deleted.to_vec(),
        partial: false,
    });
    process
}

pub(super) fn deleted_file(inode: u64, size: u64) -> DeletedFile {
    DeletedFile {
        device: 2049,
        inode,
        size,
    }
}
