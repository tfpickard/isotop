//! What the anthill says: numbers as words, and the legend, the cards and the popup lines built
//! from what the view knows. Paths and names are handed on raw, as plain strings: `SafeText` makes
//! them safe where the app builds a label, a popup or a status row, and nothing here cleans text or
//! writes to a terminal.

use super::flows::{Pile, Tally, Unit};
use super::scale;
use crate::model::{Identity, Process, Snapshot};
use crate::platform::Source;
use crate::status::{missing_note, sources_note, unreadable_note};

/// A byte count as it is written on the slab: `64 B`, `4 KiB`, `1.5 MiB`, `128 KiB`.
pub fn bytes(value: f32) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut scaled = f64::from(value.max(0.0));
    let mut unit = 0;
    while scaled >= 1024.0 && unit + 1 < UNITS.len() {
        scaled /= 1024.0;
        unit += 1;
    }
    if scaled >= 100.0 || (scaled - scaled.round()).abs() < 0.05 {
        format!("{:.0} {}", scaled, UNITS[unit])
    } else {
        format!("{:.1} {}", scaled, UNITS[unit])
    }
}

/// Bytes per second.
pub fn per_second(value: f32) -> String {
    format!("{}/s", bytes(value))
}

/// A count of things per second, short: `950`, `1.2k`, `48k`, `1.3M`.
pub fn count(value: f32) -> String {
    let value = value.max(0.0);
    if value < 1000.0 {
        format!("{value:.0}")
    } else if value < 10_000.0 {
        format!("{:.1}k", value / 1000.0)
    } else if value < 1_000_000.0 {
        format!("{:.0}k", value / 1000.0)
    } else {
        format!("{:.1}M", value / 1_000_000.0)
    }
}

/// The last component of a path, which is what labels a pile.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

/// The word for the thing an ant of this stream stands for one of.
pub fn unit_word(unit: Unit) -> &'static str {
    match unit {
        Unit::Calls => "syscalls",
        Unit::Kibibytes => "KiB moved",
    }
}

/// What the view knows about its last frame that its legend tells.
#[derive(Clone, Debug, Default)]
pub struct Facts {
    pub unit: Option<Unit>,
    pub k: u64,
    pub ants: usize,
    pub refused: u64,
    pub nests: usize,
    pub piles: usize,
    pub tally: Tally,
    /// Some process has a reading of where its I/O goes.
    pub read_any: bool,
}

/// The legend line: what is drawn now first, then what is missing and what is used instead, then
/// the sources read, then the mappings.
pub fn legend(facts: &Facts, snapshot: &Snapshot) -> String {
    let unit = facts.unit.unwrap_or(Unit::Calls);
    let mut line = format!(
        " ant = K {} (K = {}) | {} ants | {} nests, {} piles",
        unit_word(unit),
        scale::text(facts.k.max(1)),
        scale::text(facts.ants as u64),
        facts.nests,
        facts.piles,
    );
    if facts.refused > 0 {
        line.push_str(&format!(
            " | {} ants over the cap",
            scale::text(facts.refused)
        ));
    }
    let tally = &facts.tally;
    if !facts.read_any && !snapshot.sources.contains(Source::IoDetail) {
        line.push_str(" | reading open files");
    }
    let notes = [
        missing_note(snapshot, Source::IoDetail, "no piles, no ants"),
        missing_note(
            snapshot,
            Source::SyscallIo,
            "ant = K KiB moved, crumbs unsized",
        ),
        missing_note(
            snapshot,
            Source::UnattributedIo,
            "tunnels show no rate, nothing goes to a blanket",
        ),
        unreadable_note("fdinfo", tally.unreadable),
        unreadable_note("syscall counts", tally.uncounted),
        (tally.unplaced > 0.5)
            .then(|| format!(" | {} unattributed, not drawn", per_second(tally.unplaced))),
        (tally.partial > 0).then(|| format!(" | {} readings cut short", tally.partial)),
        (tally.beyond > 0).then(|| format!(" | {} files beyond 256 not drawn", tally.beyond)),
        (tally.tunnels_beyond > 0)
            .then(|| format!(" | {} tunnels not drawn", tally.tunnels_beyond)),
        sources_note(snapshot),
    ];
    for note in notes.into_iter().flatten() {
        line.push_str(&note);
    }
    line.push_str(
        " | mound = process (size ~ memory) | pile = open file | crumb = bytes per syscall \
         | pale = read, coral = write | glow = pheromone | dashes = pipes, sockets",
    );
    line
}

