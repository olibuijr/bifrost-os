//! Music in a Golden Gate full-size window: the source list at the left, the unified toolbar over the content pane,
//! and the pane itself (`music::Music`). Playback goes through the audio service (`is.olibuijr.audio`).

use crate::music;
use crate::music::chrome::{BAR_SEARCH, BAR_TITLE, SIDE_TAB};
use bifrost_app::audio::Client;
use bifrost_app::controls::nav::ToolbarLayout;
use bifrost_app::controls::sidebar::SidebarItem;
use bifrost_app::controls::{Node, Rect, Role, TextField};
use bifrost_app::kit::ui::{Ui, Weight, LABEL, MAC_TITLE3_PT, SEPARATOR_C};
use bifrost_app::kit::{Damage, Event, Surface};
use bifrost_app::layout::{SIDEBAR_FRACTION, SIDEBAR_MAX_PT, SIDEBAR_MIN_PT, UNIFIED_BAR_PT};
use bifrost_app::widgets;
use bifrost_app::{App, Ctx, Input, KeyInput, TouchKind};
use springboard::shell::music::radio::Http;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The source list starts under the traffic lights, like Files's.
const SIDEBAR_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.);
/// The open search field's width, in points.
const SEARCH_W: f32 = 180.;
/// Accessibility ids of the Play and Shuffle buttons on the album and playlist pages (sidebar rows use 100 to 299).
const NODE_PLAY: u64 = 300;
const NODE_SHUFFLE: u64 = 301;

/// What a pointer went down on.
#[derive(Clone, Copy, PartialEq)]
enum Down {
    Sidebar,
    Bar,
    Pane,
}

/// Music in a Golden Gate full-size window: the source list at the left, the unified toolbar over the
/// content pane, and the pane itself (`music::Music`, in pane-local pixels).
pub struct MusicApp {
    app: Option<music::Music>,
    keyboard: bool,
    /// window size, points
    w: f32,
    h: f32,
    title_w: f32,
    down: Option<Down>,
    /// the toolbar as last drawn, and its buttons as controls (window points)
    placed: Option<ToolbarLayout>,
    nodes: Vec<Node>,
}

impl MusicApp {
    pub fn new() -> Self {
        MusicApp { app: None, keyboard: false, w: 0., h: 0., title_w: 40., down: None, placed: None, nodes: vec![] }
    }

    /// Sidebar width in points: a fraction of the window within the system limits.
    fn side(&self) -> f32 {
        (self.w * SIDEBAR_FRACTION).clamp(SIDEBAR_MIN_PT, SIDEBAR_MAX_PT).round()
    }

    /// Sidebar width in pixels: where the content pane starts.
    fn x0(&self, ui: &Ui) -> usize {
        ui.pt(self.side()).round() as usize
    }

    fn bar_bounds(&self) -> Rect {
        let x = self.side() + 12.;
        let h = widgets::CAPSULE_PT;
        Rect { x, y: (UNIFIED_BAR_PT - h) / 2., w: (self.w - x - 16.).max(0.), h }
    }

    /// The search field is always open on the Songs page, as Music's toolbar field is; it widens only while typing.
    fn search_w(_: &music::Music) -> f32 {
        SEARCH_W
    }

    fn layout(&self) -> Option<ToolbarLayout> {
        let m = self.app.as_ref()?;
        Some(m.toolbar(self.title_w, Self::search_w(m)).layout_with(self.bar_bounds(), |_| widgets::CAPSULE_PT, widgets::CAPSULE_PT, widgets::TOOLBAR_GAP_PT))
    }

    /// A click in the toolbar band (points).
    fn bar_click(&mut self, x: f32, y: f32, t: f64) {
        let Some(placed) = self.placed.clone() else { return };
        let Some(m) = &mut self.app else { return };
        match placed.hit(x, y) {
            Some(BAR_SEARCH) if m.searching() => {
                if let Some(r) = placed.rect_of(BAR_SEARCH) {
                    if bifrost_app::controls::search::hits_clear(r, !m.search_text().is_empty(), x, y) {
                        m.clear_search();
                    }
                }
            }
            Some(id) => m.bar_press(id, t),
            None => {}
        }
    }

    fn sidebar_click(&mut self, x: f32, y: f32, t: f64) {
        let bounds = Rect { x: 0., y: 0., w: self.side(), h: self.h };
        let Some(m) = &mut self.app else { return };
        if let Some(id) = m.sidebar().hit(bounds, SIDEBAR_SCROLL, x, y) {
            m.pick_sidebar(id, t);
        }
    }

