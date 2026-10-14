//! Activity Monitor as an installed BifrOSt app: the processes of this machine, with CPU, memory, energy, disk and
//! network readings from `/proc`. The window is `window.rs` and `draw.rs`; the readings are `sample.rs`, and the
//! table's rows and the graph history are `model.rs`. Quitting sends SIGTERM, and only to processes of this user.

#[allow(dead_code)]
mod diagnostics;
#[allow(dead_code)]
mod draw;
#[allow(dead_code)]
mod inspect;
#[allow(dead_code)]
mod job;
#[allow(dead_code)]
mod model;
#[allow(dead_code)]
mod sample;
#[allow(dead_code)]
mod sample_proc;
#[allow(dead_code)]
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::dock_tile;
use bifrost_app::proto::DocErrorCode;
use bifrost_app::windows::{self, WindowListOptions};
use bifrost_app::workspace;
use bifrost_core::json::Json;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use model::{ColumnSet, DockMode, Filter, Tab, UpdateFrequency, SIGNALS};
use std::io::Write;
use std::path::PathBuf;
use window::{
    Monitor, Request, MENU_CLOSE_REPORT, MENU_COLUMN, MENU_DOCK, MENU_COPY, MENU_CPU_HISTORY, MENU_DELTAS, MENU_DIAGNOSTICS, MENU_FILTER, MENU_FIND, MENU_FORCE,
    MENU_FREQ, MENU_INSPECT, MENU_QUIT, MENU_SAMPLE, MENU_SAMPLE_LENGTH, MENU_SAVE, MENU_SIGNAL, MENU_SIGNAL_SHEET, MENU_STOP,
};

/// The preference holding View > Update Frequency, in seconds.
const PREF_FREQUENCY: &str = "updateSeconds";
/// The preference holding the View > Columns choices (`ColumnSet::to_pref`).
const PREF_COLUMNS: &str = "columns";
/// The preference holding View > CPU History.
const PREF_CPU_HISTORY: &str = "cpuHistory";
/// The preference holding View > Dock Icon, as an index into `DockMode::ALL`.
const PREF_DOCK: &str = "dockMode";
/// The preference holding Process > Sample Duration, in seconds.
const PREF_SAMPLE: &str = "sampleSeconds";

/// What a pending save panel will write once the user has chosen a place.
enum Export {
    /// Report text.
    Text(Vec<u8>),
    /// A prepared archive in the app's storage; removed after it is written.
    File(PathBuf),
}

struct ActivityApp {
    monitor: Option<Monitor>,
    /// The save panel asked for, by request id.
    export: Option<(u32, Export)>,
    /// The window list asked for, by request id.
    window_request: Option<u32>,
    /// The read grant asked for the program that Reveal in Files shows, by request id.
    reveal_request: Option<u32>,
}

impl ActivityApp {
    /// Asks the shell for the save panel the window wants, if it wants one.
    fn serve(&mut self, ctx: &Ctx) {
        if let Some(tile) = self.monitor.as_mut().and_then(Monitor::take_dock_update) {
            dock_tile::set_badge(ctx, &tile.badge);
            dock_tile::set_progress(ctx, tile.fraction);
        }
        if self.monitor.as_mut().map_or(false, Monitor::take_window_poll) {
            self.window_request = Some(windows::list(ctx, WindowListOptions::all()));
        }
        // Reveal in Files needs a read grant for the program; the grant comes back as a document event
        if let Some(path) = self.monitor.as_mut().and_then(Monitor::take_reveal) {
            self.reveal_request = Some(ctx.grant_path(&path.to_string_lossy()));
        }
        let Some(request) = self.monitor.as_mut().and_then(Monitor::take_request) else { return };
        // a save still waiting is dropped, and an archive it held is cleaned up
        if let Some((_, Export::File(path))) = self.export.take() {
            let _ = std::fs::remove_file(path);
        }
        self.export = Some(match request {
            Request::SaveReport { name, text } => (ctx.save_document(&name, &["public.text"]), Export::Text(text.into_bytes())),
            Request::SaveBundle(path) => (ctx.save_document("System Diagnostics.zip", &["zip"]), Export::File(path)),
        });
    }

