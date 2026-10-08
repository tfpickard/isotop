//! Globe: where this machine's TCP connections go. The world turns once every five minutes;
//! great-circle arcs rise from home to every remote place, brighter with more traffic, and pulses
//! travel the way the bytes flow. Processes with connections hover above home. Locations come from
//! a local GeoIP database; addresses it cannot place circle the north pole.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::net::IpAddr;

use crate::model::{Identity, Place, Process, Remote, Snapshot, bounded, bytes};
use crate::pack::Seats;
use crate::render::{
    Color, GOLDEN_ANGLE, NONE, Point, SCALE, Stage, dot, kind_color, mass_radius, normalize, tint,
};

const RADIUS: f32 = 20.0;
const SPIN_SECONDS: f32 = 300.0;
/// Natural Earth 110 m coastlines: per line a little-endian u16 point count, then i16 pairs of
/// longitude and latitude in hundredths of a degree.
const COASTLINE: &[u8] = include_bytes!("coastline.bin");
const OCEAN: Color = [20, 50, 82];
const COAST: Color = [140, 196, 166];
const GRATICULE: Color = [34, 66, 96];
const DOWN: Color = [110, 220, 255];
const UP: Color = [255, 140, 210];
const HOME: Color = [255, 214, 150];

#[derive(Default)]
pub struct Globe {
    coast: Vec<Vec<[f32; 2]>>,
    /// Places in the cloud above home, kept for life so connections coming and going don't
    /// reshuffle the other processes.
    seats: Seats,
}

/// Remote connections that land in the same place.
struct Endpoint {
    place: Option<Place>,
    up: f32,
    down: f32,
    count: usize,
    /// The process moving the most bytes there, which clicking the endpoint selects.
    top: (f32, Identity),
    key: Spot,
    /// A stable number derived from the key, for display angles and pulse phases.
    phase: u64,
}

/// Where connections are grouped: located ones by rounded coordinates, unlocated ones per address.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Spot {
    Place(i32, i32),
    Address(IpAddr),
}

