//! Test only: text a hostile machine could supply, through every view and the whole overlay.
//!
//! Every string the model holds from the system (names, command lines, cgroup and unit names,
//! thread names, the paths of open files, interrupt and device names, the exit records' command
//! names, the lines of the journal, the place names of the GeoIP lookups and the parents isotop
//! cannot read) is replaced
//! by `HOSTILE`, which carries a window title, a screen clear, a C1 control, a bidirectional
//! override and line breaks. Each view is then drawn for twelve seconds of demo time with a
//! selection, a hover, a tour and every thing it drew opened, and everything the app hands the
//! terminal is written out and searched for what a terminal would act on.

use std::collections::HashMap;
use std::sync::Arc;

use crate::io_detail::{FileFlow, IoDetail};
use crate::journal::{Delivery, Line};
use crate::model::{Measured, Process, Snapshot};
use crate::platform::LifeKind;
use crate::terminal::{self, Label, Popup, SafeText};
use crate::threads::{Thread, ThreadList};

use super::fixtures::*;

/// A name that tries a window title (OSC 0), a screen clear (CSI 2 J), a C1 control sequence
/// introducer, a C1 next line, a right-to-left override, an isolate and line breaks.
const HOSTILE: &str = "ev\x1b]0;pwned\x07il\x1b[2J\u{9b}31m\u{85}\u{202e}\r\n\u{2066}z";

/// What `HOSTILE` becomes in the first thirteen characters, which survive every cut of a label.
const CLEANED: &str = "ev?]0;pwned?i";

fn hostile(original: &str) -> String {
    format!("{HOSTILE}{original}")
}

/// `snapshot` with every string the system supplies replaced by a hostile one.
fn poisoned(mut snapshot: Snapshot, number: u64) -> Snapshot {
    let process = |process: &mut Process| {
        process.name = hostile(&process.name);
        process.command = hostile(&process.command);
        process.group = hostile(&process.group);
        process.cgroup = hostile(&process.cgroup);
        if let Measured::Known(detail) = &process.io_detail {
            let files = detail
                .files
                .iter()
                .map(|file| FileFlow {
                    path: hostile(&file.path),
                    ..file.clone()
                })
                .collect();
            process.io_detail = Measured::Known(Arc::new(IoDetail {
                files,
                ..(**detail).clone()
            }));
        }
        if let Measured::Known(list) = &process.thread_list {
            let threads: Vec<Thread> = list
                .threads
                .iter()
                .map(|thread| Thread {
                    name: hostile(&thread.name),
                    ..thread.clone()
                })
                .collect();
            process.thread_list = Measured::Known(ThreadList {
                threads: threads.into(),
                complete: list.complete,
            });
        }
    };
    snapshot.processes.iter_mut().for_each(process);
    snapshot.observer.iter_mut().for_each(process);
    snapshot.units = std::mem::take(&mut snapshot.units)
        .into_iter()
        .map(|(name, unit)| (hostile(&name), unit))
        .collect::<HashMap<_, _>>();
    for shadow in &mut snapshot.shadows {
        shadow.name = hostile(&shadow.name);
    }
    snapshot.geo = hostile(&snapshot.geo);
    if let Some(home) = &mut snapshot.home {
        home.name = hostile(&home.name);
    }
    if let Measured::Known(rows) = &mut snapshot.interrupts {
        for row in rows {
            row.id = hostile(&row.id);
            row.chip = hostile(&row.chip);
            row.trigger = hostile(&row.trigger);
            row.description = hostile(&row.description);
            row.devices
                .iter_mut()
                .for_each(|device| *device = hostile(device));
        }
    }
    if let Measured::Known(rows) = &mut snapshot.softirqs {
        rows.iter_mut().for_each(|(name, _)| *name = hostile(name));
    }
    for event in &mut snapshot.life {
        if let LifeKind::Exit {
            record: Some(record),
            ..
        } = &mut event.kind
        {
            record.comm = hostile(&record.comm);
        }
    }
    snapshot.journal = Delivery {
        lines: vec![Line::new(3, HOSTILE)],
        through: number,
        dropped: 0,
    };
    snapshot
}

/// The residue of terminal output once the cursor moves and styles the overlay writes itself are
/// taken out: whatever is left must be text, with nothing a terminal acts on.
fn residue(output: &[u8]) -> String {
    let text = String::from_utf8(output.to_vec()).expect("the overlay writes UTF-8");
    let mut kept = String::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\x1b' && characters.peek() == Some(&'[') {
            characters.next();
            // The parameters, then one final byte.
            for next in characters.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&next) {
                    break;
                }
            }
        } else {
            kept.push(character);
        }
    }
    kept
}

