//! Board items other than ink: text boxes, sticky notes, tables, links, files and pictures, and the connectors that
//! join two of them. Items live in canvas pixels, like strokes. They are painted under the ink, and a connector is
//! stored by the ids of its two ends, so it follows an item that moves. Everything here is plain data and painting
//! into the pixel canvas, so the host tests cover it; `freeform.rs` owns the gestures and the keyboard.
use crate::ink;
use bifrost_app::controls::Rect;
use bifrost_app::kit::ui::{Ui, Weight};
use bifrost_app::kit::Image;
use std::collections::HashMap;

/// Most characters one text field (a text box, a note, a link, a table cell) takes.
pub const MAX_TEXT_CHARS: usize = 4000;
/// Most rows or columns a table has.
pub const MAX_TABLE_SIDE: usize = 12;
/// Most items and connectors a board file is read back with.
const MAX_ITEMS: usize = 2000;
const MAX_CONNECTORS: usize = 4000;

/// Type sizes and spacing (points).
pub const TEXT_PT: f32 = 18.0;
pub const NOTE_PT: f32 = 15.0;
pub const CELL_PT: f32 = 13.0;
const CARD_TITLE_PT: f32 = 13.0;
const CARD_SUB_PT: f32 = 11.0;
const PAD_PT: f32 = 8.0;
const CELL_PAD_PT: f32 = 5.0;
const MIN_TEXT_H_PT: f32 = 32.0;
/// Smallest and largest side an item is scaled to (points).
const MIN_SIDE_PT: f32 = 24.0;
const MAX_SIDE_PT: f32 = 2400.0;

/// An opaque canvas pixel from its colour channels (the canvas packs red in the low byte).
pub const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    0xFF00_0000 | (b as u32) << 16 | (g as u32) << 8 | r as u32
}

const INK: u32 = rgb(0, 0, 0);
const GRID: u32 = rgb(142, 142, 147);
const SHADOW: u32 = rgb(206, 206, 210);
const CARD_FILL: u32 = rgb(242, 242, 247);
const LINK_FILL: u32 = rgb(232, 241, 254);
const LINK_INK: u32 = rgb(10, 100, 220);
const SUB_INK: u32 = rgb(99, 99, 102);
const CONNECTOR_INK: u32 = rgb(60, 60, 67);
const TABLE_FILL: u32 = rgb(255, 255, 255);

/// The pastel a sticky note takes for each of the board's seven ink colours, in the palette's order.
pub const NOTE_COLORS: [u32; 7] = [
    rgb(229, 229, 234),
    rgb(255, 199, 199),
    rgb(255, 218, 170),
    rgb(255, 240, 150),
    rgb(198, 236, 190),
    rgb(190, 220, 255),
    rgb(223, 205, 250),
];

/// What an item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Free text on the board.
    Text,
    /// A coloured square with text.
    Sticky,
    /// A grid of text cells.
    Table,
    /// A web address, shown as a card.
    Link,
    /// A file kept with the board, shown as a card.
    File,
    /// A picture kept with the board.
    Picture,
}

impl Kind {
    fn tag(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Sticky => "sticky",
            Kind::Table => "table",
            Kind::Link => "link",
            Kind::File => "file",
            Kind::Picture => "picture",
        }
    }

    fn from_tag(tag: &str) -> Option<Kind> {
        Some(match tag {
            "text" => Kind::Text,
            "sticky" => Kind::Sticky,
            "table" => Kind::Table,
            "link" => Kind::Link,
            "file" => Kind::File,
            "picture" => Kind::Picture,
            _ => return None,
        })
    }
}

/// One thing on the board.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Unique on its board; `Items::add` sets it.
    pub id: u32,
    pub kind: Kind,
    /// The box in canvas pixels.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// A note's fill, or a text box's ink (an opaque canvas pixel).
    pub color: u32,
    /// The text of a text box or note, the address of a link, the name of a file.
    pub text: String,
    /// For a file or a picture, its name in the board's `files` folder.
    pub file: String,
    /// For a file, its size in bytes.
    pub size: u64,
    pub rows: usize,
    pub cols: usize,
    /// A table's cells, row by row (`rows * cols`).
    pub cells: Vec<String>,
}

impl Item {
    pub fn new(kind: Kind, x: f32, y: f32, w: f32, h: f32) -> Item {
        Item { id: 0, kind, x, y, w, h, color: INK, text: String::new(), file: String::new(), size: 0, rows: 0, cols: 0, cells: Vec::new() }
    }

    /// An empty table of `rows` by `cols` cells (each at least one).
    pub fn table(x: f32, y: f32, w: f32, h: f32, rows: usize, cols: usize) -> Item {
        let (rows, cols) = (rows.clamp(1, MAX_TABLE_SIDE), cols.clamp(1, MAX_TABLE_SIDE));
        Item { rows, cols, cells: vec![String::new(); rows * cols], ..Item::new(Kind::Table, x, y, w, h) }
    }

    pub fn rect(&self) -> Rect {
        Rect { x: self.x, y: self.y, w: self.w, h: self.h }
    }

    pub fn centre(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }

    /// Whether the keyboard can type into the item.
    pub fn editable(&self) -> bool {
        matches!(self.kind, Kind::Text | Kind::Sticky | Kind::Table | Kind::Link)
    }

    /// The cell of a table under (px, py).
    pub fn cell_at(&self, px: f32, py: f32) -> Option<usize> {
        if self.kind != Kind::Table || self.rows == 0 || self.cols == 0 || !self.contains(px, py) {
            return None;
        }
        let col = (((px - self.x) / self.w * self.cols as f32) as usize).min(self.cols - 1);
        let row = (((py - self.y) / self.h * self.rows as f32) as usize).min(self.rows - 1);
        Some(row * self.cols + col)
    }

    /// The text being edited in `cell` (a table's cell, or the item's own text).
    pub fn cell_text(&self, cell: usize) -> &str {
        match self.kind {
            Kind::Table => self.cells.get(cell).map(String::as_str).unwrap_or(""),
            _ => &self.text,
        }
    }

    fn buffer(&mut self, cell: usize) -> Option<&mut String> {
        match self.kind {
            Kind::Table => self.cells.get_mut(cell),
            Kind::Text | Kind::Sticky | Kind::Link => Some(&mut self.text),
            Kind::File | Kind::Picture => None,
        }
    }

    /// Whether the item holds no text at all (a table with only empty cells counts).
    pub fn is_blank(&self) -> bool {
        match self.kind {
            Kind::Table => self.cells.iter().all(|c| c.trim().is_empty()),
            Kind::Text | Kind::Sticky | Kind::Link => self.text.trim().is_empty(),
            Kind::File | Kind::Picture => false,
        }
    }
}

/// A line with an arrowhead (or none) between two items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Connector {
    pub from: u32,
    pub to: u32,
    pub arrow: bool,
}

/// Every item and connector of one board, the items in drawing order (the last is on top).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Items {
    list: Vec<Item>,
    links: Vec<Connector>,
}

impl Items {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn list(&self) -> &[Item] {
        &self.list
    }

    pub fn connectors(&self) -> &[Connector] {
        &self.links
    }

