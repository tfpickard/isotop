//! Text that may be written to the terminal: `SafeText`, and the cell widths it is measured in.

use std::fmt;

/// Text the terminal may be given. Process names, command lines, thread and cgroup names, journal
/// lines, socket peers and file paths are untrusted, and a control sequence in one of them could
/// move the cursor, retitle the window or ask the terminal to answer. The only way to make a
/// `SafeText` is a constructor in this module, each of which replaces every character it does not
/// allow with `?`, so a label, a popup line or a status row (`Label`, `Popup`,
/// `Terminal::present`) that takes one cannot carry a control character. The field is private
/// even to the rest of `terminal`.
///
/// What passes is the printable ASCII range and the space; `braille` also passes the Braille
/// Patterns block. Every character that passes is one cell wide, and `width` measures with
/// `cell_width` all the same, so allowing printable Unicode later (a follow-up that needs the
/// width tables to be checked against real terminals) changes the constructors and nothing that
/// lays text out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SafeText(String);

impl SafeText {
    /// `text` with every character outside printable ASCII and the space replaced by `?`.
    pub fn new(text: &str) -> Self {
        Self(
            text.chars()
                .map(|c| if allowed(c) { c } else { '?' })
                .collect(),
        )
    }

    /// A literal of the program's own. It is cleaned like any text, and a debug build asserts
    /// that cleaning changed nothing, so a literal that would show a `?` is found by running the
    /// tests. A `&'static str` cannot hold data read at run time.
    pub fn trusted(text: &'static str) -> Self {
        let safe = Self::new(text);
        debug_assert_eq!(safe.0, text, "a trusted literal must be printable ASCII");
        safe
    }

