//! Layout, hit tests and drawing for the Viewer window. Every size comes from the window size and the
//! `Ui` scale (`ui.pt`), and every colour from the shared tokens in `kit::ui`. The picture and text
//! draw first; the sidebar and the toolbar draw last, so anything that runs under them is covered.

use crate::document::{Document, THUMB_W};
use crate::forms::{FormWidget, Kind};
use crate::export_panel;
use crate::find::Hit;
use crate::signature::Signature;
use crate::signature_panel;
use crate::model::{self, Adjust, Mark, MarkKind, Norm, Slider, View, HIGHLIGHT, MARK_BLUE};
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::kit::ui::{self, Ui, Weight};
use bifrost_app::kit::{rgba, Image, Surface};
use bifrost_app::widgets;

/// The tool a drag or tap on the canvas uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pan,
    Highlight,
    Note,
    Rect,
    Rounded,
    /// A freehand stroke, for a signature.
    Sign,
    Oval,
    Line,
    Arrow,
    Underline,
    Strike,
    /// Drags out a rectangular selection, for Crop and Zoom to Selection.
    Select,
    /// A click clears the connected region of one colour to transparent.
    Alpha,
    /// A five-point star dragged out.
    Star,
    /// A hexagon dragged out.
    Polygon,
    /// A speech bubble dragged out.
    Bubble,
    /// A click places a text box, which takes the typed text.
    Text,
    /// Drags out a black box over a picture's content.
    Redact,
}

impl Tool {
    /// The mark a drag with this tool makes, or `None` for panning.
    pub fn mark_kind(self) -> Option<MarkKind> {
        match self {
            Tool::Pan => None,
            Tool::Highlight => Some(MarkKind::Highlight),
            Tool::Note => Some(MarkKind::Note),
            Tool::Rect => Some(MarkKind::Rect),
            Tool::Rounded => Some(MarkKind::Rounded),
            Tool::Sign => Some(MarkKind::Ink),
            Tool::Oval => Some(MarkKind::Oval),
            Tool::Line => Some(MarkKind::Line),
            Tool::Arrow => Some(MarkKind::Arrow),
            Tool::Underline => Some(MarkKind::Underline),
            Tool::Strike => Some(MarkKind::Strike),
            Tool::Star => Some(MarkKind::Star),
            Tool::Polygon => Some(MarkKind::Polygon),
            Tool::Bubble => Some(MarkKind::Bubble),
            Tool::Text => Some(MarkKind::Text),
            Tool::Redact => Some(MarkKind::Redact),
            Tool::Select | Tool::Alpha => None,
        }
    }
}

/// What the sidebar lists: the page thumbnails, the PDF outline, the markup, the bookmarks, or the search hits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Pages,
    Outline,
    Notes,
    Bookmarks,
    /// The hits of Find in PDF.
    Search,
}

/// How the canvas shows the document: one page, a grid of all the pages, or the pages one after another with nothing
/// else on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Page,
    Contact,
    Slideshow,
}

/// Everything a toolbar button or a menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Previous,
    Next,
    ZoomOut,
    ZoomIn,
    ActualSize,
    FitWindow,
    RotateLeft,
    RotateRight,
    UseTool(Tool),
    Brighter,
    Darker,
    Saturate,
    Desaturate,
    ResetColour,
    ShowPages,
    ShowOutline,
    MovePageUp,
    MovePageDown,
    DeletePage,
    ExportPng,
    ExportJpeg,
    ExportPdf,
    ExportAt25,
    ExportAt50,
    ExportAt100,
    ExportAt200,
    /// File > Print: the file as opened goes to the default printer. Menu only, not a toolbar button.
    Print,
    /// Edit > Undo and Redo: the document's edits (markup, page order, turns, colour).
    Undo,
    Redo,
    /// File > Revert to Original: drops all markup, restores page order and turns, resets colour.
    RevertToOriginal,
    /// Go > Back and Forward through the pages visited.
    GoBack,
    GoForward,
    FirstPage,
    LastPage,
    /// Go > Go to Page: the status line takes a page number.
    GoToPage,
    /// Go > Add Bookmark toggles a bookmark on the shown page.
    AddBookmark,
    NextBookmark,
    PreviousBookmark,
    /// View > Hide Sidebar / Show Sidebar.
    ToggleSidebar,
    /// Tools > Crop: the picture keeps the selected rectangle.
    Crop,
    FlipHorizontal,
    FlipVertical,
    /// Tools > Remove Background: the background that touches the picture's edge becomes transparent.
    RemoveBackground,
    ZoomToSelection,
    /// Tools > Show Magnifier / Hide Magnifier.
    ToggleMagnifier,
    ShowNotes,
    ShowBookmarks,
    /// View > Contact Sheet / Single Page.
    ContactSheet,
    Slideshow,
    /// Tools > Show Inspector / Hide Inspector.
    Inspector,
    /// Tools > Show Location Info.
    ShowLocation,
    /// File > New from Clipboard.
    NewFromClipboard,
    /// Edit > Find: the status line takes the search text.
    Find,
    FindNext,
    FindPrevious,
    ShowSearch,
    /// Tools > Adjust Size: the status line takes a width, a size or a percentage and the picture is resampled to it.
    AdjustSize,
    /// File > JPEG Quality: how hard an exported JPEG is compressed.
    QualityHigh,
    QualityMedium,
    QualityLow,
    /// File > Save: the document is written back, with its markup, page order and turns, and reopened.
    Save,
    /// File > Revert to Saved: the file is opened again, so every change since the last save goes.
    RevertToSaved,
    /// Edit > Copy: the selected text of a PDF page, or the picture, goes to the clipboard.
    CopyText,
    /// Edit > Select All: the whole shown page is selected.
    SelectAll,
    /// Tools > Save Signature: the last drawing on the shown page joins the signature library.
    SaveSignature,
    /// Tools > Clear Signatures: the signature library is emptied.
    ClearSignatures,
    /// View > Hide Toolbar / Show Toolbar.
    ToggleToolbar,
    /// Settings > Remember Last Page.
    ToggleRememberPage,
    /// Settings > Open PDFs as Contact Sheet.
    ToggleContactDefault,
    /// Settings > Annotation Author: the status line takes the author's name.
    SetAuthor,
    /// Adjust > Show Adjust Color / Hide Adjust Color: the right side shows sliders for the colour adjustments.
    ShowColorPanel,
    /// Adjust > Sharpen and Soften: one step of the sharpness up or down.
    Sharpen,
    Soften,
    /// File > Export As: the export panel on the right shows the format, quality and size choices and Export.
    ExportAs,
    /// The export panel's format choice.
    ChooseFormat(export_panel::Format),
    /// The export panel's Export button: the document is written in the chosen format.
    ExportNow,
    /// Tools > Show Signatures / Hide Signatures: the saved signatures list on the right, and a click places one.
    ShowSignatures,
}

/// Every action. A menu item's id is its index here.
pub const ACTIONS: &[Action] = &[
    Action::Previous,
    Action::Next,
    Action::ZoomOut,
    Action::ZoomIn,
    Action::FitWindow,
    Action::ActualSize,
    Action::RotateLeft,
    Action::RotateRight,
    Action::UseTool(Tool::Pan),
    Action::UseTool(Tool::Highlight),
    Action::UseTool(Tool::Note),
    Action::UseTool(Tool::Rect),
    Action::UseTool(Tool::Rounded),
    Action::UseTool(Tool::Sign),
    Action::Brighter,
    Action::Darker,
    Action::Saturate,
    Action::Desaturate,
    Action::ResetColour,
    Action::ShowPages,
    Action::ShowOutline,
    Action::MovePageUp,
    Action::MovePageDown,
    Action::DeletePage,
    Action::ExportPng,
    Action::ExportJpeg,
    Action::ExportPdf,
    Action::ExportAt25,
    Action::ExportAt50,
    Action::ExportAt100,
    Action::ExportAt200,
    Action::Print,
    Action::Undo,
    Action::Redo,
    Action::RevertToOriginal,
    Action::GoBack,
    Action::GoForward,
    Action::FirstPage,
    Action::LastPage,
    Action::GoToPage,
    Action::AddBookmark,
    Action::NextBookmark,
    Action::PreviousBookmark,
    Action::ToggleSidebar,
    Action::Crop,
    Action::FlipHorizontal,
    Action::FlipVertical,
    Action::RemoveBackground,
    Action::ZoomToSelection,
    Action::ToggleMagnifier,
    Action::ShowNotes,
    Action::ShowBookmarks,
    Action::ContactSheet,
    Action::Slideshow,
    Action::Inspector,
    Action::ShowLocation,
    Action::NewFromClipboard,
    Action::Find,
    Action::FindNext,
    Action::FindPrevious,
    Action::ShowSearch,
    Action::AdjustSize,
    Action::QualityHigh,
    Action::QualityMedium,
    Action::QualityLow,
    Action::UseTool(Tool::Oval),
    Action::UseTool(Tool::Line),
    Action::UseTool(Tool::Arrow),
    Action::UseTool(Tool::Underline),
    Action::UseTool(Tool::Strike),
    Action::UseTool(Tool::Select),
    Action::UseTool(Tool::Alpha),
    Action::UseTool(Tool::Star),
    Action::UseTool(Tool::Polygon),
    Action::UseTool(Tool::Bubble),
    Action::UseTool(Tool::Text),
    Action::UseTool(Tool::Redact),
    Action::Save,
    Action::RevertToSaved,
    Action::CopyText,
    Action::SelectAll,
    Action::SaveSignature,
    Action::ClearSignatures,
    Action::ToggleToolbar,
    Action::ToggleRememberPage,
    Action::ToggleContactDefault,
    Action::SetAuthor,
    Action::ShowColorPanel,
    Action::Sharpen,
    Action::Soften,
    Action::ExportAs,
    Action::ChooseFormat(export_panel::Format::Png),
    Action::ChooseFormat(export_panel::Format::Jpeg),
    Action::ChooseFormat(export_panel::Format::Pdf),
    Action::ExportNow,
    Action::ShowSignatures,
];

