//! The journal lines the Matrix view draws: when the follower should run, and what each frame
//! attaches to the newest sample.

use std::time::{Duration, Instant};

use crate::journal::{self, Journal, Line};
use crate::model::Snapshot;
use crate::platform::Source;

use super::App;

/// How long the journal follower keeps running after the demand last contained
/// `Source::Journal` (Matrix was shown), so a look at another view does not restart it, and with
/// it the rain's backlog.
pub(crate) const JOURNAL_LINGER: Duration = Duration::from_secs(30);

/// The follower's schedule and the numbering of the lines handed over.
#[derive(Default)]
pub(crate) struct JournalFeed {
    /// When the demand last contained `Source::Journal`; None while it has not.
    demanded: Option<Instant>,
    /// What has been attached to samples so far, which numbers the next lines.
    totals: journal::Totals,
}

impl JournalFeed {
    /// Whether the follower should run at `now`: while `demanded` (the shown view reads the
    /// journal) and for `JOURNAL_LINGER` after it last was.
    pub(crate) fn wanted(&mut self, demanded: bool, now: Instant) -> bool {
        if demanded {
            self.demanded = Some(now);
        }
        self.demanded
            .is_some_and(|shown| now.saturating_duration_since(shown) < JOURNAL_LINGER)
    }

    /// Attaches `lines` that just arrived, and the `dropped` count the follower reported, to
    /// `newest`, the sample a view draws them from.
    pub(crate) fn attach(&mut self, newest: &mut Snapshot, lines: Vec<Line>, dropped: u64) {
        newest.journal.add(&mut self.totals, lines, dropped);
    }
}

impl App {
    /// Whether the journal follower should run at `now`: while the demand contains
    /// `Source::Journal` and for `JOURNAL_LINGER` after it last did.
    pub(crate) fn journal_wanted(&mut self, now: Instant) -> bool {
        let demanded = self.demand().contains(Source::Journal);
        self.journal.wanted(demanded, now)
    }

