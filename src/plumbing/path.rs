//! Polylines in the world, for the pipework: lengths along them, the point at a distance, a piece
//! of one, a corner rounded for a hose, and the glass window of a pipe. Pure.

use crate::render::Point;

fn sub(a: Point, b: Point) -> Point {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: Point, b: Point) -> Point {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn scale(a: Point, factor: f32) -> Point {
    a.map(|value| value * factor)
}

/// The distance between two points.
pub fn distance(a: Point, b: Point) -> f32 {
    let d = sub(a, b);
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// The point `t` of the way from `a` to `b`.
pub fn lerp(a: Point, b: Point, t: f32) -> Point {
    add(a, scale(sub(b, a), t))
}

/// The length of `points` along its legs.
pub fn length(points: &[Point]) -> f32 {
    points
        .windows(2)
        .map(|pair| distance(pair[0], pair[1]))
        .sum()
}

/// The point at distance `at` along `points`, and the unit direction of the leg it is on. `at` is
/// clamped to the path. None for fewer than two points or a path of no length.
pub fn at(points: &[Point], at: f32) -> Option<(Point, Point)> {
    let mut passed = 0.0;
    let mut last = None;
    for pair in points.windows(2) {
        let size = distance(pair[0], pair[1]);
        if size < 1e-6 {
            continue;
        }
        let direction = scale(sub(pair[1], pair[0]), 1.0 / size);
        last = Some((pair[1], direction));
        if at <= passed + size {
            let t = ((at - passed) / size).clamp(0.0, 1.0);
            return Some((lerp(pair[0], pair[1], t), direction));
        }
        passed += size;
    }
    last
}

/// The part of `points` between distances `from` and `to` along it, with its end points on the
/// path exactly. Empty when `to <= from` or the path has no length.
pub fn between(points: &[Point], from: f32, to: f32) -> Vec<Point> {
    if to <= from || length(points) < 1e-6 {
        return Vec::new();
    }
    let total = length(points);
    let (from, to) = (from.clamp(0.0, total), to.clamp(0.0, total));
    let Some((start, _)) = at(points, from) else {
        return Vec::new();
    };
    let mut piece = vec![start];
    let mut passed = 0.0;
    for pair in points.windows(2) {
        passed += distance(pair[0], pair[1]);
        if passed > from + 1e-5 && passed < to - 1e-5 {
            piece.push(pair[1]);
        }
    }
    if let Some((end, _)) = at(points, to) {
        piece.push(end);
    }
    piece.dedup_by(|a, b| distance(*a, *b) < 1e-6);
    piece
}

/// The glass window of a run of legs `lengths`: centred in the longest leg, up to `longest`
/// units long and leaving `margin` at each end, as distances along the whole run. None when no
/// leg is long enough to hold a window of at least `shortest` units.
pub fn window(lengths: &[f32], longest: f32, margin: f32, shortest: f32) -> Option<(f32, f32)> {
    let (index, &leg) = lengths
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1).then(b.0.cmp(&a.0)))?;
    let room = leg - 2.0 * margin;
    if room < shortest {
        return None;
    }
    let size = room.min(longest);
    let start: f32 = lengths[..index].iter().sum::<f32>() + (leg - size) * 0.5;
    Some((start, start + size))
}

/// `points` with each interior corner cut by an arc of up to `radius` units, as `steps`
/// segments, so a run of axis-aligned legs reads as a hose that bends. The arc starts and ends
/// where it leaves the two legs, and never reaches past the middle of a leg.
pub fn rounded(points: &[Point], radius: f32, steps: usize) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for corner in 1..points.len() - 1 {
        let (before, here, after) = (points[corner - 1], points[corner], points[corner + 1]);
        let cut = radius
            .min(distance(before, here) * 0.5)
            .min(distance(here, after) * 0.5);
        if cut < 1e-4 {
            out.push(here);
            continue;
        }
        let start = lerp(here, before, cut / distance(here, before));
        let end = lerp(here, after, cut / distance(here, after));
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            // A quadratic Bezier with the corner as its control point.
            let a = lerp(start, here, t);
            let b = lerp(here, end, t);
            out.push(lerp(a, b, t));
        }
    }
    out.push(points[points.len() - 1]);
    out
}

