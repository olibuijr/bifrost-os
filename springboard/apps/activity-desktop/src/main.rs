//! Activity Monitor as an installed BifrOSt app: the processes of this machine, with CPU, GPU, memory, energy, disk and
//! network readings from `/proc`. The window is `window.rs` and `draw.rs`; the readings are `sample.rs`, and the
//! table's rows and the graph history are `model.rs`. Quitting sends SIGTERM, and only to processes of this user.
//! `inspect.rs` reads one process, `sample_proc.rs` samples where its threads wait, and `diagnostics.rs` collects the
//! machine's state into an archive.

#[allow(dead_code)]
mod diagnostics;
#[allow(dead_code)]
mod draw;
#[allow(dead_code)]
mod inspect;
#[allow(dead_code)]
mod model;
#[allow(dead_code)]
mod sample;
#[allow(dead_code)]
mod sample_proc;
#[allow(dead_code)]
mod window;

use bifrost_app::controls::Node;
use bifrost_app::dock_tile;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::proto::DocErrorCode;
use bifrost_app::windows::{self, WindowListOptions};
use bifrost_app::workspace;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use model::{DockIcon, Filter, UpdateFrequency, SIGNALS};
use std::collections::HashMap;
use std::io::Write;
use window::{
    Effect, Monitor, Overlay, MENU_COLUMN, MENU_COPY, MENU_CPU_HISTORY, MENU_CPU_USAGE, MENU_DELTAS, MENU_DIAGNOSTICS, MENU_DOCK, MENU_EXTRA, MENU_FILTER, MENU_FIND, MENU_FORCE,
    MENU_FREQ, MENU_GPU_HISTORY, MENU_INSPECT, MENU_QUIT, MENU_SAMPLE, MENU_SIGNAL, MENU_SIGNAL_SHEET,
};

/// The preference holding View > Update Frequency, in seconds.
const PREF_FREQUENCY: &str = "updateSeconds";
/// The preference holding View > Columns, as a mask of hidden default columns.
const PREF_COLUMNS: &str = "hiddenColumns";
/// The preference holding View > Columns, as a mask of added columns.
const PREF_EXTRA_COLUMNS: &str = "shownColumns";
/// The preference holding View > Dock Icon, as an index into `DockIcon::ALL`.
const PREF_DOCK: &str = "dockIcon";

struct ActivityApp {
    monitor: Option<Monitor>,
    /// Files waiting for the save panel's answer, by request id.
    saves: HashMap<u32, Vec<u8>>,
    /// Requests for a grant on a file that Reveal in Files will show.
    reveals: Vec<u32>,
    /// The window-list request in flight.
    window_request: Option<u32>,
}

impl ActivityApp {
    /// Carries out what the window queued: the things only the shell can do.
    fn run_effects(&mut self, ctx: &Ctx) {
        let Some(m) = &mut self.monitor else { return };
        for effect in m.take_effects() {
            match effect {
                Effect::SaveFile { name, bytes } => {
                    let ext = name.rsplit('.').next().unwrap_or("txt").to_string();
                    let request = ctx.save_document(&name, &[ext.as_str()]);
                    self.saves.insert(request, bytes);
                }
                Effect::CopyText(text) => ctx.set_pasteboard_string(&text),
                Effect::Reveal(path) => {
                    let request = ctx.grant_path(&path);
                    self.reveals.push(request);
                }
                Effect::ListWindows => {
                    if self.window_request.is_none() {
                        self.window_request = Some(windows::list(ctx, WindowListOptions::all()));
                    }
                }
                Effect::Dock { badge, progress } => {
                    dock_tile::set_badge(ctx, &badge);
                    dock_tile::set_progress(ctx, progress);
                }
            }
        }
    }

