use std::collections::HashMap;

use crate::model::{GpuMemory, Identity, Process, bytes, describe};
use crate::platform::{self};
use crate::render::{Card, Frame, Point, Target, Thing, label_name};
use crate::terminal::{Label, Popup, Tone};

use super::App;
use super::screen::{Board, Layout, place_lines, place_panel};

impl App {
    pub(super) fn screen(&self, frame: &Frame, id: Identity) -> Option<[f32; 2]> {
        let &p = self.scene.positions.get(&id)?;
        frame.locate(&self.camera, p)
    }

    /// The selected process, unless the view draws no processes and shows a line of its own in
    /// place of the inspector; the selection itself is kept for when a process view returns.
    pub(super) fn inspected(&self) -> Option<Identity> {
        self.selected
            .filter(|_| self.scene.inspector_line(self.view).is_none())
    }

    /// The selected thing's popup: its card's title and lines beside the point the card names,
    /// or a note that the view no longer shows it.
    pub(super) fn thing_popup(
        &self,
        thing: Thing,
        frame: &Frame,
        layout: &Layout,
    ) -> Option<Popup> {
        let (lines, anchor) = match self.scene.card(self.view, thing, self.snapshot()) {
            Some(card) => {
                let mut lines = vec![card.title];
                lines.extend(card.lines);
                lines.push("Esc close".into());
                (lines, frame.locate(&self.camera, card.at))
            }
            None => (
                vec![
                    format!("{} no longer shown", thing.kind),
                    "Esc close".into(),
                ],
                None,
            ),
        };
        place_panel(lines, anchor.map(|a| layout.cell_of(a)), layout, false, 32)
    }

    pub(super) fn popup(&self, frame: &Frame, layout: &Layout) -> Option<Popup> {
        if let Some(thing) = self.thing {
            return self.thing_popup(thing, frame, layout);
        }
        let id = self.inspected()?;
        let snapshot = self.snapshot();
        // The lines `Stage::sparklines` supplied, which the terminal cleans as braille.
        let mut braille = Vec::new();
        let lines = match snapshot.processes.iter().find(|p| p.id == id) {
            Some(p) => {
                let mut lines = vec![
                    p.name.clone(),
                    describe(p),
                    format!(
                        "pid {} | parent {} | state {} | {} thread{}",
                        id.pid,
                        p.parent,
                        p.state,
                        p.threads,
                        if p.threads == 1 { "" } else { "s" }
                    ),
                    format!(
                        "CPU {:.1}% | {} {} | IO {}",
                        p.cpu,
                        platform::MEMORY_LABEL,
                        bytes(p.memory),
                        p.io_rate.map_or_else(
                            || "unavailable".into(),
                            |v| format!("{}/s", bytes(v as u64))
                        )
                    ),
                ];
                match p.gpu_memory {
                    None => {}
                    Some(GpuMemory::Unreported) => {
                        lines.push("NVIDIA GPU in use (memory not reported)".into());
                    }
                    Some(GpuMemory::Bytes(gpu)) => {
                        lines.push(format!("NVIDIA GPU memory {}", bytes(gpu)));
                    }
                }
                lines.extend(
                    self.scene
                        .popup
                        .iter()
                        .filter(|(owner, _)| *owner == id)
                        .map(|(_, line)| line.clone()),
                );
                let local = snapshot
                    .links
                    .iter()
                    .filter(|(a, b, _)| *a == id || *b == id)
                    .map(|&(_, _, n)| n)
                    .sum::<u32>();
                let outside = snapshot.outside.get(&id).copied().unwrap_or(0);
                if local + outside > 0 {
                    lines.push(format!(
                        "sockets: {local} to local processes, {outside} leaving the machine"
                    ));
                }
                lines.extend(self.scene.notes.get(&id).into_iter().flatten().cloned());
                for line in self.scene.sparklines.get(&id).into_iter().flatten() {
                    braille.push(lines.len());
                    lines.push(line.clone());
                }
                lines.extend([
                    p.group.clone(),
                    p.command.clone(),
                    "f focus subtree | Esc close".into(),
                ]);
                lines
            }
            None => vec![
                format!("pid {}", id.pid),
                "exited / unavailable in this snapshot".into(),
                "Esc close".into(),
            ],
        };
        place_lines(
            lines,
            braille,
            frame.anchor.map(|a| layout.cell_of(a)),
            layout,
            false,
            32,
        )
    }

