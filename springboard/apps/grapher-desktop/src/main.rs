//! Grapher: an equation plotter hosted through the SDK. Equations are typed in the sidebar (`y = f(x)`, parametric
//! pairs and polar curves), plotted in colour on a grid, panned by dragging and zoomed with the toolbar, and saved as
//! a PNG. The document and the view are in `model.rs` and `view.rs`; the language is in `expr.rs` and `equation.rs`;
//! the window is `window.rs` (state, geometry, input), `draw.rs` (the chrome) and `plot.rs` (the plot, shared with
//! the export).

mod draw;
mod equation;
mod expr;
mod model;
mod plot;
mod png;
mod view;
mod window;

use bifrost_app::kit::{Damage, Surface};
use bifrost_app::controls::{Node, Role};
use bifrost_app::{App, Ctx, Input};
use window::{Grapher, Target, ROW_BASE};

struct GrapherApp {
    grapher: Grapher,
}

impl App for GrapherApp {
    fn id(&self) -> &str {
        "is.olibuijr.grapher.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        // every change is saved as it happens, so nothing is lost when the app closes
        ctx.enable_sudden_termination();
        self.grapher.size = (ctx.cfg.w as f32, ctx.cfg.h as f32);
        self.grapher.load(&ctx.data_dir);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        match ev {
            Input::Touch { kind, x, y, .. } => self.grapher.touch(&ctx.ui, &ctx.data_dir, kind, x, y),
            Input::Key(key) => self.grapher.key(&ctx.data_dir, key),
            Input::Activate(id) => {
                let target = if id as u32 >= ROW_BASE {
                    Target::Row(id as u32 - ROW_BASE)
                } else {
                    Target::Button(id as u32)
                };
                self.grapher.activate(&ctx.ui, &ctx.data_dir, target)
            }
            _ => false,
        }
    }

    fn controls(&self, ctx: &Ctx) -> Vec<Node> {
        let g = &self.grapher;
        let mut nodes = Vec::new();
        let toolbar = g.toolbar();
        for (id, bounds) in g.toolbar_layout(&ctx.ui).visible {
            if let Some(item) = toolbar.items.iter().find(|item| item.id == id) {
                nodes.push(Node { id: id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds, enabled: item.enabled });
            }
        }
        let list = g.sidebar_list();
        let bounds = g.list_rect();
        for (index, item) in list.items.iter().enumerate() {
            if let bifrost_app::controls::sidebar::SidebarItem::Row { id, label, enabled, .. } = item {
                let row = list.item_rect(index, bounds, 0.0);
                if row.w > 0.0 && row.h > 0.0 {
                    nodes.push(Node { id: *id as u64, role: Role::Row, label: label.clone(), value: String::new(), bounds: row, enabled: *enabled });
                }
            }
        }
        nodes
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, _t: f64) -> Damage {
        draw::paint(&mut self.grapher, s, &ctx.ui);
        let mut damage = Damage::new(s.w, s.h);
        damage.all();
        damage
    }
}

fn main() {
    bifrost_app::run(GrapherApp { grapher: Grapher::new() });
}
