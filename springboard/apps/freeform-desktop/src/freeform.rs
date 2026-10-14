//! Freeform: a finger drawing canvas with pen, marker and eraser, a lasso that selects
//! and moves or deletes strokes, rectangle and oval shapes, a colour palette, undo and
//! "save to Photos" (a PNG in the screenshot directory).
//!
//! Every stroke is also kept in the drawing model (`bifrost_app::controls::drawing`), so a
//! lasso can move or delete it and the canvas is repainted from the strokes. The pixel canvas
//! stays the picture that is drawn and saved.

use crate::arrange::{self, Align, Handle};
use crate::boards::{self, BoardList};
use crate::ink::{self, ShapeKind};
use crate::items::{self, Item, Items, Kind as ItemKind};
use crate::pdfout;
use crate::style::{self, Bitmap, Fill, Style, OPACITY_STEPS};
use bifrost_app::controls::cg::path::Affine;
use bifrost_app::controls::cursor::Kind;
use bifrost_app::controls::drawing::{Drawing, Ink, InkKind, Stroke, StrokePoint};
use bifrost_app::controls::lasso::{Lasso, Selection};
use bifrost_app::print::{Destination, PrintError, PrintOperation};
use bifrost_app::controls::layout_protocol::{FlowLayout, Layout};
use bifrost_app::controls::tips::{self, Datastore, Tip, TipSection, TipView, TipViewStyle};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::kit::ui::{
    Ui, Weight, BLACK, LABEL, SYSTEM_GREEN, SYSTEM_ORANGE, SYSTEM_PURPLE, SYSTEM_RED, SYSTEM_YELLOW, TINT,
    TOAST_FILL, WHITE,
};
use bifrost_app::kit::ui::{SPACE_L, SPACE_S, SPACE_XXL, SPACE_XXXL};
use bifrost_app::kit::ui::{MAC_CALLOUT_PT, MAC_FOOTNOTE_PT, SEPARATOR_C, SIDEBAR_BG, LABEL_2};
use bifrost_app::kit::{Event, Surface, TextCache};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::widgets;
use std::collections::HashMap;
use std::path::PathBuf;
use bifrost_app::kit::ui::{SPACE_M, SPACE_XS};
use bifrost_app::kit::Image;
use bifrost_app::KeyInput;

mod itemtools;
mod viewport;
pub use itemtools::{Shared, MAX_FILE_BYTES};
use itemtools::{ItemDrag, Linking, Pick, Pictures, Search};
use viewport::to_board;

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
const BAR_LASSO: u32 = 8;
const BAR_RECT: u32 = 9;
const BAR_OVAL: u32 = 10;
const BAR_DELETE: u32 = 11;
const BAR_COPY: u32 = 12;
const BAR_ARRANGE: u32 = 13;
const BAR_INSPECTOR: u32 = 14;
const BAR_PENCIL: u32 = 15;
const BAR_TEXT: u32 = 16;
const BAR_NOTE: u32 = 17;
const BAR_CONNECT: u32 = 18;

const UNDO_DEPTH: usize = 12;

/// Selection handles (points): the square's side, how far above the box the rotate handle sits, and how close a press
/// must be to grab one.
const HANDLE_PT: f32 = 8.0;
const ROTATE_GAP_PT: f32 = 22.0;
const HANDLE_REACH_PT: f32 = 9.0;
/// The grid's spacing in points, and a grid dot's side.
const GRID_PT: f32 = 20.0;
const GRID_DOT_PT: f32 = 2.0;

/// Width of the shape inspector's panel on the right, in points.
const PANEL_PT: f32 = 220.0;
/// Inspector layout (points, below the toolbar): the heading band, then one row per setting.
const HEAD_PT: f32 = 40.0;
const CAPTION_PT: f32 = 16.0;
const CONTROL_PT: f32 = 24.0;
const ROW_PT: f32 = 60.0;

/// The inspector's settings rows, each a segmented control: the row, its caption and its segment labels.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Fill,
    Width,
    Dash,
    Shadow,
    Opacity,
    Transform,
}

const ROWS: [(Row, &str, &[&str]); 6] = [
    (Row::Fill, "Fill", &["None", "Solid", "Gradient"]),
    (Row::Width, "Stroke width", &["Thin", "Medium", "Thick"]),
    (Row::Dash, "Dash", &["Solid", "Dashed"]),
    (Row::Shadow, "Shadow", &["Off", "On"]),
    (Row::Opacity, "Opacity", &["100%", "50%", "25%"]),
    (Row::Transform, "Transform", &["Turn", "Smaller", "Larger"]),
];

/// One change the inspector makes to the selected strokes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Edit {
    Fill(usize),
    Width(usize),
    Dash(usize),
    Shadow(usize),
    Opacity(usize),
    /// Rotate 15 degrees about the selection's centre.
    Turn,
    /// Scale to 80 per cent about the selection's centre.
    Smaller,
    /// Scale to 125 per cent about the selection's centre.
    Larger,
}

/// The stroke widths the inspector offers, from the pen's width: half, the pen's, and twice.
fn width_steps() -> [f32; 3] {
    let base = ink_width(Tool::Pen);
    [base * 0.5, base, base * 2.0]
}

