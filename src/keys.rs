//! The keymap: the one table that says which key does what. `App::command` and the two modal
//! handlers (the search line and the view picker) match on the `Action` it returns, never on a
//! key; the short and `?` help lines, the `--help` key list, `--keys` and the README's `##
//! Controls` tables are all generated from it, and a test keeps the README honest.
//!
//! A `Binding` is one thing a key can do. Which one a press picks is decided by `lookup`:
//! among the bindings of the context (and those of `Context::Any`) whose pattern matches, the
//! one that requires the most modifier keys, then the first in the table; a `KeyPattern::Text`
//! binding is only the fallback for a character nothing else binds. The table's order is also the
//! order of the README rows and of the hints on each help line.

use crossterm::event::{KeyCode, KeyModifiers};

/// The characters a view's key hook may claim (`ViewKey::Local`); every other key is global. No
/// global binding may use one (`view_local_keys_do_not_collide_with_global_bindings`).
pub(crate) const LOCAL_KEYS: &str = "bijkmopuxyz,.;";

/// Where a binding applies. A modal context takes every key while it is open, so a key it does
/// not bind never reaches `Global`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Context {
    /// In every context, whichever is open: the emergency exit.
    Any,
    /// The scene: no search line and no picker is open.
    Global,
    /// The search line is open.
    Search,
    /// The view picker is open.
    Picker,
}

impl Context {
    /// Whether a press in `self` can match a binding declared for `declared`.
    const fn includes(self, declared: Context) -> bool {
        matches!(declared, Context::Any) || self as u8 == declared as u8
    }

    /// Whether one key could be bound in both contexts at once.
    #[cfg(test)]
    const fn overlaps(self, other: Context) -> bool {
        matches!(self, Context::Any) || self.includes(other)
    }
}

/// What a binding does. `App` performs it; the modal handlers read their own variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Quit, after offering the shown view the chance to defer it (`ViewKey::Quit`).
    Quit,
    /// Quit at once, in every context.
    Interrupt,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    ZoomIn,
    ZoomOut,
    RotateClockwise,
    RotateCounterclockwise,
    TiltUp,
    TiltDown,
    /// Toggle top-down and isometric.
    TopDown,
    /// Fit the scene (Home).
    Fit,
    /// Reset the camera and fit (`r`).
    Reset,
    NextView,
    PreviousView,
    /// Open the view picker.
    Picker,
    /// Jump to the view a digit names.
    Jump,
    /// Start the tour now.
    Tour,
    VisitNext,
    VisitRandom,
    /// Cycle the socket links.
    Links,
    Labels,
    /// Hide or show the status panel.
    Panel,
    Help,
    /// Open the search line.
    Search,
    NextMatch,
    Focus,
    Escape,
    Pause,
    Rewind,
    Forward,
    /// A key the shown view may claim (`LOCAL_KEYS`); listed here for the README only, since the
    /// view's own hook reads it.
    ViewLocal,
    /// A mouse gesture, listed here for the README and the help lines only: `App::mouse` reads it.
    Pointer,
    SearchClose,
    SearchErase,
    SearchNext,
    SearchType,
    PickerClose,
    PickerShow,
    PickerUp,
    PickerDown,
    PickerErase,
    PickerPlaylist,
    PickerType,
}

/// Which keys a binding matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyPattern {
    /// This key with any modifiers.
    Key(KeyCode),
    /// This key with at least these modifiers held; any others are ignored.
    With(KeyCode, KeyModifiers),
    /// Any digit.
    Digit,
    /// Any character no other pattern claims.
    Text,
}

impl KeyPattern {
    /// How many modifier keys the pattern requires: the more, the more specific.
    const fn specificity(self) -> u32 {
        match self {
            KeyPattern::With(_, modifiers) => modifiers.bits().count_ones(),
            _ => 0,
        }
    }

    fn matches(self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        match self {
            KeyPattern::Key(key) => key == code,
            KeyPattern::With(key, held) => key == code && modifiers.contains(held),
            KeyPattern::Digit => matches!(code, KeyCode::Char('0'..='9')),
            KeyPattern::Text => matches!(code, KeyCode::Char(_)),
        }
    }

