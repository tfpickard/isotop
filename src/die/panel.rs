//! The sensor panel, to the right of the die in the same plane: the colour bar (quads), its tick
//! labels, a row of readout for the package, the power and each group of cores, and two sparklines
//! of the last minute, package temperature and package power. Text that is not a literal of the
//! view's own is a `Tag`, which the app cleans; the only text drawn into the frame is the bar's
//! unit, a literal.

use super::floorplan::Rect;
use super::paint::{Pen, plane};
use super::readings::{Mode, Point as SeriesPoint, Series, WINDOW};
use crate::hershey::{Align, Face};
use crate::palette::{ironbow, mix};
use crate::render::{Color, SCALE, Side, Tag};

/// The panel's rectangle on the plan.
pub const AREA: Rect = [204.0, 0.0, 256.0, 96.0];
const BAR: Rect = [206.0, 14.0, 210.5, 70.0];
const SWATCH_X: f32 = 221.0;
const TEXT_X: f32 = 224.5;
const FIRST_ROW: f32 = 14.0;
const ROW_PITCH: f32 = 6.0;
/// The most rows the panel holds: the package, the power, and the groups of cores.
pub const MOST_ROWS: usize = 10;
const PACKAGE_BOX: Rect = [221.0, 76.0, 252.0, 86.0];
const POWER_BOX: Rect = [221.0, 89.0, 252.0, 95.0];
const BAR_STEPS: usize = 28;

const BACKDROP: Color = [12, 10, 22];
const BORDER: Color = [70, 72, 100];
const TEXT: Color = [214, 218, 236];
const TICK: Color = [150, 156, 184];
const TJ_MAX: Color = [96, 224, 240];
const WATTS: Color = [150, 200, 240];
const MISSING: Color = [58, 58, 76];

/// One line of readout.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The swatch's level on the colour scale, or None for no colour (a reading that is not
    /// there is not drawn as a cold one).
    pub level: Option<f32>,
    pub text: String,
}

/// What the panel draws.
pub struct Input<'a> {
    pub mode: Mode,
    /// The scale's ends, in degrees or in model units.
    pub scale: (f32, f32),
    /// TjMax in degrees, drawn on the bar only in `Mode::Celsius`.
    pub tj_max: f32,
    pub rows: &'a [Row],
    pub series: &'a Series,
    /// What the sparklines say when they have nothing to plot.
    pub package_title: String,
    pub power_title: String,
}

/// Where plan y `y` is on the colour bar for `value`.
fn bar_y(value: f32, scale: (f32, f32)) -> f32 {
    let level = ((value - scale.0) / (scale.1 - scale.0)).clamp(0.0, 1.0);
    BAR[3] - level * (BAR[3] - BAR[1])
}

pub fn draw(pen: &mut Pen, tags: &mut Vec<Tag>, input: &Input) {
    let tag = |tags: &mut Vec<Tag>, x: f32, y: f32, text: String| {
        tags.push(Tag {
            at: plane(x, y, 0.0),
            side: Side::Right,
            text,
        });
    };
    pen.flat(AREA, -0.5, BACKDROP);
    pen.outline(AREA, -0.45, 1.2, BORDER);
    match input.mode {
        Mode::Celsius => tag(tags, 206.0, 6.0, "SENSORS".to_owned()),
        Mode::Relative => {
            // Written into the frame as well, so that a render says it too.
            caption(pen, "NO SENSOR", plane(230.0, 6.0, -0.3), 3.4, TEXT);
            tag(
                tags,
                206.0,
                9.8,
                "relative heat: the model, not degrees".to_owned(),
            );
        }
    }
    // The bar, hottest at the top.
    for step in 0..BAR_STEPS {
        let (top, bottom) = (
            BAR[1] + (BAR[3] - BAR[1]) * step as f32 / BAR_STEPS as f32,
            BAR[1] + (BAR[3] - BAR[1]) * (step + 1) as f32 / BAR_STEPS as f32,
        );
        let level = 1.0 - (step as f32 + 0.5) / BAR_STEPS as f32;
        pen.flat([BAR[0], top, BAR[2], bottom], -0.4, ironbow(level));
    }
    pen.outline(BAR, -0.35, 1.2, BORDER);
    match input.mode {
        Mode::Celsius => {
            let (low, high) = input.scale;
            let mut value = (low / 15.0).ceil() * 15.0;
            while value <= high {
                let y = bar_y(value, input.scale);
                pen.segment([BAR[2], y], [BAR[2] + 1.2, y], -0.3, 1.2, TICK);
                tag(tags, BAR[2] + 0.5, y, format!("{value:.0}"));
                value += 15.0;
            }
            let y = bar_y(input.tj_max, input.scale);
            pen.segment([BAR[0] - 1.5, y], [BAR[2] + 0.8, y], -0.25, 1.6, TJ_MAX);
            tag(
                tags,
                BAR[2] + 0.5,
                y + 2.6,
                format!("TjMax {:.0}", input.tj_max),
            );
        }
        Mode::Relative => {
            for (value, word) in [(input.scale.1, "hot"), (input.scale.0, "cold")] {
                let y = bar_y(value, input.scale);
                pen.segment([BAR[2], y], [BAR[2] + 1.2, y], -0.3, 1.2, TICK);
                tag(tags, BAR[2] + 0.5, y, word.to_owned());
            }
            tag(tags, BAR[2] + 0.5, 42.0, "model".to_owned());
        }
    }
    // The bar's unit, drawn into the frame so that a render says what the colours are.
    let unit = match input.mode {
        Mode::Celsius => "C",
        Mode::Relative => "REL",
    };
    caption(
        pen,
        unit,
        plane((BAR[0] + BAR[2]) / 2.0, BAR[3] + 4.0, -0.3),
        3.4,
        TEXT,
    );
    for (index, row) in input.rows.iter().take(MOST_ROWS).enumerate() {
        let y = FIRST_ROW + index as f32 * ROW_PITCH;
        let color = row.level.map_or(MISSING, ironbow);
        pen.flat([SWATCH_X, y - 1.2, SWATCH_X + 2.4, y + 1.2], -0.4, color);
        pen.outline(
            [SWATCH_X, y - 1.2, SWATCH_X + 2.4, y + 1.2],
            -0.35,
            1.0,
            BORDER,
        );
        tag(tags, TEXT_X, y, row.text.clone());
    }
    sparkline(pen, PACKAGE_BOX, input.series, Which::Package, input);
    tag(
        tags,
        PACKAGE_BOX[0],
        PACKAGE_BOX[1] - 2.2,
        input.package_title.clone(),
    );
    sparkline(pen, POWER_BOX, input.series, Which::Power, input);
    tag(
        tags,
        POWER_BOX[0],
        POWER_BOX[1] - 2.2,
        input.power_title.clone(),
    );
}