    /// The label of a system centred on `star` that reaches `reach` world units: just below its
    /// outer edge for a small system, at the star for a large one (an edge more than six rows
    /// below it), when the screen has room for it.
    pub(super) fn title_label(
        &self,
        frame: &Frame,
        layout: &Layout,
        board: &mut Board,
        star: Point,
        reach: f32,
        text: String,
    ) -> Option<Label> {
        let at = frame.locate(&self.camera, star)?;
        let width = text.chars().count() as u16;
        let edge = frame
            .locate(&self.camera, self.camera.front(star, reach))
            .filter(|edge| edge[1] - at[1] < layout.cell_size()[1] * 6.0)
            .unwrap_or(at);
        let (column, row) = layout.cell_of([at[0], edge[1]]);
        let column = column.saturating_sub(width / 2);
        (row + 1 < layout.rows && board.claim(column, row + 1, width, 1)).then_some(Label {
            column,
            row: row + 1,
            text,
            tone: Tone::Bright,
        })
    }

    /// Text drawn over the scene, highest priority first: the inspector popup, the tour
    /// callout (with a pointer drawn into the frame), the hover name, then system and busy labels.
    pub(crate) fn overlay(
        &self,
        frame: &mut Frame,
        layout: &Layout,
        radius: f32,
    ) -> (Vec<Label>, Vec<Popup>) {
        let snapshot = self.snapshot();
        let lookup: HashMap<Identity, &Process> =
            snapshot.processes.iter().map(|p| (p.id, p)).collect();
        let mut board = Board::default();
        let mut labels = Vec::new();
        let mut panels = Vec::new();
        // The open picker covers the inspector popup, which comes back when it closes.
        let popup = if self.picker.is_some() {
            self.picker_popup(layout)
        } else {
            self.popup(frame, layout)
        };
        if let Some(popup) = popup {
            board.reserve(popup.column, popup.row, popup.width, popup.height());
            panels.push(popup);
        }
        if let Some(tour) = &self.tour
            && let Some(p) = lookup.get(&tour.target)
            && let Some(anchor) = self
                .tour_point(tour.target)
                .and_then(|point| frame.locate(&self.camera, point))
        {
            let mut lines = vec![
                p.name.clone(),
                describe(p),
                format!(
                    "CPU {:.1}% | {} {} | pid {}",
                    p.cpu,
                    platform::MEMORY_LABEL,
                    bytes(p.memory),
                    p.id.pid
                ),
            ];
            lines.extend(
                self.scene
                    .callout
                    .iter()
                    .filter(|(owner, _)| *owner == tour.target)
                    .map(|(_, line)| line.clone()),
            );
            if let Some(panel) = place_panel(lines, Some(layout.cell_of(anchor)), layout, true, 24)
            {
                let [w, h] = layout.cell_size();
                let x = anchor[0].clamp(
                    panel.column as f32 * w,
                    (panel.column + panel.width) as f32 * w,
                );
                let y = anchor[1].clamp(
                    panel.row as f32 * h,
                    (panel.row + panel.height()) as f32 * h,
                );
                pointer(frame, [x, y], anchor);
                board.reserve(panel.column, panel.row, panel.width, panel.height());
                panels.push(panel);
            }
        }
        let tag = match self
            .pointer
            .hover
            .and_then(|hover| frame.target_near(hover.frame[0], hover.frame[1], radius))
        {
            Some(Target::Process(id))
                if Some(id) != self.selected
                    && self.tour.as_ref().is_none_or(|tour| tour.target != id) =>
            {
                lookup
                    .get(&id)
                    .map(|p| format!(" {} - {} ", p.name, describe(p)))
            }
            Some(Target::Thing(thing)) if Some(thing) != self.thing => self
                .scene
                .card(self.view, thing, snapshot)
                .map(|card| thing_tag(&card)),
            _ => None,
        };
        if let Some(hover) = self.pointer.hover
            && let Some(text) = tag
        {
            let width = text.chars().count() as u16;
            let column = (hover.cell.0 + 2).min(layout.columns.saturating_sub(width));
            let row = hover.cell.1.saturating_sub(1);
            if board.claim(column, row, width, 1) {
                labels.push(Label {
                    column,
                    row,
                    text,
                    tone: Tone::Tag,
                });
            }
        }
        if !self.labels {
            return (labels, panels);
        }
        for (point, text) in &self.scene.places {
            if let Some(at) = frame.locate(&self.camera, *point) {
                let width = text.chars().count() as u16;
                let (column, row) = layout.cell_of(at);
                let column = column.saturating_sub(width / 2);
                if row < layout.rows && board.claim(column, row, width, 1) {
                    labels.push(Label {
                        column,
                        row,
                        text: text.clone(),
                        tone: Tone::Quiet,
                    });
                }
            }
        }
        for title in &self.scene.titles {
            labels.extend(self.title_label(
                frame,
                layout,
                &mut board,
                title.at,
                title.reach,
                title.text.clone(),
            ));
        }
        let mut busy: Vec<&Process> = lookup
            .values()
            .copied()
            .filter(|p| p.cpu >= 2.0 && self.scene.positions.contains_key(&p.id))
            .collect();
        busy.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        for p in busy.into_iter().take(6) {
            if let Some(at) = self.screen(frame, p.id) {
                let text = format!("{} {:.0}%", label_name(&p.name), p.cpu);
                let (column, row) = layout.cell_of(at);
                if board.claim(column + 2, row, text.chars().count() as u16, 1) {
                    labels.push(Label {
                        column: column + 2,
                        row,
                        text,
                        tone: Tone::Quiet,
                    });
                }
            }
        }
        (labels, panels)
    }
}

