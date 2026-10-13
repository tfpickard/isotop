//! What a process puts out through its write calls and where it can have gone, with no drawing:
//! the attribution rule of the plumbing view, and the two transforms that turn a rate into a
//! gauge and a water speed.
//!
//! **The rule.** `R` is the growth of the process's `wchar` that no file position explains
//! (`IoDetail::unattributed`, which already takes off what its write-only files advanced), less
//! what it sent over TCP, which is measured exactly. The outlets that remain for `R` are every
//! write-only or read-write regular file, every pipe or FIFO it may write, every Unix socket pair,
//! every socket that is none of those, and a terminal. One outlet carries `R` exactly. Two or more
//! cannot be told apart: the flow is `R` as a whole and its share of each outlet is unknown, which
//! is never drawn as a number. A file counts as an outlet whatever its position says, because
//! `pwrite`, AIO and io_uring write to a file without moving it, so those bytes would otherwise
//! land in `R` and make a lone pipe look exact.
//!
//! **What is not an outlet.** Loopback and remote TCP connections whose send rate was measured
//! are taken off `R`, so they are not outlets for it. Anonymous kernel objects (epoll, eventfd)
//! and descriptors of other kinds are not counted: B8 gives no access mode for them.

use crate::io_detail::IoDetail;
use crate::model::{Measured, PipeHold};

/// Water speed of a run whose rate is `rate` bytes per second, in floor cells per second: 0.6 at
/// 1 KB/s and below, up by 0.45 for every decade, and 0 when nothing flows. Four decades of rate
/// make a speed factor of four, so it says faster or slower and never how much.
pub fn water_speed(rate: f32) -> f32 {
    if rate <= 0.0 || rate.is_nan() {
        0.0
    } else {
        0.6 + 0.45 * (rate.max(1e3) / 1e3).log10()
    }
}

/// Where a gauge needle stands for an outflow of `rate` bytes per second, 0 to 1 of its sweep: a
/// log scale from 1 KiB/s to 64 MiB/s, clamped at both ends.
pub fn gauge_fill(rate: f32) -> f32 {
    const LOW: f32 = 1024.0;
    const HIGH: f32 = 64.0 * 1048576.0;
    if rate.is_nan() || rate <= LOW {
        0.0
    } else {
        ((rate / LOW).log10() / (HIGH / LOW).log10()).clamp(0.0, 1.0)
    }
}

/// Why a flow cannot be told: what the water shows as unmeasured, and the inspector says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// The process is not among the busiest, so the I/O detail has not reached it (or the source
    /// is not read).
    NotRead,
    /// Another user's process: its descriptors cannot be read.
    Unreadable,
    /// The detail has no remainder: the first reading of the process, or a platform with no
    /// `rchar` and `wchar` to take a remainder from (macOS).
    NoRemainder,
    /// It holds TCP connections whose send rate has not been measured yet, so what they took out
    /// of its write calls is not known.
    TcpUnmeasured,
    /// The pipe scan has not reached it, so its writable pipes are not known.
    PipesUnknown,
}

impl Why {
    /// The reason in a few words, for the inspector.
    pub fn text(self) -> &'static str {
        match self {
            Why::NotRead => "its descriptors were not read (only the busiest processes are)",
            Why::Unreadable => "another user's process: its descriptors cannot be read",
            Why::NoRemainder => {
                "no write-call bytes to subtract files from yet (or on this platform)"
            }
            Why::TcpUnmeasured => "its TCP send rate is not measured yet",
            Why::PipesUnknown => "its pipes were not read yet",
        }
    }
}

/// The places `R` can have gone, as far as the process's descriptors say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outlets {
    /// Distinct regular files it may write (opened for writing or for both).
    pub files: u32,
    /// Distinct pipes and FIFOs it may write.
    pub pipes: u32,
    /// Unix socket pairs with another process that is drawn.
    pub tubes: u32,
    /// Socket descriptors that are neither a drawn Unix socket nor a TCP connection whose send
    /// rate was measured: listeners, datagram sockets, netlink, connections to processes that
    /// are not drawn.
    pub sockets: u32,
    /// It holds a terminal descriptor, which may take writes. Several count as one.
    pub terminal: bool,
    /// A cap cut the walk of its descriptors short, so there may be outlets that are not counted.
    pub at_least: bool,
}