    /// Whether some press matches both patterns with the same specificity, so that only the
    /// table's order would choose between them.
    #[cfg(test)]
    fn clashes(self, other: KeyPattern) -> bool {
        let digit = |pattern| {
            matches!(
                pattern,
                KeyPattern::Digit | KeyPattern::Key(KeyCode::Char('0'..='9'))
            )
        };
        self == other || (digit(self) && digit(other))
    }
}

/// What the keys of a binding are for, to group its rows in the README.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Camera,
    Views,
    Tour,
    Pointer,
    Selection,
    Display,
    History,
    ViewLocal,
    Session,
    /// The rows of a modal context's own table, which has no groups.
    Modal,
}

impl Group {
    const fn title(self) -> &'static str {
        match self {
            Group::Camera => "Camera",
            Group::Views => "Views",
            Group::Tour => "Tour and visits",
            Group::Pointer => "Pointer",
            Group::Selection => "Search, selection and focus",
            Group::Display => "Display",
            Group::History => "History",
            Group::ViewLocal => "View keys",
            Group::Session => "Panels and quit",
            Group::Modal => "",
        }
    }
}

/// Which help line a hint belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Line {
    /// The status line while help is off.
    Brief,
    /// The `?` help line, after the shown view's own keys.
    Keys,
    /// The `?` help line under the statistics: the mouse and a few more keys.
    More,
    /// The line of a modal context (the search line, the picker).
    Prompt,
}

/// A few words of a help line. `{random}`, `{scroll}` and `{Scroll}` in the text are filled in by
/// `Expand`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hint {
    pub(crate) line: Line,
    pub(crate) text: &'static str,
}

/// How a binding appears in the README table.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Docs {
    /// A row: the keys as Markdown and what they do.
    Row {
        label: &'static str,
        help: &'static str,
    },
    /// The keys belong to the row above (Arrows or WASD are four bindings, one row).
    Also,
}

/// One thing a key does.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Binding {
    pub(crate) context: Context,
    pub(crate) group: Group,
    /// Empty for a mouse gesture (`Action::Pointer`), which is listed for the README only.
    pub(crate) keys: &'static [KeyPattern],
    pub(crate) action: Action,
    pub(crate) docs: Docs,
    pub(crate) hints: &'static [Hint],
}

const fn key(character: char) -> KeyPattern {
    KeyPattern::Key(KeyCode::Char(character))
}

const fn code(code: KeyCode) -> KeyPattern {
    KeyPattern::Key(code)
}

const fn row(label: &'static str, help: &'static str) -> Docs {
    Docs::Row { label, help }
}

const fn hint(line: Line, text: &'static str) -> Hint {
    Hint { line, text }
}

const fn global(
    group: Group,
    keys: &'static [KeyPattern],
    action: Action,
    docs: Docs,
    hints: &'static [Hint],
) -> Binding {
    Binding {
        context: Context::Global,
        group,
        keys,
        action,
        docs,
        hints,
    }
}

const fn modal(
    context: Context,
    keys: &'static [KeyPattern],
    action: Action,
    docs: Docs,
    hints: &'static [Hint],
) -> Binding {
    Binding {
        context,
        group: Group::Modal,
        keys,
        action,
        docs,
        hints,
    }
}

/// A mouse gesture: no keys, only the row and the hints.
const fn gesture(label: &'static str, help: &'static str, hints: &'static [Hint]) -> Binding {
    global(
        Group::Pointer,
        &[],
        Action::Pointer,
        row(label, help),
        hints,
    )
}

use Line::{Brief, Keys, More, Prompt};