/// The hover tag of a thing: like a process's, a name and what it is, here the card's title and
/// its first line.
pub(super) fn thing_tag(card: &Card) -> String {
    match card.lines.first() {
        Some(line) => format!(" {} - {line} ", card.title),
        None => format!(" {} ", card.title),
    }
}

/// Leader line from a callout to its target, with an arrowhead and a ring around the target.
pub(super) fn pointer(frame: &mut Frame, from: [f32; 2], to: [f32; 2]) {
    const COLOR: [u8; 3] = [255, 214, 150];
    let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
    let length = dx.hypot(dy);
    if length > 16.0 {
        let (ux, uy) = (dx / length, dy / length);
        let tip = [to[0] - ux * 12.0, to[1] - uy * 12.0];
        frame.stroke(from, tip, COLOR);
        for side in [-0.45_f32, 0.45] {
            let (s, c) = side.sin_cos();
            let back = [-(ux * c - uy * s) * 9.0, -(ux * s + uy * c) * 9.0];
            frame.stroke(tip, [tip[0] + back[0], tip[1] + back[1]], COLOR);
        }
    }
    frame.circle(to, 10.0, COLOR);
}

#[cfg(test)]
mod tests {

    use crate::app::fixtures::*;

    #[test]
    fn long_process_names_are_cut_in_labels_but_not_in_the_hover_tag() {
        let mut snapshot = model::demo(1.0, 64);
        for process in &mut snapshot.processes {
            process.name = "com.google.BatteriesAvocadoWidgetExtension".into();
            process.cpu = 30.0;
        }
        let hovered = snapshot.processes[0].id;
        let mut app = App::new(view("orbit"), snapshot, all_available());
        let mut frame = app.render(1600, 900, 512);
        frame.rasterize();
        let layout = Layout {
            columns: 200,
            rows: 56,
            width: 1600,
            height: 900,
            cell: None,
        };
        let at = app
            .screen(&frame, hovered)
            .expect("the process is on screen");
        app.pointer.hover = Some(Spot {
            frame: at,
            cell: layout.cell_of(at),
        });
        let (labels, _) = app.overlay(&mut frame, &layout, layout.pick_radius(false));
        let busy: Vec<&Label> = labels.iter().filter(|l| l.tone == Tone::Quiet).collect();
        assert!(!busy.is_empty());
        for label in &busy {
            assert!(label.text.starts_with("com.google.Ba.. "), "{}", label.text);
            assert!(label.text.chars().count() <= 15 + 5, "{}", label.text);
        }
        let tags: Vec<&Label> = labels.iter().filter(|l| l.tone == Tone::Tag).collect();
        assert_eq!(tags.len(), 1, "the hovered process has a tag");
        assert!(
            tags[0].text.contains("WidgetExtension"),
            "the tag keeps the full name"
        );
        for label in labels.iter().filter(|l| l.tone == Tone::Bright) {
            assert!(!label.text.contains("WidgetExtension"), "{}", label.text);
        }
    }

