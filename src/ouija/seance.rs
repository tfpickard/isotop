//! The pure half of the ouija view: what the planchette spells and where it is, with no drawing.
//!
//! A `Seance` takes journal lines (`push`), queues them, and moves a planchette over the board
//! (`advance`) from visit to visit. A line is filtered to the characters the board has, uppercased
//! with everything else dropped (no transliteration), and a run of spaces becomes one visit to
//! the board's centre. A line that is an error, or the end of a failed job, visits NO first; one
//! that is a finished job visits YES. Each visit is a glide (smoothstep along a path bowed to
//! one side) and then a dwell; the pace follows the backlog. `Farewell` is the glide to GOODBYE
//! that plays when the app quits, a separate machine that nothing here ever starts.
//!
//! Everything is a function of the time fed to `advance`, carried across the end of one visit
//! into the next, so ten frames a second and sixty arrive at the same state at the same time.

use std::collections::VecDeque;

use super::layout::{Mark, Spot, spot};
use crate::journal::{JobResult, Line};
use crate::simulation::smoothstep;

/// The most lines waiting to be spelled; when more are queued the oldest are dropped.
pub const MOST_QUEUED: usize = 8;
/// The most board characters (visits) spelled for one line; the rest of its ribbon decodes at once.
pub const MOST_VISITS: usize = 40;
/// Seconds a visit takes with nothing else queued, and the fewest it ever takes.
const IDLE_PERIOD: f32 = 0.25;
const FASTEST_PERIOD: f32 = 0.06;
/// The share of a visit that is the glide; the rest is the dwell.
const GLIDE_SHARE: f32 = 0.6;
/// How far a glide bows sideways at its middle, as a share of its length.
const BOW: f32 = 0.12;
/// Seconds a visit to YES or NO lasts.
pub const VERDICT_SECONDS: f32 = 0.6;
/// How long the planchette waits where it is after the last line, and how long it then takes to
/// go back to the centre.
const LINGER_SECONDS: f32 = 1.2;
const SETTLE_SECONDS: f32 = 0.8;
/// How far through its glide a planchette is when the letter under its window lights.
const LIT_AT: f32 = 0.85;

/// Seconds the glide to GOODBYE takes, and the dwell there that ends the farewell, which together
/// are as long as the farewell lasts.
pub const FAREWELL_GLIDE: f32 = 0.85;
pub const FAREWELL_DWELL: f32 = 0.15;

/// A journal line as the board holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub text: String,
    pub priority: u8,
    pub job: Option<JobResult>,
}

impl Entry {
    pub fn new(line: &Line) -> Self {
        Self {
            text: line.text.clone(),
            priority: line.priority,
            job: line.job,
        }
    }

    /// The mark this line visits before it is spelled, if any: NO for a priority of 3 (error)
    /// or worse and for a job that did not finish, otherwise YES for a job that did. A line gets
    /// at most one.
    pub fn verdict(&self) -> Option<Mark> {
        if self.priority <= 3 || self.job.is_some_and(JobResult::failed) {
            Some(Mark::No)
        } else if self.job == Some(JobResult::Done) {
            Some(Mark::Yes)
        } else {
            None
        }
    }
}

/// One visit of the planchette.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    pub mark: Mark,
    /// How many characters of the line the ribbon has decoded once the planchette lands.
    pub reveal: usize,
    /// How long the visit lasts, when it does not follow the backlog.
    pub seconds: Option<f32>,
}

/// The visits that spell `entry`: its verdict, then a visit for each letter or digit with one
/// to the centre for each run of spaces between them, at most `MOST_VISITS` of those. A line with
/// nothing the board has visits the centre once, so the ribbon still decodes. The last visit
/// decodes the whole line, so what the cap leaves out and any trailing punctuation appear at once.
pub fn plan(entry: &Entry) -> Vec<Stop> {
    let length = entry.text.chars().count();
    let mut stops: Vec<Stop> = entry
        .verdict()
        .map(|mark| Stop {
            mark,
            reveal: 0,
            seconds: Some(VERDICT_SECONDS),
        })
        .into_iter()
        .collect();
    let verdicts = stops.len();
    let visit = |mark, reveal| Stop {
        mark,
        reveal,
        seconds: None,
    };
    let mut gap = false;
    for (index, character) in entry.text.chars().enumerate() {
        let board = character.to_ascii_uppercase();
        if character == ' ' {
            gap = stops.len() > verdicts;
        } else if board.is_ascii_uppercase() || board.is_ascii_digit() {
            if stops.len() - verdicts + usize::from(gap) >= MOST_VISITS {
                break;
            }
            if std::mem::take(&mut gap) {
                stops.push(visit(Mark::Rest, index));
            }
            stops.push(visit(Mark::Letter(board), index + 1));
        }
    }
    if stops.len() == verdicts {
        stops.push(visit(Mark::Rest, length));
    }
    if let Some(last) = stops.last_mut() {
        last.reveal = length;
    }
    stops
}