impl Globe {
    pub fn draw(
        &mut self,
        stage: &mut Stage,
        processes: &[&Process],
        snapshot: &Snapshot,
    ) -> usize {
        if self.coast.is_empty() {
            self.coast = coastline();
        }
        let camera = stage.camera;
        let time = stage.time;
        let center: Point = [0.0, 0.0, RADIUS];
        let home = snapshot.home.clone().unwrap_or(Place {
            latitude: 0.0,
            longitude: 0.0,
            name: "home unknown: pass --home LAT,LON".into(),
        });
        // Start with home turned towards the default isometric camera, which looks from 45°.
        let spin =
            std::f32::consts::FRAC_PI_4 - home.longitude.to_radians() + TAU * time / SPIN_SECONDS;
        let direction = |latitude: f32, longitude: f32| -> Point {
            let (lat, lon) = (latitude.to_radians(), longitude.to_radians() + spin);
            [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
        };
        let at = |d: Point, r: f32| -> Point { [0, 1, 2].map(|k| center[k] + d[k] * r) };
        let viewer = camera.viewer();
        let facing = |p: Point| {
            let offset = [p[0] - center[0], p[1] - center[1], p[2] - center[2]];
            let along = dot(offset, viewer);
            along > 0.0 || dot(offset, offset) - along * along > RADIUS * RADIUS
        };
        stage.frame.glow(
            camera,
            center,
            RADIUS * camera.zoom * SCALE * 1.12,
            [70, 140, 220],
            0.35,
        );
        stage.frame.world_sphere(camera, center, RADIUS, OCEAN);
        for line in &self.coast {
            for pair in line.windows(2) {
                let a = at(direction(pair[0][1], pair[0][0]), RADIUS * 1.004);
                let b = at(direction(pair[1][1], pair[1][0]), RADIUS * 1.004);
                stage.frame.line(camera, a, b, COAST);
            }
        }
        for meridian in (0..360).step_by(30) {
            for step in 0..32 {
                let (a, b) = (-80.0 + step as f32 * 5.0, -75.0 + step as f32 * 5.0);
                let lon = meridian as f32;
                stage.frame.line(
                    camera,
                    at(direction(a, lon), RADIUS * 1.002),
                    at(direction(b, lon), RADIUS * 1.002),
                    GRATICULE,
                );
            }
        }
        for parallel in [-60.0, -30.0, 0.0, 30.0, 60.0] {
            for step in 0..72 {
                let (a, b) = (step as f32 * 5.0, (step + 1) as f32 * 5.0);
                stage.frame.line(
                    camera,
                    at(direction(parallel, a), RADIUS * 1.002),
                    at(direction(parallel, b), RADIUS * 1.002),
                    GRATICULE,
                );
            }
        }
        let up = direction(home.latitude, home.longitude);
        let home_point = at(up, RADIUS * 1.01);
        stage
            .frame
            .glow(camera, home_point, (1.2 * camera.zoom).max(10.0), HOME, 0.8);
        stage
            .frame
            .sphere(camera, home_point, 0.35, HOME, NONE, false);
        stage
            .places
            .push((at(up, RADIUS * 1.01), home.name.clone()));
        let index: HashMap<Identity, usize> = processes
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id, i))
            .collect();
        let mut endpoints: HashMap<Spot, Endpoint> = HashMap::new();
        let mut per_process: HashMap<Identity, Vec<&Remote>> = HashMap::new();
        for remote in snapshot
            .remotes
            .iter()
            .filter(|r| index.contains_key(&r.id))
        {
            per_process.entry(remote.id).or_default().push(remote);
            let key = match &remote.place {
                Some(place) => Spot::Place(
                    place.latitude.round() as i32,
                    place.longitude.round() as i32,
                ),
                None => Spot::Address(remote.address),
            };
            let traffic = remote.up + remote.down;
            let endpoint = endpoints.entry(key).or_insert_with(|| Endpoint {
                place: remote.place.clone(),
                up: 0.0,
                down: 0.0,
                count: 0,
                top: (-1.0, remote.id),
                key,
                phase: hash(&match key {
                    Spot::Place(lat, lon) => format!("{lat},{lon}"),
                    Spot::Address(address) => address.to_string(),
                }),
            });
            endpoint.up += remote.up;
            endpoint.down += remote.down;
            endpoint.count += 1;
            if traffic > endpoint.top.0 {
                endpoint.top = (traffic, remote.id);
            }
        }
        let mut endpoints: Vec<Endpoint> = endpoints.into_values().collect();
        endpoints.sort_by(|a, b| {
            (b.up + b.down)
                .total_cmp(&(a.up + a.down))
                .then(a.key.cmp(&b.key))
        });
        for (rank, endpoint) in endpoints.iter().enumerate() {
            let target = match &endpoint.place {
                Some(place) => direction(place.latitude, place.longitude),
                None => {
                    let angle = (endpoint.phase % 1000) as f32 / 1000.0 * TAU + spin;
                    normalize([0.45 * angle.cos(), 0.45 * angle.sin(), 1.0])
                }
            };
            let height = if endpoint.place.is_some() { 1.01 } else { 1.25 };
            let traffic = endpoint.up + endpoint.down;
            let color = if endpoint.down >= endpoint.up {
                DOWN
            } else {
                UP
            };
            let shade = tint(color, 0.35 + 0.65 * bounded(traffic, 50_000.0));
            let angle = dot(up, target).clamp(-1.0, 1.0).acos();
            let arc = |t: f32| {
                let d = slerp(up, target, angle, t);
                let bulge = RADIUS * (0.04 + 0.22 * angle / PI) * (PI * t).sin();
                at(d, RADIUS * (1.01 + (height - 1.01) * t) + bulge)
            };
            let mut last = arc(0.0);
            for k in 1..=40 {
                let next = arc(k as f32 / 40.0);
                stage.frame.line(camera, last, next, shade);
                last = next;
            }
            if traffic > 64.0 {
                let toward_home = endpoint.down >= endpoint.up;
                let speed = 0.15 + 0.5 * bounded(traffic, 100_000.0);
                for k in 0..3 {
                    let mut t =
                        (time * speed + k as f32 / 3.0 + (endpoint.phase % 97) as f32 / 97.0)
                            .fract();
                    if toward_home {
                        t = 1.0 - t;
                    }
                    let p = arc(t);
                    if facing(p) {
                        stage
                            .frame
                            .glow(camera, p, (0.5 * camera.zoom).max(5.0), color, 0.8);
                    }
                }
            }
            let end = arc(1.0);
            let pick = index.get(&endpoint.top.1).map_or(NONE, |&i| i as u32);
            stage.frame.glow(
                camera,
                end,
                (0.9 * camera.zoom).max(7.0),
                color,
                0.3 + 0.5 * bounded(traffic, 50_000.0),
            );
            stage.frame.sphere(camera, end, 0.3, color, pick, false);
            if rank < 12 && facing(end) {
                let name = endpoint
                    .place
                    .as_ref()
                    .map_or_else(|| "unlocated".into(), |p| p.name.clone());
                let many = if endpoint.count > 1 {
                    format!(" x{}", endpoint.count)
                } else {
                    String::new()
                };
                stage.places.push((end, format!("{name}{many}")));
            }
        }
        let (east, north) = tangents(up);
        let mut talkers: Vec<Identity> = per_process.keys().copied().collect();
        talkers.sort();
        self.seats.assign(talkers.iter().map(|&id| (id, "home")));
        for id in &talkers {
            let process = processes[index[id]];
            let k = self.seats.seat(*id).unwrap_or(0);
            let distance = 0.9 * (k as f32 + 0.5).sqrt();
            let angle = k as f32 * GOLDEN_ANGLE;
            let offset =
                [0, 1, 2].map(|i| (east[i] * angle.cos() + north[i] * angle.sin()) * distance);
            let base = at(up, RADIUS + 2.5);
            let position = [0, 1, 2].map(|i| base[i] + offset[i]);
            stage
                .frame
                .line(camera, home_point, position, tint(kind_color(process), 0.4));
            let size = (0.25 + 0.3 * mass_radius(process.memory as f32)).min(0.6)
                * stage.growth(process.id);
            stage.frame.sphere(
                camera,
                position,
                size,
                kind_color(process),
                index[id] as u32,
                stage.selected == Some(*id),
            );
            stage.positions.insert(*id, position);
            let mut lines: Vec<String> = per_process[id]
                .iter()
                .take(4)
                .map(|r| {
                    let place = r.place.as_ref().map_or("unlocated", |p| p.name.as_str());
                    format!(
                        "{}:{} {place} | {:.0} ms | up {}/s down {}/s",
                        r.address,
                        r.port,
                        r.rtt,
                        bytes(r.up as u64),
                        bytes(r.down as u64)
                    )
                })
                .collect();
            if per_process[id].len() > 4 {
                lines.push(format!(
                    "and {} more connections",
                    per_process[id].len() - 4
                ));
            }
            stage.notes.insert(*id, lines);
        }
        // Points spread evenly over a sphere a little larger than the globe and its arcs.
        for k in 0..64 {
            let z = 1.0 - 2.0 * (k as f32 + 0.5) / 64.0;
            let ring = (1.0 - z * z).sqrt();
            let angle = k as f32 * GOLDEN_ANGLE;
            stage.bounds.push(at(
                [ring * angle.cos(), ring * angle.sin(), z],
                RADIUS * 1.3,
            ));
        }
        talkers.len()
    }
}