    fn set_status(&mut self, text: &str) {
        if let Some(m) = &mut self.monitor {
            m.status = text.into();
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
                m.set_frequency(UpdateFrequency::from_seconds(ctx.defaults().int(PREF_FREQUENCY)));
                m.hidden_columns = ctx.defaults().int(PREF_COLUMNS);
                m.extra_columns = ctx.defaults().int(PREF_EXTRA_COLUMNS);
                m.dock = DockIcon::from_index(ctx.defaults().int(PREF_DOCK));
                self.monitor = Some(m);
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(m) = &mut self.monitor else { return false };
        let changed = match ev {
            Input::Touch { kind, x, y, t, .. } => m.touch(&ctx.ui, kind, x, y, t),
            Input::Key(KeyInput::Text(text)) => m.text(&text),
            Input::Key(KeyInput::Code(code)) => m.key(&ctx.ui, code),
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
        ctx.wants_keyboard(m.search_focused && m.overlay.is_none());
        self.run_effects(ctx);
        changed
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let changed = self.monitor.as_mut().map_or(false, |m| m.tick(t));
        self.run_effects(ctx);
        changed
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
            AppMenuItem::new(MENU_QUIT, "Quit Process").key("cmd+alt+q").enabled(can_signal),
            AppMenuItem::new(MENU_FORCE, "Force Quit Process").enabled(can_signal),
            AppMenuItem::new(MENU_SIGNAL_SHEET, "Send Signal to Process...").enabled(can_signal),
            AppMenuItem::submenu("Send Signal to Process").enabled(can_signal),
        ];
        for (i, sig) in SIGNALS.iter().enumerate() {
            process.push(AppMenuItem::new(MENU_SIGNAL + i as u32, &format!("SIG{}", sig.name)).enabled(can_signal).depth(1));
        }
        process.push(AppMenuItem::separator());
        process.push(AppMenuItem::new(MENU_INSPECT, "Inspect Process").key("cmd+i").enabled(m.has_selected()));
        process.push(AppMenuItem::new(MENU_SAMPLE, "Sample Process").enabled(m.has_selected()));
        process.push(AppMenuItem::new(MENU_FIND, "Find Process").key("cmd+f"));

        let mut view = vec![AppMenuItem::submenu("Filter Processes")];
        for (i, f) in Filter::ALL.iter().enumerate() {
            let usable = *f != Filter::Selected || m.has_selected() || !m.pinned.is_empty();
            view.push(AppMenuItem::new(MENU_FILTER + i as u32, f.label()).checked(m.filter == *f).enabled(usable).depth(1));
        }
        view.push(AppMenuItem::submenu("Update Frequency"));
        for (i, f) in UpdateFrequency::ALL.iter().enumerate() {
            view.push(AppMenuItem::new(MENU_FREQ + i as u32, f.label()).checked(m.frequency == *f).depth(1));
        }
        view.push(AppMenuItem::submenu("Columns"));
        for (i, (_, title)) in model::OPTIONAL_COLUMNS.iter().enumerate() {
            view.push(AppMenuItem::new(MENU_COLUMN + i as u32, title).checked(m.column_shown(i)).depth(1));
        }
        for (i, (_, title, _)) in model::EXTRA_COLUMNS.iter().enumerate() {
            view.push(AppMenuItem::new(MENU_EXTRA + i as u32, title).checked(m.extra_shown(i)).depth(1));
        }
        view.push(AppMenuItem::submenu("Dock Icon"));
        for (i, d) in DockIcon::ALL.iter().enumerate() {
            view.push(AppMenuItem::new(MENU_DOCK + i as u32, d.label()).checked(m.dock == *d).depth(1));
        }
        view.push(AppMenuItem::separator());
        view.push(AppMenuItem::new(MENU_DELTAS, "Show Deltas for Process").checked(m.baseline.is_some()).enabled(m.snapshot.is_some()));
        view.push(AppMenuItem::new(MENU_DIAGNOSTICS, "Run System Diagnostics..."));

        let open = |o: fn(&Overlay) -> bool| m.overlay.as_ref().is_some_and(o);
        let window_items = vec![
            AppMenuItem::new(MENU_CPU_USAGE, "CPU Usage").key("cmd+2").checked(open(|o| matches!(o, Overlay::CpuUsage))),
            AppMenuItem::new(MENU_CPU_HISTORY, "CPU History").key("cmd+3").checked(open(|o| matches!(o, Overlay::CpuHistory))),
            AppMenuItem::new(MENU_GPU_HISTORY, "GPU History").key("cmd+4").checked(open(|o| matches!(o, Overlay::GpuHistory))),
        ];
        vec![
            AppMenu { title: "Edit".into(), items: vec![AppMenuItem::new(MENU_COPY, "Copy").key("cmd+c").enabled(m.has_selected())] },
            AppMenu { title: "View".into(), items: view },
            AppMenu { title: "Process".into(), items: process },
            AppMenu { title: "Window".into(), items: window_items },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        let Some(m) = &mut self.monitor else { return false };
        let handled = match id {
            MENU_QUIT => {
                m.quit_selected();
                true
            }
            MENU_FORCE => {
                m.force_quit_selected();
                true
            }
            MENU_SIGNAL_SHEET => {
                m.open_signal_sheet();
                true
            }
            MENU_INSPECT => {
                m.toggle_inspector();
                true
            }
            MENU_SAMPLE => {
                m.start_sample();
                true
            }
            MENU_FIND => {
                m.focus_search();
                ctx.wants_keyboard(true);
                true
            }
            MENU_COPY => {
                if let Some(text) = m.copy_text() {
                    ctx.set_pasteboard_string(&text);
                }
                true
            }
            MENU_DELTAS => {
                m.toggle_deltas();
                true
            }
            MENU_DIAGNOSTICS => {
                m.start_diagnostics();
                true
            }
            MENU_CPU_USAGE => {
                m.toggle_panel(Overlay::CpuUsage);
                true
            }
            MENU_CPU_HISTORY => {
                m.toggle_panel(Overlay::CpuHistory);
                true
            }
            MENU_GPU_HISTORY => {
                m.toggle_panel(Overlay::GpuHistory);
                true
            }
            n if (MENU_SIGNAL..MENU_SIGNAL + SIGNALS.len() as u32).contains(&n) => {
                m.send_signal(SIGNALS[(n - MENU_SIGNAL) as usize]);
                true
            }
            n if (MENU_COLUMN..MENU_COLUMN + model::OPTIONAL_COLUMNS.len() as u32).contains(&n) => {
                m.toggle_column((n - MENU_COLUMN) as usize);
                ctx.defaults_mut().set_int(PREF_COLUMNS, m.hidden_columns);
                true
            }
            n if (MENU_EXTRA..MENU_EXTRA + model::EXTRA_COLUMNS.len() as u32).contains(&n) => {
                m.toggle_extra((n - MENU_EXTRA) as usize);
                ctx.defaults_mut().set_int(PREF_EXTRA_COLUMNS, m.extra_columns);
                true
            }
            n if (MENU_DOCK..MENU_DOCK + DockIcon::ALL.len() as u32).contains(&n) => {
                let d = DockIcon::ALL[(n - MENU_DOCK) as usize];
                m.set_dock(d);
                ctx.defaults_mut().set_int(PREF_DOCK, d.index());
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
        };
        self.run_effects(ctx);
        handled
    }

    /// The save panel's answer for a report or the diagnostics archive, and the grant Reveal in Files waits for.
    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::SaveTo(mut doc) => {
                let Some(bytes) = self.saves.remove(&doc.request) else { return false };
                match doc.file.write_all(&bytes) {
                    Ok(()) => ctx.commit_document(doc.request),
                    Err(e) => {
                        ctx.log(&format!("save: {e}"));
                        ctx.abort_document(doc.request);
                        self.set_status("The file could not be saved");
                    }
                }
                true
            }
            DocumentEvent::Saved(doc) => {
                self.set_status(&format!("Saved {}", doc.name));
                true
            }
            DocumentEvent::Opened(doc) if self.reveals.contains(&doc.request) => {
                self.reveals.retain(|r| *r != doc.request);
                workspace::activate_file_viewer_selecting(ctx, &[doc.grant], |_| {});
                false
            }
            DocumentEvent::Failed { request, code } => {
                let was_save = self.saves.remove(&request).is_some();
                let was_reveal = self.reveals.contains(&request);
                self.reveals.retain(|r| *r != request);
                if was_reveal {
                    self.set_status("This file cannot be shown in Files");
                } else if was_save && code != DocErrorCode::Cancelled {
                    self.set_status("The file could not be saved");
                }
                was_save || was_reveal
            }
            _ => false,
        }
    }

    /// The shell's window list: the processes that own a window.
    fn service_reply(&mut self, _ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.window_request != Some(request) {
            return false;
        }
        self.window_request = None;
        let Some(m) = &mut self.monitor else { return false };
        match windows::decode_list(result) {
            Ok(list) => m.set_windowed(list.iter().filter(|w| w.on_screen).map(|w| w.owner_pid)),
            Err(e) => m.status = format!("The window list is not available: {e}"),
        }
        true
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
    bifrost_app::run(ActivityApp { monitor: None, saves: HashMap::new(), reveals: Vec::new(), window_request: None });
}
