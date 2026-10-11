//! The view picker opened by `v` and the Tab playlist. The state and the rules are pure, so tests
//! drive them without a terminal; `App` owns one `Picker` while it is open, draws its lines in a
//! popup and sends it every key.

use crate::views::View;

/// One available view as the picker lists it: the view and its two names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub view: View,
    pub name: &'static str,
    pub title: &'static str,
}

/// Whether a view called `name` and titled `title` matches `query`: the query is a substring of
/// either, ignoring case. The empty query matches everything.
pub fn matches(name: &str, title: &str, query: &str) -> bool {
    let query = query.to_lowercase();
    name.to_lowercase().contains(&query) || title.to_lowercase().contains(&query)
}

/// The entries that match `query`, in the order given.
pub fn filter<'a>(entries: &'a [Entry], query: &str) -> Vec<&'a Entry> {
    entries
        .iter()
        .filter(|entry| matches(entry.name, entry.title, query))
        .collect()
}

/// The Tab playlist for `--views`: the requested views that are available, each once at its first
/// occurrence. No request means every available view, in registry order.
pub fn playlist(requested: &[View], available: &[bool]) -> Vec<View> {
    let supported = |view: View| available.get(view.index()) == Some(&true);
    if requested.is_empty() {
        return (0..available.len())
            .map(View::at)
            .filter(|&view| supported(view))
            .collect();
    }
    let mut list: Vec<View> = Vec::new();
    for &view in requested {
        if supported(view) && !list.contains(&view) {
            list.push(view);
        }
    }
    list
}

/// The view Tab (`forward`) or Shift+Tab goes to from `current`, or None for an empty playlist.
/// A playlist member goes to its neighbour in playlist order. Any other view (picked with `v` or
/// given by `--view`) goes to the playlist view nearest after it in registry order, wrapping
/// round, and backwards to the nearest before it.
pub fn cycle(playlist: &[View], current: View, forward: bool) -> Option<View> {
    let count = playlist.len();
    if let Some(position) = playlist.iter().position(|&view| view == current) {
        let next = if forward {
            (position + 1) % count
        } else {
            (position + count - 1) % count
        };
        return Some(playlist[next]);
    }
    let by_index = |view: &&View| view.index();
    if forward {
        let after = playlist
            .iter()
            .filter(|view| view.index() > current.index());
        after
            .min_by_key(by_index)
            .or_else(|| playlist.iter().min_by_key(by_index))
            .copied()
    } else {
        let before = playlist
            .iter()
            .filter(|view| view.index() < current.index());
        before
            .max_by_key(by_index)
            .or_else(|| playlist.iter().max_by_key(by_index))
            .copied()
    }
}

/// What toggling a view's playlist membership did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Added,
    Removed,
    /// The view is the playlist's last, which is never removed.
    Refused,
}

/// Adds `view` to `playlist`, or removes it when it is a member. An added view goes before the
/// first member of higher registry index, so a registry-ordered playlist stays in registry order.
pub fn toggle(playlist: &mut Vec<View>, view: View) -> Toggle {
    if let Some(position) = playlist.iter().position(|&member| member == view) {
        if playlist.len() == 1 {
            return Toggle::Refused;
        }
        playlist.remove(position);
        return Toggle::Removed;
    }
    let position = playlist
        .iter()
        .position(|member| member.index() > view.index())
        .unwrap_or(playlist.len());
    playlist.insert(position, view);
    Toggle::Added
}

/// The first row of a window of `capacity` rows over `count`, chosen so `cursor` is inside it and
/// as near the middle as the ends allow.
pub fn window_start(count: usize, cursor: usize, capacity: usize) -> usize {
    if count <= capacity {
        return 0;
    }
    cursor.saturating_sub(capacity / 2).min(count - capacity)
}

/// The open picker: the typed filter and the highlighted row among the matches.
#[derive(Debug, PartialEq, Eq)]
pub struct Picker {
    filter: String,
    cursor: usize,
    /// Why the last key did nothing, shown until the next key.
    note: Option<&'static str>,
}

impl Picker {
    /// A picker with the highlight on `current`, or on the first row when it is not listed.
    pub fn open(entries: &[Entry], current: View) -> Self {
        Self {
            filter: String::new(),
            cursor: entries
                .iter()
                .position(|entry| entry.view == current)
                .unwrap_or(0),
            note: None,
        }
    }

    /// The highlighted view, or None when nothing matches.
    pub fn highlighted(&self, entries: &[Entry]) -> Option<View> {
        filter(entries, &self.filter)
            .get(self.cursor)
            .map(|entry| entry.view)
    }