    pub fn get(&self, id: u32) -> Option<&Item> {
        self.list.iter().find(|i| i.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut Item> {
        self.list.iter_mut().find(|i| i.id == id)
    }

    /// Adds `item` on top with a new id (one more than the largest) and returns the id.
    pub fn add(&mut self, mut item: Item) -> u32 {
        item.id = self.list.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        let id = item.id;
        self.list.push(item);
        id
    }

    /// Removes an item and the connectors that end at it.
    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.list.len();
        self.list.retain(|i| i.id != id);
        self.links.retain(|c| c.from != id && c.to != id);
        self.list.len() != before
    }

    /// The topmost item under (x, y).
    pub fn hit(&self, x: f32, y: f32) -> Option<u32> {
        self.list.iter().rev().find(|i| i.contains(x, y)).map(|i| i.id)
    }

    pub fn move_by(&mut self, id: u32, dx: f32, dy: f32) {
        if let Some(i) = self.get_mut(id) {
            i.x += dx;
            i.y += dy;
        }
    }

    /// Scales an item about its centre by `factor`, keeping each side between the smallest and largest an item takes.
    /// `unit` is one point in pixels.
    pub fn scale(&mut self, id: u32, factor: f32, unit: f32) {
        let Some(i) = self.get_mut(id) else { return };
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let (cx, cy) = i.centre();
        let w = (i.w * factor).clamp(MIN_SIDE_PT * unit, MAX_SIDE_PT * unit);
        let h = (i.h * factor).clamp(MIN_SIDE_PT * unit, MAX_SIDE_PT * unit);
        i.x = cx - w / 2.0;
        i.y = cy - h / 2.0;
        i.w = w;
        i.h = h;
    }

    /// Whether `connect` would join the two items.
    pub fn can_connect(&self, from: u32, to: u32) -> bool {
        from != to
            && self.get(from).is_some()
            && self.get(to).is_some()
            && !self.links.iter().any(|c| (c.from == from && c.to == to) || (c.from == to && c.to == from))
    }

    /// Joins two items. Refused (`None`) for the same item twice, an unknown item, or a pair that is joined already
    /// (in either direction). Returns the new connector's index.
    pub fn connect(&mut self, from: u32, to: u32, arrow: bool) -> Option<usize> {
        if !self.can_connect(from, to) {
            return None;
        }
        self.links.push(Connector { from, to, arrow });
        Some(self.links.len() - 1)
    }

    pub fn remove_connector(&mut self, index: usize) -> bool {
        if index < self.links.len() {
            self.links.remove(index);
            true
        } else {
            false
        }
    }

    /// The two ends of a connector: where its line leaves one item and enters the other.
    pub fn connector_line(&self, c: &Connector) -> Option<((f32, f32), (f32, f32))> {
        let (a, b) = (self.get(c.from)?, self.get(c.to)?);
        Some((edge_point(a.rect(), b.centre()), edge_point(b.rect(), a.centre())))
    }

    /// The connector whose line passes within `reach` of (x, y).
    pub fn connector_at(&self, x: f32, y: f32, reach: f32) -> Option<usize> {
        self.links.iter().position(|c| self.connector_line(c).is_some_and(|(a, b)| segment_distance((x, y), a, b) <= reach))
    }

    /// Types `text` into the item (or one cell of a table) at the caret, character `at`, which is clamped to the
    /// text's length (`usize::MAX` types at the end). Control characters are dropped; a line break is kept only where
    /// the field has lines (a text box or a note). Returns the caret after the new text, or None when nothing was added.
    pub fn insert_text(&mut self, id: u32, cell: usize, at: usize, text: &str) -> Option<usize> {
        let item = self.get_mut(id)?;
        let multiline = matches!(item.kind, Kind::Text | Kind::Sticky);
        let buf = item.buffer(cell)?;
        let len = buf.chars().count();
        let room = MAX_TEXT_CHARS.saturating_sub(len);
        let add: String = text.chars().filter(|c| (*c == '\n' && multiline) || !c.is_control()).take(room).collect();
        if add.is_empty() {
            return None;
        }
        let at = at.min(len);
        let byte = buf.char_indices().nth(at).map_or(buf.len(), |(b, _)| b);
        buf.insert_str(byte, &add);
        Some(at + add.chars().count())
    }

    /// Deletes the character before the caret, character `at` (clamped to the text's length). Returns the caret after
    /// the deletion, or None when the caret is at the start.
    pub fn backspace(&mut self, id: u32, cell: usize, at: usize) -> Option<usize> {
        let buf = self.get_mut(id)?.buffer(cell)?;
        let at = at.min(buf.chars().count());
        if at == 0 {
            return None;
        }
        let start = buf.char_indices().nth(at - 1).map(|(b, _)| b)?;
        buf.remove(start);
        Some(at - 1)
    }

    /// The number of characters in the text being edited in `cell` (0 for an unknown item).
    pub fn text_len(&self, id: u32, cell: usize) -> usize {
        self.get(id).map_or(0, |i| i.cell_text(cell).chars().count())
    }

    /// Adds a row at the bottom of a table, as tall as the others.
    pub fn add_row(&mut self, id: u32) -> bool {
        let Some(t) = self.get_mut(id) else { return false };
        if t.kind != Kind::Table || t.rows >= MAX_TABLE_SIDE {
            return false;
        }
        t.h += t.h / t.rows as f32;
        t.rows += 1;
        for _ in 0..t.cols {
            t.cells.push(String::new());
        }
        true
    }

    /// Adds a column at the right of a table, as wide as the others.
    pub fn add_column(&mut self, id: u32) -> bool {
        let Some(t) = self.get_mut(id) else { return false };
        if t.kind != Kind::Table || t.cols >= MAX_TABLE_SIDE {
            return false;
        }
        let mut cells = Vec::with_capacity(t.rows * (t.cols + 1));
        for r in 0..t.rows {
            for c in 0..t.cols {
                cells.push(t.cells.get(r * t.cols + c).cloned().unwrap_or_default());
            }
            cells.push(String::new());
        }
        t.w += t.w / t.cols as f32;
        t.cols += 1;
        t.cells = cells;
        true
    }

    /// An independent copy of item `id`, moved by (dx, dy) and put on top. Connectors are not copied. The caller
    /// gives a copied file or picture its own file name (`file`).
    pub fn duplicate(&mut self, id: u32, dx: f32, dy: f32) -> Option<u32> {
        let mut copy = self.get(id)?.clone();
        copy.x += dx;
        copy.y += dy;
        Some(self.add(copy))
    }

    /// The items as they appear with the board magnified by `zoom` and shifted by `pan` (canvas pixels): every box
    /// moves and scales, the text stays. The board's own items are not changed.
    pub fn viewed(&self, zoom: f32, pan: (f32, f32)) -> Items {
        let mut out = self.clone();
        for i in out.list.iter_mut() {
            i.x = i.x * zoom + pan.0;
            i.y = i.y * zoom + pan.1;
            i.w *= zoom;
            i.h *= zoom;
        }
        out
    }

    /// Sets the heights of the text boxes to fit their text at the width each has.
    pub fn fit_all(&mut self, ui: &Ui) {
        for i in self.list.iter_mut() {
            fit(ui, i);
        }
    }

    /// The names of the files in the board's `files` folder that the items use.
    pub fn files(&self) -> Vec<&str> {
        self.list.iter().filter(|i| !i.file.is_empty()).map(|i| i.file.as_str()).collect()
    }

    /// The saved form: one line per item and per connector, fields separated by tabs.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for i in &self.list {
            out.push_str(&format!("item\t{}\t{}\t{}\t{}\t{}\t{}\t{:08x}\t{}\t{}", i.id, i.kind.tag(), i.x, i.y, i.w, i.h, i.color, i.rows, i.cols));
            if i.kind == Kind::Table {
                for c in &i.cells {
                    out.push('\t');
                    out.push_str(&escape(c));
                }
            } else {
                out.push_str(&format!("\t{}\t{}\t{}", escape(&i.text), escape(&i.file), i.size));
            }
            out.push('\n');
        }
        for c in &self.links {
            out.push_str(&format!("link\t{}\t{}\t{}\n", c.from, c.to, c.arrow as u8));
        }
        out
    }

    /// Reads `to_text` back. A line that does not parse is skipped, and a connector whose end is missing is dropped.
    pub fn from_text(text: &str) -> Items {
        let mut out = Items::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.first().copied() {
                Some("item") if out.list.len() < MAX_ITEMS => {
                    if let Some(item) = parse_item(&f) {
                        if !out.list.iter().any(|i| i.id == item.id) {
                            out.list.push(item);
                        }
                    }
                }
                Some("link") if f.len() >= 4 && out.links.len() < MAX_CONNECTORS => {
                    if let (Ok(from), Ok(to)) = (f[1].parse::<u32>(), f[2].parse::<u32>()) {
                        out.links.push(Connector { from, to, arrow: f[3] == "1" });
                    }
                }
                _ => {}
            }
        }
        let ids: Vec<u32> = out.list.iter().map(|i| i.id).collect();
        let mut seen: Vec<(u32, u32)> = Vec::new();
        out.links.retain(|c| {
            let key = (c.from.min(c.to), c.from.max(c.to));
            let keep = c.from != c.to && ids.contains(&c.from) && ids.contains(&c.to) && !seen.contains(&key);
            if keep {
                seen.push(key);
            }
            keep
        });
        out
    }
}