/// `text` written into the frame, centred on `at`, `height` cells high: Hershey strokes as
/// ribbons a little over a pixel wide. Only literals of the view's own are written this way.
fn caption(pen: &mut Pen, text: &str, at: [f32; 3], height: f32, color: Color) {
    let weight = 1.3 / (pen.camera.zoom * SCALE);
    pen.frame.stroke_text(
        pen.camera,
        Face::Triplex,
        text,
        at,
        [
            std::f32::consts::FRAC_1_SQRT_2,
            -std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        ],
        [
            -std::f32::consts::FRAC_1_SQRT_2,
            -std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        ],
        height * super::paint::CELL,
        color,
        weight,
        Align::Middle,
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Package,
    Power,
}

/// A box and, in it, the last minute of one series: x is the age within the window ending at the
/// newest sample, y the value on the series' own scale. A run with no reading in it is not
/// drawn across (a gap is a gap), and a series with no reading at all leaves a dashed midline.
fn sparkline(pen: &mut Pen, area: Rect, series: &Series, which: Which, input: &Input) {
    pen.flat(area, -0.4, [8, 6, 18]);
    pen.outline(area, -0.35, 1.0, BORDER);
    let Some(newest) = series.last().map(|point| point.elapsed) else {
        dashed(pen, area);
        return;
    };
    let value = |point: &SeriesPoint| match which {
        Which::Package => point.celsius,
        Which::Power => point.watts,
    };
    let readings: Vec<(f64, f32)> = series
        .points()
        .filter_map(|point| value(point).map(|found| (point.elapsed, found)))
        .collect();
    if readings.is_empty() {
        dashed(pen, area);
        return;
    }
    let (low, high) = match which {
        Which::Package => input.scale,
        Which::Power => (
            0.0,
            readings
                .iter()
                .map(|&(_, watts)| watts)
                .fold(10.0_f32, f32::max)
                * 1.1,
        ),
    };
    let (width, height) = (area[2] - area[0], area[3] - area[1]);
    let place = |elapsed: f64, found: f32| {
        let x = area[0] + 1.0 + (width - 2.0) * ((elapsed - (newest - WINDOW)) / WINDOW) as f32;
        let level = ((found - low) / (high - low)).clamp(0.0, 1.0);
        plane(x, area[3] - 1.0 - (height - 2.0) * level, -0.3)
    };
    if which == Which::Package && input.mode == Mode::Celsius {
        let y =
            area[3] - 1.0 - (height - 2.0) * ((input.tj_max - low) / (high - low)).clamp(0.0, 1.0);
        pen.segment(
            [area[0] + 1.0, y],
            [area[2] - 1.0, y],
            -0.32,
            1.0,
            mix(TJ_MAX, [0, 0, 0], 0.5),
        );
    }
    // The series holds one point per sample, so a sample with no reading breaks the line.
    let mut run: Vec<(f64, f32)> = Vec::new();
    let flush = |pen: &mut Pen, run: &mut Vec<(f64, f32)>| {
        for pair in run.windows(2) {
            let color = match which {
                Which::Power => WATTS,
                Which::Package => mix(
                    ironbow(((pair[1].1 - low) / (high - low)).clamp(0.0, 1.0)),
                    [210, 212, 230],
                    0.3,
                ),
            };
            pen.stroke(
                &[place(pair[0].0, pair[0].1), place(pair[1].0, pair[1].1)],
                1.8,
                color,
            );
        }
        run.clear();
    };
    for point in series.points() {
        match value(point) {
            Some(found) => run.push((point.elapsed, found)),
            None => flush(pen, &mut run),
        }
    }
    flush(pen, &mut run);
}

/// A dashed midline: nothing to plot.
fn dashed(pen: &mut Pen, area: Rect) {
    let y = (area[1] + area[3]) / 2.0;
    let mut x = area[0] + 1.5;
    while x + 1.5 < area[2] {
        pen.segment([x, y], [x + 1.5, y], -0.32, 1.0, MISSING);
        x += 3.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_puts_the_scales_ends_at_its_ends() {
        let scale = (30.0, 105.0);
        assert_eq!(bar_y(105.0, scale), BAR[1]);
        assert_eq!(bar_y(30.0, scale), BAR[3]);
        assert_eq!(bar_y(900.0, scale), BAR[1], "clamped");
        assert!((bar_y(67.5, scale) - (BAR[1] + BAR[3]) / 2.0).abs() < 1e-4);
    }
}