/// The toolbar buttons, in order. The rest are in the menus only.
const TOOLBAR: &[Action] = &[
    Action::Previous,
    Action::Next,
    Action::ZoomOut,
    Action::ZoomIn,
    Action::FitWindow,
    Action::ActualSize,
    Action::RotateLeft,
    Action::RotateRight,
    Action::UseTool(Tool::Pan),
    Action::UseTool(Tool::Highlight),
    Action::UseTool(Tool::Note),
    Action::UseTool(Tool::Rect),
    Action::UseTool(Tool::Rounded),
    Action::UseTool(Tool::Sign),
    Action::ShowPages,
    Action::ShowOutline,
    Action::Brighter,
    Action::Darker,
    Action::Saturate,
    Action::Desaturate,
    Action::ResetColour,
];

impl Action {
    pub fn id(self) -> u32 {
        ACTIONS.iter().position(|a| *a == self).unwrap_or(0) as u32
    }

    pub fn from_id(id: u32) -> Option<Action> {
        ACTIONS.get(id as usize).copied()
    }

    pub fn label(self) -> &'static str {
        match self {
            Action::Previous => "Previous",
            Action::Next => "Next",
            Action::ZoomOut => "Zoom Out",
            Action::ZoomIn => "Zoom In",
            Action::ActualSize => "Actual Size",
            Action::FitWindow => "Fit",
            Action::RotateLeft => "Rotate Left",
            Action::RotateRight => "Rotate Right",
            Action::UseTool(Tool::Pan) => "Pan",
            Action::UseTool(Tool::Highlight) => "Highlight",
            Action::UseTool(Tool::Note) => "Note",
            Action::UseTool(Tool::Rect) => "Rectangle",
            Action::UseTool(Tool::Rounded) => "Rounded",
            Action::UseTool(Tool::Sign) => "Sign",
            Action::Brighter => "Brighter",
            Action::Darker => "Darker",
            Action::Saturate => "Saturate",
            Action::Desaturate => "Desaturate",
            Action::ResetColour => "Reset Colour",
            Action::ShowPages => "Pages",
            Action::ShowOutline => "Outline",
            Action::MovePageUp => "Move Page Up",
            Action::MovePageDown => "Move Page Down",
            Action::DeletePage => "Delete Page",
            Action::ExportPng => "Export PNG",
            Action::ExportJpeg => "Export JPEG",
            Action::ExportPdf => "Export PDF",
            Action::ExportAt25 => "Export at 25%",
            Action::ExportAt50 => "Export at 50%",
            Action::ExportAt100 => "Export at 100%",
            Action::ExportAt200 => "Export at 200%",
            Action::Print => "Print\u{2026}",
            Action::Undo => "Undo",
            Action::Redo => "Redo",
            Action::RevertToOriginal => "Revert to Original",
            Action::GoBack => "Back",
            Action::GoForward => "Forward",
            Action::FirstPage => "First Page",
            Action::LastPage => "Last Page",
            Action::GoToPage => "Go to Page\u{2026}",
            Action::AddBookmark => "Add Bookmark",
            Action::NextBookmark => "Next Bookmark",
            Action::PreviousBookmark => "Previous Bookmark",
            Action::ToggleSidebar => "Hide Sidebar",
            Action::Crop => "Crop",
            Action::FlipHorizontal => "Flip Horizontal",
            Action::FlipVertical => "Flip Vertical",
            Action::RemoveBackground => "Remove Background",
            Action::ZoomToSelection => "Zoom to Selection",
            Action::ToggleMagnifier => "Show Magnifier",
            Action::ShowNotes => "Markup",
            Action::ShowBookmarks => "Bookmarks",
            Action::ContactSheet => "Contact Sheet",
            Action::Slideshow => "Slideshow",
            Action::Inspector => "Show Inspector",
            Action::ShowLocation => "Show Location Info",
            Action::NewFromClipboard => "New from Clipboard",
            Action::Find => "Find\u{2026}",
            Action::FindNext => "Find Next",
            Action::FindPrevious => "Find Previous",
            Action::ShowSearch => "Search Results",
            Action::AdjustSize => "Adjust Size\u{2026}",
            Action::QualityHigh => "JPEG Quality: High",
            Action::QualityMedium => "JPEG Quality: Medium",
            Action::QualityLow => "JPEG Quality: Low",
            Action::UseTool(Tool::Oval) => "Oval",
            Action::UseTool(Tool::Line) => "Line",
            Action::UseTool(Tool::Arrow) => "Arrow",
            Action::UseTool(Tool::Underline) => "Underline",
            Action::UseTool(Tool::Strike) => "Strike Through",
            Action::UseTool(Tool::Select) => "Select",
            Action::UseTool(Tool::Alpha) => "Instant Alpha",
            Action::UseTool(Tool::Star) => "Star",
            Action::UseTool(Tool::Polygon) => "Polygon",
            Action::UseTool(Tool::Bubble) => "Speech Bubble",
            Action::UseTool(Tool::Text) => "Text Box",
            Action::UseTool(Tool::Redact) => "Redact",
            Action::Save => "Save",
            Action::RevertToSaved => "Revert to Saved",
            Action::CopyText => "Copy",
            Action::SelectAll => "Select All",
            Action::SaveSignature => "Save Signature",
            Action::ClearSignatures => "Clear Signatures",
            Action::ToggleToolbar => "Hide Toolbar",
            Action::ToggleRememberPage => "Remember Last Page",
            Action::ToggleContactDefault => "Open PDFs as Contact Sheet",
            Action::SetAuthor => "Annotation Author\u{2026}",
            Action::ShowColorPanel => "Show Adjust Color",
            Action::Sharpen => "Sharpen",
            Action::Soften => "Soften",
            Action::ExportAs => "Export As\u{2026}",
            Action::ChooseFormat(format) => format.label(),
            Action::ExportNow => "Export",
            Action::ShowSignatures => "Show Signatures",
        }
    }

    /// The scale of an export-size action.
    pub fn export_scale(self) -> Option<f32> {
        match self {
            Action::ExportAt25 => Some(0.25),
            Action::ExportAt50 => Some(0.5),
            Action::ExportAt100 => Some(1.0),
            Action::ExportAt200 => Some(2.0),
            _ => None,
        }
    }

    /// The JPEG quality of a quality action.
    pub fn jpeg_quality(self) -> Option<u8> {
        match self {
            Action::QualityHigh => Some(90),
            Action::QualityMedium => Some(75),
            Action::QualityLow => Some(50),
            _ => None,
        }
    }

    /// The toolbar buttons, in order.
    pub fn toolbar() -> &'static [Action] {
        TOOLBAR
    }
}

/// Window geometry: the toolbar along the top, the thumbnails down the left, the canvas in between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub w: i32,
    pub h: i32,
    pub bar: i32,
    pub side: i32,
    /// The width of the Inspector along the right edge; 0 while it is hidden.
    pub inspector: i32,
}

impl Layout {
    pub fn new(ui: &Ui, w: i32, h: i32) -> Layout {
        let bar = ui.pt(ui::SPACE_XXXL + ui::SPACE_L) as i32;
        let side = (ui.pt(ui::SPACE_XXXL * 5.0) as i32).min(w / 3).max(0);
        Layout { w: w.max(1), h: h.max(1), bar, side, inspector: 0 }
    }

    /// The canvas rectangle (x, y, w, h).
    pub fn canvas(&self) -> (i32, i32, i32, i32) {
        (self.side, self.bar, (self.w - self.side - self.inspector).max(1), (self.h - self.bar).max(1))
    }

    /// The window with the sidebar, toolbar and Inspector gone: the slideshow's layout.
    pub fn bare(w: i32, h: i32) -> Layout {
        Layout { w: w.max(1), h: h.max(1), bar: 0, side: 0, inspector: 0 }
    }