/// The name of the process `id` in `snapshot`, raw, or its pid when it is gone.
pub fn name_of(snapshot: &Snapshot, id: Identity) -> String {
    snapshot
        .processes
        .iter()
        .find(|process| process.id == id)
        .map_or_else(|| format!("pid {}", id.pid), |process| process.name.clone())
}

/// The inspector card of a pile: its path as the title, then its size, who holds it and what
/// their positions say they moved, which is a lower bound, and what is not known.
pub fn pile_card(pile: &Pile, snapshot: &Snapshot) -> (String, Vec<String>) {
    let mut lines = vec![format!(
        "size {}{}",
        bytes(pile.size as f32),
        if pile.deleted {
            ", deleted while open"
        } else {
            ""
        }
    )];
    let names: Vec<String> = pile
        .holders
        .iter()
        .take(3)
        .map(|holder| name_of(snapshot, holder.id))
        .collect();
    let more = pile.holders.len().saturating_sub(names.len());
    lines.push(format!(
        "readers {}, writers {}: {}{}",
        pile.readers(),
        pile.writers(),
        names.join(", "),
        if more > 0 {
            format!(", +{more} more")
        } else {
            String::new()
        }
    ));
    match pile.moved() {
        Some(moved) => lines.push(format!(
            "moved at least {} read, {} written, {} read-write",
            per_second(moved.read),
            per_second(moved.written),
            per_second(moved.either)
        )),
        None => lines.push("moved: not measured".to_owned()),
    }
    if pile.unmeasured() > 0 {
        lines.push(format!(
            "{} without an earlier reading, which is not zero",
            pile.unmeasured()
        ));
    }
    if pile.unpositioned() > 0 {
        lines.push(format!(
            "{} appending or seeking: the position says nothing of bytes",
            pile.unpositioned()
        ));
    }
    (pile.path.clone(), lines)
}

