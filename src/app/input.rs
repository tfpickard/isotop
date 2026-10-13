use std::f32::consts::FRAC_PI_2;

use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::keys::{self, Action, Context, LOCAL_KEYS};
use crate::picker::Picker;
use crate::render::ISOMETRIC;
use crate::views::{View, ViewKey};

use super::App;
use super::tour::Step;

impl App {
    pub(crate) fn key(&mut self, code: KeyCode, modifiers: KeyModifiers, step: f32) -> bool {
        // With every key reported as an escape code (see `terminal::KEYBOARD_FLAGS`), the press of
        // a modifier or a lock arrives as a key of its own, just before the key it modifies. It
        // is not a command, and must not end a tour or reset the idle timer.
        if matches!(
            code,
            KeyCode::Modifier(_) | KeyCode::CapsLock | KeyCode::NumLock | KeyCode::ScrollLock
        ) {
            return false;
        }
        let before = self.camera.goal.clone();
        let quit = self.command(code, modifiers, step);
        self.camera.follow(&before);
        quit
    }

    pub(super) fn command(&mut self, code: KeyCode, modifiers: KeyModifiers, step: f32) -> bool {
        let touring = self.tour.as_ref().is_some_and(|tour| tour.running);
        self.touch();
        let context = if self.picker.is_some() {
            Context::Picker
        } else if self.search.is_open() {
            Context::Search
        } else {
            Context::Global
        };
        let action = keys::lookup(context, code, modifiers);
        if action == Some(Action::Interrupt) {
            return true;
        }
        match context {
            Context::Picker => return self.picker_key(action, code, modifiers),
            Context::Search => {
                self.search_key(action, code);
                return false;
            }
            Context::Global | Context::Any => {}
        }
        if action == Some(Action::Escape) && self.thing.take().is_some() {
            return false;
        }
        if let Some(key) = view_key(code, action) {
            let consumed = self.scene.key(self.view, key);
            match key {
                ViewKey::Local(_) => return false,
                ViewKey::Escape if consumed => return false,
                _ => {}
            }
        }
        action.is_some_and(|action| self.perform(action, code, step, touring))
    }

    /// Carries out a global action; returns whether it quits. `touring` is whether a tour was
    /// running when the key arrived.
    fn perform(&mut self, action: Action, code: KeyCode, step: f32, touring: bool) -> bool {
        match action {
            Action::Quit => {
                // A view may take the quit to play a farewell first; none does by default. A
                // paused or rewound app stands still (`Stage::dt` is 0), so a farewell could
                // never finish: it quits at once.
                if self.history.is_paused() || !self.scene.key(self.view, ViewKey::Quit) {
                    return true;
                }
                self.quitting = true;
            }
            Action::NextView => self.switch(self.cycle(true)),
            Action::PreviousView => self.switch(self.cycle(false)),
            Action::PanLeft => self.camera.goal.pan(-step, 0.0),
            Action::PanRight => self.camera.goal.pan(step, 0.0),
            Action::PanUp => self.camera.goal.pan(0.0, -step),
            Action::PanDown => self.camera.goal.pan(0.0, step),
            Action::ZoomIn => self.camera.goal.scale(1.15),
            Action::ZoomOut => self.camera.goal.scale(1.0 / 1.15),
            Action::RotateClockwise => self.camera.goal.rotation += FRAC_PI_2,
            Action::RotateCounterclockwise => self.camera.goal.rotation -= FRAC_PI_2,
            Action::TiltUp => self.camera.goal.tilt(0.12),
            Action::TiltDown => self.camera.goal.tilt(-0.12),
            Action::TopDown => {
                self.camera.goal.pitch = if self.camera.goal.pitch > 1.2 {
                    ISOMETRIC
                } else {
                    FRAC_PI_2
                };
            }
            Action::Picker => {
                self.picker = Some(Picker::open(&self.entries(), self.view));
            }
            Action::Labels => self.labels = !self.labels,
            Action::Panel => self.panel = !self.panel,
            Action::Jump => {
                if let KeyCode::Char(digit) = code {
                    // 1 to 9 pick the first nine rows and 0 the tenth.
                    let index = (digit as usize + 10 - '1' as usize) % 10;
                    if self.available.get(index) == Some(&true) {
                        self.switch(View::at(index));
                    }
                }
            }
            Action::Tour if !touring && self.row().tour => self.visit(Step::Stay, true),
            Action::VisitNext if self.row().tour => self.visit(Step::Next, touring),
            Action::VisitRandom if self.row().tour => self.visit(Step::Random, touring),
            Action::Links => self.scene.links.cycle(),
            Action::Fit => self.camera.refit(),
            Action::Reset => {
                let (time, pitch) = (self.render_time(), self.row().pitch);
                self.camera.reset(time, pitch);
            }
            Action::Help => self.show_help = !self.show_help,
            Action::Search => self.search.open(),
            Action::NextMatch => {
                if let Some(id) = self.search.next() {
                    self.select_match(id);
                }
            }
            Action::Focus => {
                if self.selected.is_some() {
                    self.focus = self.selected;
                    self.camera.refit();
                }
            }
            Action::Escape => {
                if self.selected.take().is_none() {
                    self.focus = None;
                    self.camera.refit();
                }
            }
            Action::Pause => {
                self.history.toggle_pause();
            }
            Action::Rewind => self.history.rewind(),
            Action::Forward => self.history.forward(),
            // The tour and the visits when the view opts out of them, keys a view reads itself,
            // gestures the pointer handles, and the modal contexts' own actions do nothing here.
            _ => {}
        }
        false
    }
}

