//! Freeform: a finger drawing canvas with pen, marker and eraser, a colour
//! palette, undo and "save to Photos" (a PNG in the screenshot directory).

use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{
    Ui, Weight, BLACK, LABEL, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_PURPLE, SYSTEM_RED, SYSTEM_YELLOW, TINT,
    TOAST_FILL, WHITE,
};
use bifrost_app::kit::ui::{SPACE_L, SPACE_S, SPACE_XXL, SPACE_XXXL};
use bifrost_app::kit::ui::MAC_CALLOUT_PT;
use bifrost_app::kit::{Event, Surface, TextCache};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::widgets;
use std::path::PathBuf;
use bifrost_app::kit::ui::{SPACE_M, SPACE_XS};

/// The board is paper in both appearances, so a saved PNG looks the same either way.
const PAPER: u32 = WHITE;
/// One colour well in the toolbar's colour slot (points).
const WELL_PT: f32 = 28.0;
const BAR_PEN: u32 = 1;
const BAR_MARKER: u32 = 2;
const BAR_ERASER: u32 = 3;
const BAR_COLORS: u32 = 4;
const BAR_UNDO: u32 = 5;
const BAR_CLEAR: u32 = 6;
const BAR_SAVE: u32 = 7;
const UNDO_DEPTH: usize = 12;

const COLORS: [u32; 7] = [
    BLACK,
    SYSTEM_RED,
    SYSTEM_ORANGE,
    SYSTEM_YELLOW,
    SYSTEM_GREEN,
    TINT,
    SYSTEM_PURPLE,
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Pen,
    Marker,
    Eraser,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Tool(Tool),
    Color(usize),
    Undo,
    Clear,
    Save,
}

pub struct Freeform {
    w: i32,
    h: i32,
    /// the drawing, `cw` x `ch` px, always opaque
    canvas: Vec<u32>,
    cw: usize,
    ch: usize,
    undo: Vec<Vec<u32>>,
    tool: Tool,
    color: usize,
    last: Option<(f32, f32)>,
    drawing: bool,
    pressed: Option<Target>,
    save_dir: Option<PathBuf>,
    toast: Option<(String, f64)>,
    now: f64,
    dirty: bool,
    cache: TextCache,
    /// The toolbar as last drawn (points), for hit tests and the published controls.
    placed: Option<ToolbarLayout>,
    /// Points per pixel inverse (`ui.s`), kept for `controls`.
    scale: f32,
}

/// Linear blend of two packed pixels, byte by byte (`a` = 0..=255 of `src`).
fn mix(dst: u32, src: u32, a: u32) -> u32 {
    let mut out = 0u32;
    for shift in [0u32, 8, 16, 24] {
        let d = (dst >> shift) & 255;
        let s = (src >> shift) & 255;
        out |= ((d * (255 - a) + s * a + 127) / 255) << shift;
    }
    out
}

impl Freeform {
    pub fn new(_ui: &Ui, w: i32, h: i32) -> Self {
        Self {
            w,
            h,
            canvas: Vec::new(),
            cw: 0,
            ch: 0,
            undo: Vec::new(),
            tool: Tool::Pen,
            color: 0,
            last: None,
            drawing: false,
            pressed: None,
            save_dir: None,
            toast: None,
            now: 0.0,
            dirty: true,
            cache: TextCache::default(),
            placed: None,
            scale: 1.0,
        }
    }

    /// Saved drawings go to `dir` (the Photos screenshot directory).
    pub fn with_save_dir(mut self, dir: PathBuf) -> Self {
        self.save_dir = Some(dir);
        self
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.dirty = true;
    }

    fn bar_h(&self, ui: &Ui) -> f32 {
        ui.pt(UNIFIED_BAR_PT)
    }

    /// The unified toolbar's row (points), right of the traffic lights.
    fn bar_bounds(&self, ui: &Ui) -> Rect {
        let x = traffic_lights_trailing() + widgets::TOOLBAR_GAP_PT;
        let h = widgets::CAPSULE_PT;
        Rect { x, y: (UNIFIED_BAR_PT - h) / 2.0, w: (self.w as f32 / ui.s - x - SPACE_L).max(0.0), h }
    }

    /// Freeform's toolbar: the tools as one segmented group, the colour wells, then undo, clear and share.
    fn bar(&self) -> Toolbar {
        Toolbar::new(vec![
            ToolbarItem::button(BAR_PEN, "Pen").in_group(1).selected(self.tool == Tool::Pen),
            ToolbarItem::button(BAR_MARKER, "Marker").in_group(1).selected(self.tool == Tool::Marker),
            ToolbarItem::button(BAR_ERASER, "Eraser").in_group(1).selected(self.tool == Tool::Eraser),
            ToolbarItem::custom(BAR_COLORS, "Colours", WELL_PT * COLORS.len() as f32 + SPACE_S * 2.0),
            ToolbarItem::flexible_space(),
            ToolbarItem::button(BAR_UNDO, "Undo").in_group(2).enabled(!self.undo.is_empty()),
            ToolbarItem::button(BAR_CLEAR, "Clear").in_group(2),
            ToolbarItem::icon(BAR_SAVE, "Save to Photos", "square.and.arrow.up"),
        ])
    }

