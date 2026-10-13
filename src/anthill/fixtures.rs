//! Test only: processes that hold files, snapshots of them and a scene to draw them in, which the
//! tests of the `anthill` files share.

use std::sync::Arc;

use super::Anthill;
use crate::io_detail::{FileFlow, Flow, IoDetail, Unattributed};
use crate::model::{Identity, Measured, Process, Snapshot, SyscallIo, testing};
use crate::platform::AccessMode;
use crate::render::{Backdrop, Camera, Frame, RenderInput, Scene};
use crate::simulation::hash;
use crate::views::{LinkRule, View, ViewInfo};

/// One module view, the anthill, as a scene's whole table.
pub static ROWS: &[ViewInfo] = &[ViewInfo {
    links: LinkRule::DrawnByView,
    ..ViewInfo::new("anthill", "ANTHILL", Backdrop::Dusk, || {
        Box::new(Anthill::default())
    })
}];

pub fn id(pid: u32) -> Identity {
    Identity {
        pid,
        start: u64::from(pid),
    }
}

/// A process of `mebibytes` MiB with nothing measured of where its I/O goes.
pub fn process(pid: u32, mebibytes: u64) -> Process {
    Process {
        memory: mebibytes << 20,
        ..testing::process(pid, 1)
    }
}

/// `process` with the calls per second and bytes per second of its reads and writes.
pub fn counting(
    process: Process,
    reads: f32,
    read_size: f32,
    writes: f32,
    write_size: f32,
) -> Process {
    Process {
        syscall_io: Some(SyscallIo {
            reads,
            writes,
            read_chars: reads * read_size,
            write_chars: writes * write_size,
        }),
        ..process
    }
}

/// A regular file of 1 MiB at `path`, on device 7, whose inode its path hashes to.
pub fn file(path: &str, access: AccessMode, flow: Flow) -> FileFlow {
    FileFlow {
        device: 7,
        inode: hash::fnv1a_64(path) >> 8,
        size: 1 << 20,
        path: path.to_owned(),
        deleted: false,
        access,
        flow,
    }
}

/// What a file's positions advanced by: `read` and `written` bytes per second, nothing through a
/// read-write descriptor.
pub fn moved(read: f32, written: f32) -> Flow {
    Flow::Measured {
        read,
        written,
        either: 0.0,
    }
}

/// What no file position explained of a process's bytes.
pub fn left(read: f32, write: f32, either: f32) -> Unattributed {
    Unattributed {
        read,
        write,
        either,
    }
}

/// `process` with its descriptors read: `files`, what they leave, and `ttys` terminals.
pub fn holding(
    process: Process,
    files: Vec<FileFlow>,
    unattributed: Option<Unattributed>,
    ttys: u32,
) -> Process {
    Process {
        io_detail: Measured::Known(Arc::new(IoDetail {
            files,
            ttys,
            unattributed,
            ..IoDetail::default()
        })),
        ..process
    }
}

/// A sample of `processes`, taken `elapsed` seconds in, on a platform that counts calls.
pub fn snapshot(processes: Vec<Process>, elapsed: f64) -> Snapshot {
    Snapshot {
        processes,
        elapsed,
        ..Snapshot::default()
    }
}

/// The processes of `snapshot` as a view is handed them.
pub fn drawn(snapshot: &Snapshot) -> Vec<&Process> {
    snapshot.processes.iter().collect()
}

/// The scene a test draws the anthill in.
pub fn scene() -> Scene {
    Scene::with_rows(ROWS)
}

/// Draws `snapshot` in `scene` at each time from `from` to `to` in steps of `step`, and returns
/// the last frame.
pub fn play(scene: &mut Scene, snapshot: &Snapshot, from: f32, to: f32, step: f32) -> Frame {
    let camera = Camera::default();
    let mut time = from;
    loop {
        let input = RenderInput::small(snapshot, View::at(0), &camera, time);
        let frame = scene.render(&input);
        if time >= to - step * 0.5 {
            return frame;
        }
        time += step;
    }
}

/// The anthill's state in `scene`.
pub fn state(scene: &Scene) -> &Anthill {
    scene
        .module::<Anthill>(View::at(0))
        .expect("the anthill has drawn")
}