/// Seconds one visit takes with `queued` lines waiting: a quarter of a second, faster the longer
/// the queue, never under 0.06.
pub fn period(queued: usize) -> f32 {
    (IDLE_PERIOD / (1.0 + queued as f32 / 4.0)).max(FASTEST_PERIOD)
}

/// One leg of the planchette's journey: where it goes, over how long, and when in that time it
/// glides.
#[derive(Clone, Debug)]
struct Leg {
    from: Spot,
    to: Spot,
    seconds: f32,
    elapsed: f32,
    /// The fractions of the leg at which the glide starts and ends.
    glide: (f32, f32),
    /// The mark the leg lands on, and the fractions of the leg in which it is lit.
    lit: Option<(Mark, (f32, f32))>,
    /// How many characters of the line the ribbon decodes once the glide ends; None for a leg
    /// that decodes nothing. Taken when applied.
    reveal: Option<usize>,
    settling: bool,
}

impl Leg {
    /// A planchette that stays at `at`, with nothing to do.
    fn hold(at: Spot) -> Self {
        Self {
            from: at,
            to: at,
            seconds: 0.0,
            elapsed: 0.0,
            glide: (0.0, 1.0),
            lit: None,
            reveal: None,
            settling: false,
        }
    }

    fn done(&self) -> bool {
        self.elapsed >= self.seconds
    }

    fn fraction(&self) -> f32 {
        if self.seconds > 0.0 {
            (self.elapsed / self.seconds).min(1.0)
        } else {
            1.0
        }
    }

    /// Where the planchette is: along the glide with smoothstep, bowed sideways.
    fn position(&self) -> Spot {
        let (start, end) = self.glide;
        let along = ((self.fraction() - start) / (end - start)).clamp(0.0, 1.0);
        glide(self.from, self.to, smoothstep(along))
    }

    fn lit(&self) -> Option<Mark> {
        let (mark, (from, until)) = self.lit?;
        let fraction = self.fraction();
        (fraction >= from && fraction < until).then_some(mark)
    }
}

/// The point `eased` of the way from `from` to `to`, bowed sideways by `BOW` of the distance at
/// the middle, to the left of the direction of travel.
fn glide(from: Spot, to: Spot, eased: f32) -> Spot {
    // The ends are exact: the planchette lands on its letter, not within rounding of it.
    if eased <= 0.0 {
        return from;
    }
    if eased >= 1.0 {
        return to;
    }
    let delta = [to[0] - from[0], to[1] - from[1]];
    let bow = (std::f32::consts::PI * eased).sin() * BOW;
    [
        from[0] + delta[0] * eased - delta[1] * bow,
        from[1] + delta[1] * eased + delta[0] * bow,
    ]
}

/// The line being spelled, or the last one that was.
#[derive(Clone, Debug)]
pub struct Reading {
    pub entry: Entry,
    stops: Vec<Stop>,
    next: usize,
    /// How many characters of the entry have decoded.
    pub decoded: usize,
}

impl Reading {
    fn begin(entry: Entry) -> Self {
        let stops = plan(&entry);
        Self {
            entry,
            stops,
            next: 0,
            decoded: 0,
        }
    }

    /// Whether every visit has been made.
    pub fn finished(&self) -> bool {
        self.next >= self.stops.len()
    }
}

/// The board's state: lines waiting, the line being spelled and the planchette's leg.
pub struct Seance {
    queue: VecDeque<Entry>,
    reading: Option<Reading>,
    leg: Leg,
    skipped: u64,
    newest: Option<String>,
}

