//! The shared view harness: the behaviour every registered view must show, checked for each row
//! of the registry by `every_view_satisfies_the_view_contract`, so a view is covered by being
//! registered. A row's `Contract` sets how far its processes may move, whether they are checked
//! where they are drawn or at their anchors, and which properties it is known to fail.
//!
//! Every run is a headless `--output` run at `TIME`: the row's warm-up (`warm_up`), then frames
//! drawn by `App::render` and rasterized on the CPU. The stability properties compare two runs
//! that differ only in the sample after the first frame, frame for frame at the same times, so
//! whatever moves by itself (orbits, walking chickens) moves alike in both.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use clap::Parser;

use crate::model::{Blocker, Identity, Measured, Process, Snapshot};
use crate::render::{Frame, NONE, Point, Scene};
use crate::simulation::{Rng, hash_identity};
use crate::views::{Property, Unstable, View, ViewInfo};
use crate::{App, Options, demo_snapshot, warm_up};

/// The frame width, the smallest `--width` takes; the height follows at 16:9. Small, so the
/// harness stays quick in a debug build and under emulation.
const WIDTH: u32 = 160;
/// The demo time of the first frame: the default reference time.
const TIME: f64 = 12.0;
/// The demo population, `--processes`' default.
const PROCESSES: usize = 192;
/// Frames drawn after the changed sample arrives, and the seconds between them: three seconds,
/// long after the scene's 0.3 s easing of CPU and memory has settled.
const AFTER_FRAMES: u32 = 12;
const AFTER_STEP: f32 = 0.25;
/// Separates the harness's per-process draws from every other use of the identity hash.
const SALT: u64 = 0x6861_7265_7373;
/// The first pid of the processes `churned` starts: above every demo pid.
const NEW_PID: u32 = 900_000;

/// What one frame of a run showed.
pub struct Render {
    pub frame: Frame,
    /// Where each drawn process is: `Scene::positions`.
    pub positions: HashMap<Identity, Point>,
    /// Where the stability properties look for each process: the row's anchors if it has
    /// them, otherwise `positions`.
    pub places: HashMap<Identity, Point>,
    pub visible: usize,
}

/// One headless run of a view over a table of rows.
pub struct Session {
    app: App,
    options: Options,
}

impl Session {
    /// Warms `view` up over `snapshot` as `isotop --output` does at `time` (with `--demo` when
    /// `demo` is set, `--processes PROCESSES` and `--width width`), and draws its first frame.
    pub fn start(
        rows: &'static [ViewInfo],
        view: View,
        snapshot: Snapshot,
        time: f64,
        width: u32,
        demo: bool,
    ) -> (Session, Render) {
        let (time_text, width_text, count) =
            (time.to_string(), width.to_string(), PROCESSES.to_string());
        let mut arguments = vec![
            "isotop",
            "--time",
            &time_text,
            "--width",
            &width_text,
            "--processes",
            &count,
            "--renderer",
            "cpu",
            "--output",
            "harness.png",
        ];
        if demo {
            arguments.push("--demo");
        }
        let options = Options::parse_from(arguments);
        let mut app = App::new(view, snapshot, vec![true; rows.len()]);
        app.scene = Scene::with_rows(rows);
        // The matrix greets `$USER`, which differs between machines.
        app.scene.matrix.user = Some("Neo".into());
        warm_up(&mut app, &options, None);
        let mut session = Session { app, options };
        let first = session.frame(time as f32);
        (session, first)
    }

    /// Takes in a new sample, as the main loop does when one arrives.
    pub fn sample(&mut self, snapshot: Snapshot) {
        self.app.scene.record(&snapshot, self.app.animation);
        self.app.history.push_back(snapshot);
    }