    /// The width the Inspector takes when it is shown.
    pub fn inspector_width(ui: &Ui, w: i32) -> i32 {
        (ui.pt(ui::SPACE_XXXL * 6.0) as i32).min(w / 2).max(0)
    }
}

/// The shared Golden Gate toolbar model and the exact layout used to draw it and publish its controls.
pub fn toolbar(ui: &Ui, lay: &Layout, open: bool, tool: Tool, side: Side) -> (Toolbar, ToolbarLayout) {
    let mut items: Vec<ToolbarItem> = Action::toolbar()
        .iter()
        .copied()
        .map(|action| {
            let symbol = match action {
                Action::Previous => Some("chevron.left"),
                Action::Next => Some("chevron.right"),
                Action::ZoomOut => Some("magnifyingglass"),
                Action::ZoomIn => Some("plus"),
                Action::RotateRight => Some("arrow.clockwise"),
                Action::UseTool(Tool::Note) => Some("square.and.pencil"),
                Action::UseTool(Tool::Sign) => Some("pencil"),
                Action::ShowPages => Some("rectangle.grid.1x2"),
                Action::ShowOutline => Some("list.bullet"),
                _ => None,
            };
            let mut item = symbol.map_or_else(
                || ToolbarItem::button(action.id(), action.label()),
                |symbol| ToolbarItem::icon(action.id(), action.label(), symbol),
            );
            let group = match action {
                Action::Previous | Action::Next => Some(1),
                Action::ZoomOut | Action::ZoomIn => Some(2),
                _ => None,
            };
            if let Some(group) = group {
                item = item.in_group(group);
            }
            let selected = match action {
                Action::UseTool(selected) => selected == tool,
                Action::ShowPages => side == Side::Pages,
                Action::ShowOutline => side == Side::Outline,
                _ => false,
            };
            item.enabled(open).selected(selected)
        })
        .collect();
    let scale = ui.pt(1.0).max(0.001);
    let bounds = bifrost_app::controls::Rect {
        x: lay.side as f32 / scale,
        y: 0.0,
        w: (lay.w - lay.side).max(0) as f32 / scale,
        h: lay.bar.max(0) as f32 / scale,
    };
    // Preserve the old narrow-window behavior: actions that do not fit remain available from the menus, rather than
    // exposing the kit's overflow affordance without an app-specific overflow menu.
    let full = Toolbar::new(items.clone());
    let first = widgets::toolbar_layout(ui, bounds, &full);
    if !first.overflow.is_empty() {
        items = items.into_iter().filter(|item| !first.overflow.contains(&item.id)).collect();
    }
    let bar = Toolbar::new(items);
    let layout = widgets::toolbar_layout(ui, bounds, &bar);
    (bar, layout)
}

/// The thumbnail rows in content coordinates: (top, height) for each page, with the gap before.
pub fn rows(ui: &Ui, doc: &Document) -> Vec<(i32, i32)> {
    let gap = ui.pt(ui::SPACE_S) as i32;
    let label = ui.pt(ui::MAC_FOOTNOTE_PT * 1.6) as i32;
    let mut y = gap;
    let mut out = Vec::with_capacity(doc.count());
    for i in 0..doc.count() {
        let box_h = doc.thumb_box(i).map_or((THUMB_W as f32 * 1.3) as i32, |(_, h)| h as i32);
        let h = box_h + label + gap;
        out.push((y, h));
        y += h;
    }
    out
}

/// The outline rows in content coordinates: one line each.
pub fn outline_rows(ui: &Ui, doc: &Document) -> Vec<(i32, i32)> {
    let row = ui.pt(ui::MAC_BODY_PT * 2.0) as i32;
    (0..doc.outline.len()).map(|i| (i as i32 * row, row)).collect()
}

/// The rows of a sidebar list of `n` one-line entries (the markup list and the bookmarks).
pub fn list_rows(ui: &Ui, n: usize) -> Vec<(i32, i32)> {
    let row = ui.pt(ui::MAC_BODY_PT * 2.0) as i32;
    (0..n).map(|i| (i as i32 * row, row)).collect()
}

/// The entries of the markup list, the bookmarks list or the search hits: the text to show and the shown page it goes to.
/// The markup list holds every mark but strokes, by shown page; the bookmarks list holds the bookmarked pages in page
/// order; the search list holds the hits on pages that are still shown. The other sidebar lists have no entries here.
pub fn side_items(doc: &Document, side: Side, found: &[Hit]) -> Vec<(String, Option<usize>)> {
    match side {
        Side::Notes => {
            let mut out = Vec::new();
            for shown in 0..doc.count() {
                let Some(src) = doc.source_of(shown) else { continue };
                for m in doc.marks.iter().filter(|m| m.page == src && m.kind != MarkKind::Ink) {
                    let text = if m.text.is_empty() { String::new() } else { format!(": {}", m.text) };
                    out.push((format!("{}  {}{}", shown + 1, m.kind.label(), text), Some(shown)));
                }
            }
            out
        }
        Side::Bookmarks => {
            let mut places = doc.bookmark_places();
            places.sort_unstable();
            places.into_iter().map(|p| (format!("Page {}", p + 1), Some(p))).collect()
        }
        Side::Search => found
            .iter()
            .filter_map(|hit| {
                let shown = doc.display_of(hit.page)?;
                Some((format!("{}  {}", shown + 1, hit.snippet), Some(shown)))
            })
            .collect(),
        Side::Pages | Side::Outline => Vec::new(),
    }
}

/// Shared source-list rows for the text-based sidebar modes. Page thumbnails retain their variable-height gallery
/// rows; outline, markup, bookmark and search entries use the Golden Gate source-list widget and hit model.
fn source_list(doc: &Document, side: Side, found: &[Hit], current_page: usize) -> SourceList {
    let (entries, symbol): (Vec<(String, Option<usize>, Option<String>)>, &str) = match side {
        Side::Outline => (
            doc.outline
                .iter()
                .map(|item| {
                    let page = item.page.and_then(|p| doc.display_of(p));
                    let trailing = page.map(|p| format!("Page {}", p + 1));
                    (item.title.clone(), page, trailing)
                })
                .collect(),
            "doc",
        ),
        Side::Notes => (side_items(doc, side, found).into_iter().map(|(label, page)| (label, page, None)).collect(), "tag"),
        Side::Bookmarks => (side_items(doc, side, found).into_iter().map(|(label, page)| (label, page, None)).collect(), "bookmark"),
        Side::Search => (side_items(doc, side, found).into_iter().map(|(label, page)| (label, page, None)).collect(), "magnifyingglass"),
        Side::Pages => (Vec::new(), "doc"),
    };
    let mut items = Vec::with_capacity(entries.len());
    let mut selected = None;
    for (index, (label, page, trailing)) in entries.into_iter().enumerate() {
        let mut row = SidebarItem::row(index as u32, &label).with_symbol(symbol);
        if let Some(trailing) = trailing { row = row.with_trailing(&trailing); }
        if page == Some(current_page) && selected.is_none() { selected = Some(index as u32); }
        items.push(row);
    }
    let mut list = SourceList::new(items, ui::MAC_BODY_PT * 2.0, ui::MAC_FOOTNOTE_PT * 2.0);
    list.set_selected(selected);
    list
}

/// Hit-tests a text-based sidebar using the same shared source-list model used to draw it.
pub fn source_list_hit(ui: &Ui, lay: &Layout, doc: &Document, side: Side, found: &[Hit], current_page: usize, scroll: f32, x: f32, y: f32) -> Option<usize> {
    let scale = ui.pt(1.0).max(0.001);
    let list = source_list(doc, side, found, current_page);
    let bounds = bifrost_app::controls::Rect {
        x: 0.0,
        y: lay.bar as f32 / scale,
        w: lay.side as f32 / scale,
        h: (lay.h - lay.bar) as f32 / scale,
    };
    list.hit(bounds, scroll / scale, x / scale, y / scale).map(|id| id as usize)
}

/// The contact sheet's grid: equal cells in rows, the grid centred across the canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sheet {
    pub cols: usize,
    pub cell_w: i32,
    pub cell_h: i32,
    /// The space left of the first column.
    pub left: i32,
}

impl Sheet {
    /// A grid of `cell_w` by `cell_h` cells across a canvas `canvas_w` wide: as many columns as fit, at least one.
    pub fn new(canvas_w: i32, cell_w: i32, cell_h: i32) -> Sheet {
        let (cell_w, cell_h) = (cell_w.max(1), cell_h.max(1));
        let cols = (canvas_w / cell_w).max(1) as usize;
        let left = ((canvas_w - cols as i32 * cell_w) / 2).max(0);
        Sheet { cols, cell_w, cell_h, left }
    }

    /// The top-left of cell `i`, from the canvas's top-left with no scroll.
    pub fn origin(&self, i: usize) -> (i32, i32) {
        (self.left + (i % self.cols) as i32 * self.cell_w, (i / self.cols) as i32 * self.cell_h)
    }

    /// The height of `count` cells in rows.
    pub fn height(&self, count: usize) -> i32 {
        count.div_ceil(self.cols) as i32 * self.cell_h
    }

