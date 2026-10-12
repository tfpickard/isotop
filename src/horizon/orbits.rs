//! The population of the horizon view, with no drawing: which processes ride the disk and at
//! what radius, how they move, and what becomes of one that vanishes. Radius is rank, the next
//! OOM victim on the ISCO, and a rise of the kernel's OOM kill counter turns the best-ranked
//! process that has just vanished into a plunge.
//!
//! Time runs at `M_PER_SECOND` M of coordinate time per second. A body goes round at the
//! angular velocity of a circular orbit at its radius, and its radius eases towards its rank's
//! with a time constant of 1.5 s, at most 2 M per second, so a change of rank never jumps and the
//! azimuth stays continuous.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use super::lens::{self, ISCO, PlungeTrack};
use crate::layout::GOLDEN_ANGLE;
use crate::model::{BACKGROUND_PERIOD, Identity};
use crate::render::Color;
use crate::simulation::{Rng, hash_identity, smoothing, smoothstep};

/// Coordinate time in M that passes per second of the animation clock.
pub const M_PER_SECOND: f32 = 10.0;
/// The most processes that ride the disk by rank: the top of the ranking.
pub const RANKED: usize = 28;
/// Where `oom_score_adj` -1000 processes ride, from the inner edge of the band to the outer.
pub const BAND: (f32, f32) = (25.7, 27.3);
/// The most of them drawn.
pub const BAND_LIMIT: usize = 12;
/// The farthest a process outside the top ranks rides, when it is selected.
const SELECTED_FARTHEST: f32 = 25.0;
/// How close in time a rise of the kill counter must be to a body's disappearance to count as its
/// kill, in seconds, before the delay in seeing it is added (`Orbits::claim_window`).
pub const GHOST_WINDOW: f32 = 2.5;
/// How long an ordinary exit takes to fade where the process stood, in seconds.
pub const FADE_SECONDS: f32 = 1.2;
/// How long a killed body takes to ease to the ISCO before it plunges, in seconds.
const ISCO_SECONDS: f32 = 0.8;
/// The brightness below which a plunging image is removed.
const GONE: f32 = 0.02;
/// Radius easing: time constant in seconds, and fastest speed in M per second.
const EASE_SECONDS: f32 = 1.5;
const EASE_SPEED: f32 = 2.0;
/// Two neighbours in the ranking swap only if their scores differ by at least this much.
const SWAP_MARGIN: i32 = 5;
/// How long the view may go without a new sample before what it remembers is stale, in seconds:
/// longer than the slowest `--sample-ms` (10 s) with room to spare. The animation clock only runs
/// while the view is shown, so a longer gap means it was away.
const MAX_GAP: f32 = 30.0;
/// The longest a sample interval counts for the claim window, in seconds: the slowest `--sample-ms`.
const MAX_INTERVAL: f32 = 10.0;
/// The most rises of the kill counter one sample may add, so a restart of the counter cannot
/// flood the ghosts.
const MOST_RISES: u64 = 8;

/// Separates the seeds of the initial phase and the band radius from every other use of an identity.
const PHASE_SALT: u64 = 0x0068_6f72_697a;
const BAND_SALT: u64 = 0x0068_6f72_6261;

/// The radius of the orbit of rank `rank` (0 is the next victim): `6 + 17.5 (rank / 27)^0.8`,
/// from the ISCO outwards, growing fastest at the first ranks.
pub fn rank_radius(rank: usize) -> f32 {
    ISCO + 17.5 * (rank as f32 / (RANKED - 1) as f32).powf(0.8)
}

/// The radius at which an unkillable process rides, from its identity.
pub fn band_radius(id: Identity) -> f32 {
    let fraction = (hash_identity(id, BAND_SALT) >> 40) as f32 / 16_777_216.0;
    BAND.0 + (BAND.1 - BAND.0) * fraction
}

/// Where a process starts going round, from its identity.
pub fn initial_phase(id: Identity) -> f32 {
    Rng::for_identity(id, PHASE_SALT).unit() * std::f32::consts::TAU
}

/// A process the view could show, as `Orbits::observe` takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate<'a> {
    pub id: Identity,
    /// The process's name, which the body keeps for its label.
    pub name: &'a str,
    /// What ranks it: the OOM score, or its memory in MiB where there is none. None: it cannot
    /// be ranked, and is neither shown nor counted.
    pub score: Option<i32>,
    /// `oom_score_adj` is -1000: the kernel never picks it.
    pub unkillable: bool,
    pub color: Color,
    /// The radius of its sphere, in M.
    pub size: f32,
}

/// The kernel's count of OOM kills, and where it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kills {
    pub count: u64,
    pub source: KillSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KillSource {
    /// `oom_kill` in `/proc/vmstat`.
    Vmstat,
    /// The sum of the cgroups' `memory.events` `oom_kill`.
    Cgroups,
}

impl KillSource {
    pub fn name(self) -> &'static str {
        match self {
            KillSource::Vmstat => "vmstat",
            KillSource::Cgroups => "cgroups",
        }
    }
}

