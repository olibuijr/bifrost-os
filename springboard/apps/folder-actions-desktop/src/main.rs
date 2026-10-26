//! Folder Actions Setup as an installed BifrOSt app in the full-size window: attach an app's intent to a folder so it
//! runs for each file that lands there, and take it off again. The shell runs the actions (`shell::folder_actions`);
//! this window edits the list through the `folder-actions` service (`bifrost_app::folder_actions`). The folder is typed
//! as a path (the SDK has no folder panel yet). The pure parts, including the geometry, are in `model.rs`.

mod model;

use bifrost_app::folder_actions::{self, Setup};
use bifrost_app::kit::ui::{self, Weight};
use bifrost_app::kit::{Damage, Surface, Ui};
use bifrost_app::layout::Layout;
use bifrost_app::proto::CHORD_FLAG;
use bifrost_app::widgets;
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use bifrost_core::json::Json;
use model::{Geo, Rect};
use std::collections::HashMap;

/// What a service request was for, so its reply can be handled.
enum Job {
    List,
    Attach,
    Detach,
}

struct FolderActionsApp {
    started: bool,
    setup: Setup,
    /// The selected row of the attached list, and of the list of actions that could be attached.
    attached: Option<usize>,
    offer: Option<usize>,
    /// The folder path being typed.
    path: String,
    message: String,
    pending: HashMap<u32, Job>,
    /// The window and the content column, in points, from the last draw.
    window: (f32, f32),
    col: (f32, f32),
}

impl FolderActionsApp {
    fn geo(&self) -> Geo {
        Geo::new(self.col.0, self.col.1, self.window.1)
    }

    fn refresh(&mut self, ctx: &Ctx) {
        let id = folder_actions::list(ctx);
        self.pending.insert(id, Job::List);
    }

    fn attach(&mut self, ctx: &Ctx) {
        let home = std::env::var("HOME").unwrap_or_default();
        let folder = match model::clean_folder(&self.path, &home) {
            Ok(f) => f,
            Err(e) => {
                self.message = e;
                return;
            }
        };
        let Some(offer) = self.offer.and_then(|i| self.setup.offers.get(i)).cloned() else {
            self.message = "Choose the action to run.".to_string();
            return;
        };
        let id = folder_actions::attach(ctx, &folder, &offer.app, &offer.intent);
        self.pending.insert(id, Job::Attach);
        self.message = format!("Attaching {} to {folder}...", offer.title);
    }

    fn remove(&mut self, ctx: &Ctx) {
        let Some(action) = self.attached.and_then(|i| self.setup.actions.get(i)).cloned() else {
            self.message = "Choose an attached action to remove.".to_string();
            return;
        };
        let id = folder_actions::detach(ctx, &action.folder, &action.app, &action.intent);
        self.pending.insert(id, Job::Detach);
    }

    /// Handles a touch release. True when the window needs a redraw.
    fn touch(&mut self, ctx: &Ctx, x: f32, y: f32) -> bool {
        let g = self.geo();
        if let Some(i) = g.attached_row_at(x, y) {
            if i < self.setup.actions.len() {
                self.attached = Some(i);
                return true;
            }
            return false;
        }
        if let Some(i) = g.offer_row_at(x, y) {
            if i < self.setup.offers.len() {
                self.offer = Some(i);
                return true;
            }
            return false;
        }
        if g.remove.contains(x, y) {
            self.remove(ctx);
            return true;
        }
        if g.attach.contains(x, y) {
            self.attach(ctx);
            return true;
        }
        false
    }
}

fn text(s: &mut Surface, u: &Ui, label: &str, size: f32, color: u32, x: f32, y: f32) {
    s.blit(&u.text(label, size, Weight::Regular, color), u.pt(x) as i32, u.pt(y) as i32, 255, 255);
}

fn button(s: &mut Surface, u: &Ui, r: Rect, label: &str, enabled: bool, destructive: bool) {
    let fill = if !enabled { ui::FIELD_BG.now() } else if destructive { ui::DESTRUCTIVE } else { ui::TINT };
    let color = if enabled { ui::WHITE } else { ui::LABEL_2.now() };
    s.fill_rrect(u.pt(r.x), u.pt(r.y), u.pt(r.w), u.pt(r.h), u.pt(8.0), fill);
    text(s, u, label, ui::MAC_BODY_PT, color, r.x + 14.0, r.y + 6.0);
}

/// One list row: a selection plate, the first line, the second line, and a separator.
fn row(s: &mut Surface, u: &Ui, g: &Geo, top: f32, i: usize, lines: (String, String), selected: bool) {
    let y = top + i as f32 * model::ROW_PT;
    if selected {
        s.fill_rrect(u.pt(g.col), u.pt(y), u.pt(g.w), u.pt(model::ROW_PT - 2.0), u.pt(9.0), ui::ROW_PRESSED.now());
    }
    let max = u.pt(g.w - 24.0);
    text(s, u, &u.fit(&lines.0, ui::MAC_BODY_PT, max), ui::MAC_BODY_PT, ui::LABEL.now(), g.col + 12.0, y + 4.0);
    text(s, u, &u.fit(&lines.1, ui::MAC_FOOTNOTE_PT, max), ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), g.col + 12.0, y + 22.0);
    s.fill_rect(u.pt(g.col + 12.0) as usize, u.pt(y + model::ROW_PT - 1.0) as usize, u.pt(g.w - 24.0) as usize, 1, ui::SEPARATOR_C.now());
}