    /// The cell under a point given from the canvas's top-left, with the sheet scrolled by `scroll`.
    pub fn hit(&self, count: usize, scroll: f32, x: f32, y: f32) -> Option<usize> {
        let (lx, ly) = (x - self.left as f32, y + scroll);
        if lx < 0.0 || ly < 0.0 {
            return None;
        }
        let col = lx as i32 / self.cell_w;
        if col as usize >= self.cols {
            return None;
        }
        let i = (ly as i32 / self.cell_h) as usize * self.cols + col as usize;
        (i < count).then_some(i)
    }
}

/// The contact sheet for `doc` on a canvas `canvas_w` wide. A cell fits the widest thumbnail and the tallest, with the
/// page number under it.
pub fn contact_sheet(ui: &Ui, doc: &Document, canvas_w: i32) -> Sheet {
    let pad = ui.pt(ui::SPACE_L) as i32;
    let label = ui.pt(ui::MAC_FOOTNOTE_PT * 1.6) as i32;
    let tallest = (0..doc.count()).filter_map(|i| doc.thumb_box(i)).map(|(_, h)| h as i32).max().unwrap_or(THUMB_W as i32);
    Sheet::new(canvas_w, THUMB_W as i32 + 2 * pad, tallest + label + 2 * pad)
}

/// The sidebar's total content height.
pub fn rows_height(rows: &[(i32, i32)]) -> i32 {
    rows.last().map_or(0, |(top, h)| top + h)
}

/// The row under a sidebar point, if any.
pub fn sidebar_hit(lay: &Layout, rows: &[(i32, i32)], scroll: f32, x: f32, y: f32) -> Option<usize> {
    if x < 0.0 || x >= lay.side as f32 || y < lay.bar as f32 {
        return None;
    }
    let content = y - lay.bar as f32 + scroll;
    rows.iter().position(|(top, h)| content >= *top as f32 && content < (top + h) as f32)
}

/// The toolbar action under a point, if any.
pub fn button_hit(ui: &Ui, lay: &Layout, open: bool, tool: Tool, side: Side, x: f32, y: f32) -> Option<Action> {
    let (bar, layout) = toolbar(ui, lay, open, tool, side);
    let scale = ui.pt(1.0).max(0.001);
    let id = layout.hit(x / scale, y / scale)?;
    bar.items.iter().find(|item| item.id == id && item.enabled).and_then(|item| Action::from_id(item.id))
}

/// Where the picture sits on the canvas: its top-left and its drawn size, after the pan is clamped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub x: i32,
    pub y: i32,
    pub w: usize,
    pub h: usize,
    pub pan: (f32, f32),
}

/// Places a `rw` by `rh` picture drawn at `scale` on the canvas, centred, panned by `pan` and kept
/// in view horizontally and vertically (a picture smaller than the canvas is not panned).
pub fn place(canvas: (i32, i32, i32, i32), rw: usize, rh: usize, scale: f32, pan: (f32, f32)) -> Placed {
    let (cx, cy, cw, ch) = canvas;
    let w = ((rw as f32 * scale).round() as usize).max(1);
    let h = ((rh as f32 * scale).round() as usize).max(1);
    let lim_x = ((w as f32 - cw as f32) / 2.0).max(0.0);
    let lim_y = ((h as f32 - ch as f32) / 2.0).max(0.0);
    let px = pan.0.clamp(-lim_x, lim_x);
    let py = pan.1.clamp(-lim_y, lim_y);
    let x = cx + (cw - w as i32) / 2 + px.round() as i32;
    let y = cy + (ch - h as i32) / 2 + py.round() as i32;
    Placed { x, y, w, h, pan: (px, py) }
}

/// The source-page coordinates (normalised, unrotated) of a canvas point over a placed picture.
/// Points past the picture are clamped to its edge.
pub fn to_source(placed: &Placed, quarters: u8, x: f32, y: f32) -> (f32, f32) {
    let u = ((x - placed.x as f32) / placed.w as f32).clamp(0.0, 1.0);
    let v = ((y - placed.y as f32) / placed.h as f32).clamp(0.0, 1.0);
    model::unrot_point(u, v, quarters)
}

/// The canvas point of a source-page point over a placed picture.
fn screen_point(placed: &Placed, quarters: u8, u: f32, v: f32) -> (f32, f32) {
    let (a, b) = model::rot_point(u, v, quarters);
    (placed.x as f32 + a * placed.w as f32, placed.y as f32 + b * placed.h as f32)
}

/// The canvas rectangle of a source-page rectangle over a placed picture.
fn to_screen(placed: &Placed, quarters: u8, r: Norm) -> (f32, f32, f32, f32) {
    let t = model::rot_rect(r, quarters);
    (
        placed.x as f32 + t.x * placed.w as f32,
        placed.y as f32 + t.y * placed.h as f32,
        t.w * placed.w as f32,
        t.h * placed.h as f32,
    )
}

/// The rendered picture of one page, kept while nothing that changes it has changed. The key names the source page.
pub struct PictureCache {
    key: (usize, Adjust, u8, usize, usize, u32),
    pub img: Image,
}

/// Looks up or makes the scaled, adjusted and turned picture of source page `src` for the current view.
/// `revision` counts the document's pixel edits, so an edited picture of the same size is drawn again.
// The picture, the cache and the view are separate inputs of one lookup.
#[allow(clippy::too_many_arguments)]
pub fn picture_for<'a>(
    cache: &'a mut Option<PictureCache>,
    src: usize,
    revision: u32,
    picture: &Image,
    adjust: &Adjust,
    quarters: u8,
    w: usize,
    h: usize,
) -> &'a Image {
    let key = (src, *adjust, quarters, w, h, revision);
    if cache.as_ref().map(|c| c.key) != Some(key) {
        let base = crate::document::adjusted(picture, adjust);
        let turned = model::rotate(&base, quarters);
        let img = if (turned.w, turned.h) == (w, h) { turned } else { turned.resized(w, h) };
        *cache = Some(PictureCache { key, img });
    }
    &cache.as_ref().expect("the picture was just cached").img
}

/// What the canvas draws this frame.
pub struct Scene<'a> {
    pub doc: Option<&'a Document>,
    pub view: View,
    pub tool: Tool,
    pub adjust: Adjust,
    /// The sidebar's list.
    pub side: Side,
    /// The mark being dragged out, drawn with the saved ones.
    pub draft: Option<Mark>,
    /// The note whose text the keyboard is editing.
    pub editing: Option<usize>,
    /// The form widget that the keyboard is typing into.
    pub form_focus: Option<usize>,
    /// A message to show in the middle of the canvas (an open error, an export result).
    pub status: &'a str,
    pub mode: Mode,
    /// The rectangle selected on the shown page, in source-page coordinates.
    pub selection: Option<Norm>,
    /// Where the magnifier is, in canvas pixels; None while it is off.
    pub loupe: Option<(f32, f32)>,
    /// The Inspector's rows; None while it is hidden.
    pub inspector: Option<&'a [(String, String)]>,
    /// How far the contact sheet is scrolled.
    pub contact_scroll: f32,
    /// The hits of the last search.
    pub found: &'a [Hit],
    /// The Adjust Color panel's sliders, when the panel is shown; it takes the right side instead of the Inspector.
    pub color: Option<&'a [Slider]>,
    /// The Export As panel, when it is shown; it takes the right side before the Adjust Color panel and the Inspector.
    pub export: Option<export_panel::Panel>,
    /// The saved signatures, when their picker is shown on the right.
    pub signatures: Option<&'a [Signature]>,
}