/// The popup lines of a nest: what its calls were, how many files it holds and how much of its
/// bytes they explain.
pub fn nest_lines(
    process: &Process,
    piles: usize,
    files: usize,
    explained: Option<f32>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(io) = process.syscall_io {
        lines.push(format!(
            "{} reads/s of {} each, {} writes/s of {} each",
            count(io.reads),
            if io.reads > 0.0 {
                bytes(io.read_chars / io.reads)
            } else {
                "-".to_owned()
            },
            count(io.writes),
            if io.writes > 0.0 {
                bytes(io.write_chars / io.writes)
            } else {
                "-".to_owned()
            },
        ));
    }
    lines.push(format!("{files} open files, {piles} drawn as piles"));
    if let Some(share) = explained {
        lines.push(format!(
            "{:.0}% of its bytes went to files; the rest is unattributed",
            share * 100.0
        ));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthill::flows::Holder;
    use crate::io_detail::Flow;
    use crate::platform::{AccessMode, Sources};

    #[test]
    fn bytes_are_written_in_the_unit_that_keeps_them_short() {
        for (value, text) in [
            (0.0, "0 B"),
            (64.0, "64 B"),
            (1023.0, "1023 B"),
            (1024.0, "1 KiB"),
            (1536.0, "1.5 KiB"),
            (4096.0, "4 KiB"),
            (131_072.0, "128 KiB"),
            (2.5 * 1_048_576.0, "2.5 MiB"),
            (1_073_741_824.0, "1 GiB"),
            (-5.0, "0 B"),
        ] {
            assert_eq!(bytes(value), text, "{value}");
        }
        assert_eq!(per_second(2048.0), "2 KiB/s");
    }

    #[test]
    fn counts_are_short() {
        assert_eq!(count(950.0), "950");
        assert_eq!(count(1234.0), "1.2k");
        assert_eq!(count(48_200.0), "48k");
        assert_eq!(count(1_300_000.0), "1.3M");
    }

    #[test]
    fn a_file_is_labelled_by_its_last_component() {
        assert_eq!(file_name("/var/log/syslog"), "syslog");
        assert_eq!(file_name("/var/log/"), "log");
        assert_eq!(file_name("syslog"), "syslog");
        assert_eq!(file_name("/"), "/");
    }

    fn facts() -> Facts {
        Facts {
            unit: Some(Unit::Calls),
            k: 50,
            ants: 2651,
            nests: 10,
            piles: 13,
            read_any: true,
            ..Facts::default()
        }
    }

    #[test]
    fn the_legend_leads_with_what_is_drawn_and_ends_with_the_mappings() {
        let line = legend(&facts(), &Snapshot::default());
        assert!(
            line.starts_with(" ant = K syscalls (K = 50) | 2,651 ants | 10 nests, 13 piles"),
            "{line}"
        );
        assert!(line.contains("| mound = process (size ~ memory) | pile = open file"));
        assert!(line.ends_with("| glow = pheromone | dashes = pipes, sockets"));
        for word in ["crumb = bytes per syscall", "pale = read, coral = write"] {
            assert!(line.contains(word), "{word}");
        }
    }

    #[test]
    fn the_legend_says_what_is_missing_and_what_is_used_instead() {
        let snapshot = Snapshot {
            missing: Sources::of(&[Source::SyscallIo, Source::UnattributedIo]),
            ..Snapshot::default()
        };
        let mut facts = facts();
        facts.unit = Some(Unit::Kibibytes);
        facts.tally = Tally {
            unreadable: 3,
            uncounted: 2,
            unplaced: 4096.0,
            ..Tally::default()
        };
        let line = legend(&facts, &snapshot);
        assert!(line.contains(" ant = K KiB moved (K = 50)"), "{line}");
        assert!(
            line.contains(" | no syscall io: ant = K KiB moved, crumbs unsized"),
            "{line}"
        );
        assert!(line.contains(" | no unattributed io: "), "{line}");
        assert!(line.contains(" | fdinfo unreadable for 3"), "{line}");
        assert!(
            line.contains(" | syscall counts unreadable for 2"),
            "{line}"
        );
        assert!(
            line.contains(" | 4 KiB/s unattributed, not drawn"),
            "{line}"
        );
        // The notes come before the mappings.
        let note = line.find("no syscall io").unwrap();
        assert!(note < line.find("mound = process").unwrap());
    }

    #[test]
    fn the_legend_says_when_the_open_files_are_not_read_yet() {
        let mut facts = facts();
        facts.read_any = false;
        let waiting = legend(&facts, &Snapshot::default());
        assert!(waiting.contains(" | reading open files"), "{waiting}");
        let snapshot = Snapshot {
            sources: Source::IoDetail.into(),
            ..Snapshot::default()
        };
        let read = legend(&facts, &snapshot);
        assert!(!read.contains("reading open files"), "{read}");
        assert!(read.contains(" | sources: io detail"), "{read}");
    }

    #[test]
    fn a_pile_card_says_what_is_known_and_what_is_not() {
        let id = |pid| Identity { pid, start: 1 };
        let pile = Pile {
            key: 1,
            path: "/var/log/syslog".to_owned(),
            size: 3 * 1_048_576,
            deleted: true,
            holders: vec![
                Holder {
                    id: id(1),
                    access: AccessMode::Read,
                    flow: Flow::Measured {
                        read: 2048.0,
                        written: 0.0,
                        either: 0.0,
                    },
                },
                Holder {
                    id: id(2),
                    access: AccessMode::Write,
                    flow: Flow::Unpositioned,
                },
                Holder {
                    id: id(3),
                    access: AccessMode::Both,
                    flow: Flow::Unmeasured,
                },
            ],
        };
        let snapshot = Snapshot::default();
        let (title, lines) = pile_card(&pile, &snapshot);
        assert_eq!(title, "/var/log/syslog");
        assert_eq!(lines[0], "size 3 MiB, deleted while open");
        assert!(
            lines[1].starts_with("readers 2, writers 2: pid 1, pid 2, pid 3"),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("moved at least 2 KiB/s read"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("1 without an earlier reading"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.contains("1 appending or seeking"))
        );
    }
}
