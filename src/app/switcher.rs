use crossterm::event::{KeyCode, KeyModifiers};

use crate::picker::{self, Entry, Toggle};
use crate::terminal::Popup;
use crate::views::View;

use super::App;
use super::screen::{Layout, place_panel};

impl App {
    /// The view Tab (or Shift+Tab, when `forward` is false) goes to: the shown view's neighbour
    /// in the playlist, or the shown view itself when the playlist is empty.
    pub(super) fn cycle(&self, forward: bool) -> View {
        picker::cycle(&self.playlist, self.view, forward).unwrap_or(self.view)
    }

    /// The views the picker lists: every available one, in registry order.
    pub(super) fn entries(&self) -> Vec<Entry> {
        View::all()
            .filter(|view| self.available.get(view.index()) == Some(&true))
            .map(|view| {
                let row = self.scene.row(view);
                Entry {
                    view,
                    name: row.name,
                    title: row.title,
                }
            })
            .collect()
    }

    /// Handles a key while the picker is open: it takes every key, so nothing reaches the scene
    /// or the shown view. Returns whether to quit.
    pub(super) fn picker_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        let entries = self.entries();
        let Some(picker) = self.picker.as_mut() else {
            return false;
        };
        match code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Enter => {
                if let Some(view) = picker.highlighted(&entries) {
                    self.picker = None;
                    self.switch(view);
                }
            }
            KeyCode::Up => picker.step(&entries, false),
            KeyCode::Down => picker.step(&entries, true),
            KeyCode::Backspace => picker.backspace(&entries),
            KeyCode::Char(' ') => match picker.highlighted(&entries) {
                Some(view) => {
                    if picker::toggle(&mut self.playlist, view) == Toggle::Refused {
                        picker.refuse();
                    } else {
                        picker.quiet();
                    }
                }
                None => picker.quiet(),
            },
            KeyCode::Char(character)
                if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER) =>
            {
                picker.type_character(&entries, character);
            }
            _ => picker.quiet(),
        }
        false
    }

    /// The picker as a popup in the scene's top right corner, listing as many rows as fit.
    pub(super) fn picker_popup(&self, layout: &Layout) -> Option<Popup> {
        let picker = self.picker.as_ref()?;
        // The popup also holds a border above and below, the filter line and the key line.
        let capacity = usize::from(layout.rows).saturating_sub(4).max(1);
        let lines = picker.lines(&self.entries(), &self.playlist, capacity);
        place_panel(lines, None, layout, false, 40)
    }

    /// Shows `view`, framed, at its row's pitch if it has one. Things belong to the view that
    /// drew them, so a selected or hovered one is let go.
    pub(crate) fn switch(&mut self, view: View) {
        self.view = view;
        self.fit = true;
        self.thing = None;
        self.hovered_thing = None;
        if let Some(pitch) = self.row().pitch {
            self.goal.pitch = pitch;
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::app::fixtures::*;

    #[test]
    fn shift_tab_steps_back_through_the_views_and_wraps() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.key(KeyCode::BackTab, KeyModifiers::SHIFT, 10.0);
        assert_eq!(app.view, View::all().last().unwrap());
        app.key(KeyCode::Tab, KeyModifiers::NONE, 10.0);
        assert_eq!(app.view, view("city"));
    }

    #[test]
    fn existing_views_keep_their_names_titles_tab_order_and_digits() {
        let existing = [
            ("city", "CITY"),
            ("orbit", "ORBIT"),
            ("ripple", "RIPPLE"),
            ("flow", "FLOW"),
            ("cores", "CORES"),
            ("cells", "CELLS"),
            ("strata", "STRATA"),
            ("globe", "GLOBE"),
            ("reef", "REEF"),
            ("matrix", "MATRIX"),
            ("coop", "COOP"),
        ];
        let rows: Vec<(&str, &str)> = REGISTRY.iter().map(|row| (row.name, row.title)).collect();
        assert_eq!(rows[..existing.len()], existing);
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        let press = |app: &mut App, code: KeyCode| app.key(code, KeyModifiers::NONE, 10.0);
        for (name, title) in existing {
            assert_eq!(app.view.name(), name);
            assert!(
                app.text(0.0, "test", 20, 512)[0].starts_with(&format!(" ISOTOP / {title} / "))
            );
            press(&mut app, KeyCode::Tab);
        }
        for (digit, (name, _)) in "1234567890".chars().zip(existing) {
            press(&mut app, KeyCode::Char(digit));
            assert_eq!(app.view.name(), name, "digit {digit}");
        }
    }

    #[test]
    fn tab_skips_views_that_are_unavailable() {
        let shown = ["city", "flow", "matrix"];
        let available = View::all()
            .map(|view| shown.contains(&view.name()))
            .collect();
        let mut app = App::new(view("city"), model::demo(1.0, 16), available);
        let press = |app: &mut App, code: KeyCode| {
            app.key(code, KeyModifiers::NONE, 10.0);
            app.view.name()
        };
        assert_eq!(press(&mut app, KeyCode::Tab), "flow");
        assert_eq!(press(&mut app, KeyCode::Tab), "matrix");
        assert_eq!(press(&mut app, KeyCode::Tab), "city", "wraps round");
        assert_eq!(press(&mut app, KeyCode::BackTab), "matrix");
        assert_eq!(press(&mut app, KeyCode::BackTab), "flow");
        let mut alone = App::new(
            view("reef"),
            model::demo(1.0, 16),
            vec![false; REGISTRY.len()],
        );
        alone.available[view("reef").index()] = true;
        assert_eq!(
            press(&mut alone, KeyCode::Tab),
            "reef",
            "nowhere else to go"
        );
    }

    #[test]
    fn a_digit_bound_to_an_unavailable_row_does_nothing() {
        let mut available = all_available();
        available[view("orbit").index()] = false;
        let mut app = App::new(view("cells"), model::demo(1.0, 16), available);
        app.render(320, 180, 512);
        assert!(!app.fit);
        app.key(KeyCode::Char('2'), KeyModifiers::NONE, 10.0);
        assert_eq!((app.view, app.fit), (view("cells"), false));
        app.key(KeyCode::Char('3'), KeyModifiers::NONE, 10.0);
        assert_eq!((app.view, app.fit), (view("ripple"), true));
    }

    /// Opens the picker, types `name` and presses Enter, as the smoke test does.
    fn pick(app: &mut App, name: &str) {
        press(app, KeyCode::Char('v'));
        type_text(app, name);
        press(app, KeyCode::Enter);
    }

    /// The picker's popup lines in a layout of `rows` scene rows.
    fn picker_lines(app: &App, rows: u16) -> Vec<String> {
        let layout = Layout {
            columns: 120,
            rows,
            width: 960,
            height: rows as u32 * 16,
            cell: None,
        };
        app.picker_popup(&layout).expect("the picker fits").lines
    }

    #[test]
    fn the_picker_filters_by_name_and_title_ignoring_case() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        press(&mut app, KeyCode::Char('v'));
        let lines = picker_lines(&app, 31);
        assert_eq!(lines.len(), REGISTRY.len() + 2, "{lines:?}");
        assert!(lines[1].starts_with("> "), "the highlight starts on city");
        type_text(&mut app, "RE");
        let entries = app.entries();
        let matching = |text: &str| {
            let names: Vec<_> = picker::filter(&entries, text)
                .iter()
                .map(|entry| entry.name)
                .collect();
            names
        };
        assert_eq!(matching("RE"), ["cores", "reef"]);
        assert_eq!(matching("CoRe"), ["cores"]);
        assert_eq!(matching("matrix"), ["matrix"]);
        let lines = picker_lines(&app, 31);
        assert!(lines[0].starts_with("View: RE_"), "{lines:?}");
        assert!(lines[1].contains("cores") && lines[1].contains("CORES"));
        assert!(lines[2].contains("reef") && lines[2].contains("REEF"));
        assert!(
            lines[2].contains(" 9 "),
            "reef is the ninth view: {lines:?}"
        );
        press(&mut app, KeyCode::Backspace);
        type_text(&mut app, "eef");
        assert_eq!(
            picker_lines(&app, 31)[1].trim_end(),
            "> *  9 reef        REEF"
        );
        assert_eq!(app.view, view("city"), "typing never switches");
    }

    #[test]
    fn an_empty_filter_shows_no_view_matches_and_enter_does_nothing() {
        let mut app = App::new(view("orbit"), model::demo(1.0, 16), all_available());
        app.render(320, 180, 512);
        press(&mut app, KeyCode::Char('v'));
        type_text(&mut app, "zzz");
        let lines = picker_lines(&app, 31);
        assert_eq!(lines[1], "no view matches", "{lines:?}");
        assert_eq!(lines.len(), 3);
        assert!(!app.fit);
        press(&mut app, KeyCode::Enter);
        assert!(app.picker.is_some(), "Enter keeps the picker open");
        assert_eq!((app.view, app.fit), (view("orbit"), false));
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(
            playlist_names(&app).len(),
            REGISTRY.len(),
            "nothing toggled"
        );
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        press(&mut app, KeyCode::Backspace);
        assert_eq!(picker_lines(&app, 31).len(), REGISTRY.len() + 2);
    }

    #[test]
    fn tab_cycles_the_playlist_and_resumes_after_a_picked_view() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.playlist =
            picker::playlist(&[view("city"), view("flow"), view("reef")], &app.available);
        for expected in ["flow", "reef", "city", "flow"] {
            press(&mut app, KeyCode::Tab);
            assert_eq!(app.view.name(), expected);
        }
        pick(&mut app, "orbit");
        assert_eq!(app.view, view("orbit"), "not in the playlist");
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.view.name(),
            "flow",
            "the first playlist view after orbit"
        );
        pick(&mut app, "coop");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.view.name(), "city", "wraps round past the last row");
        pick(&mut app, "ripple");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.view.name(), "flow");
    }

    #[test]
    fn shift_tab_mirrors_tab_around_a_view_outside_the_playlist() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.playlist = picker::playlist(&[view("flow"), view("reef")], &app.available);
        pick(&mut app, "orbit");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(
            app.view.name(),
            "reef",
            "wraps round to the last playlist view"
        );
        pick(&mut app, "ripple");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(
            app.view.name(),
            "reef",
            "nothing before ripple, so the last"
        );
        pick(&mut app, "matrix");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.view.name(), "reef", "the nearest before matrix");
        pick(&mut app, "coop");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.view.name(), "reef");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.view.name(), "flow", "inside the playlist it steps back");
        pick(&mut app, "cells");
        press(&mut app, KeyCode::BackTab);
        assert_eq!(app.view.name(), "flow", "the nearest before cells");
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.view.name(), "reef");
    }

    #[test]
    fn the_playlist_cannot_be_emptied() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.playlist = picker::playlist(&[view("city"), view("orbit")], &app.available);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(playlist_names(&app), ["orbit"], "city leaves");
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(playlist_names(&app), ["orbit"], "the last view stays");
        let lines = picker_lines(&app, 31);
        assert!(
            lines.last().unwrap().contains("keeps at least one view"),
            "{lines:?}"
        );
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(playlist_names(&app), ["orbit", "ripple"]);
        assert!(!picker_lines(&app, 31).last().unwrap().contains("keeps"));
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Up);
        press(&mut app, KeyCode::Char(' '));
        assert_eq!(
            playlist_names(&app),
            ["city", "orbit", "ripple"],
            "back in registry order"
        );
        press(&mut app, KeyCode::Esc);
        assert!(app.picker.is_none());
        assert_eq!(app.view, view("city"), "Esc leaves the view as it was");
    }

    #[test]
    fn digits_ignore_the_playlist() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.playlist = picker::playlist(&[view("reef")], &app.available);
        for (digit, name) in [
            ('2', "orbit"),
            ('7', "strata"),
            ('0', "matrix"),
            ('1', "city"),
        ] {
            press(&mut app, KeyCode::Char(digit));
            assert_eq!(app.view.name(), name, "digit {digit}");
        }
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.view.name(), "reef");
        assert_eq!(
            playlist_names(&app),
            ["reef"],
            "digits leave the playlist alone"
        );
    }

    #[test]
    fn keys_typed_into_the_picker_do_not_reach_the_scene() {
        let mut app = probe_app();
        let legend = |app: &App| app.scene.legend(View::at(0), app.snapshot());
        let before = (
            legend(&app),
            app.goal.clone(),
            (app.labels, app.panel, app.history.is_paused()),
        );
        press(&mut app, KeyCode::Char('v'));
        assert!(app.picker.is_some());
        for code in [
            KeyCode::Char('q'),
            KeyCode::Char('x'),
            KeyCode::Char('a'),
            KeyCode::Char('m'),
        ] {
            assert!(!press(&mut app, code), "{code:?} must not quit");
        }
        for code in [
            KeyCode::Char('1'),
            KeyCode::Char('/'),
            KeyCode::Char('+'),
            KeyCode::Char('l'),
            KeyCode::Char('h'),
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Home,
            KeyCode::PageUp,
            KeyCode::Left,
        ] {
            assert!(!press(&mut app, code));
        }
        assert_eq!(app.view, View::at(0));
        assert!(!app.search.is_open());
        let lines = picker_lines(&app, 31);
        assert!(lines[0].starts_with("View: qxam1/+lh_"), "{lines:?}");
        assert_eq!(legend(&app), before.0, "the view saw no key");
        assert!(app.goal == before.1, "the camera did not move");
        assert_eq!((app.labels, app.panel, app.history.is_paused()), before.2);
        assert!(
            app.key(KeyCode::Char('c'), KeyModifiers::CONTROL, 10.0),
            "Ctrl-C still quits"
        );
        press(&mut app, KeyCode::Esc);
        assert!(app.picker.is_none());
        assert!(!press(&mut app, KeyCode::Char('x')));
        assert_ne!(
            legend(&app),
            before.0,
            "closed, the key reaches the view again"
        );
        let mut plain = App::new(view("city"), model::demo(1.0, 16), all_available());
        press(&mut plain, KeyCode::Char('v'));
        assert!(
            !press(&mut plain, KeyCode::Char('q')),
            "q filters and does not quit"
        );
        assert_eq!(
            picker_lines(&plain, 31)[0],
            format!("View: q_  0/{}", REGISTRY.len())
        );
    }

    #[test]
    fn enter_in_the_picker_switches_through_the_usual_path() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.render(320, 180, 512);
        assert!(app.demand().is_empty());
        pick(&mut app, "matrix");
        assert_eq!(app.view, view("matrix"));
        assert!(app.picker.is_none() && app.fit);
        assert_eq!(app.demand(), Demand::from(view("matrix").info().sources));
        assert!(app.demand().contains(Source::Journal));
        pick(&mut app, "ripple");
        assert_eq!(app.goal.pitch, app.row().pitch.unwrap_or(app.goal.pitch));
        assert!(app.demand().is_empty());
    }

    #[test]
    fn the_picker_scrolls_to_keep_the_highlight_visible() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        press(&mut app, KeyCode::Char('v'));
        let last = REGISTRY.last().expect("the registry has rows").name;
        for _ in 0..REGISTRY.len() - 1 {
            press(&mut app, KeyCode::Down);
        }
        let lines = picker_lines(&app, 8);
        assert_eq!(lines.len(), 6, "filter, four rows and the keys: {lines:?}");
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("> ") && line.contains(last))
        );
        press(&mut app, KeyCode::Down);
        let lines = picker_lines(&app, 8);
        assert!(
            lines[1].starts_with("> ")
                || lines
                    .iter()
                    .any(|line| line.starts_with("> ") && line.contains("city")),
            "{lines:?}"
        );
        press(&mut app, KeyCode::Up);
        assert!(
            picker_lines(&app, 8)
                .iter()
                .any(|line| line.starts_with("> ") && line.contains(last))
        );
    }
}