/// Draws the window. `cache` keeps the scaled picture between frames.
pub fn draw(s: &mut Surface, ui: &Ui, lay: &Layout, scene: &Scene, cache: &mut Option<PictureCache>) -> Option<Placed> {
    let (cx, cy, cw, ch) = lay.canvas();
    let show = scene.mode == Mode::Slideshow;
    fill(s, cx, cy, cw, ch, ui::CONTENT_BG.now());
    if let (Mode::Contact, Some(doc)) = (scene.mode, scene.doc) {
        draw_sheet(s, ui, lay, scene, doc);
        draw_sidebar(s, ui, lay, scene);
        draw_toolbar(s, ui, lay, scene);
        draw_right_panel(s, ui, lay, scene);
        return None;
    }
    let mut placed = None;
    let mut shown_src = None;
    // the drawn picture in the cache is this page's only once the page's picture has arrived
    let mut cached = false;
    match scene.doc {
        None => {
            let msg = if scene.status.is_empty() { "Open an image or a PDF to view it." } else { scene.status };
            centred_text(s, ui, msg, lay, ui::LABEL_2.now());
        }
        Some(doc) => {
            let page_no = scene.view.page.min(doc.count().saturating_sub(1));
            if let (Some(src), Some((w, h))) = (doc.source_of(page_no), doc.page_size(page_no)) {
                let q = scene.view.quarters;
                let (rw, rh) = model::turn_size(w, h, q);
                let scale = scene.view.zoom.scale(rw, rh, cw as usize, ch as usize);
                let target_w = ((rw as f32 * scale).round() as usize).max(1);
                let target_h = ((rh as f32 * scale).round() as usize).max(1);
                let p = place((cx, cy, cw, ch), rw, rh, scale, scene.view.pan);
                match doc.picture(page_no) {
                    Some(img) => {
                        let shown = picture_for(cache, src, doc.revision, &img, &scene.adjust, q, target_w, target_h);
                        s.blit(shown, p.x, p.y, 255, 255);
                        cached = true;
                    }
                    // a PDF page is still on its way from the worker: the paper shows first
                    // compliance-allow: COL-03 the loading placeholder intentionally depicts the PDF's white paper.
                    None => fill(s, p.x, p.y, p.w as i32, p.h as i32, ui::WHITE),
                }
                for m in doc.marks.iter().filter(|m| m.page == src).chain(scene.draft.iter()) {
                    draw_mark(s, &p, q, m);
                }
                // the form fields of this page, over the marks
                for (i, w) in doc.widgets.iter().enumerate().filter(|(_, w)| w.page == src) {
                    let field = doc.form.get(&w.name);
                    let value = field.map(|f| f.value()).unwrap_or("");
                    let on = field.is_some_and(|f| f.is_on());
                    draw_widget(s, ui, &p, q, w, value, on, scene.form_focus == Some(i));
                }
                placed = Some(p);
                shown_src = Some(src);
            }
        }
    }
    // a note's text sits beside its marker on picture pages; a text box holds its text inside its frame
    if let (Some(doc), Some(p), Some(src)) = (scene.doc, placed, shown_src) {
        for (i, m) in doc.marks.iter().enumerate().filter(|(_, m)| m.page == src && matches!(m.kind, MarkKind::Note | MarkKind::Text)) {
            if m.text.is_empty() && scene.editing != Some(i) {
                continue;
            }
            let (x, y, w, h) = to_screen(&p, scene.view.quarters, m.rect);
            let shown = if scene.editing == Some(i) { format!("{}|", m.text) } else { m.text.clone() };
            if m.kind == MarkKind::Text {
                let pad = ui.pt(ui::SPACE_S);
                s.stroke_rrect(x, y, w.max(1.0), h.max(1.0), 0.0, 1.0, m.color);
                let img = ui.text(&shown, ui::MAC_BODY_PT, Weight::Regular, ui::WHITE);
                s.blit_colored(&img, (x + pad) as i32, (y + (h - img.h as f32) / 2.0) as i32, m.color, 255);
            } else {
                let img = ui.text(&shown, ui::MAC_CALLOUT_PT, Weight::Regular, ui::WHITE);
                s.blit_colored(&img, x as i32 + ui.pt(ui::SPACE_L) as i32, y as i32, ui::LABEL.now(), 255);
            }
        }
    }
    if let (Some(p), Some(sel)) = (placed, scene.selection) {
        let (x, y, w, h) = to_screen(&p, scene.view.quarters, sel);
        s.stroke_rrect(x, y, w.max(1.0), h.max(1.0), 0.0, 1.5, ui::TINT);
    }
    if let (Some(p), Some(at), true) = (placed, scene.loupe, cached) {
        if let Some(c) = cache.as_ref() {
            draw_loupe(s, ui, &p, &c.img, at);
        }
    }
    if scene.doc.is_some() && !scene.status.is_empty() && !show {
        let img = ui.text(scene.status, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::WHITE);
        let pad = ui.pt(ui::SPACE_S) as i32;
        s.blit_colored(&img, cx + pad, cy + ch - img.h as i32 - pad, ui::LABEL_2.now(), 255);
    }
    draw_sidebar(s, ui, lay, scene);
    if !show {
        draw_toolbar(s, ui, lay, scene);
        draw_right_panel(s, ui, lay, scene);
    }
    placed
}

/// How much the magnifier enlarges.
const LOUPE_ZOOM: f32 = 2.5;

/// The picture pixel the magnifier shows at offset (`dx`, `dy`) from its centre. `origin` is the picture's top-left on
/// the canvas and `centre` the pointer, both in canvas pixels; the picture is the one drawn, already at the zoom.
pub fn loupe_sample(centre: (f32, f32), origin: (i32, i32), dx: i32, dy: i32) -> (i32, i32) {
    (
        (centre.0 - origin.0 as f32 + dx as f32 / LOUPE_ZOOM).floor() as i32,
        (centre.1 - origin.1 as f32 + dy as f32 / LOUPE_ZOOM).floor() as i32,
    )
}

/// Draws the magnifier: a round lens around `at` that shows the picture under it enlarged. Pixels that are not opaque
/// (a transparent corner, or past the picture's edge) are left as they are.
fn draw_loupe(s: &mut Surface, ui: &Ui, p: &Placed, shown: &Image, at: (f32, f32)) {
    let r = ui.pt(ui::SPACE_XXXL * 1.5) as i32;
    let (cx, cy) = (at.0 as i32, at.1 as i32);
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy > r * r {
                continue;
            }
            let (x, y) = (cx + dx, cy + dy);
            if x < 0 || y < s.y0 as i32 || x >= s.w as i32 || y >= s.h as i32 {
                continue;
            }
            let (sx, sy) = loupe_sample(at, (p.x, p.y), dx, dy);
            if sx < 0 || sy < 0 || sx as usize >= shown.w || sy as usize >= shown.h {
                continue;
            }
            let pixel = shown.px[sy as usize * shown.w + sx as usize];
            if pixel >> 24 == 255 {
                *s.at(x as usize, y as usize) = pixel;
            }
        }
    }
    s.stroke_rrect((cx - r) as f32, (cy - r) as f32, (2 * r) as f32, (2 * r) as f32, r as f32, 2.0, ui::TINT);
}

/// The contact sheet: every page's thumbnail in a grid, the shown page marked.
fn draw_sheet(s: &mut Surface, ui: &Ui, lay: &Layout, scene: &Scene, doc: &Document) {
    let (cx, cy, cw, ch) = lay.canvas();
    let sheet = contact_sheet(ui, doc, cw);
    let pad = ui.pt(ui::SPACE_S) as i32;
    for i in 0..doc.count() {
        let (ox, oy) = sheet.origin(i);
        let (x, y) = (cx + ox, cy + oy - scene.contact_scroll as i32);
        if y + sheet.cell_h < cy || y > cy + ch {
            continue;
        }
        if i == scene.view.page {
            s.fill_rrect((x + pad / 2) as f32, (y + pad / 2) as f32, (sheet.cell_w - pad) as f32, (sheet.cell_h - pad) as f32, ui.pt(ui::SPACE_S), ui::SELECTION.now());
        }
        let (bw, bh) = doc.thumb_box(i).unwrap_or((THUMB_W, (THUMB_W as f32 * 1.3) as usize));
        let top = y + ui.pt(ui::SPACE_L) as i32;
        match doc.thumb(i) {
            Some(t) => s.blit(&t, x + (sheet.cell_w - t.w as i32) / 2, top, 255, 255),
            None => fill(s, x + (sheet.cell_w - bw as i32) / 2, top, bw as i32, bh as i32, ui::CARD_BG.now()),
        }
        let text = format!("{}", i + 1);
        let img = ui.text(&text, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::WHITE);
        let tw = ui.measure(&text, ui::MAC_FOOTNOTE_PT, Weight::Regular) as i32;
        s.blit_colored(&img, x + (sheet.cell_w - tw) / 2, top + bh as i32 + pad / 2, ui::LABEL_2.now(), 255);
    }
}

/// The right-hand panel: the Export As panel while it is shown, else the signature picker, else the Adjust Color
/// sliders, else the Inspector.
fn draw_right_panel(s: &mut Surface, ui: &Ui, lay: &Layout, scene: &Scene) {
    if let Some(panel) = &scene.export {
        draw_export_panel(s, ui, lay, panel);
    } else if let Some(library) = scene.signatures {
        draw_signature_panel(s, ui, lay, library);
    } else if let Some(sliders) = scene.color {
        draw_color_panel(s, ui, lay, sliders);
    } else if let Some(rows) = scene.inspector {
        draw_inspector(s, ui, lay, rows);
    }
}

/// The top and height of Adjust Color slider `i`, in window pixels.
pub fn slider_row(ui: &Ui, lay: &Layout, i: usize) -> (i32, i32) {
    let pad = ui.pt(ui::SPACE_S) as i32;
    let row = ui.pt(ui::MAC_BODY_PT * 3.0) as i32;
    (lay.bar + pad + i as i32 * row, row)
}

/// The left edge and the width of the Adjust Color track, in window pixels.
pub fn slider_track(ui: &Ui, lay: &Layout) -> (i32, i32) {
    let pad = ui.pt(ui::SPACE_S) as i32;
    (lay.w - lay.inspector + pad, (lay.inspector - 2 * pad).max(1))
}