    /// Draws and rasterizes the frame at `time`.
    pub fn frame(&mut self, time: f32) -> Render {
        self.app.animation = time;
        let frame = self.app.render(
            self.options.width,
            self.options.headless_height(),
            self.options.limit as usize,
        );
        let scene = &self.app.scene;
        let positions = scene.positions.clone();
        let places = match self.app.row().contract.anchors {
            Some(anchors) => anchors(scene),
            None => positions.clone(),
        };
        Render {
            frame,
            positions,
            places,
            visible: scene.visible,
        }
    }

    /// Draws the `AFTER_FRAMES` frames that follow `time`, taking in `snapshot` as the sample
    /// of `time` + 1 s when the clock reaches it, as the main loop does; returns the last.
    fn after(&mut self, mut snapshot: Snapshot, time: f64) -> Render {
        snapshot.elapsed = time + 1.0;
        let mut pending = Some(snapshot);
        let mut last = None;
        for step in 1..=AFTER_FRAMES {
            let offset = step as f32 * AFTER_STEP;
            if offset >= 1.0
                && let Some(next) = pending.take()
            {
                self.app.animation = (time + 1.0) as f32;
                self.sample(next);
            }
            last = Some(self.frame(time as f32 + offset));
        }
        last.expect("at least one frame follows")
    }
}

/// The demo sample a run of `view` starts from: what `--demo --time TIME` gives the view.
fn base(rows: &'static [ViewInfo], view: View) -> Snapshot {
    demo_snapshot(&rows[view.index()], TIME, PROCESSES)
}

/// `snapshot` with each process's CPU scaled by a factor between 0.25 and 4 and its memory by
/// one between 0.5 and 2, both log-uniform and fixed by its identity. Cgroup totals follow.
pub fn rescaled(snapshot: &Snapshot) -> Snapshot {
    let mut changed = snapshot.clone();
    for process in &mut changed.processes {
        let mut rng = Rng::for_identity(process.id, SALT);
        process.cpu *= 0.25 * 16.0_f32.powf(rng.unit());
        process.memory = (process.memory as f64 * 0.5 * 4.0_f64.powf(rng.unit().into())) as u64;
    }
    retally(&mut changed);
    changed
}

/// `snapshot` after a tenth of its processes exited and as many started, all chosen by
/// identity, and the exited processes' identities. Only leaves exit: processes that are nobody's
/// parent and hold no lock anybody waits for, since an exiting parent re-parents its children,
/// which is a change of structure rather than of population. Each new process is a sibling of
/// a survivor, in its group and cgroup, with its figures and a fresh identity, holding no lock.
pub fn churned(snapshot: &Snapshot) -> (Snapshot, HashSet<Identity>) {
    let processes = &snapshot.processes;
    let share = processes.len() / 10;
    let parents: HashSet<u32> = processes.iter().map(|process| process.parent).collect();
    let holders: HashSet<u32> = processes
        .iter()
        .filter_map(|process| match process.blocked_on {
            Some(Blocker::Process(pid)) => Some(pid),
            _ => None,
        })
        .collect();
    let mut leaves: Vec<Identity> = processes
        .iter()
        .map(|process| process.id)
        .filter(|id| !parents.contains(&id.pid) && !holders.contains(&id.pid))
        .collect();
    leaves.sort_by_key(|&id| hash_identity(id, SALT));
    let exited: HashSet<Identity> = leaves.into_iter().take(share).collect();
    let mut changed = snapshot.clone();
    changed
        .processes
        .retain(|process| !exited.contains(&process.id));
    let mut templates: Vec<_> = changed.processes.clone();
    templates.sort_by_key(|process| hash_identity(process.id, SALT ^ 1));
    for (index, template) in templates.into_iter().take(share).enumerate() {
        let pid = NEW_PID + index as u32;
        changed.processes.push(Process {
            id: Identity {
                pid,
                start: u64::from(pid),
            },
            name: format!("{}-new", template.name),
            locks_held: Measured::Known(0),
            blocked_on: None,
            ..template
        });
    }
    let gone = |id: &Identity| exited.contains(id);
    changed.links.retain(|(a, b, _)| !gone(a) && !gone(b));
    changed
        .link_traffic
        .retain(|(a, b), _| !gone(a) && !gone(b));
    changed.unix_links.retain(|(a, b)| !gone(a) && !gone(b));
    changed.outside.retain(|id, _| !gone(id));
    changed.remotes.retain(|remote| !gone(&remote.id));
    retally(&mut changed);
    (changed, exited)
}

