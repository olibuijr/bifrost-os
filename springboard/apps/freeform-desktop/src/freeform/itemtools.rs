//! The board's items as the person works with them: picking and moving, typing into text boxes, notes, tables and
//! links, connectors drawn from one item to another, pictures and files kept beside the board, content shared from
//! other apps, and the board search. The data and painting are in `items.rs`; this is the part that belongs to the
//! open board, so it lives under `freeform.rs` and reaches its fields.
use super::*;
use crate::items::NOTE_COLORS;

/// Shapes popover: row height, the panel's padding and its width, in points.
const SHAPE_ROW_PT: f32 = 28.0;
const SHAPE_PAD_PT: f32 = 6.0;
const SHAPE_PANEL_W_PT: f32 = 168.0;
use bifrost_app::controls::prefs_layout::m;

/// Two presses on one item closer together than this (milliseconds) are a double tap.
const DOUBLE_TAP_MS: f64 = 450.0;
/// Largest file kept with a board.
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
/// Largest picture decoded (width times height) and the most either side keeps after decoding.
const MAX_PIXELS: usize = 24_000_000;
const MAX_SIDE_PX: usize = 3000;
/// Sizes new items start at (points).
const TEXT_BOX_PT: (f32, f32) = (220.0, 40.0);
const NOTE_PT_SIDE: f32 = 160.0;
const TABLE_PT: (f32, f32) = (270.0, 102.0);
const LINK_PT: (f32, f32) = (260.0, 64.0);
const FILE_PT: (f32, f32) = (240.0, 64.0);
const PICTURE_MAX_PT: f32 = 360.0;
/// Where the next item of a stack goes, and the gap between two of them (points).
const MARGIN_PT: f32 = 24.0;
const GAP_PT: f32 = 16.0;
/// How near a press must be to a connector's line to pick it (points).
const CONNECTOR_REACH_PT: f32 = 6.0;
/// Most boards the search lists.
const SEARCH_ROWS: usize = 8;
const SEARCH_W_PT: f32 = 360.0;
const SEARCH_FIELD_PT: f32 = 36.0;
const SEARCH_ROW_PT: f32 = 28.0;

/// What is picked among the items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pick {
    Item(u32),
    /// A connector, by its index.
    Connector(usize),
}

/// An item being dragged: where the press began, where the item was then, and whether it moved (the undo step is
/// taken on the first move).
#[derive(Clone, Copy, Debug)]
pub(super) struct ItemDrag {
    id: u32,
    start: (f32, f32),
    origin: (f32, f32),
    moved: bool,
}

/// A connector being drawn from item `from` to the pointer.
#[derive(Clone, Copy, Debug)]
pub(super) struct Linking {
    pub(super) from: u32,
    pub(super) at: (f32, f32),
}

/// The board search: what was typed and which row is chosen. With `rename` set the same panel renames the open board
/// instead: the field holds the new title and there are no rows.
#[derive(Clone, Debug, Default)]
pub(super) struct Search {
    query: String,
    sel: usize,
    rename: bool,
}

/// The decoded pictures. `src` holds each stored file's picture (or `None` when it would not decode) and `scaled`
/// each picture item's image at the size the item has.
#[derive(Default)]
pub(super) struct Pictures {
    pub(super) src: HashMap<String, Option<Image>>,
    pub(super) scaled: HashMap<u32, Image>,
}

/// Content another app shared with Freeform.
#[derive(Clone, Debug, PartialEq)]
pub enum Shared {
    Text(String),
    Url(String),
    File { name: String, bytes: Vec<u8> },
}

/// A picture of `bytes`, if it is one of a size the board takes. A large picture is reduced to at most 3000 pixels a side.
pub fn decode_bounded(bytes: &[u8]) -> Option<Image> {
    let (w, h) = springboard::cg::imageio::pixel_size(bytes)?;
    if w == 0 || h == 0 || w.saturating_mul(h) > MAX_PIXELS {
        return None;
    }
    let img = springboard::cg::imageio::decode(bytes)?;
    if img.w <= MAX_SIDE_PX && img.h <= MAX_SIDE_PX {
        return Some(img);
    }
    let k = (MAX_SIDE_PX as f32 / img.w.max(img.h) as f32).min(1.0);
    Some(img.resized(((img.w as f32 * k) as usize).max(1), ((img.h as f32 * k) as usize).max(1)))
}