impl Outlets {
    /// How many outlets are counted.
    pub fn count(self) -> u32 {
        self.files + self.pipes + self.tubes + self.sockets + u32::from(self.terminal)
    }

    /// Whether the count is the whole story and there is exactly one outlet.
    pub fn is_single(self) -> bool {
        self.count() == 1 && !self.at_least
    }

    /// Whether the flow splits among outlets that cannot be told apart: two or more, or one
    /// that is known and a walk of the descriptors that was cut short.
    pub fn splits(self) -> bool {
        self.count() >= 2 || (self.at_least && self.count() >= 1)
    }

    /// The branches a manifold has: at least two, since a manifold is for a split.
    pub fn branches(self) -> u32 {
        self.count().max(2)
    }
}

/// What a process's write calls carried, or why that is not known.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outflow {
    /// Not measured: the water stands still and the inspector says why. Never 0.
    Unmeasured(Why),
    /// The write-call bytes per second that no file position and no TCP connection explains, and
    /// the outlets they can have gone to.
    Measured { rate: f32, outlets: Outlets },
}

impl Outflow {
    /// The measured rate, if there is one.
    pub fn rate(self) -> Option<f32> {
        match self {
            Outflow::Measured { rate, .. } => Some(rate),
            Outflow::Unmeasured(_) => None,
        }
    }

    /// The speed of the water in the runs this outflow feeds, display only: the whole rate for a
    /// single outlet, the rate over the branches of a manifold otherwise (an equal split, which is
    /// a display convention and no measurement). None when unmeasured.
    pub fn speed(self) -> Option<f32> {
        match self {
            Outflow::Measured { rate, outlets } if outlets.is_single() => Some(water_speed(rate)),
            Outflow::Measured { rate, outlets } => {
                Some(water_speed(rate / outlets.branches() as f32))
            }
            Outflow::Unmeasured(_) => None,
        }
    }
}

/// What the view knows of one process's connections, joined from the sample.
#[derive(Clone, Copy, Debug, Default)]
pub struct Sockets {
    /// Unix socket pairs with processes that are drawn.
    pub unix_pairs: u32,
    /// Loopback TCP connections of this process whose send rate was measured, and what it sent
    /// over them per second.
    pub loopback_connections: u32,
    pub loopback_sent: f32,
    /// Remote TCP connections of this process that were measured, and what it sent over them.
    pub remote_connections: u32,
    pub remote_sent: f32,
    /// It has a TCP connection whose send rate is not measured: its bytes are in `R` and cannot
    /// be told from the rest.
    pub unmeasured_connection: bool,
}