impl Default for Seance {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            reading: None,
            leg: Leg::hold(spot(Mark::Rest)),
            skipped: 0,
            newest: None,
        }
    }
}

impl Seance {
    /// Queues `line` to be spelled. Past `MOST_QUEUED` waiting lines the oldest are dropped and
    /// counted as skipped.
    pub fn push(&mut self, line: &Line) {
        self.newest = Some(line.text.clone());
        self.queue.push_back(Entry::new(line));
        while self.queue.len() > MOST_QUEUED {
            self.queue.pop_front();
            self.skipped += 1;
        }
        // A planchette on its way back to the centre turns to the new line from where it is.
        if self.leg.settling && !self.leg.done() {
            self.leg = Leg::hold(self.leg.position());
        }
    }

    /// Counts `lines` as skipped without queueing them, for lines that were lost before they
    /// reached the board.
    pub fn skip(&mut self, lines: u64) {
        self.skipped += lines;
    }

    /// Moves the planchette on by `dt` seconds. The time left at the end of a visit goes to the
    /// next, so the state depends on the total fed and not on how it was cut into frames.
    pub fn advance(&mut self, dt: f32) {
        let mut left = dt.max(0.0);
        loop {
            let remaining = self.leg.seconds - self.leg.elapsed;
            if left < remaining {
                self.leg.elapsed += left;
                self.arrive();
                return;
            }
            left -= remaining.max(0.0);
            self.leg.elapsed = self.leg.seconds;
            self.arrive();
            if !self.depart() {
                return;
            }
        }
    }

    /// Hands the ribbon what the leg decodes once its glide has ended.
    fn arrive(&mut self) {
        if self.leg.fraction() >= self.leg.glide.1
            && let Some(reveal) = self.leg.reveal.take()
            && let Some(reading) = &mut self.reading
        {
            reading.decoded = reading.decoded.max(reveal);
        }
    }

    /// Starts the next leg: the next visit of the line being spelled, the first of the next line
    /// in the queue, or the way back to the centre. False when there is nothing to do.
    fn depart(&mut self) -> bool {
        let from = self.leg.to;
        loop {
            if let Some(reading) = &mut self.reading
                && let Some(&stop) = reading.stops.get(reading.next)
            {
                reading.next += 1;
                let seconds = stop.seconds.unwrap_or_else(|| period(self.queue.len()));
                self.leg = Leg {
                    from,
                    to: spot(stop.mark),
                    seconds,
                    elapsed: 0.0,
                    glide: (0.0, GLIDE_SHARE),
                    lit: (stop.mark != Mark::Rest)
                        .then_some((stop.mark, (GLIDE_SHARE * LIT_AT, 1.0))),
                    reveal: Some(stop.reveal),
                    settling: false,
                };
                return true;
            }
            match self.queue.pop_front() {
                Some(entry) => self.reading = Some(Reading::begin(entry)),
                None => return self.settle(from),
            }
        }
    }

    /// Lingers where the planchette is and then returns to the centre; false when it is there.
    fn settle(&mut self, from: Spot) -> bool {
        let centre = spot(Mark::Rest);
        if from == centre {
            self.leg = Leg::hold(centre);
            return false;
        }
        let seconds = LINGER_SECONDS + SETTLE_SECONDS;
        let start = LINGER_SECONDS / seconds;
        self.leg = Leg {
            from,
            to: centre,
            seconds,
            elapsed: 0.0,
            glide: (start, 1.0),
            // The letter stays lit while the planchette lingers over it.
            lit: self.leg.lit.map(|(mark, _)| (mark, (0.0, start))),
            reveal: None,
            settling: true,
        };
        true
    }

    /// Where the planchette's window is.
    pub fn position(&self) -> Spot {
        self.leg.position()
    }

    /// The mark under the window once the planchette has all but landed on it, if it is one the
    /// board writes (the centre is not).
    pub fn lit(&self) -> Option<Mark> {
        self.leg.lit()
    }

    /// Lines waiting to be spelled, the one being spelled not among them.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// Lines lost to the cap on the queue, and those counted by `skip`.
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// The newest line pushed, spelled yet or not.
    pub fn newest(&self) -> Option<&str> {
        self.newest.as_deref()
    }