/// Sets each cgroup's memory, CPU and process count to the sums over its processes, as the
/// demo builds them, keeping its limits and pressure.
fn retally(snapshot: &mut Snapshot) {
    for unit in snapshot.units.values_mut() {
        unit.memory = 0;
        unit.cpu = Some(0.0);
        unit.pids = Some(0);
    }
    for process in &snapshot.processes {
        if let Some(unit) = snapshot.units.get_mut(&process.cgroup) {
            unit.memory += process.memory;
            *unit.cpu.get_or_insert(0.0) += process.cpu;
            *unit.pids.get_or_insert(0) += 1;
        }
    }
}

/// What checking one property of one view found.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Pass,
    /// The view broke the property; the text says how.
    Fail(String),
    /// The property could not be checked: the run panicked or left nothing to compare. No
    /// `known_unstable` entry excuses this, since the entries record layouts that move.
    Broken(String),
    /// The row's contract excuses the view from the property.
    Exempt,
}

/// The runs of one view that the properties look at, each made once, when first needed.
struct Runs {
    rows: &'static [ViewInfo],
    view: View,
    /// A demo run whose next sample is the same workload one second on.
    steady: Option<(Session, Render, Render)>,
    /// A demo run whose next sample is `rescaled`.
    rescaled: Option<(Render, Render)>,
    /// A demo run whose next sample is `churned`.
    churned: Option<Render>,
}

impl Runs {
    fn new(rows: &'static [ViewInfo], view: View) -> Runs {
        Runs {
            rows,
            view,
            steady: None,
            rescaled: None,
            churned: None,
        }
    }

