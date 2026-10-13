use std::f32::consts::FRAC_PI_2;

use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::picker::Picker;
use crate::render::{Camera, ISOMETRIC};
use crate::views::{LOCAL_KEYS, View, ViewKey};

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
        let before = self.goal.clone();
        let quit = self.command(code, modifiers, step);
        self.follow(&before);
        quit
    }

    /// Moving the camera by hand stops it following the view; fitting it again resumes.
    pub(super) fn follow(&mut self, before: &Camera) {
        if self.fit {
            self.framed = true;
        } else if self.goal != *before {
            self.framed = false;
        }
    }

    pub(super) fn command(&mut self, code: KeyCode, modifiers: KeyModifiers, step: f32) -> bool {
        let touring = self.tour.as_ref().is_some_and(|tour| tour.running);
        self.touch();
        if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            return true;
        }
        if self.picker.is_some() {
            return self.picker_key(code, modifiers);
        }
        if self.search.is_open() {
            self.search_key(code);
            return false;
        }
        if code == KeyCode::Esc && self.thing.take().is_some() {
            return false;
        }
        if let Some(key) = view_key(code) {
            let consumed = self.scene.key(self.view, key);
            match key {
                ViewKey::Local(_) => return false,
                ViewKey::Escape if consumed => return false,
                ViewKey::Escape | ViewKey::Reset => {}
            }
        }
        match code {
            KeyCode::Char('q') => return true,
            KeyCode::Tab => self.switch(self.cycle(true)),
            KeyCode::BackTab => self.switch(self.cycle(false)),
            KeyCode::Left | KeyCode::Char('a') => self.goal.pan(-step, 0.0),
            KeyCode::Right | KeyCode::Char('d') => self.goal.pan(step, 0.0),
            KeyCode::Up | KeyCode::Char('w') => self.goal.pan(0.0, -step),
            KeyCode::Down | KeyCode::Char('s') => self.goal.pan(0.0, step),
            KeyCode::Char('+') | KeyCode::Char('=') => self.goal.scale(1.15),
            KeyCode::Char('-') => self.goal.scale(1.0 / 1.15),
            KeyCode::Char('e') => self.goal.rotation += FRAC_PI_2,
            KeyCode::Char('Q') => self.goal.rotation -= FRAC_PI_2,
            KeyCode::PageUp => self.goal.tilt(0.12),
            KeyCode::PageDown => self.goal.tilt(-0.12),
            KeyCode::Char('t') => {
                self.goal.pitch = if self.goal.pitch > 1.2 {
                    ISOMETRIC
                } else {
                    FRAC_PI_2
                };
            }
            KeyCode::Char('v') => {
                self.picker = Some(Picker::open(&self.entries(), self.view));
            }
            KeyCode::Char('l') => self.labels = !self.labels,
            KeyCode::Char('h') => self.panel = !self.panel,
            KeyCode::Char(digit @ '0'..='9') => {
                // 1 to 9 pick the first nine rows and 0 the tenth.
                let index = (digit as usize + 10 - '1' as usize) % 10;
                if self.available.get(index) == Some(&true) {
                    self.switch(View::at(index));
                }
            }
            KeyCode::Char('g') if !touring && self.row().tour => self.visit(Step::Stay, true),
            KeyCode::Char(' ') if self.row().tour => {
                let step = if modifiers.contains(KeyModifiers::SHIFT) {
                    Step::Random
                } else {
                    Step::Next
                };
                self.visit(step, touring);
            }
            // Shift+Space where the terminal cannot tell it from Space.
            KeyCode::Char('*') if self.row().tour => self.visit(Step::Random, touring),
            KeyCode::Char('c') => self.scene.links.cycle(),
            KeyCode::Home => self.fit = true,
            KeyCode::Char('r') => {
                // Release first: the camera must stop being held before it eases to the default
                // camera, or it would copy the goal's release and spin a second time.
                let time = self.render_time();
                self.camera.release(time);
                self.goal = Camera::default();
                if let Some(pitch) = self.row().pitch {
                    self.goal.pitch = pitch;
                }
                self.fit = true;
            }
            KeyCode::Char('?') => self.show_help = !self.show_help,
            KeyCode::Char('/') => self.search.open(),
            KeyCode::Char('n') => {
                if let Some(id) = self.search.next() {
                    self.select_match(id);
                }
            }
            KeyCode::Char('f') => {
                if self.selected.is_some() {
                    self.focus = self.selected;
                    self.fit = true;
                }
            }
            KeyCode::Esc => {
                if self.selected.take().is_none() {
                    self.focus = None;
                    self.fit = true;
                }
            }
            KeyCode::Char('P') => {
                self.history.toggle_pause();
            }
            KeyCode::Char('[') => self.history.rewind(),
            KeyCode::Char(']') => self.history.forward(),
            _ => {}
        }
        false
    }
}

/// What a key press offers the shown view's key hook, if anything.
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

pub(super) fn view_key(code: KeyCode) -> Option<ViewKey> {
    match code {
        KeyCode::Char(character) if LOCAL_KEYS.contains(character) => {
            Some(ViewKey::Local(character))
        }
        KeyCode::Home | KeyCode::Char('r') => Some(ViewKey::Reset),
        KeyCode::Esc => Some(ViewKey::Escape),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;

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
        let center = app.goal.center;
        press(&mut app, KeyCode::Char('a'));
        assert_ne!(app.goal.center, center, "a global key keeps its meaning");
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
        app.fit = false;
        press(&mut app, KeyCode::Char('r'));
        assert!(app.fit, "the global reset happens as well");
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