    /// The line being spelled, or the last that was, with how much of it has decoded.
    pub fn reading(&self) -> Option<&Reading> {
        self.reading.as_ref()
    }

    /// The seconds the current leg has run and lasts; for tests that check the pace.
    #[cfg(test)]
    pub fn leg_seconds(&self) -> f32 {
        self.leg.seconds
    }
}

/// How far the planchette trembles at `time` for a CPU pressure of `percent`, in board widths:
/// up to 0.025 board widths at 100 %, the sum of four sinusoids of the prototype (13.1, 29.3, 11.7
/// and 31.9 radians a second, which its `math.sin(t * 13.1)` takes them to be, not hertz),
/// normalised so that each axis peaks at one.
pub fn jitter(time: f32, percent: f32) -> Spot {
    let amplitude = 0.025 * percent.clamp(0.0, 100.0) / 100.0;
    [
        amplitude * (7.0 * (time * 13.1).sin() + 4.0 * (time * 29.3 + 1.0).sin()) / 11.0,
        amplitude * (6.0 * (time * 11.7 + 2.0).sin() + 4.0 * (time * 31.9).sin()) / 10.0,
    ]
}

/// The glide to GOODBYE that plays when the app quits: a smoothstep glide from where the planchette
/// was, bowed as every glide is, and a short dwell, after which the app may end. It starts only
/// when `press` is called, by the quit key of an interactive run, so nothing that merely draws
/// frames (a headless render, a benchmark) ever runs it.
#[derive(Clone, Debug, Default)]
pub struct Farewell {
    leaving: Option<Leaving>,
}

#[derive(Clone, Copy, Debug)]
struct Leaving {
    from: Spot,
    elapsed: f32,
}

impl Farewell {
    /// The quit key was pressed with the planchette at `from`. The first press takes the quit
    /// (true: the app waits for the farewell) and a press during the farewell lets it through
    /// (false), as does a farewell that has already played.
    pub fn press(&mut self, from: Spot) -> bool {
        if self.leaving.is_some() {
            return false;
        }
        self.leaving = Some(Leaving { from, elapsed: 0.0 });
        true
    }

    /// Plays `dt` seconds of the farewell; true once it is over.
    pub fn advance(&mut self, dt: f32) -> bool {
        match &mut self.leaving {
            Some(leaving) => {
                leaving.elapsed += dt.max(0.0);
                leaving.elapsed >= FAREWELL_GLIDE + FAREWELL_DWELL
            }
            None => false,
        }
    }

    /// Where the farewell has the planchette, if one is playing.
    pub fn position(&self) -> Option<Spot> {
        let leaving = self.leaving?;
        let along = smoothstep(leaving.elapsed / FAREWELL_GLIDE);
        Some(glide(leaving.from, spot(Mark::Goodbye), along))
    }

    /// Whether the planchette has reached GOODBYE, so that the word lights.
    pub fn arrived(&self) -> bool {
        self.leaving
            .is_some_and(|leaving| leaving.elapsed >= FAREWELL_GLIDE * LIT_AT)
    }

    /// Forgets a farewell that was left unfinished, when the view was hidden in the middle of it.
    pub fn cancel(&mut self) {
        self.leaving = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str) -> Line {
        Line::new(6, text)
    }

    fn marks(entry: &Entry) -> Vec<Mark> {
        plan(entry).iter().map(|stop| stop.mark).collect()
    }

    fn letters(text: &str) -> String {
        marks(&Entry::new(&line(text)))
            .iter()
            .map(|mark| match mark {
                Mark::Letter(character) => *character,
                Mark::Rest => '_',
                other => panic!("{other:?}"),
            })
            .collect()
    }

    /// Runs the seance for `seconds` in frames of `step`.
    fn run(seance: &mut Seance, seconds: f32, step: f32) {
        let frames = (seconds / step).round() as u32;
        for _ in 0..frames {
            seance.advance(step);
        }
    }