/// The index of the value nearest to `v`.
fn nearest(values: &[f32], v: f32) -> usize {
    values
        .iter()
        .enumerate()
        .min_by(|a, b| (a.1 - v).abs().partial_cmp(&(b.1 - v).abs()).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// The segment of `row` that `i` selects.
fn edit_for(row: Row, i: usize) -> Edit {
    match row {
        Row::Fill => Edit::Fill(i),
        Row::Width => Edit::Width(i),
        Row::Dash => Edit::Dash(i),
        Row::Shadow => Edit::Shadow(i),
        Row::Opacity => Edit::Opacity(i),
        Row::Transform => match i {
            0 => Edit::Turn,
            1 => Edit::Smaller,
            _ => Edit::Larger,
        },
    }
}

/// Changes the look of one stroke for a style edit (the width is on the ink, handled by the caller).
fn apply_style(st: &mut Style, e: Edit, color: u32, next: u32) {
    match e {
        Edit::Fill(k) => {
            st.fill = match k {
                0 => Fill::None,
                1 => Fill::Solid(color),
                _ => Fill::Gradient(color, next),
            }
        }
        Edit::Dash(k) => st.dashed = k == 1,
        Edit::Shadow(k) => st.shadow = k == 1,
        Edit::Opacity(k) => st.opacity = OPACITY_STEPS[k.min(OPACITY_STEPS.len() - 1)],
        _ => {}
    }
}

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
    Pencil,
    Marker,
    Eraser,
    Lasso,
    Rectangle,
    Oval,
    Line,
    Arrow,
    Triangle,
    Star,
    /// Makes a text box where the board is pressed.
    Text,
    /// Makes a sticky note where the board is pressed.
    Sticky,
    /// Joins two items by dragging from one to the other.
    Connect,
    /// Moves the view when the board is dragged.
    Hand,
}

impl Tool {
    /// The shape this tool draws by dragging, if it is a shape tool.
    fn shape_kind(self) -> Option<ShapeKind> {
        Some(match self {
            Tool::Rectangle => ShapeKind::Rectangle,
            Tool::Oval => ShapeKind::Oval,
            Tool::Line => ShapeKind::Line,
            Tool::Arrow => ShapeKind::Arrow,
            Tool::Triangle => ShapeKind::Triangle,
            Tool::Star => ShapeKind::Star,
            _ => return None,
        })
    }

    fn of_shape(kind: ShapeKind) -> Tool {
        match kind {
            ShapeKind::Rectangle => Tool::Rectangle,
            ShapeKind::Oval => Tool::Oval,
            ShapeKind::Line => Tool::Line,
            ShapeKind::Arrow => Tool::Arrow,
            ShapeKind::Triangle => Tool::Triangle,
            ShapeKind::Star => Tool::Star,
        }
    }
}

/// A resize or rotate drag of the selection: the handle, the last pointer point and the point the drag turns or
/// scales about (canvas pixels).
#[derive(Clone, Copy, Debug)]
struct HandleDrag {
    handle: Handle,
    last: (f32, f32),
    pivot: (f32, f32),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    Tool(Tool),
    Color(usize),
    Undo,
    Clear,
    Delete,
    Copy,
    Arrange,
    Save,
    /// Shows or hides the shape inspector.
    Inspector,
    /// A change from the inspector's panel.
    Edit(Edit),
}

/// What undo puts back: the pixels, the strokes they were made of, their looks, and the board's items.
type Snapshot = (Vec<u32>, Drawing, Vec<Style>, Items);

pub struct Freeform {
    w: i32,
    h: i32,
    /// the picture, `cw` x `ch` px, always opaque
    canvas: Vec<u32>,
    cw: usize,
    ch: usize,
    undo: Vec<Snapshot>,
    /// What redo puts back: filled by undo, emptied by every new edit.
    redo: Vec<Snapshot>,
    tool: Tool,
    color: usize,
    last: Option<(f32, f32)>,
    /// a pen, marker or eraser stroke is being drawn (the last stroke of `model`)
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
    /// Every stroke on the board, in canvas pixels, in drawing order.
    model: Drawing,
    /// The strokes the last lasso loop selected.
    selection: Option<Selection>,
    /// The lasso loop being drawn (canvas pixels).
    lasso: Lasso,
    lassoing: bool,
    /// The selection is being dragged; the last point (canvas pixels).
    moving: Option<(f32, f32)>,
    /// A shape is being dragged from this point (canvas pixels).
    shape: Option<(f32, f32)>,
    /// A selection handle is being dragged.
    handle: Option<HandleDrag>,
    /// View > Show Grid: a dot grid is drawn on the board.
    grid: bool,
    /// View > Snap to Grid: shape corners land on grid points.
    snap: bool,
    /// Every board, and which is open. Board 1 is the store folder itself (see `boards`).
    boards: BoardList,
    /// Pixels per canvas pixel of the export (1 unless the display is denser than the canvas).
    export_scale: u32,
    /// The strokes Copy took, waiting for the app to put them on the pasteboard (`take_copy`).
    copied: Option<Drawing>,
    /// The first-use tip state (`None` in tests that do not need it).
    tips: Option<Datastore>,
    /// The first-use tip was counted as shown in this run.
    tip_marked: bool,
    /// The look of each stroke in `model`, at the same index (see `style`). Kept in step by `sync_styles`.
    styles: Vec<Style>,
    /// The shape inspector panel is shown on the right of the board.
    inspector: bool,
    /// Where the board is kept between runs (`board.txt` and `board.styles` in this folder), if anywhere.
    store: Option<PathBuf>,
    /// The board changed since it was last written.
    unsaved: bool,
    /// When (ms) the changed board is written: a moment after the last edit, so a stroke is not saved per point.
    save_at: Option<f64>,
    /// The text boxes, notes, tables, links, files, pictures and connectors of the open board (see `items`).
    items: Items,
    /// The item or connector that is picked.
    pick: Option<Pick>,
    /// The item and cell the keyboard types into.
    edit: Option<(u32, usize)>,
    /// The character the caret is before in that text; `usize::MAX` is the end, and it is clamped to the text.
    caret: usize,
    /// An item is being dragged.
    item_drag: Option<ItemDrag>,
    /// A connector is being drawn.
    linking: Option<Linking>,
    /// The time (ms) and item of the last press on an item, for double taps.
    last_tap: Option<(f64, u32)>,
    /// The decoded pictures of the picture items.
    pics: Pictures,
    /// A web address a double tap on a link asked to open (`take_open_url`).
    open_url: Option<String>,
    /// The board search, while it is open.
    search: Option<Search>,
    /// How far the board is magnified (1 is its own size) and how far it is shifted, in canvas pixels (see `viewport`).
    zoom: f32,
    pan: (f32, f32),
    /// The hand tool is dragging the view; the last pointer point (window pixels).
    panning: Option<(f32, f32)>,
    /// The last press or move on the window (window pixels): a paste lands where the pointer is.
    pointer: Option<(f32, f32)>,
    /// The picture must be painted again before it is next drawn (an undo or redo brought back one made at another view).
    repaint_due: bool,
}

/// Milliseconds after the last edit that the board is written.
const SAVE_DELAY_MS: f64 = 1000.0;
/// How far a duplicate or a paste lands from the original, in points.
const PASTE_OFFSET_PT: f32 = 16.0;

/// Width of the ink a tool draws, in points.
fn ink_width(tool: Tool) -> f32 {
    match tool {
        Tool::Pen
        | Tool::Lasso
        | Tool::Rectangle
        | Tool::Oval
        | Tool::Line
        | Tool::Arrow
        | Tool::Triangle
        | Tool::Star
        | Tool::Text
        | Tool::Sticky
        | Tool::Connect
        | Tool::Hand => 2.0 * SPACE_XS,
        Tool::Pencil => SPACE_XS,
        Tool::Marker => 2.0 * SPACE_M,
        Tool::Eraser => 2.0 * SPACE_L,
    }
}

/// The toolbar control that carries a toolbar id (the colour slot is resolved separately).
fn id_target(id: u32) -> Option<Target> {
    Some(match id {
        BAR_PEN => Target::Tool(Tool::Pen),
        BAR_PENCIL => Target::Tool(Tool::Pencil),
        BAR_MARKER => Target::Tool(Tool::Marker),
        BAR_ERASER => Target::Tool(Tool::Eraser),
        BAR_LASSO => Target::Tool(Tool::Lasso),
        BAR_RECT => Target::Tool(Tool::Rectangle),
        BAR_OVAL => Target::Tool(Tool::Oval),
        BAR_TEXT => Target::Tool(Tool::Text),
        BAR_NOTE => Target::Tool(Tool::Sticky),
        BAR_CONNECT => Target::Tool(Tool::Connect),
        BAR_UNDO => Target::Undo,
        BAR_CLEAR => Target::Clear,
        BAR_DELETE => Target::Delete,
        BAR_COPY => Target::Copy,
        BAR_ARRANGE => Target::Arrange,
        BAR_SAVE => Target::Save,
        BAR_INSPECTOR => Target::Inspector,
        _ => return None,
    })
}

/// The toolbar id a target is drawn under (for the pressed highlight).
fn target_id(t: Target) -> Option<u32> {
    Some(match t {
        Target::Tool(Tool::Pen) => BAR_PEN,
        Target::Tool(Tool::Pencil) => BAR_PENCIL,
        Target::Tool(Tool::Marker) => BAR_MARKER,
        Target::Tool(Tool::Eraser) => BAR_ERASER,
        Target::Tool(Tool::Lasso) => BAR_LASSO,
        Target::Tool(Tool::Rectangle) => BAR_RECT,
        Target::Tool(Tool::Oval) => BAR_OVAL,
        Target::Tool(Tool::Text) => BAR_TEXT,
        Target::Tool(Tool::Sticky) => BAR_NOTE,
        Target::Tool(Tool::Connect) => BAR_CONNECT,
        Target::Undo => BAR_UNDO,
        Target::Clear => BAR_CLEAR,
        Target::Delete => BAR_DELETE,
        Target::Copy => BAR_COPY,
        Target::Arrange => BAR_ARRANGE,
        Target::Save => BAR_SAVE,
        Target::Inspector => BAR_INSPECTOR,
        Target::Tool(_) | Target::Color(_) | Target::Edit(_) => return None,
    })
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
            redo: Vec::new(),
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
            model: Drawing::new(),
            selection: None,
            lasso: Lasso::new(),
            lassoing: false,
            moving: None,
            shape: None,
            handle: None,
            grid: false,
            snap: false,
            boards: BoardList::default(),
            export_scale: 1,
            copied: None,
            tips: None,
            tip_marked: false,
            styles: Vec::new(),
            inspector: false,
            store: None,
            unsaved: false,
            save_at: None,
            items: Items::default(),
            pick: None,
            edit: None,
            caret: usize::MAX,
            item_drag: None,
            linking: None,
            last_tap: None,
            pics: Pictures::default(),
            open_url: None,
            search: None,
            zoom: 1.0,
            pan: (0.0, 0.0),
            panning: None,
            pointer: None,
            repaint_due: false,
        }
    }

    /// Keeps the boards in `dir` between runs and loads what an earlier run left there: the board list, and the open board.
    pub fn with_store(mut self, dir: PathBuf) -> Self {
        if let Ok(text) = std::fs::read_to_string(dir.join("boards.txt")) {
            self.boards = BoardList::from_text(&text);
        }
        self.store = Some(dir);
        self.load_board();
        self
    }

    /// The folder the open board is kept in.
    fn board_dir(&self) -> Option<PathBuf> {
        self.store.as_ref().map(|root| boards::dir_for(root, self.boards.current()))
    }

    /// Replaces the strokes with what the open board's folder holds (nothing when it holds no board).
    fn load_board(&mut self) {
        self.model = Drawing::new();
        self.styles.clear();
        self.items = Items::default();
        self.pics = Pictures::default();
        self.pick = None;
        self.edit = None;
        self.caret = usize::MAX;
        self.item_drag = None;
        self.linking = None;
        self.last_tap = None;
        self.search = None;
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
        self.panning = None;
        let Some(dir) = self.board_dir() else { return };
        if let Ok(text) = std::fs::read_to_string(dir.join("board.txt")) {
            if let Ok(model) = Drawing::from_text(&text) {
                let looks = std::fs::read_to_string(dir.join("board.styles")).map(|t| style::styles_from_text(&t)).unwrap_or_default();
                self.model = model;
                self.styles = looks;
                self.sync_styles();
            }
        }
        if let Ok(text) = std::fs::read_to_string(dir.join("board.items")) {
            self.items = Items::from_text(&text);
            let declared = text.lines().filter(|l| l.starts_with("item\t")).count();
            self.drop_unused_files(&dir, declared);
        }
    }

    /// Writes the board list to the store folder.
    fn write_list(&self) {
        let Some(root) = &self.store else { return };
        let _ = std::fs::create_dir_all(root);
        let _ = springboard::shell::fsutil::write_atomic(&root.join("boards.txt"), self.boards.to_text().into_bytes());
    }

    /// Writes the open board to its folder. A board with no strokes removes the stroke files, and one with no items
    /// removes the item file.
    fn write_board(&self) -> std::io::Result<()> {
        let Some(dir) = self.board_dir() else { return Ok(()) };
        std::fs::create_dir_all(&dir)?;
        if self.items.is_empty() {
            let _ = std::fs::remove_file(dir.join("board.items"));
        } else {
            springboard::shell::fsutil::write_atomic(&dir.join("board.items"), self.items.to_text().into_bytes())?;
        }
        if self.model.is_empty() {
            let _ = std::fs::remove_file(dir.join("board.txt"));
            let _ = std::fs::remove_file(dir.join("board.styles"));
            return Ok(());
        }
        let mut looks = self.styles.clone();
        looks.resize(self.model.strokes().len(), Style::default());
        springboard::shell::fsutil::write_atomic(&dir.join("board.styles"), style::styles_to_text(&looks).into_bytes())?;
        springboard::shell::fsutil::write_atomic(&dir.join("board.txt"), self.model.to_text().into_bytes())
    }

    /// Notes an edit: the board is written `SAVE_DELAY_MS` after the last one.
    fn mark_changed(&mut self) {
        if self.store.is_some() {
            self.unsaved = true;
            self.save_at = Some(self.now + SAVE_DELAY_MS);
        }
    }

    /// Writes a changed board at once (the app is quitting or leaving the foreground).
    pub fn flush(&mut self) {
        if self.unsaved {
            self.unsaved = false;
            self.save_at = None;
            let _ = self.write_board();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn has_selection(&self) -> bool {
        self.selection.as_ref().is_some_and(|s| !s.is_empty())
    }

    /// Edit > Undo: the board goes back one step, and the step it left is kept for Redo.
    pub fn undo_step(&mut self) {
        let Some((canvas, model, looks, items)) = self.undo.pop() else { return };
        self.sync_styles();
        self.redo.push((
            std::mem::replace(&mut self.canvas, canvas),
            std::mem::replace(&mut self.model, model),
            std::mem::replace(&mut self.styles, looks),
            std::mem::replace(&mut self.items, items),
        ));
        self.end_item_work();
        self.repaint_due = true;
        self.selection = None;
        self.dirty = true;
        self.mark_changed();
    }

    /// Edit > Redo: the step Undo took back is made again.
    pub fn redo_step(&mut self) {
        let Some((canvas, model, looks, items)) = self.redo.pop() else { return };
        self.sync_styles();
        self.undo.push((
            std::mem::replace(&mut self.canvas, canvas),
            std::mem::replace(&mut self.model, model),
            std::mem::replace(&mut self.styles, looks),
            std::mem::replace(&mut self.items, items),
        ));
        self.end_item_work();
        self.repaint_due = true;
        self.selection = None;
        self.dirty = true;
        self.mark_changed();
    }

    /// Where a paste lands: the board point under the last press or move on the canvas, or `None` when the pointer
    /// was over the toolbar or has not touched the window yet.
    fn paste_anchor(&self, ui: &Ui) -> Option<(f32, f32)> {
        let (x, y) = self.pointer?;
        (y >= self.canvas_top(ui) as f32).then(|| self.canvas_point(ui, x, y))
    }

    /// Puts `copies` on the board as new strokes and selects them (one undo step). With `at`, their box is centred on
    /// that board point; without it they are moved by one paste offset.
    fn add_copies(&mut self, ui: &Ui, copies: Vec<(Stroke, Style)>, at: Option<(f32, f32)>) {
        if copies.is_empty() {
            return;
        }
        self.push_undo();
        self.sync_styles();
        let base = self.model.strokes().len();
        let n = copies.len();
        let (dx, dy) = match at {
            Some(p) => {
                let boxes: Vec<Rect> = copies.iter().filter_map(|(s, _)| s.bounds()).collect();
                arrange::union_of(&boxes).map_or((0.0, 0.0), |b| arrange::centre_on(b, p))
            }
            None => (ui.pt(PASTE_OFFSET_PT), ui.pt(PASTE_OFFSET_PT)),
        };
        let off = Affine::translate(dx, dy);
        for (stroke, _) in copies.iter() {
            let mut moved = stroke.clone();
            moved.transform(off);
            self.model.push(moved);
        }
        self.sync_styles();
        // a copy joins a fresh group of its own (never the original's) and starts unlocked
        let mut next = self.styles.iter().map(|st| st.group).max().unwrap_or(0) + 1;
        let mut renamed: Vec<(u32, u32)> = Vec::new();
        for (i, (_, look)) in copies.iter().enumerate() {
            let mut look = *look;
            look.locked = false;
            if look.group != 0 {
                look.group = match renamed.iter().find(|r| r.0 == look.group) {
                    Some(r) => r.1,
                    None => {
                        renamed.push((look.group, next));
                        next += 1;
                        next - 1
                    }
                };
            }
            self.styles[base + i] = look;
        }
        self.selection = Some(Selection::from_indices((base..base + n).collect()));
        self.tool = Tool::Lasso;
        self.repaint(ui, None);
        self.mark_changed();
    }

    /// Edit > Duplicate: a copy of the selected strokes, offset a little and selected.
    pub fn duplicate_selection(&mut self, ui: &Ui) {
        if let Some(Pick::Item(id)) = self.pick {
            self.duplicate_item(ui, id);
            return;
        }
        let Some(sel) = self.selection.clone() else { return };
        self.sync_styles();
        let copies: Vec<(Stroke, Style)> = self
            .model
            .strokes()
            .iter()
            .zip(self.styles.iter())
            .enumerate()
            .filter(|(i, _)| sel.contains_stroke(*i))
            .map(|(_, (s, st))| (s.clone(), *st))
            .collect();
        self.add_copies(ui, copies, None);
    }

    /// Edit > Paste: strokes read from the pasteboard land on the board at the pointer, selected.
    pub fn paste_strokes(&mut self, ui: &Ui, strokes: &Drawing) {
        let copies = strokes.strokes().iter().map(|s| (s.clone(), Style::default())).collect();
        let at = self.paste_anchor(ui);
        self.add_copies(ui, copies, at);
    }

    /// Arrange > Bring to Front / Send to Back: the selected strokes move to the top or the bottom of the drawing
    /// order, keeping their own order. One undo step; nothing happens when they are there already.
    pub fn reorder_selection(&mut self, ui: &Ui, front: bool) {
        let Some(sel) = self.selection.clone() else { return };
        self.sync_styles();
        let total = self.model.strokes().len();
        let (mut kept, mut moved): (Vec<(Stroke, Style)>, Vec<(Stroke, Style)>) = (Vec::new(), Vec::new());
        for (i, (stroke, look)) in self.model.strokes().iter().zip(self.styles.iter()).enumerate() {
            if sel.contains_stroke(i) {
                moved.push((stroke.clone(), *look));
            } else {
                kept.push((stroke.clone(), *look));
            }
        }
        let m = moved.len();
        if m == 0 {
            return;
        }
        let target: Vec<usize> = if front { (total - m..total).collect() } else { (0..m).collect() };
        if sel.indices() == target.as_slice() {
            return;
        }
        let ordered: Vec<(Stroke, Style)> = if front { kept.into_iter().chain(moved).collect() } else { moved.into_iter().chain(kept).collect() };
        self.push_undo();
        let (strokes, looks): (Vec<Stroke>, Vec<Style>) = ordered.into_iter().unzip();
        self.model = Drawing::from_strokes(strokes);
        self.styles = looks;
        self.selection = Some(Selection::from_indices(target));
        self.repaint(ui, None);
        self.mark_changed();
    }

    /// Arrange > Arrange in Row, from the menu.
    pub fn arrange_in_row(&mut self, ui: &Ui) {
        self.arrange_selection(ui);
        self.mark_changed();
    }

    /// Edit > Delete from the menu.
    pub fn delete_selected(&mut self, ui: &Ui) {
        self.delete_selection(ui);
        self.mark_changed();
    }

    /// Edit > Copy from the menu.
    pub fn copy_selected(&mut self) {
        self.copy_selection();
        self.dirty = true;
    }

    /// Insert > shape: the next drag on the board draws this shape.
    pub fn choose_shape(&mut self, kind: ShapeKind) {
        self.tool = Tool::of_shape(kind);
        self.selection = None;
        self.dirty = true;
    }

    /// The shape tool in use, if the tool is a shape.
    pub fn shape_tool(&self) -> Option<ShapeKind> {
        self.tool.shape_kind()
    }

    /// The selected strokes that are not locked, or `None` when none is left to change.
    fn editable(&self) -> Option<Selection> {
        let sel = self.selection.as_ref()?;
        let ids: Vec<usize> = sel.indices().iter().copied().filter(|&i| !self.styles.get(i).is_some_and(|st| st.locked)).collect();
        Some(Selection::from_indices(ids)).filter(|s| !s.is_empty())
    }

    /// `sel` and every other stroke that shares a group with one of its strokes.
    fn expand_groups(&mut self, sel: Selection) -> Selection {
        self.sync_styles();
        let groups: Vec<u32> = sel.indices().iter().filter_map(|&i| self.styles.get(i)).map(|st| st.group).filter(|&g| g != 0).collect();
        if groups.is_empty() {
            return sel;
        }
        let mut ids = sel.indices().to_vec();
        ids.extend(self.styles.iter().enumerate().filter(|(_, st)| groups.contains(&st.group)).map(|(i, _)| i));
        Selection::from_indices(ids)
    }

    /// The box the selection handles surround: the unlocked part of the selection, with the lasso tool in hand.
    fn handle_box(&self) -> Option<Rect> {
        if self.tool != Tool::Lasso {
            return None;
        }
        self.editable()?.bounds(&self.model)
    }

    /// The selection handle under canvas point (x, y), if any.
    fn handle_at(&self, ui: &Ui, x: f32, y: f32) -> Option<Handle> {
        arrange::hit_handle(self.handle_box()?, ui.pt(ROTATE_GAP_PT) / self.zoom, ui.pt(HANDLE_REACH_PT) / self.zoom, x, y)
    }

    /// `p` moved to the nearest grid point when snapping is on.
    fn snapped(&self, ui: &Ui, p: (f32, f32)) -> (f32, f32) {
        if self.snap {
            let step = ui.pt(GRID_PT);
            (arrange::snap(p.0, step), arrange::snap(p.1, step))
        } else {
            p
        }
    }

    /// View > Show Grid / Hide Grid.
    pub fn toggle_grid(&mut self) {
        self.grid = !self.grid;
        self.dirty = true;
    }

    /// View > Snap to Grid.
    pub fn toggle_snap(&mut self) {
        self.snap = !self.snap;
    }

    pub fn grid_shown(&self) -> bool {
        self.grid
    }

    pub fn snapping(&self) -> bool {
        self.snap
    }

    /// How many strokes are selected, how many of them are grouped, locked, and unlocked.
    fn selection_counts(&self) -> (usize, usize, usize, usize) {
        let Some(sel) = &self.selection else { return (0, 0, 0, 0) };
        let looks: Vec<Style> = sel.indices().iter().map(|&i| self.styles.get(i).copied().unwrap_or_default()).collect();
        let locked = looks.iter().filter(|st| st.locked).count();
        (looks.len(), looks.iter().filter(|st| st.group != 0).count(), locked, looks.len() - locked)
    }

    /// Arrange > Group needs two or more strokes that are not already one group.
    pub fn can_group(&self) -> bool {
        let Some(sel) = &self.selection else { return false };
        let first = sel.indices().first().and_then(|&i| self.styles.get(i)).map_or(0, |st| st.group);
        let one_group = first != 0 && sel.indices().iter().all(|&i| self.styles.get(i).is_some_and(|st| st.group == first));
        sel.indices().len() >= 2 && !one_group
    }

    pub fn can_ungroup(&self) -> bool {
        self.selection_counts().1 > 0
    }

    pub fn can_lock(&self) -> bool {
        self.selection_counts().3 > 0
    }

    pub fn can_unlock(&self) -> bool {
        self.selection_counts().2 > 0
    }

    /// Whether the selection has at least `n` things that move as one (a group counts once) that can be changed.
    pub fn has_units(&self, n: usize) -> bool {
        self.editable_units().len() >= n
    }

    /// The unlocked selected strokes, joined into the groups they move with.
    fn editable_units(&self) -> Vec<Vec<usize>> {
        let Some(sel) = self.editable() else { return Vec::new() };
        let groups: Vec<u32> = self.styles.iter().map(|st| st.group).collect();
        arrange::units(sel.indices(), &groups)
    }

    /// Changes the looks of the selected strokes as one undo step.
    fn set_selected_looks(&mut self, change: impl Fn(&mut Style)) {
        self.sync_styles();
        let Some(sel) = self.selection.clone() else { return };
        self.push_undo();
        for &i in sel.indices() {
            if let Some(st) = self.styles.get_mut(i) {
                change(st);
            }
        }
        self.mark_changed();
        self.dirty = true;
    }

    /// Arrange > Group: the selected strokes are selected and moved together from now on.
    pub fn group_selection(&mut self) {
        if !self.can_group() {
            return;
        }
        let id = self.styles.iter().map(|st| st.group).max().unwrap_or(0) + 1;
        self.set_selected_looks(|st| st.group = id);
    }

    /// Arrange > Ungroup.
    pub fn ungroup_selection(&mut self) {
        if self.can_ungroup() {
            self.set_selected_looks(|st| st.group = 0);
        }
    }

    /// Arrange > Lock: the selected strokes stay where they are and cannot be changed or deleted.
    pub fn lock_selection(&mut self) {
        if self.can_lock() {
            self.set_selected_looks(|st| st.locked = true);
        }
    }

    /// Arrange > Unlock.
    pub fn unlock_selection(&mut self) {
        if self.can_unlock() {
            self.set_selected_looks(|st| st.locked = false);
        }
    }

    /// The box of the strokes in `unit`.
    fn unit_box(&self, unit: &[usize]) -> Option<Rect> {
        let boxes: Vec<Rect> = unit.iter().filter_map(|&i| self.model.strokes().get(i)?.bounds()).collect();
        arrange::union_of(&boxes)
    }

    /// Moves each unit (a group or a loose stroke) of the selection by the offset `plan` gives for its box. One undo
    /// step, and nothing when no unit has to move.
    fn move_units(&mut self, ui: &Ui, plan: impl Fn(&[Rect]) -> Vec<(f32, f32)>) {
        self.sync_styles();
        let units: Vec<(Vec<usize>, Rect)> = self.editable_units().into_iter().filter_map(|u| Some((u.clone(), self.unit_box(&u)?))).collect();
        let boxes: Vec<Rect> = units.iter().map(|(_, b)| *b).collect();
        let offsets = plan(&boxes);
        if offsets.iter().all(|&(dx, dy)| dx.abs() < 0.01 && dy.abs() < 0.01) {
            return;
        }
        self.push_undo();
        for ((unit, _), (dx, dy)) in units.iter().zip(offsets) {
            for &i in unit {
                if let Some(s) = self.model.stroke_mut(i) {
                    s.transform(Affine::translate(dx, dy));
                }
            }
        }
        self.repaint(ui, None);
        self.mark_changed();
    }

    /// Arrange > Align: the selected items line up on an edge or centre line of the selection.
    pub fn align_selection(&mut self, ui: &Ui, how: Align) {
        self.move_units(ui, |boxes| arrange::align_offsets(boxes, how));
    }

    /// Arrange > Distribute: the gaps between three or more items become equal.
    pub fn distribute_selection(&mut self, ui: &Ui, horizontal: bool) {
        self.move_units(ui, |boxes| arrange::distribute_offsets(boxes, horizontal));
    }

    /// Every board as (id, title, open).
    pub fn board_entries(&self) -> Vec<(u32, String, bool)> {
        self.boards.entries().iter().map(|e| (e.id, e.title.clone(), e.id == self.boards.current())).collect()
    }

    pub fn board_count(&self) -> usize {
        self.boards.len()
    }

    /// Writes the open board and starts on board `id`: the history and the selection belong to the board left behind.
    pub fn open_board(&mut self, ui: &Ui, id: u32) {
        if id == self.boards.current() {
            return;
        }
        self.flush();
        if !self.boards.open(id) {
            return;
        }
        self.show_loaded_board(ui);
        self.write_list();
        self.say(format!("Opened {}", self.boards.current_title()));
    }

    /// Reads the open board from its folder and paints it, with no history, selection or gesture left over.
    fn show_loaded_board(&mut self, ui: &Ui) {
        self.undo.clear();
        self.redo.clear();
        self.selection = None;
        self.lasso = Lasso::new();
        self.lassoing = false;
        self.moving = None;
        self.shape = None;
        self.handle = None;
        self.drawing = false;
        self.end_item_work();
        self.search = None;
        self.load_board();
        self.ensure_canvas(ui);
        self.repaint(ui, None);
    }

    /// File > New Board: an empty board is added and opened.
    pub fn new_board(&mut self, ui: &Ui) {
        self.flush();
        let id = self.boards.add();
        self.open_board(ui, id);
    }

    /// File > Duplicate Board: a copy of the open board, opened.
    pub fn duplicate_board(&mut self, ui: &Ui) {
        self.flush();
        let from = self.boards.current();
        let Some(id) = self.boards.duplicate(from) else { return };
        if let Some(root) = self.store.clone() {
            let (src, dst) = (boards::dir_for(&root, from), boards::dir_for(&root, id));
            let _ = std::fs::create_dir_all(&dst);
            for name in ["board.txt", "board.styles", "board.items"] {
                let _ = std::fs::copy(src.join(name), dst.join(name));
            }
            if let Ok(files) = std::fs::read_dir(src.join("files")) {
                let _ = std::fs::create_dir_all(dst.join("files"));
                for entry in files.flatten() {
                    let _ = std::fs::copy(entry.path(), dst.join("files").join(entry.file_name()));
                }
            }
        }
        self.open_board(ui, id);
    }

    /// File > Delete Board: the open board and its file go, and the board before it opens. The last board stays.
    pub fn delete_board(&mut self, ui: &Ui) {
        let id = self.boards.current();
        if !self.boards.remove(id) {
            return;
        }
        self.unsaved = false;
        self.save_at = None;
        if let Some(root) = &self.store {
            let dir = boards::dir_for(root, id);
            if id == 1 {
                let _ = std::fs::remove_file(dir.join("board.txt"));
                let _ = std::fs::remove_file(dir.join("board.styles"));
                let _ = std::fs::remove_file(dir.join("board.items"));
                let _ = std::fs::remove_dir_all(dir.join("files"));
            } else {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
        self.show_loaded_board(ui);
        self.write_list();
        self.say(format!("Deleted the board; opened {}", self.boards.current_title()));
    }

    /// Switches to the next or previous board.
    pub fn step_board(&mut self, ui: &Ui, forward: bool) {
        let id = self.boards.neighbour(forward);
        self.open_board(ui, id);
    }

    /// Shows `msg` in the toast for a moment.
    fn say(&mut self, msg: String) {
        self.toast = Some((msg, self.now + 2500.0));
        self.dirty = true;
    }

    /// The board as the pages a PDF or a print job carries.
    fn pdf_pages(&self, ui: &Ui) -> Vec<bifrost_app::print::PdfPage> {
        // the page shows the board as it is in view: moved and scaled strokes, with their widths scaled too
        let mut shown: Vec<Stroke> = self.view_strokes().strokes().to_vec();
        for s in shown.iter_mut() {
            s.ink.width *= self.zoom;
        }
        let page = pdfout::board_page(&shown, &self.styles, self.cw, self.ch, ui.s);
        if self.items.is_empty() {
            return vec![page];
        }
        // the items are painted once and sit under the vector ink as the page's backdrop picture
        let layer = self.items_layer(ui);
        let rgb: Vec<u8> = layer.iter().flat_map(|p| [*p as u8, (*p >> 8) as u8, (*p >> 16) as u8]).collect();
        vec![pdfout::with_backdrop(page, self.cw, self.ch, rgb)]
    }

    /// File > Export as PDF: the board, as a vector page, is written to the Photos folder.
    pub fn export_as_pdf(&mut self, ui: &Ui) {
        let result = (|| -> Result<String, String> {
            let dir = self.save_dir.as_ref().ok_or("no Photos folder")?;
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let name = free_name_ext(dir, millis, "pdf");
            PrintOperation::pages("Freeform board", self.pdf_pages(ui), Vec::new())
                .to(Destination::Pdf(dir.join(&name)))
                .run()
                .map_err(print_message)?;
            Ok(name)
        })();
        let msg = match result {
            Ok(name) => format!("Exported {name} as PDF"),
            Err(e) => format!("Could not export: {e}"),
        };
        self.say(msg);
    }

    /// File > Print: the board goes to the default printer.
    pub fn print_board(&mut self, ui: &Ui) {
        let msg = match PrintOperation::pages(self.boards.current_title().to_string(), self.pdf_pages(ui), Vec::new()).run() {
            Ok(_) => "Sent to the printer".to_string(),
            Err(e) => print_message(e),
        };
        self.say(msg);
    }

    /// Whether the board has anything, strokes or items, to export.
    pub fn has_strokes(&self) -> bool {
        !self.model.is_empty() || !self.items.is_empty()
    }

    /// Keeps one `Style` per stroke of the model: new strokes get the default look, removed ones lose theirs.
    fn sync_styles(&mut self) {
        let n = self.model.strokes().len();
        self.styles.resize(n, Style::default());
    }

    /// The board's width in pixels: the window, less the inspector panel when it is shown.
    fn board_px(&self, ui: &Ui) -> i32 {
        if self.inspector {
            (self.w - ui.pt(PANEL_PT) as i32).max(1)
        } else {
            self.w.max(1)
        }
    }

    /// The inspector panel's left edge, in points.
    fn panel_x(&self, ui: &Ui) -> f32 {
        self.board_px(ui) as f32 / ui.s
    }

    /// The inspector panel's width, in points.
    fn panel_width(&self, ui: &Ui) -> f32 {
        (self.w - self.board_px(ui)) as f32 / ui.s
    }

    /// The segmented control of inspector row `i`, in points.
    fn row_control(&self, ui: &Ui, i: usize) -> Rect {
        Rect {
            x: self.panel_x(ui) + SPACE_M,
            y: UNIFIED_BAR_PT + HEAD_PT + i as f32 * ROW_PT + CAPTION_PT,
            w: self.panel_width(ui) - 2.0 * SPACE_M,
            h: CONTROL_PT,
        }
    }

    /// The first selected stroke's look and ink, or `None` with no selection.
    fn selected_look(&self) -> Option<(Style, Ink)> {
        let sel = self.selection.as_ref()?;
        let i = *sel.indices().first()?;
        let ink = self.model.strokes().get(i)?.ink;
        Some((self.styles.get(i).copied().unwrap_or_default(), ink))
    }

    /// The segment the first selected stroke has in `row`, or `None` for a row with no state (Transform).
    fn current_index(row: Row, st: &Style, ink: &Ink) -> Option<usize> {
        Some(match row {
            Row::Fill => match st.fill {
                Fill::None => 0,
                Fill::Solid(_) => 1,
                Fill::Gradient(..) => 2,
            },
            Row::Width => nearest(&width_steps(), ink.width),
            Row::Dash => st.dashed as usize,
            Row::Shadow => st.shadow as usize,
            Row::Opacity => nearest(&OPACITY_STEPS, st.opacity),
            Row::Transform => return None,
        })
    }

    /// The inspector control under point (x, y) (points), if the panel is shown and there is a selection to change.
    fn inspector_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        if !self.inspector || x < self.panel_x(ui) || !self.selection.as_ref().is_some_and(|s| !s.is_empty()) {
            return None;
        }
        for (i, (row, _, labels)) in ROWS.iter().enumerate() {
            let r = self.row_control(ui, i);
            if x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h {
                let n = labels.len();
                let idx = (((x - r.x) / r.w * n as f32).floor().max(0.0) as usize).min(n - 1);
                return Some(Target::Edit(edit_for(*row, idx)));
            }
        }
        None
    }

    /// Draws the inspector panel: a heading, then one segmented control per setting, with its caption.
    fn draw_inspector(&self, s: &mut Surface, ui: &Ui) {
        let top = self.canvas_top(ui);
        let x = self.board_px(ui);
        s.fill_rrect(x as f32, top as f32, (self.w - x) as f32, (self.h - top).max(0) as f32, 0.0, SIDEBAR_BG.now());
        s.fill_rect(x as usize, top as usize, ui.pt(1.0).max(1.0) as usize, (self.h - top).max(0) as usize, SEPARATOR_C.now());
        let head = ui.text("Inspector", MAC_CALLOUT_PT, Weight::Semibold, LABEL.now());
        s.blit(&head, ui.pt(self.panel_x(ui) + SPACE_M) as i32, ui.pt(UNIFIED_BAR_PT + SPACE_M) as i32, 255, 255);
        let look = self.selected_look();
        let enabled = look.is_some();
        for (i, (row, caption, labels)) in ROWS.iter().enumerate() {
            let r = self.row_control(ui, i);
            let cap = ui.text(caption, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
            s.blit(&cap, ui.pt(r.x) as i32, ui.pt(r.y - CAPTION_PT) as i32, 255, 255);
            let current = look.as_ref().and_then(|(st, ink)| Self::current_index(*row, st, ink)).unwrap_or(labels.len());
            widgets::segmented(s, ui, r, *labels, current, enabled);
        }
        if !enabled {
            let max_px = ui.pt(self.panel_width(ui) - 2.0 * SPACE_M);
            let msg = ui.fit("Select strokes with Lasso to change them.", MAC_FOOTNOTE_PT, max_px);
            let img = ui.text(&msg, MAC_FOOTNOTE_PT, Weight::Regular, LABEL_2.now());
            s.blit(&img, ui.pt(self.panel_x(ui) + SPACE_M) as i32, ui.pt(UNIFIED_BAR_PT + HEAD_PT + ROWS.len() as f32 * ROW_PT) as i32, 255, 255);
        }
    }

    /// The export factor from the display's backing scale (`services` learns it from the screens service).
    pub fn set_export_scale(&mut self, factor: u32) {
        self.export_scale = factor.max(1);
    }

    /// The strokes the last Copy took, once: the app puts them on the pasteboard.
    pub fn take_copy(&mut self) -> Option<Drawing> {
        self.copied.take()
    }

    /// The pointer shape over window point (x, y): the arrow over the toolbar; over the board, the tool's drawing
    /// cursor (crosshair for the pen, marker and shapes, the poof for the eraser), and the hands over a lasso selection.
    pub fn pointer_for(&self, ui: &Ui, x: f32, y: f32) -> Kind {
        if y < self.canvas_top(ui) as f32 {
            return Kind::Arrow;
        }
        match self.tool {
            Tool::Pen | Tool::Pencil | Tool::Marker | Tool::Rectangle | Tool::Oval | Tool::Line | Tool::Arrow | Tool::Triangle | Tool::Star => {
                Kind::Crosshair
            }
            Tool::Connect => Kind::Crosshair,
            Tool::Text | Tool::Sticky => Kind::IBeam,
            Tool::Hand => {
                if self.panning.is_some() {
                    Kind::ClosedHand
                } else {
                    Kind::OpenHand
                }
            }
            Tool::Eraser => Kind::DisappearingItem,
            Tool::Lasso => {
                let (cx, cy) = self.canvas_point(ui, x, y);
                if self.moving.is_some() || self.item_drag.is_some() {
                    Kind::ClosedHand
                } else if self.handle.is_some() || self.handle_at(ui, cx, cy).is_some() {
                    Kind::Crosshair
                } else if self.selection_contains(cx, cy) || self.items.hit(cx, cy).is_some() {
                    Kind::OpenHand
                } else {
                    Kind::Arrow
                }
            }
        }
    }

    /// The first-use tip state: the hint is shown until the person first presses the board.
    pub fn with_tips(mut self, store: Datastore) -> Self {
        self.tips = Some(store);
        self
    }

    /// The first-use hint's sections at `now` (seconds). Empty once it was dismissed, or with no store.
    fn tip_sections(&self, now: tips::Seconds) -> Vec<TipSection> {
        match &self.tips {
            Some(store) => TipView::new(&LassoHint, TipViewStyle::Mini).sections(store, now),
            None => Vec::new(),
        }
    }

    /// A press on the board dismisses the first-use hint for good (until the tip state is reset).
    fn dismiss_tip(&mut self) {
        if let Some(store) = &mut self.tips {
            let _ = store.invalidate(LassoHint.id());
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

    /// Freeform's toolbar: the tools as one segmented group, the colour wells, then undo, clear, delete the
    /// selection and share.
    fn bar(&self) -> Toolbar {
        Toolbar::new(vec![
            ToolbarItem::button(BAR_PEN, "Pen").in_group(1).selected(self.tool == Tool::Pen),
            ToolbarItem::button(BAR_PENCIL, "Pencil").in_group(1).selected(self.tool == Tool::Pencil),
            ToolbarItem::button(BAR_MARKER, "Marker").in_group(1).selected(self.tool == Tool::Marker),
            ToolbarItem::button(BAR_ERASER, "Eraser").in_group(1).selected(self.tool == Tool::Eraser),
            ToolbarItem::button(BAR_LASSO, "Lasso").in_group(1).selected(self.tool == Tool::Lasso),
            ToolbarItem::button(BAR_RECT, "Rectangle").in_group(1).selected(self.tool == Tool::Rectangle),
            ToolbarItem::button(BAR_OVAL, "Oval").in_group(1).selected(self.tool == Tool::Oval),
            ToolbarItem::button(BAR_TEXT, "Text").in_group(1).selected(self.tool == Tool::Text),
            ToolbarItem::button(BAR_NOTE, "Note").in_group(1).selected(self.tool == Tool::Sticky),
            ToolbarItem::button(BAR_CONNECT, "Connect").in_group(1).selected(self.tool == Tool::Connect),
            ToolbarItem::custom(BAR_COLORS, "Colours", WELL_PT * COLORS.len() as f32 + SPACE_S * 2.0),
            ToolbarItem::flexible_space(),
            ToolbarItem::button(BAR_UNDO, "Undo").in_group(2).enabled(!self.undo.is_empty()),
            ToolbarItem::button(BAR_CLEAR, "Clear").in_group(2),
            ToolbarItem::button(BAR_ARRANGE, "Arrange").in_group(2).enabled(self.selection.is_some()),
            ToolbarItem::button(BAR_COPY, "Copy").in_group(2).enabled(self.selection.is_some()),
            ToolbarItem::button(BAR_DELETE, "Delete").in_group(2).enabled(self.selection.is_some() || self.pick.is_some()),
            ToolbarItem::button(BAR_INSPECTOR, "Inspector").in_group(3).selected(self.inspector),
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
        let nw = self.board_px(ui) as usize;
        let nh = (self.h - self.canvas_top(ui)).max(1) as usize;
        if nw == self.cw && nh == self.ch && !self.canvas.is_empty() {
            return;
        }
        // a board loaded from the store has strokes but no picture yet: it is painted at the first allocation too
        let resized = !self.canvas.is_empty() || !self.model.is_empty() || !self.items.is_empty();
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
            for snap in self.undo.iter_mut().chain(self.redo.iter_mut()) {
                snap.0 = remap(&snap.0, ow, oh);
            }
        }
        self.cw = nw;
        self.ch = nh;
        if resized {
            // the strokes are the picture: a board that grew or shrank (the inspector came or went) is painted again
            self.repaint(ui, None);
        }
    }

    fn radius(&self, ui: &Ui) -> f32 {
        ui.pt(ink_width(self.tool) / 2.0) * self.zoom
    }

    fn push_undo(&mut self) {
        self.sync_styles();
        if self.undo.len() >= UNDO_DEPTH {
            self.undo.remove(0);
        }
        self.undo.push((self.canvas.clone(), self.model.clone(), self.styles.clone(), self.items.clone()));
        self.redo.clear();
    }

    /// Stamps a filled disc at canvas coordinates (x, y), in the live tool's ink.
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
                        let base = self.undo.last().and_then(|s| s.0.get(i)).copied().unwrap_or(PAPER);
                        let a = (96.0 * cover) as u32;
                        let want = ink::mix(base, color, a);
                        // keep the stronger of the existing and the new mix
                        let cur = self.canvas[i];
                        if lum(want) < lum(cur) || cur == base {
                            self.canvas[i] = want;
                        }
                    }
                    _ => self.canvas[i] = ink::mix(self.canvas[i], color, (255.0 * cover) as u32),
                }
            }
        }
    }

    /// The ink of a pen, marker or eraser stroke. An eraser stroke is paper-coloured pen, so it replays as it was drawn.
    fn pen_ink(&self) -> Ink {
        let (kind, color) = match self.tool {
            Tool::Marker => (InkKind::Marker, COLORS[self.color]),
            Tool::Pencil => (InkKind::Pencil, COLORS[self.color]),
            Tool::Eraser => (InkKind::Pen, PAPER),
            _ => (InkKind::Pen, COLORS[self.color]),
        };
        Ink { kind, color, width: ink_width(self.tool) }
    }

    /// The ink of a rectangle or oval outline.
    fn shape_ink(&self) -> Ink {
        Ink { kind: InkKind::Pen, color: COLORS[self.color], width: ink_width(Tool::Pen) }
    }

    /// The stroke being drawn: the last one in the model.
    fn current_stroke(&mut self) -> Option<&mut Stroke> {
        let n = self.model.strokes().len();
        self.model.stroke_mut(n.checked_sub(1)?)
    }

    fn stroke_to(&mut self, ui: &Ui, x: f32, y: f32) {
        // the disc is stamped on the picture; the stroke keeps the board point under it
        let (cx, cy) = (x, y - self.canvas_top(ui) as f32);
        let (bx, by) = to_board(self.zoom, self.pan, (cx, cy));
        let r = self.radius(ui);
        match self.last {
            Some((lx, ly)) => {
                for (sx, sy) in ink::trail((lx, ly), (cx, cy), r) {
                    self.stamp(sx, sy, r);
                }
            }
            None => self.stamp(cx, cy, r),
        }
        if let Some(s) = self.current_stroke() {
            s.points.push(StrokePoint::at(bx, by));
        }
        self.last = Some((cx, cy));
        self.dirty = true;
    }

    /// Repaints the picture from the strokes, with an optional unsaved preview (a shape being dragged) on top.
    fn repaint(&mut self, ui: &Ui, preview: Option<&Drawing>) {
        self.sync_styles();
        self.prepare_pictures();
        let mut px = vec![PAPER; self.cw * self.ch];
        self.paint_items(&mut px, ui, self.edit);
        if self.view_is_plain() {
            style::paint_strokes(&mut px, self.cw, self.ch, self.model.strokes(), &self.styles, ui.s);
            if let Some(p) = preview {
                style::paint_strokes(&mut px, self.cw, self.ch, p.strokes(), &[], ui.s);
            }
        } else {
            // the strokes are moved and scaled by the view; their widths scale through the points-to-pixels factor
            let shown = self.view_strokes();
            style::paint_strokes(&mut px, self.cw, self.ch, shown.strokes(), &self.styles, ui.s * self.zoom);
            if let Some(p) = preview {
                let shown = self.view_drawing(p);
                style::paint_strokes(&mut px, self.cw, self.ch, shown.strokes(), &[], ui.s * self.zoom);
            }
        }
        self.canvas = px;
        self.dirty = true;
        self.repaint_due = false;
    }

    /// Removes the selected strokes (one undo step) and repaints.
    fn delete_selection(&mut self, ui: &Ui) {
        if self.pick.is_some() && self.delete_pick(ui) {
            return;
        }
        self.sync_styles();
        let Some(sel) = self.editable() else { return };
        self.selection = None;
        self.push_undo();
        let kept: Vec<Stroke> = self
            .model
            .strokes()
            .iter()
            .enumerate()
            .filter(|(i, _)| !sel.contains_stroke(*i))
            .map(|(_, s)| s.clone())
            .collect();
        let kept_looks: Vec<Style> = self
            .styles
            .iter()
            .enumerate()
            .filter(|(i, _)| !sel.contains_stroke(*i))
            .map(|(_, st)| *st)
            .collect();
        self.model = Drawing::from_strokes(kept);
        self.styles = kept_looks;
        self.repaint(ui, None);
    }

    /// The inspector's change to the selected strokes, as one undo step. A rotate or scale turns the selection about
    /// its centre; a width, fill, dash, shadow or opacity is set on each selected stroke.
    fn edit(&mut self, ui: &Ui, e: Edit) {
        self.sync_styles();
        let Some(sel) = self.editable() else { return };
        match e {
            Edit::Turn | Edit::Smaller | Edit::Larger => {
                let Some(b) = sel.bounds(&self.model) else { return };
                let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
                let t = match e {
                    Edit::Turn => Affine::rotate(15f32.to_radians()),
                    Edit::Smaller => Affine::scale(0.8, 0.8),
                    _ => Affine::scale(1.25, 1.25),
                };
                let about = Affine::translate(-cx, -cy).then(t).then(Affine::translate(cx, cy));
                self.push_undo();
                sel.transform(&mut self.model, about);
            }
            _ => {
                let widths = width_steps();
                let color = COLORS[self.color];
                let next = COLORS[(self.color + 1) % COLORS.len()];
                self.push_undo();
                for &i in sel.indices() {
                    if let Edit::Width(k) = e {
                        if let Some(s) = self.model.stroke_mut(i) {
                            s.ink.width = widths[k.min(widths.len() - 1)];
                        }
                    }
                    if let Some(st) = self.styles.get_mut(i) {
                        apply_style(st, e, color, next);
                    }
                }
            }
        }
        self.repaint(ui, None);
    }

    /// Whether canvas point (x, y) lies in the box of the current selection.
    fn selection_contains(&self, x: f32, y: f32) -> bool {
        self.selection
            .as_ref()
            .and_then(|s| s.bounds(&self.model))
            .is_some_and(|r| x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h)
    }

    /// Converts a window point (pixels of the pane) on the canvas to the board point under it.
    fn canvas_point(&self, ui: &Ui, x: f32, y: f32) -> (f32, f32) {
        to_board(self.zoom, self.pan, (x, y - self.canvas_top(ui) as f32))
    }

    fn gesture_active(&self) -> bool {
        self.lassoing || self.moving.is_some() || self.shape.is_some() || self.handle.is_some() || self.item_gesture() || self.panning.is_some()
    }

    /// A press on the canvas starts the stroke, selection drag, lasso loop or shape of the current tool.
    fn canvas_down(&mut self, ui: &Ui, x: f32, y: f32) {
        let (cx, cy) = self.canvas_point(ui, x, y);
        if let Some((editing, _)) = self.edit {
            // typing goes on only when the press is on the item being typed into, with a tool that can pick it
            let keep = matches!(self.tool, Tool::Lasso | Tool::Text | Tool::Sticky) && self.items.hit(cx, cy) == Some(editing);
            if !keep {
                self.finish_edit(ui);
            }
        }
        if !matches!(self.tool, Tool::Lasso | Tool::Text | Tool::Sticky | Tool::Connect) {
            self.pick = None;
        }
        match self.tool {
            Tool::Text | Tool::Sticky => self.text_tool_press(ui, cx, cy),
            Tool::Connect => self.connect_press(cx, cy),
            Tool::Hand => self.panning = Some((x, y)),
            Tool::Pen | Tool::Pencil | Tool::Marker | Tool::Eraser => {
                self.selection = None;
                self.push_undo();
                self.drawing = true;
                self.last = None;
                let pen = self.pen_ink();
                self.model.push(Stroke { ink: pen, points: Vec::new() });
                self.stroke_to(ui, x, y);
            }
            Tool::Lasso => {
                if let (Some(h), Some(b)) = (self.handle_at(ui, cx, cy), self.handle_box()) {
                    let pivot = arrange::anchor_of(b, h).unwrap_or((b.x + b.w / 2.0, b.y + b.h / 2.0));
                    self.push_undo();
                    self.handle = Some(HandleDrag { handle: h, last: (cx, cy), pivot });
                } else if self.selection_contains(cx, cy) && self.editable().is_some() {
                    self.pick = None;
                    self.push_undo();
                    self.moving = Some((cx, cy));
                } else if self.item_press(ui, cx, cy) {
                    // an item or a connector was picked
                } else {
                    self.selection = None;
                    self.lasso = Lasso::new();
                    self.lasso.add(cx, cy);
                    self.lassoing = true;
                }
            }
            Tool::Rectangle | Tool::Oval | Tool::Line | Tool::Arrow | Tool::Triangle | Tool::Star => {
                self.selection = None;
                self.push_undo();
                self.shape = Some(self.snapped(ui, (cx, cy)));
            }
        }
        self.dirty = true;
    }

    /// A drag on the canvas extends the lasso loop, moves the selection or stretches the shape.
    fn canvas_move(&mut self, ui: &Ui, x: f32, y: f32) {
        let (cx, cy) = self.canvas_point(ui, x, y);
        if self.lassoing {
            self.lasso.add(cx, cy);
        } else if self.item_gesture() {
            self.item_move(ui, cx, cy);
        } else if let Some((lx, ly)) = self.panning {
            self.pan = (self.pan.0 + x - lx, self.pan.1 + y - ly);
            self.panning = Some((x, y));
            self.repaint(ui, None);
        } else if let Some((mx, my)) = self.moving {
            if let Some(sel) = self.editable() {
                sel.transform(&mut self.model, ink::nudge(cx - mx, cy - my));
            }
            self.moving = Some((cx, cy));
            self.repaint(ui, None);
        } else if let Some(drag) = self.handle {
            if let Some(sel) = self.editable() {
                let step = match drag.handle {
                    Handle::Rotate => arrange::rotate_step(drag.pivot, drag.last, (cx, cy)),
                    _ => arrange::resize_step(drag.pivot, drag.last, (cx, cy)),
                };
                sel.transform(&mut self.model, step);
            }
            self.handle = Some(HandleDrag { last: (cx, cy), ..drag });
            self.repaint(ui, None);
        } else if let (Some(a), Some(kind)) = (self.shape, self.tool.shape_kind()) {
            let preview = ink::shape_of_kind(a, self.snapped(ui, (cx, cy)), kind, self.shape_ink());
            self.repaint(ui, Some(&preview));
        }
        self.dirty = true;
    }

    /// A release ends the lasso (selecting the strokes it reached), the drag or the shape. A shape released where it
    /// started is a tap, so nothing is drawn and the snapshot taken on press is dropped.
    fn canvas_up(&mut self, ui: &Ui, x: f32, y: f32) {
        let (cx, cy) = self.canvas_point(ui, x, y);
        if self.lassoing {
            self.lassoing = false;
            self.sync_styles();
            let sel = self.lasso.select(&self.model);
            self.selection = Some(self.expand_groups(sel)).filter(|s| !s.is_empty());
            self.lasso = Lasso::new();
        } else if self.item_gesture() {
            self.item_release(ui, cx, cy);
        } else if self.panning.is_some() {
            self.panning = None;
        } else if self.moving.is_some() {
            self.moving = None;
        } else if self.handle.is_some() {
            self.handle = None;
        } else if let Some(a) = self.shape.take() {
            let end = self.snapped(ui, (cx, cy));
            let kind = self.tool.shape_kind().unwrap_or(ShapeKind::Rectangle);
            if (end.0 - a.0).abs() < 2.0 && (end.1 - a.1).abs() < 2.0 {
                if let Some((c, d, looks, items)) = self.undo.pop() {
                    self.canvas = c;
                    self.model = d;
                    self.styles = looks;
                    self.items = items;
                }
            } else {
                let shape = ink::shape_of_kind(a, end, kind, self.shape_ink());
                self.sync_styles();
                let base = self.model.strokes().len();
                for s in shape.strokes() {
                    self.model.push(s.clone());
                }
                self.sync_styles();
                // a closed shape can be filled by the inspector; a line or arrow cannot
                for st in self.styles.iter_mut().skip(base) {
                    st.shape = kind.closed();
                }
                self.repaint(ui, None);
            }
        }
        self.dirty = true;
    }

    /// The control under pixel (x, y); a colour well inside the colour slot resolves to its colour.
    fn hit(&self, ui: &Ui, x: f32, y: f32) -> Option<Target> {
        let (px, py) = (x / ui.s, y / ui.s);
        if let Some(t) = self.inspector_hit(ui, px, py) {
            return Some(t);
        }
        let id = self.placed.as_ref()?.hit(px, py)?;
        if id == BAR_COLORS {
            let r = self.placed.as_ref()?.rect_of(BAR_COLORS)?;
            let i = ((px - r.x - SPACE_S) / WELL_PT).floor();
            if i < 0.0 || i >= COLORS.len() as f32 {
                return None;
            }
            return Some(Target::Color(i as usize));
        }
        id_target(id)
    }

    /// Activates toolbar control `id` (from the shell's control list).
    pub fn activate_id(&mut self, ui: &Ui, id: u64) -> bool {
        let Some(t) = id_target(id as u32) else { return false };
        self.activate(ui, t);
        true
    }

    /// The File menu's Export as PNG: the whole board, painted through a bitmap context at the display's factor, is
    /// written to the Photos folder. The toast names the file (or the failure).
    pub fn export_as_png(&mut self, ui: &Ui) {
        let msg = match self.save(ui) {
            Ok(name) => format!("Exported {name} as PNG"),
            Err(e) => format!("Could not export: {e}"),
        };
        self.toast = Some((msg, self.now + 2500.0));
        self.dirty = true;
    }

    fn activate(&mut self, ui: &Ui, t: Target) {
        match t {
            Target::Tool(tool) => {
                self.finish_edit(ui);
                self.tool = tool;
                if tool != Tool::Lasso {
                    self.selection = None;
                    self.pick = None;
                }
            }
            Target::Color(i) => {
                self.color = i;
                if self.tool == Tool::Eraser {
                    self.tool = Tool::Pen;
                }
                self.recolour_pick(ui, i);
            }
            Target::Undo => self.undo_step(),
            Target::Clear => {
                self.finish_edit(ui);
                self.push_undo();
                self.canvas.iter_mut().for_each(|p| *p = PAPER);
                self.model = Drawing::new();
                self.styles.clear();
                self.items = Items::default();
                self.end_item_work();
                self.selection = None;
            }
            Target::Inspector => self.inspector = !self.inspector,
            Target::Edit(e) => self.edit(ui, e),
            Target::Delete => self.delete_selection(ui),
            Target::Copy => self.copy_selection(),
            Target::Arrange => self.arrange_selection(ui),
            Target::Save => {
                let msg = match self.save(ui) {
                    Ok(name) => format!("Saved {name} to Photos"),
                    Err(e) => format!("Could not save: {e}"),
                };
                self.toast = Some((msg, self.now + 2500.0));
            }
        }
        if !matches!(t, Target::Inspector | Target::Copy | Target::Save | Target::Tool(_) | Target::Color(_)) {
            self.mark_changed();
        }
        self.dirty = true;
    }

    /// Arrange: the selected strokes are lined up in a row, wrapping at the board's right edge, starting at the
    /// selection's top-left corner. Each stroke keeps its shape; only its position changes. One undo step.
    fn arrange_selection(&mut self, ui: &Ui) {
        let Some(sel) = self.selection.clone() else { return };
        let Some(origin) = sel.bounds(&self.model) else { return };
        let parts: Vec<(usize, Rect)> = sel.indices().iter().filter_map(|&i| Some((i, self.model.strokes().get(i)?.bounds()?))).collect();
        if parts.is_empty() {
            return;
        }
        let sizes: Vec<(f32, f32)> = parts.iter().map(|(_, b)| (b.w, b.h)).collect();
        let room = (self.board_pt((self.cw as f32, 0.0)).0 - origin.x).max(0.0);
        let gap = ui.pt(SPACE_M);
        let placed = FlowLayout::new(gap, gap).place_subviews(origin, (room, f32::INFINITY), &sizes);
        self.push_undo();
        for ((i, b), r) in parts.iter().zip(placed) {
            let (dx, dy) = (r.x - b.x, r.y - b.y);
            if dx != 0.0 || dy != 0.0 {
                if let Some(s) = self.model.stroke_mut(*i) {
                    s.transform(Affine::translate(dx, dy));
                }
            }
        }
        self.repaint(ui, None);
    }

    fn save(&self, ui: &Ui) -> Result<String, String> {
        let dir = self.save_dir.as_ref().ok_or("no Photos folder")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let name = free_name(dir, millis);
        let png = self.export_png(ui);
        springboard::shell::fsutil::write_atomic(&dir.join(&name), png).map_err(|e| e.to_string())?;
        Ok(name)
    }

    /// The picture as a PNG. On a display denser than the canvas the strokes are painted again at the export factor,
    /// so the file is sharp; otherwise the canvas itself is written.
    fn export_png(&self, ui: &Ui) -> Vec<u8> {
        let k = self.export_scale.max(1) as usize;
        let mut big = self.view_strokes();
        big.transform(Affine::scale(k as f32, k as f32));
        // the picture is painted into a bitmap context of the export size, with each stroke's look
        let mut bmp = Bitmap::new(self.cw * k, self.ch * k, PAPER);
        if !self.items.is_empty() {
            let layer = self.items_layer(ui);
            bmp.px = if k == 1 { layer } else { Image { w: self.cw, h: self.ch, px: layer }.resized(self.cw * k, self.ch * k).px };
        }
        let (w, h) = (bmp.w, bmp.h);
        style::paint_strokes(&mut bmp.px, w, h, big.strokes(), &self.styles, ui.s * self.zoom * k as f32);
        bmp.png()
    }

    /// Copy: the selected strokes go to `copied` for the app to put on the pasteboard.
    fn copy_selection(&mut self) {
        let Some(sel) = &self.selection else { return };
        let strokes: Vec<Stroke> = self
            .model
            .strokes()
            .iter()
            .enumerate()
            .filter(|(i, _)| sel.contains_stroke(*i))
            .map(|(_, s)| s.clone())
            .collect();
        if strokes.is_empty() {
            return;
        }
        self.copied = Some(Drawing::from_strokes(strokes));
        self.toast = Some(("Copied the selection".to_string(), self.now + 2500.0));
    }

    pub fn touch(&mut self, ui: &Ui, event: Event, t: f64) {
        self.now = t;
        if let Event::Down(x, y) | Event::Move(x, y) = event {
            self.pointer = Some((x, y));
        }
        self.ensure_canvas(ui);
        if self.search.is_some() {
            // the board search takes every press until it closes
            if let Event::Down(x, y) = event {
                self.search_press(ui, x, y);
            }
            self.dirty = true;
            return;
        }
        let top = self.canvas_top(ui) as f32;
        match event {
            Event::Down(x, y) => {
                self.dismiss_tip();
                if y >= top && x < self.board_px(ui) as f32 {
                    self.canvas_down(ui, x, y);
                } else {
                    self.pressed = self.hit(ui, x, y);
                }
                self.dirty = true;
            }
            Event::Move(x, y) => {
                if self.drawing {
                    self.stroke_to(ui, x, y.max(top));
                } else if self.gesture_active() {
                    self.canvas_move(ui, x, y);
                } else if self.pressed.is_some() && self.hit(ui, x, y) != self.pressed {
                    self.pressed = None;
                    self.dirty = true;
                }
            }
            Event::Up(x, y) => {
                if self.drawing || self.gesture_active() {
                    self.mark_changed();
                }
                if self.drawing {
                    self.drawing = false;
                    self.last = None;
                } else if self.gesture_active() {
                    self.canvas_up(ui, x, y);
                } else if let Some(p) = self.pressed.take() {
                    if self.hit(ui, x, y) == Some(p) {
                        self.activate(ui, p);
                    }
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, _ui: &Ui, t: f64) -> bool {
        self.now = t;
        if self.unsaved && !self.drawing && !self.gesture_active() && self.save_at.is_some_and(|at| t >= at) {
            self.flush();
        }
        if matches!(&self.toast, Some((_, until)) if t >= *until) {
            self.toast = None;
            self.dirty = true;
        }
        self.dirty
    }

    pub fn next_wake(&self, _t: f64) -> Option<f64> {
        let save = if self.unsaved { self.save_at } else { None };
        match (self.toast.as_ref().map(|(_, until)| *until), save) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// The state line the tests compare (the SDK `App` trait has no describe hook).
    #[cfg(test)]
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

    /// The dot grid over the board (not part of an export): one dot per grid point.
    fn draw_grid(&self, s: &mut Surface, ui: &Ui, top: i32) {
        let step = (ui.pt(GRID_PT) * self.zoom).max(4.0);
        let dot = ui.pt(GRID_DOT_PT).max(1.0);
        let colour = SEPARATOR_C.now();
        // the dots sit on multiples of the grid step in board coordinates, so they move with the view
        let first = |shift: f32| {
            let at = shift.rem_euclid(step);
            if at < 1.0 {
                at + step
            } else {
                at
            }
        };
        let mut y = first(self.pan.1);
        while y < self.ch as f32 {
            let mut x = first(self.pan.0);
            while x < self.cw as f32 {
                s.fill_rect(x as usize, (top as f32 + y) as usize, dot as usize, dot as usize, colour);
                x += step;
            }
            y += step;
        }
    }

    /// Square handles on the corners of the selection and a rotate handle above it.
    fn draw_handles(&self, s: &mut Surface, ui: &Ui, top: f32, b: Rect) {
        let side = ui.pt(HANDLE_PT);
        let gap = ui.pt(ROTATE_GAP_PT);
        let line = ui.pt(1.0).max(1.0);
        // the stem from the top edge to the rotate handle
        s.fill_rrect(b.x + b.w / 2.0 - line / 2.0, b.y + top - gap, line, gap, 0.0, TINT);
        for (_, (x, y)) in arrange::handle_points(b, gap) {
            let (x0, y0) = (x - side / 2.0, y + top - side / 2.0);
            s.fill_rrect(x0, y0, side, side, 0.0, TINT);
            s.fill_rrect(x0 + line, y0 + line, side - 2.0 * line, side - 2.0 * line, 0.0, WHITE);
        }
    }

    /// A thin tint frame around the selection's box (canvas pixels, drawn below the toolbar).
    fn outline(&self, s: &mut Surface, ui: &Ui, top: f32, b: Rect) {
        let t = ui.pt(1.0).max(1.0);
        let (x0, y0) = (b.x, b.y + top);
        s.fill_rrect(x0, y0, b.w, t, 0.0, TINT);
        s.fill_rrect(x0, y0 + b.h, b.w, t, 0.0, TINT);
        s.fill_rrect(x0, y0, t, b.h, 0.0, TINT);
        s.fill_rrect(x0 + b.w, y0, t, b.h, 0.0, TINT);
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui, _t: f64) {
        self.dirty = false;
        self.scale = ui.s;
        self.ensure_canvas(ui);
        if self.repaint_due {
            self.repaint(ui, None);
            self.dirty = false;
        }
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
        if self.grid {
            self.draw_grid(s, ui, top);
        }
        if let Some(b) = self.selection.as_ref().and_then(|sel| sel.bounds(&self.model)) {
            self.outline(s, ui, top as f32, self.view_rect(b));
        }
        self.draw_pick(s, ui, top as f32);
        if let Some(b) = self.handle_box() {
            self.draw_handles(s, ui, top as f32, self.view_rect(b));
        }
        if self.lassoing {
            let d = ui.pt(bifrost_app::kit::ui::SPACE_XXS);
            for &p in self.lasso.points().iter().step_by(2) {
                let (x, y) = self.view_pt(p);
                s.fill_rrect(x, y + top as f32, d, d, d / 2.0, TINT);
            }
        }
        if self.inspector {
            self.draw_inspector(s, ui);
        }
        widgets::toolbar_band(s, ui, Rect { x: 0.0, y: 0.0, w: self.w as f32 / ui.s, h: UNIFIED_BAR_PT });
        let pressed = self.pressed.and_then(target_id);
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
        self.draw_search(s, ui);
        let now = now_seconds();
        let sections = self.tip_sections(now);
        if !sections.is_empty() && !self.tip_marked {
            self.tip_marked = true;
            if let Some(store) = &mut self.tips {
                let _ = store.mark_shown(LassoHint.id(), now);
            }
        }
        let lines: Vec<(String, Weight)> = sections
            .iter()
            .filter_map(|sec| match sec {
                TipSection::Title(t) => Some((t.clone(), Weight::Semibold)),
                TipSection::Message(m) => Some((m.clone(), Weight::Regular)),
                TipSection::Actions(_) => None,
            })
            .collect();
        if !lines.is_empty() {
            let mut sizes = Vec::with_capacity(lines.len());
            for (text, weight) in &lines {
                let img = self.cache.get(ui, text, MAC_CALLOUT_PT, *weight, WHITE);
                sizes.push((img.w as f32, img.h as f32));
            }
            let max_w = sizes.iter().fold(0.0f32, |a, s| a.max(s.0));
            let text_h: f32 = sizes.iter().map(|s| s.1).sum::<f32>() + ui.pt(SPACE_XS) * (lines.len() - 1) as f32;
            let pw = max_w + ui.pt(SPACE_XXXL);
            let ph = text_h + ui.pt(SPACE_L);
            let x = ui.pt(SPACE_L);
            let y = self.h as f32 - ph - ui.pt(SPACE_XXL);
            s.fill_rrect(x, y, pw, ph, 1.0e4, TOAST_FILL);
            let mut cy = y + ui.pt(SPACE_S);
            for (text, weight) in &lines {
                let img = self.cache.get(ui, text, MAC_CALLOUT_PT, *weight, WHITE);
                let h = img.h as f32;
                s.blit(img, (x + ui.pt(SPACE_L)) as i32, cy as i32, 255, 255);
                cy += h + ui.pt(SPACE_XS);
            }
        }
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

/// The first-use hint for the lasso and Arrange.
struct LassoHint;

impl Tip for LassoHint {
    fn id(&self) -> &str {
        "is.olibuijr.freeform.tip.lasso"
    }

    fn title(&self) -> &str {
        "Select, then arrange"
    }

    fn message(&self) -> &str {
        "Choose Lasso and drag a loop around strokes. Arrange lines them up in a row."
    }
}

/// Seconds since the Unix epoch, for the tip's display gap.
fn now_seconds() -> tips::Seconds {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Rough brightness for the marker's "keep the stronger mix" rule.
fn lum(c: u32) -> u32 {
    (c & 255) + ((c >> 8) & 255) + ((c >> 16) & 255)
}

/// `Freeform-<ms>.png`, numbered `-2`, `-3`... when that name is taken, so a
/// second save never overwrites an earlier one.
fn free_name(dir: &std::path::Path, millis: u128) -> String {
    free_name_ext(dir, millis, "png")
}

/// `free_name` for any file extension.
fn free_name_ext(dir: &std::path::Path, millis: u128, ext: &str) -> String {
    let mut name = format!("Freeform-{millis}.{ext}");
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("Freeform-{millis}-{n}.{ext}");
        n += 1;
    }
    name
}

/// What a failed print or PDF job says.
fn print_message(e: PrintError) -> String {
    match e {
        PrintError::NoCups => "The printing service is not running".to_string(),
        PrintError::NoPrinter => "No printer is set up".to_string(),
        PrintError::Failed(why) => format!("Printing failed: {why}"),
    }
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

    /// A board of 200 x 200 points with its canvas allocated.
    fn board(ui: &Ui) -> Freeform {
        let mut app = Freeform::new(ui, 200, 200);
        app.ensure_canvas(ui);
        app
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
        assert!(app.model.is_empty());
    }

    #[test]
    fn a_pen_drag_is_one_stroke_and_one_undo_step() {
        let ui = test_ui();
        let mut app = board(&ui);
        let top = app.canvas_top(&ui) as f32;
        app.touch(&ui, Event::Down(10.0, top + 10.0), 0.0);
        app.touch(&ui, Event::Move(60.0, top + 10.0), 0.0);
        app.touch(&ui, Event::Up(60.0, top + 10.0), 0.0);
        assert_eq!(app.model.strokes().len(), 1);
        assert_eq!(app.model.strokes()[0].ink.kind, InkKind::Pen);
        assert!(app.model.strokes()[0].points.len() >= 2);
        assert_eq!(app.undo.len(), 1);
    }

    #[test]
    fn a_lasso_selects_the_strokes_it_reaches_and_delete_removes_them() {
        let ui = test_ui();
        let mut app = board(&ui);
        let line = |x: f32, y: f32| Stroke { ink: Ink { kind: InkKind::Pen, color: BLACK, width: 4.0 }, points: vec![StrokePoint::at(x, y)] };
        app.model = Drawing::from_strokes(vec![line(5.0, 5.0), line(120.0, 120.0)]);
        app.tool = Tool::Lasso;
        app.lassoing = true;
        for (x, y) in [(0.0, 0.0), (30.0, 0.0), (30.0, 30.0), (0.0, 30.0)] {
            app.lasso.add(x, y);
        }
        app.canvas_up(&ui, 0.0, app.canvas_top(&ui) as f32);
        assert_eq!(app.selection.as_ref().unwrap().indices(), &[0usize]);

        app.delete_selection(&ui);
        assert_eq!(app.model.strokes().len(), 1);
        assert_eq!(app.model.strokes()[0].points[0].x, 120.0);
        assert!(app.selection.is_none());
        assert_eq!(app.undo.len(), 1, "delete is undoable");

        app.activate(&ui, Target::Undo);
        assert_eq!(app.model.strokes().len(), 2, "undo brings the stroke back");
    }

    #[test]
    fn dragging_a_selection_moves_its_strokes() {
        let ui = test_ui();
        let mut app = board(&ui);
        let top = app.canvas_top(&ui) as f32;
        app.model = Drawing::from_strokes(vec![Stroke {
            ink: Ink { kind: InkKind::Pen, color: BLACK, width: 4.0 },
            points: vec![StrokePoint::at(5.0, 5.0), StrokePoint::at(105.0, 45.0)],
        }]);
        // the selection is dragged with the lasso tool active, as a user does
        app.tool = Tool::Lasso;
        app.selection = Some(Selection::default());
        app.lassoing = true;
        for (x, y) in [(0.0, 0.0), (110.0, 0.0), (110.0, 50.0), (0.0, 50.0)] {
            app.lasso.add(x, y);
        }
        app.canvas_up(&ui, 0.0, top);
        assert!(app.selection_contains(5.0, 5.0));

        app.canvas_down(&ui, 55.0, top + 25.0);
        app.canvas_move(&ui, 65.0, top + 25.0);
        app.canvas_up(&ui, 65.0, top + 25.0);
        assert_eq!(app.model.strokes()[0].points[0].x, 15.0);
    }

    #[test]
    fn a_rectangle_drag_adds_one_closed_stroke_and_a_tap_adds_none() {
        let ui = test_ui();
        let mut app = board(&ui);
        let top = app.canvas_top(&ui) as f32;
        app.tool = Tool::Rectangle;
        app.canvas_down(&ui, 10.0, top + 10.0);
        app.canvas_move(&ui, 60.0, top + 40.0);
        app.canvas_up(&ui, 60.0, top + 40.0);
        assert_eq!(app.model.strokes().len(), 1);
        let b = app.model.bounds().unwrap();
        assert_eq!((b.x, b.y, b.w, b.h), (10.0, 10.0, 50.0, 30.0));

        let undo_before = app.undo.len();
        app.canvas_down(&ui, 100.0, top + 100.0);
        app.canvas_up(&ui, 100.0, top + 100.0);
        assert_eq!(app.model.strokes().len(), 1, "a tap draws nothing");
        assert_eq!(app.undo.len(), undo_before, "the tap's snapshot is dropped");
    }

    #[test]
    fn an_oval_drag_draws_an_oval_stroke() {
        let ui = test_ui();
        let mut app = board(&ui);
        let top = app.canvas_top(&ui) as f32;
        app.tool = Tool::Oval;
        app.canvas_down(&ui, 0.0, top);
        app.canvas_move(&ui, 40.0, top + 20.0);
        app.canvas_up(&ui, 40.0, top + 20.0);
        assert_eq!(app.model.strokes().len(), 1);
        assert!(app.model.strokes()[0].points.len() > 8, "an oval is flattened to many points");
    }

    fn two_lines() -> Drawing {
        let line = |x: f32| Stroke { ink: Ink { kind: InkKind::Pen, color: BLACK, width: 4.0 }, points: vec![StrokePoint::at(x, 5.0)] };
        Drawing::from_strokes(vec![line(5.0), line(120.0)])
    }

    /// Selects the strokes a lasso drawn 10 points around (x, y) reaches, as a drag on the board would.
    fn select_at(app: &mut Freeform, x: f32, y: f32) {
        let mut lasso = Lasso::new();
        for (px, py) in [(x - 10.0, y - 10.0), (x + 10.0, y - 10.0), (x + 10.0, y + 10.0), (x - 10.0, y + 10.0)] {
            lasso.add(px, py);
        }
        app.selection = Some(lasso.select(&app.model)).filter(|s| !s.is_empty());
    }

    #[test]
    fn copy_takes_only_the_selected_strokes_once() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.model = two_lines();
        assert!(app.take_copy().is_none());
        select_at(&mut app, 120.0, 5.0);
        app.activate(&ui, Target::Copy);
        let copied = app.take_copy().expect("the selected stroke is copied");
        assert_eq!(copied.strokes().len(), 1);
        assert_eq!(copied.strokes()[0].points[0].x, 120.0);
        assert!(app.take_copy().is_none(), "a copy is taken once");
        assert_eq!(app.model.strokes().len(), 2, "copy leaves the board alone");
    }

    #[test]
    fn copy_needs_a_selection_and_the_button_follows_it() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.model = two_lines();
        app.activate(&ui, Target::Copy);
        assert!(app.take_copy().is_none());
        assert!(app.bar().items.iter().any(|i| i.id == BAR_COPY && !i.enabled));
        select_at(&mut app, 5.0, 5.0);
        assert!(app.bar().items.iter().any(|i| i.id == BAR_COPY && i.enabled));
    }

    #[test]
    fn the_export_is_scaled_by_the_display_factor() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.model = two_lines();
        let (w, h) = (app.cw, app.ch);
        let dims = |png: &[u8]| (u32::from_be_bytes([png[16], png[17], png[18], png[19]]) as usize, u32::from_be_bytes([png[20], png[21], png[22], png[23]]) as usize);
        assert_eq!(dims(&app.export_png(&ui)), (w, h));
        app.set_export_scale(2);
        assert_eq!(dims(&app.export_png(&ui)), (w * 2, h * 2));
        app.set_export_scale(0);
        assert_eq!(dims(&app.export_png(&ui)), (w, h), "a zero factor is treated as 1");
    }

    #[test]
    fn arrange_lines_the_selected_strokes_up_in_a_row_from_their_corner() {
        let ui = test_ui();
        let mut app = board(&ui);
        let seg = |a: (f32, f32), b: (f32, f32)| Stroke {
            ink: Ink { kind: InkKind::Pen, color: BLACK, width: 4.0 },
            points: vec![StrokePoint::at(a.0, a.1), StrokePoint::at(b.0, b.1)],
        };
        // bounds: stroke 0 is (10,10) 20 x 10; stroke 1 is (100,50) 40 x 20; the union is (10,10) 130 x 60
        app.model = Drawing::from_strokes(vec![seg((10.0, 10.0), (30.0, 20.0)), seg((100.0, 50.0), (140.0, 70.0))]);
        let mut lasso = Lasso::new();
        for (x, y) in [(0.0, 0.0), (200.0, 0.0), (200.0, 200.0), (0.0, 200.0)] {
            lasso.add(x, y);
        }
        app.selection = Some(lasso.select(&app.model)).filter(|s| !s.is_empty());
        assert_eq!(app.selection.as_ref().unwrap().indices(), &[0usize, 1]);
        assert!(app.bar().items.iter().any(|i| i.id == BAR_ARRANGE && i.enabled));
        let undo_before = app.undo.len();
        app.activate(&ui, Target::Arrange);
        // room = 200 - 10 = 190; the gap is 12 pt at scale 1.5625 = 18.75 px. Stroke 0 (20 wide) sits at x 0, stroke 1
        // (40 wide) at 20 + 18.75 = 38.75, which fits on the row. Placed at (10,10) and (48.75,10).
        assert_eq!(app.model.strokes()[0].points[0], StrokePoint::at(10.0, 10.0), "the first stroke is already in place");
        assert_eq!(app.model.strokes()[1].points[0], StrokePoint::at(48.75, 10.0));
        assert_eq!(app.model.strokes()[1].points[1], StrokePoint::at(88.75, 30.0));
        assert_eq!(app.undo.len(), undo_before + 1, "arrange is one undo step");
        assert_eq!(app.selection.as_ref().unwrap().indices(), &[0usize, 1], "the selection is kept");
    }

    #[test]
    fn the_first_use_hint_shows_until_the_board_is_pressed() {
        let ui = test_ui();
        let store = tips::configure(&[]).unwrap();
        let mut app = board(&ui).with_tips(store);
        assert_eq!(
            app.tip_sections(0),
            vec![TipSection::Title("Select, then arrange".to_string()), TipSection::Message("Choose Lasso and drag a loop around strokes. Arrange lines them up in a row.".to_string())]
        );
        let top = app.canvas_top(&ui) as f32;
        app.touch(&ui, Event::Down(10.0, top + 10.0), 0.0);
        assert!(app.tip_sections(0).is_empty(), "a press on the board dismisses the hint");
        assert!(board(&ui).tip_sections(0).is_empty(), "no store, no hint");
    }

    #[test]
    fn the_pointer_is_an_arrow_on_the_toolbar_and_the_tool_cursor_on_the_board() {
        let ui = test_ui();
        let mut app = board(&ui);
        let top = app.canvas_top(&ui) as f32;
        assert_eq!(app.pointer_for(&ui, 10.0, 0.0), Kind::Arrow);
        assert_eq!(app.pointer_for(&ui, 10.0, top + 10.0), Kind::Crosshair);
        app.tool = Tool::Eraser;
        assert_eq!(app.pointer_for(&ui, 10.0, top + 10.0), Kind::DisappearingItem);
        app.tool = Tool::Lasso;
        assert_eq!(app.pointer_for(&ui, 10.0, top + 10.0), Kind::Arrow);
        app.model = Drawing::from_strokes(vec![Stroke {
            ink: Ink { kind: InkKind::Pen, color: BLACK, width: 4.0 },
            points: vec![StrokePoint::at(5.0, 5.0), StrokePoint::at(105.0, 45.0)],
        }]);
        app.selection = Some(Selection::from_indices(vec![0]));
        assert_eq!(app.pointer_for(&ui, 55.0, top + 25.0), Kind::OpenHand, "over the selected stroke, away from its handles");
        app.moving = Some((0.0, 0.0));
        assert_eq!(app.pointer_for(&ui, 55.0, top + 25.0), Kind::ClosedHand, "while it is dragged");
    }

    #[test]
    fn the_pencil_is_a_thin_pencil_ink_in_the_pen_group() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.tool = Tool::Pencil;
        let ink = app.pen_ink();
        assert_eq!(ink.kind, InkKind::Pencil, "the pencil draws with the pencil ink of the drawing model");
        assert_eq!(ink.width, SPACE_XS, "half the pen's width (the pen is 2 x SPACE_XS)");
        assert_eq!(id_target(BAR_PENCIL), Some(Target::Tool(Tool::Pencil)));
        assert_eq!(target_id(Target::Tool(Tool::Pencil)), Some(BAR_PENCIL));
        assert_eq!(app.pointer_for(&ui, 10.0, app.canvas_top(&ui) as f32 + 10.0), Kind::Crosshair);
    }

    /// A whole-board lasso, so every stroke is selected.
    fn select_all(app: &mut Freeform) {
        let mut lasso = Lasso::new();
        for (x, y) in [(0.0, 0.0), (400.0, 0.0), (400.0, 400.0), (0.0, 400.0)] {
            lasso.add(x, y);
        }
        app.selection = Some(lasso.select(&app.model)).filter(|s| !s.is_empty());
    }

    #[test]
    fn the_inspector_width_and_fill_set_the_selected_strokes_look() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.model = Drawing::from_strokes(vec![Stroke {
            ink: Ink { kind: InkKind::Pen, color: BLACK, width: 4.0 },
            points: vec![StrokePoint::at(5.0, 5.0), StrokePoint::at(12.0, 5.0)],
        }]);
        select_all(&mut app);
        let undo_before = app.undo.len();
        app.edit(&ui, Edit::Width(2));
        assert_eq!(app.model.strokes()[0].ink.width, ink_width(Tool::Pen) * 2.0, "thick is twice the pen's width");
        app.edit(&ui, Edit::Fill(1));
        assert_eq!(app.styles[0].fill, Fill::Solid(COLORS[0]));
        app.edit(&ui, Edit::Dash(1));
        assert!(app.styles[0].dashed);
        assert_eq!(app.undo.len(), undo_before + 3, "each inspector change is one undo step");
        app.activate(&ui, Target::Undo);
        assert!(!app.styles[0].dashed, "undo puts the look back");
    }

    #[test]
    fn larger_scales_the_selection_about_its_centre() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.model = Drawing::from_strokes(vec![Stroke {
            ink: Ink { kind: InkKind::Pen, color: BLACK, width: 4.0 },
            points: vec![StrokePoint::at(10.0, 10.0), StrokePoint::at(30.0, 10.0)],
        }]);
        select_all(&mut app);
        // the centre is (20, 10); 1.25 times the distance from it: 10 becomes 7.5 and 30 becomes 32.5
        app.edit(&ui, Edit::Larger);
        let pts = &app.model.strokes()[0].points;
        assert_eq!((pts[0].x, pts[0].y), (7.5, 10.0));
        assert_eq!((pts[1].x, pts[1].y), (32.5, 10.0));
    }

    #[test]
    fn the_inspector_panel_narrows_the_board_and_its_rows_hit_their_segments() {
        let ui = test_ui();
        let mut app = Freeform::new(&ui, 800, 600);
        app.ensure_canvas(&ui);
        assert_eq!(app.cw, 800);
        app.activate(&ui, Target::Inspector);
        app.ensure_canvas(&ui);
        // the panel is 220 points, 343.75 pixels at 1.5625, which truncates to 343
        assert_eq!(app.cw, 800 - 343);
        app.model = two_lines();
        select_at(&mut app, 5.0, 5.0);
        // the middle segment of the Fill row (Solid) is hit at its centre
        let r = app.row_control(&ui, 0);
        assert_eq!(app.inspector_hit(&ui, r.x + r.w / 2.0, r.y + r.h / 2.0), Some(Target::Edit(Edit::Fill(1))));
        app.selection = None;
        assert_eq!(app.inspector_hit(&ui, r.x + r.w / 2.0, r.y + r.h / 2.0), None, "no selection, no control");
    }

    #[test]
    fn the_export_is_a_png_of_the_board_size_at_the_display_factor() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.model = two_lines();
        app.set_export_scale(2);
        assert!(app.has_strokes());
        let png = app.export_png(&ui);
        assert_eq!(&png[1..4], b"PNG");
        assert_eq!(u32::from_be_bytes([png[16], png[17], png[18], png[19]]) as usize, app.cw * 2);
        assert_eq!(u32::from_be_bytes([png[20], png[21], png[22], png[23]]) as usize, app.ch * 2);
    }

    #[test]
    fn redo_makes_an_undone_step_again_and_a_new_edit_forgets_it() {
        let ui = test_ui();
        let mut app = board(&ui);
        let top = app.canvas_top(&ui) as f32;
        app.tool = Tool::Rectangle;
        app.canvas_down(&ui, 10.0, top + 10.0);
        app.canvas_up(&ui, 60.0, top + 40.0);
        assert_eq!(app.model.strokes().len(), 1);
        assert!(!app.can_redo());
        app.push_undo();
        app.model = Drawing::new();
        app.undo_step();
        assert!(app.can_redo(), "undo keeps the step it left");
        app.redo_step();
        assert!(app.model.is_empty(), "redo returns to the state undo left");
        assert!(!app.can_redo());
        app.undo_step();
        app.push_undo();
        assert!(!app.can_redo(), "a new edit clears redo");
    }

    #[test]
    fn duplicate_adds_offset_copies_with_their_looks_and_selects_them() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.model = two_lines();
        app.styles = vec![Style { dashed: true, ..Style::default() }, Style::default()];
        app.selection = Some(Selection::from_indices(vec![0]));
        app.duplicate_selection(&ui);
        assert_eq!(app.model.strokes().len(), 3);
        assert_eq!(app.model.strokes()[2].points[0].x, 5.0 + ui.pt(PASTE_OFFSET_PT));
        assert!(app.styles[2].dashed, "the copy keeps the original's look");
        assert_eq!(app.selection.as_ref().unwrap().indices(), &[2]);
        app.undo_step();
        assert_eq!(app.model.strokes().len(), 2, "one undo step");
    }

    #[test]
    fn paste_puts_the_strokes_on_the_board_selected() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.paste_strokes(&ui, &two_lines());
        assert_eq!(app.model.strokes().len(), 2);
        assert_eq!(app.selection.as_ref().unwrap().indices(), &[0, 1]);
        assert_eq!(app.tool, Tool::Lasso);
    }

    #[test]
    fn paste_centres_the_strokes_on_the_pointer_on_the_board() {
        let ui = test_ui();
        let mut app = board(&ui);
        let top = app.canvas_top(&ui) as f32;
        app.pointer = Some((300.0, top + 200.0));
        app.paste_strokes(&ui, &two_lines());
        let boxes: Vec<Rect> = app.model.strokes().iter().filter_map(|s| s.bounds()).collect();
        let all = arrange::union_of(&boxes).unwrap();
        let want = app.canvas_point(&ui, 300.0, top + 200.0);
        assert!((all.x + all.w / 2.0 - want.0).abs() < 0.01, "the box centre lands on the pointer's x");
        assert!((all.y + all.h / 2.0 - want.1).abs() < 0.01, "the box centre lands on the pointer's y");
    }

    #[test]
    fn paste_keeps_the_offset_when_the_pointer_is_over_the_toolbar() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.pointer = Some((10.0, 0.0));
        app.paste_strokes(&ui, &two_lines());
        assert_eq!(app.model.strokes()[0].points[0].x, 5.0 + ui.pt(PASTE_OFFSET_PT));
    }

    #[test]
    fn bring_to_front_and_send_to_back_reorder_the_selection() {
        let ui = test_ui();
        let mut app = board(&ui);
        app.model = two_lines();
        app.styles = vec![Style { shadow: true, ..Style::default() }, Style::default()];
        app.selection = Some(Selection::from_indices(vec![0]));
        app.reorder_selection(&ui, true);
        assert_eq!(app.model.strokes()[1].points[0].x, 5.0, "the first stroke is now on top");
        assert!(app.styles[1].shadow, "its look went with it");
        assert_eq!(app.selection.as_ref().unwrap().indices(), &[1]);
        let undo = app.undo.len();
        app.reorder_selection(&ui, true);
        assert_eq!(app.undo.len(), undo, "already on top: nothing happens");
        app.reorder_selection(&ui, false);
        assert_eq!(app.model.strokes()[0].points[0].x, 5.0);
        assert_eq!(app.selection.as_ref().unwrap().indices(), &[0]);
    }

    #[test]
    fn the_board_is_written_after_an_edit_and_loaded_by_the_next_run() {
        let ui = test_ui();
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-freeform-board"));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = Freeform::new(&ui, 200, 200).with_store(dir.clone());
        app.ensure_canvas(&ui);
        app.model = two_lines();
        app.styles = vec![Style { dashed: true, ..Style::default() }, Style::default()];
        app.now = 0.0;
        app.mark_changed();
        assert_eq!(app.next_wake(0.0), Some(SAVE_DELAY_MS), "the save is scheduled");
        assert!(!dir.join("board.txt").exists(), "not written before the delay");
        app.tick(&ui, SAVE_DELAY_MS);
        assert!(dir.join("board.txt").exists());

        let mut again = Freeform::new(&ui, 200, 200).with_store(dir.clone());
        again.ensure_canvas(&ui);
        assert_eq!(again.model.strokes().len(), 2);
        assert!(again.styles[0].dashed);
        assert!(again.canvas.iter().any(|&p| p != PAPER), "the loaded strokes are painted");

        again.model = Drawing::new();
        again.mark_changed();
        again.flush();
        assert!(!dir.join("board.txt").exists(), "an empty board leaves no file");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
