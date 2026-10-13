//! System Information as an installed BifrOSt system app: the hardware tree (processor, memory, graphics, PCI, USB,
//! displays, network, power, storage) and the software section (the operating system and the installed apps). The
//! facts are read by `bifrost-core`'s `sysinfo`; `model.rs` builds the rows, `window.rs` holds the state and the
//! geometry, and `draw.rs` paints them.

mod draw;
mod extras;
mod model;
mod window;

use bifrost_app::controls::Node;
use bifrost_app::kit::{Damage, Surface};
use bifrost_app::proto::DocErrorCode;
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, DocumentEvent, Input, KeyInput, TouchKind};
use std::io::Write;
use window::{
    Focus, SysInfo, Target, MENU_COPY, MENU_FIND, MENU_FIND_NEXT, MENU_FIND_PREVIOUS, MENU_HELP, MENU_LESS, MENU_REFRESH, MENU_SAVE, MENU_SELECT_ALL, STEP_PT,
};

/// The help page the Help menu opens.
const HELP_URL: &str = "https://bifrost.olibuijr.com/docs/system-information";

/// The name the save panel suggests.
const REPORT_NAME: &str = "System Report.txt";

impl App for SysInfo {
    fn id(&self) -> &str {
        "is.olibuijr.sysinfo.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state: the facts are read again on every launch
        self.size = (ctx.cfg.w as f32, ctx.cfg.h as f32);
        if !self.loaded {
            self.reload();
        }
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => {
                let scale = ctx.ui.pt(1.0);
                let (x, y) = (x / scale, y / scale);
                match kind {
                    TouchKind::Down => {
                        self.press = self.hit(&ctx.ui, x, y);
                        self.drag_y = Some(y);
                        self.dragged = false;
                        false
                    }
                    TouchKind::Move => {
                        // a drag over the card scrolls it; the press then is not a tap
                        let Some(last) = self.drag_y else { return false };
                        if !matches!(self.press, Target::Content | Target::Line(_)) {
                            return false;
                        }
                        self.drag_y = Some(y);
                        if (y - last).abs() < 0.5 {
                            return false;
                        }
                        self.dragged = true;
                        self.scroll_by(last - y)
                    }
                    TouchKind::Up => {
                        let released = self.hit(&ctx.ui, x, y);
                        let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                        let was_drag = std::mem::replace(&mut self.dragged, false);
                        self.drag_y = None;
                        if !was_drag && released == pressed && released != Target::Nothing {
                            self.activate(released)
                        } else {
                            false
                        }
                    }
                    TouchKind::Cancel => {
                        self.press = Target::Nothing;
                        self.drag_y = None;
                        self.dragged = false;
                        false
                    }
                }
            }
            // a published control (accessibility, `bifrost pc midget tap-window`): the same action as a click
            Input::Activate(id) => self.activate(SysInfo::target_of_id(id)),
            Input::Key(key) => {
                let changed = self.key(key);
                ctx.wants_keyboard(self.find.is_some());
                changed
            }
            _ => false,
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let finding = self.find.as_ref().is_some_and(|q| !q.trim().is_empty());
        vec![
            AppMenu { title: "File".into(), items: vec![AppMenuItem::new(MENU_SAVE, "Save Report As...").key("cmd+s")] },
            AppMenu {
                title: "Edit".into(),
                items: vec![
                    AppMenuItem::new(MENU_COPY, "Copy").key("cmd+c").enabled(self.copy_text().is_some()),
                    AppMenuItem::new(MENU_SELECT_ALL, "Select All").key("cmd+a"),
                    AppMenuItem::separator(),
                    AppMenuItem::new(MENU_FIND, "Find").key("cmd+f"),
                    AppMenuItem::new(MENU_FIND_NEXT, "Find Next").key("cmd+g").enabled(finding),
                    AppMenuItem::new(MENU_FIND_PREVIOUS, "Find Previous").key("cmd+shift+g").enabled(finding),
                ],
            },
            AppMenu {
                title: "View".into(),
                items: vec![
                    AppMenuItem::new(MENU_REFRESH, "Refresh Information").key("cmd+r"),
                    AppMenuItem::new(MENU_LESS, "Show Less Information").checked(self.mini),
                ],
            },
            AppMenu { title: "Help".into(), items: vec![AppMenuItem::new(MENU_HELP, "System Information Help")] },
        ]
    }

    fn menu_action(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_SAVE => {
                let request = ctx.save_document(REPORT_NAME, &["public.text"]);
                self.export = Some((request, self.report_text().into_bytes()));
                false
            }
            MENU_REFRESH => {
                self.reload();
                true
            }
            MENU_LESS => self.toggle_mini(),
            MENU_COPY => {
                if let Some(text) = self.copy_text() {
                    ctx.set_pasteboard_string(&text);
                }
                false
            }
            MENU_SELECT_ALL => self.select_all(),
            MENU_FIND => {
                ctx.wants_keyboard(true);
                self.begin_find()
            }
            MENU_FIND_NEXT => self.find_step(true),
            MENU_FIND_PREVIOUS => self.find_step(false),
            MENU_HELP => {
                ctx.open_url(HELP_URL);
                false
            }
            _ => false,
        }
    }

    fn document(&mut self, ctx: &Ctx, ev: DocumentEvent) -> bool {
        match ev {
            DocumentEvent::SaveTo(mut doc) => match self.export.take() {
                Some((request, bytes)) if request == doc.request => match doc.file.write_all(&bytes) {
                    Ok(()) => ctx.commit_document(request),
                    Err(e) => {
                        ctx.log(&format!("save report: {e}"));
                        ctx.abort_document(request);
                    }
                },
                other => self.export = other,
            },
            DocumentEvent::Failed { request, code } => {
                if self.export.as_ref().is_some_and(|(r, _)| *r == request) {
                    self.export = None;
                }
                if code != DocErrorCode::Cancelled {
                    ctx.log(&format!("document: {code:?}"));
                }
            }
            _ => {}
        }
        false
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        draw::controls(self, &ctx.ui)
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        draw::paint(self, s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

impl SysInfo {
    /// A key press. While the find bar is open it takes the text; otherwise the arrows move the sidebar selection or
    /// scroll the card, whichever has focus, and Tab changes the focus.
    fn key(&mut self, key: KeyInput) -> bool {
        if self.find.is_some() {
            return match key {
                KeyInput::Text(text) => self.find_type(&text),
                KeyInput::Code(8) => self.find_backspace(),
                KeyInput::Code(13) => self.find_step(true),
                KeyInput::Code(27) => self.end_find(),
                _ => false,
            };
        }
        match key {
            KeyInput::Code(9) => self.toggle_focus(),
            KeyInput::Code(38) if self.focus == Focus::Sidebar => self.step_category(-1),
            KeyInput::Code(40) if self.focus == Focus::Sidebar => self.step_category(1),
            KeyInput::Code(38) => self.scroll_by(-STEP_PT),
            KeyInput::Code(40) => self.scroll_by(STEP_PT),
            _ => false,
        }
    }
}

fn main() {
    bifrost_app::run(SysInfo::new());
}