    #[test]
    fn only_letters_and_digits_reach_the_board_and_spaces_return_to_centre() {
        assert_eq!(
            letters("kernel: nvme0: I/O 42 QID 6 timeout"),
            "KERNEL_NVME0_IO_42_QID_6_TIMEOUT"
        );
        // Everything else is dropped, with no transliteration: an accent loses its letter.
        assert_eq!(letters("caf\u{e9} \u{65e5}\u{672c} #!?"), "CAF");
        // A run of spaces is one visit, and so is one that other dropped characters split.
        assert_eq!(letters("a   b"), "A_B");
        assert_eq!(letters("a - b"), "A_B");
        assert_eq!(letters("a\tb"), "AB", "a tab is not a space");
        // A line neither starts nor ends at the centre.
        assert_eq!(letters("   hello   "), "HELLO");
        // A line with nothing the board has visits the centre once.
        assert_eq!(letters("!!! ???"), "_");
        assert_eq!(letters(""), "_");
        // Case does not matter, and the characters the terminal could act on are just dropped.
        assert_eq!(letters("aBc\x1b[2J\u{202e}d"), "ABC2JD");
        // The marks are those of the board.
        assert!(marks(&Entry::new(&line("x9"))).iter().all(
            |mark| matches!(mark, Mark::Letter(c) if c.is_ascii_uppercase() || c.is_ascii_digit())
        ));
    }

    #[test]
    fn a_line_is_cut_at_forty_visits_and_the_rest_decodes_at_once() {
        let text = "abcdefghij ".repeat(8);
        let stops = plan(&Entry::new(&line(&text)));
        assert_eq!(stops.len(), MOST_VISITS);
        let length = text.chars().count();
        assert_eq!(
            stops.last().unwrap().reveal,
            length,
            "the whole line decodes on the last visit"
        );
        assert!(
            stops[..MOST_VISITS - 1]
                .iter()
                .all(|stop| stop.reveal < length)
        );
        // Each visit decodes up to its own character, so the ribbon follows the planchette.
        assert_eq!(stops[0].reveal, 1);
        assert_eq!(stops[9].reveal, 10);
        assert_eq!(stops[10].mark, Mark::Rest);
        assert_eq!(stops[11].reveal, 12);
        // A short line decodes completely at its last visit, punctuation and all.
        let short = plan(&Entry::new(&line("ok.")));
        assert_eq!(short.last().unwrap().reveal, 3);
    }

    #[test]
    fn an_error_line_visits_no_before_it_is_spelled() {
        for (priority, visits) in [(0, true), (3, true), (4, false), (6, false)] {
            let entry = Entry::new(&Line::new(priority, "disk gone"));
            let stops = plan(&entry);
            assert_eq!(stops[0].mark == Mark::No, visits, "priority {priority}");
            if visits {
                assert_eq!(stops[0].seconds, Some(VERDICT_SECONDS));
                assert_eq!(stops[0].reveal, 0, "nothing decodes at NO");
                assert_eq!(stops[1].mark, Mark::Letter('D'));
            }
        }
        // The visit comes first: the planchette is at NO, not on a letter, when it has glided.
        let mut seance = Seance::default();
        seance.push(&Line::new(3, "disk"));
        seance.advance(0.0);
        run(&mut seance, VERDICT_SECONDS * 0.95, 0.01);
        assert_eq!(seance.lit(), Some(Mark::No));
        let at = seance.position();
        let no = spot(Mark::No);
        assert!(
            ((at[0] - no[0]).hypot(at[1] - no[1])) < 1e-4,
            "{at:?} is not at NO"
        );
        assert_eq!(seance.reading().unwrap().decoded, 0);
    }

    #[test]
    fn a_finished_job_touches_yes_and_a_failed_job_touches_no() {
        let job = |priority, result| {
            Entry::new(&Line::new(priority, "systemd[1]: job").with_job(Some(result)))
        };
        let first = |entry: &Entry| plan(entry)[0].mark;
        assert_eq!(first(&job(6, JobResult::Done)), Mark::Yes);
        for result in [
            JobResult::Failed,
            JobResult::Timeout,
            JobResult::Canceled,
            JobResult::Dependency,
        ] {
            assert_eq!(first(&job(6, result)), Mark::No, "{result:?}");
        }
        assert_eq!(
            first(&job(6, JobResult::Skipped)),
            Mark::Letter('S'),
            "neither"
        );
        // One visit at most: an error that also reports a finished job goes to NO and not to YES.
        let both = plan(&job(3, JobResult::Done));
        assert_eq!(
            both.iter()
                .filter(|stop| matches!(stop.mark, Mark::Yes | Mark::No))
                .count(),
            1
        );
        assert_eq!(both[0].mark, Mark::No);
        let yes = plan(&job(6, JobResult::Done));
        assert_eq!(yes[0].seconds, Some(VERDICT_SECONDS));
        assert_eq!(yes.iter().filter(|stop| stop.mark == Mark::No).count(), 0);
    }

