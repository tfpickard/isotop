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
//!     whether the source could be read at all (when not, the collector reports all three
//!     pressures as missing);
//!   - `cpus(&mut self, dt: f32) -> Vec<Cpu>`: per-CPU use over the last `dt` seconds, sorted
//!     by id. It is called after `processes` in every sample, so a platform may derive CPU
//!     readings from the processes it has just read;
//!   - `missing(&self) -> Vec<&'static str>`: what this platform can never measure, named as
//!     `Snapshot::missing` names it and appended to it every sample. The names views react to
//!     are "cpu pressure", "memory pressure", "io pressure", "last cpu" (`Process::core` is
//!     meaningless), "cpu clock" (`Cpu::mhz` is 0), "run queue" (`Cpu::wait` is 0), "cgroups"
//!     (no accounting units, so no limits, quotas or pressure) and "socket traffic" (no
//!     per-connection rates or round-trip times). Linux returns none;
//!   - `per_cluster(&self) -> Vec<&'static str>`: sources measured per CPU cluster rather than
//!     per CPU, named as `Snapshot::per_cluster` names them: "cpu clock" when every CPU's
//!     `Cpu::mhz` is the average clock of the cluster of its kind. Linux returns none;
//!   - `unreadable(&self) -> usize`: how many processes the last `processes()` call could see
//!     but not measure because they belong to another user, and so left out. Races with exiting
//!     processes are not permission gaps and do not count; Linux returns 0.
//! - Slow sources, read on the background thread every two seconds:
//!   - `network() -> Network`;
//!   - `account(paths, state) -> HashMap<String, Unit>`: resource accounting of the named
//!     cgroups, given the state the previous call left;
//!   - `Gpu::load() -> Option<Gpu>` and `Gpu::sample(&self) -> HashMap<u32, u64>`: GPU memory
//!     per pid.
//! - `journal() -> io::Result<(Child, JournalParser)>`: a running log follower with piped
//!   stdout and stderr, and the parser that reads one entry at a time from its stdout;
//!   `JOURNAL: &str` names the follower (`journalctl` or `log`) in the messages isotop prints
//!   when it cannot run or when it complains on stderr.
//! - `MEMORY_LABEL: &str` names what `Process::memory` measures (RSS on Linux), as the
//!   inspector and legends show it.

use std::collections::HashMap;
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

// The macOS collector's pure logic, so its tests also run in the Linux gates.
#[cfg(all(test, not(target_os = "macos")))]
#[path = "macos/logic.rs"]
mod macos_logic;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("isotop has platform support for Linux and macOS only");

// The macOS log parser is std-only; Linux test builds compile it too so its tests run in the
// Linux gates.
#[cfg(all(test, not(target_os = "macos")))]
#[allow(dead_code)]
#[path = "macos/journal.rs"]
mod macos_journal;

/// One process as the OS reports it, before isotop smooths anything.
pub struct RawProcess {
    /// Every `Process` field the OS gives directly: id, parent, name, command, group, kind,
    /// state, memory, threads, core, cgroup, priority, nice and written. The derived fields
    /// (cpu, the I/O rates, cpu_time and gpu_memory) are left at zero or None.
    pub process: Process,
    /// Cumulative CPU time in ticks of `Sampler::hz()`.
    pub ticks: u64,
    /// Cumulative storage I/O counters, or None when unreadable.
    pub io: Option<IoBytes>,
    /// Cumulative CPU time on performance cores, in ticks, from the same reading as `ticks`;
    /// None where the platform does not split CPU time by core kind.
    pub performance_ticks: Option<u64>,
    /// Cumulative time its threads were runnable, running or waiting for a CPU, in ticks, from
    /// the same reading as `ticks`; None when unmeasured. A kernel may bring it up to date only
    /// when a thread is switched onto a CPU or blocks (macOS does), so it can trail `ticks`
    /// while threads run and catch up later; the collector counts as waiting only the growth of
    /// `runnable_ticks - ticks` beyond its highest earlier value.
    pub runnable_ticks: Option<u64>,
}

/// Reads one journal entry from the follower's output; `Ok(None)` at the end of the stream.
pub type JournalParser = fn(&mut dyn BufRead) -> io::Result<Option<Line>>;

/// Socket links between processes, by pid.
#[derive(Clone, Debug, Default)]
pub struct Network {
    /// Unordered pid pairs (smaller first) with the number of sockets connecting them.
    pub links: HashMap<(u32, u32), u32>,
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
