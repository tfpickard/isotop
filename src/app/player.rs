//! The samples kept for pause and rewind, and the one of them that is shown.

use std::collections::VecDeque;

use crate::model::Snapshot;

/// The history ring of snapshots and where the player stands in it: on the newest sample while
/// live, on a chosen older one while paused or rewound.
pub(crate) struct HistoryPlayer {
    /// The samples, oldest first. Never empty.
    ring: VecDeque<Snapshot>,
    paused: bool,
    /// The sample shown while paused.
    index: usize,
}

impl HistoryPlayer {
    /// A player holding `snapshot`, live.
    pub(crate) fn new(snapshot: Snapshot) -> Self {
        Self {
            ring: VecDeque::from([snapshot]),
            paused: false,
            index: 0,
        }
    }

    /// Whether the player is stopped on a sample instead of following the newest.
    pub(crate) fn is_paused(&self) -> bool {
        self.paused
    }

    /// How many samples are kept.
    pub(crate) fn len(&self) -> usize {
        self.ring.len()
    }

    /// The position in the ring of the sample shown: the chosen one while paused, else the
    /// newest.
    fn shown(&self) -> usize {
        if self.paused {
            self.index
        } else {
            self.ring.len() - 1
        }
    }

    /// The sample shown.
    pub(crate) fn snapshot(&self) -> &Snapshot {
        &self.ring[self.shown()]
    }

    /// The newest sample, whether or not it is the one shown.
    #[cfg(test)]
    pub(crate) fn newest(&self) -> &Snapshot {
        &self.ring[self.ring.len() - 1]
    }

    /// The newest sample, which the journal follower attaches each frame's lines to.
    pub(crate) fn newest_mut(&mut self) -> &mut Snapshot {
        let last = self.ring.len() - 1;
        &mut self.ring[last]
    }

    /// Takes in a new sample as the newest, keeping at most `capacity`. The sample it replaces as
    /// the newest lets go of its journal lines: they were drawn, and only the newest sample is
    /// ever fed more.
    pub(crate) fn push(&mut self, snapshot: Snapshot, capacity: usize) {
        self.newest_mut().journal.retire();
        self.ring.push_back(snapshot);
        if self.ring.len() > capacity {
            self.ring.pop_front();
        }
    }

    /// Keeps `snapshot` alone, as a headless warm-up does to draw a past second again.
    pub(crate) fn replace(&mut self, snapshot: Snapshot) {
        self.ring = VecDeque::from([snapshot]);
        self.index = 0;
    }

    /// Stops on the newest sample, or goes live again.
    pub(crate) fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        self.index = self.ring.len() - 1;
    }

    /// Steps one sample back, stopping first if the player was live.
    pub(crate) fn rewind(&mut self) {
        if !self.paused {
            self.index = self.ring.len() - 1;
            self.paused = true;
        }
        self.index = self.index.saturating_sub(1);
    }

    /// Steps one sample forward, up to the newest. Does nothing while live.
    pub(crate) fn forward(&mut self) {
        if self.paused {
            self.index = (self.index + 1).min(self.ring.len() - 1);
        }
    }

    /// The samples, oldest first.
    #[cfg(test)]
    pub(crate) fn samples(&self) -> &VecDeque<Snapshot> {
        &self.ring
    }

    /// The samples, oldest first, for a test to arrange.
    #[cfg(test)]
    pub(crate) fn samples_mut(&mut self) -> &mut VecDeque<Snapshot> {
        &mut self.ring
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model;

    fn player(samples: usize) -> HistoryPlayer {
        let mut player = HistoryPlayer::new(model::demo(0.0, 4));
        for second in 1..samples {
            player.push(model::demo(second as f64, 4), usize::MAX);
        }
        player
    }

    fn shown_second(player: &HistoryPlayer) -> f64 {
        player.snapshot().elapsed
    }

    #[test]
    fn a_live_player_shows_the_newest_sample_as_it_arrives() {
        let mut player = player(3);
        assert!(!player.is_paused());
        assert_eq!(shown_second(&player), 2.0);
        player.push(model::demo(3.0, 4), usize::MAX);
        assert_eq!(shown_second(&player), 3.0);
    }

    #[test]
    fn the_ring_keeps_at_most_its_capacity_of_the_newest_samples() {
        let mut player = player(1);
        for second in 1..10 {
            player.push(model::demo(f64::from(second), 4), 4);
        }
        assert_eq!(player.len(), 4);
        assert_eq!(player.samples()[0].elapsed, 6.0);
        assert_eq!(player.newest().elapsed, 9.0);
    }

    #[test]
    fn rewinding_stops_on_the_sample_before_the_newest_and_stays_inside_the_ring() {
        let mut player = player(5);
        player.rewind();
        assert!(player.is_paused());
        assert_eq!(shown_second(&player), 3.0);
        for _ in 0..10 {
            player.rewind();
        }
        assert_eq!(
            shown_second(&player),
            0.0,
            "the oldest is as far as it goes"
        );
        for _ in 0..10 {
            player.forward();
        }
        assert_eq!(shown_second(&player), 4.0, "and the newest as far forward");
        assert!(player.is_paused(), "stepping forward does not go live");
    }

    #[test]
    fn stepping_forward_does_nothing_while_live() {
        let mut player = player(3);
        player.forward();
        assert!(!player.is_paused());
        assert_eq!(shown_second(&player), 2.0);
    }

    #[test]
    fn pausing_stops_on_the_newest_sample_and_a_second_pause_goes_live() {
        let mut player = player(3);
        player.toggle_pause();
        assert!(player.is_paused());
        player.push(model::demo(3.0, 4), usize::MAX);
        assert_eq!(
            shown_second(&player),
            2.0,
            "paused, a new sample is not shown"
        );
        player.toggle_pause();
        assert!(!player.is_paused());
        assert_eq!(shown_second(&player), 3.0);
    }

    #[test]
    fn a_pushed_sample_retires_the_journal_lines_of_the_one_it_replaces() {
        let mut player = player(1);
        let mut totals = crate::journal::Totals::default();
        let lines = crate::journal::demo(0.0, 1.0);
        assert!(!lines.is_empty());
        player.newest_mut().journal.add(&mut totals, lines, 0);
        player.push(model::demo(1.0, 4), usize::MAX);
        assert!(player.samples()[0].journal.lines.is_empty());
        assert!(
            player.samples()[0].journal.through > 0,
            "the numbering stays"
        );
    }

    #[test]
    fn replacing_leaves_one_sample() {
        let mut player = player(4);
        player.replace(model::demo(9.0, 4));
        assert_eq!(player.len(), 1);
        assert_eq!(shown_second(&player), 9.0);
    }
}
