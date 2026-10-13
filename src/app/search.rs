//! The `/` search: what is typed, which processes it finds and which of them is current.

use crossterm::event::KeyCode;

use crate::model::{Identity, Process};

use super::App;

/// The query being typed, the processes it matches in the shown sample and the match the user
/// has stepped to. The matches outlive the query line so `n` keeps stepping through them.
#[derive(Default)]
pub(crate) struct Search {
    /// What is typed; None while the search line is closed.
    query: Option<String>,
    matches: Vec<Identity>,
    index: usize,
}

impl Search {
    /// Whether the search line is open and takes the keys.
    pub(crate) fn is_open(&self) -> bool {
        self.query.is_some()
    }

    /// What is typed, while the line is open.
    pub(crate) fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// How many processes the query matches.
    pub(crate) fn found(&self) -> usize {
        self.matches.len()
    }

    /// Opens the line with nothing typed and forgets the last matches.
    pub(crate) fn open(&mut self) {
        self.query = Some(String::new());
        self.matches.clear();
    }

    /// Closes the line, keeping the matches for `next`.
    pub(crate) fn close(&mut self) {
        self.query = None;
    }

    /// Types `character` and finds the matches again among `processes`. Returns the first match,
    /// the one to select.
    pub(crate) fn type_character(
        &mut self,
        character: char,
        processes: &[Process],
    ) -> Option<Identity> {
        self.query.as_mut()?.push(character);
        self.find(processes)
    }

    /// Erases the last character typed and finds the matches again among `processes`.
    pub(crate) fn backspace(&mut self, processes: &[Process]) -> Option<Identity> {
        self.query.as_mut()?.pop();
        self.find(processes)
    }

    /// Steps to the next match, wrapping round; None when there is none.
    pub(crate) fn next(&mut self) -> Option<Identity> {
        if self.matches.is_empty() {
            return None;
        }
        self.index = (self.index + 1) % self.matches.len();
        self.matches.get(self.index).copied()
    }

    /// Matches the query against `processes` and returns the first match, unless the query is
    /// empty, which matches everything and selects nothing.
    fn find(&mut self, processes: &[Process]) -> Option<Identity> {
        let query = self.query.as_deref().unwrap_or("").to_lowercase();
        self.matches = matching(processes, &query);
        self.index = 0;
        if query.is_empty() {
            None
        } else {
            self.matches.first().copied()
        }
    }
}

/// The processes whose name or command contains `query` (lowercase), or whose pid is `query`.
fn matching(processes: &[Process], query: &str) -> Vec<Identity> {
    processes
        .iter()
        .filter(|process| {
            process.name.to_lowercase().contains(query)
                || process.command.to_lowercase().contains(query)
                || process.id.pid.to_string() == query
        })
        .map(|process| process.id)
        .collect()
}

impl App {
    /// Handles a key while the search line is open: it takes every key, so nothing reaches the
    /// scene or the shown view.
    pub(super) fn search_key(&mut self, code: KeyCode) {
        let processes = &self.history.snapshot().processes;
        let found = match code {
            KeyCode::Esc | KeyCode::Enter => {
                self.search.close();
                None
            }
            KeyCode::Backspace => self.search.backspace(processes),
            KeyCode::Tab => self.search.next(),
            KeyCode::Char(character) => self.search.type_character(character, processes),
            _ => None,
        };
        if let Some(id) = found {
            self.select_match(id);
        }
    }