    fn handle_event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(m) = &mut self.monitor else { return false };
        let changed = match ev {
            Input::Touch { kind, x, y, t, .. } => m.touch(&ctx.ui, kind, x, y, t),
            Input::Key(KeyInput::Text(text)) => m.text(&text),
            Input::Key(KeyInput::Code(code)) => m.key(code),
            // a published toolbar control (accessibility, `bifrost pc midget tap-window`): a click at its centre
            Input::Activate(id) => {
                let Some(node) = m.controls(&ctx.ui).into_iter().find(|n| n.id == id && n.enabled) else { return false };
                let x = ctx.ui.pt(node.bounds.x + node.bounds.w / 2.0);
                let y = ctx.ui.pt(node.bounds.y + node.bounds.h / 2.0);
                let t = bifrost_app::proto::sys::now_ms();
                m.touch(&ctx.ui, TouchKind::Down, x, y, t);
                m.touch(&ctx.ui, TouchKind::Up, x, y, t)
            }
            _ => false,
        };
        ctx.wants_keyboard(m.search_focused);
        changed
    }

    fn run_menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(m) = &mut self.monitor else { return false };
        match id {
            MENU_QUIT => {
                m.quit_selected();
                true
            }
            MENU_FORCE => {
                m.force_quit_selected();
                true
            }
            MENU_INSPECT => {
                m.toggle_inspector();
                true
            }
            MENU_FIND => {
                m.focus_search();
                ctx.wants_keyboard(true);
                true
            }
            MENU_DELTAS => {
                m.toggle_deltas();
                true
            }
            n if (MENU_DOCK..MENU_DOCK + DockMode::ALL.len() as u32).contains(&n) => {
                let mode = DockMode::ALL[(n - MENU_DOCK) as usize];
                m.set_dock_mode(mode);
                ctx.defaults_mut().set_int(PREF_DOCK, mode.index());
                true
            }
            MENU_SIGNAL_SHEET => {
                m.open_signal_sheet();
                true
            }
            MENU_SAMPLE => {
                m.start_sample();
                true
            }
            n if (MENU_SAMPLE_LENGTH..MENU_SAMPLE_LENGTH + sample_proc::DURATIONS.len() as u32).contains(&n) => {
                let seconds = sample_proc::DURATIONS[(n - MENU_SAMPLE_LENGTH) as usize];
                if m.set_sample_seconds(seconds) {
                    ctx.defaults_mut().set_int(PREF_SAMPLE, i64::from(seconds));
                }
                true
            }
            MENU_DIAGNOSTICS => {
                m.start_diagnostics();
                true
            }
            MENU_STOP => {
                m.stop_job();
                true
            }
            MENU_SAVE => {
                m.save_report();
                true
            }
            MENU_CLOSE_REPORT => {
                m.close_report();
                true
            }
            MENU_CPU_HISTORY => {
                let on = !m.cpu_history;
                m.set_cpu_history(on);
                ctx.defaults_mut().set_bool(PREF_CPU_HISTORY, on);
                true
            }
            n if (MENU_COLUMN..MENU_COLUMN + 32).contains(&n) => {
                if let Some(pref) = m.toggle_column((n - MENU_COLUMN) as usize) {
                    ctx.defaults_mut().set_string(PREF_COLUMNS, &pref);
                }
                true
            }
            MENU_COPY => {
                if let Some(text) = m.copy_text() {
                    ctx.set_pasteboard_string(&text);
                }
                true
            }
            n if (MENU_SIGNAL..MENU_SIGNAL + SIGNALS.len() as u32).contains(&n) => {
                m.send_signal(SIGNALS[(n - MENU_SIGNAL) as usize]);
                true
            }
            n if (MENU_FILTER..MENU_FILTER + Filter::ALL.len() as u32).contains(&n) => {
                m.set_filter(Filter::ALL[(n - MENU_FILTER) as usize]);
                true
            }
            n if (MENU_FREQ..MENU_FREQ + UpdateFrequency::ALL.len() as u32).contains(&n) => {
                let f = UpdateFrequency::ALL[(n - MENU_FREQ) as usize];
                m.set_frequency(f);
                ctx.defaults_mut().set_int(PREF_FREQUENCY, f.seconds());
                true
            }
            _ => false,
        }
    }
}