    fn row(&self) -> &'static ViewInfo {
        &self.rows[self.view.index()]
    }

    /// A demo run from `base` whose next sample is `next`: its first frame and its last.
    fn run(&self, next: Snapshot) -> (Session, Render, Render) {
        let (mut session, first) = Session::start(
            self.rows,
            self.view,
            base(self.rows, self.view),
            TIME,
            WIDTH,
            true,
        );
        let last = session.after(next, TIME);
        (session, first, last)
    }

    fn steady(&mut self) -> &mut (Session, Render, Render) {
        if self.steady.is_none() {
            self.steady = Some(self.run(base(self.rows, self.view)));
        }
        self.steady.as_mut().expect("just made")
    }

    fn rescaled(&mut self) -> &(Render, Render) {
        if self.rescaled.is_none() {
            let (_, first, last) = self.run(rescaled(&base(self.rows, self.view)));
            self.rescaled = Some((first, last));
        }
        self.rescaled.as_ref().expect("just made")
    }

    fn churned(&mut self) -> &Render {
        if self.churned.is_none() {
            let (changed, _) = churned(&base(self.rows, self.view));
            self.churned = Some(self.run(changed).2);
        }
        self.churned.as_ref().expect("just made")
    }

    fn check(&mut self, property: Property) -> Outcome {
        match property {
            Property::Deterministic => self.deterministic(),
            Property::NonEmpty => self.non_empty(),
            Property::ValidPicks => valid_picks(&self.steady().1.frame),
            Property::ResourceChange => self.resource_change(),
            Property::Churn => self.churn(),
            Property::EmptyMachine => self.empty_machine(),
        }
    }

    /// Two runs from the same inputs, the steady run and the rescaled one before its change,
    /// draw the same first frame.
    fn deterministic(&mut self) -> Outcome {
        let other = self.rescaled().0.frame.pixels.clone();
        let other_picks = self.rescaled().0.frame.picks.clone();
        let first = &self.steady().1.frame;
        let differing = |a: &[u8], b: &[u8]| a.iter().zip(b).filter(|(a, b)| a != b).count();
        if first.pixels != other {
            return Outcome::Fail(format!(
                "{} of {} colour bytes differ between two runs from the same inputs",
                differing(&first.pixels, &other),
                first.pixels.len()
            ));
        }
        if first.picks != other_picks {
            let count = first
                .picks
                .iter()
                .zip(&other_picks)
                .filter(|(a, b)| a != b)
                .count();
            return Outcome::Fail(format!(
                "{count} picks differ between two runs from the same inputs"
            ));
        }
        Outcome::Pass
    }

    /// The demo frame shows processes, has positions and leaves a pixel to click.
    fn non_empty(&mut self) -> Outcome {
        if !self.row().contract.draws_processes {
            return Outcome::Exempt;
        }
        let first = &self.steady().1;
        let picks = first
            .frame
            .picks
            .iter()
            .filter(|&&pick| pick != NONE)
            .count();
        match (first.visible, first.positions.len(), picks) {
            (0, ..) => Outcome::Fail("no process is visible".into()),
            (_, 0, _) => Outcome::Fail("no process has a position".into()),
            (_, _, 0) => Outcome::Fail("no pixel can be clicked".into()),
            _ => Outcome::Pass,
        }
    }

    /// Survivors stand where they stand in the steady run after every CPU and memory changed.
    fn resource_change(&mut self) -> Outcome {
        if !self.row().contract.draws_processes {
            return Outcome::Exempt;
        }
        let tolerance = self.row().contract.tolerance;
        let after = self.rescaled().1.places.clone();
        stayed(&self.steady().2.places, &after, tolerance)
    }

    /// Survivors stand where they stand in the steady run after a tenth of the processes
    /// exited and as many started.
    fn churn(&mut self) -> Outcome {
        if !self.row().contract.draws_processes {
            return Outcome::Exempt;
        }
        let tolerance = self.row().contract.tolerance;
        let after = self.churned().places.clone();
        stayed(&self.steady().2.places, &after, tolerance)
    }

    /// An empty machine draws, from the first frame and after every process of the steady run
    /// exited.
    fn empty_machine(&mut self) -> Outcome {
        let (mut empty, _) = Session::start(
            self.rows,
            self.view,
            Snapshot::default(),
            TIME,
            WIDTH,
            false,
        );
        empty.frame(TIME as f32 + AFTER_STEP);
        let time = TIME + AFTER_FRAMES as f64 * AFTER_STEP as f64;
        let session = &mut self.steady().0;
        let gone = session.after(Snapshot::default(), time);
        if gone.visible > 0 {
            return Outcome::Fail(format!(
                "{} processes are visible on an empty machine",
                gone.visible
            ));
        }
        Outcome::Pass
    }
}

/// Every pick is `NONE` or an index into the frame's identities.
fn valid_picks(frame: &Frame) -> Outcome {
    let identities = frame.identities.len();
    let invalid: Vec<u32> = frame
        .picks
        .iter()
        .copied()
        .filter(|&pick| pick != NONE && pick as usize >= identities)
        .collect();
    match invalid.first() {
        None => Outcome::Pass,
        Some(pick) => Outcome::Fail(format!(
            "{} pixels pick past the frame's {identities} identities, such as {pick}",
            invalid.len()
        )),
    }
}