/// The export panel's sizes at this window: it takes the place of the inspector on the right.
pub fn export_metrics(ui: &Ui, lay: &Layout) -> export_panel::Metrics {
    let pad = ui.pt(ui::SPACE_S) as i32;
    let row = ui.pt(ui::MAC_BODY_PT * 2.0) as i32;
    let heading = ui.pt(ui::MAC_FOOTNOTE_PT * 1.6) as i32;
    export_panel::Metrics { left: lay.w - lay.inspector, top: lay.bar, width: lay.inspector, pad, row, heading, gap: pad }
}

/// The slider of the Adjust Color panel under a window point, when the panel is shown and the point is in one of its
/// `count` rows.
pub fn slider_hit(ui: &Ui, lay: &Layout, count: usize, x: f32, y: f32) -> Option<usize> {
    if lay.inspector <= 0 || x < (lay.w - lay.inspector) as f32 {
        return None;
    }
    (0..count).find(|&i| {
        let (top, h) = slider_row(ui, lay, i);
        y >= top as f32 && y < (top + h) as f32
    })
}

/// The value a point across the track sets on `slider`: its range, by the point's share of the track, clamped to the range.
pub fn slider_value(ui: &Ui, lay: &Layout, slider: &Slider, x: f32) -> f32 {
    let (tx, tw) = slider_track(ui, lay);
    let t = ((x - tx as f32) / tw as f32).clamp(0.0, 1.0);
    slider.min + t * (slider.max - slider.min)
}

/// The Adjust Color panel: each adjustment's name and value over a track with a knob at its place in the range.
fn draw_color_panel(s: &mut Surface, ui: &Ui, lay: &Layout, sliders: &[Slider]) {
    let w = lay.inspector;
    if w <= 0 {
        return;
    }
    let (x, top) = (lay.w - w, lay.bar);
    fill(s, x, top, w, lay.h - top, ui::SIDEBAR_BG.now());
    fill(s, x, top, 1, lay.h - top, ui::SEPARATOR_C.now());
    let pad = ui.pt(ui::SPACE_S) as i32;
    let (tx, tw) = slider_track(ui, lay);
    for (i, slider) in sliders.iter().enumerate() {
        let (row_top, row_h) = slider_row(ui, lay, i);
        let name = ui.text(slider.label, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL.now());
        s.blit_colored(&name, x + pad, row_top, ui::LABEL_2.now(), 255);
        let readout = format!("{:.2}", slider.value);
        let value = ui.text(&readout, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL.now());
        s.blit_colored(&value, x + w - pad - value.w as i32, row_top, ui::LABEL_2.now(), 255);
        let ty = row_top + row_h * 2 / 3;
        fill(s, tx, ty, tw, 2, ui::TRACK_C.now());
        let t = if slider.max > slider.min { (slider.value - slider.min) / (slider.max - slider.min) } else { 0.0 };
        let kx = tx as f32 + t.clamp(0.0, 1.0) * tw as f32;
        let radius = ui.pt(ui::CELL_RADIUS_PT);
        s.fill_rrect(kx - radius, ty as f32 - radius, 2.0 * radius, 2.0 * radius, radius, ui::TINT);
    }
}

/// The Export As panel: a heading over each group, then its segmented buttons. The chosen segment is tinted and a
/// disabled one is drawn dim.
fn draw_export_panel(s: &mut Surface, ui: &Ui, lay: &Layout, panel: &export_panel::Panel) {
    let w = lay.inspector;
    if w <= 0 {
        return;
    }
    let (x, top) = (lay.w - w, lay.bar);
    fill(s, x, top, w, lay.h - top, ui::SIDEBAR_BG.now());
    fill(s, x, top, 1, lay.h - top, ui::SEPARATOR_C.now());
    let pad = ui.pt(ui::SPACE_S) as i32;
    for (y, title) in &panel.headings {
        let img = ui.text(title, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL.now());
        s.blit_colored(&img, x + pad, *y, ui::LABEL_2.now(), 255);
    }
    for b in &panel.buttons {
        let (background, text) = match (b.enabled, b.active) {
            (false, _) => (ui::FIELD_BG.now(), ui::LABEL_2.now()),
            (true, true) => (ui::TINT, ui::WHITE),
            (true, false) => (ui::FIELD_BG.now(), ui::LABEL.now()),
        };
        s.fill_rrect(b.x as f32, b.y as f32, b.w as f32, b.h as f32, ui.pt(ui::SPACE_S), background);
        let img = ui.text(b.label, ui::MAC_FOOTNOTE_PT, Weight::Regular, text);
        let tx = b.x + (b.w - img.w as i32) / 2;
        let ty = b.y + (b.h - img.h as i32) / 2;
        s.blit_colored(&img, tx, ty, text, 255);
    }
}

/// The signature picker: one cell per saved signature, each showing the signature's strokes. With no signature saved, a
/// line says how to save one.
fn draw_signature_panel(s: &mut Surface, ui: &Ui, lay: &Layout, library: &[Signature]) {
    let w = lay.inspector;
    if w <= 0 {
        return;
    }
    let (x, top) = (lay.w - w, lay.bar);
    fill(s, x, top, w, lay.h - top, ui::SIDEBAR_BG.now());
    fill(s, x, top, 1, lay.h - top, ui::SEPARATOR_C.now());
    let metrics = export_metrics(ui, lay);
    if library.is_empty() {
        let lines = ["No saved signatures.", "Draw one with the Sign tool, then choose Save Signature."];
        for (i, line) in lines.iter().enumerate() {
            let text = fit_text(ui, line, ui::MAC_FOOTNOTE_PT, (w - 2 * metrics.pad).max(0) as f32);
            let img = ui.text(&text, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL.now());
            s.blit_colored(&img, x + metrics.pad, top + metrics.pad + i as i32 * (img.h as i32 + metrics.gap), ui::LABEL_2.now(), 255);
        }
    }
    for cell in signature_panel::cells(&metrics, library.len()) {
        s.fill_rrect(cell.x as f32, cell.y as f32, cell.w as f32, cell.h as f32, ui.pt(ui::SPACE_S), ui::CARD_BG.now());
        for stroke in signature_panel::strokes_in(&library[cell.index], &cell) {
            draw_path(s, &stroke, ui::LABEL.now());
        }
    }
}

/// The Inspector: a panel along the right edge listing each fact as a small label over its value.
fn draw_inspector(s: &mut Surface, ui: &Ui, lay: &Layout, rows: &[(String, String)]) {
    let w = lay.inspector;
    if w <= 0 {
        return;
    }
    let (x, top) = (lay.w - w, lay.bar);
    fill(s, x, top, w, lay.h - top, ui::SIDEBAR_BG.now());
    fill(s, x, top, 1, lay.h - top, ui::SEPARATOR_C.now());
    let pad = ui.pt(ui::SPACE_S) as i32;
    let mut y = top + pad;
    for (label, value) in rows {
        if y > lay.h {
            break;
        }
        let name = ui.text(label, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::LABEL.now());
        s.blit_colored(&name, x + pad, y, ui::LABEL_2.now(), 255);
        y += name.h as i32;
        let text = fit_text(ui, value, ui::MAC_BODY_PT, (w - 2 * pad).max(0) as f32);
        let img = ui.text(&text, ui::MAC_BODY_PT, Weight::Regular, ui::LABEL.now());
        s.blit_colored(&img, x + pad, y, ui::LABEL.now(), 255);
        y += img.h as i32 + pad;
    }
}

/// Strokes a path of canvas points: a 2 pixel dot at every step along each segment.
fn draw_path(s: &mut Surface, pts: &[(f32, f32)], color: u32) {
    for pair in pts.windows(2) {
        let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
        let steps = (bx - ax).abs().max((by - ay).abs()).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let t = k as f32 / steps as f32;
            let x = (ax + (bx - ax) * t) as i32;
            let y = (ay + (by - ay) * t) as i32;
            fill(s, x - 1, y - 1, 2, 2, color);
        }
    }
}

/// Strokes a closed outline of canvas points: the path and the last segment back to its first point.
fn draw_closed(s: &mut Surface, pts: &[(f32, f32)], color: u32) {
    let Some(&first) = pts.first() else { return };
    let mut closed = pts.to_vec();
    closed.push(first);
    draw_path(s, &closed, color);
}

/// Draws a form widget over the page: a text field shows its text on paper, a check box its tick when it is on. The
/// focused field's border takes the mark blue.
#[allow(clippy::too_many_arguments)]
fn draw_widget(s: &mut Surface, ui: &Ui, p: &Placed, quarters: u8, w: &FormWidget, value: &str, on: bool, focused: bool) {
    let (x, y, bw, bh) = to_screen(p, quarters, w.rect);
    let (x, y, bw, bh) = (x as i32, y as i32, bw.max(1.0) as i32, bh.max(1.0) as i32);
    fill(s, x, y, bw, bh, rgba(255, 255, 255, 255));
    let border = if focused { MARK_BLUE } else { ui::SEPARATOR_C.now() };
    s.stroke_rrect(x as f32, y as f32, bw as f32, bh as f32, 0.0, 1.0, border);
    match w.kind {
        Kind::Text => {
            if !value.is_empty() {
                let img = ui.text(value, ui::MAC_CALLOUT_PT, Weight::Regular, ui::WHITE);
                s.blit_colored(&img, x + ui.pt(ui::SPACE_S) as i32, y + (bh - img.h as i32) / 2, rgba(0, 0, 0, 255), 255);
            }
        }
        Kind::Check => {
            if on {
                fill(s, x + 3, y + 3, (bw - 6).max(1), (bh - 6).max(1), MARK_BLUE);
            }
        }
    }
}

