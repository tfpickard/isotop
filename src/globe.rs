//! Globe: where this machine's TCP connections go. The world turns once every five minutes;
//! great-circle arcs rise from home to every remote place, brighter with more traffic, and pulses
//! travel the way the bytes flow. Processes with connections hover above home. Locations come from
//! a local GeoIP database; addresses that have no place circle the north pole, and a label says
//! why: no database, not listed, still being located, or private.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
use std::net::IpAddr;

use crate::model::{Identity, Location, Place, Process, Remote, Snapshot, bounded, bytes};
use crate::pack::Seats;
use crate::render::{
    Camera, Color, GOLDEN_ANGLE, LOWEST_PITCH, NONE, Point, SCALE, Stage, dot, kind_color,
    mass_radius, normalize, tint,
};

pub(crate) const RADIUS: f32 = 20.0;
const SPIN_SECONDS: f32 = 300.0;
/// Natural Earth 110 m coastlines: per line a little-endian u16 point count, then i16 pairs of
/// longitude and latitude in hundredths of a degree.
const COASTLINE: &[u8] = include_bytes!("coastline.bin");
const OCEAN: Color = [20, 50, 82];
const COAST: Color = [140, 196, 166];
const GRATICULE: Color = [34, 66, 96];
const DOWN: Color = [110, 220, 255];
const UP: Color = [255, 140, 210];
/// Arc colour when the platform reports no per-connection rates, so no direction is implied.
const NEUTRAL: Color = [170, 190, 215];
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
    location: Location,
    address: IpAddr,
    up: f32,
    down: f32,
    count: usize,
    /// The process moving the most bytes there, which clicking the endpoint selects.
    top: (f32, Identity),
    key: Spot,
    /// A stable number derived from the key, for display angles and pulse phases.
    phase: u64,
}