    #[test]
    fn a_hub_is_labelled_as_unreadable_and_explained_only_while_one_is_drawn() {
        let mut snapshot = model::demo(1.0, 20);
        snapshot.links.clear();
        for (index, process) in snapshot.processes.iter_mut().enumerate() {
            process.id = Identity {
                pid: 100 + index as u32,
                start: 1,
            };
            process.parent = 1;
        }
        let plain = snapshot.clone();
        snapshot.shadows = vec![platform::Shadow {
            pid: 1,
            parent: 0,
            name: "launchd".into(),
        }];
        let layout = Layout {
            columns: 200,
            rows: 56,
            width: 1600,
            height: 900,
            cell: None,
        };
        for view in [view("orbit"), view("flow")] {
            let mut app = App::new(view, snapshot.clone(), all_available());
            let mut frame = app.render(1600, 900, 512);
            frame.rasterize();
            let (labels, _) = app.overlay(&mut frame, &layout, layout.pick_radius(false));
            let stars: Vec<&str> = labels
                .iter()
                .filter(|label| label.tone == Tone::Bright)
                .map(|label| label.text.as_str())
                .collect();
            assert_eq!(stars, ["launchd (unreadable) (20)"], "{view:?}");
            let legend = &app.text(0.0, "test", 20, 512)[2];
            assert!(
                legend.ends_with(" | hollow star = a parent isotop can't read (run with sudo)"),
                "{legend}"
            );
            app.focus = Some(snapshot.processes[0].id);
            app.render(1600, 900, 512);
            let legend = &app.text(0.0, "test", 20, 512)[2];
            assert!(!legend.contains("hollow star"), "focused: {legend}");
            let mut app = App::new(view, plain.clone(), all_available());
            app.render(1600, 900, 512);
            let legend = &app.text(0.0, "test", 20, 512)[2];
            assert!(!legend.contains("hollow star"), "{legend}");
        }
    }

    #[test]
    fn titles_from_the_stage_label_the_system_under_its_outer_edge() {
        let (mut app, last) = guided_app();
        let layout = Layout {
            columns: 80,
            rows: 24,
            width: 320,
            height: 180,
            cell: None,
        };
        let mut frame = app.render(320, 180, 512);
        frame.rasterize();
        let (labels, _) = app.overlay(&mut frame, &layout, layout.pick_radius(false));
        let titles: Vec<&Label> = labels
            .iter()
            .filter(|label| label.text == views::testing::GUIDE_TITLE)
            .collect();
        assert_eq!(titles.len(), 1, "{:?}", app.scene.titles);
        assert!(matches!(titles[0].tone, Tone::Bright));
        let seat = frame
            .locate(&app.camera, app.scene.positions[&last])
            .expect("the seat is on screen");
        let (_, row) = layout.cell_of(seat);
        assert!(titles[0].row > row, "just below the star's own row");
        app.labels = false;
        let (labels, _) = app.overlay(&mut frame, &layout, layout.pick_radius(false));
        assert!(labels.is_empty(), "labels off hides titles too");
    }

    #[test]
    fn popup_lines_from_stage_outputs_reach_the_inspector() {
        let (mut app, last) = guided_app();
        let layout = Layout {
            columns: 80,
            rows: 24,
            width: 320,
            height: 180,
            cell: None,
        };
        let popup = |app: &mut App, id: Identity| {
            app.selected = Some(id);
            let frame = app.render(320, 180, 512);
            app.popup(&frame, &layout)
                .expect("the selection has a popup")
                .lines
        };
        let lines = popup(&mut app, last);
        let at = lines
            .iter()
            .position(|line| line == views::testing::GUIDE_LINE)
            .expect("the view's line is in the popup");
        assert!(
            lines[at - 1].starts_with("CPU "),
            "right after the process's own figures: {lines:?}"
        );
        let other = app.snapshot().processes[0].id;
        assert!(
            !popup(&mut app, other)
                .iter()
                .any(|line| line == views::testing::GUIDE_LINE)
        );
    }