impl App for FolderActionsApp {
    fn id(&self) -> &str {
        "is.olibuijr.folderactions.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination();
        if !self.started {
            self.started = true;
            self.refresh(ctx);
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind: TouchKind::Up, x, y, .. } => {
                let s = ctx.ui.s;
                self.touch(ctx, x / s, y / s)
            }
            Input::Key(KeyInput::Text(t)) => {
                let before = self.path.len();
                self.path.extend(t.chars().filter(|c| !c.is_control()));
                self.path.len() != before
            }
            Input::Key(KeyInput::Code(code)) => {
                if code & CHORD_FLAG != 0 {
                    return false;
                }
                match code {
                    13 => {
                        self.attach(ctx);
                        true
                    }
                    8 => self.path.pop().is_some(),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        let Some(job) = self.pending.remove(&request) else { return false };
        match job {
            Job::List => match folder_actions::decode_setup(result) {
                Some(setup) => {
                    self.setup = setup;
                    self.attached = self.attached.filter(|&i| i < self.setup.actions.len());
                    self.offer = self.offer.filter(|&i| i < self.setup.offers.len());
                }
                None => {
                    self.message = match result {
                        Err(e) => format!("Could not read the folder actions: {e}"),
                        Ok(_) => "Could not read the folder actions.".to_string(),
                    }
                }
            },
            Job::Attach => {
                self.message = match folder_actions::decode_flag(result, "attached") {
                    Ok(true) => "Attached. It runs for each file that is added to the folder from now on.".to_string(),
                    Ok(false) => "That action is already attached to the folder.".to_string(),
                    Err(e) => format!("Could not attach: {e}"),
                };
                self.refresh(ctx);
            }
            Job::Detach => {
                self.message = match folder_actions::decode_flag(result, "detached") {
                    Ok(true) => "Removed.".to_string(),
                    Ok(false) => "That action was not attached.".to_string(),
                    Err(e) => format!("Could not remove: {e}"),
                };
                self.attached = None;
                self.refresh(ctx);
            }
        }
        true
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let u = &ctx.ui;
        // The window colour, never pure black (`DETAIL_BG` is the pane behind cards).
        s.fill_rect(0, 0, s.w, s.h, ui::CONTENT_BG.now());
        self.window = (s.w as f32 / u.pt(1.0), s.h as f32 / u.pt(1.0));
        let mut l = Layout::from_points(self.window.0, self.window.1, 0.0, 0.0);
        // No sidebar here: the form is centred in the whole window instead of right of an empty column.
        l.sidebar = 0.0;
        self.col = model::form_column(self.window.0, self.window.1);
        widgets::large_title(s, u, &l, "Folder Actions", 40.0);
        let g = self.geo();

        text(s, u, "ATTACHED ACTIONS", ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), g.col, g.attached_y - 20.0);
        if self.setup.actions.is_empty() {
            text(s, u, "No folder has an action yet. Add one below.", ui::MAC_BODY_PT, ui::LABEL_2.now(), g.col + 12.0, g.attached_y + 10.0);
        }
        for (i, a) in self.setup.actions.iter().enumerate().take(g.attached_rows) {
            row(s, u, &g, g.attached_y, i, model::action_lines(&self.setup, a), self.attached == Some(i));
        }
        if self.setup.actions.len() > g.attached_rows {
            let more = format!("{} more not shown", self.setup.actions.len() - g.attached_rows);
            text(s, u, &more, ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), g.col + 12.0, g.remove.y + 8.0);
        }
        button(s, u, g.remove, "Remove", self.attached.is_some(), true);

        text(s, u, "FOLDER", ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), g.col, g.field.y - 20.0);
        s.fill_rrect(u.pt(g.field.x), u.pt(g.field.y), u.pt(g.field.w), u.pt(g.field.h), u.pt(9.0), ui::FIELD_BG.now());
        let (shown, color) = if self.path.is_empty() {
            ("Path of the folder, such as ~/Downloads".to_string(), ui::LABEL_2.now())
        } else {
            (u.fit(&self.path, ui::MAC_BODY_PT, u.pt(g.field.w - 24.0)), ui::LABEL.now())
        };
        text(s, u, &shown, ui::MAC_BODY_PT, color, g.field.x + 12.0, g.field.y + 8.0);

        text(s, u, "ACTION TO RUN FOR EACH NEW FILE", ui::MAC_FOOTNOTE_PT, ui::LABEL_2.now(), g.col, g.offers_y - 20.0);
        if self.setup.offers.is_empty() {
            text(s, u, "No installed app offers an action that takes a file.", ui::MAC_BODY_PT, ui::LABEL_2.now(), g.col + 12.0, g.offers_y + 10.0);
        }
        for (i, o) in self.setup.offers.iter().enumerate().take(g.offer_rows) {
            row(s, u, &g, g.offers_y, i, model::offer_lines(o), self.offer == Some(i));
        }
        button(s, u, g.attach, "Attach", self.offer.is_some() && !self.path.trim().is_empty(), false);

        text(s, u, &u.fit(&self.message, ui::MAC_BODY_PT, u.pt(g.w)), ui::MAC_BODY_PT, ui::LABEL_2.now(), g.col, g.status_y);

        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(FolderActionsApp {
        started: false,
        setup: Setup::default(),
        attached: None,
        offer: None,
        path: String::new(),
        message: "Type a folder, choose an action, then Attach.".to_string(),
        pending: HashMap::new(),
        window: (1200.0, 780.0),
        col: (32.0, 600.0),
    });
}
