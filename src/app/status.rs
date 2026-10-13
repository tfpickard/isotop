use crate::keys::{self, Context, Expand, Line};
use crate::model::{Snapshot, bytes};
use crate::platform::{self, Source};
use crate::render::Links;

use super::App;
use super::tour::Tour;

impl App {
    pub(crate) fn text(
        &self,
        render_ms: f32,
        transport: &str,
        fps: u32,
        limit: usize,
    ) -> Vec<String> {
        let s = self.snapshot();
        let row = self.row();
        let (visible, collapsed) = row
            .header
            .map_or((self.scene.visible, self.scene.collapsed), |header| {
                header(&self.scene)
            });
        let mode = match (self.history.is_paused(), &self.tour) {
            (true, _) => "PAUSED",
            (false, Some(Tour { running: true, .. })) => "TOUR",
            (false, _) => "LIVE",
        };
        let mut lines = vec![format!(
            " ISOTOP / {} / {mode}   {} processes{}{} | {visible} visible | {collapsed} collapsed | {:.1}/{} CPU cores | RAM {} / {}",
            row.title,
            s.processes.len(),
            unreadable_text(s),
            sampler_text(s),
            s.processes.iter().map(|p| p.cpu).sum::<f32>() / 100.0,
            s.cores,
            known_bytes(
                s.memory_total
                    .zip(s.memory_available)
                    .map(|(total, available)| total.saturating_sub(available))
            ),
            known_bytes(s.memory_total)
        )];
        let expand = Expand {
            random: if self.keyboard { "Shift-Space" } else { "*" },
            turn: row.turn.is_some(),
        };
        let inspector = if let Some(line) = self.scene.inspector_line(self.view) {
            line
        } else if let Some(id) = self.inspected() {
            if let Some(p) = s.processes.iter().find(|p| p.id == id) {
                format!(
                    " {} [pid {} / parent {} / {}] CPU {:.1}%  {} {}  IO {}  {}",
                    p.name,
                    id.pid,
                    p.parent,
                    p.state,
                    p.cpu,
                    platform::MEMORY_LABEL,
                    bytes(p.memory),
                    p.io_rate.map_or_else(
                        || "unavailable".into(),
                        |v| format!("{}/s", bytes(v as u64))
                    ),
                    p.group
                )
            } else {
                format!(" pid {} exited / unavailable in this snapshot", id.pid)
            }
        } else {
            " Click or hover a building/body to inspect; / searches name, command or PID".into()
        };
        lines.push(inspector);
        lines.push(
            self.inspected()
                .and_then(|id| s.processes.iter().find(|p| p.id == id))
                .map_or_else(
                    || self.scene.legend(self.view, s),
                    |p| format!(" {}", p.command),
                ),
        );
        if let Some(query) = self.search.query() {
            lines.push(format!(
                " /{query}_  {} matches | {}",
                self.search.found(),
                keys::line(Context::Search, Line::Prompt, expand)
            ));
        } else if self.picker.is_some() {
            lines.push(format!(
                " {}",
                keys::line(Context::Picker, Line::Prompt, expand)
            ));
        } else if self.show_help {
            // The shown view's own keys lead, so they survive a narrow terminal.
            let own = match row.keys {
                "" => String::new(),
                own => format!(" {own} |"),
            };
            lines.push(format!(
                "{own} {}",
                keys::line(Context::Global, Line::Keys, expand)
            ));
        } else {
            lines.push(format!(
                " {}",
                keys::line(Context::Global, Line::Brief, expand)
            ));
        }
        let links = match self.scene.links {
            Links::All => "",
            Links::Focused => " | links: focused",
            Links::Off => " | links: off",
        };
        lines.push(if self.show_help {
            format!(" {}", keys::line(Context::Global, Line::More, expand))
        } else { format!(" {render_ms:.1}ms {} | {transport} | target {fps}fps | {} samples | t={:.1}s | cap {limit} | {}{links}{}",
            self.renderer.kind(), self.history.len(), s.elapsed, pressure_text(s),
            if self.focus.is_some() { " | SUBTREE FOCUS" } else { "" }) });
        lines
    }
}

/// A byte count for the status line, `n/a` when it was not measured.
pub(super) fn known_bytes(value: Option<u64>) -> String {
    value.map_or_else(|| "n/a".to_owned(), bytes)
}

