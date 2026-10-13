use std::f32::consts::{FRAC_PI_2, TAU};

use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::model::Identity;
use crate::render::{Frame, ORIGIN_Y, SCALE, Target, Thing};
use crate::terminal::Popup;

use super::App;
use super::screen::Layout;

/// Where a mouse report points: the frame position under it and the terminal cell.
#[derive(Clone, Copy)]
pub(crate) struct Spot {
    pub(crate) frame: [f32; 2],
    pub(crate) cell: (u16, u16),
}

/// What the mouse is doing and what it is over.
#[derive(Default)]
pub(crate) struct Pointer {
    /// Where the pointer is in the scene; None outside it.
    pub(crate) hover: Option<Spot>,
    /// The process under the pointer in the last frame, which the next frame highlights.
    pub(crate) hovered: Option<Identity>,
    /// The thing under the pointer in the last frame, which the view highlights while no thing
    /// is selected.
    pub(crate) hovered_thing: Option<Thing>,
    /// The button held and where the drag last stood in the frame.
    pub(crate) dragging: Option<(MouseButton, [f32; 2])>,
    /// A click on empty space closes the popup when the button is released, unless the press
    /// turned into a drag.
    pub(crate) dismiss: bool,
}

impl Pointer {
    /// Notes the process or thing under the pointer in `frame`, for the next frame to highlight.
    pub(crate) fn hover_over(&mut self, frame: &Frame, radius: f32) {
        let target = self
            .hover
            .and_then(|spot| frame.target_near(spot.frame[0], spot.frame[1], radius));
        (self.hovered, self.hovered_thing) = match target {
            Some(Target::Process(id)) => (Some(id), None),
            Some(Target::Thing(thing)) => (None, Some(thing)),
            None => (None, None),
        };
    }

    /// Lets go of the thing under the pointer, which belongs to a view that is no longer shown.
    pub(crate) fn forget_thing(&mut self) {
        self.hovered_thing = None;
    }
}

impl App {
    pub(crate) fn mouse(
        &mut self,
        mouse: MouseEvent,
        layout: &Layout,
        frame: &Frame,
        panels: &[Popup],
        pixels: bool,
    ) {
        if self.picker.is_some() {
            return;
        }
        let before = self.camera.goal.clone();
        self.pointer_event(mouse, layout, frame, panels, pixels);
        self.camera.follow(&before);
    }