/// One sample, as the orbits take it in.
pub struct Sample<'a> {
    /// The sample's own time, `Snapshot::elapsed`. Only a newer sample counts for the exits and
    /// the kill counter, so a paused or rewound frame changes nothing.
    pub elapsed: f64,
    /// The animation clock now.
    pub time: f32,
    /// The processes that may be shown, in the slice order.
    pub candidates: &'a [Candidate<'a>],
    /// Every process the sample lists, shown or not: a body whose process is not among them has
    /// exited.
    pub alive: &'a HashSet<Identity>,
    pub selected: Option<Identity>,
    pub kills: Option<Kills>,
}

/// The processes in OOM order, the next victim first, with a margin so two near neighbours do
/// not trade places on every sample.
#[derive(Default)]
pub struct Ranking {
    order: Vec<Identity>,
    ranks: HashMap<Identity, usize>,
}

impl Ranking {
    /// Takes in the scores of the processes that can be killed. Processes already ranked keep
    /// their order unless a lower one scores `SWAP_MARGIN` more than the one above it; a process
    /// that has just appeared goes where its score puts it, ties by identity.
    pub fn update(&mut self, scored: &[(Identity, i32)]) {
        let scores: HashMap<Identity, i32> = scored.iter().copied().collect();
        let mut order: Vec<Identity> = self
            .order
            .iter()
            .copied()
            .filter(|id| scores.contains_key(id))
            .collect();
        let known: HashSet<Identity> = order.iter().copied().collect();
        let mut entrants: Vec<(Identity, i32)> = scored
            .iter()
            .copied()
            .filter(|(id, _)| !known.contains(id))
            .collect();
        entrants.sort_by_key(|&(id, score)| (Reverse(score), id));
        for (id, score) in entrants {
            let position = order.partition_point(|other| {
                let other_score = scores[other];
                other_score > score || (other_score == score && *other < id)
            });
            order.insert(position, id);
        }
        loop {
            let mut swapped = false;
            for index in 1..order.len() {
                if scores[&order[index]] >= scores[&order[index - 1]] + SWAP_MARGIN {
                    order.swap(index, index - 1);
                    swapped = true;
                }
            }
            if !swapped {
                break;
            }
        }
        self.ranks = order
            .iter()
            .enumerate()
            .map(|(rank, &id)| (id, rank))
            .collect();
        self.order = order;
    }

    /// The rank of `id`, 0 for the next victim.
    pub fn rank(&self, id: Identity) -> Option<usize> {
        self.ranks.get(&id).copied()
    }

    /// How many processes are ranked.
    pub fn len(&self) -> usize {
        self.order.len()
    }
}

/// A process riding the disk.
#[derive(Clone, Debug)]
pub struct Body {
    /// The azimuth in the disk plane, in radians, counted from the right of the screen towards
    /// the far side; it grows as the body goes round.
    pub phase: f32,
    /// The radius now, easing towards `target`.
    pub radius: f32,
    pub target: f32,
    /// The rank (0 for the next victim), None for an unkillable process.
    pub rank: Option<usize>,
    pub color: Color,
    pub size: f32,
    pub name: String,
}

/// What becomes of a vanished body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    /// An exit, so far: it fades where it stood unless the kill counter rises in time.
    Fading,
    /// An OOM kill: it eases to the ISCO and plunges.
    Plunging,
}

/// A body whose process has vanished, kept so the picture can say how it went.
#[derive(Clone, Debug)]
pub struct Ghost {
    pub id: Identity,
    pub rank: Option<usize>,
    pub color: Color,
    pub size: f32,
    pub name: String,
    /// Where it was when it was last seen.
    pub radius: f32,
    pub phase: f32,
    /// When it vanished: the middle of the interval between the last sample that had it and the
    /// first that did not, on the animation clock.
    pub since: f32,
    /// When a killed body starts for the ISCO, on the animation clock: `since` when the kill is
    /// known as soon as the body vanishes, later when the counter rose only in a later sample.
    pub plunge_at: f32,
    pub fate: Fate,
}

/// Where a ghost is and how it looks `age` seconds after it vanished.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub radius: f32,
    pub azimuth: f32,
    /// How much of its light is left, 1 to 0, as it is drawn.
    pub light: f32,
    /// Radial stretch of a plunging body, 1 where it is not stretched.
    pub stretch: f32,
    pub plunging: bool,
}