/// Spherical interpolation between unit vectors `a` and `b`, `angle` apart.
fn slerp(a: Point, b: Point, angle: f32, t: f32) -> Point {
    if angle < 1e-4 {
        return a;
    }
    let (wa, wb) = (
        ((1.0 - t) * angle).sin() / angle.sin(),
        (t * angle).sin() / angle.sin(),
    );
    normalize([0, 1, 2].map(|k| a[k] * wa + b[k] * wb))
}

/// East and north unit vectors on the surface at unit vector `up`.
fn tangents(up: Point) -> (Point, Point) {
    // At a pole every horizontal direction is a tangent and "east" is undefined; pick one.
    let east = if up[0].hypot(up[1]) < 1e-4 {
        [1.0, 0.0, 0.0]
    } else {
        normalize([-up[1], up[0], 0.0])
    };
    let north = [
        up[1] * east[2] - up[2] * east[1],
        up[2] * east[0] - up[0] * east[2],
        up[0] * east[1] - up[1] * east[0],
    ];
    (east, north)
}

fn hash(text: &str) -> u64 {
    text.bytes().fold(14_695_981_039_346_656_037_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(1_099_511_628_211)
    })
}

/// Coastlines as polylines of (longitude, latitude) in degrees.
fn coastline() -> Vec<Vec<[f32; 2]>> {
    let mut lines = Vec::new();
    let mut at = 0;
    let value = |i: usize| i16::from_le_bytes([COASTLINE[i], COASTLINE[i + 1]]) as f32 / 100.0;
    while at + 2 <= COASTLINE.len() {
        let count = u16::from_le_bytes([COASTLINE[at], COASTLINE[at + 1]]) as usize;
        at += 2;
        if at + count * 4 > COASTLINE.len() {
            break;
        }
        lines.push(
            (0..count)
                .map(|k| [value(at + k * 4), value(at + k * 4 + 2)])
                .collect(),
        );
        at += count * 4;
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coastline_decodes_and_arcs_stay_on_the_sphere() {
        let lines = coastline();
        assert_eq!(lines.len(), 134);
        assert_eq!(lines.iter().map(Vec::len).sum::<usize>(), 5128);
        assert!(
            lines
                .iter()
                .flatten()
                .all(|p| p[0].abs() <= 180.0 && p[1].abs() <= 90.0)
        );
        let (a, b) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let middle = slerp(a, b, PI / 2.0, 0.5);
        assert!((dot(middle, middle) - 1.0).abs() < 1e-5);
        assert!((middle[0] - middle[1]).abs() < 1e-5);
        for up in [[0.6, 0.8, 0.0], [0.0, 0.0, 1.0], [0.0, 0.0, -1.0]] {
            let (east, north) = tangents(up);
            assert!((dot(east, east) - 1.0).abs() < 1e-5, "{up:?}");
            assert!((dot(north, north) - 1.0).abs() < 1e-5, "{up:?}");
            assert!(dot(east, north).abs() < 1e-5 && dot(east, up).abs() < 1e-5);
        }
    }
}