/// The key as legacy terminals would have typed it. Reporting every key as an escape code sends
/// a letter typed with Caps Lock on as the lowercase key with a lock state, where the legacy
/// encoding sent the capital.
pub(crate) fn typed(key: &event::KeyEvent) -> (KeyCode, KeyModifiers) {
    match key.code {
        KeyCode::Char(letter)
            if letter.is_ascii_lowercase()
                && key.state.contains(event::KeyEventState::CAPS_LOCK)
                && !key.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            (KeyCode::Char(letter.to_ascii_uppercase()), key.modifiers)
        }
        code => (code, key.modifiers),
    }
}

/// What a key press offers the shown view's key hook, if anything. `action` is what the keymap
/// makes of the press.
pub(super) fn view_key(code: KeyCode, action: Option<Action>) -> Option<ViewKey> {
    match (code, action) {
        (KeyCode::Char(character), _) if LOCAL_KEYS.contains(character) => {
            Some(ViewKey::Local(character))
        }
        (_, Some(Action::Fit | Action::Reset)) => Some(ViewKey::Reset),
        (_, Some(Action::Escape)) => Some(ViewKey::Escape),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;

    #[test]
    fn the_m_key_toggles_mode_only_while_hr_is_shown() {
        let hr = view("hr");
        let diagram = |app: &App| {
            app.scene
                .module::<crate::hr::Hr>(hr)
                .expect("the hr module")
                .in_diagram()
        };
        let mut app = App::new(view("city"), model::demo(12.0, 64), all_available());
        app.render(320, 180, 512);
        press(&mut app, KeyCode::Char('m'));
        app.switch(hr);
        app.render(320, 180, 512);
        assert!(
            !diagram(&app),
            "an `m` pressed in another view did not reach hr"
        );
        app.show_help = true;
        let help = app.text(1.0, "test", 10, 512)[3].clone();
        assert!(
            help.starts_with(" m diagram or sky | Arrows/WASD pan"),
            "{help}"
        );
        press(&mut app, KeyCode::Char('m'));
        app.render(320, 180, 512);
        assert!(diagram(&app), "`m` sends the stars to the diagram");
        assert!(
            app.scene.legend(hr, app.snapshot()).contains("m sky"),
            "and the legend says how to come back"
        );
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Char('m'));
        press(&mut app, KeyCode::Esc);
        app.render(320, 180, 512);
        assert!(diagram(&app), "a search takes the letter instead");
        press(&mut app, KeyCode::Char('m'));
        app.render(320, 180, 512);
        assert!(!diagram(&app), "`m` again returns to the sky");
        app.switch(view("city"));
        press(&mut app, KeyCode::Char('m'));
        app.switch(hr);
        app.render(320, 180, 512);
        assert!(!diagram(&app), "nor does it while another view is shown");
        // `--hr-diagram` starts in the diagram.
        let mut started = App::new(hr, model::demo(12.0, 64), all_available());
        started
            .scene
            .configure(&options(&["--hr-diagram"]).view_options());
        started.render(320, 180, 512);
        assert!(diagram(&started));
    }

    #[test]
    fn caps_lock_letters_arrive_as_capitals() {
        use event::{KeyEvent, KeyEventKind, KeyEventState};
        let key = |code, modifiers, state| {
            KeyEvent::new_with_kind_and_state(code, modifiers, KeyEventKind::Press, state)
        };
        let caps = KeyEventState::CAPS_LOCK;
        let plain = KeyModifiers::NONE;
        assert_eq!(
            typed(&key(KeyCode::Char('p'), plain, caps)).0,
            KeyCode::Char('P')
        );
        assert_eq!(
            typed(&key(KeyCode::Char('p'), plain, KeyEventState::NONE)).0,
            KeyCode::Char('p')
        );
        assert_eq!(
            typed(&key(KeyCode::Char('P'), KeyModifiers::SHIFT, caps)).0,
            KeyCode::Char('P')
        );
        assert_eq!(
            typed(&key(KeyCode::Char('1'), plain, caps)).0,
            KeyCode::Char('1')
        );
    }

    #[test]
    fn h_hides_the_panel_except_while_typing_a_search() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        let press = |app: &mut App, c: char| app.key(KeyCode::Char(c), KeyModifiers::NONE, 10.0);
        assert!(app.panel_shown());
        press(&mut app, 'h');
        assert!(!app.panel_shown());
        press(&mut app, '/');
        assert!(app.panel_shown(), "the search line stays visible");
        press(&mut app, 'h');
        assert_eq!(app.search.query(), Some("h"), "h is typed, not a toggle");
        app.key(KeyCode::Esc, KeyModifiers::NONE, 10.0);
        assert!(!app.panel_shown(), "hidden again after the search");
        press(&mut app, 'h');
        assert!(app.panel_shown());
    }

    #[test]
    fn the_rotation_keys_are_the_ones_the_help_names() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        app.show_help = true;
        let help = app.text(0.0, "test", 20, 512)[3].clone();
        assert!(help.contains("| Q/e rotate |"), "{help}");
        let turned = |app: &mut App, character: char| {
            let before = app.camera.goal.rotation;
            let quit = press(app, KeyCode::Char(character));
            (app.camera.goal.rotation - before, quit)
        };
        assert_eq!(turned(&mut app, 'e'), (FRAC_PI_2, false));
        assert_eq!(turned(&mut app, 'Q'), (-FRAC_PI_2, false));
        assert_eq!(turned(&mut app, 'E'), (0.0, false), "E is not bound");
        assert_eq!(turned(&mut app, 'q'), (0.0, true), "q quits");
    }

    #[test]
    fn a_modifier_the_keymap_does_not_name_changes_nothing() {
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        let center = app.camera.goal.center;
        app.key(KeyCode::Char('a'), KeyModifiers::ALT, 10.0);
        assert_ne!(app.camera.goal.center, center, "Alt-a pans as a does");
        let mut app = App::new(view("city"), model::demo(1.0, 16), all_available());
        assert!(
            app.key(KeyCode::Char('q'), KeyModifiers::CONTROL, 10.0),
            "Ctrl-q quits as q does"
        );
        assert!(app.key(KeyCode::Char('c'), KeyModifiers::CONTROL, 10.0));
    }

    #[test]
    fn a_view_can_defer_the_quit_until_its_farewell_is_over() {
        let (farewell, probe) = (View::at(0), View::at(1));
        let mut app = app_over(views::testing::FAREWELLS);
        assert!(
            !press(&mut app, KeyCode::Char('q')),
            "the view took the quit"
        );
        assert!(app.quitting && !app.farewell_over());
        for frame in 1..=testing::FAREWELL_FRAMES {
            assert!(!app.farewell_over(), "frame {frame} is not the last");
            app.render(320, 180, 512);
        }
        assert!(app.farewell_over(), "the view said it is done");
        assert_eq!(
            app.scene.legend(farewell, app.snapshot()),
            format!(" farewell: {} frames", testing::FAREWELL_FRAMES)
        );

        // A second q during the farewell ends it at once, and Ctrl-C never waits for it.
        let mut app = app_over(views::testing::FAREWELLS);
        press(&mut app, KeyCode::Char('q'));
        assert!(
            press(&mut app, KeyCode::Char('q')),
            "the view let the second q through"
        );
        let mut app = app_over(views::testing::FAREWELLS);
        assert!(app.key(KeyCode::Char('c'), KeyModifiers::CONTROL, 10.0));

        // A paused app has no time to play a farewell in, so it quits at once.
        let mut app = app_over(views::testing::FAREWELLS);
        press(&mut app, KeyCode::Char('P'));
        assert!(app.history.is_paused());
        assert!(
            press(&mut app, KeyCode::Char('q')),
            "no farewell while paused"
        );

        // Switching away cancels the deferred quit, and a view that defers nothing quits at once.
        let mut app = app_over(views::testing::FAREWELLS);
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.view, probe);
        for _ in 0..=testing::FAREWELL_FRAMES {
            app.render(320, 180, 512);
        }
        assert!(!app.farewell_over());
        assert!(
            press(&mut app, KeyCode::Char('q')),
            "the default is to quit"
        );

        // Search and the picker take q as text before any view sees it.
        let mut app = app_over(views::testing::FAREWELLS);
        press(&mut app, KeyCode::Char('/'));
        assert!(!press(&mut app, KeyCode::Char('q')));
        assert!(!app.quitting);
    }

    #[test]
    fn view_local_keys_reach_only_the_shown_view_and_never_during_search() {
        let mut app = probe_app();
        let (probe, quiet) = (View::at(0), View::at(1));
        let press = |app: &mut App, code: KeyCode| app.key(code, KeyModifiers::NONE, 10.0);
        let keys = |app: &App, view: View| {
            let legend = app.scene.legend(view, app.snapshot());
            let start = legend.find("keys [").expect("the probe lists its keys") + 6;
            legend[start..legend.len() - 1].to_owned()
        };
        for character in "x,z".chars() {
            press(&mut app, KeyCode::Char(character));
        }
        let center = app.camera.goal.center;
        press(&mut app, KeyCode::Char('a'));
        assert_ne!(
            app.camera.goal.center, center,
            "a global key keeps its meaning"
        );
        assert_eq!(
            (keys(&app, probe), keys(&app, quiet)),
            ("x,z".into(), "".into())
        );
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Char('y'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            (keys(&app, probe), keys(&app, quiet)),
            ("x,z".into(), "y".into())
        );
        for code in [KeyCode::Char('/'), KeyCode::Char('b'), KeyCode::Char('k')] {
            press(&mut app, code);
        }
        assert_eq!(app.search.query(), Some("bk"), "typed into the search");
        press(&mut app, KeyCode::Esc);
        assert_eq!(
            keys(&app, probe),
            "x,z",
            "nothing reached the view during the search"
        );
        app.camera.fit = false;
        press(&mut app, KeyCode::Char('r'));
        assert!(app.camera.fit_pending(), "the global reset happens as well");
        press(&mut app, KeyCode::Home);
        assert_eq!(keys(&app, probe), "x,zRR");
        // The probe's mode takes the first Esc; the next one closes the selection as usual.
        let selected = app.snapshot().processes[0].id;
        app.selected = Some(selected);
        press(&mut app, KeyCode::Char('m'));
        press(&mut app, KeyCode::Esc);
        assert_eq!(
            app.selected,
            Some(selected),
            "the view consumed the first Esc"
        );
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.selected, None);
        assert_eq!(keys(&app, probe), "x,zRRmEE");
    }
}