impl Ghost {
    pub fn pose(&self, time: f32) -> Pose {
        let age = (time - self.since).max(0.0);
        let orbit = |radius: f32, seconds: f32| {
            self.phase + lens::angular_velocity(radius) * M_PER_SECOND * seconds
        };
        match self.fate {
            Fate::Fading => Pose {
                radius: self.radius,
                azimuth: orbit(self.radius, age),
                light: (1.0 - age / FADE_SECONDS).clamp(0.0, 1.0),
                stretch: 1.0,
                plunging: false,
            },
            Fate::Plunging if time - self.plunge_at < ISCO_SECONDS => {
                let age = (time - self.plunge_at).max(0.0);
                let eased = smoothstep(age / ISCO_SECONDS);
                let radius = self.radius + (ISCO - self.radius) * eased;
                Pose {
                    radius,
                    azimuth: orbit(0.5 * (self.radius + radius), age),
                    light: 1.0,
                    stretch: 1.0,
                    plunging: true,
                }
            }
            Fate::Plunging => {
                let leaving = orbit(0.5 * (self.radius + ISCO), ISCO_SECONDS);
                let (radius, turned) = PlungeTrack::shared()
                    .seen_at((time - self.plunge_at - ISCO_SECONDS) * M_PER_SECOND);
                let light = lens::plunge_brightness(radius).min(1.0);
                let inward = ((ISCO - radius) / 4.0).clamp(0.0, 1.0);
                Pose {
                    radius,
                    azimuth: leaving + turned,
                    light,
                    stretch: 1.0 + 2.6 * inward.powf(1.5),
                    plunging: true,
                }
            }
        }
    }

    /// Whether nothing of it is left to draw at `time`.
    pub fn gone(&self, time: f32) -> bool {
        match self.fate {
            Fate::Fading => time - self.since >= FADE_SECONDS,
            Fate::Plunging => {
                let age = time - self.plunge_at;
                age >= ISCO_SECONDS
                    && lens::plunge_brightness(
                        PlungeTrack::shared()
                            .seen_at((age - ISCO_SECONDS) * M_PER_SECOND)
                            .0,
                    ) < GONE
            }
        }
    }
}

/// Every body, ghost and the counter's history.
#[derive(Default)]
pub struct Orbits {
    pub ranking: Ranking,
    bodies: HashMap<Identity, Body>,
    ghosts: Vec<Ghost>,
    counter: Option<Kills>,
    /// When the kill counter rose and no ghost has claimed the rise yet, on the animation clock.
    rises: Vec<f32>,
    /// The newest sample taken in: its `elapsed` and the animation time it arrived at.
    seen: Option<(f64, f32)>,
    /// How long the newest sample came after the one before it, in seconds.
    interval: f32,
}

impl Orbits {
    /// Takes in a sample: ghosts for the bodies that vanished and the kill counter's rises (once
    /// per newer sample), then the ranks and targets of the processes now shown.
    pub fn observe(&mut self, sample: &Sample) {
        if self
            .seen
            .is_none_or(|(elapsed, _)| sample.elapsed > elapsed)
        {
            // After a stretch in another view every body that exited meanwhile would be a fresh
            // ghost and the kill counter's whole change a fresh rise, so old exits would plunge as
            // if they were current kills. Start afresh instead.
            if self
                .seen
                .is_some_and(|(_, time)| sample.time - time > MAX_GAP)
            {
                self.bodies.clear();
                self.ghosts.clear();
                self.rises.clear();
                self.counter = None;
                self.seen = None;
            }
            let before = self
                .seen
                .map_or(sample.time, |(_, time)| time.min(sample.time));
            let when = 0.5 * (before + sample.time);
            let mut gone: Vec<Identity> = self
                .bodies
                .keys()
                .filter(|id| !sample.alive.contains(id))
                .copied()
                .collect();
            gone.sort();
            for id in gone {
                if let Some(body) = self.bodies.remove(&id) {
                    self.ghosts.push(Ghost {
                        id,
                        rank: body.rank,
                        color: body.color,
                        size: body.size,
                        name: body.name,
                        radius: body.radius,
                        phase: body.phase,
                        since: when,
                        plunge_at: when,
                        fate: Fate::Fading,
                    });
                }
            }
            if let (Some(before), Some(now)) = (self.counter, sample.kills)
                && before.source == now.source
                && now.count > before.count
            {
                for _ in 0..(now.count - before.count).min(MOST_RISES) {
                    self.rises.push(when);
                }
            }
            self.counter = sample.kills;
            self.interval = (sample.time - before).clamp(0.0, MAX_INTERVAL);
            self.seen = Some((sample.elapsed, sample.time));
        }
        self.retarget(sample);
        self.classify(sample.time);
    }