    /// An accessibility press on a node the window published (`App::controls`): Play or Shuffle, a sidebar row,
    /// or a toolbar button. True when a frame is needed.
    fn press_node(&mut self, id: u64) -> bool {
        match id {
            NODE_PLAY | NODE_SHUFFLE => {
                if let Some(m) = &mut self.app {
                    m.press_action(id == NODE_SHUFFLE, 0.);
                }
                true
            }
            id if (SIDE_TAB as u64..NODE_PLAY).contains(&id) => {
                if let Some(m) = &mut self.app {
                    m.pick_sidebar(id as u32, 0.);
                }
                true
            }
            id => {
                let Some(r) = self.layout().and_then(|l| l.rect_of(id as u32)) else { return false };
                self.bar_click(r.x + r.w / 2., r.y + r.h / 2., 0.);
                true
            }
        }
    }

    /// Draws the source list and the toolbar around the pane and records the controls the window publishes.
    fn chrome(&mut self, s: &mut Surface, ui: &Ui) {
        let Some(m) = &self.app else { return };
        let side = self.side();
        let list = m.sidebar();
        widgets::source_list(s, ui, Rect { x: 0., y: 0., w: side, h: self.h }, &list, SIDEBAR_SCROLL, None);
        let mut nodes: Vec<Node> = list
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let SidebarItem::Row { id, label, enabled, .. } = item else { return None };
                Some(Node {
                    id: *id as u64,
                    role: Role::Row,
                    label: label.clone(),
                    value: if list.selected() == Some(*id) { "Selected".to_string() } else { String::new() },
                    bounds: list.item_rect(i, Rect { x: 0., y: 0., w: side, h: self.h }, SIDEBAR_SCROLL),
                    enabled: *enabled,
                })
            })
            .collect();
        s.fill_rect(self.x0(ui).saturating_sub(1), 0, 1, s.h, SEPARATOR_C.now());
        if m.overlay() {
            self.placed = None;
            self.nodes.clear();
            return;
        }
        let title = m.title();
        let others = 6. * widgets::CAPSULE_PT + SEARCH_W + 4. * widgets::TOOLBAR_GAP_PT;
        let room = (self.bar_bounds().w - others).max(40.);
        self.title_w = (ui.measure(&title, MAC_TITLE3_PT, Weight::Bold) / ui.pt(1.) + 4.).clamp(40., room);
        widgets::toolbar_band(s, ui, Rect { x: side, y: 0., w: self.w - side, h: UNIFIED_BAR_PT });
        let bar = m.toolbar(self.title_w, Self::search_w(m));
        let placed = widgets::toolbar(s, ui, self.bar_bounds(), &bar, None);
        if let Some(r) = placed.rect_of(BAR_TITLE) {
            let im = ui.text(&ui.fit(&title, MAC_TITLE3_PT, ui.pt(r.w)), MAC_TITLE3_PT, Weight::Bold, LABEL.now());
            s.blit(&im, ui.pt(r.x) as i32, ui.pt(r.y + r.h / 2.) as i32 - im.h as i32 / 2, 255, 255);
        }
        let mut field = TextField::new();
        field.push_str(m.search_text());
        // The search field is a kit TextField that records its own accessibility node (`widgets::search_field`),
        // so the toolbar publishes only its buttons here.
        if let Some(r) = placed.rect_of(BAR_SEARCH) {
            let focused = m.searching();
            widgets::search_field(s, ui, &field, Rect { x: r.x, y: r.y + 4., w: r.w, h: r.h - 8. }, "Search Songs", focused);
        }
        nodes.extend(
            placed
                .visible
                .iter()
                .filter(|(id, _)| *id != BAR_TITLE && *id != BAR_SEARCH)
                .filter_map(|(id, r)| {
                    let item = bar.items.iter().find(|i| i.id == *id)?;
                    Some(Node { id: *id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: *r, enabled: item.enabled })
                }),
        );
        let x0 = self.x0(ui) as f32;
        for (label, r) in m.action_buttons(ui) {
            nodes.push(Node {
                id: if label == "Play" { NODE_PLAY } else { NODE_SHUFFLE },
                role: Role::Button,
                label: label.to_string(),
                value: String::new(),
                bounds: Rect { x: (x0 + r.x) / ui.s, y: r.y / ui.s, w: r.w / ui.s, h: r.h / ui.s },
                enabled: true,
            });
        }
        self.nodes = nodes;
        self.placed = Some(placed);
    }

    fn sync_keyboard(&mut self, ctx: &Ctx) {
        let want = self.app.as_ref().is_some_and(|m| m.wants_keyboard());
        if want != self.keyboard {
            self.keyboard = want;
            ctx.wants_keyboard(want);
        }
    }
}