/// Panics if `text` holds a character a terminal could act on or an RTL control.
fn assert_inert(text: &str, what: &str) {
    for character in text.chars() {
        let bidirectional = matches!(character, '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
        assert!(
            !character.is_control() && !bidirectional,
            "{what}: {character:?} reached the terminal in {text:?}"
        );
    }
}

/// Draws labels and panels as `Terminal::present` does, and checks them.
fn written(labels: &[Label], panels: &[Popup], columns: u16) -> String {
    let mut output = Vec::new();
    for label in labels {
        terminal::draw_label(&mut output, label, columns).unwrap();
    }
    for panel in panels {
        terminal::draw_popup(&mut output, panel).unwrap();
    }
    residue(&output)
}

/// Everything the app hands the terminal for `row` after twelve seconds, checked, and whether any
/// of it showed the hostile text.
fn shown_by(view: View) -> bool {
    let row = view.info();
    let count = 48;
    let first = poisoned(crate::demo_snapshot(row, -1.0, 0.0, count), 1);
    let mut app = App::new(view, first, all_available());
    let mut shown = String::new();
    let layout = SCENE;
    for second in 1..=12 {
        let time = f64::from(second);
        let snapshot = poisoned(
            crate::demo_snapshot(row, time - 1.0, time, count),
            second as u64 + 1,
        );
        app.push_sample(snapshot, usize::MAX);
        app.animation = time as f32;
        app.render(layout.width, layout.height, 512);
    }
    let mut frame = app.render(layout.width, layout.height, 512);
    let radius = layout.pick_radius(false);
    let gather = |app: &mut App, frame: &mut Frame, shown: &mut String| {
        let (labels, panels) = app.overlay(frame, &layout, radius);
        shown.push_str(&written(&labels, &panels, layout.columns));
        for text in labels.iter().map(|label| &label.text) {
            assert_inert(text.as_str(), view.name());
        }
        for text in panels.iter().flat_map(|panel| &panel.lines) {
            assert_inert(text.as_str(), view.name());
        }
        for text in app.status_rows(1.0, "shared memory", 20, 512) {
            assert_inert(text.as_str(), view.name());
            shown.push_str(text.as_str());
        }
    };
    gather(&mut app, &mut frame, &mut shown);
    // The first process drawn, selected, hovered and toured.
    let drawn: Vec<Identity> = app.scene.positions.keys().copied().collect();
    for id in drawn.into_iter().take(6) {
        app.selected = Some(id);
        if let Some(at) = app.screen(&frame, id) {
            app.pointer.hover = Some(Spot {
                frame: at,
                cell: layout.cell_of(at),
            });
        }
        let mut frame = app.render(layout.width, layout.height, 512);
        gather(&mut app, &mut frame, &mut shown);
    }
    app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
    let mut frame = app.render(layout.width, layout.height, 512);
    gather(&mut app, &mut frame, &mut shown);
    // Every thing the view drew, opened as a card and hovered as a tag.
    app.tour = None;
    app.selected = None;
    for thing in frame.things.clone().into_iter().take(12) {
        app.thing = Some(thing);
        let mut frame = app.render(layout.width, layout.height, 512);
        gather(&mut app, &mut frame, &mut shown);
    }
    shown.contains(CLEANED)
}

#[test]
fn hostile_text_reaches_no_terminal_in_any_view() {
    let mut showing = Vec::new();
    for view in View::all() {
        if shown_by(view) {
            showing.push(view.name());
        }
    }
    // The check is not vacuous: the process views do put the names on the screen.
    for name in ["city", "orbit", "murmuration", "arbor", "anthill"] {
        assert!(
            showing.contains(&name),
            "{name} shows no process name: {showing:?}"
        );
    }
}

#[test]
fn the_journal_follower_lines_of_the_matrix_become_status_text_only_as_safe_text() {
    let mut app = App::new(view("matrix"), model::demo(1.0, 16), all_available());
    let App {
        history, journal, ..
    } = &mut app;
    journal.attach(
        &mut history.samples_mut()[0],
        vec![Line::new(6, HOSTILE)],
        0,
    );
    app.render(320, 180, 512);
    let rows = app.status_rows(0.0, "test", 20, 512);
    assert_eq!(rows[1], SafeText::new(&format!(" > {HOSTILE}")));
    assert!(rows[1].as_str().contains(CLEANED), "{:?}", rows[1]);
    assert_inert(rows[1].as_str(), "the matrix's status row");
}

#[test]
fn every_status_row_carries_the_name_of_the_selected_process_cleaned() {
    let mut snapshot = poisoned(model::demo(1.0, 16), 1);
    snapshot.processes[0].cpu = 40.0;
    let id = snapshot.processes[0].id;
    let mut app = App::new(view("city"), snapshot, all_available());
    app.selected = Some(id);
    app.render(320, 180, 512);
    let rows = app.status_rows(0.0, "test", 20, 512);
    let selected = rows.iter().any(|row| row.as_str().contains(CLEANED));
    assert!(selected, "{rows:?}");
    for row in &rows {
        assert_inert(row.as_str(), "a status row");
    }
}

#[test]
fn residue_finds_what_the_overlay_would_leave_for_a_terminal_to_act_on() {
    assert_eq!(residue(b"\x1b[1;1H\x1b[38;2;1;2;3mhi\x1b[0m"), "hi");
    assert_eq!(residue(b"a\x1b]0;x\x07"), "a\x1b]0;x\x07");
    assert_eq!(residue("a\u{9b}2J".as_bytes()), "a\u{9b}2J");
    assert!(std::panic::catch_unwind(|| assert_inert("a\u{202e}b", "test")).is_err());
    assert!(std::panic::catch_unwind(|| assert_inert("a\u{85}b", "test")).is_err());
    assert!(std::panic::catch_unwind(|| assert_inert("a\x1bb", "test")).is_err());
    assert_inert("plain text, ⣿ and box ─", "test");
}