    /// Ranks the processes and gives every one that is shown a target: the top ranks by rank, the
    /// unkillables in their band, and the selected process wherever its rank puts it.
    fn retarget(&mut self, sample: &Sample) {
        let scored: Vec<(Identity, i32)> = sample
            .candidates
            .iter()
            .filter(|candidate| !candidate.unkillable)
            .filter_map(|candidate| Some((candidate.id, candidate.score?)))
            .collect();
        self.ranking.update(&scored);
        let mut band: Vec<&Candidate> = sample
            .candidates
            .iter()
            .filter(|candidate| candidate.unkillable)
            .collect();
        band.sort_by_key(|candidate| candidate.id);
        let mut wanted: HashMap<Identity, (Option<usize>, f32, usize)> = HashMap::new();
        for (seat, candidate) in band.iter().enumerate().take(BAND_LIMIT) {
            wanted.insert(candidate.id, (None, band_radius(candidate.id), seat));
        }
        for candidate in sample.candidates {
            let Some(rank) = self.ranking.rank(candidate.id) else {
                continue;
            };
            if rank < RANKED {
                wanted.insert(candidate.id, (Some(rank), rank_radius(rank), rank));
            } else if sample.selected == Some(candidate.id) {
                let radius = rank_radius(rank).min(SELECTED_FARTHEST);
                wanted.insert(candidate.id, (Some(rank), radius, rank));
            }
        }
        if let Some(&selected) = sample.selected.as_ref()
            && !wanted.contains_key(&selected)
            && let Some(position) = band.iter().position(|candidate| candidate.id == selected)
        {
            wanted.insert(selected, (None, band_radius(selected), position));
        }
        self.bodies.retain(|id, _| wanted.contains_key(id));
        for candidate in sample.candidates {
            let Some(&(rank, target, seat)) = wanted.get(&candidate.id) else {
                continue;
            };
            let body = self.bodies.entry(candidate.id).or_insert_with(|| Body {
                phase: if rank.is_some() {
                    initial_phase(candidate.id)
                } else {
                    seat as f32 * GOLDEN_ANGLE
                },
                radius: target,
                target,
                rank,
                color: candidate.color,
                size: candidate.size,
                name: candidate.name.to_owned(),
            });
            body.rank = rank;
            body.target = target;
            body.color = candidate.color;
            body.size = candidate.size;
            if body.name != candidate.name {
                candidate.name.clone_into(&mut body.name);
            }
        }
    }

    /// How far apart a body's disappearance and a rise of the kill counter may be seen: the
    /// `GHOST_WINDOW` between the events themselves, plus the delay in seeing the counter. The
    /// counter comes from the background thread, which reads it once per `BACKGROUND_PERIOD` and
    /// hands it over with the next sample, while the frame thread sees the process go at once.
    fn claim_window(&self) -> f32 {
        GHOST_WINDOW + BACKGROUND_PERIOD.as_secs_f32() + self.interval
    }

    /// Lets each unclaimed rise of the kill counter take the best-ranked ghost that vanished
    /// within `claim_window` of it, and drops what is too old to be claimed. A body claimed
    /// later than the sample it vanished in starts its plunge as if the kill had been known half
    /// a sample interval ago, as it is when the counter rises in that same sample.
    fn classify(&mut self, time: f32) {
        let window = self.claim_window();
        self.rises.retain(|&rise| time - rise <= window);
        let mut waiting: Vec<usize> = (0..self.ghosts.len())
            .filter(|&index| self.ghosts[index].fate == Fate::Fading)
            .collect();
        waiting.sort_by_key(|&index| {
            let ghost = &self.ghosts[index];
            (ghost.rank.unwrap_or(usize::MAX), ghost.id)
        });
        for index in waiting {
            let since = self.ghosts[index].since;
            if let Some(rise) = self
                .rises
                .iter()
                .position(|&rise| (rise - since).abs() <= window)
            {
                self.rises.remove(rise);
                self.ghosts[index].fate = Fate::Plunging;
                self.ghosts[index].plunge_at = since.max(time - 0.5 * self.interval);
            }
        }
    }

    /// Moves every body on by `dt` seconds: the radius eases and the azimuth goes round at the
    /// angular velocity of the radius it passed through. Ghosts that have run their course go.
    pub fn advance(&mut self, dt: f32, time: f32) {
        for body in self.bodies.values_mut() {
            let wanted = (body.target - body.radius) * smoothing(dt, EASE_SECONDS);
            let reach = EASE_SPEED * dt;
            let before = body.radius;
            body.radius += wanted.clamp(-reach, reach);
            let mean = 0.5 * (before + body.radius);
            body.phase = (body.phase + lens::angular_velocity(mean) * M_PER_SECOND * dt)
                .rem_euclid(std::f32::consts::TAU);
        }
        let window = self.claim_window();
        // A body that has faded stays until a late rise of the counter can no longer claim it.
        self.ghosts.retain(|ghost| match ghost.fate {
            Fate::Fading => time - ghost.since < window,
            Fate::Plunging => !ghost.gone(time),
        });
    }

    pub fn bodies(&self) -> &HashMap<Identity, Body> {
        &self.bodies
    }

    pub fn ghosts(&self) -> &[Ghost] {
        &self.ghosts
    }

    /// The identity of the body with rank 0, the next victim, if one is shown.
    pub fn next_victim(&self) -> Option<Identity> {
        self.bodies
            .iter()
            .find(|(_, body)| body.rank == Some(0))
            .map(|(&id, _)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::lens::PLUNGE_FLOOR;
    use super::*;

    fn id(pid: u32) -> Identity {
        Identity {
            pid,
            start: u64::from(pid),
        }
    }

    fn candidate(pid: u32, score: i32) -> Candidate<'static> {
        Candidate {
            id: id(pid),
            name: "worker",
            score: Some(score),
            unkillable: false,
            color: [200, 100, 50],
            size: 0.4,
        }
    }

    fn unkillable(pid: u32) -> Candidate<'static> {
        Candidate {
            unkillable: true,
            ..candidate(pid, 0)
        }
    }

