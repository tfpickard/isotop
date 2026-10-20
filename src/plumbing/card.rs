//! The inspector cards of what the plumbing view draws that is not a process: a run of pipes, a
//! severed pipe, a Unix tube, a hose and a manifold. A frame keeps a `Subject` for each of them,
//! with the figures the card needs, and `card` builds the text on demand for the one the pointer
//! is on. Names come from the sample at that time, are raw, and reach the terminal as `SafeText`
//! like every other string a view gives the app.

use super::draw::pipes::SPRAY_SECONDS;
use super::draw::tubes::{CAPSULE_BYTES, MOST_CAPSULES, capsules};
use super::flow::{Outflow, Outlets};
use super::room::Lane;
use crate::model::{Hose, Identity, Snapshot, bytes};
use crate::platform::Wait;
use crate::render::{Card, Point};

/// A thing of the room a card can be built for.
#[derive(Clone, Debug)]
pub enum Subject {
    /// The pipes from one process to another.
    Pipe {
        writer: Identity,
        reader: Identity,
        pipes: usize,
        fifos: usize,
        two_way: bool,
        /// The bytes waiting in the pipes, where the platform says (macOS).
        buffered: Option<u64>,
        /// What the writer's outflow says, and where each end is blocked.
        outflow: Outflow,
        writer_wait: Option<Wait>,
        reader_wait: Option<Wait>,
        at: Point,
    },
    /// A pipe nothing reads. `reader` is the process that held the other end when it was last
    /// seen; None when it never was.
    Severed {
        writer: Identity,
        reader: Option<Identity>,
        /// Seconds since the break was seen, while the spray lasts.
        age: Option<f32>,
        at: Point,
    },
    /// A Unix socket between two processes, with a lane each way.
    Tube {
        low: Identity,
        high: Identity,
        toward_high: Lane,
        toward_low: Lane,
        at: Point,
    },
    /// Loopback TCP between two processes.
    Hose {
        low: Identity,
        high: Identity,
        toward_high: Option<Hose>,
        toward_low: Option<Hose>,
        at: Point,
    },
    /// Where one writer's flow splits among outlets that cannot be told apart.
    Manifold {
        owner: Identity,
        outflow: Outflow,
        at: Point,
    },
}

/// The name of process `id` in `snapshot`, raw, or its pid when it is not there.
fn name(snapshot: &Snapshot, id: Identity) -> String {
    snapshot
        .processes
        .iter()
        .find(|process| process.id == id)
        .map_or_else(|| format!("pid {}", id.pid), |process| process.name.clone())
}

fn rate(value: f32) -> String {
    format!("{}/s", bytes(value.max(0.0) as u64))
}

/// What a process's flow is, in a line or two, for a card and for the inspector.
pub fn flow_lines(who: &str, outflow: Outflow) -> Vec<String> {
    match outflow {
        Outflow::Unmeasured(why) => {
            vec![format!("{who}: flow not measured"), why.text().to_owned()]
        }
        Outflow::Measured {
            rate: value,
            outlets,
        } if outlets.is_single() => vec![format!(
            "{who} writes {}: all of it, through its only outlet that isotop can see",
            rate(value)
        )],
        Outflow::Measured {
            rate: value,
            outlets,
        } => vec![
            format!(
                "{who} writes {}{} over {} outlets",
                if outlets.files > 0 { "up to " } else { "" },
                rate(value),
                outlets.count()
            ),
            "unknown share of it goes through each".to_owned(),
        ],
    }
}

/// The outlets of a flow, counted by kind, for the card of a manifold.
fn outlet_lines(outlets: Outlets) -> String {
    let mut parts = Vec::new();
    for (count, kind) in [
        (outlets.pipes, "pipe"),
        (outlets.tubes, "unix socket"),
        (outlets.files, "writable file"),
        (outlets.sockets, "other socket"),
    ] {
        if count > 0 {
            parts.push(format!(
                "{count} {kind}{}",
                if count == 1 { "" } else { "s" }
            ));
        }
    }
    if outlets.terminal {
        parts.push("a terminal".to_owned());
    }
    let mut text = parts.join(", ");
    if outlets.at_least {
        text += " and maybe more (descriptors cut short)";
    }
    text
}