fn shell_dir() -> PathBuf {
    PathBuf::from(std::env::var("BIFROST_SERVICE_ROOT").unwrap_or_else(|_| "/var/lib/springboard".into()))
}

impl App for MusicApp {
    fn id(&self) -> &str {
        "is.olibuijr.music.desktop"
    }

    fn configure(&mut self, ctx: &Ctx) {
        ctx.enable_sudden_termination(); // the queue and playlists are written when they change
        self.w = ctx.cfg.w as f32 / ctx.ui.s;
        self.h = ctx.cfg.h as f32 / ctx.ui.s;
        let (w, h) = (ctx.cfg.w as i32 - self.x0(&ctx.ui) as i32, ctx.cfg.h as i32);
        if let Some(m) = &mut self.app {
            m.set_size(w, h);
            return;
        }
        let roots = vec![PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Music")];
        let Ok(mut m) = music::Music::new(w, h, roots, Client::for_root(&shell_dir())) else { return };
        m.enable_library_store(ctx.data_dir.join("Music"));
        m.enable_radio(ctx.data_dir.join("Radio"), Arc::new(Http::default()));
        self.app = Some(m);
    }

    fn event(&mut self, ctx: &Ctx, ev: Input) -> bool {
        let Some(overlay) = self.app.as_ref().map(|m| m.overlay()) else { return false };
        let changed = match ev {
            Input::Touch { kind, x, y, t, .. } => {
                let (xp, yp) = (x / ctx.ui.s, y / ctx.ui.s);
                if kind == TouchKind::Down {
                    self.down = Some(if overlay || xp >= self.side() && yp >= UNIFIED_BAR_PT {
                        Down::Pane
                    } else if xp < self.side() {
                        Down::Sidebar
                    } else {
                        Down::Bar
                    });
                }
                let down = self.down;
                if matches!(kind, TouchKind::Up | TouchKind::Cancel) {
                    self.down = None;
                }
                match down {
                    Some(Down::Pane) | None => {
                        let x = x - self.x0(&ctx.ui) as f32;
                        let e = match kind {
                            TouchKind::Down => Event::Down(x, y),
                            TouchKind::Move => Event::Move(x, y),
                            TouchKind::Up | TouchKind::Cancel => Event::Up(x, y),
                        };
                        if let Some(m) = &mut self.app {
                            m.touch(&ctx.ui, e, t);
                        }
                    }
                    Some(Down::Sidebar) if kind == TouchKind::Up => self.sidebar_click(xp, yp, t),
                    Some(Down::Bar) if kind == TouchKind::Up => self.bar_click(xp, yp, t),
                    _ => {}
                }
                true
            }
            Input::Activate(id) => self.press_node(id as u64),
            Input::Key(KeyInput::Code(27)) => self.app.as_mut().is_some_and(|m| m.cancel(0.)),
            Input::Key(KeyInput::Text(s)) => {
                self.app.as_mut().unwrap().type_text(&s);
                true
            }
            Input::Key(KeyInput::Code(8)) => {
                self.app.as_mut().unwrap().backspace();
                true
            }
            Input::Key(KeyInput::Code(13)) => {
                self.app.as_mut().unwrap().type_text("\n");
                true
            }
            Input::Key(KeyInput::Code(_)) => false,
            Input::Open(url) => {
                let path = url.strip_prefix("file://").unwrap_or(&url);
                self.app.as_mut().unwrap().play_path(Path::new(path));
                true
            }
            _ => false,
        };
        self.sync_keyboard(ctx);
        changed
    }

    fn tick(&mut self, ctx: &Ctx, t: f64) -> bool {
        let Some(m) = &mut self.app else { return false };
        m.pump(&ctx.ui);
        let due = m.tick(&ctx.ui, t);
        self.sync_keyboard(ctx);
        due
    }

    fn controls(&self, _: &Ctx) -> Vec<Node> {
        self.nodes.clone()
    }

    fn next_wake(&self, t: f64) -> Option<f64> {
        self.app.as_ref().and_then(|m| m.next_wake(t))
    }

    fn draw(&mut self, ctx: &Ctx, s: &mut Surface, t: f64) -> Damage {
        let x0 = self.x0(&ctx.ui).min(s.w);
        if let Some(m) = &mut self.app {
            let mut pane = Surface { w: s.w - x0, h: s.h, stride: s.stride, px: &mut s.px[x0..], y0: s.y0 };
            m.draw(&mut pane, &ctx.ui, t);
        }
        self.chrome(s, &ctx.ui);
        let mut d = Damage::new(s.w, s.h);
        d.all();
        d
    }
}