    /// Takes in one sample of `candidates` at `elapsed`, with every candidate alive unless `gone`.
    fn take(
        orbits: &mut Orbits,
        elapsed: f64,
        candidates: &[Candidate<'_>],
        kills: Option<u64>,
        selected: Option<Identity>,
    ) {
        let alive: HashSet<Identity> = candidates.iter().map(|c| c.id).collect();
        orbits.observe(&Sample {
            elapsed,
            time: elapsed as f32,
            candidates,
            alive: &alive,
            selected,
            kills: kills.map(|count| Kills {
                count,
                source: KillSource::Vmstat,
            }),
        });
    }

    fn population(count: u32) -> Vec<Candidate<'static>> {
        (1..=count)
            .map(|pid| candidate(pid, 1000 - pid as i32 * 7))
            .collect()
    }

    #[test]
    fn the_next_oom_victim_rides_the_isco_and_unkillables_ride_far_out() {
        let mut orbits = Orbits::default();
        let mut candidates = population(40);
        candidates.extend([unkillable(100), unkillable(101), unkillable(102)]);
        take(&mut orbits, 1.0, &candidates, None, None);
        let bodies = orbits.bodies();
        // Rank 0 is the highest score, on the ISCO exactly; rank 27 is the last that rides.
        assert_eq!(bodies[&id(1)].rank, Some(0));
        assert_eq!(bodies[&id(1)].radius, 6.0);
        assert_eq!(bodies[&id(28)].rank, Some(27));
        assert!((bodies[&id(28)].radius - 23.5).abs() < 1e-4);
        assert!(!bodies.contains_key(&id(29)), "the rest are collapsed");
        assert_eq!(orbits.next_victim(), Some(id(1)));
        // Radius grows with rank.
        let mut radii: Vec<(usize, f32)> = bodies
            .values()
            .filter_map(|body| Some((body.rank?, body.radius)))
            .collect();
        radii.sort_by_key(|&(rank, _)| rank);
        assert!(radii.windows(2).all(|pair| pair[0].1 < pair[1].1));
        // The unkillables ride the band and are never ranked.
        for pid in [100, 101, 102] {
            let body = &bodies[&id(pid)];
            assert_eq!(body.rank, None);
            assert!((BAND.0..=BAND.1).contains(&body.radius), "{}", body.radius);
        }
        assert_eq!(orbits.ranking.len(), 40);
        // Golden-angle phases: the seats of the band are one golden angle apart.
        let phase = |pid| bodies[&id(pid)].phase;
        assert!((phase(101) - phase(100) - GOLDEN_ANGLE).abs() < 1e-4);
    }

    #[test]
    fn at_most_twelve_unkillables_ride_and_a_selected_process_rides_at_most_to_twenty_five() {
        let mut orbits = Orbits::default();
        let mut candidates = population(40);
        candidates.extend((100..120).map(unkillable));
        take(&mut orbits, 1.0, &candidates, None, Some(id(35)));
        let band = orbits
            .bodies()
            .values()
            .filter(|body| body.rank.is_none())
            .count();
        assert_eq!(band, BAND_LIMIT);
        let selected = &orbits.bodies()[&id(35)];
        assert_eq!(selected.rank, Some(34));
        assert_eq!(
            selected.radius,
            25.0,
            "the formula gives {}",
            rank_radius(34)
        );
        take(&mut orbits, 2.0, &candidates, None, None);
        assert!(
            !orbits.bodies().contains_key(&id(35)),
            "and goes when deselected"
        );
    }

    #[test]
    fn two_neighbours_swap_only_when_their_scores_differ_by_five() {
        let mut ranking = Ranking::default();
        ranking.update(&[(id(1), 100), (id(2), 98), (id(3), 50)]);
        assert_eq!(
            (ranking.rank(id(1)), ranking.rank(id(2))),
            (Some(0), Some(1))
        );
        // Two behind: no swap.
        ranking.update(&[(id(1), 100), (id(2), 103), (id(3), 50)]);
        assert_eq!(ranking.rank(id(1)), Some(0), "3 points do not swap");
        // Five ahead: swap.
        ranking.update(&[(id(1), 100), (id(2), 105), (id(3), 50)]);
        assert_eq!(ranking.rank(id(2)), Some(0), "5 points do");
        // A newcomer goes where its score puts it, whatever the margin; ties go by identity.
        ranking.update(&[
            (id(1), 100),
            (id(2), 105),
            (id(3), 50),
            (id(4), 101),
            (id(5), 101),
        ]);
        let order: Vec<usize> = (1..=5).map(|pid| ranking.rank(id(pid)).unwrap()).collect();
        assert_eq!(order, [3, 0, 4, 1, 2]);
    }