    #[test]
    fn a_longer_queue_spells_faster_and_a_flood_skips_and_counts_lines() {
        assert_eq!(period(0), 0.25);
        assert!((period(4) - 0.125).abs() < 1e-6);
        assert!(period(1) > period(2) && period(2) > period(8));
        assert!((period(8) - 0.25 / 3.0).abs() < 1e-6);
        assert_eq!(period(1000), FASTEST_PERIOD, "the floor");
        let mut seance = Seance::default();
        for number in 0..20 {
            seance.push(&line(&format!("line {number}")));
        }
        assert_eq!((seance.queued(), seance.skipped()), (MOST_QUEUED, 12));
        assert_eq!(seance.newest(), Some("line 19"));
        // The oldest were the ones dropped: the first line spelled is the thirteenth.
        seance.advance(0.01);
        assert_eq!(seance.reading().unwrap().entry.text, "line 12");
        // The first visit's period is that of the lines left waiting.
        assert_eq!(seance.leg_seconds(), period(MOST_QUEUED - 1));
        seance.skip(5);
        assert_eq!(seance.skipped(), 17);
    }

    #[test]
    fn a_visit_glides_for_sixty_percent_of_its_period_and_dwells_for_the_rest() {
        let mut seance = Seance::default();
        seance.push(&line("ab"));
        seance.advance(0.0);
        let start = seance.position();
        let a = spot(Mark::Letter('A'));
        let period = seance.leg_seconds();
        assert_eq!(period, 0.25, "nothing else is queued");
        seance.advance(period * 0.3);
        let middle = seance.position();
        assert!(middle != start && middle != a, "midway through the glide");
        // The glide bows to the left of the way to the letter, by up to 0.12 of the distance.
        let chord = (a[1] - start[1]).hypot(a[0] - start[0]);
        let off = {
            let (dx, dy) = ((a[0] - start[0]) / chord, (a[1] - start[1]) / chord);
            let (mx, my) = (middle[0] - start[0], middle[1] - start[1]);
            dx * my - dy * mx
        };
        assert!(off > 0.0 && off <= 0.12 * chord + 1e-5, "{off} of {chord}");
        seance.advance(period * 0.31);
        assert_eq!(seance.position(), a, "the glide ends at 60 %");
        seance.advance(period * 0.3);
        assert_eq!(seance.position(), a, "and the planchette dwells");
        assert_eq!(seance.lit(), Some(Mark::Letter('A')));
    }

    #[test]
    fn the_ribbon_decodes_as_the_planchette_lands() {
        let mut seance = Seance::default();
        seance.push(&line("go on"));
        seance.advance(0.0);
        assert_eq!(seance.reading().unwrap().decoded, 0);
        seance.advance(0.25 * GLIDE_SHARE + 0.001);
        assert_eq!(seance.reading().unwrap().decoded, 1, "G has landed");
        run(&mut seance, 0.25 * 3.0, 0.01);
        assert!(seance.reading().unwrap().decoded >= 3);
        run(&mut seance, 3.0, 0.05);
        let reading = seance.reading().unwrap();
        assert!(
            reading.finished() && reading.decoded == 5,
            "the whole line decoded"
        );
    }