    /// The toolbar buttons as semantic controls (window points), for the shell's drag band and accessibility.
    pub fn controls(&self) -> Vec<Node> {
        let (Some(placed), bar) = (&self.placed, self.bar()) else { return Vec::new() };
        placed
            .visible
            .iter()
            .filter_map(|(id, r)| {
                let item = bar.items.iter().find(|i| i.id == *id)?;
                Some(Node { id: *id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: *r, enabled: item.enabled })
            })
            .collect()
    }

    fn canvas_top(&self, ui: &Ui) -> i32 {
        self.bar_h(ui) as i32
    }

    /// (Re)allocates the canvas for the current pane, keeping what fits.
    fn ensure_canvas(&mut self, ui: &Ui) {
        let nw = self.w.max(1) as usize;
        let nh = (self.h - self.canvas_top(ui)).max(1) as usize;
        if nw == self.cw && nh == self.ch && !self.canvas.is_empty() {
            return;
        }
        let remap = |old: &Vec<u32>, ow: usize, oh: usize| {
            let mut px = vec![PAPER; nw * nh];
            for y in 0..oh.min(nh) {
                for x in 0..ow.min(nw) {
                    px[y * nw + x] = old[y * ow + x];
                }
            }
            px
        };
        if self.canvas.is_empty() {
            self.canvas = vec![PAPER; nw * nh];
        } else {
            self.canvas = remap(&self.canvas, self.cw, self.ch);
            let (ow, oh) = (self.cw, self.ch);
            for snap in &mut self.undo {
                *snap = remap(snap, ow, oh);
            }
        }
        self.cw = nw;
        self.ch = nh;
    }

    fn radius(&self, ui: &Ui) -> f32 {
        match self.tool {
            Tool::Pen => ui.pt(SPACE_XS),
            Tool::Marker => ui.pt(SPACE_M),
            Tool::Eraser => ui.pt(SPACE_L),
        }
    }

    fn push_undo(&mut self) {
        if self.undo.len() >= UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.undo.push(self.canvas.clone());
    }

    /// Stamps a filled disc at canvas coordinates (x, y).
    fn stamp(&mut self, x: f32, y: f32, r: f32) {
        let color = match self.tool {
            Tool::Eraser => PAPER,
            _ => COLORS[self.color],
        };
        let x0 = ((x - r).floor() as i32).max(0);
        let x1 = ((x + r).ceil() as i32).min(self.cw as i32 - 1);
        let y0 = ((y - r).floor() as i32).max(0);
        let y1 = ((y + r).ceil() as i32).min(self.ch as i32 - 1);
        for py in y0..=y1 {
            for px in x0..=x1 {
                let d = ((px as f32 + 0.5 - x).powi(2) + (py as f32 + 0.5 - y).powi(2)).sqrt();
                let cover = (r + 0.5 - d).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let i = py as usize * self.cw + px as usize;
                match self.tool {
                    // The marker is translucent but never darkens within a
                    // stroke: it blends against the pre-stroke snapshot.
                    Tool::Marker => {
                        let base = self.undo.last().and_then(|s| s.get(i)).copied().unwrap_or(PAPER);
                        let a = (96.0 * cover) as u32;
                        let want = mix(base, color, a);
                        // keep the stronger of the existing and the new mix
                        let cur = self.canvas[i];
                        if lum(want) < lum(cur) || cur == base {
                            self.canvas[i] = want;
                        }
                    }
                    _ => self.canvas[i] = mix(self.canvas[i], color, (255.0 * cover) as u32),
                }
            }
        }
    }

    fn stroke_to(&mut self, ui: &Ui, x: f32, y: f32) {
        let (cx, cy) = (x, y - self.canvas_top(ui) as f32);
        let r = self.radius(ui);
        match self.last {
            Some((lx, ly)) => {
                let dist = ((cx - lx).powi(2) + (cy - ly).powi(2)).sqrt();
                let steps = (dist / (r * 0.4).max(1.0)).ceil().max(1.0) as usize;
                for i in 1..=steps {
                    let f = i as f32 / steps as f32;
                    self.stamp(lx + (cx - lx) * f, ly + (cy - ly) * f, r);
                }
            }
            None => self.stamp(cx, cy, r),
        }
        self.last = Some((cx, cy));
        self.dirty = true;
    }