    #[test]
    fn orbits_keep_their_places_when_cpu_or_population_changes_and_ease_when_rank_changes() {
        let mut orbits = Orbits::default();
        let mut candidates = population(40);
        take(&mut orbits, 1.0, &candidates, None, None);
        let before: HashMap<Identity, (f32, f32)> = orbits
            .bodies()
            .iter()
            .map(|(&id, body)| (id, (body.target, body.phase)))
            .collect();
        // CPU and memory are not the ranking's inputs: a new sample of the same scores moves nothing.
        take(&mut orbits, 2.0, &candidates, None, None);
        for (id, body) in orbits.bodies() {
            assert_eq!((body.target, body.phase), before[id]);
        }
        // Processes appearing below the top change no one's rank and so no one's place.
        candidates.extend((50..60).map(|pid| candidate(pid, 3)));
        take(&mut orbits, 3.0, &candidates, None, None);
        for (id, body) in orbits.bodies() {
            assert_eq!(body.target, before[id].0, "{id:?}");
        }
        // A process that scores more than the next victim takes rank 0; the old victim's target
        // moves out by a rank at once, and its radius follows over time, never jumping.
        candidates.push(candidate(70, 2000));
        take(&mut orbits, 4.0, &candidates, None, None);
        let victim = &orbits.bodies()[&id(1)];
        assert_eq!((victim.rank, victim.radius), (Some(1), 6.0));
        assert!(victim.target > 6.5);
        let mut radius = victim.radius;
        for step in 0..60 {
            orbits.advance(0.05, 4.0 + step as f32 * 0.05);
            let now = orbits.bodies()[&id(1)].radius;
            assert!(
                now >= radius && now - radius <= EASE_SPEED * 0.05 + 1e-6,
                "step {step}"
            );
            radius = now;
        }
        assert!(radius > 6.4 && radius < orbits.bodies()[&id(1)].target + 1e-3);
        // The azimuth stays continuous while the radius eases: each step turns by about the
        // angular velocity of the radius it is at.
        let mut previous = orbits.bodies()[&id(1)].phase;
        orbits.advance(0.1, 8.0);
        let step = (orbits.bodies()[&id(1)].phase - previous).rem_euclid(std::f32::consts::TAU);
        previous = lens::angular_velocity(orbits.bodies()[&id(1)].radius) * M_PER_SECOND * 0.1;
        assert!(
            (step - previous).abs() < 0.01 * previous,
            "{step} vs {previous}"
        );
    }

    #[test]
    fn a_body_goes_round_at_keplers_rate_whatever_the_frame_rate() {
        let run = |fps: u32| {
            let mut orbits = Orbits::default();
            take(&mut orbits, 1.0, &population(10), None, None);
            let start = orbits.bodies()[&id(5)].phase;
            for frame in 0..(3 * fps) {
                orbits.advance(1.0 / fps as f32, frame as f32 / fps as f32);
            }
            let body = &orbits.bodies()[&id(5)];
            let expected = start + lens::angular_velocity(body.radius) * M_PER_SECOND * 3.0;
            (body.phase, expected.rem_euclid(std::f32::consts::TAU))
        };
        for fps in [10, 30, 60] {
            let (phase, expected) = run(fps);
            assert!(
                (phase - expected).abs() < 1e-3,
                "{fps} fps: {phase} vs {expected}"
            );
        }
    }