fn parse_item(f: &[&str]) -> Option<Item> {
    if f.len() < 10 {
        return None;
    }
    let num = |s: &str| s.parse::<f32>().ok().filter(|v| v.is_finite());
    let id: u32 = f[1].parse().ok().filter(|&v| v > 0)?;
    let kind = Kind::from_tag(f[2])?;
    let (x, y, w, h) = (num(f[3])?, num(f[4])?, num(f[5])?.max(4.0), num(f[6])?.max(4.0));
    let color = u32::from_str_radix(f[7], 16).ok()? | 0xFF00_0000;
    let mut item = Item { id, color, ..Item::new(kind, x, y, w, h) };
    let fields = &f[10..];
    if kind == Kind::Table {
        let rows = f[8].parse::<usize>().ok()?.clamp(1, MAX_TABLE_SIDE);
        let cols = f[9].parse::<usize>().ok()?.clamp(1, MAX_TABLE_SIDE);
        let mut cells: Vec<String> = fields.iter().map(|s| unescape(s)).collect();
        cells.resize(rows * cols, String::new());
        item.rows = rows;
        item.cols = cols;
        item.cells = cells;
    } else {
        item.text = fields.first().map(|s| unescape(s)).unwrap_or_default();
        item.file = fields.get(1).map(|s| unescape(s)).unwrap_or_default();
        item.size = fields.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    }
    Some(item)
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            _ => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Where the line from the centre of `r` toward `toward` leaves the box (the centre when `toward` is inside it).
pub fn edge_point(r: Rect, toward: (f32, f32)) -> (f32, f32) {
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (dx, dy) = (toward.0 - cx, toward.1 - cy);
    if dx.abs() < 1e-3 && dy.abs() < 1e-3 {
        return (cx, cy);
    }
    let tx = if dx.abs() < 1e-6 { f32::INFINITY } else { (r.w / 2.0) / dx.abs() };
    let ty = if dy.abs() < 1e-6 { f32::INFINITY } else { (r.h / 2.0) / dy.abs() };
    let t = tx.min(ty).min(1.0);
    (cx + dx * t, cy + dy * t)
}

/// Distance from `p` to the segment `a`-`b`.
pub fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 <= f32::EPSILON { 0.0 } else { (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + vx * t, a.1 + vy * t);
    ((p.0 - qx).powi(2) + (p.1 - qy).powi(2)).sqrt()
}

/// The host part of a web address, for a link card's title ("https://example.com/a/b" gives "example.com").
pub fn link_host(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or("").to_string()
}

/// Whether `text` is a web address a link item can open.
pub fn is_web_url(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with("http://") || t.starts_with("https://")) && !t.contains(char::is_whitespace) && link_host(t).contains('.')
}

/// "812 bytes", "12 KB", "3.4 MB".
pub fn human_size(bytes: u64) -> String {
    if bytes < 1000 {
        format!("{bytes} bytes")
    } else if bytes < 1_000_000 {
        format!("{} KB", (bytes + 500) / 1000)
    } else {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    }
}

/// A file name that is safe to keep in the board's folder: letters, digits, dot, dash and underscore only, at most
/// 64 characters, never empty and never starting with a dot.
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let mut out: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).take(64).collect();
    while out.starts_with('.') {
        out.remove(0);
    }
    if out.is_empty() {
        out.push_str("file");
    }
    out
}

/// The size a picture of `w` by `h` pixels is shown at: it keeps its shape and fits `max_w` by `max_h`, and is never
/// enlarged.
pub fn fit_picture(w: f32, h: f32, max_w: f32, max_h: f32) -> (f32, f32) {
    if !(w > 0.0 && h > 0.0) {
        return (max_w.max(1.0), max_h.max(1.0));
    }
    let k = (max_w / w).min(max_h / h).min(1.0);
    ((w * k).max(1.0), (h * k).max(1.0))
}

// ---- layout and painting ----

/// The lines `text` wraps to inside `max_px`, at least one. A line break in the text starts a new line.
pub fn lines(ui: &Ui, text: &str, pt: f32, weight: Weight, max_px: f32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let wrapped = ui.wrap(para, pt, weight, max_px);
        if wrapped.is_empty() {
            out.push(String::new());
        } else {
            out.extend(wrapped);
        }
    }
    out
}

/// The distance between two lines of text at `pt` (pixels).
pub fn line_height(ui: &Ui, pt: f32) -> f32 {
    (ui.text("Ag", pt, Weight::Regular, INK).h as f32 * 1.15).ceil().max(1.0)
}

/// Sets a text box's height to fit its wrapped text.
pub fn fit(ui: &Ui, item: &mut Item) {
    if item.kind != Kind::Text {
        return;
    }
    let pad = ui.pt(PAD_PT);
    let n = lines(ui, &item.text, TEXT_PT, Weight::Regular, (item.w - 2.0 * pad).max(1.0)).len().max(1);
    item.h = (n as f32 * line_height(ui, TEXT_PT) + 2.0 * pad).max(ui.pt(MIN_TEXT_H_PT));
}