/// The outflow of a process from its I/O detail, the pipes it may write and its connections.
/// `pipes_known` is false when the pipe scan has not read it (it is absent from the holds and the
/// scan was partial, or the source was not read at all).
pub fn outflow(
    detail: &Measured<std::sync::Arc<IoDetail>>,
    hold: Option<PipeHold>,
    pipes_known: bool,
    sockets: Sockets,
) -> Outflow {
    let detail = match detail {
        Measured::Known(detail) => detail,
        Measured::Pending => return Outflow::Unmeasured(Why::NotRead),
        Measured::Unreadable => return Outflow::Unmeasured(Why::Unreadable),
    };
    let Some(remainder) = detail.unattributed else {
        return Outflow::Unmeasured(Why::NoRemainder);
    };
    if sockets.unmeasured_connection {
        return Outflow::Unmeasured(Why::TcpUnmeasured);
    }
    if !pipes_known {
        return Outflow::Unmeasured(Why::PipesUnknown);
    }
    let sent = sockets.loopback_sent + sockets.remote_sent;
    let accounted = sockets.unix_pairs + sockets.loopback_connections + sockets.remote_connections;
    Outflow::Measured {
        rate: (remainder.write - sent).max(0.0),
        outlets: Outlets {
            files: detail
                .files
                .iter()
                .filter(|file| file.access.writes())
                .count() as u32,
            pipes: hold.map_or(0, |hold| hold.writes),
            tubes: sockets.unix_pairs,
            sockets: detail.sockets.saturating_sub(accounted),
            terminal: detail.ttys > 0,
            at_least: detail.partial,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::io_detail::{FileFlow, Flow, Unattributed};
    use crate::platform::AccessMode;

    fn file(access: AccessMode) -> FileFlow {
        FileFlow {
            device: 1,
            inode: 2,
            size: 100,
            path: "/var/log/x".into(),
            deleted: false,
            access,
            flow: Flow::Unmeasured,
        }
    }

    /// A process that wrote `write` bytes per second no file position explains, holding `files`
    /// and `ttys` terminals, with nothing else.
    fn detail(write: f32, files: Vec<FileFlow>, ttys: u32) -> Measured<Arc<IoDetail>> {
        Measured::Known(Arc::new(IoDetail {
            files,
            ttys,
            unattributed: Some(Unattributed {
                read: 0.0,
                write,
                either: 0.0,
            }),
            ..IoDetail::default()
        }))
    }

    fn one_pipe() -> Option<PipeHold> {
        Some(PipeHold {
            writes: 1,
            reads: 0,
        })
    }

    fn measured(outflow: Outflow) -> (f32, Outlets) {
        match outflow {
            Outflow::Measured { rate, outlets } => (rate, outlets),
            other => panic!("unmeasured: {other:?}"),
        }
    }

    #[test]
    fn a_single_outlet_carries_the_whole_residual_after_file_writes() {
        // 3 MB/s through write calls; the files took none of it (the remainder already has the
        // write-only files' positions taken off), and a read-only file is no outlet.
        let io = detail(3.0e6, vec![file(AccessMode::Read)], 0);
        let found = outflow(&io, one_pipe(), true, Sockets::default());
        let (rate, outlets) = measured(found);
        assert_eq!(rate, 3.0e6);
        assert!(outlets.is_single() && outlets.pipes == 1);
        assert_eq!(found.speed(), Some(water_speed(3.0e6)), "the whole rate");
    }

    #[test]
    fn several_outlets_feed_a_split_unknown_manifold_with_equal_branches() {
        let hold = Some(PipeHold {
            writes: 3,
            reads: 0,
        });
        let io = detail(3.0e6, Vec::new(), 0);
        let found = outflow(&io, hold, true, Sockets::default());
        let (rate, outlets) = measured(found);
        assert_eq!(
            (rate, outlets.count(), outlets.is_single()),
            (3.0e6, 3, false)
        );
        assert_eq!(outlets.branches(), 3);
        // The branches animate at the speed of an equal share, which is a display convention.
        assert_eq!(found.speed(), Some(water_speed(1.0e6)));
        // A pipe and a terminal are two outlets: the write calls cannot be told apart.
        let tty = outflow(
            &detail(2.0e6, Vec::new(), 4),
            one_pipe(),
            true,
            Sockets::default(),
        );
        let (_, outlets) = measured(tty);
        assert_eq!(
            (outlets.count(), outlets.terminal),
            (2, true),
            "4 descriptors, 1 outlet"
        );
    }

    #[test]
    fn a_writable_file_makes_a_lone_pipe_split_unknown() {
        for access in [AccessMode::Write, AccessMode::Both] {
            let io = detail(3.0e6, vec![file(access)], 0);
            let (_, outlets) = measured(outflow(&io, one_pipe(), true, Sockets::default()));
            assert_eq!(outlets.count(), 2, "{access:?}: the pipe and the file");
            assert!(!outlets.is_single());
        }
        // Without it the same pipe is exact.
        let io = detail(3.0e6, vec![file(AccessMode::Read)], 0);
        let (_, outlets) = measured(outflow(&io, one_pipe(), true, Sockets::default()));
        assert!(outlets.is_single());
    }

    #[test]
    fn a_cut_walk_of_the_descriptors_is_never_exact() {
        let mut io = detail(1.0e6, Vec::new(), 0);
        if let Measured::Known(detail) = &mut io {
            Arc::make_mut(detail).partial = true;
        }
        let (_, outlets) = measured(outflow(&io, one_pipe(), true, Sockets::default()));
        assert_eq!(outlets.count(), 1);
        assert!(
            !outlets.is_single(),
            "there may be outlets the walk did not reach"
        );
    }

    #[test]
    fn tcp_sends_come_off_the_residual_and_their_sockets_are_not_outlets() {
        let mut io = detail(6.4e6, Vec::new(), 0);
        if let Measured::Known(detail) = &mut io {
            Arc::make_mut(detail).sockets = 3;
        }
        let sockets = Sockets {
            unix_pairs: 1,
            loopback_connections: 1,
            loopback_sent: 1.4e6,
            ..Sockets::default()
        };
        let (rate, outlets) = measured(outflow(&io, None, true, sockets));
        assert!((rate - 5.0e6).abs() < 1.0, "{rate}");
        assert_eq!(
            (outlets.tubes, outlets.sockets, outlets.count()),
            (1, 1, 2),
            "one drawn Unix pair, and of the three sockets one more that nothing explains"
        );
        // Sent more than it wrote in the interval: nothing is left, which is a floor at 0.
        let more = Sockets {
            loopback_sent: 9.0e6,
            ..sockets
        };
        assert_eq!(measured(outflow(&io, None, true, more)).0, 0.0);
    }

    #[test]
    fn unsampled_file_writes_leave_pipe_flow_unmeasured() {
        let pending: Measured<Arc<IoDetail>> = Measured::Pending;
        assert_eq!(
            outflow(&pending, one_pipe(), true, Sockets::default()),
            Outflow::Unmeasured(Why::NotRead)
        );
        assert_eq!(
            outflow(&Measured::Unreadable, one_pipe(), true, Sockets::default()),
            Outflow::Unmeasured(Why::Unreadable)
        );
        // Read, but with no remainder: the first reading, or no byte counters.
        let mut first = detail(0.0, Vec::new(), 0);
        if let Measured::Known(detail) = &mut first {
            Arc::make_mut(detail).unattributed = None;
        }
        assert_eq!(
            outflow(&first, one_pipe(), true, Sockets::default()),
            Outflow::Unmeasured(Why::NoRemainder)
        );
        // Holding a TCP connection nobody has measured yet hides what it took.
        let io = detail(1.0e6, Vec::new(), 0);
        let unknown = Sockets {
            unmeasured_connection: true,
            ..Sockets::default()
        };
        assert_eq!(
            outflow(&io, one_pipe(), true, unknown),
            Outflow::Unmeasured(Why::TcpUnmeasured)
        );
        assert_eq!(
            outflow(&io, None, false, Sockets::default()),
            Outflow::Unmeasured(Why::PipesUnknown)
        );
        for why in [
            Why::NotRead,
            Why::Unreadable,
            Why::NoRemainder,
            Why::TcpUnmeasured,
            Why::PipesUnknown,
        ] {
            assert_eq!(Outflow::Unmeasured(why).rate(), None, "{why:?} is not 0");
            assert_eq!(Outflow::Unmeasured(why).speed(), None);
        }
    }

    #[test]
    fn a_measured_zero_is_zero_and_stills_the_water() {
        let io = detail(0.0, Vec::new(), 0);
        let found = outflow(&io, one_pipe(), true, Sockets::default());
        assert_eq!(found.rate(), Some(0.0));
        assert_eq!(found.speed(), Some(0.0));
    }

    #[test]
    fn water_speed_follows_the_log_law_and_stops_at_zero() {
        assert_eq!(water_speed(0.0), 0.0);
        assert_eq!(water_speed(1.0), 0.6, "below 1 KB/s the speed is the floor");
        assert!((water_speed(1.0e3) - 0.6).abs() < 1e-6);
        assert!(
            (water_speed(1.0e6) - 1.95).abs() < 1e-5,
            "{}",
            water_speed(1.0e6)
        );
        assert!(
            (water_speed(64.0e6) - 2.763).abs() < 1e-3,
            "{}",
            water_speed(64.0e6)
        );
        assert_eq!(water_speed(f32::NAN), 0.0);
    }

    #[test]
    fn the_gauge_is_a_log_scale_from_one_kib_to_sixty_four_mib() {
        assert_eq!(gauge_fill(0.0), 0.0);
        assert_eq!(gauge_fill(1024.0), 0.0);
        assert!((gauge_fill(64.0 * 1048576.0) - 1.0).abs() < 1e-6);
        assert_eq!(gauge_fill(1.0e12), 1.0);
        let middle = gauge_fill(256.0 * 1024.0);
        assert!(
            (middle - 0.5).abs() < 1e-5,
            "256 KiB is half the decades: {middle}"
        );
        assert!(gauge_fill(1.0e5) < gauge_fill(1.0e6));
        assert_eq!(gauge_fill(f32::NAN), 0.0);
    }
}