/// The points of a flat ground path (x, y) at height `z`.
pub fn lifted(points: &[[f32; 2]], z: f32) -> Vec<Point> {
    points.iter().map(|p| [p[0], p[1], z]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: [Point; 3] = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 3.0, 0.0]];

    #[test]
    fn a_path_has_the_length_of_its_legs_and_a_point_at_each_distance() {
        assert_eq!(length(&L), 7.0);
        let (point, direction) = at(&L, 1.0).unwrap();
        assert_eq!((point, direction), ([1.0, 0.0, 0.0], [1.0, 0.0, 0.0]));
        let (point, direction) = at(&L, 5.5).unwrap();
        assert_eq!((point, direction), ([4.0, 1.5, 0.0], [0.0, 1.0, 0.0]));
        let (end, _) = at(&L, 99.0).unwrap();
        assert_eq!(end, [4.0, 3.0, 0.0], "past the end is the end");
        assert_eq!(at(&L[..1], 0.0), None);
    }

    #[test]
    fn a_piece_of_a_path_keeps_the_corners_inside_it() {
        assert_eq!(
            between(&L, 1.0, 5.0),
            vec![[1.0, 0.0, 0.0], [4.0, 0.0, 0.0], [4.0, 1.0, 0.0]]
        );
        assert_eq!(
            between(&L, 1.0, 3.0),
            vec![[1.0, 0.0, 0.0], [3.0, 0.0, 0.0]]
        );
        assert_eq!(
            between(&L, 4.0, 5.0),
            vec![[4.0, 0.0, 0.0], [4.0, 1.0, 0.0]]
        );
        assert!(between(&L, 3.0, 3.0).is_empty());
        assert_eq!(length(&between(&L, 0.0, 7.0)), 7.0);
        // The pieces round a window make up the whole.
        let whole = length(&between(&L, 0.0, 2.0))
            + length(&between(&L, 2.0, 5.0))
            + length(&between(&L, 5.0, 7.0));
        assert!((whole - 7.0).abs() < 1e-5);
    }

    #[test]
    fn the_window_sits_in_the_middle_of_the_longest_leg() {
        // Legs of 1, 6 and 2 units: the window is 2.2 long, centred in the second.
        let (from, to) = window(&[1.0, 6.0, 2.0], 2.2, 0.8, 0.6).unwrap();
        assert!((from - (1.0 + (6.0 - 2.2) / 2.0)).abs() < 1e-5);
        assert!((to - from - 2.2).abs() < 1e-5);
        // A short leg gives a short window, and none that cannot hold the least.
        let (from, to) = window(&[2.0], 2.2, 0.8, 0.3).unwrap();
        assert!((to - from - 0.4).abs() < 1e-5);
        assert_eq!(window(&[1.0, 1.2], 2.2, 0.8, 0.3), None);
        assert_eq!(window(&[], 2.2, 0.8, 0.3), None);
    }

    #[test]
    fn rounding_a_corner_keeps_the_ends_and_stays_inside_the_corner() {
        let smooth = rounded(&L, 1.0, 4);
        assert_eq!(smooth[0], L[0]);
        assert_eq!(smooth[smooth.len() - 1], L[2]);
        assert!(smooth.len() > 3);
        assert!(
            length(&smooth) < length(&L),
            "the arc is shorter than the two legs it replaces"
        );
        // No point reaches past the legs' box.
        for point in &smooth {
            assert!((0.0..=4.0).contains(&point[0]) && (0.0..=3.0).contains(&point[1]));
        }
        // A radius larger than half a leg is cut to it.
        let tight = rounded(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]], 5.0, 3);
        assert!(tight.iter().all(|p| p[0] >= 0.0 && p[1] <= 1.0));
        assert_eq!(rounded(&L[..2], 1.0, 4), L[..2].to_vec());
    }
}
