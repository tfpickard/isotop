//! The wording a legend uses for optional sources, so every view says it the same way.

use crate::model::Snapshot;
use crate::platform::Source;

/// ` | no <source>: <consequence>` when `source` is missing from `snapshot`, which the legend
/// appends where it names what the view does without; None when the source is there or was never
/// asked for.
pub fn missing_note(snapshot: &Snapshot, source: Source, consequence: &str) -> Option<String> {
    snapshot
        .missing
        .contains(source)
        .then(|| format!(" | no {}: {consequence}", source.name()))
}

/// ` | sources: a, b` naming the optional sources read for `snapshot`, in `Source` declaration
/// order; None when there are none.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "no legend uses it until the first new view")
)]
pub fn sources_note(snapshot: &Snapshot) -> Option<String> {
    let names: Vec<&str> = snapshot.sources.iter().map(Source::name).collect();
    (!names.is_empty()).then(|| format!(" | sources: {}", names.join(", ")))
}

/// ` | <what> unreadable for <count>` when `count` processes could not be measured for `what`,
/// a per-process gap such as I/O that the kernel refuses for other users' processes; None when
/// every process was measured.
pub fn unreadable_note(what: &str, count: usize) -> Option<String> {
    (count > 0).then(|| format!(" | {what} unreadable for {count}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Sources;

    #[test]
    fn missing_note_appears_only_for_missing_sources() {
        let mut snapshot = Snapshot::default();
        assert_eq!(
            missing_note(&snapshot, Source::SyscallIo, "no read and write bytes"),
            None
        );
        snapshot.missing = Source::SocketTraffic.into();
        assert_eq!(
            missing_note(&snapshot, Source::SyscallIo, "no read and write bytes"),
            None,
            "another source is missing"
        );
        assert_eq!(
            missing_note(
                &snapshot,
                Source::SocketTraffic,
                "corridors from socket counts"
            ),
            Some(" | no socket traffic: corridors from socket counts".to_owned())
        );
    }

    #[test]
    fn sources_note_lists_active_sources_in_order_and_is_absent_when_none() {
        let mut snapshot = Snapshot::default();
        assert_eq!(sources_note(&snapshot), None);
        snapshot.sources = Sources::from(Source::Journal);
        assert_eq!(
            sources_note(&snapshot),
            Some(" | sources: journal".to_owned())
        );
        // Inserted out of order; listed in the order `Source` declares them.
        snapshot.sources.insert(Source::Locations);
        snapshot.sources.insert(Source::CpuClock);
        assert_eq!(
            sources_note(&snapshot),
            Some(" | sources: cpu clock, remote locations, journal".to_owned())
        );
    }

    #[test]
    fn unreadable_note_counts_processes_and_is_absent_when_all_were_measured() {
        assert_eq!(unreadable_note("I/O", 0), None);
        assert_eq!(
            unreadable_note("I/O", 12),
            Some(" | I/O unreadable for 12".to_owned())
        );
    }
}