/// What one lane of a tube holds, in words.
fn lane_line(from: &str, to: &str, lane: Lane) -> String {
    match (lane, capsules(lane)) {
        (None, _) | (_, None) => format!("{from} to {to}: queue not measured"),
        (Some(queue), Some((drawn, total))) => {
            let mut text = format!("{from} to {to}: ");
            if total == 0 {
                text += "nothing waits";
            } else if queue.unread == 0 {
                text += "a datagram waits";
            } else {
                text += &format!(
                    "{} capsule{} = {} unread",
                    total,
                    if total == 1 { "" } else { "s" },
                    bytes(queue.unread)
                );
                if queue.datagram_waiting {
                    text += " and a datagram";
                }
            }
            if total > drawn {
                text += &format!(" ({} not drawn, {MOST_CAPSULES} at most)", total - drawn);
            }
            text
        }
    }
}

/// The card of `subject`, from the names in `snapshot`.
pub fn card(subject: &Subject, snapshot: &Snapshot) -> Card {
    match subject {
        Subject::Pipe {
            writer,
            reader,
            pipes,
            fifos,
            two_way,
            buffered,
            outflow,
            writer_wait,
            reader_wait,
            at,
        } => {
            let (w, r) = (name(snapshot, *writer), name(snapshot, *reader));
            let mut lines = vec![format!(
                "{pipes} pipe{}{}{}",
                if *pipes == 1 { "" } else { "s" },
                if *fifos > 0 {
                    format!(" ({fifos} named)")
                } else {
                    String::new()
                },
                if *two_way { ", runs both ways" } else { "" }
            )];
            lines.extend(flow_lines(&w, *outflow));
            if *writer_wait == Some(Wait::PipeWrite) {
                lines.push(format!(
                    "{w} is blocked writing to a full pipe (one of its {pipes}, which is not told)"
                ));
            }
            if *reader_wait == Some(Wait::PipeRead) {
                lines.push(format!("{r} is waiting to read an empty pipe"));
            }
            lines.push(match buffered {
                Some(waiting) => format!("{} waits unread in the pipe", bytes(*waiting)),
                None => "how full the pipe is cannot be measured".to_owned(),
            });
            Card {
                title: format!("pipe {w} to {r}"),
                lines,
                at: *at,
            }
        }
        Subject::Severed {
            writer,
            reader,
            age,
            at,
        } => {
            let w = name(snapshot, *writer);
            let mut lines = Vec::new();
            match reader {
                Some(reader) => {
                    lines.push(format!("{} dropped its end", name(snapshot, *reader)));
                    lines.push(format!(
                        "reader gone: EPIPE next, and SIGPIPE unless {w} ignores it"
                    ));
                }
                None => lines.push(
                    "no process holds a read end that isotop can see (it may belong to a process it cannot read)"
                        .to_owned(),
                ),
            }
            if let Some(age) = age {
                lines.push(format!(
                    "{age:.0} s since the break was seen, spray for {SPRAY_SECONDS:.0} s"
                ));
            }
            Card {
                title: format!("pipe from {w}, no reader"),
                lines,
                at: *at,
            }
        }
        Subject::Tube {
            low,
            high,
            toward_high,
            toward_low,
            at,
        } => {
            let (l, h) = (name(snapshot, *low), name(snapshot, *high));
            Card {
                title: format!("unix socket {l} and {h}"),
                lines: vec![
                    lane_line(&l, &h, *toward_high),
                    lane_line(&h, &l, *toward_low),
                    format!(
                        "one capsule for every {} KiB a reader has not read; a datagram counts one",
                        CAPSULE_BYTES / 1024
                    ),
                ],
                at: *at,
            }
        }
        Subject::Hose {
            low,
            high,
            toward_high,
            toward_low,
            at,
        } => {
            let (l, h) = (name(snapshot, *low), name(snapshot, *high));
            let direction = |from: &str, to: &str, hose: &Option<Hose>| match hose {
                Some(hose) => format!(
                    "{from} to {to}: {} over {} connection{}",
                    rate(hose.rate),
                    hose.connections,
                    if hose.connections == 1 { "" } else { "s" }
                ),
                None => format!("{from} to {to}: rate not measured"),
            };
            Card {
                title: format!("loopback TCP {l} and {h}"),
                lines: vec![
                    direction(&l, &h, toward_high),
                    direction(&h, &l, toward_low),
                ],
                at: *at,
            }
        }
        Subject::Manifold { owner, outflow, at } => {
            let who = name(snapshot, *owner);
            let mut lines = flow_lines(&who, *outflow);
            if let Outflow::Measured { outlets, .. } = outflow {
                lines.push(outlet_lines(*outlets));
            }
            lines.push("branches are drawn equal; no per-branch number exists".to_owned());
            Card {
                title: format!("split unknown: {who}"),
                lines,
                at: *at,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Queue;
    use crate::plumbing::flow::Why;

    fn who(pid: u32) -> Identity {
        Identity {
            pid,
            start: u64::from(pid),
        }
    }

    #[test]
    fn a_card_says_what_is_known_and_what_is_not() {
        let snapshot = Snapshot::default();
        let unmeasured = card(
            &Subject::Pipe {
                writer: who(1),
                reader: who(2),
                pipes: 2,
                fifos: 1,
                two_way: false,
                buffered: None,
                outflow: Outflow::Unmeasured(Why::NotRead),
                writer_wait: Some(Wait::PipeWrite),
                reader_wait: None,
                at: [0.0; 3],
            },
            &snapshot,
        );
        assert_eq!(unmeasured.title, "pipe pid 1 to pid 2");
        let text = unmeasured.lines.join(" | ");
        assert!(text.contains("2 pipes (1 named)"), "{text}");
        assert!(text.contains("flow not measured"), "{text}");
        assert!(text.contains("blocked writing to a full pipe"), "{text}");
        assert!(text.contains("cannot be measured"), "{text}");
        assert!(!text.contains("B/s"), "no number invented: {text}");
    }

    #[test]
    fn a_pipe_card_reports_the_bytes_waiting_where_the_platform_tells() {
        let known = card(
            &Subject::Pipe {
                writer: who(1),
                reader: who(2),
                pipes: 1,
                fifos: 0,
                two_way: false,
                buffered: Some(2048),
                outflow: Outflow::Unmeasured(Why::NotRead),
                writer_wait: None,
                reader_wait: None,
                at: [0.0; 3],
            },
            &Snapshot::default(),
        );
        let text = known.lines.join(" | ");
        assert!(text.contains("2 KiB waits unread in the pipe"), "{text}");
        assert!(!text.contains("cannot be measured"), "{text}");
    }

    #[test]
    fn a_split_flow_names_its_outlets_and_gives_no_share() {
        let outflow = Outflow::Measured {
            rate: 3.0e6,
            outlets: Outlets {
                pipes: 3,
                files: 1,
                ..Outlets::default()
            },
        };
        let lines = card(
            &Subject::Manifold {
                owner: who(1),
                outflow,
                at: [0.0; 3],
            },
            &Snapshot::default(),
        )
        .lines;
        let text = lines.join(" | ");
        assert!(text.contains("over 4 outlets"), "{text}");
        assert!(text.contains("unknown share"), "{text}");
        assert!(text.contains("3 pipes, 1 writable file"), "{text}");
        assert!(
            text.contains("up to "),
            "a writable file may hold some of it: {text}"
        );
        assert!(text.contains("no per-branch number"), "{text}");
    }

    #[test]
    fn a_tube_card_counts_capsules_and_names_the_overflow() {
        let lane = Some(Queue {
            unread: 4096 * 20,
            datagram_waiting: false,
        });
        let line = lane_line("a", "b", lane);
        assert!(line.contains("20 capsules = 80 KiB unread"), "{line}");
        assert!(line.contains("8 not drawn"), "{line}");
        assert_eq!(lane_line("a", "b", None), "a to b: queue not measured");
        let datagram = lane_line(
            "a",
            "b",
            Some(Queue {
                unread: 0,
                datagram_waiting: true,
            }),
        );
        assert!(datagram.contains("a datagram waits"), "{datagram}");
        let idle = lane_line("a", "b", Some(Queue::default()));
        assert!(idle.contains("nothing waits"), "{idle}");
    }

    #[test]
    fn a_severed_pipe_says_whether_its_reader_was_seen_to_go() {
        let snapshot = Snapshot::default();
        let seen = card(
            &Subject::Severed {
                writer: who(1),
                reader: Some(who(2)),
                age: Some(3.0),
                at: [0.0; 3],
            },
            &snapshot,
        );
        let text = seen.lines.join(" | ");
        assert!(text.contains("reader gone: EPIPE next"), "{text}");
        let unseen = card(
            &Subject::Severed {
                writer: who(1),
                reader: None,
                age: None,
                at: [0.0; 3],
            },
            &snapshot,
        );
        let text = unseen.lines.join(" | ");
        assert!(!text.contains("reader gone"), "never seen with one: {text}");
        assert!(text.contains("no process holds a read end"), "{text}");
    }
}