/// Whether every process in both `before` and `after` moved at most `tolerance` world units
/// over the ground; the failure lists how many moved and the farthest. Height is left out
/// because several views draw a measurement with it, such as a ridge's crest at its CPU.
fn stayed(
    before: &HashMap<Identity, Point>,
    after: &HashMap<Identity, Point>,
    tolerance: f32,
) -> Outcome {
    let mut moves: Vec<(f32, Identity)> = before
        .iter()
        .filter_map(|(id, from)| {
            let to = after.get(id)?;
            let distance = (to[0] - from[0]).hypot(to[1] - from[1]);
            Some((distance, *id))
        })
        .collect();
    if moves.is_empty() {
        return Outcome::Broken("no surviving process is placed in both runs".into());
    }
    let compared = moves.len();
    moves.retain(|&(distance, _)| distance > tolerance);
    if moves.is_empty() {
        return Outcome::Pass;
    }
    moves.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let farthest: Vec<String> = moves
        .iter()
        .take(5)
        .map(|(distance, id)| format!("pid {} by {distance:.2}", id.pid))
        .collect();
    Outcome::Fail(format!(
        "{} of {compared} surviving processes moved more than {tolerance}: {}",
        moves.len(),
        farthest.join(", ")
    ))
}

/// What the contract test makes of one view and property.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    Pass,
    Exempt,
    /// It fails, and the row knows: the text is the issue, the row's reason and the failure.
    KnownUnstable(String),
    /// It passes although the row lists it as known to fail: the text names the issue to close.
    NowPasses(&'static str),
    Fail(String),
}

/// Checks every property of `view`, each on its own, so a panic fails only the property that
/// hit it.
pub fn check(rows: &'static [ViewInfo], view: View) -> Vec<(Property, Verdict)> {
    let mut runs = Runs::new(rows, view);
    let contract = &rows[view.index()].contract;
    Property::ALL
        .into_iter()
        .map(|property| {
            let outcome =
                catch_unwind(AssertUnwindSafe(|| runs.check(property))).unwrap_or_else(|panic| {
                    let message = panic
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| panic.downcast_ref::<&str>().copied())
                        .unwrap_or("a panic");
                    Outcome::Broken(format!("panicked: {message}"))
                });
            (property, verdict(outcome, contract.unstable(property)))
        })
        .collect()
}