    #[test]
    fn the_planchette_lingers_then_returns_to_the_centre_and_a_new_line_interrupts_it() {
        let mut seance = Seance::default();
        seance.push(&line("o"));
        run(&mut seance, 0.6, 0.01);
        assert_eq!(seance.position(), spot(Mark::Letter('O')));
        assert_eq!(
            seance.lit(),
            Some(Mark::Letter('O')),
            "lit while it lingers"
        );
        run(&mut seance, LINGER_SECONDS - 0.4, 0.01);
        assert_eq!(
            seance.position(),
            spot(Mark::Letter('O')),
            "it has not left yet"
        );
        run(&mut seance, SETTLE_SECONDS + 0.3, 0.01);
        assert_eq!(seance.position(), spot(Mark::Rest));
        assert_eq!(seance.lit(), None);
        // On its way back, a new line is taken up from where the planchette is.
        seance.push(&line("x"));
        run(&mut seance, 1.8, 0.01);
        let underway = seance.position();
        assert!(underway != spot(Mark::Letter('X')) && underway != spot(Mark::Rest));
        seance.push(&line("z"));
        assert_eq!(seance.position(), underway, "no jump when interrupted");
        seance.advance(0.01);
        assert_eq!(seance.reading().unwrap().entry.text, "z");
    }

    #[test]
    fn the_state_depends_on_the_time_fed_and_not_on_the_frames_it_was_cut_into() {
        let feed = |step: f32| {
            let mut seance = Seance::default();
            for text in ["kernel: nvme0 timeout", "ssh ok"] {
                seance.push(&Line::new(3, text));
            }
            run(&mut seance, 6.0, step);
            seance
        };
        let (slow, fast) = (feed(0.1), feed(1.0 / 60.0));
        let (a, b) = (slow.position(), fast.position());
        assert!(
            (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3,
            "{a:?} {b:?}"
        );
        assert_eq!(slow.reading().unwrap().entry, fast.reading().unwrap().entry);
        assert_eq!(
            slow.reading().unwrap().decoded,
            fast.reading().unwrap().decoded
        );
        assert_eq!((slow.queued(), slow.lit()), (fast.queued(), fast.lit()));
    }

    #[test]
    fn jitter_follows_pressure_and_is_zero_without_it() {
        assert_eq!(jitter(3.7, 0.0), [0.0, 0.0]);
        let peak = |percent: f32| {
            (0..2000)
                .map(|step| jitter(step as f32 * 0.01, percent))
                .fold([0.0_f32; 2], |peak, spot| {
                    [peak[0].max(spot[0].abs()), peak[1].max(spot[1].abs())]
                })
        };
        let full = peak(100.0);
        assert!(full[0] > 0.015 && full[0] <= 0.025 + 1e-6, "{full:?}");
        assert!(full[1] > 0.015 && full[1] <= 0.025 + 1e-6, "{full:?}");
        let quarter = peak(25.0);
        assert!(
            (quarter[0] / full[0] - 0.25).abs() < 1e-3,
            "proportional to pressure"
        );
        assert_eq!(peak(1000.0), full, "pressure is a percentage");
        assert_eq!(
            jitter(1.0, 40.0),
            jitter(1.0, 40.0),
            "a function of the time"
        );
    }

    #[test]
    fn the_farewell_glides_to_goodbye_and_a_second_press_lets_the_quit_through() {
        let mut farewell = Farewell::default();
        assert!(farewell.position().is_none());
        assert!(!farewell.advance(100.0), "it never starts by itself");
        let from = spot(Mark::Letter('M'));
        assert!(farewell.press(from), "the first press takes the quit");
        assert_eq!(farewell.position(), Some(from));
        assert!(!farewell.advance(FAREWELL_GLIDE * 0.5));
        let middle = farewell.position().unwrap();
        assert!(middle != from && middle != spot(Mark::Goodbye));
        assert!(!farewell.advance(FAREWELL_GLIDE * 0.5 + 0.01));
        let goodbye = spot(Mark::Goodbye);
        let at = farewell.position().unwrap();
        assert!((at[0] - goodbye[0]).abs() < 1e-4 && (at[1] - goodbye[1]).abs() < 1e-4);
        assert!(farewell.arrived());
        assert!(!farewell.press(from), "a second press ends it at once");
        assert!(
            farewell.advance(FAREWELL_DWELL),
            "over after its glide and its dwell"
        );
        // A farewell that was left half played is forgotten.
        let mut left = Farewell::default();
        left.press(from);
        left.advance(0.3);
        left.cancel();
        assert!(left.position().is_none() && !left.advance(10.0));
        assert!(left.press(from), "and can be asked for again");
    }
}