    /// Selects the match `id` and takes the camera to it, or focuses its subtree when the view
    /// does not draw it.
    pub(super) fn select_match(&mut self, id: Identity) {
        self.selected = Some(id);
        self.thing = None;
        if let Some(&p) = self.scene.positions.get(&id) {
            self.goal.center = [p[0], p[1]];
            self.face(p);
        } else {
            self.focus = Some(id);
            self.fit = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fixtures::*;
    use crate::model::testing::process;

    fn named(pid: u32, name: &str, command: &str) -> Process {
        let mut process = process(pid, 1);
        process.name = name.into();
        process.command = command.into();
        process
    }

    fn found(processes: &[Process], query: &str) -> Vec<u32> {
        matching(processes, &query.to_lowercase())
            .iter()
            .map(|id| id.pid)
            .collect()
    }

    #[test]
    fn a_query_matches_names_commands_and_exact_pids_ignoring_case() {
        let processes = [
            named(10, "Firefox", "/usr/lib/firefox/firefox -new-window"),
            named(20, "sshd", "sshd: /usr/sbin/sshd -D"),
            named(30, "kworker", "kworker/0:1"),
        ];
        assert_eq!(found(&processes, "FIRE"), [10]);
        assert_eq!(found(&processes, "-d"), [20]);
        assert_eq!(found(&processes, "30"), [30]);
        assert!(found(&processes, "3").is_empty(), "a pid matches whole");
        assert_eq!(found(&processes, "w"), [10, 30], "name or command");
    }

    #[test]
    fn typing_selects_the_first_match_and_an_empty_query_selects_nothing() {
        let processes = [
            named(10, "alpha", ""),
            named(20, "alpine", ""),
            named(30, "beta", ""),
        ];
        let mut search = Search::default();
        assert!(!search.is_open());
        search.open();
        assert!(search.is_open());
        assert_eq!(
            search.type_character('a', &processes).map(|id| id.pid),
            Some(10)
        );
        assert_eq!(search.found(), 3);
        assert_eq!(
            search.type_character('l', &processes).map(|id| id.pid),
            Some(10)
        );
        assert_eq!(search.found(), 2);
        assert_eq!(search.type_character('z', &processes), None);
        assert_eq!(search.found(), 0);
        assert_eq!(search.backspace(&processes).map(|id| id.pid), Some(10));
        search.backspace(&processes);
        assert_eq!(
            search.backspace(&processes),
            None,
            "nothing typed selects nothing"
        );
        assert_eq!(search.found(), 3, "though everything matches");
    }

    #[test]
    fn next_steps_through_the_matches_and_wraps_even_after_the_line_closes() {
        let processes = [
            named(10, "alpha", ""),
            named(20, "alpine", ""),
            named(30, "beta", ""),
        ];
        let mut search = Search::default();
        assert_eq!(search.next(), None, "nothing to step through");
        search.open();
        search.type_character('a', &processes);
        search.close();
        assert!(!search.is_open());
        let order: Vec<u32> = (0..4)
            .filter_map(|_| search.next())
            .map(|id| id.pid)
            .collect();
        assert_eq!(order, [20, 30, 10, 20]);
        search.open();
        assert_eq!(search.found(), 0, "opening forgets the matches");
    }

    #[test]
    fn keys_are_ignored_while_the_line_is_closed() {
        let processes = [named(10, "alpha", "")];
        let mut search = Search::default();
        assert_eq!(search.type_character('a', &processes), None);
        assert_eq!(search.query(), None);
    }

    #[test]
    fn searching_keeps_the_hold_but_stands_the_earth_upright_on_the_match() {
        let mut app = globe_at(30.0);
        scroll(&mut app, MouseEventKind::ScrollDown, 6);
        scroll(&mut app, MouseEventKind::ScrollRight, 3);
        app.render(640, 360, 512);
        let target = *app.scene.positions.keys().next().unwrap();
        app.select_match(target);
        assert!(app.goal.globe_held, "the found process must not drift away");
        assert_eq!(app.goal.globe_tilt, 0.0);
        let goal = app.goal.clone();
        app.camera.approach(&goal, 1.0);
        app.camera = app.goal.clone();
        let frame = app.render(640, 360, 512);
        let at = frame.project(&app.camera, app.scene.positions[&target]);
        assert!((at[0] - 320.0).abs() < 1.5 && (at[1] - 360.0 * ORIGIN_Y).abs() < 1.5);
    }
}