    pub(super) fn pointer_event(
        &mut self,
        mouse: MouseEvent,
        layout: &Layout,
        frame: &Frame,
        panels: &[Popup],
        pixels: bool,
    ) {
        let pointer = layout.spot(mouse.column, mouse.row, pixels);
        let (column, row) = pointer.cell;
        let in_scene = row < layout.rows;
        self.pointer.hover = in_scene.then_some(pointer);
        let scroll = match mouse.kind {
            MouseEventKind::ScrollUp => Some((0.0, -1.0)),
            MouseEventKind::ScrollDown => Some((0.0, 1.0)),
            MouseEventKind::ScrollLeft => Some((-1.0, 0.0)),
            MouseEventKind::ScrollRight => Some((1.0, 0.0)),
            _ => None,
        };
        if let Some((x, y)) = scroll {
            // Terminals deliver touchpad two-finger scrolling as wheel events; pinch never arrives.
            if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                let offset = [
                    pointer.frame[0] - layout.width as f32 * 0.5,
                    pointer.frame[1] - layout.height as f32 * ORIGIN_Y,
                ];
                let factor = 1.1_f32.powf(-(x + y));
                self.camera
                    .both(|c| c.scale_at(factor, offset[0], offset[1]));
            } else if mouse.modifiers.contains(KeyModifiers::ALT) {
                self.camera.both(|c| c.rotation += 0.08 * (x + y));
            } else if let Some(turn) = self.row().turn {
                // The surface follows the fingers the way panning moves content: scrolling right
                // moves it left, which turns it westward, and scrolling down moves it up, which
                // brings the south up. The angle moves the surface under the pointer by `step`.
                let step = layout.height as f32 * 0.04;
                let angle = (step / (turn.radius * SCALE * self.camera.current.zoom)).min(0.25);
                let time = self.render_time();
                self.camera.both(|c| {
                    c.hold(time);
                    c.globe_turn = (c.globe_turn + x * angle).rem_euclid(TAU);
                    c.globe_tilt =
                        (c.globe_tilt + y * angle).clamp(c.pitch - FRAC_PI_2, c.pitch + FRAC_PI_2);
                });
            } else {
                let step = layout.height as f32 * 0.04;
                self.camera.both(|c| c.pan(x * step, y * step));
            }
            return;
        }
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left)
                if in_scene && !panels.iter().any(|p| p.contains(column, row)) =>
            {
                let picked = frame.target_near(
                    pointer.frame[0],
                    pointer.frame[1],
                    layout.pick_radius(pixels),
                );
                // Clicking empty space closes the popup, unless the press turns into a drag.
                self.pointer.dismiss = picked.is_none();
                match picked {
                    Some(Target::Process(id)) => (self.selected, self.thing) = (Some(id), None),
                    Some(Target::Thing(thing)) => (self.selected, self.thing) = (None, Some(thing)),
                    None => {}
                }
                self.pointer.dragging = Some((MouseButton::Left, pointer.frame));
            }
            MouseEventKind::Down(button) if in_scene && button != MouseButton::Left => {
                self.pointer.dragging = Some((button, pointer.frame));
            }
            MouseEventKind::Drag(button) => {
                if let Some((held, last)) = self.pointer.dragging
                    && held == button
                {
                    let dx = pointer.frame[0] - last[0];
                    let dy = pointer.frame[1] - last[1];
                    if button == MouseButton::Left {
                        self.camera.both(|c| c.pan(-dx, -dy));
                        self.pointer.dismiss = false;
                    } else {
                        self.camera.both(|c| {
                            c.rotation += dx * 0.01;
                            c.tilt(dy * 0.008);
                        });
                    }
                    self.pointer.dragging = Some((held, pointer.frame));
                }
            }
            MouseEventKind::Up(button) => {
                self.pointer.dragging = None;
                if button == MouseButton::Left && std::mem::take(&mut self.pointer.dismiss) {
                    self.selected = None;
                    self.thing = None;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;

    /// A 1600 by 896 scene of 8 by 16 pixel cells whose mouse reports arrive in pixels.
    const THING_LAYOUT: Layout = Layout {
        columns: 200,
        rows: 56,
        width: 1600,
        height: 896,
        cell: Some([8.0, 16.0]),
    };

    /// An app on the lantern view of `testing::THINGS`, and its first frame.
    fn lantern_app() -> (App, Frame) {
        let scene = Scene::with_rows(testing::THINGS);
        let mut app = App::with_scene(scene, View::at(0), model::demo(1.0, 16), vec![true; 2]);
        let frame = app.render(THING_LAYOUT.width, THING_LAYOUT.height, 512);
        (app, frame)
    }

    /// Where lantern `index` is drawn in `frame`.
    fn lantern_at(app: &App, frame: &Frame, index: usize) -> [f32; 2] {
        frame
            .locate(&app.camera.current, testing::Lanterns::position(index))
            .expect("the lantern is on screen")
    }

    /// A left click at frame pixel `at`, pressed and released in place.
    fn click(app: &mut App, frame: &Frame, at: [f32; 2]) {
        for kind in [
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::Up(MouseButton::Left),
        ] {
            let event = MouseEvent {
                kind,
                column: at[0] as u16,
                row: at[1] as u16,
                modifiers: KeyModifiers::NONE,
            };
            app.mouse(event, &THING_LAYOUT, frame, &[], true);
        }
    }

    /// The text of the open popup, if one is open.
    fn popup_lines(app: &App, frame: &mut Frame) -> Option<Vec<String>> {
        let radius = THING_LAYOUT.pick_radius(true);
        let (_, panels) = app.overlay(frame, &THING_LAYOUT, radius);
        panels.first().map(|panel| panel.lines.clone())
    }

    #[test]
    fn a_selected_thing_opens_its_card_and_esc_closes_it() {
        let (mut app, mut frame) = lantern_app();
        assert_eq!(frame.things.len(), 3);
        let hovered = lantern_at(&app, &frame, 1);
        app.pointer.hover = Some(Spot {
            frame: hovered,
            cell: THING_LAYOUT.cell_of(hovered),
        });
        let radius = THING_LAYOUT.pick_radius(true);
        let (labels, _) = app.overlay(&mut frame, &THING_LAYOUT, radius);
        let tags: Vec<String> = labels
            .into_iter()
            .filter(|label| label.tone == Tone::Tag)
            .map(|label| label.text)
            .collect();
        assert_eq!(
            tags,
            [format!(
                " {} - {} ",
                testing::lantern_title(101),
                testing::WICK
            )],
            "a hovered thing shows its title and its card's first line"
        );
        app.pointer.hover = None;
        let lantern = Thing {
            kind: testing::LANTERN,
            key: 100,
        };
        let spot = lantern_at(&app, &frame, 0);
        click(&mut app, &frame, spot);
        assert_eq!((app.thing, app.selected), (Some(lantern), None));
        assert_eq!(
            popup_lines(&app, &mut frame).expect("the card is open"),
            [
                testing::lantern_title(100).as_str(),
                testing::WICK,
                "Esc close"
            ]
        );
        app.render(THING_LAYOUT.width, THING_LAYOUT.height, 512);
        assert_eq!(
            app.scene.legend(app.view, app.snapshot()),
            " lanterns | highlighted 100",
            "the view highlights the selected thing"
        );
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.thing, None);
        assert_eq!(popup_lines(&app, &mut frame), None);
        app.render(THING_LAYOUT.width, THING_LAYOUT.height, 512);
        assert_eq!(app.scene.legend(app.view, app.snapshot()), " lanterns");
        let spot = lantern_at(&app, &frame, 0);
        click(&mut app, &frame, spot);
        let process = app.snapshot().processes[0].id;
        let at = app
            .screen(&frame, process)
            .expect("the process is on screen");
        click(&mut app, &frame, at);
        assert_eq!(
            (app.thing, app.selected),
            (None, Some(process)),
            "selecting a process lets the thing go"
        );
    }

    #[test]
    fn a_thing_that_vanishes_says_so() {
        let (mut app, mut frame) = lantern_app();
        let spot = lantern_at(&app, &frame, 2);
        click(&mut app, &frame, spot);
        assert_eq!(app.thing.map(|thing| thing.key), Some(102));
        press(&mut app, KeyCode::Char('x'));
        frame = app.render(THING_LAYOUT.width, THING_LAYOUT.height, 512);
        assert_eq!(frame.things.len(), 2, "the last lantern went out");
        assert_eq!(
            popup_lines(&app, &mut frame).expect("the popup stays open"),
            ["lantern no longer shown", "Esc close"]
        );
    }

    #[test]
    fn switching_views_clears_the_selected_thing() {
        let (mut app, frame) = lantern_app();
        let spot = lantern_at(&app, &frame, 0);
        click(&mut app, &frame, spot);
        assert!(app.thing.is_some());
        press(&mut app, KeyCode::Tab);
        assert_eq!((app.view, app.thing), (View::at(1), None));
        press(&mut app, KeyCode::Tab);
        assert_eq!((app.view, app.thing), (View::at(0), None));
    }

    #[test]
    fn thing_card_text_cannot_inject_terminal_controls() {
        let (mut app, mut frame) = lantern_app();
        let radius = THING_LAYOUT.pick_radius(true);
        let at = lantern_at(&app, &frame, 0);
        app.pointer.hover = Some(Spot {
            frame: at,
            cell: THING_LAYOUT.cell_of(at),
        });
        let (labels, _) = app.overlay(&mut frame, &THING_LAYOUT, radius);
        click(&mut app, &frame, at);
        app.pointer.hover = None;
        let (_, panels) = app.overlay(&mut frame, &THING_LAYOUT, radius);
        let tag = labels.iter().find(|label| label.tone == Tone::Tag);
        assert!(tag.expect("the hover tag").text.contains('\x1b'));
        assert!(panels[0].lines.iter().any(|line| line == testing::WICK));
        let mut output = Vec::new();
        for label in &labels {
            terminal::draw_label(&mut output, label, THING_LAYOUT.columns).unwrap();
        }
        for panel in &panels {
            terminal::draw_popup(&mut output, panel).unwrap();
        }
        let output = String::from_utf8(output).unwrap();
        assert!(
            output.contains("lantern 100?]0;flare?"),
            "the tag: {output:?}"
        );
        assert!(
            output.contains("wick ?[2J?lit"),
            "the card's line: {output:?}"
        );
        for control in ["\x1b]0;", "\x1b[2J", "\x07"] {
            assert!(
                !output.contains(control),
                "{control:?} reached the terminal"
            );
        }
    }

    #[test]
    fn the_mouse_is_ignored_while_the_picker_is_open() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        let frame = app.render(320, 180, 512);
        let layout = Layout {
            columns: 40,
            rows: 12,
            width: 320,
            height: 180,
            cell: None,
        };
        let scroll = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let center = app.camera.goal.center;
        press(&mut app, KeyCode::Char('v'));
        app.mouse(scroll, &layout, &frame, &[], false);
        assert_eq!(
            (app.camera.goal.center, app.pointer.hover.is_none()),
            (center, true)
        );
        press(&mut app, KeyCode::Esc);
        app.mouse(scroll, &layout, &frame, &[], false);
        assert_ne!(
            app.camera.goal.center, center,
            "the same event pans once it is closed"
        );
    }

    #[test]
    fn scrolling_turns_and_tilts_the_globe_the_way_panning_moves_the_view() {
        let step = SCENE.height as f32 * 0.04;
        for (kind, sideways) in [
            (MouseEventKind::ScrollRight, true),
            (MouseEventKind::ScrollLeft, true),
            (MouseEventKind::ScrollDown, false),
            (MouseEventKind::ScrollUp, false),
        ] {
            let mut app = globe_at(0.0);
            let centre = app.camera.goal.center;
            let before = home_on_screen(&mut app);
            let point = app.scene.places[0].0;
            let (dx, dy) = match kind {
                MouseEventKind::ScrollRight => (step, 0.0),
                MouseEventKind::ScrollLeft => (-step, 0.0),
                MouseEventKind::ScrollDown => (0.0, step),
                _ => (0.0, -step),
            };
            // What panning by the same scroll does to the same point is the way it must move.
            let by_pan = panned_by(&app.camera.current, point, dx, dy);
            let pan_moves = [by_pan[0] - before[0], by_pan[1] - before[1]];
            scroll(&mut app, kind, 1);
            let after = home_on_screen(&mut app);
            let moved = [after[0] - before[0], after[1] - before[1]];
            let (along, across) = if sideways { (0, 1) } else { (1, 0) };
            assert_eq!(moved[along].signum(), pan_moves[along].signum(), "{kind:?}");
            let ratio = moved[along] / pan_moves[along];
            assert!(
                (0.3..1.05).contains(&ratio),
                "{kind:?} moved {ratio} of a step"
            );
            assert!(
                moved[across].abs() < 0.35 * step,
                "{kind:?} drifted {moved:?}"
            );
            assert!(
                app.camera.goal.globe_held && app.camera.current.globe_held,
                "{kind:?} holds"
            );
            assert!(app.camera.goal.center == centre, "{kind:?} panned");
        }
    }

    #[test]
    fn scrolling_the_globe_moves_the_surface_by_one_step_whatever_the_zoom() {
        let step = SCENE.height as f32 * 0.04;
        for zoom in [1.0, 5.0, 30.0] {
            let mut app = globe_at(0.0);
            app.camera.both(|c| c.zoom = zoom);
            let before = home_on_screen(&mut app);
            scroll(&mut app, MouseEventKind::ScrollRight, 1);
            let after = home_on_screen(&mut app);
            let moved = (after[0] - before[0]).abs();
            // The surface at home is a little foreshortened and the angle is capped at 0.25 rad.
            let reach = (0.25 * globe::RADIUS * render::SCALE * zoom).min(step);
            assert!(
                moved > 0.5 * reach && moved <= reach * 1.02,
                "zoom {zoom}: {moved} px for {reach}"
            );
        }
    }

    #[test]
    fn the_tilt_stops_at_the_poles_for_the_camera_it_is_clamped_against() {
        let mut app = globe_at(0.0);
        scroll(&mut app, MouseEventKind::ScrollDown, 80);
        let pitch = app.camera.current.pitch;
        assert!((app.camera.current.globe_tilt - (pitch + FRAC_PI_2)).abs() < 1e-5);
        scroll(&mut app, MouseEventKind::ScrollUp, 160);
        assert!((app.camera.current.globe_tilt - (pitch - FRAC_PI_2)).abs() < 1e-5);
        assert_eq!(app.camera.current.globe_tilt, app.camera.goal.globe_tilt);
    }

    #[test]
    fn dragging_still_pans_the_globe() {
        let mut app = globe_at(0.0);
        let frame = app.render(SCENE.width, SCENE.height, 512);
        let mouse = |kind, column| MouseEvent {
            kind,
            column,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        let centre = app.camera.goal.center;
        for event in [
            mouse(MouseEventKind::Down(MouseButton::Left), 10),
            mouse(MouseEventKind::Drag(MouseButton::Left), 14),
            mouse(MouseEventKind::Up(MouseButton::Left), 14),
        ] {
            app.mouse(event, &SCENE, &frame, &[], false);
        }
        assert_ne!(app.camera.goal.center, centre, "dragging pans");
        assert_eq!(app.camera.goal.globe_turn, 0.0);
        assert_eq!(app.camera.goal.globe_tilt, 0.0);
        assert!(
            !app.camera.goal.globe_held,
            "dragging leaves the earth turning"
        );
    }

    #[test]
    fn other_views_still_pan_when_scrolled() {
        for view in ["city", "orbit", "reef"].map(view) {
            let mut app = App::new(view, model::demo(1.0, 64), all_available());
            let frame = app.render(SCENE.width, SCENE.height, 512);
            let mut expected = app.camera.current.clone();
            let step = SCENE.height as f32 * 0.04;
            expected.pan(step, -step);
            for kind in [MouseEventKind::ScrollRight, MouseEventKind::ScrollUp] {
                let event = MouseEvent {
                    kind,
                    column: 10,
                    row: 5,
                    modifiers: KeyModifiers::NONE,
                };
                app.mouse(event, &SCENE, &frame, &[], false);
            }
            for k in 0..2 {
                assert!(
                    (app.camera.goal.center[k] - expected.center[k]).abs() < 1e-3,
                    "{view:?}: {:?} against {:?}",
                    app.camera.goal.center,
                    expected.center
                );
            }
            assert!(!app.camera.goal.globe_held, "{view:?}");
        }
    }
}