    #[test]
    fn sparklines_from_the_stage_reach_the_inspector_cleaned_as_braille() {
        let (mut app, last) = guided_app();
        let layout = Layout {
            columns: 80,
            rows: 24,
            width: 320,
            height: 180,
            cell: None,
        };
        app.selected = Some(last);
        let frame = app.render(320, 180, 512);
        let popup = app
            .popup(&frame, &layout)
            .expect("the selection has a popup");
        let at = popup
            .lines
            .iter()
            .position(|line| line == views::testing::GUIDE_SPARKLINE)
            .expect("the sparkline is in the popup");
        assert_eq!(popup.braille, [at]);
        assert!(
            popup.width >= 44,
            "forty cells and the border fit: {}",
            popup.width
        );
        let mut output = Vec::new();
        crate::terminal::draw_popup(&mut output, &popup).expect("drawn");
        let text = String::from_utf8(output).expect("text");
        assert!(
            text.contains(views::testing::GUIDE_SPARKLINE),
            "braille survives in a sparkline line"
        );
        // The same braille in an ordinary line is cleaned away.
        let mut plain = popup;
        plain.braille.clear();
        let mut output = Vec::new();
        crate::terminal::draw_popup(&mut output, &plain).expect("drawn");
        let text = String::from_utf8(output).expect("text");
        assert!(!text.contains('⣿'));
    }

    #[test]
    fn the_cortex_inspector_lists_the_neuron_and_draws_its_trace_in_braille() {
        let mut app = App::new(view("cortex"), model::demo(12.0, 64), all_available());
        let layout = Layout {
            columns: 120,
            rows: 40,
            width: 320,
            height: 180,
            cell: None,
        };
        app.selected = Some(app.snapshot().processes[3].id);
        for step in 0..40 {
            app.animation = 1.0 + step as f32 * 0.1;
            app.render(320, 180, 512);
        }
        let frame = app.render(320, 180, 512);
        let popup = app
            .popup(&frame, &layout)
            .expect("the selection has a popup");
        assert!(
            popup.lines.iter().any(|line| line.contains(" a b c d = ")),
            "{:?}",
            popup.lines
        );
        assert_eq!(popup.braille.len(), 2, "two lines of trace");
        for &index in &popup.braille {
            let line = &popup.lines[index];
            assert_eq!(line.chars().count(), 40);
            assert!(
                line.chars()
                    .all(|c| c == ' ' || ('\u{2800}'..='\u{28ff}').contains(&c))
            );
        }
        assert!(popup.width >= 44, "forty cells fit: {}", popup.width);
        let legend = app.scene.legend(app.view, app.snapshot());
        assert!(legend.starts_with(" chi "), "{legend}");
        let mut frame = frame;
        let (labels, _) = app.overlay(&mut frame, &layout, 4.0);
        let words: Vec<&str> = labels.iter().map(|label| label.text.as_str()).collect();
        for layer in ["II/III", "IV", "V", "VI"] {
            assert!(words.contains(&layer), "{layer} is labelled: {words:?}");
        }
    }

    #[test]
    fn module_views_learn_the_tour_target_focus_and_limit_and_add_callout_lines() {
        let rows = views::testing::WITNESSED;
        let snapshot = model::demo(12.0, 16);
        let mut app = App::new(View::at(0), snapshot, vec![true; rows.len()]);
        app.scene = Scene::with_rows(rows);
        let layout = Layout {
            columns: 120,
            rows: 40,
            width: 320,
            height: 180,
            cell: None,
        };
        let mut frame = app.render(320, 180, 7);
        let legend = |app: &App| app.scene.legend(app.view, app.snapshot());
        assert_eq!(legend(&app), " witness: toured None | focus None | limit 7");
        let callouts = |app: &App, frame: &mut Frame| -> Vec<String> {
            let (_, panels) = app.overlay(frame, &layout, 4.0);
            panels.into_iter().flat_map(|panel| panel.lines).collect()
        };
        assert!(!callouts(&app, &mut frame).contains(&views::testing::WITNESS_CALLOUT.into()));
        app.key(KeyCode::Char('g'), KeyModifiers::NONE, 10.0);
        let target = app.tour.as_ref().map(|tour| tour.target).expect("a tour");
        let mut frame = app.render(320, 180, 7);
        assert_eq!(
            legend(&app),
            format!(
                " witness: toured Some({}) | focus None | limit 7",
                target.pid
            )
        );
        let lines = callouts(&app, &mut frame);
        let at = lines
            .iter()
            .position(|line| line == views::testing::WITNESS_CALLOUT)
            .expect("the view's line is in the callout");
        assert!(lines[at - 1].starts_with("CPU "), "{lines:?}");
        app.tour = None;
        app.focus = Some(target);
        app.render(320, 180, 7);
        assert_eq!(
            legend(&app),
            format!(
                " witness: toured None | focus Some({}) | limit 7",
                target.pid
            )
        );
    }
}
