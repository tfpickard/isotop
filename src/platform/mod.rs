//! Everything isotop reads from the operating system. The rest of the program sees only the
//! contract below, which each platform module provides under the same names:
//!
//! - `Sampler`, read on the frame loop once per sample:
//!   - `Sampler::new() -> Sampler`
//!   - `hz(&self) -> f32`: the unit of `RawProcess::ticks`, in ticks per second;
//!   - `processes(&mut self) -> io::Result<Vec<RawProcess>>`, in any order; an error means
//!     live mode cannot run at all;
//!   - `memory(&self) -> (u64, u64)`: total and available bytes;
//!   - `pressure(&self) -> ([f32; 3], bool)`: CPU, memory and I/O stall percentages, and
//!     whether the source could be read at all;
//!   - `cpus(&mut self, dt: f32) -> Vec<Cpu>`: per-CPU use over the last `dt` seconds, sorted
//!     by id.
//! - Slow sources, read on the background thread every two seconds:
//!   - `network() -> Network`;
//!   - `account(paths, state) -> HashMap<String, Unit>`: resource accounting of the named
//!     cgroups, given the state the previous call left;
//!   - `FileScan::new() -> FileScan` and `FileScan::sample(&mut self) -> HashMap<u32, Option<Files>>`:
//!     the regular files each process holds open, None for a table that could not be read; a
//!     pid the scan has not reached is absent;
//!   - `locks() -> Option<Locks>`: file locks with their holders and waiters, None when they
//!     cannot be read;
//!   - `Gpu::load() -> Option<Gpu>` and `Gpu::sample(&self) -> HashMap<u32, u64>`: GPU memory
//!     per pid.
//! - `journal() -> io::Result<(Child, JournalParser)>`: a running log follower with piped
//!   stdout and stderr, and the parser that reads one entry at a time from its stdout.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead};
use std::net::IpAddr;

use crate::journal::Line;
use crate::model::{IoBytes, Process};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("isotop has platform support for Linux and macOS only");

/// One process as the OS reports it, before isotop smooths anything.
pub struct RawProcess {
    /// Every `Process` field the OS gives directly: id, parent, name, command, group, kind,
    /// state, memory, threads, core, cgroup, priority, nice and written. The derived fields
    /// (cpu, the I/O rates, cpu_time and gpu_memory) are left at zero or None, and those the
    /// background thread fills (files, locks_held and blocked_on) at Pending or None, except
    /// that a kernel thread's files are known to be none.
    pub process: Process,
    /// Cumulative CPU time in ticks of `Sampler::hz()`.
    pub ticks: u64,
    /// Cumulative storage I/O counters, or None when unreadable.
    pub io: Option<IoBytes>,
}

/// Reads one journal entry from the follower's output; `Ok(None)` at the end of the stream.
pub type JournalParser = fn(&mut dyn BufRead) -> io::Result<Option<Line>>;

/// Socket links between processes, by pid.
#[derive(Clone, Debug, Default)]
pub struct Network {
    /// Unordered pid pairs (smaller first) with the number of sockets connecting them.
    pub links: HashMap<(u32, u32), u32>,
    /// The pairs in `links` connected by at least one Unix socket. Empty on a platform that does
    /// not tell the protocols apart.
    pub unix: HashSet<(u32, u32)>,
    /// Established TCP connections whose far end is not a local socket, per pid.
    pub outside: HashMap<u32, u32>,
    /// Those outside connections in detail, from the kernel's TCP statistics.
    pub remotes: Vec<Remote>,
    /// Loopback TCP sockets between two known processes, for per-link traffic.
    pub loopback: Vec<Loopback>,
}

/// One end of an established loopback TCP connection between two different processes.
#[derive(Clone, Debug, PartialEq)]
pub struct Loopback {
    /// Identifies the socket while it is open (its inode on Linux).
    pub inode: u64,
    /// The process that owns this end and the one that owns the other end.
    pub pid: u32,
    pub peer: u32,
    /// Bytes received on this end since the connection opened. What one end receives the other
    /// end sent, so summing both ends counts every byte of the connection once.
    pub received: u64,
}

/// One established TCP connection to another machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    pub pid: u32,
    /// Identifies the socket while it is open (its inode on Linux).
    pub inode: u64,
    pub address: IpAddr,
    pub port: u16,
    /// Smoothed round-trip time in microseconds.
    pub rtt: u32,
    /// Bytes acknowledged by the peer and bytes received, since the connection opened.
    pub sent: u64,
    pub received: u64,
}

/// A regular file whose last link is gone while it is still open: its disk space is not returned
/// until every descriptor on it is closed. Identified by device and inode, so a file held through
/// several descriptors or by several processes counts once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeletedFile {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
}

/// The regular files one process holds open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Files {
    /// Distinct regular files: several descriptors on one file count once.
    pub open: u32,
    /// The deleted files among them, sorted. Found only among examined descriptors.
    pub deleted: Vec<DeletedFile>,
    /// Whether the table was larger than what was examined: `open` is then an estimate and
    /// `deleted` a lower bound.
    pub partial: bool,
}

impl Files {
    pub fn deleted_bytes(&self) -> u64 {
        self.deleted
            .iter()
            .fold(0, |total, file| total.saturating_add(file.size))
    }
}

/// The pid standing for a lock holder that no process can be named for: an OFD lock, which
/// belongs to an open file and reports -1, or a holder outside this pid namespace (0).
pub const UNNAMED: u32 = 0;

/// File locks and the processes blocked on them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Locks {
    /// Locks and leases held, per pid.
    pub held: HashMap<u32, u32>,
    /// Pids blocked on a lock, with the pid holding the lock that blocks them, or UNNAMED.
    pub blocked: HashMap<u32, u32>,
    /// Open file description locks: they belong to an open file, not a process, and report pid
    /// -1, so no process can be named as their holder.
    pub unattributed: u32,
}