    /// The control under pixel (x, y); a colour well inside the colour slot resolves to its colour.
    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let (px, py) = (x / ui.s, y / ui.s);
        let id = self.placed.as_ref()?.hit(px, py)?;
        Some(match id {
            BAR_PEN => Target::Tool(Tool::Pen),
            BAR_MARKER => Target::Tool(Tool::Marker),
            BAR_ERASER => Target::Tool(Tool::Eraser),
            BAR_COLORS => {
                let r = self.placed.as_ref()?.rect_of(BAR_COLORS)?;
                let i = ((px - r.x - SPACE_S) / WELL_PT).floor();
                if i < 0.0 || i >= COLORS.len() as f32 {
                    return None;
                }
                Target::Color(i as usize)
            }
            BAR_UNDO => Target::Undo,
            BAR_CLEAR => Target::Clear,
            BAR_SAVE => Target::Save,
            _ => return None,
        })
    }

    /// Activates toolbar control `id` (from the shell's control list).
    pub fn activate_id(&mut self, id: u64) -> bool {
        let t = match id as u32 {
            BAR_PEN => Target::Tool(Tool::Pen),
            BAR_MARKER => Target::Tool(Tool::Marker),
            BAR_ERASER => Target::Tool(Tool::Eraser),
            BAR_UNDO => Target::Undo,
            BAR_CLEAR => Target::Clear,
            BAR_SAVE => Target::Save,
            _ => return false,
        };
        self.activate(t);
        true
    }

    fn activate(&mut self, t: Target) {
        match t {
            Target::Tool(tool) => self.tool = tool,
            Target::Color(i) => {
                self.color = i;
                if self.tool == Tool::Eraser {
                    self.tool = Tool::Pen;
                }
            }
            Target::Undo => {
                if let Some(prev) = self.undo.pop() {
                    self.canvas = prev;
                }
            }
            Target::Clear => {
                self.push_undo();
                self.canvas.iter_mut().for_each(|p| *p = PAPER);
            }
            Target::Save => {
                let msg = match self.save() {
                    Ok(name) => format!("Saved {name} to Photos"),
                    Err(e) => format!("Could not save: {e}"),
                };
                self.toast = Some((msg, self.now + 2500.0));
            }
        }
        self.dirty = true;
    }

    fn save(&self) -> Result<String, String> {
        let dir = self.save_dir.as_ref().ok_or("no Photos folder")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let name = free_name(dir, millis);
        let png = springboard::pngenc::encode_rgb(self.cw, self.ch, &self.canvas);
        springboard::shell::fsutil::write_atomic(&dir.join(&name), png).map_err(|e| e.to_string())?;
        Ok(name)
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) {
        self.now = t;
        self.ensure_canvas(ui);
        let top = self.canvas_top(ui) as f32;
        match event {
            Event::Down(x, y) => {
                if y >= top {
                    self.push_undo();
                    self.drawing = true;
                    self.last = None;
                    self.stroke_to(ui, x, y);
                } else {
                    self.pressed = self.hit(ui, x, y);
                }
                self.dirty = true;
            }
            Event::Move(x, y) => {
                if self.drawing {
                    self.stroke_to(ui, x, y.max(top));
                } else if self.pressed.is_some() && self.hit(ui, x, y) != self.pressed {
                    self.pressed = None;
                    self.dirty = true;
                }
            }
            Event::Up(x, y) => {
                if self.drawing {
                    self.drawing = false;
                    self.last = None;
                } else if let Some(p) = self.pressed.take() {
                    if self.hit(ui, x, y) == Some(p) {
                        self.activate(p);
                    }
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, _ui: &Ui, t: f64) -> bool {
        self.now = t;
        if matches!(&self.toast, Some((_, until)) if t >= *until) {
            self.toast = None;
            self.dirty = true;
        }
        self.dirty
    }

    pub fn next_wake(&self, _t: f64) -> Option<f64> {
        self.toast.as_ref().map(|(_, until)| *until)
    }

    pub fn describe(&self) -> String {
        format!(
            "freeform_tool={:?} freeform_color={} freeform_undo={} freeform_canvas={}x{}",
            self.tool,
            self.color,
            self.undo.len(),
            self.cw,
            self.ch
        )
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, _t: f64) {
        self.dirty = false;
        self.scale = ui.s;
        self.ensure_canvas(ui);
        if self.cache.len() > 64 {
            self.cache.clear();
        }
        let top = self.canvas_top(ui);
        for row in 0..self.ch {
            let y = top + row as i32;
            if y < 0 || y >= self.h {
                break;
            }
            for col in 0..self.cw {
                *s.at(col, y as usize) = self.canvas[row * self.cw + col];
            }
        }
        widgets::toolbar_band(s, ui, Rect { x: 0.0, y: 0.0, w: self.w as f32 / ui.s, h: UNIFIED_BAR_PT });
        let pressed = match self.pressed {
            Some(Target::Tool(Tool::Pen)) => Some(BAR_PEN),
            Some(Target::Tool(Tool::Marker)) => Some(BAR_MARKER),
            Some(Target::Tool(Tool::Eraser)) => Some(BAR_ERASER),
            Some(Target::Undo) => Some(BAR_UNDO),
            Some(Target::Clear) => Some(BAR_CLEAR),
            Some(Target::Save) => Some(BAR_SAVE),
            _ => None,
        };
        let placed = widgets::toolbar(s, ui, self.bar_bounds(ui), &self.bar(), pressed);
        if let Some(r) = placed.rect_of(BAR_COLORS) {
            let d = WELL_PT - SPACE_S;
            for (i, c) in COLORS.iter().enumerate() {
                let (x, y) = (r.x + SPACE_S + i as f32 * WELL_PT + SPACE_XS, r.y + (r.h - d) / 2.0);
                if self.color == i && self.tool != Tool::Eraser {
                    s.fill_rrect(ui.pt(x - 3.0), ui.pt(y - 3.0), ui.pt(d + 6.0), ui.pt(d + 6.0), ui.pt(d + 6.0) / 2.0, LABEL.now());
                    s.fill_rrect(ui.pt(x - 1.5), ui.pt(y - 1.5), ui.pt(d + 3.0), ui.pt(d + 3.0), ui.pt(d + 3.0) / 2.0, PAPER);
                }
                s.fill_rrect(ui.pt(x), ui.pt(y), ui.pt(d), ui.pt(d), ui.pt(d) / 2.0, *c);
            }
        }
        self.placed = Some(placed);
        if let Some((msg, _)) = self.toast.clone() {
            let img = self.cache.get(ui, &msg, MAC_CALLOUT_PT, Weight::Regular, WHITE);
            let pw = img.w as f32 + ui.pt(SPACE_XXXL);
            let ph = img.h as f32 + ui.pt(SPACE_L);
            let x = (self.w as f32 - pw) / 2.0;
            let y = self.h as f32 - ph - ui.pt(SPACE_XXL);
            s.fill_rrect(x, y, pw, ph, 1.0e4, TOAST_FILL);
            s.blit(img, (x + ui.pt(SPACE_L)) as i32, (y + ui.pt(SPACE_S)) as i32, 255, 255);
        }
    }
}

/// Rough brightness for the marker's "keep the stronger mix" rule.
fn lum(c: u32) -> u32 {
    (c & 255) + ((c >> 8) & 255) + ((c >> 16) & 255)
}

/// `Freeform-<ms>.png`, numbered `-2`, `-3`... when that name is taken, so a
/// second save never overwrites an earlier one.
fn free_name(dir: &std::path::Path, millis: u128) -> String {
    let mut name = format!("Freeform-{millis}.png");
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("Freeform-{millis}-{n}.png");
        n += 1;
    }
    name
}

#[cfg(test)]
mod name_tests {
    use super::free_name;

    #[test]
    fn a_taken_name_gets_a_number_instead_of_being_overwritten() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-freeform"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_name(&dir, 42), "Freeform-42.png");
        std::fs::write(dir.join("Freeform-42.png"), b"x").unwrap();
        assert_eq!(free_name(&dir, 42), "Freeform-42-2.png");
        std::fs::write(dir.join("Freeform-42-2.png"), b"x").unwrap();
        assert_eq!(free_name(&dir, 42), "Freeform-42-3.png");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod model_tests {
    use super::*;

    fn test_ui() -> Ui {
        use bifrost_app::kit::Font;
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        let mut ui = Ui::new(Font::parse(data).unwrap(), 1200.0 / 768.0);
        ui.symbols = springboard::symbols::Symbols::load(dir);
        ui
    }

    #[test]
    fn mix_returns_either_colour_at_the_ends() {
        assert_eq!(mix(0x1122_3344, 0xAABB_CCDD, 0), 0x1122_3344);
        assert_eq!(mix(0x1122_3344, 0xAABB_CCDD, 255), 0xAABB_CCDD);
    }

    #[test]
    fn mix_at_half_is_the_byte_midpoint() {
        assert_eq!(mix(0x0000_0000, 0xFFFF_FFFF, 128), 0x8080_8080);
    }

    #[test]
    fn lum_sums_the_three_colour_channels() {
        assert_eq!(lum(0x0001_0203), 6);
        assert_eq!(lum(0x0000_FF00), 255);
    }

    #[test]
    fn a_new_canvas_starts_empty_with_the_pen_in_the_first_colour() {
        let ui = test_ui();
        let app = Freeform::new(&ui, 800, 600);
        assert_eq!(app.describe(), "freeform_tool=Pen freeform_color=0 freeform_undo=0 freeform_canvas=0x0");
        assert!(app.undo.is_empty());
        assert!(app.save_dir.is_none());
    }
}