/// The pressure readings for the status line, `n/a` for each one the platform cannot measure.
pub(super) fn pressure_text(snapshot: &Snapshot) -> String {
    let [cpu, memory, io] = snapshot.pressure;
    let reading = |source: Source, value: f32| {
        if snapshot.missing.contains(source) {
            "n/a".to_owned()
        } else {
            format!("{value:.0}%")
        }
    };
    format!(
        "pressure cpu {} mem {} io {}",
        reading(Source::CpuPressure, cpu),
        reading(Source::MemoryPressure, memory),
        reading(Source::IoPressure, io)
    )
}

/// A short note beside the process count for processes of other users that could not be
/// measured at all. It sits on the first status line, which is the one least likely to be
/// clipped on a narrow terminal.
pub(super) fn unreadable_text(snapshot: &Snapshot) -> String {
    match snapshot.unreadable {
        0 => String::new(),
        count => format!(" (+{count} unreadable: run with sudo)"),
    }
}

/// A short note beside the process count when the background sampler has stopped or stalled:
/// sockets, GPU memory, cgroups, open files and locks then keep what it measured last.
pub(super) fn sampler_text(snapshot: &Snapshot) -> String {
    let sampler = &snapshot.sampler;
    if !sampler.running {
        " (background sampler stopped: links, files and cgroups frozen)".into()
    } else if sampler.stalled() {
        let seconds = sampler.quiet.as_secs();
        format!(" (background sampler stalled {seconds}s)")
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {

    use crate::app::fixtures::*;

    #[test]
    fn the_help_names_the_random_visit_key_the_terminal_can_send() {
        let mut app = visiting_app();
        app.show_help = true;
        let help = |app: &App| app.text(0.0, "test", 20, 512)[3].clone();
        assert!(
            help(&app).contains("Space next process | * random"),
            "{}",
            help(&app)
        );
        app.keyboard = true;
        assert!(
            help(&app).contains("| Shift-Space random"),
            "{}",
            help(&app)
        );
    }

    #[test]
    fn memory_is_labelled_with_what_the_platform_measures() {
        let snapshot = model::demo(1.0, 16);
        let selected = snapshot.processes[0].id;
        let mut app = App::new(view("city"), snapshot, all_available());
        let label = platform::MEMORY_LABEL;
        assert!(app.text(0.0, "test", 20, 512)[2].contains(&format!("footprint = {label} |")));
        app.selected = Some(selected);
        let text = app.text(0.0, "test", 20, 512);
        assert!(text[1].contains(&format!("%  {label} ")), "{}", text[1]);
        let frame = app.render(320, 180, 512);
        let layout = Layout {
            columns: 120,
            rows: 40,
            width: 320,
            height: 180,
            cell: None,
        };
        let popup = app
            .popup(&frame, &layout)
            .expect("a popup for the selection");
        assert!(
            popup
                .lines
                .iter()
                .any(|line| line.contains(&format!("| {label} "))),
            "{:?}",
            popup.lines
        );
        // macOS reports the physical footprint, which is not a resident set size.
        if cfg!(target_os = "macos") {
            assert!(!text[1].contains("RSS") && popup.lines.iter().all(|l| !l.contains("RSS")));
        }
    }

    /// The macOS gaps, as the collector reports them on a Mac without cluster clocks.
    const MACOS: &[Source] = &[
        Source::CpuPressure,
        Source::IoPressure,
        Source::LastCpu,
        Source::CpuClock,
        Source::RunQueue,
        Source::Cgroups,
        Source::SocketTraffic,
        Source::FileLocks,
        Source::GpuMemory,
        Source::MemoryDetail,
    ];

    /// The status text of `view` over the demo workload with the given gaps, after one frame so
    /// views that learn the gaps while drawing have seen them.
    fn lines_with(view: View, missing: &[Source], unreadable: usize) -> Vec<String> {
        let mut snapshot = model::demo(1.0, 16);
        snapshot.missing = missing.iter().copied().collect();
        snapshot.unreadable = unreadable;
        if missing.contains(&Source::LastCpu) {
            // A Mac without the rusage counters that place marbles by cluster: no share of
            // performance-core time and no waiting threads.
            for process in &mut snapshot.processes {
                process.performance_share = None;
                process.waiting = None;
            }
        }
        if missing.contains(&Source::FileLocks) {
            // No lock table: every process's locks are unreadable and none waits on one.
            for process in &mut snapshot.processes {
                process.locks_held = model::Measured::Unreadable;
                process.blocked_on = None;
            }
            snapshot.unattributed_locks = 0;
        }
        let mut app = App::new(view, snapshot, all_available());
        app.render(320, 180, 512);
        app.text(0.0, "test", 20, 512)
    }

    #[test]
    fn status_line_shows_n_a_for_memory_it_could_not_read() {
        let status = |total: Option<u64>, available: Option<u64>| {
            let mut snapshot = model::demo(1.0, 16);
            (snapshot.memory_total, snapshot.memory_available) = (total, available);
            let mut app = App::new(view("city"), snapshot, all_available());
            app.render(320, 180, 512);
            app.text(0.0, "test", 20, 512).swap_remove(0)
        };
        let line = status(Some(3 << 30), Some(1 << 30));
        assert!(
            line.ends_with(&format!("RAM {} / {}", bytes(2 << 30), bytes(3 << 30))),
            "{line}"
        );
        let line = status(Some(3 << 30), None);
        assert!(
            line.ends_with(&format!("RAM n/a / {}", bytes(3 << 30))),
            "{line}"
        );
        let line = status(None, None);
        assert!(line.ends_with("RAM n/a / n/a"), "{line}");
    }

    #[test]
    fn status_line_shows_n_a_for_each_missing_pressure() {
        let [cpu, memory, io] = model::demo(1.0, 16)
            .pressure
            .map(|value| format!("{value:.0}%"));
        let full = lines_with(view("city"), &[], 0);
        assert!(
            full[4].contains(&format!("pressure cpu {cpu} mem {memory} io {io}")),
            "{}",
            full[4]
        );
        let mac = lines_with(view("city"), MACOS, 0);
        assert!(
            mac[4].contains(&format!("pressure cpu n/a mem {memory} io n/a")),
            "{}",
            mac[4]
        );
        let none = lines_with(
            view("city"),
            &[
                Source::CpuPressure,
                Source::MemoryPressure,
                Source::IoPressure,
            ],
            0,
        );
        assert!(
            none[4].contains("pressure cpu n/a mem n/a io n/a"),
            "{}",
            none[4]
        );
    }

    #[test]
    fn status_line_says_when_the_background_sampler_stopped_or_stalled() {
        let status = |sampler: model::SamplerHealth| {
            let mut snapshot = model::demo(1.0, 16);
            snapshot.sampler = sampler;
            App::new(view("city"), snapshot, all_available()).text(0.0, "test", 20, 512)[0].clone()
        };
        let healthy = status(model::demo(1.0, 16).sampler);
        assert!(!healthy.contains("sampler"), "{healthy}");
        let stopped = status(model::SamplerHealth::default());
        assert!(
            stopped.contains(" processes (background sampler stopped: "),
            "{stopped}"
        );
        let stalled = status(model::SamplerHealth {
            running: true,
            last_cycle: Some(Duration::from_secs(95)),
            quiet: Duration::from_secs(95),
            scan_latency: Vec::new(),
        });
        assert!(
            stalled.contains("(background sampler stalled 95s)"),
            "{stalled}"
        );
    }

    #[test]
    fn status_line_counts_unreadable_processes_only_when_there_are_some() {
        let none = lines_with(view("city"), &[], 0);
        assert!(!none[0].contains("unreadable") && !none[4].contains("unreadable"));
        let one = lines_with(view("city"), &[], 1);
        assert!(
            one[0].contains(" processes (+1 unreadable: run with sudo) | "),
            "{}",
            one[0]
        );
        let many = lines_with(view("city"), MACOS, 37);
        assert!(
            many[0].contains(" processes (+37 unreadable: run with sudo) | "),
            "{}",
            many[0]
        );
        assert!(!many[4].contains("unreadable"), "{}", many[4]);
    }

    #[test]
    fn unreadable_notice_survives_an_80_column_terminal() {
        let first = &lines_with(view("city"), MACOS, 212)[0];
        let visible: String = first.chars().take(80).collect();
        assert!(
            visible.contains("(+212 unreadable: run with sudo)"),
            "{visible}"
        );
    }

    #[test]
    fn legends_name_what_the_platform_cannot_measure() {
        let cores = lines_with(view("cores"), MACOS, 0)[2].clone();
        assert!(
            cores.contains("lanes = load; macOS reports no last CPU per process, so no marbles"),
            "{cores}"
        );
        assert!(cores.contains("no clock readings"), "{cores}");
        assert!(!cores.contains("marble ="), "{cores}");
        let cells = lines_with(view("cells"), MACOS, 0)[2].clone();
        assert!(
            cells.contains(&format!(
                "{}, so no limits, quotas or pressure",
                platform::WITHOUT_CGROUPS
            )),
            "{cells}"
        );
        let orbit = lines_with(view("orbit"), MACOS, 0)[2].clone();
        assert!(
            orbit.contains("glow + trail = CPU | cyan arcs") && !orbit.contains("NVIDIA"),
            "{orbit}"
        );
        let globe = lines_with(view("globe"), MACOS, 0)[2].clone();
        assert!(
            globe.contains("no per-connection rates or RTT on macOS"),
            "{globe}"
        );
        let coop = lines_with(view("coop"), MACOS, 0)[2].clone();
        assert!(
            coop.contains("no last CPU: running chickens share one trough"),
            "{coop}"
        );
        assert!(coop.contains("no CPU pressure"), "{coop}");
    }

    #[test]
    fn coop_legend_names_app_groups_and_drops_foxes_without_cgroups() {
        let mac = lines_with(view("coop"), MACOS, 0)[2].clone();
        assert!(
            mac.contains(&format!("flock = {} | ", platform::GROUP_LABEL)),
            "{mac}"
        );
        assert!(
            mac.contains("dust = reads | eyes = memory pressure"),
            "{mac}"
        );
        assert!(mac.contains("no cgroups: no OOM foxes"), "{mac}");
        assert!(
            !mac.contains("flock = cgroup") && !mac.contains("fox ="),
            "{mac}"
        );
        let linux = lines_with(view("coop"), &[], 0)[2].clone();
        assert!(
            linux.contains("flock = cgroup | ") && linux.contains("fox = OOM kill, eyes = "),
            "{linux}"
        );
        assert!(!linux.contains("no cgroups"), "{linux}");
    }

    #[test]
    fn coop_legend_on_macos_counts_open_files_but_has_no_brooding() {
        let mac = lines_with(view("coop"), MACOS, 0)[2].clone();
        assert!(
            mac.contains("eggs = open files (log2), cracked = deleted but open | chicks = threads"),
            "{mac}"
        );
        assert!(
            mac.contains("no file lock table: no brooding or queueing"),
            "{mac}"
        );
        assert!(
            !mac.contains("brooding =") && !mac.contains("file locks unreadable"),
            "{mac}"
        );
        assert!(!mac.contains("without a process (OFD)"), "{mac}");
        let linux = lines_with(view("coop"), &[], 0)[2].clone();
        assert!(
            linux.contains("brooding = holds a file lock, queue at a nest = blocked on one"),
            "{linux}"
        );
        assert!(!linux.contains("no file lock table"), "{linux}");
    }

    #[test]
    fn legends_keep_their_full_text_when_nothing_is_missing() {
        let text = |view| lines_with(view, &[], 0)[2].clone();
        assert!(
            text(view("cores")).contains("chevrons = clock | red queue = waiting tasks | marble =")
        );
        assert!(text(view("cells")).contains("dashed ring = memory limit | arc = CPU vs quota"));
        assert!(text(view("globe")).contains("brighter with traffic | cyan = mostly download"));
        assert!(
            text(view("orbit")).contains("glow + trail = CPU | green = NVIDIA GPU | cyan arcs")
        );
        assert!(!text(view("coop")).contains("share one trough"));
    }

    #[test]
    fn the_help_line_leads_with_the_shown_views_keys() {
        let rows = views::testing::WITNESSED;
        let mut app = App::new(View::at(0), model::demo(12.0, 16), vec![true; rows.len()]);
        app.scene = Scene::with_rows(rows);
        app.show_help = true;
        let help = |app: &App| app.text(1.0, "test", 10, 512)[3].clone();
        assert!(
            help(&app).starts_with(" m witness mode | Arrows/WASD pan"),
            "{}",
            help(&app)
        );
        app.switch(View::at(1));
        assert!(help(&app).starts_with(" Arrows/WASD pan"), "{}", help(&app));
    }

    #[test]
    fn globe_hints_say_scroll_turns_and_drag_pans() {
        let mut globe = App::new(view("globe"), model::demo(1.0, 64), all_available());
        let mut city = App::new(view("city"), model::demo(1.0, 64), all_available());
        let line = |app: &App, index: usize| app.text(0.0, "test", 20, 512)[index].clone();
        assert!(line(&globe, 3).contains(" | scroll turn/tilt | drag pan | Ctrl-scroll zoom"));
        assert!(!line(&globe, 3).contains("scroll pan"));
        assert!(line(&city, 3).contains(" | scroll pan | Ctrl-scroll zoom"));
        globe.show_help = true;
        city.show_help = true;
        assert!(line(&globe, 4).starts_with(" Scroll turn/tilt | drag pan | Ctrl-scroll zoom"));
        assert!(line(&city, 4).starts_with(" Scroll pan | Ctrl-scroll zoom"));
    }
}