    /// A sparkline a view generated: the Braille Patterns block (U+2800 to U+28FF) and the space
    /// pass, and every other character becomes `?`, so a sparkline carries nothing but dots. The
    /// type holds the difference, so a sparkline cannot be shown as an ordinary line or the
    /// reverse. Only `Stage::sparklines` reaches it.
    pub fn braille(text: &str) -> Self {
        Self(
            text.chars()
                .map(|c| {
                    if ('\u{2800}'..='\u{28ff}').contains(&c) || c == ' ' {
                        c
                    } else {
                        '?'
                    }
                })
                .collect(),
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The width in terminal cells, which every layout of text uses: never the byte length.
    pub fn width(&self) -> usize {
        display_width(&self.0)
    }

    /// The longest start of the text that is at most `cells` wide. A character two cells wide
    /// that would cross the edge is left out whole.
    pub fn cut(&self, cells: usize) -> &str {
        let mut used = 0;
        for (index, c) in self.0.char_indices() {
            used += cell_width(c);
            if used > cells {
                return &self.0[..index];
            }
        }
        &self.0
    }
}

impl fmt::Display for SafeText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl PartialEq<str> for SafeText {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for SafeText {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// Whether a character is shown as it is: printable ASCII and the space.
fn allowed(c: char) -> bool {
    c.is_ascii_graphic() || c == ' '
}

/// The width of `text` in terminal cells.
pub fn display_width(text: &str) -> usize {
    text.chars().map(cell_width).sum()
}

/// Characters that take no cell of their own: controls, the joiners and marks that combine with
/// the character before, bidirectional controls, variation selectors and tags.
const ZERO_WIDTH: &[(u32, u32)] = &[
    (0x0000, 0x001F),
    (0x007F, 0x009F),
    (0x0300, 0x036F),
    (0x0483, 0x0489),
    (0x0591, 0x05BD),
    (0x0610, 0x061A),
    (0x064B, 0x065F),
    (0x0E31, 0x0E31),
    (0x0E34, 0x0E3A),
    (0x0E47, 0x0E4E),
    (0x1AB0, 0x1AFF),
    (0x1DC0, 0x1DFF),
    (0x200B, 0x200F),
    (0x2028, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0x20D0, 0x20FF),
    (0xFE00, 0xFE0F),
    (0xFE20, 0xFE2F),
    (0xFEFF, 0xFEFF),
    (0xE0001, 0xE007F),
    (0xE0100, 0xE01EF),
];

/// Characters two cells wide: Hangul Jamo, the CJK blocks, the fullwidth forms and the emoji
/// that default to emoji presentation.
const DOUBLE_WIDTH: &[(u32, u32)] = &[
    (0x1100, 0x115F),
    (0x231A, 0x231B),
    (0x2329, 0x232A),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x2E80, 0x303E),
    (0x3041, 0x33FF),
    (0x3400, 0x4DBF),
    (0x4E00, 0xA4CF),
    (0xA960, 0xA97F),
    (0xAC00, 0xD7A3),
    (0xF900, 0xFAFF),
    (0xFE10, 0xFE19),
    (0xFE30, 0xFE6F),
    (0xFF00, 0xFF60),
    (0xFFE0, 0xFFE6),
    (0x16FE0, 0x16FE4),
    (0x17000, 0x18CFF),
    (0x1B000, 0x1B2FF),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F200, 0x1F265),
    (0x1F300, 0x1F64F),
    (0x1F680, 0x1F6FF),
    (0x1F7E0, 0x1F7EB),
    (0x1F90C, 0x1F9FF),
    (0x1FA70, 0x1FAFF),
    (0x20000, 0x2FFFD),
    (0x30000, 0x3FFFD),
];

/// The width of one character in terminal cells: 0 for the ranges in `ZERO_WIDTH`, 2 for those in
/// `DOUBLE_WIDTH` and 1 for everything else. The ranges follow the East Asian Width and emoji
/// properties of Unicode without being the whole of the tables, and terminals themselves differ
/// on ambiguous characters, so nothing may rely on a wide character's width until the follow-up
/// that allows printable Unicode has measured it in Ghostty and Kitty.
pub fn cell_width(c: char) -> usize {
    if allowed(c) {
        return 1;
    }
    let code = c as u32;
    let within = |ranges: &[(u32, u32)]| {
        ranges
            .iter()
            .any(|&(low, high)| (low..=high).contains(&code))
    };
    if within(ZERO_WIDTH) {
        0
    } else if within(DOUBLE_WIDTH) {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_wide_combining_and_zero_width_characters_have_their_own_widths() {
        assert_eq!(display_width("abc DEF ~"), 9);
        assert_eq!(display_width("日本語"), 6, "CJK ideographs take two cells");
        assert_eq!(display_width("한글"), 4, "so does Hangul");
        assert_eq!(display_width("ＡＢ"), 4, "and the fullwidth forms");
        assert_eq!(display_width("ｱ"), 1, "halfwidth katakana is one");
        assert_eq!(display_width("e\u{301}"), 1, "a combining accent adds none");
        assert_eq!(
            display_width("a\u{200b}b"),
            2,
            "a zero width space adds none"
        );
        assert_eq!(display_width("a\u{200d}b"), 2, "nor does a joiner");
        assert_eq!(display_width("\u{202e}x\u{2066}"), 1, "nor bidi controls");
        assert_eq!(display_width("a\u{fe0f}"), 1, "nor a variation selector");
        assert_eq!(display_width("🦀"), 2, "an emoji takes two cells");
        assert_eq!(display_width("🦀\u{fe0f}"), 2, "its selector adds none");
        assert_eq!(display_width("é"), 1, "a precomposed letter is one");
        assert_eq!(display_width("⣿"), 1, "and a braille cell");
        assert_eq!(
            display_width("\x1b\n\t\u{7f}\u{85}"),
            0,
            "controls take no cell"
        );
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn an_ascii_character_is_one_cell_and_nothing_else_is_probed() {
        for c in (0x20_u8..0x7f).map(char::from) {
            assert_eq!(cell_width(c), 1, "{c:?}");
        }
    }

    #[test]
    fn text_is_cleaned_to_printable_ascii_and_measured_in_cells_not_bytes() {
        let safe = SafeText::new("bad\x1b[2J\nname");
        assert_eq!(safe, "bad?[2J?name");
        assert_eq!(safe.width(), 12);
        assert!(!safe.as_str().contains('\x1b'));
        // Each character becomes one `?`, so a three-byte ideograph is one cell, not three.
        let wide = SafeText::new("日本語");
        assert_eq!(wide, "???");
        assert_eq!(wide.width(), 3);
        assert_eq!(wide.as_str().len(), 3);
        // A combining mark is a character of its own, and so takes the cell of its `?`.
        assert_eq!(SafeText::new("e\u{301}"), "e?");
        assert_eq!(SafeText::new("🦀"), "?");
        assert_eq!(
            SafeText::new("a\u{85}\u{9b}\u{202e}b"),
            "a???b",
            "C1 and bidi"
        );
        assert_eq!(SafeText::new("a\0b"), "a?b");
        assert_eq!(SafeText::new("a\u{7f}b"), "a?b");
        assert_eq!(SafeText::new(""), "");
    }

    #[test]
    fn text_is_cut_to_a_width_without_splitting_a_character() {
        let safe = SafeText::new("abcdef");
        assert_eq!(safe.cut(3), "abc");
        assert_eq!(safe.cut(6), "abcdef");
        assert_eq!(safe.cut(99), "abcdef");
        assert_eq!(safe.cut(0), "");
        let sparkline = SafeText::braille("⣿⣿⣿⣿");
        assert_eq!(
            sparkline.cut(2),
            "⣿⣿",
            "cut to the width in cells, not bytes"
        );
    }

    #[test]
    fn a_wide_character_that_would_cross_the_edge_is_left_out_whole() {
        // Only a widened constructor can hold one; the cut is what a layout will rely on.
        let wide = SafeText("a日b".into());
        assert_eq!(wide.width(), 4);
        assert_eq!(wide.cut(1), "a");
        assert_eq!(wide.cut(2), "a", "the ideograph needs cells 2 and 3");
        assert_eq!(wide.cut(3), "a日");
        assert_eq!(wide.cut(4), "a日b");
        let marked = SafeText("e\u{301}x".into());
        assert_eq!(marked.cut(1), "e\u{301}", "a mark stays with its letter");
    }

    #[test]
    fn the_sparkline_constructor_passes_braille_but_not_controls_or_other_text() {
        assert_eq!(SafeText::braille("⣀⣤⣶⣿ ⠁"), "⣀⣤⣶⣿ ⠁");
        assert_eq!(SafeText::braille("\u{2800}\u{28ff}"), "\u{2800}\u{28ff}");
        assert_eq!(SafeText::braille("⣿\x1b[2J\n⣿"), "⣿?????⣿");
        assert_eq!(SafeText::braille("a\u{27ff}\u{2900}\u{202e}"), "????");
        assert_eq!(SafeText::braille("⣿⣿⣿⣿").width(), 4, "a cell per dot");
    }

    #[test]
    fn ordinary_text_loses_braille() {
        assert_eq!(SafeText::new("⣿⣤name"), "??name");
        assert_eq!(SafeText::new("\u{2800}"), "?");
    }

    #[test]
    fn a_trusted_literal_is_the_literal() {
        assert_eq!(
            SafeText::trusted("Esc close | f focus"),
            "Esc close | f focus"
        );
    }

    /// The two halves of a test of whether a conversion exists, told without the compiler
    /// refusing the question: of the two `answer` methods, `Yes` is found first (for a
    /// `&Probe`) and applies only where `A: From<B>`; otherwise `No` is found after an
    /// auto-reference.
    struct Probe<A, B>(std::marker::PhantomData<(A, B)>);

    trait Yes {
        fn answer(&self) -> bool {
            true
        }
    }

    impl<A: From<B>, B> Yes for Probe<A, B> {}

    trait No {
        fn answer(&self) -> bool {
            false
        }
    }

    impl<A, B> No for &Probe<A, B> {}

    /// Whether `$target` converts from `$source`.
    macro_rules! converts {
        ($target:ty, $source:ty) => {
            (&Probe::<$target, $source>(std::marker::PhantomData)).answer()
        };
    }

    #[test]
    fn safe_text_cannot_be_made_from_a_string_without_a_constructor() {
        // The probe itself works: a type that converts says so.
        assert!(converts!(String, &str));
        assert!(!converts!(SafeText, String));
        assert!(!converts!(SafeText, &str));
        assert!(!converts!(SafeText, &'static str));
        assert!(!converts!(SafeText, Box<str>));
        assert!(!converts!(SafeText, std::borrow::Cow<'static, str>));
    }

    #[test]
    fn the_terminal_types_hold_safe_text_and_no_string() {
        use crate::terminal::{Label, Popup};
        // These coercions compile only while the fields have exactly these types; a `String`
        // field, to which raw process text could be assigned, is a type error here.
        let _: fn(&Label) -> &SafeText = |label| &label.text;
        let _: fn(&Popup) -> &Vec<SafeText> = |popup| &popup.lines;
    }
}
