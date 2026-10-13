//! Activity Monitor as an installed BifrOSt app: the processes of this machine, with CPU, memory, energy, disk and
//! network readings from `/proc`. The window is `window.rs` and `draw.rs`; the readings are `sample.rs`, and the
//! table's rows and the graph history are `model.rs`. Quitting sends SIGTERM, and only to processes of this user.

#[allow(dead_code)]
mod draw;
#[allow(dead_code)]
mod inspect;
#[allow(dead_code)]
mod model;
#[allow(dead_code)]
mod sample;
#[allow(dead_code)]
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};
use model::{Filter, UpdateFrequency, SIGNALS};
use window::{Monitor, MENU_COLUMN, MENU_COPY, MENU_FILTER, MENU_FIND, MENU_FORCE, MENU_FREQ, MENU_INSPECT, MENU_QUIT, MENU_SIGNAL};

/// The preference holding View > Update Frequency, in seconds.
const PREF_FREQUENCY: &str = "updateSeconds";
/// The preference holding View > Columns, as a mask of hidden columns.
const PREF_COLUMNS: &str = "hiddenColumns";

struct ActivityApp {
    monitor: Option<Monitor>,
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
                self.monitor = Some(m);
            }
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
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

    fn tick(&mut self, _ctx: &Ctx, t: f64) -> bool {
        self.monitor.as_mut().map_or(false, |m| m.tick(t))
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
            AppMenuItem::submenu("Send Signal to Process").enabled(can_signal),
        ];
        for (i, sig) in SIGNALS.iter().enumerate() {
            process.push(AppMenuItem::new(MENU_SIGNAL + i as u32, &format!("SIG{}", sig.name)).enabled(can_signal).depth(1));
        }
        process.push(AppMenuItem::separator());
        process.push(AppMenuItem::new(MENU_INSPECT, "Inspect Process").enabled(m.has_selected()));
        process.push(AppMenuItem::new(MENU_FIND, "Find Process").key("cmd+f"));

        let mut view = vec![AppMenuItem::submenu("Filter Processes")];
        for (i, f) in Filter::ALL.iter().enumerate() {
            view.push(AppMenuItem::new(MENU_FILTER + i as u32, f.label()).checked(m.filter == *f).depth(1));
        }
        view.push(AppMenuItem::submenu("Update Frequency"));
        for (i, f) in UpdateFrequency::ALL.iter().enumerate() {
            view.push(AppMenuItem::new(MENU_FREQ + i as u32, f.label()).checked(m.frequency == *f).depth(1));
        }
        view.push(AppMenuItem::submenu("Columns"));
        for (i, (_, title)) in model::OPTIONAL_COLUMNS.iter().enumerate() {
            view.push(AppMenuItem::new(MENU_COLUMN + i as u32, title).checked(m.column_shown(i)).depth(1));
        }
        vec![
            AppMenu { title: "Edit".into(), items: vec![AppMenuItem::new(MENU_COPY, "Copy").key("cmd+c").enabled(m.has_selected())] },
            AppMenu { title: "View".into(), items: view },
            AppMenu { title: "Process".into(), items: process },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
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
            n if (MENU_COLUMN..MENU_COLUMN + model::OPTIONAL_COLUMNS.len() as u32).contains(&n) => {
                m.toggle_column((n - MENU_COLUMN) as usize);
                ctx.defaults_mut().set_int(PREF_COLUMNS, m.hidden_columns);
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
    bifrost_app::run(ActivityApp { monitor: None });
}