fn fill(px: &mut [u32], cw: usize, ch: usize, r: Rect, color: u32) {
    let x0 = r.x.round().max(0.0) as usize;
    let y0 = r.y.round().max(0.0) as usize;
    let x1 = ((r.x + r.w).round().max(0.0) as usize).min(cw);
    let y1 = ((r.y + r.h).round().max(0.0) as usize).min(ch);
    for y in y0..y1 {
        if let Some(row) = px.get_mut(y * cw + x0..y * cw + x1) {
            row.fill(color);
        }
    }
}

fn frame(px: &mut [u32], cw: usize, ch: usize, r: Rect, t: f32, color: u32) {
    fill(px, cw, ch, Rect { x: r.x, y: r.y, w: r.w, h: t }, color);
    fill(px, cw, ch, Rect { x: r.x, y: r.y + r.h - t, w: r.w, h: t }, color);
    fill(px, cw, ch, Rect { x: r.x, y: r.y, w: t, h: r.h }, color);
    fill(px, cw, ch, Rect { x: r.x + r.w - t, y: r.y, w: t, h: r.h }, color);
}

/// `src` (premultiplied) over the opaque pixel `dst`.
fn over_opaque(dst: u32, src: u32) -> u32 {
    let a = src >> 24;
    if a == 255 {
        return src;
    }
    let inv = 255 - a;
    let mut out = 0xFF00_0000;
    for shift in [0u32, 8, 16] {
        let c = ((src >> shift) & 255) + (((dst >> shift) & 255) * inv + 127) / 255;
        out |= c.min(255) << shift;
    }
    out
}

/// Draws `img` with its top left at (x, y), clipped to `clip` and to the canvas.
fn blit(px: &mut [u32], cw: usize, ch: usize, img: &Image, x: i32, y: i32, clip: Rect) {
    let cx0 = clip.x.floor().max(0.0) as i32;
    let cy0 = clip.y.floor().max(0.0) as i32;
    let cx1 = ((clip.x + clip.w).ceil() as i32).min(cw as i32);
    let cy1 = ((clip.y + clip.h).ceil() as i32).min(ch as i32);
    for iy in 0..img.h {
        let dy = y + iy as i32;
        if dy < cy0 || dy >= cy1 {
            continue;
        }
        for ix in 0..img.w {
            let dx = x + ix as i32;
            if dx < cx0 || dx >= cx1 {
                continue;
            }
            let s = img.px[iy * img.w + ix];
            if s >> 24 == 0 {
                continue;
            }
            let d = &mut px[dy as usize * cw + dx as usize];
            *d = over_opaque(*d, s);
        }
    }
}

/// Draws wrapped text inside `r` (inset by `pad`), clipped to `r`. With `caret` (a character index, clamped to the text)
/// a bar is drawn where that character would go: the end of the wrapped text before it.
#[allow(clippy::too_many_arguments)]
fn draw_text(px: &mut [u32], cw: usize, ch: usize, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, r: Rect, pad: f32, caret: Option<usize>) {
    let width = (r.w - 2.0 * pad).max(1.0);
    let lh = line_height(ui, pt);
    for (i, l) in lines(ui, text, pt, weight, width).iter().enumerate() {
        if !l.is_empty() {
            let img = ui.text(l, pt, weight, color);
            blit(px, cw, ch, &img, (r.x + pad).round() as i32, (r.y + pad + i as f32 * lh).round() as i32, r);
        }
    }
    if let Some(at) = caret {
        let before: String = text.chars().take(at).collect();
        let wrapped = lines(ui, &before, pt, weight, width);
        let row = wrapped.len().saturating_sub(1);
        let last = wrapped.last().map_or("", String::as_str);
        let end = (r.x + pad + ui.measure(last, pt, weight), r.y + pad + row as f32 * lh);
        fill(px, cw, ch, Rect { x: end.0 + ui.pt(1.0), y: end.1, w: ui.pt(1.5).max(1.0), h: lh }, color);
    }
}

/// One line of text cut to fit `max_px`, drawn with its top left at (x, y) and clipped to `clip`.
#[allow(clippy::too_many_arguments)]
fn draw_line(px: &mut [u32], cw: usize, ch: usize, ui: &Ui, text: &str, pt: f32, weight: Weight, color: u32, x: f32, y: f32, max_px: f32, clip: Rect) {
    let shown = ui.fit(text, pt, max_px);
    if shown.is_empty() {
        return;
    }
    let img = ui.text(&shown, pt, weight, color);
    blit(px, cw, ch, &img, x.round() as i32, y.round() as i32, clip);
}

/// Paints every item and connector into `px`, a `cw` x `ch` canvas, over whatever is there. `pictures` holds each
/// picture item's image already scaled to the item's size; `editing` is the item and cell that has the caret, and
/// `caret` the character the caret is before in that text (`usize::MAX` for the end). The items' boxes are in canvas
/// pixels; `zoom` is how much the board is magnified, so type, padding and line weights scale with the boxes (1 draws
/// them at their own size).
#[allow(clippy::too_many_arguments)]
pub fn paint(px: &mut [u32], cw: usize, ch: usize, ui: &Ui, items: &Items, pictures: &HashMap<u32, Image>, editing: Option<(u32, usize)>, caret: usize, zoom: f32) {
    let z = zoom;
    for it in items.list() {
        let r = it.rect();
        let caret_cell = editing.filter(|(id, _)| *id == it.id).map(|(_, cell)| cell);
        match it.kind {
            Kind::Text => draw_text(px, cw, ch, ui, &it.text, TEXT_PT * z, Weight::Regular, it.color, r, ui.pt(PAD_PT) * z, caret_cell.map(|_| caret)),
            Kind::Sticky => {
                let off = Rect { x: r.x + ui.pt(2.0) * z, y: r.y + ui.pt(3.0) * z, w: r.w, h: r.h };
                fill(px, cw, ch, off, SHADOW);
                fill(px, cw, ch, r, it.color);
                draw_text(px, cw, ch, ui, &it.text, NOTE_PT * z, Weight::Regular, INK, r, ui.pt(PAD_PT) * z, caret_cell.map(|_| caret));
            }
            Kind::Table => paint_table(px, cw, ch, ui, it, caret_cell, caret, z),
            Kind::Link => paint_link(px, cw, ch, ui, it, caret_cell.map(|_| caret), z),
            Kind::File => paint_file(px, cw, ch, ui, it, z),
            Kind::Picture => match pictures.get(&it.id) {
                Some(img) => blit(px, cw, ch, img, it.x.round() as i32, it.y.round() as i32, r),
                None => {
                    fill(px, cw, ch, r, CARD_FILL);
                    frame(px, cw, ch, r, ui.pt(1.0).max(1.0), GRID);
                    let pad = ui.pt(PAD_PT) * z;
                    draw_line(px, cw, ch, ui, "Picture", CARD_TITLE_PT * z, Weight::Regular, SUB_INK, r.x + pad, r.y + pad, r.w - 2.0 * pad, r);
                }
            },
        }
    }
    paint_connectors(px, cw, ch, ui, items, None, z);
}