/// The verdict on a property that checked out as `outcome`, given the row's `known_unstable`
/// entry for it, if any.
fn verdict(outcome: Outcome, known: Option<&Unstable>) -> Verdict {
    match (outcome, known) {
        (Outcome::Exempt, _) => Verdict::Exempt,
        (Outcome::Pass, None) => Verdict::Pass,
        (Outcome::Pass, Some(entry)) => Verdict::NowPasses(entry.issue),
        (Outcome::Broken(failure), _) | (Outcome::Fail(failure), None) => Verdict::Fail(failure),
        (Outcome::Fail(failure), Some(entry)) => {
            Verdict::KnownUnstable(format!("{}, {}: {failure}", entry.issue, entry.reason))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::{REGISTRY, testing};

    #[test]
    fn every_view_satisfies_the_view_contract() {
        let results: Vec<(View, Vec<(Property, Verdict)>)> = std::thread::scope(|scope| {
            let checks: Vec<_> = View::all()
                .map(|view| scope.spawn(move || (view, check(REGISTRY, view))))
                .collect();
            checks
                .into_iter()
                .map(|check| check.join().expect("the harness catches panics"))
                .collect()
        });
        let mut failures = Vec::new();
        for (view, verdicts) in &results {
            for (property, verdict) in verdicts {
                let name = format!("{view:?}: {}", property.name());
                match verdict {
                    Verdict::Pass => println!("{name}: pass"),
                    Verdict::Exempt => println!("{name}: exempt"),
                    Verdict::KnownUnstable(text) => println!("{name}: known unstable ({text})"),
                    Verdict::NowPasses(issue) => println!(
                        "{name}: passes now; drop its known_unstable entry and close {issue}"
                    ),
                    Verdict::Fail(text) => failures.push(format!("{name} ({text})")),
                }
            }
        }
        assert!(
            failures.is_empty(),
            "views break the view contract:\n  {}",
            failures.join("\n  ")
        );
    }

    #[test]
    fn the_harness_detects_a_view_that_reshuffles() {
        let verdicts = check(testing::SHUFFLED, View::at(0));
        let verdict = |wanted: Property| {
            verdicts
                .iter()
                .find(|(property, _)| *property == wanted)
                .map(|(_, verdict)| verdict)
                .expect("every property is checked")
        };
        let Verdict::Fail(failure) = verdict(Property::Churn) else {
            panic!("seats by rank survive churn: {verdicts:?}");
        };
        assert!(
            failure.contains("surviving processes moved more than 1"),
            "{failure}"
        );
        for property in [
            Property::Deterministic,
            Property::NonEmpty,
            Property::ValidPicks,
            Property::ResourceChange,
            Property::EmptyMachine,
        ] {
            assert_eq!(*verdict(property), Verdict::Pass, "{property:?}");
        }
    }

    #[test]
    fn a_known_failure_is_excused_and_a_listed_property_that_passes_is_reported() {
        let verdicts = check(testing::SHUFFLED, View::at(1));
        let verdict = |wanted: Property| {
            verdicts
                .iter()
                .find(|(property, _)| *property == wanted)
                .map(|(_, verdict)| verdict)
                .expect("every property is checked")
        };
        let Verdict::KnownUnstable(text) = verdict(Property::Churn) else {
            panic!("the listed churn failure is not excused: {verdicts:?}");
        };
        assert!(
            text.starts_with("ISSUE_LISTED_CHURN, seats by rank: "),
            "{text}"
        );
        assert_eq!(
            *verdict(Property::ResourceChange),
            Verdict::NowPasses("ISSUE_LISTED_RESOURCE_CHANGE")
        );
        assert_eq!(*verdict(Property::NonEmpty), Verdict::Pass);
    }

    #[test]
    fn a_panic_or_nothing_to_compare_is_never_excused_as_known_instability() {
        let entry = Unstable {
            property: Property::Churn,
            issue: "ISSUE",
            reason: "seats by rank",
        };
        for failure in ["panicked: index out of bounds", "no surviving process"] {
            assert_eq!(
                verdict(Outcome::Broken(failure.into()), Some(&entry)),
                Verdict::Fail(failure.into())
            );
        }
        assert_eq!(
            verdict(Outcome::Fail("moved".into()), Some(&entry)),
            Verdict::KnownUnstable("ISSUE, seats by rank: moved".into())
        );
    }

    #[test]
    fn churn_replaces_a_tenth_of_the_leaves_and_rescaling_keeps_every_process() {
        let snapshot = crate::model::demo(TIME, PROCESSES);
        let (changed, exited) = churned(&snapshot);
        assert_eq!(exited.len(), PROCESSES / 10);
        assert_eq!(changed.processes.len(), PROCESSES);
        let parents: HashSet<u32> = snapshot.processes.iter().map(|p| p.parent).collect();
        assert!(exited.iter().all(|id| !parents.contains(&id.pid)));
        assert!(changed.processes.iter().all(|p| !exited.contains(&p.id)));
        assert!(
            changed
                .links
                .iter()
                .all(|(a, b, _)| !exited.contains(a) && !exited.contains(b))
        );
        assert_eq!(
            format!("{:?}", churned(&snapshot).0.processes),
            format!("{:?}", changed.processes),
            "the same processes are chosen every time"
        );
        let scaled = rescaled(&snapshot);
        for (before, after) in snapshot.processes.iter().zip(&scaled.processes) {
            assert_eq!(before.id, after.id);
            let cpu = after.cpu / before.cpu;
            assert!(before.cpu == 0.0 || (0.25..=4.0).contains(&cpu), "{cpu}");
            let memory = after.memory as f64 / before.memory as f64;
            assert!((0.49..=2.01).contains(&memory), "{memory}");
        }
    }
}
