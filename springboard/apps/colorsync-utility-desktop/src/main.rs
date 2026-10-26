//! Colour Utility as an installed BifrOSt system app: the ICC colour profiles in the system and the user's folders,
//! each with its header, its tags, the structural checks and, for an RGB matrix profile, the gamut on the CIE 1931 xy
//! chart. `library.rs` finds the files, `model.rs` reads and checks them, `window.rs` holds the state and the geometry
//! and `draw.rs` paints the window.

mod draw;
mod library;
mod model;
mod window;

use bifrost_app::kit::{Damage, Surface};
use bifrost_app::{App, AppMenu, AppMenuItem, Ctx, Input, KeyInput, TouchKind};
use window::{ColourUtility, Panel, Target};

/// The File menu's refresh item.
const MENU_REFRESH: u32 = 1;
/// The View menu's panel items: this id plus the panel's index in `Panel::ALL`.
const MENU_PANEL_BASE: u32 = 10;
/// The shortcuts of the panel items, in the order of `Panel::ALL`.
const PANEL_KEYS: [&str; 4] = ["cmd+1", "cmd+2", "cmd+3", "cmd+4"];
/// Arrow key codes, as the shell sends them.
const KEY_UP: u32 = 38;
const KEY_DOWN: u32 = 40;

/// The window size in points, from the configured size in pixels and the display scale.
fn size_pt(w_px: u32, h_px: u32, scale: f32) -> (f32, f32) {
    let scale = scale.max(f32::EPSILON);
    (w_px as f32 / scale, h_px as f32 / scale)
}

impl ColourUtility {
    /// What a tap or a click on `target` does. True when the window needs a new frame.
    fn activate(&mut self, target: Target) -> bool {
        match target {
            Target::Row(index) => self.select(index),
            Target::Tab(panel) => self.show(panel),
            Target::Nothing | Target::Side | Target::Body => false,
        }
    }
}

impl App for ColourUtility {
    fn id(&self) -> &str {
        "is.olibuijr.colour-utility.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // keeps no unsaved state: every profile is read again from its file
        // `cfg` is in pixels, the layout in points: a scaled window would otherwise lay out wider than it is
        self.size = size_pt(ctx.cfg.w, ctx.cfg.h, ctx.ui.pt(1.0));
        if !self.loaded {
            self.loaded = true;
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
                        self.press = self.hit(x, y);
                        self.drag_y = Some(y);
                        self.dragged = false;
                        false
                    }
                    TouchKind::Move => {
                        let Some(last) = self.drag_y else {
                            return false;
                        };
                        self.drag_y = Some(y);
                        let delta = last - y;
                        if delta.abs() < 0.5 {
                            return false;
                        }
                        match self.press {
                            Target::Side => {
                                self.dragged = true;
                                self.scroll_side_by(delta)
                            }
                            Target::Body => {
                                self.dragged = true;
                                self.scroll_body_by(delta)
                            }
                            _ => false,
                        }
                    }
                    TouchKind::Up => {
                        let released = self.hit(x, y);
                        let pressed = std::mem::replace(&mut self.press, Target::Nothing);
                        let was_drag = std::mem::replace(&mut self.dragged, false);
                        self.drag_y = None;
                        if was_drag || released != pressed {
                            return false;
                        }
                        self.activate(released)
                    }
                    TouchKind::Cancel => {
                        self.press = Target::Nothing;
                        self.drag_y = None;
                        self.dragged = false;
                        false
                    }
                }
            }
            Input::Key(KeyInput::Code(KEY_UP)) => self.step(-1),
            Input::Key(KeyInput::Code(KEY_DOWN)) => self.step(1),
            _ => false,
        }
    }

    fn main_menu(&self, _ctx: &Ctx) -> Vec<AppMenu> {
        let mut view = Vec::new();
        for (i, panel) in Panel::ALL.iter().enumerate() {
            let item = AppMenuItem::new(MENU_PANEL_BASE + i as u32, panel.title()).key(PANEL_KEYS[i]).checked(self.panel == *panel);
            view.push(item);
        }
        vec![
            AppMenu { title: "File".into(), items: vec![AppMenuItem::new(MENU_REFRESH, "Refresh Profiles").key("cmd+r")] },
            AppMenu { title: "View".into(), items: view },
        ]
    }

    fn menu_action(&mut self, _ctx: &Ctx, id: u32) -> bool {
        if id == MENU_REFRESH {
            self.reload();
            return true;
        }
        let index = id.wrapping_sub(MENU_PANEL_BASE) as usize;
        match Panel::ALL.get(index) {
            Some(panel) => self.show(*panel),
            None => false,
        }
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        let mut labels = std::mem::take(&mut self.labels);
        draw::paint(self, &mut labels, s, &ctx.ui);
        self.labels = labels;
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}

fn main() {
    bifrost_app::run(ColourUtility::new());
}