    /// Adds `character` to the filter. The highlight stays on its view while that still matches.
    pub fn type_character(&mut self, entries: &[Entry], character: char) {
        self.note = None;
        self.refilter(entries, |filter| filter.push(character));
    }

    /// Removes the last filter character.
    pub fn backspace(&mut self, entries: &[Entry]) {
        self.note = None;
        self.refilter(entries, |filter| {
            filter.pop();
        });
    }

    /// Moves the highlight one match down (`forward`) or up, wrapping round.
    pub fn step(&mut self, entries: &[Entry], forward: bool) {
        self.note = None;
        let count = filter(entries, &self.filter).len();
        if count > 0 {
            self.cursor = if forward {
                (self.cursor + 1) % count
            } else {
                (self.cursor + count - 1) % count
            };
        }
    }

    /// Records that the highlighted view could not be removed from the playlist.
    pub fn refuse(&mut self) {
        self.note = Some("the playlist keeps at least one view");
    }

    /// Clears the note before a key that does not change the rows.
    pub fn quiet(&mut self) {
        self.note = None;
    }

    fn refilter(&mut self, entries: &[Entry], edit: impl FnOnce(&mut String)) {
        let kept = self.highlighted(entries);
        edit(&mut self.filter);
        self.cursor = kept
            .and_then(|view| {
                filter(entries, &self.filter)
                    .iter()
                    .position(|entry| entry.view == view)
            })
            .unwrap_or(0);
    }

    /// The popup's lines: the filter, then up to `capacity` matching rows around the highlight,
    /// then the keys (or the note). A row reads `> * 11 coop   COOP`: the highlight, playlist
    /// membership, the view's place among all `entries`, its name and its title.
    pub fn lines(&self, entries: &[Entry], playlist: &[View], capacity: usize) -> Vec<String> {
        let matching = filter(entries, &self.filter);
        let mut lines = vec![format!(
            "View: {}_  {}/{}",
            self.filter,
            matching.len(),
            entries.len()
        )];
        if matching.is_empty() {
            lines.push("no view matches".into());
        }
        let name_width = entries
            .iter()
            .map(|entry| entry.name.len())
            .max()
            .unwrap_or(0);
        let start = window_start(matching.len(), self.cursor, capacity);
        for (offset, entry) in matching.iter().skip(start).take(capacity).enumerate() {
            let number = entries
                .iter()
                .position(|other| other.view == entry.view)
                .map_or(0, |position| position + 1);
            lines.push(format!(
                "{} {} {number:>2} {:<name_width$} {}",
                if start + offset == self.cursor {
                    '>'
                } else {
                    ' '
                },
                if playlist.contains(&entry.view) {
                    '*'
                } else {
                    ' '
                },
                entry.name,
                entry.title
            ));
        }
        lines.push(
            self.note
                .unwrap_or("Enter show | Space playlist | Esc close")
                .into(),
        );
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::{REGISTRY, view};

    fn entries() -> Vec<Entry> {
        View::all()
            .map(|view| Entry {
                view,
                name: view.info().name,
                title: view.info().title,
            })
            .collect()
    }

    fn names(found: &[&Entry]) -> Vec<&'static str> {
        found.iter().map(|entry| entry.name).collect()
    }

    #[test]
    fn the_empty_playlist_request_means_every_available_view() {
        let mut available = vec![true; REGISTRY.len()];
        available[view("orbit").index()] = false;
        let all = playlist(&[], &available);
        assert_eq!(all.len(), REGISTRY.len() - 1);
        assert!(!all.contains(&view("orbit")));
        assert!(all.windows(2).all(|pair| pair[0].index() < pair[1].index()));
    }

    #[test]
    fn a_window_keeps_the_highlight_in_view() {
        assert_eq!(window_start(5, 4, 8), 0, "everything fits");
        assert_eq!(window_start(30, 0, 8), 0);
        assert_eq!(window_start(30, 15, 8), 11);
        assert_eq!(window_start(30, 29, 8), 22, "the end of the list");
        for cursor in 0..30 {
            let start = window_start(30, cursor, 8);
            assert!((start..start + 8).contains(&cursor), "{cursor} in {start}");
        }
    }

    #[test]
    fn the_title_alone_can_match() {
        assert!(matches("coop", "THE FLOCK", "flock"));
        assert!(!matches("coop", "THE FLOCK", "orbit"));
        assert!(matches("coop", "COOP", ""));
        assert_eq!(names(&filter(&entries(), "")).len(), REGISTRY.len());
    }
}