#[allow(clippy::too_many_arguments)]
fn paint_table(px: &mut [u32], cw: usize, ch: usize, ui: &Ui, it: &Item, caret_cell: Option<usize>, caret: usize, z: f32) {
    let r = it.rect();
    fill(px, cw, ch, r, TABLE_FILL);
    let line = ui.pt(1.0).max(1.0);
    let (cell_w, cell_h) = (r.w / it.cols.max(1) as f32, r.h / it.rows.max(1) as f32);
    for row in 0..it.rows {
        for col in 0..it.cols {
            let cell = Rect { x: r.x + col as f32 * cell_w, y: r.y + row as f32 * cell_h, w: cell_w, h: cell_h };
            let text = it.cells.get(row * it.cols + col).map(String::as_str).unwrap_or("");
            let editing = caret_cell == Some(row * it.cols + col);
            if !text.is_empty() || editing {
                let at = editing.then_some(caret);
                draw_text(px, cw, ch, ui, text, CELL_PT * z, Weight::Regular, INK, cell, ui.pt(CELL_PAD_PT) * z, at);
            }
        }
    }
    for col in 0..=it.cols {
        fill(px, cw, ch, Rect { x: r.x + col as f32 * cell_w - line / 2.0, y: r.y, w: line, h: r.h }, GRID);
    }
    for row in 0..=it.rows {
        fill(px, cw, ch, Rect { x: r.x, y: r.y + row as f32 * cell_h - line / 2.0, w: r.w, h: line }, GRID);
    }
}

fn paint_link(px: &mut [u32], cw: usize, ch: usize, ui: &Ui, it: &Item, caret: Option<usize>, z: f32) {
    let r = it.rect();
    let pad = ui.pt(PAD_PT) * z;
    fill(px, cw, ch, r, LINK_FILL);
    frame(px, cw, ch, r, ui.pt(1.0).max(1.0), LINK_INK);
    if let Some(caret) = caret {
        draw_text(px, cw, ch, ui, &it.text, CARD_SUB_PT * z, Weight::Regular, LINK_INK, r, pad, Some(caret));
        return;
    }
    let max = r.w - 2.0 * pad;
    let host = link_host(&it.text);
    let title = if host.is_empty() { "Link".to_string() } else { host };
    draw_line(px, cw, ch, ui, &title, CARD_TITLE_PT * z, Weight::Semibold, LINK_INK, r.x + pad, r.y + pad, max, r);
    let sub_y = r.y + pad + line_height(ui, CARD_TITLE_PT * z);
    draw_line(px, cw, ch, ui, &it.text, CARD_SUB_PT * z, Weight::Regular, SUB_INK, r.x + pad, sub_y, max, r);
}

fn paint_file(px: &mut [u32], cw: usize, ch: usize, ui: &Ui, it: &Item, z: f32) {
    let r = it.rect();
    let pad = ui.pt(PAD_PT) * z;
    fill(px, cw, ch, r, CARD_FILL);
    frame(px, cw, ch, r, ui.pt(1.0).max(1.0), GRID);
    let max = r.w - 2.0 * pad;
    draw_line(px, cw, ch, ui, &it.text, CARD_TITLE_PT * z, Weight::Semibold, INK, r.x + pad, r.y + pad, max, r);
    let sub_y = r.y + pad + line_height(ui, CARD_TITLE_PT * z);
    draw_line(px, cw, ch, ui, &human_size(it.size), CARD_SUB_PT * z, Weight::Regular, SUB_INK, r.x + pad, sub_y, max, r);
}