/// Every binding, in the order of the README's rows and of the hints on each help line.
pub(crate) static KEYMAP: &[Binding] = &[
    // Camera.
    global(
        Group::Camera,
        &[code(KeyCode::Left), key('a')],
        Action::PanLeft,
        row("Arrows or WASD", "Pan"),
        &[hint(Keys, "Arrows/WASD pan")],
    ),
    global(
        Group::Camera,
        &[code(KeyCode::Right), key('d')],
        Action::PanRight,
        Docs::Also,
        &[],
    ),
    global(
        Group::Camera,
        &[code(KeyCode::Up), key('w')],
        Action::PanUp,
        Docs::Also,
        &[],
    ),
    global(
        Group::Camera,
        &[code(KeyCode::Down), key('s')],
        Action::PanDown,
        Docs::Also,
        &[],
    ),
    global(
        Group::Camera,
        &[key('+'), key('=')],
        Action::ZoomIn,
        row("`+` / `-`", "Zoom"),
        &[hint(Keys, "+/- zoom")],
    ),
    global(Group::Camera, &[key('-')], Action::ZoomOut, Docs::Also, &[]),
    global(
        Group::Camera,
        &[key('Q')],
        Action::RotateCounterclockwise,
        row(
            "`Q` / `e`",
            "Rotate a quarter turn counterclockwise / clockwise",
        ),
        &[hint(Keys, "Q/e rotate")],
    ),
    global(
        Group::Camera,
        &[key('e')],
        Action::RotateClockwise,
        Docs::Also,
        &[],
    ),
    global(
        Group::Camera,
        &[code(KeyCode::PageUp)],
        Action::TiltUp,
        row("PgUp / PgDn", "Tilt up / down"),
        &[hint(Keys, "PgUp/PgDn tilt")],
    ),
    global(
        Group::Camera,
        &[code(KeyCode::PageDown)],
        Action::TiltDown,
        Docs::Also,
        &[],
    ),
    global(
        Group::Camera,
        &[key('t')],
        Action::TopDown,
        row("`t`", "Toggle top-down and isometric"),
        &[hint(Keys, "t top-down")],
    ),
    global(
        Group::Camera,
        &[code(KeyCode::Home)],
        Action::Fit,
        row(
            "Home",
            "Fit scene (in the tesseract, also the canonical 4D orientation, the lab frame and the drift)",
        ),
        &[hint(Keys, "Home fit")],
    ),
    // Views.
    global(
        Group::Views,
        &[code(KeyCode::Tab)],
        Action::NextView,
        row(
            "Tab",
            "Next view in the playlist (every view unless `--views` or the picker narrowed it; `--list-views` names them)",
        ),
        &[
            hint(Brief, "Tab next view"),
            hint(Keys, "Tab/Shift-Tab or 0-9 view"),
        ],
    ),
    global(
        Group::Views,
        &[code(KeyCode::BackTab)],
        Action::PreviousView,
        row("Shift + Tab", "Previous view in the playlist"),
        &[],
    ),
    global(
        Group::Views,
        &[key('v')],
        Action::Picker,
        row("`v`", "Open the view picker, which has its own keys"),
        &[hint(Brief, "v views"), hint(Keys, "v views")],
    ),
    global(
        Group::Views,
        &[KeyPattern::Digit],
        Action::Jump,
        row(
            "`1`-`9`, `0`",
            "Jump to a view in that order; the playlist does not apply",
        ),
        &[],
    ),
    // Tour and visits.
    global(
        Group::Tour,
        &[key('g')],
        Action::Tour,
        row(
            "`g`",
            "Start the guided tour now, from the process last visited; `g` again (or any other input) ends it; on the globe it also lets a turned earth spin again",
        ),
        &[hint(Brief, "g tour"), hint(Keys, "g tour")],
    ),
    global(
        Group::Tour,
        &[key(' ')],
        Action::VisitNext,
        row(
            "Space",
            "Visit the next process: the camera zooms to it and shows its callout, as a tour step does. A running tour goes on from there; with no tour the visit just holds. The order is the tour's notable processes, then every other visible process by identity. A view that opts out of the tour ignores it",
        ),
        &[hint(Brief, "Space visit"), hint(Keys, "Space next process")],
    ),
    global(
        Group::Tour,
        &[
            KeyPattern::With(KeyCode::Char(' '), KeyModifiers::SHIFT),
            key('*'),
        ],
        Action::VisitRandom,
        row(
            "Shift + Space, or `*`",
            "Visit a random other process; a running tour goes on from it. Shift + Space needs a terminal with the Kitty keyboard protocol (Ghostty and Kitty have it); without it, Shift + Space arrives as Space, so use `*`, which the help line then names. To tell Shift + Space apart isotop asks such a terminal to report every key as an escape code, so a character composed with AltGr, Option or a dead key reaches the search box as its base key (known limitation)",
        ),
        &[hint(Keys, "{random} random")],
    ),
    // Pointer.
    gesture(
        "Two-finger scroll / wheel",
        "Pan (vertical and horizontal); on the globe, turn the earth (horizontal) and tilt it (vertical)",
        &[hint(Brief, "{scroll}"), hint(More, "{Scroll}")],
    ),
    gesture(
        "Ctrl + scroll",
        "Zoom towards the pointer",
        &[
            hint(Brief, "Ctrl-scroll zoom"),
            hint(More, "Ctrl-scroll zoom"),
        ],
    ),
    gesture(
        "Alt + scroll",
        "Rotate",
        &[hint(More, "Alt-scroll or right-drag rotate/tilt")],
    ),
    gesture(
        "Right or middle drag",
        "Rotate (horizontal) and tilt (vertical)",
        &[],
    ),
    gesture(
        "Hover",
        "Name and description of the process under the pointer, or the title of a thing a view draws",
        &[],
    ),
    gesture(
        "Left click",
        "Select the nearest process, or thing, and open its inspector popup",
        &[hint(Brief, "click inspect")],
    ),
    gesture("Left click on empty space", "Close the popup", &[]),
    gesture("Left drag", "Pan (on the globe too)", &[]),
    // Search, selection and focus.
    global(
        Group::Selection,
        &[key('/')],
        Action::Search,
        row("`/`", "Search name, command, or exact PID"),
        &[hint(Brief, "/ search")],
    ),
    global(
        Group::Selection,
        &[key('n')],
        Action::NextMatch,
        row("`n`", "Next match of the search"),
        &[hint(More, "n next match")],
    ),
    global(
        Group::Selection,
        &[key('f')],
        Action::Focus,
        row("`f`", "Focus selected process and descendants"),
        &[hint(Keys, "f focus")],
    ),
    global(
        Group::Selection,
        &[code(KeyCode::Esc)],
        Action::Escape,
        row(
            "Esc",
            "Close the popup, then the selection, then clear subtree focus",
        ),
        &[],
    ),
    // Display.
    global(
        Group::Display,
        &[key('l')],
        Action::Labels,
        row("`l`", "Toggle labels"),
        &[hint(Keys, "l labels")],
    ),
    global(
        Group::Display,
        &[key('c')],
        Action::Links,
        row("`c`", "Socket links: all, highlighted only, off"),
        &[hint(Brief, "c links"), hint(Keys, "c links")],
    ),
    global(
        Group::Display,
        &[key('r')],
        Action::Reset,
        row(
            "`r`",
            "Reset camera and fit; on the globe, also lets a turned earth spin again; in the tesseract, as Home",
        ),
        &[hint(More, "r reset")],
    ),
    // History.
    global(
        Group::History,
        &[key('P')],
        Action::Pause,
        row("`P` (Shift + p)", "Pause / resume"),
        &[hint(Brief, "P pause")],
    ),
    global(
        Group::History,
        &[key('[')],
        Action::Rewind,
        row("`[` / `]`", "Previous / next retained snapshot"),
        &[hint(More, "[/] rewind")],
    ),
    global(
        Group::History,
        &[key(']')],
        Action::Forward,
        Docs::Also,
        &[],
    ),
    // The keys views may claim. The tesseract is the only view that does.
    global(
        Group::ViewLocal,
        &[key(','), key('.')],
        Action::ViewLocal,
        row(
            "`,` / `.`",
            "Tesseract: a quarter turn back / forward in the current 4D plane (stops the drift)",
        ),
        &[],
    ),
    global(
        Group::ViewLocal,
        &[key(';')],
        Action::ViewLocal,
        row(
            "`;`",
            "Tesseract: cycle the 4D plane, XW, YW, ZW (stops the drift)",
        ),
        &[],
    ),
    global(
        Group::ViewLocal,
        &[key('b')],
        Action::ViewLocal,
        row(
            "`b`",
            "Tesseract: boost into the selected process's rest frame; `b` again or Esc returns to the lab frame",
        ),
        &[],
    ),
    // Panels, help and quit.
    global(
        Group::Session,
        &[key('h')],
        Action::Panel,
        row(
            "`h`",
            "Hide or show the status panel, giving the scene the whole terminal",
        ),
        &[hint(Brief, "h hide panel")],
    ),
    global(
        Group::Session,
        &[key('?')],
        Action::Help,
        row(
            "`?`",
            "Toggle help; its first line leads with the shown view's own keys",
        ),
        &[hint(Brief, "? help")],
    ),
    global(
        Group::Session,
        &[key('q')],
        Action::Quit,
        row(
            "`q`",
            "Quit; the shown view may play a short farewell first, and `q` again ends it",
        ),
        &[hint(Brief, "q quit")],
    ),
    Binding {
        context: Context::Any,
        group: Group::Session,
        keys: &[KeyPattern::With(KeyCode::Char('c'), KeyModifiers::CONTROL)],
        action: Action::Interrupt,
        docs: row("Ctrl-C", "Quit at once, whatever is open"),
        hints: &[hint(More, "Ctrl-C quit")],
    },
    // The search line.
    modal(
        Context::Search,
        &[KeyPattern::Text],
        Action::SearchType,
        row(
            "Any character",
            "Add it to the search; the first match is selected as you type",
        ),
        &[],
    ),
    modal(
        Context::Search,
        &[code(KeyCode::Backspace)],
        Action::SearchErase,
        row("Backspace", "Delete the last character"),
        &[],
    ),
    modal(
        Context::Search,
        &[code(KeyCode::Enter)],
        Action::SearchClose,
        row("Enter", "Accept the search and close the line"),
        &[hint(Prompt, "Enter accept")],
    ),
    modal(
        Context::Search,
        &[code(KeyCode::Esc)],
        Action::SearchClose,
        row("Esc", "Leave the search, keeping the selection"),
        &[hint(Prompt, "Esc cancel")],
    ),
    modal(
        Context::Search,
        &[code(KeyCode::Tab)],
        Action::SearchNext,
        row("Tab", "Next match"),
        &[hint(Prompt, "Tab next")],
    ),
    // The view picker.
    modal(
        Context::Picker,
        &[KeyPattern::Text],
        Action::PickerType,
        row(
            "Letters and digits",
            "Filter by name or title, ignoring case; `q` filters rather than quits",
        ),
        &[hint(Prompt, "Type to filter views")],
    ),
    modal(
        Context::Picker,
        &[code(KeyCode::Backspace)],
        Action::PickerErase,
        row("Backspace", "Delete a filter character"),
        &[],
    ),
    modal(
        Context::Picker,
        &[code(KeyCode::Up)],
        Action::PickerUp,
        row("Up / Down", "Move the highlight among the matches"),
        &[hint(Prompt, "Up/Down move")],
    ),
    modal(
        Context::Picker,
        &[code(KeyCode::Down)],
        Action::PickerDown,
        Docs::Also,
        &[],
    ),
    modal(
        Context::Picker,
        &[code(KeyCode::Enter)],
        Action::PickerShow,
        row(
            "Enter",
            "Show the highlighted view, fitted, and close the picker (nothing happens when no view matches)",
        ),
        &[hint(Prompt, "Enter show")],
    ),
    modal(
        Context::Picker,
        &[key(' ')],
        Action::PickerPlaylist,
        row(
            "Space",
            "Add the highlighted view to the Tab playlist or remove it; the last view cannot be removed",
        ),
        &[hint(Prompt, "Space add or remove from the Tab playlist")],
    ),
    modal(
        Context::Picker,
        &[code(KeyCode::Esc)],
        Action::PickerClose,
        row("Esc", "Close without switching"),
        &[hint(Prompt, "Esc close")],
    ),
];