    /// The ranked processes of `population(40)` with `gone` removed, as the next sample.
    fn without(gone: &[u32]) -> Vec<Candidate<'static>> {
        population(40)
            .into_iter()
            .filter(|candidate| !gone.contains(&candidate.id.pid))
            .collect()
    }

    #[test]
    fn an_oom_kill_plunges_the_best_ranked_vanished_process() {
        let mut orbits = Orbits::default();
        take(&mut orbits, 10.0, &population(40), Some(5), None);
        // Two exits at once, ranks 2 and 0, and the counter rises: the best-ranked one plunges.
        take(&mut orbits, 11.0, &without(&[3, 1]), Some(6), None);
        let fates: HashMap<Identity, Fate> = orbits
            .ghosts()
            .iter()
            .map(|ghost| (ghost.id, ghost.fate))
            .collect();
        assert_eq!(fates[&id(1)], Fate::Plunging);
        assert_eq!(fates[&id(3)], Fate::Fading);
        assert!(!orbits.bodies().contains_key(&id(1)));
        // It vanished in the middle of the interval between the two samples.
        assert_eq!(orbits.ghosts()[0].since, 10.5);
        // It plunges from where it stood.
        let ghost = &orbits.ghosts()[0];
        assert_eq!((ghost.id, ghost.radius), (id(1), 6.0));
    }

    #[test]
    fn an_exit_without_a_rise_of_the_counter_fades_in_place_and_goes() {
        let mut orbits = Orbits::default();
        take(&mut orbits, 10.0, &population(40), Some(5), None);
        take(&mut orbits, 11.0, &without(&[2]), Some(5), None);
        let ghost = orbits.ghosts()[0].clone();
        assert_eq!(ghost.fate, Fate::Fading);
        let radius = ghost.radius;
        assert_eq!(ghost.pose(10.5).light, 1.0);
        assert!((ghost.pose(10.5 + 0.6).light - 0.5).abs() < 1e-4);
        assert!(ghost.pose(10.5 + 1.2).light < 1e-3);
        assert_eq!(
            ghost.pose(11.0).radius,
            radius,
            "it does not leave its orbit"
        );
        // It has faded by 1.2 s but waits out the window for a late rise: the window of 2.5 s
        // plus the 2 s of the background thread plus a sample interval (1 s).
        assert!(ghost.gone(10.5 + 1.3));
        orbits.advance(0.1, 10.5 + 5.4);
        assert_eq!(orbits.ghosts().len(), 1);
        orbits.advance(0.1, 10.5 + 5.6);
        assert!(orbits.ghosts().is_empty());
        // A rise that comes in the next sample or the one after counts, one that comes later
        // than the window does not.
        for (late, fate) in [(1, Fate::Plunging), (4, Fate::Plunging), (8, Fate::Fading)] {
            let mut orbits = Orbits::default();
            take(&mut orbits, 10.0, &population(40), Some(5), None);
            take(&mut orbits, 11.0, &without(&[2]), Some(5), None);
            for sample in 1..late {
                take(
                    &mut orbits,
                    11.0 + sample as f64,
                    &without(&[2]),
                    Some(5),
                    None,
                );
            }
            take(
                &mut orbits,
                11.0 + late as f64,
                &without(&[2]),
                Some(6),
                None,
            );
            assert_eq!(orbits.ghosts()[0].fate, fate, "a rise {late} samples later");
        }
    }

    #[test]
    fn a_return_from_another_view_makes_no_ghosts_of_what_exited_meanwhile() {
        let mut orbits = Orbits::default();
        take(&mut orbits, 10.0, &population(40), Some(5), None);
        // Two minutes in another view: two processes exited and the counter rose twice.
        take(&mut orbits, 130.0, &without(&[1, 2]), Some(7), None);
        assert!(orbits.ghosts().is_empty(), "no ghost for an old exit");
        assert!(!orbits.bodies().contains_key(&id(1)));
        assert_eq!(
            orbits.bodies().len(),
            RANKED,
            "the top ranks are shown at once"
        );
        // From then on exits and kills are read as usual, and the old rises are not left to be
        // claimed by them.
        take(&mut orbits, 131.0, &without(&[1, 2]), Some(7), None);
        take(&mut orbits, 132.0, &without(&[1, 2, 3]), Some(7), None);
        assert_eq!(orbits.ghosts()[0].fate, Fate::Fading, "no rise: it fades");
        take(&mut orbits, 133.0, &without(&[1, 2, 3, 4]), Some(8), None);
        let fates: Vec<Fate> = orbits.ghosts().iter().map(|ghost| ghost.fate).collect();
        // A new rise takes the best-ranked of the two exits, which is the one before it.
        assert_eq!(fates, [Fate::Plunging, Fate::Fading]);
    }

    #[test]
    fn a_kill_the_counter_reports_samples_late_plunges_from_the_isco_not_mid_track() {
        // The frame thread sees the process go in the sample at 11 s; the background thread's
        // counter only rises in the sample at 13 s.
        let mut orbits = Orbits::default();
        take(&mut orbits, 10.0, &population(40), Some(5), None);
        take(&mut orbits, 11.0, &without(&[1]), Some(5), None);
        take(&mut orbits, 12.0, &without(&[1]), Some(5), None);
        assert_eq!(orbits.ghosts()[0].fate, Fate::Fading);
        take(&mut orbits, 13.0, &without(&[1]), Some(6), None);
        let ghost = orbits.ghosts()[0].clone();
        assert_eq!(ghost.fate, Fate::Plunging);
        assert_eq!(ghost.since, 10.5);
        assert_eq!(ghost.plunge_at, 12.5, "as if known half an interval ago");
        // So at 13 s it is still easing to the ISCO at full light, not 12 M down the track.
        let now = ghost.pose(13.0);
        assert!(now.plunging && now.light == 1.0 && now.stretch == 1.0);
        assert!(now.radius < ghost.radius + 0.01);
        assert!(!ghost.gone(13.0));
        // When it was known at once, the plunge starts with the exit, as before.
        let mut orbits = Orbits::default();
        take(&mut orbits, 10.0, &population(40), Some(5), None);
        take(&mut orbits, 11.0, &without(&[1]), Some(6), None);
        assert_eq!(orbits.ghosts()[0].plunge_at, orbits.ghosts()[0].since);
    }

    #[test]
    fn a_rise_that_comes_before_the_exit_still_counts() {
        // The kernel counts the kill before the process is gone.
        let mut orbits = Orbits::default();
        take(&mut orbits, 10.0, &population(40), Some(5), None);
        take(&mut orbits, 11.0, &population(40), Some(6), None);
        assert!(orbits.ghosts().is_empty());
        take(&mut orbits, 12.0, &without(&[1]), Some(6), None);
        assert_eq!(orbits.ghosts()[0].fate, Fate::Plunging);
    }

    #[test]
    fn without_a_counter_every_exit_fades_and_a_changed_source_is_not_a_rise() {
        let mut orbits = Orbits::default();
        take(&mut orbits, 10.0, &population(40), None, None);
        take(&mut orbits, 11.0, &without(&[1]), None, None);
        assert_eq!(orbits.ghosts()[0].fate, Fate::Fading);
        // From cgroups to vmstat, the numbers are not comparable.
        let mut orbits = Orbits::default();
        let candidates = population(40);
        let alive: HashSet<Identity> = candidates.iter().map(|c| c.id).collect();
        for (elapsed, count, source) in [
            (10.0, 4, KillSource::Cgroups),
            (11.0, 90, KillSource::Vmstat),
        ] {
            orbits.observe(&Sample {
                elapsed,
                time: elapsed as f32,
                candidates: &candidates,
                alive: &alive,
                selected: None,
                kills: Some(Kills { count, source }),
            });
        }
        take(&mut orbits, 12.0, &without(&[1]), Some(90), None);
        assert_eq!(orbits.ghosts()[0].fate, Fate::Fading);
    }

    #[test]
    fn a_paused_or_rewound_frame_changes_neither_ghosts_nor_the_counter() {
        let mut orbits = Orbits::default();
        take(&mut orbits, 10.0, &population(40), Some(5), None);
        take(&mut orbits, 11.0, &without(&[1]), Some(6), None);
        assert_eq!(orbits.ghosts().len(), 1);
        // Drawing the sample before, as a rewind does, does not bring the body back or make a
        // second ghost, and drawing the same sample again does not count the rise again.
        take(&mut orbits, 10.0, &population(40), Some(5), None);
        take(&mut orbits, 11.0, &without(&[1]), Some(6), None);
        assert_eq!(orbits.ghosts().len(), 1);
        take(&mut orbits, 12.0, &without(&[1, 2]), Some(6), None);
        assert_eq!(orbits.ghosts().len(), 2);
        let second = orbits
            .ghosts()
            .iter()
            .find(|ghost| ghost.id == id(2))
            .unwrap();
        assert_eq!(
            second.fate,
            Fate::Fading,
            "the first rise went to the first ghost"
        );
    }

    #[test]
    fn the_plunge_eases_to_the_isco_then_follows_the_track_and_goes() {
        let mut orbits = Orbits::default();
        take(&mut orbits, 10.0, &population(40), Some(5), None);
        take(&mut orbits, 11.0, &without(&[5]), Some(6), None);
        let ghost = orbits.ghosts()[0].clone();
        assert_eq!(ghost.fate, Fate::Plunging);
        let start = ghost.radius;
        assert!(start > 6.5, "rank 4 rides outside the ISCO");
        let mut last = (start, ghost.phase);
        let mut previous_light = 1.0;
        let mut ended = None;
        for step in 0..=2000 {
            let time = ghost.since + step as f32 * 0.005;
            if ghost.gone(time) {
                ended = Some(time - ghost.since);
                break;
            }
            let pose = ghost.pose(time);
            assert!(pose.plunging && pose.radius > 2.0 && pose.radius >= PLUNGE_FLOOR);
            assert!(pose.radius <= last.0 + 1e-3, "radius only falls, at {step}");
            assert!(
                pose.azimuth >= last.1 - 1e-4,
                "and it goes round, at {step}"
            );
            assert!(pose.light <= previous_light + 1e-4, "and dims, at {step}");
            (last, previous_light) = ((pose.radius, pose.azimuth), pose.light);
        }
        assert!((ghost.pose(ghost.since + ISCO_SECONDS).radius - 6.0).abs() < 1e-3);
        let ended = ended.expect("the image fades below 0.02 within ten seconds");
        assert!((6.0..8.0).contains(&ended), "removed after {ended} s");
        // Stretch grows as it falls in.
        assert_eq!(ghost.pose(ghost.since + 0.5).stretch, 1.0);
        assert!(ghost.pose(ghost.since + 5.0).stretch > 2.0);
    }

    #[test]
    fn a_vanished_process_that_was_not_shown_leaves_no_ghost() {
        let mut orbits = Orbits::default();
        take(&mut orbits, 10.0, &population(40), Some(5), None);
        // Rank 33 is collapsed: its exit and the counter's rise draw nothing.
        take(&mut orbits, 11.0, &without(&[34]), Some(6), None);
        assert!(orbits.ghosts().is_empty());
        // And a body that merely stops being shown is not an exit.
        take(&mut orbits, 12.0, &population(40), Some(6), Some(id(35)));
        take(&mut orbits, 13.0, &population(40), Some(6), None);
        assert!(orbits.ghosts().is_empty());
    }
}