/// Where connections are grouped: placed ones by rounded coordinates, the rest per address.
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
        let spin = spin(&home, camera, time);
        // How the earth leans, applied to every direction on it so that it all leans together.
        let lean = tilt_of(camera);
        let tilt = |v: Point| -> Point {
            if lean == 0.0 {
                v
            } else {
                lean_about_right(v, camera.rotation, lean)
            }
        };
        let direction = |latitude: f32, longitude: f32| -> Point {
            let (lat, azimuth) = (latitude.to_radians(), spin - longitude.to_radians());
            tilt([
                lat.cos() * azimuth.cos(),
                lat.cos() * azimuth.sin(),
                lat.sin(),
            ])
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
        let has_rates = !snapshot.missing.contains(&"socket traffic");
        let mut endpoints: HashMap<Spot, Endpoint> = HashMap::new();
        let mut per_process: HashMap<Identity, Vec<&Remote>> = HashMap::new();
        for remote in snapshot
            .remotes
            .iter()
            .filter(|r| index.contains_key(&r.id))
        {
            per_process.entry(remote.id).or_default().push(remote);
            let key = match remote.location.place() {
                Some(place) => Spot::Place(
                    place.latitude.round() as i32,
                    place.longitude.round() as i32,
                ),
                None => Spot::Address(remote.address),
            };
            let traffic = remote.up + remote.down;
            let endpoint = endpoints.entry(key).or_insert_with(|| Endpoint {
                location: remote.location.clone(),
                address: remote.address,
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
            let target = match endpoint.location.place() {
                Some(place) => direction(place.latitude, place.longitude),
                None => {
                    let angle = (endpoint.phase % 1000) as f32 / 1000.0 * TAU + spin;
                    tilt(normalize([0.45 * angle.cos(), 0.45 * angle.sin(), 1.0]))
                }
            };
            let height = if endpoint.location.place().is_some() {
                1.01
            } else {
                1.25
            };
            let traffic = endpoint.up + endpoint.down;
            let color = if !has_rates {
                NEUTRAL
            } else if endpoint.down >= endpoint.up {
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
            if rank < 12
                && facing(end)
                && let Some(name) = endpoint_label(&endpoint.location, endpoint.address)
            {
                let many = if endpoint.count > 1 {
                    format!(" x{}", endpoint.count)
                } else {
                    String::new()
                };
                stage.places.push((end, format!("{name}{many}")));
            }
        }
        // The ring's centre, which is where addresses without a place circle.
        let pole = tilt([0.0, 0.0, 1.0]);
        let ring = at(pole, RADIUS * 1.25);
        if facing(ring) {
            let count = |wanted: Location| {
                endpoints
                    .iter()
                    .filter(|endpoint| endpoint.location == wanted)
                    .count()
            };
            if let Some(text) = ring_label(count(Location::NoDatabase), count(Location::Pending)) {
                stage.places.push((ring, text));
            }
        }
        let (east, north) = tangents(up, pole, tilt([1.0, 0.0, 0.0]));
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
                    let place = location_note(&r.location);
                    if has_rates {
                        format!(
                            "{}:{} {place} | {:.0} ms | up {}/s down {}/s",
                            r.address,
                            r.port,
                            r.rtt,
                            bytes(r.up as u64),
                            bytes(r.down as u64)
                        )
                    } else {
                        format!("{}:{} {place}", r.address, r.port)
                    }
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

/// How far the earth has turned by `time` while it is free, in radians about its axis. The
/// earth turns eastward, which is towards falling azimuth (see `spin`), so this falls.
pub(crate) fn drift(time: f32) -> f32 {
    -TAU * time / SPIN_SECONDS
}

/// The azimuth, about the world's vertical axis, at which longitude zero lies. A place's
/// azimuth is `spin - longitude`, so east is the direction of falling azimuth. Camera::view puts
/// screen-right at falling azimuth, which draws east on the right as on any map.
///
/// At time zero home sits at azimuth 45 degrees, facing the default isometric camera. The
/// earth then turns eastward, so the surface that faces the viewer moves to the right, until
/// someone holds it still and turns it by hand (`Camera::globe_turn`).
fn spin(home: &Place, camera: &Camera, time: f32) -> f32 {
    let free = if camera.globe_held { 0.0 } else { drift(time) };
    FRAC_PI_4 + home.longitude.to_radians() + camera.globe_turn + free
}

/// The earth's lean from `camera`, kept so the latitude facing the viewer, `pitch - tilt`, stays
/// between the poles: the pitch changes without telling the tilt.
fn tilt_of(camera: &Camera) -> f32 {
    camera
        .globe_tilt
        .clamp(camera.pitch - FRAC_PI_2, camera.pitch + FRAC_PI_2)
}

/// `v` turned by `angle` about the screen-horizontal axis R = (cos(r + pi/4), -sin(r + pi/4), 0)
/// of a camera with rotation `rotation`, right-handed about +R. A positive angle brings the
/// southern hemisphere up towards the viewer.
fn lean_about_right(v: Point, rotation: f32, angle: f32) -> Point {
    let (s, c) = (rotation + FRAC_PI_4).sin_cos();
    let axis = [c, -s, 0.0];
    let (sin, cos) = angle.sin_cos();
    let cross = [
        axis[1] * v[2] - axis[2] * v[1],
        axis[2] * v[0] - axis[0] * v[2],
        axis[0] * v[1] - axis[1] * v[0],
    ];
    let along = dot(axis, v) * (1.0 - cos);
    [0, 1, 2].map(|k| v[k] * cos + cross[k] * sin + axis[k] * along)
}

/// A point of the scene drawn with `camera` as it would be drawn on an earth that was not
/// leaning, which is where it ends up once the earth has been stood upright again.
pub(crate) fn upright(point: Point, camera: &Camera) -> Point {
    let lean = tilt_of(camera);
    if lean == 0.0 {
        return point;
    }
    let offset = [point[0], point[1], point[2] - RADIUS];
    let [x, y, z] = lean_about_right(offset, camera.rotation, -lean);
    [x, y, z + RADIUS]
}

/// The label at an endpoint on the globe: a place's name, or the address of a connection the
/// GeoIP database does not list or that is private. An address with no database or still being
/// located has none; the ring's label speaks for those.
fn endpoint_label(location: &Location, address: IpAddr) -> Option<String> {
    match location {
        Location::Known(place) => Some(place.name.clone()),
        Location::Unlisted => Some(address.to_string()),
        Location::Private => Some(format!("{address} (private)")),
        Location::NoDatabase | Location::Pending => None,
    }
}

/// The one label at the ring's centre, counting addresses that have no place yet or no way to
/// get one.
fn ring_label(no_database: usize, pending: usize) -> Option<String> {
    let addresses = |count: usize| format!("{count} address{}", if count == 1 { "" } else { "es" });
    let mut parts = Vec::new();
    if no_database > 0 {
        parts.push(format!(
            "no GeoIP database: {} (see --geoip)",
            addresses(no_database)
        ));
    }
    if pending > 0 {
        parts.push(format!("locating {}", addresses(pending)));
    }
    (!parts.is_empty()).then(|| parts.join("; "))
}

/// What the inspector says about where a connection goes.
fn location_note(location: &Location) -> &str {
    match location {
        Location::Known(place) => &place.name,
        Location::Private => "private",
        Location::Unlisted => "not in the GeoIP database",
        Location::NoDatabase => "no GeoIP database",
        Location::Pending => "locating",
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

/// The camera rotation and pitch that put `point` on the near side of the globe. The rotation
/// turns the viewer to the point's bearing from the globe's centre; the camera's pitch is kept
/// unless the point would still sit near or behind the limb, as it can below the equator,
/// because the camera only looks from above. At the lowest pitch the camera allows, about 20
/// degrees, nothing more than about 70 degrees south of the equator can be brought into view.
pub fn face(point: Point, camera: &Camera) -> (f32, f32) {
    let offset = [point[0], point[1], point[2] - RADIUS];
    let across = offset[0].hypot(offset[1]);
    // Camera::viewer points along (sin(r + pi/4), cos(r + pi/4)) on the ground.
    let rotation = if across > 1e-4 {
        offset[0].atan2(offset[1]) - FRAC_PI_4
    } else {
        camera.rotation
    };
    let length = across.hypot(offset[2]);
    let (sin, cos) = camera.pitch.sin_cos();
    // With the bearing matched, how far the point leans towards the viewer is
    // across * cos(pitch) + z * sin(pitch); a quarter of its distance keeps it clear of the limb.
    let pitch = if across * cos + offset[2] * sin >= 0.25 * length {
        camera.pitch
    } else {
        offset[2].atan2(across).clamp(LOWEST_PITCH, FRAC_PI_2)
    };
    (rotation, pitch)
}

/// East and north unit vectors on the surface at unit vector `up`, on an earth whose north pole
/// points along `pole`. Longitude grows as azimuth falls (see `spin`), so east is the direction
/// of falling azimuth, `up x pole`, and (east, north, up) is left-handed in world coordinates:
/// north is east x up. At a pole every horizontal direction is a tangent and "east" is
/// undefined, so `fallback`, a unit vector at right angles to `pole`, stands in.
fn tangents(up: Point, pole: Point, fallback: Point) -> (Point, Point) {
    let across = [
        up[1] * pole[2] - up[2] * pole[1],
        up[2] * pole[0] - up[0] * pole[2],
        up[0] * pole[1] - up[1] * pole[0],
    ];
    let east = if dot(across, across).sqrt() < 1e-4 {
        fallback
    } else {
        normalize(across)
    };
    let north = [
        east[1] * up[2] - east[2] * up[1],
        east[2] * up[0] - east[0] * up[2],
        east[0] * up[1] - east[1] * up[0],
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

/// The status legend, saying what the snapshot's platform leaves out.
pub fn legend(snapshot: &Snapshot) -> String {
    if snapshot.missing.contains(&"socket traffic") {
        format!(
            " Arcs = TCP connections from home | no per-connection rates or RTT on macOS | {}",
            snapshot.geo
        )
    } else {
        format!(
            " Arcs = TCP connections from home, brighter with traffic | cyan = mostly download, pink = mostly upload | {}",
            snapshot.geo
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facing_turns_the_viewer_towards_any_point_on_the_sphere() {
        let center = [0.0, 0.0, RADIUS];
        // As far south as people live; see face for the limit.
        for latitude in [-55.0f32, -30.0, -10.0, 0.0, 30.0, 70.0] {
            for longitude in (0..360).step_by(20) {
                let (lat, lon) = (latitude.to_radians(), (longitude as f32).to_radians());
                let direction = [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()];
                let point = [0, 1, 2].map(|k| center[k] + direction[k] * (RADIUS + 2.5));
                for rotation in [0.0, 2.0, -3.0] {
                    let mut camera = Camera {
                        rotation,
                        ..Camera::default()
                    };
                    (camera.rotation, camera.pitch) = face(point, &camera);
                    let towards = dot(camera.viewer(), direction);
                    assert!(
                        towards > 0.2,
                        "{latitude} {longitude} from {rotation}: {towards}"
                    );
                }
            }
        }
        let northern = [10.0, -5.0, RADIUS + 12.0];
        let camera = Camera::default();
        assert_eq!(
            face(northern, &camera).1,
            camera.pitch,
            "a point already in view keeps the pitch"
        );
    }

    #[test]
    fn legend_drops_rates_when_the_platform_has_no_socket_traffic() {
        let mut snapshot = Snapshot {
            geo: "demo locations".into(),
            ..Default::default()
        };
        let full = legend(&snapshot);
        assert!(full.contains("brighter with traffic | cyan = mostly download"));
        assert!(full.ends_with("| demo locations"));
        snapshot.missing = vec!["socket traffic"];
        let text = legend(&snapshot);
        assert!(text.contains("no per-connection rates or RTT on macOS"));
        assert!(!text.contains("cyan") && text.ends_with("| demo locations"));
    }

    #[test]
    fn notes_and_arcs_carry_no_invented_rates_without_socket_traffic() {
        use crate::model::demo;
        use crate::render::{Camera, Item, Scene, View};
        let mut snapshot = demo(10.0, 128);
        for remote in &mut snapshot.remotes {
            (remote.rtt, remote.up, remote.down) = (0.0, 0.0, 0.0);
        }
        let id = snapshot.remotes[0].id;
        let render = |snapshot: &Snapshot| {
            let mut scene = Scene::new();
            let frame = scene.render(
                snapshot,
                View::Globe,
                &Camera::default(),
                320,
                180,
                None,
                10.0,
                512,
                None,
            );
            (scene.notes, frame.items)
        };
        let (notes, _) = render(&snapshot);
        assert!(notes[&id][0].contains(" ms | up "));
        snapshot.missing = vec!["socket traffic"];
        let (notes, items) = render(&snapshot);
        let lines = &notes[&id];
        assert!(!lines.is_empty());
        assert!(
            lines
                .iter()
                .all(|line| !line.contains(" ms") && !line.contains("/s")),
            "{lines:?}"
        );
        assert!(lines[0].starts_with(&snapshot.remotes[0].address.to_string()));
        let colors: Vec<Color> = items
            .iter()
            .filter_map(|item| match item {
                Item::Sphere { color, .. } => Some(*color),
                _ => None,
            })
            .collect();
        assert!(colors.contains(&NEUTRAL));
        assert!(!colors.contains(&DOWN) && !colors.contains(&UP));
    }

    fn named(name: &str, latitude: f32, longitude: f32) -> Place {
        Place {
            latitude,
            longitude,
            name: name.into(),
        }
    }

    /// The scene and frame the globe draws for `camera`.
    fn drawn(
        snapshot: &Snapshot,
        camera: &Camera,
        time: f32,
    ) -> (crate::render::Scene, crate::render::Frame) {
        use crate::render::{Scene, View};
        let mut scene = Scene::new();
        let frame = scene.render(
            snapshot,
            View::Globe,
            camera,
            640,
            360,
            None,
            time,
            512,
            None,
        );
        (scene, frame)
    }

    /// Frame-pixel positions of the globe's labelled points, by label, as the camera projects them.
    fn labelled(snapshot: &Snapshot, camera: &Camera, time: f32) -> HashMap<String, [f32; 2]> {
        let (scene, frame) = drawn(snapshot, camera, time);
        scene
            .places
            .iter()
            .map(|(point, name)| {
                let at = frame.project(camera, *point);
                (name.clone(), [at[0], at[1]])
            })
            .collect()
    }

    /// Where the globe's centre falls in the frame.
    fn centre(camera: &Camera) -> [f32; 2] {
        let (_, frame) = drawn(&connected_to(&[]), camera, 0.0);
        let at = frame.project(camera, [0.0, 0.0, RADIUS]);
        [at[0], at[1]]
    }

    /// A camera that holds the earth still, leaning by `tilt`.
    fn leaning(tilt: f32, turn: f32) -> Camera {
        Camera {
            globe_tilt: tilt,
            globe_turn: turn,
            globe_held: true,
            ..Camera::default()
        }
    }

    fn rotated_about(axis: Point, angle: f32, v: Point) -> Point {
        let (sin, cos) = angle.sin_cos();
        let cross = [
            axis[1] * v[2] - axis[2] * v[1],
            axis[2] * v[0] - axis[0] * v[2],
            axis[0] * v[1] - axis[1] * v[0],
        ];
        let along = dot(axis, v) * (1.0 - cos);
        [0, 1, 2].map(|k| v[k] * cos + cross[k] * sin + axis[k] * along)
    }

    #[test]
    fn tilting_brings_either_pole_to_face_the_viewer() {
        let snapshot = connected_to(&[
            named("North Pole", 90.0, 0.0),
            named("South Pole", -90.0, 0.0),
        ]);
        let pitch = Camera::default().pitch;
        let middle = centre(&Camera::default());
        for (pole, tilt) in [
            ("North Pole", pitch - FRAC_PI_2),
            ("South Pole", pitch + FRAC_PI_2),
        ] {
            let camera = leaning(tilt, 0.0);
            let at = labelled(&snapshot, &camera, 0.0)[pole];
            assert!(
                (at[0] - middle[0]).abs() < 0.1 && (at[1] - middle[1]).abs() < 0.1,
                "{pole} at {at:?}, centre {middle:?}"
            );
        }
        // Without the lean, the south pole is behind the earth and the north pole is not at its centre.
        let level = labelled(&snapshot, &leaning(0.0, 0.0), 0.0);
        assert!(!level.contains_key("South Pole"));
        assert!((level["North Pole"][1] - middle[1]).abs() > 20.0);
    }

    #[test]
    fn turning_spins_the_earth_about_its_own_axis_even_when_tilted() {
        let mut snapshot = connected_with(&[("8.8.8.8", Location::NoDatabase)]);
        snapshot.home = Some(named("London", 51.5, -0.1));
        let label = "no GeoIP database: 1 address (see --geoip)";
        let first = labelled(&snapshot, &leaning(0.5, 0.0), 0.0);
        // The label sits over the north pole as the lean has carried it.
        let camera = leaning(0.5, 0.0);
        let (_, frame) = drawn(&snapshot, &camera, 0.0);
        let pole = lean_about_right([0.0, 0.0, 1.0], camera.rotation, 0.5);
        let over = frame.project(
            &camera,
            [0, 1, 2].map(|k| [0.0, 0.0, RADIUS][k] + pole[k] * RADIUS * 1.25),
        );
        assert!(
            (first[label][0] - over[0]).abs() < 0.01 && (first[label][1] - over[1]).abs() < 0.01,
            "{:?} against {over:?}",
            first[label]
        );
        for turn in [0.4, 1.3, 3.0] {
            let later = labelled(&snapshot, &leaning(0.5, turn), 0.0);
            let (before, after) = (first[label], later[label]);
            assert!(
                (before[0] - after[0]).abs() < 0.01 && (before[1] - after[1]).abs() < 0.01,
                "the pole moved: {before:?} to {after:?} at turn {turn}"
            );
            let (home, moved) = (first["London"], later["London"]);
            assert!(
                (home[0] - moved[0]).hypot(home[1] - moved[1]) > 5.0,
                "home did not move at turn {turn}"
            );
        }
    }

    #[test]
    fn talkers_keep_their_seats_around_home_when_a_tilted_earth_turns() {
        let mut snapshot = crate::model::demo(10.0, 128);
        snapshot.home = Some(named("London", 51.5, -0.1));
        let tilt = 0.5;
        let (a, b) = (0.4, 1.3);
        let (first, _) = drawn(&snapshot, &leaning(tilt, a), 0.0);
        let (second, _) = drawn(&snapshot, &leaning(tilt, b), 0.0);
        assert!(first.positions.len() > 3);
        // The earth's axis after the lean, and turning by hand about it by b - a.
        let axis = lean_about_right([0.0, 0.0, 1.0], Camera::default().rotation, tilt);
        for (id, at) in &first.positions {
            let offset = [at[0], at[1], at[2] - RADIUS];
            let turned = rotated_about(axis, b - a, offset);
            let expected = [turned[0], turned[1], turned[2] + RADIUS];
            let seen = second.positions[id];
            assert!(
                (0..3).all(|k| (expected[k] - seen[k]).abs() < 1e-3),
                "{expected:?} against {seen:?}"
            );
            assert!((0..3).any(|k| (at[k] - seen[k]).abs() > 0.1), "never moved");
        }
    }

    #[test]
    fn changing_pitch_never_tips_the_earth_past_a_pole() {
        let snapshot = crate::model::demo(10.0, 128);
        let picture = |camera: &Camera| format!("{:?}", drawn(&snapshot, camera, 0.0).1.items);
        for pitch in [LOWEST_PITCH, Camera::default().pitch, 1.2, FRAC_PI_2] {
            for tilt in [-5.0, -1.0, 0.0, 1.0, 5.0] {
                let camera = Camera {
                    pitch,
                    ..leaning(tilt, 0.3)
                };
                let used = tilt_of(&camera);
                assert!(
                    (-FRAC_PI_2 - 1e-6..=FRAC_PI_2 + 1e-6).contains(&(pitch - used)),
                    "pitch {pitch} tilt {tilt}: the facing latitude is {}",
                    pitch - used
                );
                let at_the_limit = Camera {
                    globe_tilt: used,
                    ..camera.clone()
                };
                assert!(
                    picture(&camera) == picture(&at_the_limit),
                    "pitch {pitch} tilt {tilt} drew something other than the clamped lean"
                );
            }
        }
        // A tilt the old pitch allowed is held back when the pitch drops.
        let steep = Camera {
            pitch: FRAC_PI_2,
            ..leaning(2.0, 0.0)
        };
        let shallow = Camera {
            pitch: LOWEST_PITCH,
            ..steep.clone()
        };
        assert_eq!(tilt_of(&steep), 2.0);
        assert!((tilt_of(&shallow) - (LOWEST_PITCH + FRAC_PI_2)).abs() < 1e-6);
    }

    #[test]
    fn standing_the_earth_upright_undoes_the_lean_of_any_point_drawn() {
        let mut snapshot = crate::model::demo(10.0, 128);
        snapshot.home = Some(named("London", 51.5, -0.1));
        let (level, _) = drawn(&snapshot, &leaning(0.0, 0.6), 0.0);
        for (tilt, rotation) in [(0.5, 0.0), (-0.9, 1.1), (2.0, -2.0)] {
            let camera = Camera {
                rotation,
                ..leaning(tilt, 0.6)
            };
            let (leant, _) = drawn(&snapshot, &camera, 0.0);
            for (id, at) in &leant.positions {
                let back = upright(*at, &camera);
                let expected = level.positions[id];
                assert!(
                    (0..3).all(|k| (back[k] - expected[k]).abs() < 1e-3),
                    "{back:?} against {expected:?} at tilt {tilt}"
                );
            }
        }
        let flat = Camera::default();
        assert_eq!(upright([3.0, 4.0, 25.0], &flat), [3.0, 4.0, 25.0]);
    }

    /// A snapshot whose home is London and whose first process talks to each of `places`.
    fn connected_to(places: &[Place]) -> Snapshot {
        let mut snapshot = crate::model::demo(10.0, 128);
        snapshot.home = Some(named("London", 51.5, -0.1));
        let template = snapshot.remotes[0].clone();
        snapshot.remotes = places
            .iter()
            .enumerate()
            .map(|(k, place)| Remote {
                address: IpAddr::from([8, 8, 8, k as u8 + 1]),
                location: Location::Known(place.clone()),
                ..template.clone()
            })
            .collect();
        snapshot
    }

    /// A snapshot whose first process connects to each of these addresses.
    fn connected_with(addresses: &[(&str, Location)]) -> Snapshot {
        let mut snapshot = connected_to(&[]);
        let template = crate::model::demo(10.0, 128).remotes[0].clone();
        snapshot.remotes = addresses
            .iter()
            .map(|(address, location)| Remote {
                address: address.parse().unwrap(),
                location: location.clone(),
                ..template.clone()
            })
            .collect();
        snapshot
    }

    #[test]
    fn without_a_database_one_label_counts_the_unplaced_addresses() {
        let mut snapshot = connected_with(&[
            ("8.8.8.8", Location::NoDatabase),
            ("1.1.1.1", Location::NoDatabase),
            ("9.9.9.9", Location::NoDatabase),
        ]);
        let at = labelled(&snapshot, &Camera::default(), 0.0);
        let counting: Vec<&String> = at.keys().filter(|name| name.contains("GeoIP")).collect();
        assert_eq!(counting, ["no GeoIP database: 3 addresses (see --geoip)"]);
        assert!(at.keys().all(|name| !name.contains("8.8.8.8")), "{at:?}");
        snapshot.remotes.truncate(1);
        let at = labelled(&snapshot, &Camera::default(), 0.0);
        assert!(at.contains_key("no GeoIP database: 1 address (see --geoip)"));
    }

    #[test]
    fn addresses_still_being_located_share_one_label() {
        let snapshot = connected_with(&[
            ("8.8.8.8", Location::Pending),
            ("1.1.1.1", Location::Pending),
        ]);
        let at = labelled(&snapshot, &Camera::default(), 0.0);
        assert!(at.contains_key("locating 2 addresses"), "{at:?}");
        assert!(at.keys().all(|name| !name.contains("GeoIP")), "{at:?}");
    }

    #[test]
    fn unlisted_and_private_endpoints_are_labelled_with_their_address() {
        let snapshot = connected_with(&[
            ("8.8.8.8", Location::Unlisted),
            ("10.0.0.2", Location::Private),
        ]);
        let at = labelled(&snapshot, &Camera::default(), 0.0);
        assert!(at.contains_key("8.8.8.8"), "{at:?}");
        assert!(at.contains_key("10.0.0.2 (private)"), "{at:?}");
        assert!(at.keys().all(|name| !name.contains("GeoIP")), "{at:?}");
        use crate::render::{Scene, View};
        let mut scene = Scene::new();
        scene.render(
            &snapshot,
            View::Globe,
            &Camera::default(),
            640,
            360,
            None,
            0.0,
            512,
            None,
        );
        let notes = &scene.notes[&snapshot.remotes[0].id];
        assert!(notes[0].starts_with("8.8.8.8:"), "{notes:?}");
        assert!(notes[0].contains("not in the GeoIP database"), "{notes:?}");
        assert!(notes[1].contains("10.0.0.2:") && notes[1].contains(" private"));
    }

    #[test]
    fn endpoints_keep_their_places_whatever_the_reason_they_have_none() {
        let drawn = |location: Location| {
            let snapshot = connected_with(&[
                ("8.8.8.8", location.clone()),
                ("1.1.1.1", location.clone()),
                ("9.9.9.9", location),
            ]);
            let mut scene = crate::render::Scene::new();
            let frame = scene.render(
                &snapshot,
                crate::render::View::Globe,
                &Camera::default(),
                640,
                360,
                None,
                0.0,
                512,
                None,
            );
            format!("{:?}", frame.items)
        };
        let expected = drawn(Location::Pending);
        for location in [Location::NoDatabase, Location::Unlisted, Location::Private] {
            assert!(drawn(location.clone()) == expected, "{location:?}");
        }
    }

    #[test]
    fn east_is_to_the_right_of_west_on_the_facing_side() {
        let snapshot = connected_to(&[
            named("Berlin", 52.5, 13.4),
            named("Madrid", 40.4, -3.7),
            named("Reykjavik", 64.1, -21.9),
        ]);
        let at = labelled(&snapshot, &Camera::default(), 0.0);
        let (london, berlin) = (at["London"], at["Berlin"]);
        let (madrid, reykjavik) = (at["Madrid"], at["Reykjavik"]);
        assert!(berlin[0] > london[0], "Berlin {berlin:?} London {london:?}");
        assert!(reykjavik[0] < london[0], "Iceland is west of Britain");
        assert!(madrid[0] < berlin[0], "Spain is west of Germany");
        // Screen y grows downwards.
        assert!(madrid[1] > berlin[1], "Spain is south of Germany");
        assert!(reykjavik[1] < london[1], "Iceland is north of Britain");
    }

    #[test]
    fn the_earth_turns_eastward() {
        let snapshot = connected_to(&[named("Berlin", 52.5, 13.4)]);
        let camera = Camera::default();
        let mut last = labelled(&snapshot, &camera, 0.0);
        for time in [3.0, 6.0, 9.0] {
            let now = labelled(&snapshot, &camera, time);
            for name in ["London", "Berlin"] {
                assert!(now[name][0] > last[name][0], "{name} at {time} s");
            }
            last = now;
        }
    }

    #[test]
    fn tangents_point_east_where_longitude_grows_and_north_up_the_globe() {
        // Longitude grows as azimuth falls, so a step east is a step to a smaller azimuth.
        for (latitude, azimuth) in [(0.0f32, 0.7f32), (50.0, 2.0), (-35.0, -1.2)] {
            let at = |azimuth: f32| {
                let lat = latitude.to_radians();
                [
                    lat.cos() * azimuth.cos(),
                    lat.cos() * azimuth.sin(),
                    lat.sin(),
                ]
            };
            let (up, further) = (at(azimuth), at(azimuth - 0.01));
            let (east, north) = tangents(up, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
            let step = normalize([0, 1, 2].map(|k| further[k] - up[k]));
            assert!(dot(east, step) > 0.999, "{latitude}: {east:?} {step:?}");
            assert!(north[2] > 0.1, "{latitude}: north is up the globe");
        }
    }

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
            let (east, north) = tangents(up, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
            assert!((dot(east, east) - 1.0).abs() < 1e-5, "{up:?}");
            assert!((dot(north, north) - 1.0).abs() < 1e-5, "{up:?}");
            assert!(dot(east, north).abs() < 1e-5 && dot(east, up).abs() < 1e-5);
        }
    }
}