/// The action a press has in `context`, if any: of the bindings that match, the one requiring the
/// most modifiers, the first in the table among equals; failing those, the first text binding.
pub(crate) fn lookup(context: Context, code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    let mut best: Option<(u32, Action)> = None;
    let mut text = None;
    for binding in KEYMAP.iter().filter(|b| context.includes(b.context)) {
        for pattern in binding.keys.iter().filter(|p| p.matches(code, modifiers)) {
            if *pattern == KeyPattern::Text {
                text = text.or(Some(binding.action));
            } else if best.is_none_or(|(specificity, _)| pattern.specificity() > specificity) {
                best = Some((pattern.specificity(), binding.action));
            }
        }
    }
    best.map(|(_, action)| action).or(text)
}

/// The words `Hint` texts leave open.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Expand {
    /// What the random visit is called: `*`, or `Shift-Space` where the terminal reports it.
    pub(crate) random: &'static str,
    /// The shown view's row turns the globe with the wheel instead of panning.
    pub(crate) turn: bool,
}

/// The hints of `line` in `context`, in table order, joined for the status line (without its
/// leading space).
pub(crate) fn line(context: Context, line: Line, expand: Expand) -> String {
    let scroll = if expand.turn {
        "scroll turn/tilt | drag pan"
    } else {
        "scroll pan"
    };
    let capitalized = if expand.turn {
        "Scroll turn/tilt | drag pan"
    } else {
        "Scroll pan"
    };
    KEYMAP
        .iter()
        .filter(|binding| context.includes(binding.context))
        .flat_map(|binding| binding.hints)
        .filter(|hint| hint.line == line)
        .map(|hint| {
            hint.text
                .replace("{random}", expand.random)
                .replace("{scroll}", scroll)
                .replace("{Scroll}", capitalized)
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Markdown with its backticks removed, for the plain text of `--help`.
fn plain(markdown: &str) -> String {
    markdown.replace('`', "")
}

/// The rows of one context's table: the documented bindings with their label and help.
fn rows(context: Context) -> impl Iterator<Item = (&'static Binding, &'static str, &'static str)> {
    KEYMAP
        .iter()
        // The emergency exit is listed with the scene's keys, not again in each modal table.
        .filter(move |binding| {
            binding.context == context
                || (context == Context::Global && binding.context == Context::Any)
        })
        .filter_map(|binding| match binding.docs {
            Docs::Row { label, help } => Some((binding, label, help)),
            Docs::Also => None,
        })
}

/// The table of `context` as Markdown, in the form of the README's `## Controls` tables: a header,
/// then a row for every documented binding, with a bold row naming each group of the global table.
fn table(context: Context) -> String {
    let first = if context == Context::Global {
        "Key / mouse"
    } else {
        "Key"
    };
    let mut lines = vec![format!("| {first} | Action |"), "| --- | --- |".to_owned()];
    let titled = context == Context::Global;
    let mut group = None;
    for (binding, label, help) in rows(context) {
        if titled && group != Some(binding.group) {
            group = Some(binding.group);
            lines.push(format!("| **{}** | |", binding.group.title()));
        }
        lines.push(format!("| {label} | {help} |"));
    }
    lines.join("\n")
}

/// The README's `## Controls` tables: the keys and the mouse, the search line and the picker.
pub(crate) fn tables() -> [String; 3] {
    [
        table(Context::Global),
        table(Context::Search),
        table(Context::Picker),
    ]
}

/// What `--keys` prints: the tables, a blank line between.
pub(crate) fn markdown() -> String {
    tables().join("\n\n") + "\n"
}

/// The keys and gestures as plain text, for the end of `--help`.
pub(crate) fn summary() -> String {
    let width = rows(Context::Global)
        .map(|(_, label, _)| plain(label).len())
        .max()
        .unwrap_or(0);
    let mut lines = vec!["Controls (--keys prints them as a Markdown table):".to_owned()];
    for (_, label, help) in rows(Context::Global) {
        let sentence = help.split(". ").next().unwrap_or(help);
        lines.push(format!("  {:<width$}  {}", plain(label), plain(sentence)));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: KeyModifiers = KeyModifiers::NONE;

    fn at(context: Context, character: char) -> Option<Action> {
        lookup(context, KeyCode::Char(character), NONE)
    }

    #[test]
    fn every_binding_has_help_text_and_no_key_is_bound_twice_in_one_context() {
        let mut documented = None;
        for (index, binding) in KEYMAP.iter().enumerate() {
            match binding.docs {
                Docs::Row { label, help } => {
                    assert!(
                        !label.is_empty() && !help.is_empty(),
                        "{:?} needs a label and help text",
                        binding.action
                    );
                    documented = Some(binding);
                }
                Docs::Also => {
                    let above = documented.expect("a continuation follows a row");
                    assert_eq!(
                        (above.context, above.group),
                        (binding.context, binding.group),
                        "{:?} continues a row of another table",
                        binding.action
                    );
                    assert!(!binding.keys.is_empty());
                }
            }
            for hint in binding.hints {
                assert!(!hint.text.is_empty());
                assert_eq!(
                    hint.line == Prompt,
                    binding.context == Context::Search || binding.context == Context::Picker,
                    "{:?}: only a modal context has a prompt",
                    binding.action
                );
            }
            // A gesture has no key, and nothing else is without one.
            assert_eq!(
                binding.keys.is_empty(),
                binding.action == Action::Pointer,
                "{:?}",
                binding.action
            );
            for other in &KEYMAP[index + 1..] {
                if !binding.context.overlaps(other.context) {
                    continue;
                }
                for (ours, theirs) in binding
                    .keys
                    .iter()
                    .flat_map(|ours| other.keys.iter().map(move |theirs| (ours, theirs)))
                {
                    assert!(
                        !ours.clashes(*theirs),
                        "{ours:?} is bound to both {:?} and {:?}",
                        binding.action,
                        other.action
                    );
                }
            }
        }
    }

    #[test]
    fn view_local_keys_do_not_collide_with_global_bindings() {
        for binding in KEYMAP {
            let claimed = binding.context == Context::Global || binding.context == Context::Any;
            for pattern in binding.keys {
                let character = match pattern {
                    KeyPattern::Key(KeyCode::Char(c)) | KeyPattern::With(KeyCode::Char(c), _) => {
                        Some(*c)
                    }
                    _ => None,
                };
                let local = character.is_some_and(|c| LOCAL_KEYS.contains(c));
                if binding.action == Action::ViewLocal {
                    assert!(local, "{pattern:?} is not one of the view-local keys");
                } else if claimed {
                    assert!(
                        !local,
                        "{:?} binds {pattern:?}, which belongs to the views",
                        binding.action
                    );
                }
            }
        }
        // The lookup agrees: a key the views may claim is bound to nothing, or only to the row
        // that documents the view's use of it.
        for character in LOCAL_KEYS.chars() {
            assert!(
                matches!(
                    at(Context::Global, character),
                    None | Some(Action::ViewLocal)
                ),
                "{character}"
            );
        }
    }

    #[test]
    fn labels_name_the_characters_they_bind() {
        let mut label = "";
        for binding in KEYMAP {
            if let Docs::Row { label: shown, .. } = binding.docs {
                label = shown;
            }
            for pattern in binding.keys {
                let character = match pattern {
                    KeyPattern::Key(KeyCode::Char(c)) => *c,
                    KeyPattern::Digit => '1',
                    _ => continue,
                };
                // WASD is named as a set, `=` is the unshifted `+`, and Space is named by its word.
                if (label.contains("WASD") && "adsw".contains(character))
                    || character == '='
                    || character == ' '
                {
                    continue;
                }
                assert!(
                    label.contains(character),
                    "the row \"{label}\" does not name {character:?}, bound to {:?}",
                    binding.action
                );
            }
        }
    }

    #[test]
    fn the_most_specific_binding_wins_and_text_is_the_fallback() {
        let shifted = |character| {
            lookup(
                Context::Global,
                KeyCode::Char(character),
                KeyModifiers::SHIFT,
            )
        };
        assert_eq!(at(Context::Global, ' '), Some(Action::VisitNext));
        assert_eq!(shifted(' '), Some(Action::VisitRandom));
        assert_eq!(at(Context::Global, '*'), Some(Action::VisitRandom));
        assert_eq!(at(Context::Global, 'c'), Some(Action::Links));
        let control = |context| lookup(context, KeyCode::Char('c'), KeyModifiers::CONTROL);
        for context in [Context::Global, Context::Search, Context::Picker] {
            assert_eq!(control(context), Some(Action::Interrupt), "{context:?}");
        }
        // The picker's Space is its own, and any other character is text.
        assert_eq!(at(Context::Picker, ' '), Some(Action::PickerPlaylist));
        assert_eq!(at(Context::Picker, 'q'), Some(Action::PickerType));
        assert_eq!(at(Context::Search, 'q'), Some(Action::SearchType));
        assert_eq!(at(Context::Global, 'q'), Some(Action::Quit));
        // Modifiers other than the ones a binding names are ignored, as they always were.
        assert_eq!(
            lookup(Context::Global, KeyCode::Char('a'), KeyModifiers::ALT),
            Some(Action::PanLeft)
        );
        assert_eq!(
            lookup(Context::Global, KeyCode::BackTab, KeyModifiers::SHIFT),
            Some(Action::PreviousView)
        );
        for digit in '0'..='9' {
            assert_eq!(at(Context::Global, digit), Some(Action::Jump), "{digit}");
        }
        assert_eq!(at(Context::Global, 'x'), None);
        assert_eq!(lookup(Context::Global, KeyCode::F(5), NONE), None);
    }

    #[test]
    fn the_help_lines_are_made_from_the_table() {
        let expand = |random, turn| Expand { random, turn };
        let global = |kind, random, turn| line(Context::Global, kind, expand(random, turn));
        assert_eq!(
            global(Brief, "*", false),
            "Tab next view | v views | g tour | Space visit | scroll pan | Ctrl-scroll zoom | click inspect | / search | c links | P pause | h hide panel | ? help | q quit"
        );
        assert_eq!(
            global(Brief, "*", true),
            "Tab next view | v views | g tour | Space visit | scroll turn/tilt | drag pan | Ctrl-scroll zoom | click inspect | / search | c links | P pause | h hide panel | ? help | q quit"
        );
        assert_eq!(
            global(Keys, "*", false),
            "Arrows/WASD pan | +/- zoom | Q/e rotate | PgUp/PgDn tilt | t top-down | Home fit | Tab/Shift-Tab or 0-9 view | v views | g tour | Space next process | * random | f focus | l labels | c links"
        );
        assert!(global(Keys, "Shift-Space", false).contains("| Shift-Space random |"));
        assert_eq!(
            global(More, "*", false),
            "Scroll pan | Ctrl-scroll zoom | Alt-scroll or right-drag rotate/tilt | n next match | r reset | [/] rewind | Ctrl-C quit"
        );
        assert_eq!(
            line(Context::Search, Prompt, expand("*", false)),
            "Enter accept | Esc cancel | Tab next"
        );
        assert_eq!(
            line(Context::Picker, Prompt, expand("*", false)),
            "Type to filter views | Up/Down move | Enter show | Space add or remove from the Tab playlist | Esc close"
        );
    }

    #[test]
    fn readme_controls_match_the_keymap() {
        let readme = include_str!("../README.md");
        let controls = readme
            .split("\n## Controls\n")
            .nth(1)
            .expect("the README has a Controls section");
        let controls = controls.split("\n## ").next().unwrap_or(controls);
        let mut found: Vec<Vec<&str>> = Vec::new();
        let mut inside = false;
        for line in controls.lines() {
            if line.starts_with('|') {
                if !inside {
                    found.push(Vec::new());
                }
                found.last_mut().expect("a table").push(line);
            }
            inside = line.starts_with('|');
        }
        let readme_tables: Vec<String> = found.iter().map(|lines| lines.join("\n")).collect();
        assert_eq!(
            readme_tables,
            tables(),
            "README.md's Controls tables differ from the keymap; paste the output of `isotop --keys` over them"
        );
    }

    #[test]
    fn the_summary_for_help_lists_every_row() {
        let summary = summary();
        assert!(summary.contains("Q / e"), "{summary}");
        assert!(!summary.contains('`'), "{summary}");
        assert_eq!(
            summary.lines().count(),
            1 + rows(Context::Global).count(),
            "{summary}"
        );
    }
}