fn draw_mark(s: &mut Surface, p: &Placed, quarters: u8, m: &Mark) {
    if m.kind.is_outline() {
        let pts: Vec<(f32, f32)> = model::outline(m.kind, m.rect).iter().map(|&(u, v)| screen_point(p, quarters, u, v)).collect();
        draw_closed(s, &pts, m.color);
        return;
    }
    // a text box is drawn with the notes, in the text's own font
    if m.kind == MarkKind::Text {
        return;
    }
    if m.kind.has_points() {
        // a stroke, a line or an arrow
        let pts: Vec<(f32, f32)> = m.ink.iter().map(|&(u, v)| screen_point(p, quarters, u, v)).collect();
        draw_path(s, &pts, m.color);
        if m.kind == MarkKind::Arrow {
            if let [.., from, to] = pts.as_slice() {
                if let Some([a, b]) = model::arrow_head(*from, *to, 1.0) {
                    draw_path(s, &[a, *to, b], m.color);
                }
            }
        }
        return;
    }
    let (x, y, w, h) = to_screen(p, quarters, m.rect);
    if matches!(m.kind, MarkKind::Underline | MarkKind::Strike) {
        let line = if m.kind == MarkKind::Underline { y + h } else { y + h / 2.0 };
        draw_path(s, &[(x, line), (x + w, line)], m.color);
        return;
    }
    let (x, y, w, h) = (x as i32, y as i32, w.max(1.0) as i32, h.max(1.0) as i32);
    match m.kind {
        MarkKind::Highlight => {
            // the highlight colour as a premultiplied tile, blended source-over
            let c = HIGHLIGHT;
            let hl = rgba((c & 0xff) as u8, ((c >> 8) & 0xff) as u8, ((c >> 16) & 0xff) as u8, model::HIGHLIGHT_ALPHA);
            let px = vec![hl; (w.max(1) * h.max(1)) as usize];
            let tile = Image { w: w.max(1) as usize, h: h.max(1) as usize, px };
            s.blit(&tile, x, y, 255, 255);
        }
        MarkKind::Rect => s.stroke_rrect(x as f32, y as f32, w as f32, h as f32, 0.0, 2.0, m.color),
        MarkKind::Rounded => {
            let r = (w.min(h) as f32) * 0.2;
            s.stroke_rrect(x as f32, y as f32, w as f32, h as f32, r, 2.0, m.color);
        }
        MarkKind::Note => {
            fill(s, x, y, 14, 14, MARK_BLUE);
        }
        MarkKind::Oval => draw_path(s, &model::ellipse_points(x as f32, y as f32, w as f32, h as f32), m.color),
        // compliance-allow: COL-03 opaque black is required to permanently conceal redacted picture content.
        MarkKind::Redact => fill(s, x, y, w, h, ui::BLACK),
        MarkKind::Ink | MarkKind::Line | MarkKind::Arrow | MarkKind::Underline | MarkKind::Strike => {}
        MarkKind::Star | MarkKind::Polygon | MarkKind::Bubble | MarkKind::Text => {}
    }
}

fn draw_sidebar(s: &mut Surface, ui: &Ui, lay: &Layout, scene: &Scene) {
    if lay.side <= 0 {
        return;
    }
    let Some(doc) = scene.doc else {
        fill(s, 0, lay.bar, lay.side, lay.h - lay.bar, ui::SIDEBAR_BG.now());
        return;
    };
    match scene.side {
        Side::Pages => {
            fill(s, 0, lay.bar, lay.side, lay.h - lay.bar, ui::SIDEBAR_BG.now());
            draw_pages(s, ui, lay, scene, doc);
        }
        side => {
            let scale = ui.pt(1.0).max(0.001);
            let bounds = bifrost_app::controls::Rect {
                x: 0.0,
                y: lay.bar as f32 / scale,
                w: lay.side as f32 / scale,
                h: (lay.h - lay.bar) as f32 / scale,
            };
            let list = source_list(doc, side, scene.found, scene.view.page);
            widgets::source_list(s, ui, bounds, &list, scene.view.sidebar_scroll / scale, None);
        }
    }
}

fn draw_pages(s: &mut Surface, ui: &Ui, lay: &Layout, scene: &Scene, doc: &Document) {
    let rows = rows(ui, doc);
    let pad = ui.pt(ui::SPACE_S) as i32;
    for (i, &(top, h)) in rows.iter().enumerate() {
        let y = lay.bar + top - scene.view.sidebar_scroll as i32;
        if y + h < lay.bar || y > lay.h {
            continue;
        }
        if i == scene.view.page {
            // The selected thumbnail uses the same neutral plate as a selected source-list row.
            s.fill_rrect(pad as f32 / 2.0, (y + pad / 2) as f32, (lay.side - pad) as f32, (h - pad) as f32, ui.pt(ui::SPACE_S), ui::MATERIAL_SELECTION_INACTIVE.now());
        }
        // only the thumbnails of visible rows are drawn; each is kept after its first draw
        let (bw, bh) = doc.thumb_box(i).unwrap_or((THUMB_W, (THUMB_W as f32 * 1.3) as usize));
        match doc.thumb(i) {
            Some(t) => {
                let x = (lay.side - t.w as i32) / 2;
                s.blit(&t, x, y + pad, 255, 255);
            }
            None => {
                let x = (lay.side - bw as i32) / 2;
                fill(s, x, y + pad, bw as i32, bh as i32, ui::CARD_BG.now());
            }
        }
        let box_h = bh as i32;
        let text = format!("{}", i + 1);
        let img = ui.text(&text, ui::MAC_FOOTNOTE_PT, Weight::Regular, ui::WHITE);
        let tw = ui.measure(&text, ui::MAC_FOOTNOTE_PT, Weight::Regular) as i32;
        s.blit_colored(&img, (lay.side - tw) / 2, y + pad + box_h + pad / 4, ui::LABEL_2.now(), 255);
    }
}

/// `text`, shortened with an ellipsis until it fits `max_w` points wide.
fn fit_text(ui: &Ui, text: &str, pt: f32, max_w: f32) -> String {
    if ui.measure(text, pt, Weight::Regular) <= max_w {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if ui.measure(&candidate, pt, Weight::Regular) <= max_w {
            return candidate;
        }
    }
    String::new()
}

fn draw_toolbar(s: &mut Surface, ui: &Ui, lay: &Layout, scene: &Scene) {
    // View > Hide Toolbar leaves no bar to draw
    if lay.bar <= 0 {
        return;
    }
    let scale = ui.pt(1.0).max(0.001);
    widgets::toolbar_band(
        s,
        ui,
        bifrost_app::controls::Rect { x: 0.0, y: 0.0, w: lay.w as f32 / scale, h: lay.bar as f32 / scale },
    );
    let (bar, _) = toolbar(ui, lay, scene.doc.is_some(), scene.tool, scene.side);
    widgets::toolbar(
        s,
        ui,
        bifrost_app::controls::Rect {
            x: lay.side as f32 / scale,
            y: 0.0,
            w: (lay.w - lay.side).max(0) as f32 / scale,
            h: lay.bar as f32 / scale,
        },
        &bar,
        None,
    );
}

fn centred_text(s: &mut Surface, ui: &Ui, text: &str, lay: &Layout, color: u32) {
    let (cx, cy, cw, ch) = lay.canvas();
    let img = ui.text(text, ui::MAC_BODY_PT, Weight::Regular, ui::WHITE);
    let x = cx + (cw - img.w as i32) / 2;
    let y = cy + ch / 2;
    s.blit_colored(&img, x.max(cx), y, color, 255);
}