    /// Attaches the journal lines of this frame to the newest sample (`Snapshot::journal`),
    /// where the view that draws them finds them: synthetic ones arriving in `(from, to]` demo
    /// seconds, or whatever `journalctl` has written since the last call.
    pub(crate) fn feed_journal(
        &mut self,
        source: Option<&Journal>,
        demo: bool,
        from: f64,
        to: f64,
    ) {
        let lines = match (demo, source) {
            (true, _) => journal::demo(from, to),
            (false, Some(source)) => source.drain(),
            (false, None) => Vec::new(),
        };
        let dropped = source.map_or(0, Journal::dropped);
        if !lines.is_empty() || dropped > 0 {
            self.journal
                .attach(self.history.newest_mut(), lines, dropped);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;

    #[test]
    fn matrix_status_ignores_a_process_selected_in_another_view() {
        let snapshot = model::demo(1.0, 16);
        let selected = snapshot.processes[0].id;
        let mut app = App::new(view("matrix"), snapshot, all_available());
        app.selected = Some(selected);
        let App {
            history, journal, ..
        } = &mut app;
        journal.attach(
            &mut history.samples_mut()[0],
            vec![journal::Line::new(6, "sshd[1]: hello")],
            0,
        );
        // The line reaches the view when it draws the sample that carries it.
        let frame = app.render(320, 180, 512);
        let text = app.text(0.0, "test", 20, 512);
        assert_eq!(text[1], " > sshd[1]: hello");
        assert!(text[2].contains("queued"), "{}", text[2]);
        let layout = Layout {
            columns: 80,
            rows: 24,
            width: 320,
            height: 180,
            cell: None,
        };
        assert!(app.popup(&frame, &layout).is_none());
        assert_eq!(app.selected, Some(selected), "kept for the process views");
    }

    #[test]
    fn matrix_lines_reach_the_view_through_the_snapshot() {
        let mut app = App::new(view("matrix"), model::demo(1.0, 16), all_available());
        let line = |app: &App| {
            app.scene
                .inspector_line(app.view)
                .expect("the matrix has one")
        };
        let legend = |app: &App| app.scene.legend(app.view, app.snapshot());
        app.render(320, 180, 512);
        assert_eq!(line(&app), " > waiting for the journal...");

        // `main.rs` hands the frame's lines to the sample; the view takes them in when it draws.
        app.feed_journal(None, true, 0.0, 1.0);
        let lines = journal::demo(0.0, 1.0);
        assert_eq!(app.history.newest().journal.lines, lines);
        assert_eq!(line(&app), " > waiting for the journal...", "not drawn yet");
        app.render(320, 180, 512);
        assert_eq!(line(&app), format!(" > {}", lines.last().unwrap().text));
        let queued = legend(&app);

        // Drawing the same sample again, or a newer one that brought no lines, queues none twice.
        app.render(320, 180, 512);
        app.history.samples_mut().push_back(model::demo(2.0, 16));
        app.render(320, 180, 512);
        assert_eq!(legend(&app), queued);
        assert_eq!(line(&app), format!(" > {}", lines.last().unwrap().text));

        // Lines attached to the newer sample are the next ones taken.
        app.feed_journal(None, true, 1.0, 2.0);
        app.render(320, 180, 512);
        let all = journal::demo(0.0, 2.0);
        assert_eq!(line(&app), format!(" > {}", all.last().unwrap().text));
        assert_eq!(app.history.newest().journal.through as usize, all.len());
    }

    #[test]
    fn feed_attaches_each_frames_journal_lines_to_the_newest_sample() {
        let mut app = App::new(view("matrix"), model::demo(1.0, 16), all_available());
        app.history.samples_mut().push_front(model::demo(0.0, 16));
        app.feed_journal(None, true, 0.0, 1.0);
        assert!(
            app.history.samples()[0].journal.lines.is_empty(),
            "an older sample gets none"
        );
        assert_eq!(
            app.history.samples()[1].journal.lines,
            journal::demo(0.0, 1.0)
        );
        app.feed_journal(None, true, 1.0, 2.0);
        let newest = &app.history.samples()[1].journal;
        assert_eq!(newest.lines, journal::demo(0.0, 2.0));
        assert_eq!(newest.through as usize, newest.lines.len());
        // A frame that brought nothing leaves the sample as it was.
        let before = newest.clone();
        app.feed_journal(None, true, 2.0, 2.05);
        assert_eq!(app.history.samples()[1].journal, before);
    }

    #[test]
    fn the_history_keeps_the_journal_lines_of_the_newest_sample_only() {
        let mut app = App::new(view("matrix"), model::demo(1.0, 16), all_available());
        let held = |app: &App| -> usize {
            app.history
                .samples()
                .iter()
                .map(|s| s.journal.lines.len())
                .sum()
        };
        for sample in 0..40 {
            app.push_sample(model::demo(f64::from(sample), 16), 24);
            // A flood bigger than a sample keeps arrives every frame, twice.
            for _ in 0..2 {
                let flood: Vec<journal::Line> = (0..journal::KEPT)
                    .map(|n| journal::Line::new(6, format!("line {n}")))
                    .collect();
                let App {
                    history, journal, ..
                } = &mut app;
                journal.attach(history.newest_mut(), flood, 0);
            }
            assert!(held(&app) <= journal::KEPT, "{} lines held", held(&app));
        }
        assert_eq!(app.history.len(), 24);
        assert_eq!(held(&app), journal::KEPT, "the newest sample's lines");
        // The numbering outlives the lines: a sample that was the newest says how many had
        // arrived by then, and offers none of them to a view that has not taken them.
        for older in app.history.samples().iter().take(23) {
            assert!(older.journal.lines.is_empty());
            assert!(older.journal.through > 0);
            assert!(older.journal.unseen(0).is_empty());
        }
    }

    #[test]
    fn the_journal_stops_after_matrix_has_been_hidden_for_a_while() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        assert!(!app.journal_wanted(at(0)), "Matrix was never shown");
        app.view = view("matrix");
        assert!(app.journal_wanted(at(10)));
        assert!(app.journal_wanted(at(500)), "shown all along");
        app.view = view("globe");
        assert!(app.journal_wanted(at(501)));
        assert!(app.journal_wanted(at(529)), "a look at another view");
        assert!(!app.journal_wanted(at(530)));
        assert!(!app.journal_wanted(at(900)));
        app.view = view("matrix");
        assert!(app.journal_wanted(at(901)), "back on Matrix");
    }
}