/// Paints the connectors; the one at index `picked` is drawn thicker. `z` is the board's magnification.
pub fn paint_connectors(px: &mut [u32], cw: usize, ch: usize, ui: &Ui, items: &Items, picked: Option<usize>, z: f32) {
    for (n, c) in items.connectors().iter().enumerate() {
        let Some((a, b)) = items.connector_line(c) else { continue };
        let width = ui.pt(if picked == Some(n) { 4.0 } else { 2.0 }) * z;
        ink::stroke_polyline(px, cw, ch, &[a, b], width, None, CONNECTOR_INK, 1.0);
        if c.arrow {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt();
            if len > 1.0 {
                let (ux, uy) = (dx / len, dy / len);
                let head = (ui.pt(12.0) * z).min(len);
                let (bx, by) = (b.0 - ux * head, b.1 - uy * head);
                let (nx, ny) = (-uy * head * 0.5, ux * head * 0.5);
                ink::stroke_polyline(px, cw, ch, &[(bx + nx, by + ny), b, (bx - nx, by - ny)], width, None, CONNECTOR_INK, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::kit::Font;

    const PAPER: u32 = 0xFFFF_FFFF;

    fn test_ui() -> Ui {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    fn sticky(x: f32, y: f32) -> Item {
        Item { color: NOTE_COLORS[3], ..Item::new(Kind::Sticky, x, y, 100.0, 100.0) }
    }

    fn two_items() -> (Items, u32, u32) {
        let mut items = Items::default();
        let a = items.add(sticky(0.0, 0.0));
        let b = items.add(sticky(300.0, 0.0));
        (items, a, b)
    }

    #[test]
    fn items_get_increasing_ids_and_the_topmost_is_hit_first() {
        let mut items = Items::default();
        let a = items.add(sticky(0.0, 0.0));
        let b = items.add(sticky(50.0, 50.0));
        assert_eq!((a, b), (1, 2));
        assert_eq!(items.hit(75.0, 75.0), Some(b), "the later item is on top");
        assert_eq!(items.hit(10.0, 10.0), Some(a));
        assert_eq!(items.hit(400.0, 400.0), None);
    }

    #[test]
    fn removing_an_item_removes_its_connectors() {
        let (mut items, a, b) = two_items();
        assert_eq!(items.connect(a, b, true), Some(0));
        assert!(items.remove(a));
        assert!(items.connectors().is_empty());
        assert!(items.get(b).is_some());
        assert!(!items.remove(a), "already gone");
    }

    #[test]
    fn a_connector_needs_two_different_known_items_and_is_not_made_twice() {
        let (mut items, a, b) = two_items();
        assert_eq!(items.connect(a, a, true), None);
        assert_eq!(items.connect(a, 99, true), None);
        assert_eq!(items.connect(a, b, false), Some(0));
        assert_eq!(items.connect(b, a, true), None, "the pair is joined already");
        assert_eq!(items.connectors().len(), 1);
    }

    #[test]
    fn a_connector_leaves_each_item_at_its_edge_and_follows_a_move() {
        let (mut items, a, b) = two_items();
        items.connect(a, b, true);
        let c = items.connectors()[0];
        let (p, q) = items.connector_line(&c).unwrap();
        assert!((p.0 - 100.0).abs() < 0.01 && (q.0 - 300.0).abs() < 0.01);
        items.move_by(b, 0.0, 200.0);
        let (p2, q2) = items.connector_line(&c).unwrap();
        assert!(q2.1 > q.1, "the far end moved down with its item");
        assert!((p2.0 - 100.0).abs() < 0.01 && p2.1 > p.1, "the near end still lies on the right edge of the first item, lower down");
    }

    #[test]
    fn edge_point_cuts_the_line_at_the_box() {
        let r = Rect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
        let p = edge_point(r, (30.0, 5.0));
        assert!((p.0 - 10.0).abs() < 0.01 && (p.1 - 5.0).abs() < 0.01);
        let inside = edge_point(r, (6.0, 5.0));
        assert_eq!((inside.0, inside.1), (6.0, 5.0), "a point inside is returned unchanged");
        let same = edge_point(r, (5.0, 5.0));
        assert_eq!((same.0, same.1), (5.0, 5.0));
    }

    #[test]
    fn a_connector_is_found_by_a_point_near_its_line() {
        let (mut items, a, b) = two_items();
        items.connect(a, b, true);
        assert_eq!(items.connector_at(200.0, 52.0, 6.0), Some(0));
        assert_eq!(items.connector_at(200.0, 90.0, 6.0), None);
        assert!(items.remove_connector(0));
        assert!(!items.remove_connector(0));
    }

    #[test]
    fn segment_distance_clamps_to_the_ends() {
        assert_eq!(segment_distance((5.0, 3.0), (0.0, 0.0), (10.0, 0.0)), 3.0);
        assert_eq!(segment_distance((-4.0, 3.0), (0.0, 0.0), (10.0, 0.0)), 5.0);
        assert_eq!(segment_distance((3.0, 4.0), (0.0, 0.0), (0.0, 0.0)), 5.0, "a point-sized segment");
    }

    #[test]
    fn typing_adds_text_and_backspace_takes_it_back() {
        let (mut items, a, _) = two_items();
        assert!(items.insert_text(a, 0, usize::MAX, "Hi").is_some());
        assert!(items.insert_text(a, 0, usize::MAX, "\nthere\t!").is_some());
        assert_eq!(items.get(a).unwrap().text, "Hi\nthere!", "a note keeps line breaks and drops other control characters");
        assert!(items.backspace(a, 0, usize::MAX).is_some());
        assert_eq!(items.get(a).unwrap().text, "Hi\nthere");
        assert!(items.insert_text(99, 0, usize::MAX, "x").is_none());
    }

    #[test]
    fn typing_at_the_caret_inserts_there_and_returns_the_caret_after_the_text() {
        let (mut items, a, _) = two_items();
        items.insert_text(a, 0, usize::MAX, "ac");
        assert_eq!(items.insert_text(a, 0, 1, "b"), Some(2), "the caret moves past the new letter");
        assert_eq!(items.get(a).unwrap().text, "abc");
        assert_eq!(items.insert_text(a, 0, 0, "> "), Some(2));
        assert_eq!(items.get(a).unwrap().text, "> abc");
        assert_eq!(items.insert_text(a, 0, 99, "!"), Some(6), "a caret past the end is the end");
        assert_eq!(items.get(a).unwrap().text, "> abc!");
    }

    #[test]
    fn backspace_deletes_the_character_before_the_caret() {
        let (mut items, a, _) = two_items();
        items.insert_text(a, 0, usize::MAX, "abcd");
        assert_eq!(items.backspace(a, 0, 2), Some(1), "the b goes and the caret steps back");
        assert_eq!(items.get(a).unwrap().text, "acd");
        assert_eq!(items.backspace(a, 0, 0), None, "nothing before the start of the text");
        assert_eq!(items.get(a).unwrap().text, "acd");
    }

    #[test]
    fn the_caret_counts_characters_not_bytes() {
        let (mut items, a, _) = two_items();
        items.insert_text(a, 0, usize::MAX, "a\u{e9}c");
        assert_eq!(items.text_len(a, 0), 3, "an accented letter is one character");
        assert_eq!(items.backspace(a, 0, 2), Some(1));
        assert_eq!(items.get(a).unwrap().text, "ac");
    }

    #[test]
    fn a_link_and_a_table_cell_take_no_line_breaks() {
        let mut items = Items::default();
        let link = items.add(Item::new(Kind::Link, 0.0, 0.0, 200.0, 60.0));
        let table = items.add(Item::table(0.0, 300.0, 300.0, 90.0, 3, 3));
        items.insert_text(link, 0, usize::MAX, "https://a.b\nc");
        items.insert_text(table, 4, usize::MAX, "mid\nrow");
        assert_eq!(items.get(link).unwrap().text, "https://a.bc");
        assert_eq!(items.get(table).unwrap().cells[4], "midrow");
    }

    #[test]
    fn text_stops_at_the_limit() {
        let mut items = Items::default();
        let t = items.add(Item::new(Kind::Text, 0.0, 0.0, 200.0, 40.0));
        assert!(items.insert_text(t, 0, usize::MAX, &"a".repeat(MAX_TEXT_CHARS + 50)).is_some());
        assert_eq!(items.get(t).unwrap().text.chars().count(), MAX_TEXT_CHARS);
        assert!(items.insert_text(t, 0, usize::MAX, "more").is_none());
    }

    #[test]
    fn a_picture_takes_no_typing() {
        let mut items = Items::default();
        let p = items.add(Item::new(Kind::Picture, 0.0, 0.0, 100.0, 100.0));
        assert!(items.insert_text(p, 0, usize::MAX, "x").is_none());
        assert!(items.backspace(p, 0, usize::MAX).is_none());
        assert!(!items.get(p).unwrap().editable());
    }

    #[test]
    fn a_table_finds_the_cell_under_a_point() {
        let t = Item::table(10.0, 10.0, 300.0, 90.0, 3, 3);
        assert_eq!(t.cell_at(15.0, 15.0), Some(0));
        assert_eq!(t.cell_at(305.0, 15.0), Some(2));
        assert_eq!(t.cell_at(15.0, 95.0), Some(6));
        assert_eq!(t.cell_at(310.0, 100.0), Some(8), "the far corner is in the last cell");
        assert_eq!(t.cell_at(0.0, 0.0), None);
        assert_eq!(sticky(0.0, 0.0).cell_at(5.0, 5.0), None, "only a table has cells");
    }

    #[test]
    fn a_row_and_a_column_can_be_added_to_a_table() {
        let mut items = Items::default();
        let t = items.add(Item::table(0.0, 0.0, 300.0, 90.0, 3, 3));
        items.insert_text(t, 4, usize::MAX, "mid");
        assert!(items.add_row(t));
        let it = items.get(t).unwrap();
        assert_eq!((it.rows, it.cols, it.cells.len(), it.h), (4, 3, 12, 120.0));
        assert!(items.add_column(t));
        let it = items.get(t).unwrap();
        assert_eq!((it.rows, it.cols, it.cells.len(), it.w), (4, 4, 16, 400.0));
        assert_eq!(it.cells[5], "mid", "the middle cell kept its place: row 1, column 1 of four");
        let s = items.add(sticky(0.0, 0.0));
        assert!(!items.add_row(s) && !items.add_column(s), "only a table grows");
    }

    #[test]
    fn a_table_stops_growing_at_the_limit() {
        let mut items = Items::default();
        let t = items.add(Item::table(0.0, 0.0, 300.0, 300.0, MAX_TABLE_SIDE, MAX_TABLE_SIDE));
        assert!(!items.add_row(t));
        assert!(!items.add_column(t));
        let big = Item::table(0.0, 0.0, 10.0, 10.0, 99, 0);
        assert_eq!((big.rows, big.cols), (MAX_TABLE_SIDE, 1), "the side counts are clamped");
    }

    #[test]
    fn scaling_keeps_the_centre_and_stays_between_the_limits() {
        let mut items = Items::default();
        let a = items.add(sticky(100.0, 100.0));
        items.scale(a, 2.0, 1.0);
        let it = items.get(a).unwrap();
        assert_eq!((it.x, it.y, it.w, it.h), (50.0, 50.0, 200.0, 200.0));
        items.scale(a, 0.001, 1.0);
        assert_eq!(items.get(a).unwrap().w, MIN_SIDE_PT);
        items.scale(a, f32::NAN, 1.0);
        assert_eq!(items.get(a).unwrap().w, MIN_SIDE_PT, "a bad factor changes nothing");
    }

    #[test]
    fn a_duplicate_is_a_separate_item_on_top_without_connectors() {
        let (mut items, a, b) = two_items();
        items.connect(a, b, true);
        let copy = items.duplicate(a, 16.0, 16.0).unwrap();
        assert_eq!(items.list().len(), 3);
        assert_eq!(items.get(copy).unwrap().x, 16.0);
        assert_eq!(items.connectors().len(), 1);
        assert_eq!(items.list().last().unwrap().id, copy);
    }

    #[test]
    fn items_survive_a_text_round_trip() {
        let mut items = Items::default();
        let t = items.add(Item { color: rgb(255, 0, 0), text: "a\tb\nc \\ d".into(), ..Item::new(Kind::Text, 1.5, 2.5, 200.0, 40.0) });
        let table = items.add(Item::table(10.0, 20.0, 300.0, 90.0, 2, 2));
        items.insert_text(table, 3, usize::MAX, "x\\y");
        let file = items.add(Item { text: "report.pdf".into(), file: "3-report.pdf".into(), size: 2048, ..Item::new(Kind::File, 0.0, 0.0, 200.0, 60.0) });
        items.connect(t, table, true);
        items.connect(table, file, false);
        let back = Items::from_text(&items.to_text());
        assert_eq!(back, items);
    }

    #[test]
    fn a_damaged_board_file_loses_only_the_bad_lines() {
        let text = "item\t1\tsticky\t0\t0\t100\t100\tffffeb99\t0\t0\thello\t\t0\nitem\tx\tsticky\nitem\t2\tsticky\tnan\t0\t10\t10\t0\t0\t0\nitem\t1\ttext\t5\t5\t50\t50\t0\t0\t0\tdup\t\t0\nlink\t1\t9\t1\nlink\t1\t1\t1\nbogus\n";
        let items = Items::from_text(text);
        assert_eq!(items.list().len(), 1, "only the first sticky is whole; a repeated id is ignored");
        assert_eq!(items.get(1).unwrap().text, "hello");
        assert!(items.connectors().is_empty(), "a connector to a missing item or to itself is dropped");
    }

    #[test]
    fn a_table_read_back_has_a_cell_for_every_place() {
        let items = Items::from_text("item\t1\ttable\t0\t0\t90\t60\tff000000\t2\t3\tonly\n");
        let t = items.get(1).unwrap();
        assert_eq!((t.rows, t.cols, t.cells.len()), (2, 3, 6));
        assert_eq!(t.cells[0], "only");
    }

    #[test]
    fn blank_items_are_found() {
        let mut t = Item::table(0.0, 0.0, 90.0, 60.0, 2, 2);
        assert!(t.is_blank());
        t.cells[3] = "x".into();
        assert!(!t.is_blank());
        assert!(Item::new(Kind::Text, 0.0, 0.0, 10.0, 10.0).is_blank());
        assert!(!Item::new(Kind::Picture, 0.0, 0.0, 10.0, 10.0).is_blank());
    }

    #[test]
    fn link_helpers_read_the_address() {
        assert_eq!(link_host("https://example.com/a/b?c"), "example.com");
        assert_eq!(link_host("example.org"), "example.org");
        assert!(is_web_url("https://example.com/a"));
        assert!(is_web_url(" http://example.com "));
        assert!(!is_web_url("https://nodot"));
        assert!(!is_web_url("ftp://example.com"));
        assert!(!is_web_url("https://exa mple.com"));
    }

    #[test]
    fn sizes_and_file_names_are_tidy() {
        assert_eq!(human_size(812), "812 bytes");
        assert_eq!(human_size(12_345), "12 KB");
        assert_eq!(human_size(3_400_000), "3.4 MB");
        assert_eq!(safe_file_name("../my file (1).png"), "my_file__1_.png");
        assert_eq!(safe_file_name(".hidden"), "hidden");
        assert_eq!(safe_file_name(""), "file");
        assert_eq!(safe_file_name(&"a".repeat(200)).len(), 64);
    }

    #[test]
    fn a_picture_is_fitted_but_never_enlarged() {
        let (w, h) = fit_picture(1000.0, 500.0, 300.0, 300.0);
        assert!((w - 300.0).abs() < 0.01 && (h - 150.0).abs() < 0.01, "{w} x {h}");
        assert_eq!(fit_picture(100.0, 50.0, 300.0, 300.0), (100.0, 50.0));
        assert_eq!(fit_picture(0.0, 0.0, 300.0, 200.0), (300.0, 200.0));
    }

    #[test]
    fn a_sticky_note_paints_its_fill_and_its_text() {
        let ui = test_ui();
        let mut items = Items::default();
        let id = items.add(Item { text: "Hello".into(), ..sticky(10.0, 10.0) });
        let (cw, ch) = (200, 200);
        let mut px = vec![PAPER; cw * ch];
        paint(&mut px, cw, ch, &ui, &items, &HashMap::new(), None, usize::MAX, 1.0);
        assert_eq!(px[5 * cw + 5], PAPER, "outside the note stays paper");
        assert_eq!(px[105 * cw + 100], NOTE_COLORS[3], "the bottom of the note is its fill");
        let inked = (10..110).flat_map(|y| (10..110).map(move |x| (x, y))).any(|(x, y)| px[y * cw + x] != NOTE_COLORS[3]);
        assert!(inked, "the text marks the note");
        assert!(items.get(id).is_some());
    }

    #[test]
    fn text_outside_a_note_is_clipped_to_it() {
        let ui = test_ui();
        let mut items = Items::default();
        items.add(Item { text: "A very long line of words that cannot all fit inside a small note at this size".into(), ..Item::new(Kind::Sticky, 10.0, 10.0, 60.0, 30.0) });
        let (cw, ch) = (200, 200);
        let mut px = vec![PAPER; cw * ch];
        paint(&mut px, cw, ch, &ui, &items, &HashMap::new(), None, usize::MAX, 1.0);
        // the shadow reaches 3 px below and 2 px right of the box; nothing else is painted below it
        assert!((46..ch).all(|y| (0..cw).all(|x| px[y * cw + x] == PAPER)), "no text spills below the note");
    }

    #[test]
    fn the_caret_is_drawn_only_for_the_item_being_edited() {
        let ui = test_ui();
        let mut items = Items::default();
        let id = items.add(Item::new(Kind::Text, 10.0, 10.0, 200.0, 40.0));
        let (cw, ch) = (240, 80);
        let mut still = vec![PAPER; cw * ch];
        paint(&mut still, cw, ch, &ui, &items, &HashMap::new(), None, usize::MAX, 1.0);
        assert!(still.iter().all(|&p| p == PAPER), "an empty text box is invisible");
        let mut editing = vec![PAPER; cw * ch];
        paint(&mut editing, cw, ch, &ui, &items, &HashMap::new(), Some((id, 0)), usize::MAX, 1.0);
        assert!(editing.iter().any(|&p| p != PAPER), "the caret shows while editing");
    }

    #[test]
    fn the_caret_is_drawn_where_the_caret_is_in_the_text() {
        let ui = test_ui();
        let mut items = Items::default();
        let id = items.add(Item::new(Kind::Text, 10.0, 10.0, 200.0, 40.0));
        items.insert_text(id, 0, usize::MAX, "ab");
        let (cw, ch) = (240, 80);
        let mut at_start = vec![PAPER; cw * ch];
        paint(&mut at_start, cw, ch, &ui, &items, &HashMap::new(), Some((id, 0)), 0, 1.0);
        let mut at_end = vec![PAPER; cw * ch];
        paint(&mut at_end, cw, ch, &ui, &items, &HashMap::new(), Some((id, 0)), usize::MAX, 1.0);
        assert_ne!(at_start, at_end, "a caret before the first letter is not one after the last");
    }

    #[test]
    fn a_table_paints_its_grid_lines() {
        let ui = test_ui();
        let mut items = Items::default();
        items.add(Item::table(10.0, 10.0, 90.0, 60.0, 2, 3));
        let (cw, ch) = (120, 100);
        let mut px = vec![PAPER; cw * ch];
        paint(&mut px, cw, ch, &ui, &items, &HashMap::new(), None, usize::MAX, 1.0);
        assert_eq!(px[40 * cw + 40], GRID, "the line between the first two columns at x = 40");
        assert_eq!(px[20 * cw + 20], TABLE_FILL, "a cell is white inside");
        assert_eq!(px[90 * cw + 20], PAPER, "below the table is paper");
    }

    #[test]
    fn a_connector_with_an_arrow_marks_the_canvas_between_its_items() {
        let ui = test_ui();
        let (items, _, _) = {
            let (mut items, a, b) = two_items();
            items.connect(a, b, true);
            (items, a, b)
        };
        let (cw, ch) = (420, 120);
        let mut px = vec![PAPER; cw * ch];
        paint_connectors(&mut px, cw, ch, &ui, &items, None, 1.0);
        assert_ne!(px[50 * cw + 200], PAPER, "the line crosses the gap");
        assert_eq!(px[10 * cw + 200], PAPER);
    }

    #[test]
    fn a_text_box_grows_with_its_text_and_never_shrinks_below_a_line() {
        let ui = test_ui();
        let mut item = Item::new(Kind::Text, 0.0, 0.0, 120.0, 10.0);
        fit(&ui, &mut item);
        let one = item.h;
        assert!(one >= MIN_TEXT_H_PT);
        item.text = "one two three four five six seven eight nine ten eleven twelve".into();
        fit(&ui, &mut item);
        assert!(item.h > one, "wrapped text needs more lines");
        item.text = "a\nb\nc".into();
        let three = {
            fit(&ui, &mut item);
            item.h
        };
        assert!(three > one);
    }

    #[test]
    fn lines_split_at_breaks_and_keep_blank_lines() {
        let ui = test_ui();
        assert_eq!(lines(&ui, "", 15.0, Weight::Regular, 100.0), vec![String::new()]);
        assert_eq!(lines(&ui, "a\n\nb", 15.0, Weight::Regular, 100.0), vec!["a".to_string(), String::new(), "b".to_string()]);
    }

    #[test]
    fn a_viewed_copy_is_magnified_and_shifted_and_the_original_is_left_alone() {
        let (items, a, _) = two_items();
        let v = items.viewed(2.0, (10.0, -5.0));
        let it = v.get(a).unwrap();
        assert_eq!((it.x, it.y, it.w, it.h), (10.0, -5.0, 200.0, 200.0));
        assert_eq!(items.get(a).unwrap().w, 100.0);
        let same = items.viewed(1.0, (0.0, 0.0));
        assert_eq!(same, items, "no zoom and no shift is the same board");
    }

    #[test]
    fn zoomed_painting_scales_the_type_with_the_box() {
        let ui = test_ui();
        let ink_rows = |zoom: f32| -> usize {
            let mut items = Items::default();
            items.add(Item { text: "Hello".into(), color: INK, ..Item::new(Kind::Text, 4.0 * zoom, 4.0 * zoom, 200.0 * zoom, 60.0 * zoom) });
            let (cw, ch) = (400, 300);
            let mut px = vec![PAPER; cw * ch];
            paint(&mut px, cw, ch, &ui, &items, &HashMap::new(), None, usize::MAX, zoom);
            (0..ch).filter(|&y| (0..cw).any(|x| px[y * cw + x] != PAPER)).count()
        };
        let (small, big) = (ink_rows(1.0), ink_rows(2.0));
        assert!(small > 0);
        assert!(big as f32 > small as f32 * 1.6, "twice the zoom gives text about twice as tall: {small} -> {big}");
    }

    #[test]
    fn a_picture_is_blitted_at_its_place_and_a_missing_one_is_a_placeholder() {
        let ui = test_ui();
        let mut items = Items::default();
        let id = items.add(Item::new(Kind::Picture, 10.0, 10.0, 20.0, 20.0));
        let (cw, ch) = (60, 60);
        let red = Image { w: 20, h: 20, px: vec![0xFF00_00FF; 400] };
        let mut pics = HashMap::new();
        pics.insert(id, red);
        let mut px = vec![PAPER; cw * ch];
        paint(&mut px, cw, ch, &ui, &items, &pics, None, usize::MAX, 1.0);
        assert_eq!(px[20 * cw + 20], 0xFF00_00FF);
        assert_eq!(px[5 * cw + 5], PAPER);
        let mut px = vec![PAPER; cw * ch];
        paint(&mut px, cw, ch, &ui, &items, &HashMap::new(), None, usize::MAX, 1.0);
        assert_eq!(px[28 * cw + 28], CARD_FILL, "the placeholder is a card");
    }

    #[test]
    fn a_translucent_pixel_blends_with_the_paper_under_it() {
        let half_black = 0x8000_0000;
        let out = over_opaque(PAPER, half_black);
        assert_eq!(out >> 24, 255);
        let r = out & 255;
        assert!((126..=129).contains(&r), "about half way: {r}");
        assert_eq!(over_opaque(PAPER, 0xFF12_3456), 0xFF12_3456);
    }
}