/// A plain fill clipped to the surface.
fn fill(s: &mut Surface, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w).min(s.w as i32);
    let y1 = (y + h).min(s.h as i32);
    if x1 > x0 && y1 > y0 {
        s.fill_rect(x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_ids_round_trip_and_are_unique() {
        for (i, a) in ACTIONS.iter().enumerate() {
            assert_eq!(a.id() as usize, i);
            assert_eq!(Action::from_id(a.id()), Some(*a));
        }
        assert_eq!(Action::from_id(999), None);
    }

    #[test]
    fn every_toolbar_button_is_a_menu_action() {
        for a in Action::toolbar() {
            assert!(ACTIONS.contains(a), "{a:?}");
        }
    }

    #[test]
    fn edit_and_go_actions_are_menu_only_and_keep_the_old_ids() {
        assert_eq!(Action::Print.id(), 31);
        assert_eq!(Action::Undo.id(), 32);
        for a in [Action::Undo, Action::Redo, Action::GoBack, Action::GoToPage, Action::AddBookmark, Action::ToggleSidebar] {
            assert!(!Action::toolbar().contains(&a), "{a:?}");
            assert_eq!(Action::from_id(a.id()), Some(a));
        }
    }

    #[test]
    fn jpeg_qualities_get_lower_from_high_to_low() {
        let q = |a: Action| a.jpeg_quality().expect("a quality action");
        assert!(q(Action::QualityHigh) > q(Action::QualityMedium) && q(Action::QualityMedium) > q(Action::QualityLow));
        assert_eq!(Action::ExportJpeg.jpeg_quality(), None);
        assert!(ACTIONS.contains(&Action::AdjustSize));
    }

    #[test]
    fn export_sizes_have_their_scales() {
        assert_eq!(Action::ExportAt50.export_scale(), Some(0.5));
        assert_eq!(Action::ExportPng.export_scale(), None);
    }

    #[test]
    fn placed_picture_is_centred_and_pan_is_clamped() {
        let p = place((160, 48, 800, 600), 400, 300, 1.0, (0.0, 0.0));
        assert_eq!((p.x, p.y), (160 + 200, 48 + 150));
        // a 2000 wide picture can move at most half its overhang each way
        let p = place((0, 0, 800, 600), 2000, 300, 1.0, (5000.0, 0.0));
        assert_eq!(p.pan.0, 600.0);
        assert_eq!(p.x, 0 + (800 - 2000) / 2 + 600);
        let p = place((0, 0, 800, 600), 100, 100, 1.0, (50.0, 50.0));
        assert_eq!(p.pan, (0.0, 0.0));
    }

    #[test]
    fn canvas_point_maps_back_through_a_turn() {
        let p = place((0, 0, 100, 100), 100, 50, 1.0, (0.0, 0.0));
        // a point at the top-left of the turned canvas is the bottom-left of the source page for one turn
        let (u, v) = to_source(&p, 1, p.x as f32, p.y as f32);
        assert!((u - 0.0).abs() < 1e-5 && (v - 1.0).abs() < 1e-5, "got {u} {v}");
    }

    #[test]
    fn sidebar_hit_uses_scroll_and_ignores_the_toolbar() {
        let lay = Layout { w: 600, h: 400, bar: 40, side: 150, inspector: 0 };
        let rows = vec![(8, 100), (116, 100)];
        assert_eq!(sidebar_hit(&lay, &rows, 0.0, 10.0, 60.0), Some(0));
        assert_eq!(sidebar_hit(&lay, &rows, 100.0, 10.0, 60.0), Some(1));
        assert_eq!(sidebar_hit(&lay, &rows, 0.0, 10.0, 20.0), None);
        assert_eq!(sidebar_hit(&lay, &rows, 0.0, 200.0, 60.0), None);
        assert_eq!(rows_height(&rows), 216);
    }

    #[test]
    fn the_inspector_takes_room_from_the_canvas_and_the_slideshow_takes_none() {
        let lay = Layout { w: 800, h: 600, bar: 40, side: 150, inspector: 200 };
        assert_eq!(lay.canvas(), (150, 40, 450, 560));
        assert_eq!(Layout::bare(800, 600).canvas(), (0, 0, 800, 600));
    }

    #[test]
    fn the_sheet_fits_columns_centres_them_and_finds_the_cell_under_a_point() {
        let sheet = Sheet::new(700, 200, 100);
        assert_eq!((sheet.cols, sheet.left), (3, 50));
        assert_eq!(sheet.origin(0), (50, 0));
        assert_eq!(sheet.origin(4), (250, 100));
        assert_eq!(sheet.height(7), 300);
        assert_eq!(sheet.hit(7, 0.0, 60.0, 10.0), Some(0));
        assert_eq!(sheet.hit(7, 0.0, 460.0, 10.0), Some(2));
        assert_eq!(sheet.hit(7, 0.0, 260.0, 110.0), Some(4));
        // scrolled by a row, the same point is a row further on
        assert_eq!(sheet.hit(7, 100.0, 60.0, 10.0), Some(3));
        assert_eq!(sheet.hit(7, 0.0, 10.0, 10.0), None, "left of the grid");
        assert_eq!(sheet.hit(7, 0.0, 690.0, 10.0), None, "right of the grid");
        assert_eq!(sheet.hit(7, 200.0, 460.0, 110.0), None, "past the last page");
        assert_eq!(Sheet::new(50, 200, 100).cols, 1);
    }

    #[test]
    fn the_magnifier_samples_around_the_pointer_enlarged() {
        // the pointer is 30 pixels into a picture whose top-left is at (100, 50)
        assert_eq!(loupe_sample((130.0, 80.0), (100, 50), 0, 0), (30, 30));
        assert_eq!(loupe_sample((130.0, 80.0), (100, 50), 25, -25), (40, 20));
        assert_eq!(loupe_sample((130.0, 80.0), (100, 50), -25, 0), (20, 30));
    }

    #[test]
    fn the_new_tools_make_the_marks_their_names_say() {
        assert_eq!(Tool::Oval.mark_kind(), Some(MarkKind::Oval));
        assert_eq!(Tool::Arrow.mark_kind(), Some(MarkKind::Arrow));
        assert_eq!(Tool::Underline.mark_kind(), Some(MarkKind::Underline));
        assert_eq!(Tool::Select.mark_kind(), None);
        assert_eq!(Tool::Alpha.mark_kind(), None);
        for tool in [Tool::Oval, Tool::Line, Tool::Arrow, Tool::Underline, Tool::Strike, Tool::Select, Tool::Alpha] {
            assert!(ACTIONS.contains(&Action::UseTool(tool)), "{tool:?}");
        }
    }

    #[test]
    fn the_parity_actions_keep_the_earlier_ids() {
        assert_eq!(Action::ToggleSidebar.id(), 43);
        assert_eq!(Action::Crop.id(), 44);
        for a in [Action::Crop, Action::Inspector, Action::ContactSheet, Action::NewFromClipboard, Action::UseTool(Tool::Alpha)] {
            assert!(!Action::toolbar().contains(&a), "{a:?}");
            assert_eq!(Action::from_id(a.id()), Some(a));
        }
    }

    #[test]
    fn the_search_list_skips_hits_on_deleted_pages_and_numbers_the_rest_as_shown() {
        let doc = crate::document::tests_support::pictures(3);
        let hits = vec![
            Hit { page: 2, snippet: "two".into() },
            Hit { page: 0, snippet: "zero".into() },
            Hit { page: 7, snippet: "gone".into() },
        ];
        let items = side_items(&doc, Side::Search, &hits);
        assert_eq!(items, vec![("3  two".to_string(), Some(2)), ("1  zero".to_string(), Some(0))]);
        assert!(side_items(&doc, Side::Pages, &hits).is_empty());
    }

    #[test]
    fn the_shape_text_and_redact_tools_make_their_marks_and_sit_in_the_menus() {
        assert_eq!(Tool::Star.mark_kind(), Some(MarkKind::Star));
        assert_eq!(Tool::Polygon.mark_kind(), Some(MarkKind::Polygon));
        assert_eq!(Tool::Bubble.mark_kind(), Some(MarkKind::Bubble));
        assert_eq!(Tool::Text.mark_kind(), Some(MarkKind::Text));
        assert_eq!(Tool::Redact.mark_kind(), Some(MarkKind::Redact));
        for tool in [Tool::Star, Tool::Polygon, Tool::Bubble, Tool::Text, Tool::Redact] {
            assert!(ACTIONS.contains(&Action::UseTool(tool)), "{tool:?}");
            assert!(!Action::toolbar().contains(&Action::UseTool(tool)), "{tool:?} is in the menus only");
        }
    }

    #[test]
    fn the_file_edit_settings_and_signature_actions_are_menu_only_and_round_trip() {
        let menu_only = [
            Action::Save,
            Action::RevertToSaved,
            Action::CopyText,
            Action::SelectAll,
            Action::SaveSignature,
            Action::ClearSignatures,
            Action::ToggleToolbar,
            Action::ToggleRememberPage,
            Action::ToggleContactDefault,
            Action::SetAuthor,
        ];
        for a in menu_only {
            assert!(!Action::toolbar().contains(&a), "{a:?}");
            assert_eq!(Action::from_id(a.id()), Some(a));
            assert!(!a.label().is_empty());
        }
    }

    #[test]
    fn tool_maps_to_mark_kind() {
        assert_eq!(Tool::Pan.mark_kind(), None);
        assert_eq!(Tool::Note.mark_kind(), Some(MarkKind::Note));
        assert_eq!(Tool::Rounded.mark_kind(), Some(MarkKind::Rounded));
        assert_eq!(Tool::Sign.mark_kind(), Some(MarkKind::Ink));
    }
}