impl App for ActivityApp {
    fn id(&self) -> &str {
        "is.olibuijr.activity.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state
        let (w, h) = (ctx.cfg.w as f32, ctx.cfg.h as f32);
        match &mut self.monitor {
            Some(m) => m.resize(w, h),
            None => {
                let mut m = Monitor::new(w, h);
                m.data_dir = ctx.data_dir.clone();
                m.load_averages();
                m.set_frequency(UpdateFrequency::from_seconds(ctx.defaults().int(PREF_FREQUENCY)));
                m.set_columns(ColumnSet::from_pref(&ctx.defaults().string(PREF_COLUMNS).unwrap_or_default()));
                m.set_cpu_history(ctx.defaults().bool(PREF_CPU_HISTORY));
                m.set_dock_mode(DockMode::from_index(ctx.defaults().int(PREF_DOCK)));
                // a stored length that is not offered (or none yet) keeps the default
                m.set_sample_seconds(u32::try_from(ctx.defaults().int(PREF_SAMPLE)).unwrap_or(0));
                self.monitor = Some(m);
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let changed = self.handle_event(ctx, ev);
        self.serve(ctx);
        changed
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let changed = self.monitor.as_mut().map_or(false, |m| m.tick(t));
        self.serve(ctx);
        changed
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::SaveTo(mut doc) => match self.export.take() {
                Some((request, export)) if request == doc.request => {
                    let bytes = match &export {
                        Export::Text(bytes) => Ok(bytes.clone()),
                        Export::File(path) => std::fs::read(path),
                    };
                    match bytes.and_then(|b| doc.file.write_all(&b)) {
                        Ok(()) => ctx.commit_document(request),
                        Err(e) => {
                            ctx.log(&format!("save: {e}"));
                            ctx.abort_document(request);
                            if let Some(m) = &mut self.monitor {
                                m.status = "Could not save the file".into();
                            }
                        }
                    }
                    if let Export::File(path) = export {
                        let _ = std::fs::remove_file(path);
                    }
                    true
                }
                other => {
                    self.export = other;
                    false
                }
            },
            DocumentEvent::Saved(doc) => {
                if let Some(m) = &mut self.monitor {
                    m.status = format!("Saved {}", doc.name);
                }
                true
            }
            DocumentEvent::Opened(doc) if Some(doc.request) == self.reveal_request => {
                self.reveal_request = None;
                let grant = doc.grant;
                workspace::activate_file_viewer_selecting(ctx, &[grant], |_| {});
                // the reveal is sent before the release, so the shell still has the grant when it reads the request
                ctx.release_document(grant);
                true
            }
            DocumentEvent::Failed { request, code } if Some(request) == self.reveal_request => {
                self.reveal_request = None;
                if let Some(m) = &mut self.monitor {
                    m.status = "Could not show the program in Files".into();
                }
                if code != DocErrorCode::Cancelled {
                    ctx.log(&format!("reveal: {code:?}"));
                }
                true
            }
            DocumentEvent::Failed { request, code } => {
                if self.export.as_ref().map_or(false, |(r, _)| *r == request) {
                    if let Some((_, Export::File(path))) = self.export.take() {
                        let _ = std::fs::remove_file(path);
                    }
                }
                if code != DocErrorCode::Cancelled {
                    ctx.log(&format!("document: {code:?}"));
                }
                false
            }
            _ => false,
        }
    }

    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        // the replies to the Dock calls need nothing; taking them keeps the mailbox empty
        if self.window_request != Some(request) {
            return true;
        }
        self.window_request = None;
        let Some(m) = &mut self.monitor else { return true };
        match windows::decode_list(result) {
            // a window with no owner process (a native client) cannot be matched to a row
            Ok(list) => m.set_window_owners(list.iter().filter(|w| w.on_screen && w.owner_pid != 0).map(|w| w.owner_pid).collect()),
            Err(_) => m.status = "Could not read the window list".into(),
        }
        true
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.monitor.as_ref().map(|m| m.next_wake(t))
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        self.monitor.as_ref().map_or_else(Vec::new, |m| m.controls(&ctx.ui))
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let Some(m) = &self.monitor else { return Vec::new() };
        let can_signal = m.can_quit_selected();
        let mut process = vec![
            AppMenuItem::new(MENU_QUIT, "Quit Process").enabled(can_signal),
            AppMenuItem::new(MENU_FORCE, "Force Quit Process").enabled(can_signal),
            AppMenuItem::new(MENU_SIGNAL_SHEET, "Send Signal...").enabled(can_signal),
            AppMenuItem::submenu("Send Signal to Process").enabled(can_signal),
        ];
        for (i, sig) in SIGNALS.iter().enumerate() {
            process.push(AppMenuItem::new(MENU_SIGNAL + i as u32, &format!("SIG{}", sig.name)).enabled(can_signal).depth(1));
        }
        process.push(AppMenuItem::separator());
        process.push(AppMenuItem::new(MENU_DELTAS, "Show Deltas for Process").checked(m.deltas_on_selected()).enabled(m.has_selected()));
        process.push(AppMenuItem::new(MENU_INSPECT, "Inspect Process").enabled(m.has_selected()));
        process.push(AppMenuItem::new(MENU_SAMPLE, "Sample Process").enabled(m.has_selected() && !m.job_running()));
        process.push(AppMenuItem::submenu("Sample Duration"));
        for (i, secs) in sample_proc::DURATIONS.iter().enumerate() {
            process.push(AppMenuItem::new(MENU_SAMPLE_LENGTH + i as u32, &format!("{secs} seconds")).checked(m.sample_seconds == *secs).depth(1));
        }
        process.push(AppMenuItem::new(MENU_STOP, "Stop Background Task").key("cmd+.").enabled(m.job_running()));
        process.push(AppMenuItem::new(MENU_FIND, "Find Process").key("cmd+f"));
        process.push(AppMenuItem::new(MENU_COPY, "Copy").key("cmd+c").enabled(m.has_selected()));

        let mut view = vec![AppMenuItem::submenu("Filter Processes")];
        for (i, f) in Filter::ALL.iter().enumerate() {
            view.push(AppMenuItem::new(MENU_FILTER + i as u32, f.label()).checked(m.filter == *f).depth(1));
        }
        view.push(AppMenuItem::submenu("Columns"));
        for (i, col) in m.tab.all_columns().iter().enumerate() {
            view.push(AppMenuItem::new(MENU_COLUMN + i as u32, col.title).checked(m.columns.shown(m.tab, col)).enabled(!col.required).depth(1));
        }
        view.push(AppMenuItem::submenu("Update Frequency"));
        for (i, f) in UpdateFrequency::ALL.iter().enumerate() {
            view.push(AppMenuItem::new(MENU_FREQ + i as u32, f.label()).checked(m.frequency == *f).depth(1));
        }
        view.push(AppMenuItem::submenu("Dock Icon"));
        for (i, mode) in DockMode::ALL.iter().enumerate() {
            view.push(AppMenuItem::new(MENU_DOCK + i as u32, mode.label()).checked(m.dock_mode == *mode).depth(1));
        }
        view.push(AppMenuItem::new(MENU_CPU_HISTORY, "CPU History").checked(m.cpu_history).enabled(m.tab == Tab::Cpu));
        view.push(AppMenuItem::separator());
        view.push(AppMenuItem::new(MENU_DIAGNOSTICS, "Run System Diagnostics...").enabled(!m.job_running()));
        let has_report = m.report.is_some();
        vec![
            AppMenu {
                title: "File".into(),
                items: vec![
                    AppMenuItem::new(MENU_SAVE, "Save Report...").key("cmd+s").enabled(has_report),
                    AppMenuItem::new(MENU_CLOSE_REPORT, "Close Report").enabled(has_report),
                ],
            },
            AppMenu { title: "View".into(), items: view },
            AppMenu { title: "Process".into(), items: process },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let changed = self.run_menu_action(ctx, id);
        self.serve(ctx);
        changed
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        if let Some(m) = &mut self.monitor {
            m.draw(&ctx.ui, s);
        }
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ActivityApp { monitor: None, export: None, window_request: None, reveal_request: None });
}