/// `name`, or `name` with a number before its extension, whichever does not exist yet in `dir`.
fn unique_name(dir: &std::path::Path, name: &str) -> String {
    if !dir.join(name).exists() {
        return name.to_string();
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    let mut n = 2;
    loop {
        let candidate = format!("{stem}-{n}{ext}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
        n += 1;
    }
}

impl Freeform {
    // ---- state the menus and the app shell ask about ----

    pub fn has_pick(&self) -> bool {
        self.pick.is_some()
    }

    /// Whether the keyboard goes to an item or to the board search.
    pub fn is_editing(&self) -> bool {
        self.edit.is_some() || self.search.is_some()
    }

    /// The kind of the picked item, if an item is picked.
    fn picked_kind(&self) -> Option<ItemKind> {
        match self.pick? {
            Pick::Item(id) => self.items.get(id).map(|i| i.kind),
            Pick::Connector(_) => None,
        }
    }

    pub fn can_add_table_part(&self) -> bool {
        self.picked_kind() == Some(ItemKind::Table)
    }

    pub fn can_scale_item(&self) -> bool {
        matches!(self.pick, Some(Pick::Item(_)))
    }

    /// The connector tool is in hand.
    pub fn connector_tool(&self) -> bool {
        self.tool == Tool::Connect
    }

    /// The text box or note tool is in hand.
    pub fn text_tool(&self) -> Option<ItemKind> {
        match self.tool {
            Tool::Text => Some(ItemKind::Text),
            Tool::Sticky => Some(ItemKind::Sticky),
            _ => None,
        }
    }

    /// A web address a double tap on a link asked to open, once.
    pub fn take_open_url(&mut self) -> Option<String> {
        self.open_url.take()
    }

    /// Forgets what was picked, typed into or being dragged (the items were replaced by an undo, a redo or a clear).
    pub(super) fn end_item_work(&mut self) {
        self.edit = None;
        self.pick = None;
        self.item_drag = None;
        self.linking = None;
    }

    /// Whether an item gesture (a drag, or a connector being drawn) is going on.
    pub(super) fn item_gesture(&self) -> bool {
        self.item_drag.is_some() || self.linking.is_some()
    }

    // ---- tools ----

    /// Insert > Connector: the next drag from one item to another joins them.
    pub fn choose_connector(&mut self, ui: &Ui) {
        self.finish_edit(ui);
        self.tool = Tool::Connect;
        self.selection = None;
        self.dirty = true;
    }

    /// Insert > Text Box or Sticky Note with a click on the board: the next press makes one.
    #[cfg(test)]
    pub fn choose_text_tool(&mut self, ui: &Ui, kind: ItemKind) {
        self.finish_edit(ui);
        self.tool = if kind == ItemKind::Sticky { Tool::Sticky } else { Tool::Text };
        self.selection = None;
        self.dirty = true;
    }

    // ---- adding items ----

    fn files_dir(&self) -> Option<PathBuf> {
        self.board_dir().map(|d| d.join("files"))
    }

    /// Where an item of this size goes when the menu adds it: near the middle of what is in view, each new one a little
    /// further down and right so they do not hide each other.
    fn spot(&self, ui: &Ui, w: f32, h: f32) -> (f32, f32) {
        let step = (self.items.list().len() % 6) as f32 * ui.pt(GAP_PT);
        let (cx, cy) = self.board_pt((self.cw as f32 / 2.0, self.ch as f32 / 2.0));
        (cx - w / 2.0 + step, cy - h / 2.0 + step)
    }

    /// Where the next item of a stack goes: the left margin, below the lowest item.
    fn next_slot(&self, ui: &Ui) -> (f32, f32) {
        let bottom = self.items.list().iter().map(|i| i.y + i.h).fold(f32::MIN, f32::max);
        let y = if bottom == f32::MIN { ui.pt(MARGIN_PT) } else { bottom + ui.pt(GAP_PT) };
        (ui.pt(MARGIN_PT), y)
    }

    /// Puts `item` on the board as one undo step, picks it, and, when `cell` is given, starts typing into it.
    fn add_item(&mut self, ui: &Ui, item: Item, cell: Option<usize>) -> u32 {
        self.finish_edit(ui);
        self.push_undo();
        let id = self.items.add(item);
        self.selection = None;
        self.pick = Some(Pick::Item(id));
        self.edit = cell.map(|c| (id, c));
        self.caret = usize::MAX;
        self.items_changed(ui);
        id
    }

    /// Repaints after an item changed: text boxes are refitted, the picture is painted again and the board is saved.
    fn items_changed(&mut self, ui: &Ui) {
        self.items.fit_all(ui);
        self.repaint(ui, None);
        self.mark_changed();
    }

    /// Insert > Text Box.
    pub fn insert_text_box(&mut self, ui: &Ui) {
        let (w, h) = (ui.pt(TEXT_BOX_PT.0), ui.pt(TEXT_BOX_PT.1));
        let (x, y) = self.spot(ui, w, h);
        let item = Item { color: COLORS[self.color], ..Item::new(ItemKind::Text, x, y, w, h) };
        self.add_item(ui, item, Some(0));
        self.tool = Tool::Lasso;
    }

    /// Insert > Sticky Note, in the pastel of the ink colour.
    pub fn insert_note(&mut self, ui: &Ui) {
        let side = ui.pt(NOTE_PT_SIDE);
        let (x, y) = self.spot(ui, side, side);
        let item = Item { color: NOTE_COLORS[self.color % NOTE_COLORS.len()], ..Item::new(ItemKind::Sticky, x, y, side, side) };
        self.add_item(ui, item, Some(0));
        self.tool = Tool::Lasso;
    }

    /// Insert > Table: three rows of three empty cells, with the first cell ready for typing.
    pub fn insert_table(&mut self, ui: &Ui) {
        let (w, h) = (ui.pt(TABLE_PT.0), ui.pt(TABLE_PT.1));
        let (x, y) = self.spot(ui, w, h);
        self.add_item(ui, Item::table(x, y, w, h, 3, 3), Some(0));
        self.tool = Tool::Lasso;
    }

    /// Insert > Link. `clipboard` is the text on the pasteboard: a web address there becomes the link at once;
    /// otherwise the link starts empty and takes the address typed.
    pub fn insert_link(&mut self, ui: &Ui, clipboard: Option<&str>) {
        let (w, h) = (ui.pt(LINK_PT.0), ui.pt(LINK_PT.1));
        let (x, y) = self.spot(ui, w, h);
        let address = clipboard.map(str::trim).filter(|t| items::is_web_url(t));
        let item = Item { text: address.unwrap_or("").to_string(), ..Item::new(ItemKind::Link, x, y, w, h) };
        let cell = if address.is_some() { None } else { Some(0) };
        self.add_item(ui, item, cell);
        self.tool = Tool::Lasso;
    }

    /// Keeps `bytes` in the board's `files` folder under a name based on `name`; returns the stored name.
    fn store_file(&self, name: &str, bytes: &[u8]) -> Result<String, String> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(format!("{name} is larger than {} MB", MAX_FILE_BYTES / (1024 * 1024)));
        }
        let dir = self.files_dir().ok_or("there is no folder to keep the file in")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stored = unique_name(&dir, &items::safe_file_name(name));
        springboard::shell::fsutil::write_atomic(&dir.join(&stored), bytes.to_vec()).map_err(|e| e.to_string())?;
        Ok(stored)
    }

    /// Insert > Picture or File: a picture is drawn on the board; any other file is a card with its name and size.
    /// The bytes are kept with the board, so the item survives the original being moved. `at` is where the item goes
    /// (the centre of the board when `None`).
    pub fn insert_file(&mut self, ui: &Ui, name: &str, bytes: &[u8], at: Option<(f32, f32)>) -> Result<(), String> {
        let stored = self.store_file(name, bytes)?;
        let shown: String = name.rsplit(['/', '\\']).next().unwrap_or(name).chars().take(120).collect();
        let item = match decode_bounded(bytes) {
            Some(img) => {
                let max = ui.pt(PICTURE_MAX_PT);
                let (w, h) = items::fit_picture(img.w as f32, img.h as f32, max, max);
                self.pics.src.insert(stored.clone(), Some(img));
                let (x, y) = at.unwrap_or_else(|| self.spot(ui, w, h));
                Item { text: shown, file: stored, size: bytes.len() as u64, ..Item::new(ItemKind::Picture, x, y, w, h) }
            }
            None => {
                let (w, h) = (ui.pt(FILE_PT.0), ui.pt(FILE_PT.1));
                let (x, y) = at.unwrap_or_else(|| self.spot(ui, w, h));
                Item { text: shown, file: stored, size: bytes.len() as u64, ..Item::new(ItemKind::File, x, y, w, h) }
            }
        };
        self.add_item(ui, item, None);
        Ok(())
    }

    /// Insert > Picture or File with several files: each goes on the board, a little down and right of the one before;
    /// the toast names how many could not be added.
    pub fn insert_files(&mut self, ui: &Ui, files: Vec<(String, Vec<u8>)>) {
        let mut failed: Vec<String> = Vec::new();
        for (name, bytes) in files {
            if let Err(why) = self.insert_file(ui, &name, &bytes, None) {
                failed.push(why);
            }
        }
        if let Some(first) = failed.first() {
            let msg = if failed.len() == 1 { format!("Could not add the file: {first}") } else { format!("{} files could not be added", failed.len()) };
            self.say(msg);
        }
    }

    /// The item text becomes: a web address is a link, a short text a sticky note, a long one a text box.
    fn text_item(ui: &Ui, text: String, x: f32, y: f32) -> Item {
        if items::is_web_url(&text) {
            return Item { text: text.trim().to_string(), ..Item::new(ItemKind::Link, x, y, ui.pt(LINK_PT.0), ui.pt(LINK_PT.1)) };
        }
        if text.chars().count() <= 120 {
            let side = ui.pt(NOTE_PT_SIDE);
            return Item { text, color: NOTE_COLORS[3], ..Item::new(ItemKind::Sticky, x, y, side, side) };
        }
        Item { text, ..Item::new(ItemKind::Text, x, y, ui.pt(TEXT_BOX_PT.0), ui.pt(TEXT_BOX_PT.1)) }
    }

    /// Edit > Paste with only text on the pasteboard: it lands on the board as an item centred at the pointer (see
    /// `text_item`).
    pub fn paste_text_item(&mut self, ui: &Ui, text: String) {
        if text.trim().is_empty() {
            return;
        }
        let (w, h) = (ui.pt(NOTE_PT_SIDE), ui.pt(NOTE_PT_SIDE));
        let (x, y) = match self.paste_anchor(ui) {
            Some((cx, cy)) => (cx - w / 2.0, cy - h / 2.0),
            None => self.spot(ui, w, h),
        };
        let item = Self::text_item(ui, text, x, y);
        self.add_item(ui, item, None);
    }

    /// Content shared from another app goes onto a board of its own: text as a note (or a text box when long), a web
    /// address as a link, a file as a picture or a card, stacked down the left side.
    pub fn import_shared(&mut self, ui: &Ui, shared: Vec<Shared>) {
        if shared.is_empty() {
            return;
        }
        self.new_board(ui);
        let id = self.boards.current();
        self.boards.rename(id, &format!("Shared {id}"));
        self.write_list();
        let mut failed = 0;
        for piece in shared {
            let (x, y) = self.next_slot(ui);
            let item = match piece {
                Shared::Url(u) => Some(Item { text: u.trim().to_string(), ..Item::new(ItemKind::Link, x, y, ui.pt(LINK_PT.0), ui.pt(LINK_PT.1)) }),
                Shared::Text(t) => Some(Self::text_item(ui, t, x, y)),
                Shared::File { name, bytes } => {
                    if self.insert_file(ui, &name, &bytes, Some((x, y))).is_err() {
                        failed += 1;
                    }
                    None
                }
            };
            if let Some(item) = item {
                self.add_item(ui, item, None);
            }
        }
        self.pick = None;
        if failed > 0 {
            self.say(format!("{failed} shared file(s) could not be added"));
        }
        self.repaint(ui, None);
    }

    // ---- picking, moving, editing ----

    /// A press with the lasso on the board: it picks an item or a connector and starts dragging the item. False when
    /// the press hit neither, so the lasso starts.
    pub(super) fn item_press(&mut self, ui: &Ui, cx: f32, cy: f32) -> bool {
        if let Some(id) = self.items.hit(cx, cy) {
            self.selection = None;
            self.pick = Some(Pick::Item(id));
            if self.edit.is_some_and(|(e, _)| e == id) {
                if let Some(cell) = self.items.get(id).and_then(|i| i.cell_at(cx, cy)) {
                    self.edit = Some((id, cell));
                    self.dirty = true;
                }
            }
            let double = self.last_tap.is_some_and(|(at, tapped)| tapped == id && self.now - at < DOUBLE_TAP_MS);
            self.last_tap = Some((self.now, id));
            if double {
                self.activate_item(ui, id, cx, cy);
            } else if let Some(item) = self.items.get(id) {
                self.item_drag = Some(ItemDrag { id, start: (cx, cy), origin: (item.x, item.y), moved: false });
            }
            self.dirty = true;
            return true;
        }
        self.last_tap = None;
        if let Some(n) = self.items.connector_at(cx, cy, ui.pt(CONNECTOR_REACH_PT) / self.zoom) {
            self.selection = None;
            self.pick = Some(Pick::Connector(n));
            self.dirty = true;
            return true;
        }
        self.pick = None;
        false
    }

    /// A double tap: a link opens, a text box, note or table starts taking text.
    fn activate_item(&mut self, ui: &Ui, id: u32, cx: f32, cy: f32) {
        let Some(item) = self.items.get(id) else { return };
        match item.kind {
            ItemKind::Link => {
                if items::is_web_url(&item.text) {
                    self.open_url = Some(item.text.trim().to_string());
                }
            }
            ItemKind::Text | ItemKind::Sticky | ItemKind::Table => {
                let cell = item.cell_at(cx, cy).unwrap_or(0);
                self.begin_edit(ui, id, cell);
            }
            ItemKind::File | ItemKind::Picture => {}
        }
    }

    /// Starts typing into an item (or another cell of the table already being typed into).
    pub(super) fn begin_edit(&mut self, ui: &Ui, id: u32, cell: usize) {
        if self.edit.is_some_and(|(e, _)| e == id) {
            self.edit = Some((id, cell));
        } else {
            self.finish_edit(ui);
            if !self.items.get(id).is_some_and(|i| i.editable()) {
                return;
            }
            self.push_undo();
            self.edit = Some((id, cell));
        }
        self.caret = usize::MAX;
        self.pick = Some(Pick::Item(id));
        self.repaint(ui, None);
    }

    /// Ends typing. A text box or link left empty is removed.
    pub(super) fn finish_edit(&mut self, ui: &Ui) {
        let Some((id, _)) = self.edit.take() else { return };
        let empty = self.items.get(id).is_some_and(|i| matches!(i.kind, ItemKind::Text | ItemKind::Link) && i.is_blank());
        if empty {
            self.items.remove(id);
            if self.pick == Some(Pick::Item(id)) {
                self.pick = None;
            }
            self.mark_changed();
        }
        self.repaint(ui, None);
    }

    /// A press with the text box or note tool: it types into the item under it, or makes a new one there.
    pub(super) fn text_tool_press(&mut self, ui: &Ui, cx: f32, cy: f32) {
        if let Some(id) = self.items.hit(cx, cy) {
            if self.items.get(id).is_some_and(|i| i.editable()) {
                let cell = self.items.get(id).and_then(|i| i.cell_at(cx, cy)).unwrap_or(0);
                self.selection = None;
                self.begin_edit(ui, id, cell);
                return;
            }
        }
        let sticky = self.tool == Tool::Sticky;
        let item = if sticky {
            let side = ui.pt(NOTE_PT_SIDE);
            Item { color: NOTE_COLORS[self.color % NOTE_COLORS.len()], ..Item::new(ItemKind::Sticky, cx - side / 2.0, cy - side / 2.0, side, side) }
        } else {
            Item { color: COLORS[self.color], ..Item::new(ItemKind::Text, cx, cy, ui.pt(TEXT_BOX_PT.0), ui.pt(TEXT_BOX_PT.1)) }
        };
        self.add_item(ui, item, Some(0));
        self.tool = Tool::Lasso;
    }

    /// A press with the connector tool: on an item it starts a connector from there.
    pub(super) fn connect_press(&mut self, cx: f32, cy: f32) {
        self.selection = None;
        self.pick = None;
        if let Some(id) = self.items.hit(cx, cy) {
            self.linking = Some(Linking { from: id, at: (cx, cy) });
            self.pick = Some(Pick::Item(id));
        }
        self.dirty = true;
    }

    /// A drag with an item gesture: the item moves, or the connector's loose end follows the pointer.
    pub(super) fn item_move(&mut self, ui: &Ui, cx: f32, cy: f32) {
        if let Some(mut drag) = self.item_drag {
            if !drag.moved {
                self.push_undo();
                drag.moved = true;
            }
            // the item keeps the offset it was grabbed at; with snapping on its corner lands on a grid point
            let at = self.snapped(ui, (drag.origin.0 + cx - drag.start.0, drag.origin.1 + cy - drag.start.1));
            if let Some(item) = self.items.get_mut(drag.id) {
                item.x = at.0;
                item.y = at.1;
            }
            self.item_drag = Some(drag);
            self.repaint(ui, None);
        } else if let Some(link) = self.linking.as_mut() {
            link.at = (cx, cy);
        }
        self.dirty = true;
    }

    /// A release ends the item drag, or joins the connector's first item to the one under the pointer.
    pub(super) fn item_release(&mut self, ui: &Ui, cx: f32, cy: f32) {
        if self.item_drag.take().is_some() {
            return;
        }
        if let Some(link) = self.linking.take() {
            if let Some(to) = self.items.hit(cx, cy) {
                if self.items.can_connect(link.from, to) {
                    self.push_undo();
                    if let Some(n) = self.items.connect(link.from, to, true) {
                        self.pick = Some(Pick::Connector(n));
                    }
                    self.repaint(ui, None);
                }
            }
        }
        self.dirty = true;
    }

    /// Delete on the picked item or connector. False when nothing is picked.
    pub(super) fn delete_pick(&mut self, ui: &Ui) -> bool {
        let Some(pick) = self.pick else { return false };
        self.push_undo();
        match pick {
            Pick::Item(id) => {
                if self.edit.is_some_and(|(e, _)| e == id) {
                    self.edit = None;
                }
                self.items.remove(id);
            }
            Pick::Connector(n) => {
                self.items.remove_connector(n);
            }
        }
        self.pick = None;
        self.items_changed(ui);
        true
    }

    /// Edit > Duplicate on a picked item: a copy a little down and right, picked. A copied picture or file shares the
    /// stored file.
    pub(super) fn duplicate_item(&mut self, ui: &Ui, id: u32) {
        self.finish_edit(ui);
        self.push_undo();
        let off = ui.pt(PASTE_OFFSET_PT);
        if let Some(copy) = self.items.duplicate(id, off, off) {
            self.pick = Some(Pick::Item(copy));
        }
        self.items_changed(ui);
    }

    /// Arrange > Larger or Smaller on the picked item.
    pub fn scale_pick(&mut self, ui: &Ui, factor: f32) {
        let Some(Pick::Item(id)) = self.pick else { return };
        self.finish_edit(ui);
        self.push_undo();
        self.items.scale(id, factor, ui.s);
        self.items_changed(ui);
    }

    /// Insert > Add Table Row or Column on the picked table.
    pub fn add_table_part(&mut self, ui: &Ui, row: bool) {
        let Some(Pick::Item(id)) = self.pick else { return };
        let before = self.items.clone();
        let grown = if row { self.items.add_row(id) } else { self.items.add_column(id) };
        if !grown {
            return;
        }
        let after = std::mem::replace(&mut self.items, before);
        self.push_undo();
        self.items = after;
        self.items_changed(ui);
    }

    /// Picking a colour with an item picked paints it: a note takes the pastel, a text box the ink.
    pub(super) fn recolour_pick(&mut self, ui: &Ui, index: usize) {
        let Some(Pick::Item(id)) = self.pick else { return };
        let kind = self.items.get(id).map(|i| i.kind);
        let color = match kind {
            Some(ItemKind::Sticky) => NOTE_COLORS[index % NOTE_COLORS.len()],
            Some(ItemKind::Text) => COLORS[index % COLORS.len()],
            _ => return,
        };
        if self.items.get(id).is_some_and(|i| i.color == color) {
            return;
        }
        self.push_undo();
        if let Some(i) = self.items.get_mut(id) {
            i.color = color;
        }
        self.items_changed(ui);
    }

    // ---- the keyboard ----

    /// Typing into the item being edited, or into the board search; Backspace or Delete on a picked item removes it,
    /// and Return on one starts typing into it. True when the key was used.
    pub fn key(&mut self, ui: &Ui, key: &KeyInput) -> bool {
        if self.search.is_some() {
            return self.search_key(ui, key);
        }
        let Some((id, cell)) = self.edit else { return self.pick_key(ui, key) };
        let kind = self.items.get(id).map(|i| i.kind);
        match key {
            KeyInput::Text(text) => {
                self.type_at_caret(ui, id, cell, text);
                true
            }
            KeyInput::Code(8) => {
                if let Some(at) = self.items.backspace(id, cell, self.caret) {
                    self.caret = at;
                    self.items_changed(ui);
                }
                true
            }
            KeyInput::Code(code @ (35 | 36 | 37 | 39)) => {
                let len = self.items.text_len(id, cell);
                let at = self.caret.min(len);
                self.caret = match *code {
                    35 => len,
                    36 => 0,
                    37 => at.saturating_sub(1),
                    _ => (at + 1).min(len),
                };
                self.repaint(ui, None);
                true
            }
            KeyInput::Code(13) => {
                match kind {
                    Some(ItemKind::Text | ItemKind::Sticky) => self.type_at_caret(ui, id, cell, "\n"),
                    Some(ItemKind::Table) => {
                        let cols = self.items.get(id).map_or(1, |i| i.cols.max(1));
                        let count = self.items.get(id).map_or(0, |i| i.cells.len());
                        if cell + cols < count {
                            self.edit = Some((id, cell + cols));
                            self.caret = usize::MAX;
                            self.repaint(ui, None);
                        } else {
                            self.finish_edit(ui);
                        }
                    }
                    _ => self.finish_edit(ui),
                }
                true
            }
            KeyInput::Code(9) => {
                let count = self.items.get(id).map_or(0, |i| i.cells.len());
                if kind == Some(ItemKind::Table) && count > 0 {
                    self.edit = Some((id, (cell + 1) % count));
                    self.caret = usize::MAX;
                    self.repaint(ui, None);
                    return true;
                }
                false
            }
            KeyInput::Code(27) | KeyInput::Code(1) => {
                self.finish_edit(ui);
                true
            }
            _ => false,
        }
    }

    /// Types `text` into the cell of the item being edited, at the caret, and moves the caret after it.
    fn type_at_caret(&mut self, ui: &Ui, id: u32, cell: usize, text: &str) {
        if let Some(at) = self.items.insert_text(id, cell, self.caret, text) {
            self.caret = at;
            self.items_changed(ui);
        }
    }

    fn pick_key(&mut self, ui: &Ui, key: &KeyInput) -> bool {
        match (key, self.pick) {
            (KeyInput::Code(8 | 46), Some(_)) => self.delete_pick(ui),
            (KeyInput::Code(8 | 46), None) if self.has_selection() => {
                self.delete_selected(ui);
                true
            }
            (KeyInput::Code(code @ 37..=40), _) => self.arrow_pans(ui, *code),
            (KeyInput::Code(13), Some(Pick::Item(id))) => {
                let editable = self.items.get(id).is_some_and(|i| i.editable());
                if editable {
                    self.begin_edit(ui, id, 0);
                }
                editable
            }
            _ => false,
        }
    }

    /// Edit > Paste while typing: the pasteboard's text goes into the item or the search field. True when it did.
    pub fn paste_text(&mut self, ui: &Ui, text: &str) -> bool {
        if let Some(search) = self.search.as_mut() {
            search.query.extend(text.lines().next().unwrap_or("").chars().filter(|c| !c.is_control()));
            search.sel = 0;
            self.dirty = true;
            return true;
        }
        let Some((id, cell)) = self.edit else { return false };
        self.type_at_caret(ui, id, cell, text);
        true
    }

    // ---- pictures ----

    /// Decodes the pictures the items need and scales each to its item's size. Called before the board is painted.
    pub(super) fn prepare_pictures(&mut self) {
        let wanted: Vec<(u32, String, usize, usize)> = self
            .items
            .list()
            .iter()
            .filter(|i| i.kind == ItemKind::Picture)
            .map(|i| (i.id, i.file.clone(), (i.w * self.zoom).round().max(1.0) as usize, (i.h * self.zoom).round().max(1.0) as usize))
            .collect();
        self.pics.scaled.retain(|id, _| wanted.iter().any(|w| w.0 == *id));
        let dir = self.files_dir();
        for (id, file, w, h) in wanted {
            if self.pics.scaled.get(&id).is_some_and(|img| img.w == w && img.h == h) {
                continue;
            }
            if !self.pics.src.contains_key(&file) {
                let decoded = dir.as_ref().and_then(|d| std::fs::read(d.join(&file)).ok()).and_then(|bytes| decode_bounded(&bytes));
                self.pics.src.insert(file.clone(), decoded);
            }
            if w.saturating_mul(h) > 16_000_000 {
                continue;
            }
            if let Some(Some(src)) = self.pics.src.get(&file) {
                self.pics.scaled.insert(id, src.resized(w, h));
            }
        }
    }

    /// The items alone, painted on paper at the canvas size (for the PDF and PNG exports).
    pub(super) fn items_layer(&self, ui: &Ui) -> Vec<u32> {
        let mut px = vec![PAPER; self.cw * self.ch];
        self.paint_items(&mut px, ui, None);
        px
    }

    /// Removes the files in the board's `files` folder that no item uses (called when a board is read, never while
    /// there is history that could bring an item back). Skipped when a line of the item file did not parse.
    pub(super) fn drop_unused_files(&self, dir: &std::path::Path, declared: usize) {
        if declared != self.items.list().len() {
            return;
        }
        let Ok(read) = std::fs::read_dir(dir.join("files")) else { return };
        let used = self.items.files();
        for entry in read.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if !used.contains(&name) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    // ---- drawing the overlays ----

    /// The frame round the picked item, and the dots along the picked connector or the connector being drawn.
    pub(super) fn draw_pick(&self, s: &mut Surface, ui: &Ui, top: f32) {
        if let Some(Pick::Item(id)) = self.pick {
            if let Some(item) = self.items.get(id) {
                self.outline(s, ui, top, self.view_rect(item.rect()));
            }
        }
        let dots = |s: &mut Surface, a: (f32, f32), b: (f32, f32)| {
            // the ends are board points; the dots are laid on the picture
            let (a, b) = (self.view_pt(a), self.view_pt(b));
            let d = ui.pt(bifrost_app::kit::ui::SPACE_XXS) * 1.5;
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
            let n = (len / (d * 2.0)).ceil().max(1.0) as usize;
            for k in 0..=n {
                let f = k as f32 / n as f32;
                s.fill_rrect(a.0 + (b.0 - a.0) * f - d / 2.0, top + a.1 + (b.1 - a.1) * f - d / 2.0, d, d, d / 2.0, TINT);
            }
        };
        if let Some(Pick::Connector(n)) = self.pick {
            if let Some((a, b)) = self.items.connectors().get(n).and_then(|c| self.items.connector_line(c)) {
                dots(s, a, b);
            }
        }
        if let Some(link) = &self.linking {
            if let Some(item) = self.items.get(link.from) {
                dots(s, item.centre(), link.at);
            }
        }
    }

    // ---- board search ----

    /// Boards > Search Boards.
    pub fn open_search(&mut self, ui: &Ui) {
        self.finish_edit(ui);
        self.search = Some(Search::default());
        self.dirty = true;
    }

    /// Boards > Rename Board: the panel opens on the open board's title.
    pub fn open_rename(&mut self, ui: &Ui) {
        self.finish_edit(ui);
        self.search = Some(Search { query: self.boards.current_title().to_string(), sel: 0, rename: true });
        self.dirty = true;
    }

    /// The boards whose title contains what was typed (none while renaming).
    fn search_hits(&self) -> Vec<u32> {
        let Some(search) = self.search.as_ref().filter(|s| !s.rename) else { return Vec::new() };
        self.boards.matching(&search.query).iter().take(SEARCH_ROWS).map(|e| e.id).collect()
    }

    /// The search panel, its field and its result rows (window pixels).
    pub(super) fn search_layout(&self, ui: &Ui) -> (Rect, Rect, Vec<Rect>) {
        let n = self.search_hits().len();
        let w = ui.pt(SEARCH_W_PT).min(self.w as f32);
        let x = (self.w as f32 - w) / 2.0;
        let y = self.canvas_top(ui) as f32 + ui.pt(SPACE_XXL);
        let field_h = ui.pt(SEARCH_FIELD_PT);
        let row_h = ui.pt(SEARCH_ROW_PT);
        let panel = Rect { x, y, w, h: field_h + n as f32 * row_h + ui.pt(SPACE_S) };
        let field = Rect { x, y, w, h: field_h };
        let rows = (0..n).map(|i| Rect { x, y: y + field_h + i as f32 * row_h, w, h: row_h }).collect();
        (panel, field, rows)
    }

    fn close_search(&mut self) {
        self.search = None;
        self.dirty = true;
    }

    fn open_search_hit(&mut self, ui: &Ui, id: u32) {
        self.close_search();
        self.open_board(ui, id);
    }

    /// Typing, Up, Down, Return and Escape while the search is open. Every key is used, so none reaches the board.
    fn search_key(&mut self, ui: &Ui, key: &KeyInput) -> bool {
        let hits = self.search_hits();
        let Some(search) = self.search.as_mut() else { return false };
        match key {
            KeyInput::Text(text) => {
                search.query.extend(text.chars().filter(|c| !c.is_control()));
                search.sel = 0;
            }
            KeyInput::Code(8) => {
                search.query.pop();
                search.sel = 0;
            }
            KeyInput::Code(38) => search.sel = search.sel.saturating_sub(1),
            KeyInput::Code(40) => search.sel = (search.sel + 1).min(hits.len().saturating_sub(1)),
            KeyInput::Code(13) => {
                let (chosen, rename, title) = (hits.get(search.sel).copied(), search.rename, search.query.clone());
                if rename {
                    self.rename_current(&title);
                } else {
                    match chosen {
                        Some(id) => self.open_search_hit(ui, id),
                        None => self.close_search(),
                    }
                }
            }
            KeyInput::Code(27) | KeyInput::Code(1) => self.close_search(),
            _ => {}
        }
        self.dirty = true;
        true
    }

    /// Gives the open board the title typed. A blank title is refused and the panel closes either way.
    fn rename_current(&mut self, title: &str) {
        self.close_search();
        let id = self.boards.current();
        if self.boards.rename(id, title) {
            self.write_list();
            let shown = self.boards.current_title().to_string();
            self.say(format!("Renamed to {shown}"));
        }
    }

    /// The Shapes popover in points: its panel, and one row per shape of `crate::SHAPES` (in that order).
    pub(super) fn shapes_layout(&self) -> Option<(Rect, Vec<(ShapeKind, Rect)>)> {
        if !self.shapes_open {
            return None;
        }
        let anchor = self.placed.as_ref()?.rect_of(BAR_SHAPES)?;
        let pad = SHAPE_PAD_PT;
        let panel = Rect {
            x: anchor.x,
            y: anchor.y + anchor.h + pad,
            w: SHAPE_PANEL_W_PT,
            h: SHAPE_ROW_PT * crate::SHAPES.len() as f32 + 2.0 * pad,
        };
        let rows = crate::SHAPES
            .iter()
            .enumerate()
            .map(|(i, (kind, _))| {
                let row = Rect { x: panel.x + pad, y: panel.y + pad + i as f32 * SHAPE_ROW_PT, w: panel.w - 2.0 * pad, h: SHAPE_ROW_PT };
                (*kind, row)
            })
            .collect();
        Some((panel, rows))
    }

    /// Draws the Shapes popover: a material panel, with the shape the tool is set to in the tint.
    pub(super) fn draw_shapes(&self, s: &mut Surface, ui: &Ui) {
        let Some((panel, rows)) = self.shapes_layout() else { return };
        widgets::material(s, ui, panel, m::CARD_RADIUS, widgets::Material::Popover);
        let pad = ui.pt(SPACE_S);
        for ((kind, row), (_, name)) in rows.iter().zip(crate::SHAPES.iter()) {
            let chosen = self.tool.shape_kind() == Some(*kind);
            if chosen {
                s.fill_rrect(ui.pt(row.x), ui.pt(row.y), ui.pt(row.w), ui.pt(row.h), ui.pt(m::SELECTION_RADIUS), TINT);
            }
            let ink = if chosen { WHITE } else { LABEL.now() };
            let img = ui.text(name, MAC_CALLOUT_PT, Weight::Regular, ink);
            s.blit(&img, (ui.pt(row.x) + pad) as i32, (ui.pt(row.y) + (ui.pt(row.h) - img.h as f32) / 2.0) as i32, 255, 255);
        }
    }

    /// A press while the Shapes popover is open. A row picks its shape tool and closes the popover; a press inside the
    /// panel is used up; a press on the Shapes button is left to the toolbar, which toggles it; any other press closes
    /// the popover and is handled as usual. True when the press was used.
    pub(super) fn shapes_press(&mut self, ui: &Ui, x: f32, y: f32) -> bool {
        let Some((panel, rows)) = self.shapes_layout() else { return false };
        let (px, py) = (x / ui.pt(1.0), y / ui.pt(1.0));
        let inside = |r: &Rect| px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h;
        if let Some((kind, _)) = rows.iter().find(|(_, r)| inside(r)) {
            self.shapes_open = false;
            self.activate(ui, Target::Tool(Tool::of_shape(*kind)));
            return true;
        }
        if inside(&panel) {
            return true;
        }
        let on_button = self.placed.as_ref().and_then(|p| p.rect_of(BAR_SHAPES)).is_some_and(|r| inside(&r));
        if !on_button {
            self.shapes_open = false;
        }
        false
    }

    /// A press while the search is open: a row opens that board, a press outside the panel closes the search.
    pub(super) fn search_press(&mut self, ui: &Ui, x: f32, y: f32) {
        let hits = self.search_hits();
        let (panel, field, rows) = self.search_layout(ui);
        let inside = |r: &Rect| x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h;
        let scale = ui.pt(1.0);
        let field_points = bifrost_app::controls::Rect {
            x: field.x / scale, y: field.y / scale, w: field.w / scale, h: field.h / scale,
        };
        if bifrost_app::controls::search::hits_clear(field_points, self.search.as_ref().is_some_and(|s| !s.query.is_empty()), x / scale, y / scale) {
            if let Some(search) = self.search.as_mut() { search.query.clear(); search.sel = 0; }
            self.dirty = true;
            return;
        }
        if let Some(i) = rows.iter().position(|r| inside(r)) {
            if let Some(&id) = hits.get(i) {
                self.open_search_hit(ui, id);
                return;
            }
        }
        if !inside(&panel) {
            self.close_search();
        }
        self.dirty = true;
    }

    /// Draws the search panel: the field with what was typed, and the matching boards.
    pub(super) fn draw_search(&self, s: &mut Surface, ui: &Ui) {
        let Some(search) = &self.search else { return };
        let (panel, field, rows) = self.search_layout(ui);
        widgets::material(
            s,
            ui,
            bifrost_app::controls::Rect { x: panel.x / ui.pt(1.0), y: panel.y / ui.pt(1.0), w: panel.w / ui.pt(1.0), h: panel.h / ui.pt(1.0) },
            m::CARD_RADIUS,
            widgets::Material::Popover,
        );
        let line = ui.pt(1.0).max(1.0);
        s.fill_rect(panel.x as usize, (field.y + field.h) as usize, panel.w as usize, line as usize, SEPARATOR_C.now());
        let hint = if search.rename { "Board name" } else { "Search boards" };
        let mut field_value = bifrost_app::controls::TextField::new();
        field_value.push_str(&search.query);
        let scale = ui.pt(1.0);
        widgets::search_field(
            s,
            ui,
            &field_value,
            bifrost_app::controls::Rect { x: field.x / scale, y: field.y / scale, w: field.w / scale, h: field.h / scale },
            hint,
            true,
        );
        let pad = ui.pt(SPACE_M);
        let hits = self.search_hits();
        for (i, (row, id)) in rows.iter().zip(hits.iter()).enumerate() {
            let chosen = i == search.sel;
            if chosen {
                s.fill_rrect(row.x + ui.pt(SPACE_XS), row.y, row.w - 2.0 * ui.pt(SPACE_XS), row.h, ui.pt(m::SELECTION_RADIUS), TINT);
            }
            let title = self.boards.entries().iter().find(|e| e.id == *id).map(|e| e.title.clone()).unwrap_or_default();
            let shown = ui.fit(&title, MAC_CALLOUT_PT, row.w - 2.0 * pad);
            let ink = if chosen { WHITE } else { LABEL.now() };
            let img = ui.text(&shown, MAC_CALLOUT_PT, Weight::Regular, ink);
            s.blit(&img, (row.x + pad) as i32, (row.y + (row.h - img.h as f32) / 2.0) as i32, 255, 255);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ui() -> Ui {
        use bifrost_app::kit::Font;
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
        let data: &'static [u8] = Box::leak(std::fs::read(format!("{dir}/SFUI.ttf")).unwrap().into_boxed_slice());
        Ui::new(Font::parse(data).unwrap(), 1.0)
    }

    fn board(ui: &Ui) -> Freeform {
        let mut app = Freeform::new(ui, 800, 600);
        app.ensure_canvas(ui);
        app
    }

    /// A board whose files are kept in a fresh folder under the crate's target directory.
    fn stored_board(ui: &Ui, tag: &str) -> (Freeform, PathBuf) {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-items")).join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = Freeform::new(ui, 800, 600).with_store(dir.clone());
        app.ensure_canvas(ui);
        (app, dir)
    }

    fn text_of(app: &Freeform, id: u32) -> String {
        app.items.get(id).unwrap().text.clone()
    }

    /// The first item of the board.
    fn first(app: &Freeform) -> u32 {
        app.items.list()[0].id
    }

    #[test]
    fn a_press_with_the_text_tool_makes_a_text_box_that_takes_typing() {
        let ui = test_ui();
        let mut app = board(&ui);
        let top = app.canvas_top(&ui) as f32;
        app.tool = Tool::Text;
        app.touch(&ui, Event::Down(100.0, top + 80.0), 0.0);
        app.touch(&ui, Event::Up(100.0, top + 80.0), 0.0);
        assert_eq!(app.items.list().len(), 1);
        assert_eq!(app.tool, Tool::Lasso, "the tool is used once");
        assert!(app.is_editing());
        assert!(app.key(&ui, &KeyInput::Text("Hello".into())));
        assert!(app.key(&ui, &KeyInput::Text(" there".into())));
        let id = first(&app);
        assert_eq!(text_of(&app, id), "Hello there");
        assert!(app.key(&ui, &KeyInput::Code(8)));
        assert_eq!(text_of(&app, id), "Hello ther");
        let it = app.items.get(id).unwrap();
        assert_eq!((it.x, it.y), (100.0, 80.0));
    }

    #[test]
    fn a_text_box_left_empty_is_removed_and_one_with_text_stays() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_text_box(&ui);
        assert_eq!(app.items.list().len(), 1);
        app.key(&ui, &KeyInput::Code(27));
        assert!(app.items.is_empty(), "an empty text box goes when typing ends");
        app.insert_text_box(&ui);
        app.key(&ui, &KeyInput::Text("kept".into()));
        app.key(&ui, &KeyInput::Code(27));
        assert_eq!(app.items.list().len(), 1);
        assert!(!app.is_editing());
    }

    #[test]
    fn a_sticky_note_takes_the_pastel_of_the_ink_colour() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.color = 4;
        app.insert_note(&ui);
        let it = app.items.list()[0].clone();
        assert_eq!(it.kind, ItemKind::Sticky);
        assert_eq!(it.color, NOTE_COLORS[4]);
        assert_eq!((it.w, it.h), (160.0, 160.0));
        app.key(&ui, &KeyInput::Text("one".into()));
        app.key(&ui, &KeyInput::Code(13));
        app.key(&ui, &KeyInput::Text("two".into()));
        assert_eq!(text_of(&app, it.id), "one\ntwo", "Return is a new line in a note");
        app.key(&ui, &KeyInput::Code(27));
        assert_eq!(app.items.list().len(), 1, "a note with text stays");
    }

    #[test]
    fn a_note_is_painted_on_the_board_canvas() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Code(27));
        let it = app.items.list()[0].clone();
        let (x, y) = ((it.x + it.w / 2.0) as usize, (it.y + it.h - 4.0) as usize);
        assert_eq!(app.canvas[y * app.cw + x], NOTE_COLORS[0]);
    }

    #[test]
    fn ink_is_drawn_over_an_item() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Code(27));
        let it = app.items.list()[0].clone();
        let (mx, my) = (it.x + it.w / 2.0, it.y + it.h / 2.0);
        app.model = Drawing::from_strokes(vec![Stroke {
            ink: Ink { kind: InkKind::Pen, color: BLACK, width: 6.0 },
            points: vec![StrokePoint::at(mx - 20.0, my), StrokePoint::at(mx + 20.0, my)],
        }]);
        app.repaint(&ui, None);
        assert!(app.canvas[my as usize * app.cw + mx as usize] & 255 < 100, "the stroke is dark over the note");
    }

    #[test]
    fn a_table_takes_typing_by_cell_and_moves_with_tab_and_return() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_table(&ui);
        let id = first(&app);
        app.key(&ui, &KeyInput::Text("a".into()));
        app.key(&ui, &KeyInput::Code(9));
        app.key(&ui, &KeyInput::Text("b".into()));
        app.key(&ui, &KeyInput::Code(13));
        app.key(&ui, &KeyInput::Text("c".into()));
        let cells = app.items.get(id).unwrap().cells.clone();
        assert_eq!((cells[0].as_str(), cells[1].as_str(), cells[4].as_str()), ("a", "b", "c"), "Tab goes right, Return goes down");
        assert_eq!(app.edit, Some((id, 4)));
        app.key(&ui, &KeyInput::Code(27));
        assert_eq!(app.items.list().len(), 1, "a table with text stays");
    }

    #[test]
    fn a_tap_in_another_cell_moves_the_caret_there() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_table(&ui);
        let id = first(&app);
        let t = app.items.get(id).unwrap().clone();
        app.tool = Tool::Lasso;
        let top = app.canvas_top(&ui) as f32;
        app.touch(&ui, Event::Down(t.x + t.w - 5.0, top + t.y + 5.0), 0.0);
        app.touch(&ui, Event::Up(t.x + t.w - 5.0, top + t.y + 5.0), 0.0);
        assert_eq!(app.edit, Some((id, 2)), "the top right cell");
    }

    #[test]
    fn left_and_right_move_the_caret_and_typing_goes_where_it_is() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_text_box(&ui);
        app.key(&ui, &KeyInput::Text("ac".into()));
        app.key(&ui, &KeyInput::Code(37));
        app.key(&ui, &KeyInput::Text("b".into()));
        assert_eq!(text_of(&app, first(&app)), "abc", "the letter goes in before the caret");
        app.key(&ui, &KeyInput::Code(36));
        app.key(&ui, &KeyInput::Text(">".into()));
        assert_eq!(text_of(&app, first(&app)), ">abc", "Home moves the caret to the start");
        app.key(&ui, &KeyInput::Code(35));
        app.key(&ui, &KeyInput::Code(8));
        assert_eq!(text_of(&app, first(&app)), ">ab", "End moves the caret to the end, where Backspace takes the last letter");
        app.key(&ui, &KeyInput::Code(39));
        app.key(&ui, &KeyInput::Code(39));
        assert_eq!(app.caret, 3, "Right stops at the end of the text");
    }

    #[test]
    fn the_caret_stays_inside_the_table_cell_being_typed_into() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_table(&ui);
        let id = first(&app);
        app.key(&ui, &KeyInput::Text("ab".into()));
        for _ in 0..3 {
            app.key(&ui, &KeyInput::Code(37));
        }
        app.key(&ui, &KeyInput::Text("x".into()));
        assert_eq!(app.items.get(id).unwrap().cells[0], "xab", "Left stops at the start of the cell");
        app.key(&ui, &KeyInput::Code(9));
        app.key(&ui, &KeyInput::Text("c".into()));
        assert_eq!(app.items.get(id).unwrap().cells[1], "c", "Tab starts the next cell with the caret at its end");
    }

    #[test]
    fn a_blank_table_stays_when_typing_ends() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_table(&ui);
        app.key(&ui, &KeyInput::Code(27));
        assert_eq!(app.items.list().len(), 1);
    }

    #[test]
    fn a_table_gains_a_row_or_a_column_as_one_undo_step() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_table(&ui);
        app.key(&ui, &KeyInput::Code(27));
        assert!(app.can_add_table_part());
        let undo = app.undo.len();
        app.add_table_part(&ui, true);
        app.add_table_part(&ui, false);
        let t = app.items.list()[0].clone();
        assert_eq!((t.rows, t.cols), (4, 4));
        assert_eq!(app.undo.len(), undo + 2);
        app.undo_step();
        assert_eq!((app.items.list()[0].rows, app.items.list()[0].cols), (4, 3), "undo takes back the column");
    }

    #[test]
    fn an_item_dragged_with_snapping_on_lands_on_a_grid_point() {
        let ui = test_ui();
        let mut app = board(&ui);
        let id = app.items.add(Item::new(ItemKind::Sticky, 100.0, 100.0, 100.0, 100.0));
        app.snap = true;
        app.tool = Tool::Lasso;
        let top = app.canvas_top(&ui) as f32;
        app.touch(&ui, Event::Down(110.0, top + 110.0), 0.0);
        app.touch(&ui, Event::Move(137.0, top + 128.0), 0.0);
        app.touch(&ui, Event::Up(137.0, top + 128.0), 0.0);
        let it = app.items.get(id).unwrap();
        assert_eq!((it.x, it.y), (120.0, 120.0), "127 and 118 are nearest 120 on a 20 point grid");
        app.snap = false;
        app.touch(&ui, Event::Down(130.0, top + 130.0), 5000.0);
        app.touch(&ui, Event::Move(133.0, top + 133.0), 5000.0);
        app.touch(&ui, Event::Up(133.0, top + 133.0), 5000.0);
        let it = app.items.get(id).unwrap();
        assert_eq!((it.x, it.y), (123.0, 123.0), "without snapping it follows the pointer exactly");
    }

    #[test]
    fn pasted_text_lands_as_a_note_a_link_or_a_text_box() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.paste_text_item(&ui, "   ".into());
        assert!(app.items.is_empty(), "blank text pastes nothing");
        app.paste_text_item(&ui, "buy milk".into());
        app.paste_text_item(&ui, "https://example.com/x".into());
        app.paste_text_item(&ui, "word ".repeat(60));
        let kinds: Vec<ItemKind> = app.items.list().iter().map(|i| i.kind).collect();
        assert_eq!(kinds, vec![ItemKind::Sticky, ItemKind::Link, ItemKind::Text]);
        assert!(!app.is_editing());
        assert_eq!(app.pick, Some(Pick::Item(app.items.list()[2].id)));
    }

    #[test]
    fn dragging_an_item_with_the_lasso_moves_it_and_a_tap_does_not_make_an_undo_step() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Code(27));
        let id = first(&app);
        let it = app.items.get(id).unwrap().clone();
        let top = app.canvas_top(&ui) as f32;
        let (px, py) = (it.x + 10.0, it.y + 10.0);
        let undo = app.undo.len();
        app.touch(&ui, Event::Down(px, top + py), 1000.0);
        app.touch(&ui, Event::Up(px, top + py), 1000.0);
        assert_eq!(app.undo.len(), undo, "a tap on an item changes nothing");
        assert_eq!(app.pick, Some(Pick::Item(id)));
        app.touch(&ui, Event::Down(px, top + py), 5000.0);
        app.touch(&ui, Event::Move(px + 40.0, top + py + 20.0), 5000.0);
        app.touch(&ui, Event::Up(px + 40.0, top + py + 20.0), 5000.0);
        let moved = app.items.get(id).unwrap();
        assert_eq!((moved.x, moved.y), (it.x + 40.0, it.y + 20.0));
        assert_eq!(app.undo.len(), undo + 1);
        app.undo_step();
        assert_eq!(app.items.get(id).unwrap().x, it.x, "undo puts it back");
    }

    #[test]
    fn a_double_tap_on_a_note_starts_typing_and_on_a_link_opens_it() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Text("x".into()));
        app.key(&ui, &KeyInput::Code(27));
        let note = first(&app);
        let n = app.items.get(note).unwrap().clone();
        let top = app.canvas_top(&ui) as f32;
        for t in [100.0, 300.0] {
            app.touch(&ui, Event::Down(n.x + 5.0, top + n.y + 5.0), t);
            app.touch(&ui, Event::Up(n.x + 5.0, top + n.y + 5.0), t);
        }
        assert_eq!(app.edit, Some((note, 0)), "two quick taps edit the note");
        app.key(&ui, &KeyInput::Code(27));

        app.insert_link(&ui, Some("https://example.com/page"));
        let link = app.items.list().last().unwrap().clone();
        assert!(!app.is_editing(), "an address from the pasteboard needs no typing");
        for t in [1000.0, 1200.0] {
            app.touch(&ui, Event::Down(link.x + 5.0, top + link.y + 5.0), t);
            app.touch(&ui, Event::Up(link.x + 5.0, top + link.y + 5.0), t);
        }
        assert_eq!(app.take_open_url().as_deref(), Some("https://example.com/page"));
        assert!(app.take_open_url().is_none(), "taken once");
    }

    #[test]
    fn two_slow_taps_are_not_a_double_tap() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Text("x".into()));
        app.key(&ui, &KeyInput::Code(27));
        let n = app.items.list()[0].clone();
        let top = app.canvas_top(&ui) as f32;
        for t in [100.0, 2000.0] {
            app.touch(&ui, Event::Down(n.x + 5.0, top + n.y + 5.0), t);
            app.touch(&ui, Event::Up(n.x + 5.0, top + n.y + 5.0), t);
        }
        assert!(app.edit.is_none());
    }

    #[test]
    fn a_link_without_an_address_waits_for_typing_and_a_bad_address_does_not_open() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_link(&ui, Some("not a link"));
        assert!(app.is_editing());
        app.key(&ui, &KeyInput::Text("https://example.org".into()));
        app.key(&ui, &KeyInput::Code(13));
        assert!(!app.is_editing(), "Return ends a link");
        assert_eq!(text_of(&app, first(&app)), "https://example.org");
    }

    #[test]
    fn the_connector_tool_joins_two_items_and_the_line_follows_a_move() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Text("a".into()));
        app.key(&ui, &KeyInput::Code(27));
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Text("b".into()));
        app.key(&ui, &KeyInput::Code(27));
        let (a, b) = (app.items.list()[0].clone(), app.items.list()[1].clone());
        app.items.get_mut(b.id).unwrap().x = a.x + 400.0;
        app.items.get_mut(b.id).unwrap().y = a.y;
        let top = app.canvas_top(&ui) as f32;
        app.choose_connector(&ui);
        assert!(app.connector_tool());
        let (ax, ay) = (a.x + 20.0, a.y + 20.0);
        let (bx, by) = (a.x + 420.0, a.y + 20.0);
        app.touch(&ui, Event::Down(ax, top + ay), 0.0);
        app.touch(&ui, Event::Move(bx, top + by), 0.0);
        assert!(app.linking.is_some());
        app.touch(&ui, Event::Up(bx, top + by), 0.0);
        assert_eq!(app.items.connectors().len(), 1);
        assert_eq!(app.pick, Some(Pick::Connector(0)));
        let c = app.items.connectors()[0];
        assert_eq!((c.from, c.to, c.arrow), (a.id, b.id, true));
        // a press and release on the same item, or on nothing, joins nothing
        app.touch(&ui, Event::Down(ax, top + ay), 0.0);
        app.touch(&ui, Event::Up(ax, top + ay), 0.0);
        app.touch(&ui, Event::Down(5.0, top + 5.0), 0.0);
        app.touch(&ui, Event::Up(5.0, top + 5.0), 0.0);
        assert_eq!(app.items.connectors().len(), 1);
    }

    #[test]
    fn deleting_a_picked_item_takes_its_connectors_and_is_undoable() {
        let ui = test_ui();
        let mut app = board(&ui);
        let a = app.items.add(Item::new(ItemKind::Sticky, 10.0, 10.0, 100.0, 100.0));
        let b = app.items.add(Item::new(ItemKind::Sticky, 300.0, 10.0, 100.0, 100.0));
        app.items.connect(a, b, true);
        app.pick = Some(Pick::Item(a));
        app.delete_selection(&ui);
        assert_eq!(app.items.list().len(), 1);
        assert!(app.items.connectors().is_empty());
        assert!(app.pick.is_none());
        app.undo_step();
        assert_eq!(app.items.list().len(), 2);
        assert_eq!(app.items.connectors().len(), 1, "undo brings the connector back too");
    }

    #[test]
    fn delete_removes_a_picked_connector_and_backspace_does_the_same_with_nothing_being_typed() {
        let ui = test_ui();
        let mut app = board(&ui);
        let a = app.items.add(Item::new(ItemKind::Sticky, 10.0, 10.0, 100.0, 100.0));
        let b = app.items.add(Item::new(ItemKind::Sticky, 300.0, 10.0, 100.0, 100.0));
        app.items.connect(a, b, false);
        app.pick = Some(Pick::Connector(0));
        assert!(app.key(&ui, &KeyInput::Code(46)));
        assert!(app.items.connectors().is_empty());
        assert_eq!(app.items.list().len(), 2);
        assert!(!app.key(&ui, &KeyInput::Code(46)), "nothing is picked now");
    }

    #[test]
    fn backspace_deletes_the_selected_strokes_when_nothing_is_being_typed() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.model = Drawing::from_strokes(vec![Stroke { ink: Ink { kind: InkKind::Pen, color: BLACK, width: 4.0 }, points: vec![StrokePoint::at(5.0, 5.0)] }]);
        app.selection = Some(Selection::from_indices(vec![0]));
        assert!(app.key(&ui, &KeyInput::Code(8)));
        assert!(app.model.is_empty());
        assert!(!app.key(&ui, &KeyInput::Code(8)), "nothing is left to delete");
    }

    #[test]
    fn return_on_a_picked_note_starts_typing_into_it() {
        let ui = test_ui();
        let mut app = board(&ui);
        let a = app.items.add(Item::new(ItemKind::Sticky, 10.0, 10.0, 100.0, 100.0));
        app.pick = Some(Pick::Item(a));
        assert!(app.key(&ui, &KeyInput::Code(13)));
        assert_eq!(app.edit, Some((a, 0)));
        let pic = app.items.add(Item::new(ItemKind::Picture, 200.0, 10.0, 50.0, 50.0));
        app.finish_edit(&ui);
        app.pick = Some(Pick::Item(pic));
        assert!(!app.key(&ui, &KeyInput::Code(13)), "a picture takes no typing");
    }

    #[test]
    fn duplicate_copies_the_picked_item_and_picks_the_copy() {
        let ui = test_ui();
        let mut app = board(&ui);
        let a = app.items.add(Item { text: "x".into(), ..Item::new(ItemKind::Sticky, 10.0, 10.0, 100.0, 100.0) });
        app.pick = Some(Pick::Item(a));
        app.duplicate_selection(&ui);
        assert_eq!(app.items.list().len(), 2);
        let copy = app.items.list()[1].clone();
        assert_eq!(copy.text, "x");
        assert_eq!(copy.x, 10.0 + PASTE_OFFSET_PT);
        assert_eq!(app.pick, Some(Pick::Item(copy.id)));
    }

    #[test]
    fn larger_and_smaller_scale_the_picked_item_about_its_centre() {
        let ui = test_ui();
        let mut app = board(&ui);
        let a = app.items.add(Item::new(ItemKind::Sticky, 100.0, 100.0, 100.0, 100.0));
        app.pick = Some(Pick::Item(a));
        assert!(app.can_scale_item());
        app.scale_pick(&ui, 1.25);
        let it = app.items.get(a).unwrap();
        assert_eq!((it.w, it.h, it.x, it.y), (125.0, 125.0, 87.5, 87.5));
        app.pick = None;
        assert!(!app.can_scale_item());
    }

    #[test]
    fn a_colour_press_with_a_note_picked_recolours_the_note() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Code(27));
        let id = first(&app);
        app.pick = Some(Pick::Item(id));
        app.activate(&ui, Target::Color(5));
        assert_eq!(app.items.get(id).unwrap().color, NOTE_COLORS[5]);
        assert_eq!(app.color, 5, "the ink colour changes too");
    }

    #[test]
    fn undo_and_redo_cover_items() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Text("hi".into()));
        app.key(&ui, &KeyInput::Code(27));
        assert_eq!(app.items.list().len(), 1);
        app.undo_step();
        assert!(app.items.is_empty(), "the note is gone");
        assert!(app.can_redo());
        app.redo_step();
        assert_eq!(app.items.list().len(), 1);
        assert_eq!(text_of(&app, first(&app)), "hi");
    }

    #[test]
    fn clear_removes_the_items_too() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Code(27));
        app.activate(&ui, Target::Clear);
        assert!(app.items.is_empty());
        app.undo_step();
        assert_eq!(app.items.list().len(), 1);
    }

    #[test]
    fn items_are_saved_with_the_board_and_come_back() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "persist");
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Text("kept".into()));
        app.key(&ui, &KeyInput::Code(27));
        app.insert_table(&ui);
        app.key(&ui, &KeyInput::Text("cell".into()));
        app.key(&ui, &KeyInput::Code(27));
        let (a, b) = (app.items.list()[0].id, app.items.list()[1].id);
        app.items.connect(a, b, true);
        app.flush();
        assert!(dir.join("board.items").exists());
        let again = Freeform::new(&ui, 800, 600).with_store(dir.clone());
        assert_eq!(again.items, app.items);
        assert!(again.has_strokes(), "a board with only items has something to export");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_board_with_no_items_or_strokes_leaves_no_files() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "empty");
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Code(27));
        app.flush();
        assert!(dir.join("board.items").exists());
        app.delete_pick(&ui);
        app.flush();
        assert!(!dir.join("board.items").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_is_kept_with_the_board_as_a_card() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "file");
        app.insert_file(&ui, "../notes (1).txt", b"hello", None).unwrap();
        let it = app.items.list()[0].clone();
        assert_eq!(it.kind, ItemKind::File);
        assert_eq!(it.text, "notes (1).txt");
        assert_eq!(it.file, "notes__1_.txt");
        assert_eq!(it.size, 5);
        assert_eq!(std::fs::read(dir.join("files").join(&it.file)).unwrap(), b"hello");
        app.insert_file(&ui, "notes (1).txt", b"again", None).unwrap();
        assert_eq!(app.items.list()[1].file, "notes__1_-2.txt", "a second file of the same name does not overwrite the first");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_over_the_limit_is_refused() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "big");
        let big = vec![0u8; MAX_FILE_BYTES + 1];
        assert!(app.insert_file(&ui, "big.bin", &big, None).is_err());
        assert!(app.items.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_needs_a_folder_to_be_kept_in() {
        let ui = test_ui();
        let mut app = board(&ui);
        assert!(app.insert_file(&ui, "a.txt", b"x", None).is_err(), "a board with no store keeps no files");
    }

    /// A 2 x 2 PNG made with the app's own encoder.
    fn tiny_png() -> Vec<u8> {
        springboard::pngenc::encode_rgb(2, 2, &[0xFF00_00FF, 0xFF00_FF00, 0xFFFF_0000, 0xFFFF_FFFF])
    }

    #[test]
    fn a_picture_is_decoded_shown_at_its_size_and_painted() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "picture");
        app.insert_file(&ui, "dot.png", &tiny_png(), Some((50.0, 60.0))).unwrap();
        let it = app.items.list()[0].clone();
        assert_eq!(it.kind, ItemKind::Picture);
        assert_eq!((it.x, it.y, it.w, it.h), (50.0, 60.0, 2.0, 2.0), "a small picture is not enlarged");
        assert!(app.pics.scaled.contains_key(&it.id));
        let p = app.canvas[60 * app.cw + 50];
        assert!((p & 255) > 200 && ((p >> 8) & 255) < 60 && ((p >> 16) & 255) < 60, "its first pixel is red (R is the low byte): {p:08x}");
        // a picture read back from the store decodes again
        app.flush();
        let mut again = Freeform::new(&ui, 800, 600).with_store(dir.clone());
        again.ensure_canvas(&ui);
        again.repaint(&ui, None);
        assert!(again.pics.scaled.contains_key(&again.items.list()[0].id));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_picture_scales_with_its_item() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "picture-scale");
        app.insert_file(&ui, "dot.png", &tiny_png(), Some((50.0, 60.0))).unwrap();
        let id = first(&app);
        app.items.get_mut(id).unwrap().w = 40.0;
        app.items.get_mut(id).unwrap().h = 40.0;
        app.repaint(&ui, None);
        let img = &app.pics.scaled[&id];
        assert_eq!((img.w, img.h), (40, 40));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bytes_that_are_not_a_picture_do_not_decode() {
        assert!(decode_bounded(b"hello").is_none());
        assert!(decode_bounded(&tiny_png()).is_some());
    }

    #[test]
    fn unused_files_are_removed_when_a_board_is_read_but_used_ones_stay() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "orphans");
        app.insert_file(&ui, "keep.txt", b"k", None).unwrap();
        app.insert_file(&ui, "drop.txt", b"d", None).unwrap();
        let drop_id = app.items.list()[1].id;
        app.pick = Some(Pick::Item(drop_id));
        app.delete_pick(&ui);
        assert!(dir.join("files/drop.txt").exists(), "kept while undo could bring the item back");
        app.flush();
        let _again = Freeform::new(&ui, 800, 600).with_store(dir.clone());
        assert!(dir.join("files/keep.txt").exists());
        assert!(!dir.join("files/drop.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_duplicated_board_has_its_own_copy_of_the_items_and_files() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "dup-board");
        app.insert_file(&ui, "a.txt", b"abc", None).unwrap();
        app.duplicate_board(&ui);
        assert_eq!(app.items.list().len(), 1, "the copy has the item");
        let id = app.boards.current();
        assert!(dir.join(format!("boards/{id}/files/a.txt")).exists());
        app.delete_board(&ui);
        assert!(!dir.join(format!("boards/{id}")).exists());
        assert!(dir.join("files/a.txt").exists(), "the first board keeps its file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shared_content_lands_on_a_new_board() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "shared");
        let first_board = app.boards.current();
        app.import_shared(
            &ui,
            vec![
                Shared::Text("remember the milk".into()),
                Shared::Url("https://example.com/a".into()),
                Shared::File { name: "dot.png".into(), bytes: tiny_png() },
                Shared::Text("x".repeat(500)),
            ],
        );
        assert_ne!(app.boards.current(), first_board);
        assert_eq!(app.boards.len(), 2);
        assert!(app.boards.current_title().starts_with("Shared"));
        let kinds: Vec<ItemKind> = app.items.list().iter().map(|i| i.kind).collect();
        assert_eq!(kinds, vec![ItemKind::Sticky, ItemKind::Link, ItemKind::Picture, ItemKind::Text]);
        let ys: Vec<f32> = app.items.list().iter().map(|i| i.y).collect();
        assert!(ys.windows(2).all(|w| w[1] > w[0]), "stacked down the board: {ys:?}");
        assert!(app.pick.is_none() && app.edit.is_none());
        app.open_board(&ui, first_board);
        assert!(app.items.is_empty(), "the board that was open is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_shared_makes_no_board() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "shared-none");
        app.import_shared(&ui, Vec::new());
        assert_eq!(app.boards.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_web_address_shared_as_text_becomes_a_link() {
        let ui = test_ui();
        let (mut app, dir) = stored_board(&ui, "shared-link");
        app.import_shared(&ui, vec![Shared::Text("https://example.com/x".into())]);
        assert_eq!(app.items.list()[0].kind, ItemKind::Link);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exports_include_the_items() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Code(27));
        let layer = app.items_layer(&ui);
        assert_eq!(layer.len(), app.cw * app.ch);
        assert!(layer.iter().any(|&p| p == NOTE_COLORS[0]));
        let png = app.export_png(&ui);
        assert_eq!(&png[1..4], b"PNG");
        let pages = app.pdf_pages(&ui);
        assert_eq!(pages[0].images.len(), 1, "the items are the page's backdrop picture");
        let text = String::from_utf8(pages[0].content.clone()).unwrap();
        assert!(text.contains("/Im0 Do"));
    }

    #[test]
    fn a_board_without_items_has_a_plain_pdf_page() {
        let ui = test_ui();
        let app = board(&ui);
        assert!(app.pdf_pages(&ui)[0].images.is_empty());
    }

    fn two_boards(ui: &Ui) -> Freeform {
        let mut app = board(ui);
        app.new_board(ui);
        app.new_board(ui);
        app.boards.rename(2, "Garden plan");
        app.boards.rename(3, "Trip");
        app
    }

    #[test]
    fn the_search_lists_boards_by_title_and_opens_the_chosen_one() {
        let ui = test_ui();
        let mut app = two_boards(&ui);
        app.open_search(&ui);
        assert!(app.is_editing());
        assert_eq!(app.search_hits().len(), 3, "a blank query lists every board");
        app.key(&ui, &KeyInput::Text("gar".into()));
        assert_eq!(app.search_hits(), vec![2]);
        app.key(&ui, &KeyInput::Code(13));
        assert!(app.search.is_none());
        assert_eq!(app.boards.current(), 2);
    }

    #[test]
    fn the_search_moves_with_the_arrow_keys_and_stops_at_the_ends() {
        let ui = test_ui();
        let mut app = two_boards(&ui);
        app.open_search(&ui);
        app.key(&ui, &KeyInput::Code(40));
        app.key(&ui, &KeyInput::Code(40));
        app.key(&ui, &KeyInput::Code(40));
        assert_eq!(app.search.as_ref().unwrap().sel, 2);
        app.key(&ui, &KeyInput::Code(38));
        app.key(&ui, &KeyInput::Code(13));
        assert_eq!(app.boards.current(), 2, "the second row is board 2");
        app.open_search(&ui);
        app.key(&ui, &KeyInput::Code(38));
        assert_eq!(app.search.as_ref().unwrap().sel, 0);
    }

    #[test]
    fn escape_closes_the_search_and_a_search_with_no_match_opens_nothing() {
        let ui = test_ui();
        let mut app = two_boards(&ui);
        let before = app.boards.current();
        app.open_search(&ui);
        app.key(&ui, &KeyInput::Text("zzz".into()));
        assert!(app.search_hits().is_empty());
        app.key(&ui, &KeyInput::Code(13));
        assert!(app.search.is_none());
        assert_eq!(app.boards.current(), before);
        app.open_search(&ui);
        app.key(&ui, &KeyInput::Code(27));
        assert!(app.search.is_none());
        app.open_search(&ui);
        app.key(&ui, &KeyInput::Text("ab".into()));
        app.key(&ui, &KeyInput::Code(8));
        assert_eq!(app.search.as_ref().unwrap().query, "a");
    }

    #[test]
    fn a_press_on_a_search_row_opens_it_and_a_press_outside_closes_the_search() {
        let ui = test_ui();
        let mut app = two_boards(&ui);
        app.open_search(&ui);
        let (_, _, rows) = app.search_layout(&ui);
        let r = rows[1];
        app.touch(&ui, Event::Down(r.x + 10.0, r.y + 5.0), 0.0);
        assert!(app.search.is_none());
        assert_eq!(app.boards.current(), 2);
        app.open_search(&ui);
        app.touch(&ui, Event::Down(2.0, app.h as f32 - 2.0), 0.0);
        assert!(app.search.is_none());
        assert_eq!(app.boards.current(), 2);
    }

    #[test]
    fn renaming_edits_the_open_boards_title_and_a_blank_title_is_refused() {
        let ui = test_ui();
        let mut app = two_boards(&ui);
        app.open_rename(&ui);
        assert!(app.is_editing());
        assert_eq!(app.search.as_ref().unwrap().query, "Trip", "the field starts with the current title");
        assert!(app.search_hits().is_empty(), "no board list while renaming");
        for _ in 0..4 {
            app.key(&ui, &KeyInput::Code(8));
        }
        app.key(&ui, &KeyInput::Text("Holiday".into()));
        app.key(&ui, &KeyInput::Code(13));
        assert!(app.search.is_none());
        assert_eq!(app.boards.current_title(), "Holiday");
        app.open_rename(&ui);
        for _ in 0..7 {
            app.key(&ui, &KeyInput::Code(8));
        }
        app.key(&ui, &KeyInput::Code(13));
        assert_eq!(app.boards.current_title(), "Holiday", "a blank title changes nothing");
        app.open_rename(&ui);
        app.key(&ui, &KeyInput::Text("x".into()));
        app.key(&ui, &KeyInput::Code(27));
        assert_eq!(app.boards.current_title(), "Holiday", "Escape leaves the title alone");
    }

    #[test]
    fn a_renamed_board_is_found_by_its_new_name() {
        let ui = test_ui();
        let mut app = two_boards(&ui);
        app.open_rename(&ui);
        app.key(&ui, &KeyInput::Text(" garage".into()));
        app.key(&ui, &KeyInput::Code(13));
        app.open_search(&ui);
        app.key(&ui, &KeyInput::Text("garage".into()));
        assert_eq!(app.search_hits(), vec![3]);
    }

    #[test]
    fn the_search_swallows_board_presses_and_takes_pasted_text() {
        let ui = test_ui();
        let mut app = two_boards(&ui);
        app.open_search(&ui);
        assert!(app.paste_text(&ui, "Trip\nsecond line"));
        assert_eq!(app.search.as_ref().unwrap().query, "Trip");
        assert_eq!(app.search_hits(), vec![3]);
    }

    #[test]
    fn paste_goes_into_the_item_being_typed_into_and_nowhere_when_nothing_is() {
        let ui = test_ui();
        let mut app = board(&ui);
        assert!(!app.paste_text(&ui, "x"));
        app.insert_note(&ui);
        assert!(app.paste_text(&ui, "pasted"));
        assert_eq!(text_of(&app, first(&app)), "pasted");
    }

    #[test]
    fn keys_do_nothing_when_nothing_is_being_typed_into() {
        let ui = test_ui();
        let mut app = board(&ui);
        assert!(!app.key(&ui, &KeyInput::Text("a".into())));
        assert!(!app.key(&ui, &KeyInput::Code(8)));
    }

    #[test]
    fn a_pen_press_ends_typing_and_leaves_the_text_in_place() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Text("kept".into()));
        let top = app.canvas_top(&ui) as f32;
        app.tool = Tool::Pen;
        app.touch(&ui, Event::Down(700.0, top + 500.0), 0.0);
        app.touch(&ui, Event::Up(700.0, top + 500.0), 0.0);
        assert!(app.edit.is_none());
        assert_eq!(text_of(&app, first(&app)), "kept");
        assert!(app.pick.is_none());
    }

    #[test]
    fn switching_tools_ends_typing() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Text("kept".into()));
        app.activate(&ui, Target::Tool(Tool::Pen));
        assert!(app.edit.is_none());
    }

    #[test]
    fn the_text_tool_types_into_an_existing_note_instead_of_making_another() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.insert_note(&ui);
        app.key(&ui, &KeyInput::Text("a".into()));
        app.key(&ui, &KeyInput::Code(27));
        let n = app.items.list()[0].clone();
        app.choose_text_tool(&ui, ItemKind::Text);
        assert_eq!(app.text_tool(), Some(ItemKind::Text));
        let top = app.canvas_top(&ui) as f32;
        app.touch(&ui, Event::Down(n.x + 5.0, top + n.y + 5.0), 0.0);
        app.touch(&ui, Event::Up(n.x + 5.0, top + n.y + 5.0), 0.0);
        assert_eq!(app.items.list().len(), 1);
        assert_eq!(app.edit, Some((n.id, 0)));
    }

    #[test]
    fn the_pointer_is_a_text_cursor_for_the_text_tools_and_a_hand_over_an_item() {
        let ui = test_ui();
        let mut app = board(&ui);
        let top = app.canvas_top(&ui) as f32;
        app.tool = Tool::Text;
        assert_eq!(app.pointer_for(&ui, 100.0, top + 100.0), Kind::IBeam);
        app.tool = Tool::Connect;
        assert_eq!(app.pointer_for(&ui, 100.0, top + 100.0), Kind::Crosshair);
        app.tool = Tool::Lasso;
        let a = app.items.add(Item::new(ItemKind::Sticky, 90.0, 90.0, 100.0, 100.0));
        assert_eq!(app.pointer_for(&ui, 100.0, top + 100.0), Kind::OpenHand);
        assert_eq!(app.pointer_for(&ui, 500.0, top + 400.0), Kind::Arrow);
        assert!(app.items.get(a).is_some());
    }

    #[test]
    fn opening_another_board_resets_what_was_picked_or_typed() {
        let ui = test_ui();
        let mut app = two_boards(&ui);
        app.insert_note(&ui);
        assert!(app.edit.is_some());
        app.open_board(&ui, 1);
        assert!(app.edit.is_none() && app.pick.is_none() && app.search.is_none());
        assert!(app.items.is_empty());
    }
}
