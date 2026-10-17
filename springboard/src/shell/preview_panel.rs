//! Preview (`QLPreviewPanel`): the floating panel an app opens with `ToShell::Preview` (SDK `peek::PreviewPanel`) or the
//! `quick-look` service. It is drawn on the desktop above the windows and the Dock, at the menus' layer.
//!
//! The panel shows one item at a time:
//! - a picture from the system thumbnails (`services::quick_look` asks the `thumbnails` service for it, so the panel
//!   decodes no picture itself);
//! - a PDF, rendered page by page with the render crate's PDF renderer and scrolled with the Up, Down, Page Up and Page
//!   Down keys ([`PdfView`], [`Scroll`]);
//! - a font, with a sample paragraph set in the face the file holds ([`Body::Font`]);
//! - audio or a movie, with a transport bar (play or pause, a seek track, the time) on the shell's media player, as
//!   Photos' viewer has it ([`Media`], [`Transport`]);
//! - the first lines of a text file, or a line saying there is no preview.
//!
//! The title is the headline and the URL sits under the body. Left and Right arrows, the chevron buttons and the arrow
//! keys step through the items (no wrapping, as `PreviewPanel::step`). Esc, Space and the close button hide it. A click
//! on the desktop outside the panel is left to the desktop, and the panel stays up. The shell never reads a file the app
//! did not name: only `file://` URLs are opened, and only for a fixed size limit.
//!
//! The owning app hears what the user did: `closed` and `index` events go to it through `App::service_event` (service
//! `quick-look`); an app's own `PreviewClose` is not echoed back.

use super::ui::{Ui, Weight, CARD_BG, DETAIL_BG, LABEL, LINK, MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_HEADLINE_PT, SECONDARY, SEPARATOR_C};
use super::preview_gen::{Generator, Generators};
use super::Shell;
use crate::cg::pdf::doc::Doc;
use crate::cg::pdf::draw;
use crate::cg::pdf::page::{self, Page};
use crate::font::Font;
use crate::gfx::{Image, Surface};
use crate::layout::Rect;
use crate::media::player::{Player, State};
use bifrost_core::json::Json;
use bifrost_proto::peek::{PreviewItem, MAX_ITEMS};
use bifrost_ui::preview::Registry;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

/// The providers the panel previews with (the shell's `QLPreviewProvider` registrations). Each covers a type subtree
/// from the type tree in `bifrost_ui::uttype`, so every file of that kind gets the provider's preview.
const PICTURES: &str = "desk.pictures";
const TEXT: &str = "desk.text";
const PDF: &str = "desk.pdf";
const AUDIO: &str = "desk.audio";
const VIDEO: &str = "desk.video";
const FONT: &str = "desk.font";

/// The registry the panel previews with: one provider per kind of item.
pub fn registry() -> Registry {
    let mut r = Registry::new();
    let _ = r.register(PICTURES, &["public.image"]);
    let _ = r.register(TEXT, &["public.text"]);
    let _ = r.register(PDF, &["com.adobe.pdf"]);
    let _ = r.register(AUDIO, &["public.audio"]);
    let _ = r.register(VIDEO, &["public.movie"]);
    let _ = r.register(FONT, &["public.font"]);
    r
}

/// Pictures larger than this are not decoded (the panel is a preview, not an editor).
pub const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;
/// Text previews read this many bytes at most, and show at most [`MAX_TEXT_LINES`] lines of [`MAX_LINE_CHARS`] characters.
pub const MAX_TEXT_BYTES: u64 = 16 * 1024;
pub const MAX_TEXT_LINES: usize = 24;
pub const MAX_LINE_CHARS: usize = 120;
/// A PDF is read at most this far; a longer file is not opened.
pub const MAX_PDF_BYTES: u64 = 64 * 1024 * 1024;
/// A font file is read at most this far.
pub const MAX_FONT_BYTES: u64 = 16 * 1024 * 1024;
/// The gap between two pages in a PDF, in points.
const PAGE_GAP_PT: f32 = 10.0;
/// The largest a page may be drawn, in pixels on its longer side before the renderer's own limit.
const PAGE_MAX_PX: usize = 1 << 20;
/// How far one arrow key scrolls a PDF, in points.
const SCROLL_STEP_PT: f32 = 40.0;
/// The sample paragraphs a font item shows, in the face, at [`SAMPLE_PT`] points.
const SAMPLE_LINES: [&str; 3] = [
    "The quick brown fox jumps over the lazy dog",
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789",
    "\u{C9}g get eti\u{F0} gler \u{E1}n \u{FE}ess a\u{F0} meia\u{F0} mig",
];
const SAMPLE_PT: f32 = 22.0;

/// Evdev key codes the panel reads (the same codes as the other keys in `hwkeys`).
const KEY_UP: u16 = 103;
const KEY_PAGEUP: u16 = 104;
const KEY_PAGEDOWN: u16 = 109;
const KEY_DOWN: u16 = 108;

/// What kind of preview an item gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Text,
    Pdf,
    Audio,
    Video,
    Font,
    /// a directory: its item count and size (see [`folder_summary`]), never an extension
    Folder,
    Other,
}

/// The preview kind of a file name: the registry picks the provider through the file's type (`uttype`), so a
/// subtype such as Markdown or a PNG picture takes its ancestor's provider. A name with no known type is `Other`.
pub fn kind_for(file_name: &str) -> Kind {
    let reg = registry();
    match reg.provider_for(file_name) {
        Some(PICTURES) => Kind::Image,
        Some(TEXT) => Kind::Text,
        Some(PDF) => Kind::Pdf,
        Some(AUDIO) => Kind::Audio,
        Some(VIDEO) => Kind::Video,
        Some(FONT) => Kind::Font,
        _ => Kind::Other,
    }
}

/// The preview kind of a path: a directory is a folder, anything else goes by its name ([`kind_for`]).
pub fn kind_for_path(path: &Path) -> Kind {
    if path.is_dir() {
        return Kind::Folder;
    }
    kind_for(path.file_name().and_then(|n| n.to_str()).unwrap_or(""))
}

/// Folders are summarised from at most this many entries; a larger folder reads "more than".
pub const MAX_FOLDER_SCAN: usize = 10_000;

/// The item count of a folder's direct entries and the total size of its files, scanning at most
/// [`MAX_FOLDER_SCAN`] entries. None when the folder cannot be read. The flag is true when the scan stopped early.
pub fn folder_summary(dir: &Path) -> Option<(usize, u64, bool)> {
    let entries = std::fs::read_dir(dir).ok()?;
    let (mut count, mut bytes) = (0usize, 0u64);
    for entry in entries.take(MAX_FOLDER_SCAN) {
        count += 1;
        if let Ok(meta) = entry.and_then(|e| e.metadata()) {
            if meta.is_file() {
                bytes += meta.len();
            }
        }
    }
    Some((count, bytes, count == MAX_FOLDER_SCAN))
}

/// The folder summary line: "3 items, 2.5 KB", or "More than 10000 items, ..." when the scan stopped early.
pub fn folder_text(count: usize, bytes: u64, truncated: bool) -> String {
    let items = if truncated {
        format!("More than {count} items")
    } else if count == 1 {
        "1 item".to_string()
    } else {
        format!("{count} items")
    };
    format!("{items}, {}", crate::files::fs::size_text(bytes))
}

/// The first lines of a text file, lossy UTF-8, cut to the preview's limits. Control characters other than tab are dropped.
pub fn text_lines(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    text.lines()
        .take(MAX_TEXT_LINES)
        .map(|l| l.chars().filter(|c| !c.is_control() || *c == '\t').take(MAX_LINE_CHARS).collect())
        .collect()
}

/// The path of a `file://` URL, or None for any other scheme.
pub fn local_path(url: &str) -> Option<&str> {
    url.strip_prefix("file://").filter(|p| p.starts_with('/'))
}

/// Reads at most `limit` bytes of `path`.
fn read_limited(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    std::fs::File::open(path)?.take(limit).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Playback state of a movie or sound in the panel. Pure: the player's state is copied in each frame ([`Media::tick`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Transport {
    pub playing: bool,
    pub position_ms: f64,
    pub duration_ms: f64,
}

impl Transport {
    /// The play position as a fraction of the duration, 0 while the duration is unknown.
    pub fn fraction(&self) -> f32 {
        if self.duration_ms <= 0.0 {
            return 0.0;
        }
        (self.position_ms / self.duration_ms).clamp(0.0, 1.0) as f32
    }

    /// The position in milliseconds for a fraction of the track (clamped to 0..=1).
    pub fn seek_fraction(&self, f: f32) -> f64 {
        f64::from(f.clamp(0.0, 1.0)) * self.duration_ms.max(0.0)
    }

    /// The state with the play and pause control flipped.
    pub fn toggled(self) -> Self {
        Transport { playing: !self.playing, ..self }
    }

    /// `m:ss`, or `h:mm:ss` from an hour on.
    pub fn clock(ms: f64) -> String {
        let secs = (ms.max(0.0) / 1000.0).floor() as u64;
        let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
        if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m}:{s:02}")
        }
    }

    /// `position / duration`, as the transport bar shows it.
    pub fn label(&self) -> String {
        format!("{} / {}", Self::clock(self.position_ms), Self::clock(self.duration_ms))
    }
}

/// The fraction of `track` at the pointer's `x`, clamped to 0..=1.
pub fn track_fraction(track: Rect, x: f32) -> f32 {
    if track.w <= 0 {
        return 0.0;
    }
    ((x - track.x as f32) / track.w as f32).clamp(0.0, 1.0)
}

/// The play button and the seek track inside a transport strip `t`, in points scaled by `k`.
pub fn transport_parts(t: Rect, k: f32) -> (Rect, Rect) {
    let play_w = 28.0 * k;
    let time_w = 112.0 * k;
    let play = rect(t.x as f32, t.y as f32, play_w, t.h as f32);
    let track_x = t.x as f32 + play_w + 8.0 * k;
    let track_w = (t.x + t.w) as f32 - time_w - track_x;
    let track = rect(track_x, t.y as f32 + t.h as f32 / 2.0 - 2.0 * k, track_w.max(0.0), 4.0 * k);
    (play, track)
}

/// A movie or sound open in the panel: the shell's player, its transport, and the last video frame.
pub struct Media {
    player: Player,
    pub transport: Transport,
    frame: Option<Image>,
    video: bool,
}

impl Media {
    /// Wraps an open player. It starts paused, unlike Photos' viewer: the panel waits for the user.
    fn new(mut player: Player, video: bool) -> Self {
        player.pause();
        let mut m = Media { player, transport: Transport::default(), frame: None, video };
        m.tick();
        m
    }

    /// Reads the player into the transport and, for a movie, takes the newest frame.
    pub fn tick(&mut self) {
        if self.video {
            if let Some(f) = self.player.take_frame() {
                self.frame = Some(f);
            }
        }
        self.transport = Transport {
            playing: matches!(self.player.state(), State::Playing | State::Buffering { .. }),
            position_ms: self.player.position_ms(),
            duration_ms: self.player.duration_ms().unwrap_or(0.0),
        };
    }

    /// The newest video frame, when the item is a movie that has decoded one.
    pub fn frame_ref(&self) -> Option<&Image> {
        self.frame.as_ref()
    }

    /// The play and pause control.
    pub fn toggle(&mut self) {
        if self.transport.playing {
            self.player.pause();
        } else {
            self.player.play();
        }
        self.transport = self.transport.toggled();
    }

    /// Jumps to `f` of the track.
    pub fn seek_fraction(&mut self, f: f32) {
        let ms = self.transport.seek_fraction(f);
        self.player.seek_ms(ms);
        self.transport.position_ms = ms;
    }
}

/// The vertical layout of a PDF: each page's height in pixels and where it starts, with a gap between pages. The offset
/// is how far the document is scrolled down. Pure, so the scroll model is testable without a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scroll {
    heights: Vec<usize>,
    tops: Vec<usize>,
    gap: usize,
    offset: usize,
}

impl Scroll {
    /// Lays out pages of the given heights, `gap` pixels apart, scrolled to the top.
    pub fn new(heights: Vec<usize>, gap: usize) -> Self {
        let mut tops = Vec::with_capacity(heights.len());
        let mut y = 0usize;
        for h in &heights {
            tops.push(y);
            y += h + gap;
        }
        Scroll { heights, tops, gap, offset: 0 }
    }

    pub fn len(&self) -> usize {
        self.heights.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heights.is_empty()
    }

    /// The gap between two pages in pixels.
    pub fn gap(&self) -> usize {
        self.gap
    }

    /// The height of the whole document, without the gap after the last page.
    pub fn content_height(&self) -> usize {
        match self.tops.last() {
            Some(&t) => t + self.heights[self.heights.len() - 1],
            None => 0,
        }
    }

    /// How far the document is scrolled, in pixels.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// The top of page `i` in the document, in pixels.
    pub fn top(&self, i: usize) -> Option<usize> {
        self.tops.get(i).copied()
    }

    /// The most the document can scroll in a viewport of `viewport` pixels.
    pub fn max_offset(&self, viewport: usize) -> usize {
        self.content_height().saturating_sub(viewport)
    }

    /// Scrolls by `delta` pixels, held between the top and the bottom. Returns whether the offset moved.
    pub fn scroll_by(&mut self, delta: isize, viewport: usize) -> bool {
        let max = self.max_offset(viewport) as isize;
        let next = (self.offset as isize + delta).clamp(0, max) as usize;
        let moved = next != self.offset;
        self.offset = next;
        moved
    }

    /// The pages that show in a viewport of `viewport` pixels at the current offset, first to last.
    pub fn visible(&self, viewport: usize) -> std::ops::Range<usize> {
        let (lo, hi) = (self.offset, self.offset + viewport);
        let n = self.heights.len();
        let first = (0..n).find(|&i| self.tops[i] + self.heights[i] > lo).unwrap_or(n);
        let end = (first..n).find(|&i| self.tops[i] >= hi).unwrap_or(n);
        first..end
    }
}

/// A PDF open in the panel. The pages are drawn at the body's width, when they are shown, and kept only while visible.
pub struct PdfView {
    doc: Doc,
    pages: Vec<Page>,
    width: usize,
    scroll: Scroll,
    /// rendered pages by index; None for a page the renderer could not draw (not tried again)
    cache: HashMap<usize, Option<Image>>,
}

impl PdfView {
    /// Opens the first [`MAX_PDF_BYTES`] of a PDF. A file with no pages, or one that does not parse, is an error.
    pub fn open(path: &Path) -> Result<PdfView, String> {
        let data = read_limited(path, MAX_PDF_BYTES).map_err(|e| e.to_string())?;
        let doc = Doc::open(data)?;
        let pages = page::pages(&doc);
        if pages.is_empty() {
            return Err("The document has no pages.".into());
        }
        Ok(PdfView { doc, pages, width: 0, scroll: Scroll::default(), cache: HashMap::new() })
    }

    pub fn count(&self) -> usize {
        self.pages.len()
    }

    pub fn scroll(&self) -> &Scroll {
        &self.scroll
    }

    /// Fits every page to `width` pixels. A new width starts again from the top.
    pub fn layout(&mut self, width: usize, gap: usize) {
        if width == self.width && !self.scroll.is_empty() {
            return;
        }
        self.width = width;
        self.cache.clear();
        let heights = self.pages.iter().map(|p| draw::fit_size(p, width, PAGE_MAX_PX).map_or(0, |(_, h)| h)).collect();
        self.scroll = Scroll::new(heights, gap);
    }

    pub fn scroll_by(&mut self, delta: isize, viewport: usize) -> bool {
        self.scroll.scroll_by(delta, viewport)
    }

    /// Renders the pages that show in a viewport of `viewport` pixels and drops the others.
    pub fn prepare(&mut self, viewport: usize) {
        let range = self.scroll.visible(viewport);
        self.cache.retain(|i, _| range.contains(i));
        for i in range {
            if self.cache.contains_key(&i) {
                continue;
            }
            let img = draw::render_page_fit(&self.doc, &self.pages[i], self.width, PAGE_MAX_PX);
            self.cache.insert(i, img);
        }
    }

    /// The rendered picture of page `i`, when it is drawn.
    pub fn page_image(&self, i: usize) -> Option<&Image> {
        self.cache.get(&i).and_then(|o| o.as_ref())
    }
}

/// The rows `from..to` of `img`, as a picture of their own.
fn crop_rows(img: &Image, from: usize, to: usize) -> Image {
    let mut out = Image::new(img.w, to - from);
    out.px.copy_from_slice(&img.px[from * img.w..to * img.w]);
    out
}

/// What the body of the panel shows for the current item.
#[derive(Default)]
pub enum Body {
    #[default]
    Empty,
    /// waiting for the thumbnails service's picture of the file
    Loading,
    Image(Image),
    Text(Vec<String>),
    Pdf(Box<PdfView>),
    /// a font file's bytes, leaked once per path (see [`Panel::font_data`]); the sample is set in its face
    Font(&'static [u8]),
    Media(Box<Media>),
    Info(String),
}

/// Loads the body for one item that needs no player: a picture waits for the thumbnail service, a text file is read,
/// and the rest get their line. Never fails, a failure is an [`Body::Info`] line. PDFs, fonts and media are opened by
/// [`Panel`] itself (see `Panel::reload`).
pub fn load_body(item: Option<&PreviewItem>) -> Body {
    let Some(item) = item else { return Body::Empty };
    let Some(path) = local_path(&item.url) else { return Body::Info("No preview for this item.".into()) };
    let p = Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    match kind_for_path(p) {
        Kind::Image => Body::Loading,
        Kind::Pdf | Kind::Audio => fallback_body(item),
        Kind::Folder => match folder_summary(p) {
            Some((count, bytes, truncated)) => Body::Info(folder_text(count, bytes, truncated)),
            None => Body::Info("This folder cannot be read.".into()),
        },
        Kind::Video => Body::Info("Video file. Open it in a video player to watch it.".into()),
        Kind::Font => Body::Info(format!("Font file {name}. Install it to use the face in documents.")),
        Kind::Text => match read_limited(p, MAX_TEXT_BYTES) {
            Ok(buf) => Body::Text(text_lines(&buf)),
            Err(_) => Body::Info("This file cannot be read.".into()),
        },
        Kind::Other => Body::Info(if ext.is_empty() { "No preview for this file.".into() } else { format!("No preview for .{ext} files.") }),
    }
}

/// The body for an item whose thumbnail has only an icon: which app plays or reads it, or that it has no preview.
pub fn fallback_body(item: &PreviewItem) -> Body {
    let name = local_path(&item.url).and_then(|p| Path::new(p).file_name()).and_then(|n| n.to_str()).unwrap_or("");
    Body::Info(match kind_for(name) {
        Kind::Pdf => "PDF document. Open it in Books to read it.".into(),
        Kind::Audio => "Audio file. Open it in Music to play it.".into(),
        Kind::Video => "Video file. Open it in a video player to watch it.".into(),
        Kind::Image => "This picture cannot be shown.".into(),
        _ => "No preview for this file.".into(),
    })
}

/// The panel's state. Lives in the shell; see the module docs.
#[derive(Default)]
pub struct Panel {
    items: Vec<PreviewItem>,
    index: usize,
    /// the app that opened the panel (its events go to it)
    owner: Option<String>,
    visible: bool,
    body: Body,
    /// the local file whose thumbnail the body waits for; taken once by the `quick-look` service, which asks for it
    wanted: Option<String>,
    /// events for the owner, drained by the `quick-look` service's tick: (event name, body)
    events: Vec<(&'static str, Json)>,
    dirty: bool,
    /// the font sample's face, built for the current scale (see [`Panel::prepare`])
    face: Option<(f32, Ui)>,
    /// the font files already read, by path: each is leaked once so its bytes can back a `Font`
    font_data: Vec<(String, &'static [u8])>,
    /// the body's height and the point scale of the last frame: the viewport a PDF scrolls in
    viewport: usize,
    scale: f32,
    /// the installed apps' preview generators (see `preview_gen`): a file of a declared type waits for its app's picture
    generators: Generators,
    /// the card's size as the user dragged it (None for the default); kept while the panel is shown
    size: Option<(f32, f32)>,
    full: bool,
    /// the edge a drag is resizing, from a press until the release
    drag: Option<Grip>,
}

impl Panel {
    /// Shows `items` from `index` (clamped), opened by `owner`. An empty list hides the panel.
    pub fn show(&mut self, owner: &str, items: Vec<PreviewItem>, index: usize) {
        if items.is_empty() {
            self.hide();
            return;
        }
        self.items = items.into_iter().take(MAX_ITEMS).collect();
        self.index = index.min(self.items.len() - 1);
        self.owner = Some(owner.to_string());
        self.visible = true;
        self.reload();
    }

    /// Hides the panel. Returns whether it was shown.
    pub fn hide(&mut self) -> bool {
        let was = std::mem::replace(&mut self.visible, false);
        self.dirty = true;
        self.body = Body::Empty;
        self.face = None;
        self.drag = None;
        was
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// True once after the panel changed, so the desktop repaints the whole frame.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Moves the index by `delta` without wrapping. Returns whether it moved.
    pub fn step(&mut self, delta: i32) -> bool {
        if !self.visible {
            return false;
        }
        let next = (self.index as i64 + delta as i64).clamp(0, self.items.len() as i64 - 1) as usize;
        if next == self.index {
            return false;
        }
        self.index = next;
        self.reload();
        true
    }

    /// The user closed the panel (Esc, Space or the close button): hides it and tells the owner.
    pub fn close_by_user(&mut self) {
        if self.hide() {
            self.events.push(("closed", Json::Null));
        }
    }

    /// The user stepped: tells the owner the new index.
    pub fn user_step(&mut self, delta: i32) {
        if self.step(delta) {
            self.events.push(("index", Json::Obj(vec![("index".into(), Json::Num(self.index as f64))])));
        }
    }

    pub fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    /// The events waiting for the owner, in order.
    pub fn drain_events(&mut self) -> Vec<(&'static str, Json)> {
        std::mem::take(&mut self.events)
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    pub fn current(&self) -> Option<&PreviewItem> {
        self.items.get(self.index)
    }

    /// Whether the body has a transport strip under it (a PDF's page count, or a movie's or sound's controls).
    pub fn has_strip(&self) -> bool {
        matches!(self.body, Body::Pdf(_) | Body::Media(_))
    }

    /// Whether the open item is a PDF, which the arrow keys scroll.
    pub fn is_pdf(&self) -> bool {
        matches!(self.body, Body::Pdf(_))
    }

    /// The card's layout for the open item: the strip, the size the user dragged it to and full-screen.
    pub fn layout(&self) -> Layout {
        Layout { strip: self.has_strip(), size: self.size, full: self.full }
    }

    /// The full-screen button: the card fills the screen, or goes back to its size.
    pub fn toggle_full(&mut self) {
        self.full = !self.full;
        self.drag = None;
        self.dirty = true;
    }

    /// A press on an edge: the drag resizes the card from now on. A full-screen card has no edges to drag.
    pub fn begin_drag(&mut self, grip: Grip) {
        if !self.full {
            self.drag = Some(grip);
        }
    }

    pub fn end_drag(&mut self) -> bool {
        self.drag.take().is_some()
    }

    /// The pointer moved during a drag: the card follows it. Returns false when no drag is on, so the pointer passes on.
    pub fn drag_to(&mut self, x: f32, y: f32, screen: (f32, f32), current: (f32, f32), k: f32) -> bool {
        let Some(grip) = self.drag else { return false };
        let size = size_for_drag(grip, x, y, screen, current, k);
        if self.size != Some(size) {
            self.size = Some(size);
            self.dirty = true;
        }
        true
    }

    /// The card's size on screen now: the size the user dragged it to, or the default (for the drag's start).
    pub fn current_size(&self, k: f32) -> (f32, f32) {
        self.size.unwrap_or((DEFAULT_CARD_PT.0 * k, DEFAULT_CARD_PT.1 * k))
    }

    /// `{visible, index, count}` for the `state` method.
    pub fn state_json(&self) -> Json {
        Json::Obj(vec![
            ("visible".into(), Json::Bool(self.visible)),
            ("index".into(), Json::Num(self.index as f64)),
            ("count".into(), Json::Num(self.items.len() as f64)),
        ])
    }

    /// The local file whose picture the current body waits for, once. The service asks the thumbnails service for it.
    pub fn take_wanted(&mut self) -> Option<String> {
        self.wanted.take()
    }

    /// The thumbnails service's answer for `file`: its picture, or None when the file has none. It applies only while
    /// the current item is that file and its body waits. True when the body changed.
    pub fn thumbnail(&mut self, file: &str, picture: Option<Image>) -> bool {
        let body = match self.current() {
            Some(item) if local_path(&item.url) == Some(file) && matches!(self.body, Body::Loading) => match picture {
                Some(im) => Body::Image(im),
                None => fallback_body(item),
            },
            _ => return false,
        };
        self.body = body;
        self.dirty = true;
        true
    }

    /// The bytes of a font file, read once per path and leaked so a [`Font`] can borrow them for the rest of the run.
    /// None when the file cannot be read or does not parse as a font.
    fn font_bytes(&mut self, path: &str) -> Option<&'static [u8]> {
        if let Some((_, d)) = self.font_data.iter().find(|(p, _)| p == path) {
            return Some(*d);
        }
        let bytes = read_limited(Path::new(path), MAX_FONT_BYTES).ok()?;
        Font::parse(&bytes)?;
        let data: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        self.font_data.push((path.to_string(), data));
        Some(data)
    }

    /// Replaces the installed apps' generators (on each `show`, so an install shows up at once).
    pub fn set_generators(&mut self, generators: Generators) {
        self.generators = generators;
    }

    /// The generator an installed app declared for the file at `path`, when one takes it.
    pub fn generator_for_path(&self, path: &str) -> Option<Generator> {
        let name = Path::new(path).file_name()?.to_str()?;
        self.generators.for_file(name).cloned()
    }

    /// A generator could not answer for `file` (no app, an error, or the deadline passed): the built-in preview of the
    /// file is shown instead. Returns true when the body changed. An image's body waits for the thumbnails service
    /// again, so the caller asks for `wanted` next.
    pub fn generator_fallback(&mut self, file: &str) -> bool {
        let waits = matches!(self.body, Body::Loading) && self.current().and_then(|i| local_path(&i.url)) == Some(file);
        if !waits {
            return false;
        }
        self.load(false);
        true
    }

    fn reload(&mut self) {
        self.load(true);
    }

    /// Loads the current item. With `generators` on, a file an app declared waits for that app's picture; otherwise the
    /// built-in preview is loaded (the fallback after a generator).
    fn load(&mut self, generators: bool) {
        self.face = None;
        self.dirty = true;
        let url = self.current().map(|i| (i.url.clone(), i.title.clone()));
        let path = self.current().and_then(|i| local_path(&i.url)).map(str::to_string);
        let item = self.current().cloned();
        let claimed = generators && path.as_deref().is_some_and(|p| self.generators.for_file(Path::new(p).file_name().and_then(|n| n.to_str()).unwrap_or("")).is_some());
        self.body = match path.as_deref() {
            Some(_) if claimed => Body::Loading,
            Some(p) => match kind_for_path(Path::new(p)) {
                Kind::Pdf => match PdfView::open(Path::new(p)) {
                    Ok(v) => Body::Pdf(Box::new(v)),
                    Err(_) => Body::Info("This document cannot be read.".into()),
                },
                Kind::Audio | Kind::Video => {
                    let video = kind_for_path(Path::new(p)) == Kind::Video;
                    match Player::open(Path::new(p)) {
                        Ok(player) => Body::Media(Box::new(Media::new(player, video))),
                        Err(_) => Body::Info("This file cannot be played.".into()),
                    }
                }
                Kind::Font => match self.font_bytes(p) {
                    Some(data) => Body::Font(data),
                    None => Body::Info(format!("Font file {} cannot be read.", Path::new(p).file_name().and_then(|n| n.to_str()).unwrap_or(""))),
                },
                _ => load_body(item.as_ref()),
            },
            None => load_body(item.as_ref()),
        };
        self.wanted = match (&self.body, url) {
            (Body::Loading, Some((u, _))) => local_path(&u).map(str::to_string),
            _ => None,
        };
    }

    /// Lays the open item out for a frame: a PDF's pages are fitted to the body and rendered where visible, a movie
    /// reads its player, and a font face is built at the frame's point scale `k`. `body` is the body rectangle.
    pub fn prepare(&mut self, body: Rect, k: f32) {
        self.viewport = body.h.max(0) as usize;
        self.scale = k;
        match &mut self.body {
            Body::Pdf(v) => {
                let pad = (8.0 * k) as usize;
                v.layout((body.w.max(0) as usize).saturating_sub(2 * pad), (PAGE_GAP_PT * k) as usize);
                v.prepare(self.viewport);
            }
            Body::Media(m) => m.tick(),
            _ => {}
        }
        let font = match &self.body {
            Body::Font(d) => Some(*d),
            _ => None,
        };
        if let Some(data) = font {
            if self.face.as_ref().map_or(true, |(s, _)| *s != k) {
                if let Some(parsed) = Font::parse(data) {
                    self.face = Some((k, Ui::new(parsed, k)));
                }
            }
        }
    }

    /// The arrow and page keys for a PDF: Up and Down scroll a line, Page Up and Page Down a page. Returns true when the
    /// key scrolled, so the caller swallows it.
    pub fn scroll_key(&mut self, code: u16) -> bool {
        let line = (SCROLL_STEP_PT * self.scale) as isize;
        let page = (self.viewport as isize * 9 / 10).max(1);
        let delta = match code {
            KEY_UP => -line,
            KEY_DOWN => line,
            KEY_PAGEUP => -page,
            KEY_PAGEDOWN => page,
            _ => return false,
        };
        let Body::Pdf(v) = &mut self.body else { return false };
        if v.scroll_by(delta, self.viewport) {
            self.dirty = true;
        }
        true
    }

    /// A press on the transport strip: the play button toggles, the track seeks. Returns true when a control took it.
    pub fn transport_press(&mut self, strip: Rect, x: f32, y: f32) -> bool {
        let Body::Media(m) = &mut self.body else { return false };
        let (play, track) = transport_parts(strip, self.scale.max(f32::EPSILON));
        if inside(play, x, y) {
            m.toggle();
        } else if inside(track, x, y) {
            m.seek_fraction(track_fraction(track, x));
        } else {
            return false;
        }
        self.dirty = true;
        true
    }
}

/// The panel's rectangles in shell pixels: the card and its hit areas. `k` is `Ui::pt(1.0)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub card: Rect,
    pub close: Rect,
    pub prev: Rect,
    pub next: Rect,
    pub body: Rect,
    /// the strip under the body: a PDF's page count, or a movie's or sound's transport (empty when there is none)
    pub transport: Rect,
    /// the strip under the body that shows the item's URL when it has a title of its own
    pub caption: Rect,
    /// the full-screen toggle, at the top right of the card
    pub full: Rect,
}

/// How the card is laid out: whether a transport strip sits under the body, the size the user dragged it to (None for
/// the default), and whether it fills the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub strip: bool,
    pub size: Option<(f32, f32)>,
    pub full: bool,
}

/// Which edge of the card a drag resizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grip {
    Horizontal,
    Vertical,
    Both,
}

/// The card's default size in points, before the user resizes it.
const DEFAULT_CARD_PT: (f32, f32) = (560.0, 440.0);
/// The smallest card the user can drag it to, in points.
const MIN_CARD_PT: (f32, f32) = (240.0, 200.0);
/// The width of an edge's grab zone, in points, on each side of the edge.
const GRIP_PT: f32 = 6.0;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x: x as i32, y: y as i32, w: w as i32, h: h as i32 }
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x as f32 && x < (r.x + r.w) as f32 && y >= r.y as f32 && y < (r.y + r.h) as f32
}

/// Where the card sits on a `w` by `h` screen: centred, with sizes from the point scale `k`. Without a transport strip.
pub fn geometry(w: f32, h: f32, k: f32) -> Geometry {
    geometry_with(w, h, k, Layout::default())
}

/// The card's rectangles for a layout. A full-screen card covers the screen; otherwise the card is centred and at most
/// the screen less a margin, whatever size the user dragged it to.
pub fn geometry_with(w: f32, h: f32, k: f32, l: Layout) -> Geometry {
    let (cw, ch, x, y) = if l.full {
        (w.max(0.0), h.max(0.0), 0.0, 0.0)
    } else {
        let (dw, dh) = l.size.unwrap_or((DEFAULT_CARD_PT.0 * k, DEFAULT_CARD_PT.1 * k));
        let (cw, ch) = (dw.min(w - 32.0 * k).max(0.0), dh.min(h - 32.0 * k).max(0.0));
        (cw, ch, (w - cw) / 2.0, ((h - ch) / 2.0).max(0.0))
    };
    let head = 36.0 * k;
    let pad = 12.0 * k;
    let foot = 22.0 * k;
    let bar = if l.strip { 36.0 * k } else { 0.0 };
    Geometry {
        card: rect(x, y, cw, ch),
        close: rect(x + 6.0 * k, y + 6.0 * k, 28.0 * k, 28.0 * k),
        full: rect(x + cw - 34.0 * k, y + 6.0 * k, 28.0 * k, 28.0 * k),
        prev: rect(x + 4.0 * k, y + ch / 2.0 - 24.0 * k, 28.0 * k, 48.0 * k),
        next: rect(x + cw - 32.0 * k, y + ch / 2.0 - 24.0 * k, 28.0 * k, 48.0 * k),
        body: rect(x + pad, y + head, cw - 2.0 * pad, (ch - head - pad - foot - bar).max(0.0)),
        transport: if l.strip { rect(x + pad, y + ch - foot - bar + 4.0 * k, cw - 2.0 * pad, bar - 8.0 * k) } else { rect(0.0, 0.0, 0.0, 0.0) },
        caption: rect(x + pad, y + ch - foot, cw - 2.0 * pad, foot),
    }
}

/// The edge a pointer press grabs, for a card that is not full-screen: a strip of [`GRIP_PT`] points along the right
/// edge, the bottom edge, or both at the corner. None elsewhere.
pub fn grip_at(card: Rect, x: f32, y: f32, k: f32) -> Option<Grip> {
    let zone = GRIP_PT * k;
    let right = (card.x + card.w) as f32;
    let bottom = (card.y + card.h) as f32;
    let on_right = x >= right - zone && x < right + zone;
    let on_bottom = y >= bottom - zone && y < bottom + zone;
    let within = x >= card.x as f32 && y >= card.y as f32 && x < right + zone && y < bottom + zone;
    match (on_right, on_bottom) {
        _ if !within => None,
        (true, true) => Some(Grip::Both),
        (true, false) => Some(Grip::Horizontal),
        (false, true) => Some(Grip::Vertical),
        _ => None,
    }
}

/// The card's size when the user drags `grip` to pointer `(x, y)`. The card stays centred on the screen, so an edge
/// follows the pointer at twice its distance from the centre. Held between the minimum and the screen less a margin.
pub fn size_for_drag(grip: Grip, x: f32, y: f32, screen: (f32, f32), current: (f32, f32), k: f32) -> (f32, f32) {
    let (w, h) = screen;
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (min_w, min_h) = (MIN_CARD_PT.0 * k, MIN_CARD_PT.1 * k);
    let max_w = (w - 32.0 * k).max(min_w);
    let max_h = (h - 32.0 * k).max(min_h);
    let nw = match grip {
        Grip::Horizontal | Grip::Both => 2.0 * (x - cx).abs(),
        Grip::Vertical => current.0,
    };
    let nh = match grip {
        Grip::Vertical | Grip::Both => 2.0 * (y - cy).abs(),
        Grip::Horizontal => current.1,
    };
    (nw.clamp(min_w, max_w), nh.clamp(min_h, max_h))
}

impl Shell {
    /// Pointer events for the open panel. Returns true when the panel takes the event (a press or release inside the
    /// card, or an action on one of its buttons); other pointer events pass on to the desktop.
    pub(super) fn quick_look_pointer(&mut self, x: f32, y: f32, up: bool) -> bool {
        if !self.quick_look.is_visible() {
            return false;
        }
        let k = self.ui.pt(1.0);
        let g = geometry_with(self.w as f32, self.h as f32, k, self.quick_look.layout());
        if !inside(g.card, x, y) {
            return false;
        }
        if up {
            self.quick_look.end_drag();
            if inside(g.close, x, y) {
                self.quick_look.close_by_user();
            } else if inside(g.full, x, y) {
                self.quick_look.toggle_full();
            } else if inside(g.prev, x, y) {
                self.quick_look.user_step(-1);
            } else if inside(g.next, x, y) {
                self.quick_look.user_step(1);
            } else {
                self.quick_look.transport_press(g.transport, x, y);
            }
        } else if let Some(grip) = grip_at(g.card, x, y, k) {
            self.quick_look.begin_drag(grip);
        }
        true
    }

    /// A release ends an edge drag, wherever it lands.
    pub(super) fn quick_look_end_drag(&mut self) {
        self.quick_look.end_drag();
    }

    /// Pointer moves while the card is dragged at an edge: it follows the pointer. Returns false when no drag is on.
    pub(super) fn quick_look_drag(&mut self, x: f32, y: f32) -> bool {
        if !self.quick_look.is_visible() {
            return false;
        }
        let k = self.ui.pt(1.0);
        let current = self.quick_look.current_size(k);
        let screen = (self.w as f32, self.h as f32);
        self.quick_look.drag_to(x, y, screen, current, k)
    }

    /// Keys for the open panel: Esc and Space close it, Left and Right step, and Up, Down, Page Up and Page Down scroll
    /// a PDF. Returns None for any other key, which falls through to the desktop. A release of a panel key is swallowed too.
    pub(super) fn quick_look_key(&mut self, code: u16, down: bool) -> Option<()> {
        const KEY_ESC: u16 = 1;
        const KEY_SPACE: u16 = 57;
        const KEY_LEFT: u16 = 105;
        const KEY_RIGHT: u16 = 106;
        if !self.quick_look.is_visible() {
            return None;
        }
        match code {
            KEY_ESC | KEY_SPACE if down => self.quick_look.close_by_user(),
            KEY_LEFT if down => self.quick_look.user_step(-1),
            KEY_RIGHT if down => self.quick_look.user_step(1),
            KEY_UP | KEY_DOWN | KEY_PAGEUP | KEY_PAGEDOWN if self.quick_look.is_pdf() => {
                if down {
                    self.quick_look.scroll_key(code);
                }
            }
            KEY_ESC | KEY_SPACE | KEY_LEFT | KEY_RIGHT => {}
            _ => return None,
        }
        Some(())
    }

    /// The app's `ToShell::Preview` or the `show` call: shows the panel and redraws.
    pub(super) fn quick_look_show(&mut self, owner: &str, items: Vec<PreviewItem>, index: usize) {
        self.quick_look.show(owner, items, index);
        self.dirty = true;
    }

    /// The app's `PreviewClose` or the `close` call: hides the panel and redraws.
    pub(super) fn quick_look_hide(&mut self) {
        if self.quick_look.hide() {
            self.dirty = true;
        }
    }

    /// The file the panel's body waits for a picture of, once (see [`Panel::take_wanted`]).
    pub(super) fn quick_look_take_wanted(&mut self) -> Option<String> {
        self.quick_look.take_wanted()
    }

    /// The installed apps' preview generators, read before the panel shows (see `Panel::set_generators`).
    pub(super) fn quick_look_set_generators(&mut self, generators: Generators) {
        self.quick_look.set_generators(generators);
    }

    /// The generator an app declared for the file at `path`, when one takes it.
    pub(super) fn quick_look_generator_for(&self, path: &str) -> Option<Generator> {
        self.quick_look.generator_for_path(path)
    }

    /// A generator could not answer for `file`: the built-in preview takes its place. True when the body now waits for
    /// a thumbnail, which the caller asks for.
    pub(super) fn quick_look_generator_fallback(&mut self, file: &str) -> bool {
        let changed = self.quick_look.generator_fallback(file);
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// A thumbnail answer for the panel: redraws when the body changed.
    pub(super) fn quick_look_thumbnail(&mut self, file: &str, picture: Option<Image>) {
        if self.quick_look.thumbnail(file, picture) {
            self.dirty = true;
        }
    }

    /// The app's `step` call: moves the index without an event back to the app.
    pub(super) fn quick_look_step(&mut self, delta: i32) {
        if self.quick_look.step(delta) {
            self.dirty = true;
        }
    }

    pub(super) fn quick_look_state(&self) -> Json {
        self.quick_look.state_json()
    }

    /// The app that opened the panel, shown or not.
    pub(super) fn quick_look_owner(&self) -> Option<&str> {
        self.quick_look.owner()
    }

    pub(super) fn quick_look_drain_events(&mut self) -> Vec<(&'static str, Json)> {
        self.quick_look.drain_events()
    }

    /// Lays out the open panel for a `w` by `h` screen before it is drawn: renders the visible PDF pages and reads the
    /// movie's player. Called once per frame from the desktop, before [`Shell::draw_quick_look`].
    pub(super) fn quick_look_prepare(&mut self, w: f32, h: f32) {
        if !self.quick_look.is_visible() {
            return;
        }
        let k = self.ui.pt(1.0);
        let g = geometry_with(w, h, k, self.quick_look.layout());
        self.quick_look.prepare(g.body, k);
    }

    /// Draws the open panel over the desktop. Does nothing when it is hidden.
    pub(super) fn draw_quick_look(&self, s: &mut Surface) {
        let p = &self.quick_look;
        if !p.is_visible() {
            return;
        }
        let ui = &self.ui;
        let g = geometry_with(s.w as f32, s.h as f32, ui.pt(1.0), p.layout());
        // a full-screen card has square corners; a window has its radius
        let radius = if p.layout().full { 0.0 } else { ui.pt(13.0) };
        let (cx, cy, cw, ch) = (g.card.x as f32, g.card.y as f32, g.card.w as f32, g.card.h as f32);
        s.fill_rrect(cx, cy, cw, ch, radius, CARD_BG.now());
        s.stroke_rrect(cx, cy, cw, ch, radius, ui.pt(1.0), SEPARATOR_C.now());

        // close button: a grey disc with a cross
        let c = g.close;
        s.fill_rrect(c.x as f32, c.y as f32, c.w as f32, c.h as f32, c.w as f32 / 2.0, DETAIL_BG.now());
        let cross = "\u{00D7}";
        self.label(cross, MAC_HEADLINE_PT, Weight::Regular, SECONDARY, |im| {
            s.blit(im, c.x + (c.w - im.w as i32) / 2, c.y + (c.h - im.h as i32) / 2, 255, 255)
        });

        // full-screen button: a grey disc with a square outline (LINK) inside it
        let f = g.full;
        s.fill_rrect(f.x as f32, f.y as f32, f.w as f32, f.h as f32, f.w as f32 / 2.0, DETAIL_BG.now());
        let box_pt = ui.pt(11.0);
        let (bxf, byf) = (f.x as f32 + (f.w as f32 - box_pt) / 2.0, f.y as f32 + (f.h as f32 - box_pt) / 2.0);
        s.stroke_rrect(bxf, byf, box_pt, box_pt, ui.pt(2.0), ui.pt(1.5), LINK);

        // title (centred, cut to fit between the buttons) and the "n of N" counter, left of the full-screen button
        let title = p.current().map(|i| if i.title.is_empty() { i.url.as_str() } else { i.title.as_str() }).unwrap_or("");
        let title = ui.fit(title, MAC_HEADLINE_PT, cw - 2.0 * ui.pt(120.0));
        self.label(&title, MAC_HEADLINE_PT, Weight::Bold, LABEL.now(), |im| {
            s.blit(im, (cx + (cw - im.w as f32) / 2.0) as i32, (cy + ui.pt(18.0) - im.h as f32 / 2.0) as i32, 255, 255)
        });
        let counter = format!("{} of {}", p.index() + 1, p.count());
        self.label(&counter, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY, |im| {
            s.blit(im, (cx + cw - ui.pt(44.0) - im.w as f32) as i32, (cy + ui.pt(18.0) - im.h as f32 / 2.0) as i32, 255, 255)
        });

        // body: a grouped plate with the preview inside
        let b = g.body;
        let (bx, by, bw, bh) = (b.x as f32, b.y as f32, b.w as f32, b.h as f32);
        s.fill_rrect(bx, by, bw, bh, ui.pt(9.0), DETAIL_BG.now());
        s.stroke_rrect(bx, by, bw, bh, ui.pt(9.0), ui.pt(1.0), SEPARATOR_C.now());
        match &p.body {
            Body::Empty | Body::Loading => {}
            Body::Image(im) => {
                let k = (bw - ui.pt(24.0)) / im.w.max(1) as f32;
                let k = k.min((bh - ui.pt(24.0)) / im.h.max(1) as f32);
                let (w, h) = (im.w as f32 * k, im.h as f32 * k);
                let r = rect(bx + (bw - w) / 2.0, by + (bh - h) / 2.0, w, h);
                super::switcher::draw_scaled(s, im, r, ui.pt(4.0), 255);
            }
            Body::Pdf(v) => {
                // each page that shows is cut to the body's rows, so nothing is drawn over the card's header or strip
                let offset = v.scroll().offset();
                for i in v.scroll().visible(b.h.max(0) as usize) {
                    let (Some(top), Some(im)) = (v.scroll().top(i), v.page_image(i)) else { continue };
                    let y0 = top as i64 - offset as i64;
                    let from = (-y0).max(0) as usize;
                    let to = (bh as i64 - y0).min(im.h as i64).max(0) as usize;
                    if to <= from {
                        continue;
                    }
                    let strip = crop_rows(im, from, to);
                    let x = bx as i32 + (b.w - im.w as i32) / 2;
                    s.blit(&strip, x, by as i32 + (y0 + from as i64) as i32, 255, 255);
                }
            }
            Body::Font(_) => {
                if let Some((_, face)) = &p.face {
                    let inner = ui.pt(16.0);
                    let mut y = by + inner;
                    for line in SAMPLE_LINES {
                        let im = face.text(line, SAMPLE_PT, Weight::Regular, LABEL.now());
                        if y + im.h as f32 > by + bh {
                            break;
                        }
                        s.blit(&im, (bx + inner) as i32, y as i32, 255, 255);
                        y += im.h as f32 + ui.pt(8.0);
                    }
                }
            }
            Body::Media(m) => {
                if let Some(frame) = m.frame_ref() {
                    let k = (bw - ui.pt(24.0)) / frame.w.max(1) as f32;
                    let k = k.min((bh - ui.pt(24.0)) / frame.h.max(1) as f32);
                    let (w, h) = (frame.w as f32 * k, frame.h as f32 * k);
                    let r = rect(bx + (bw - w) / 2.0, by + (bh - h) / 2.0, w, h);
                    super::switcher::draw_scaled(s, frame, r, ui.pt(4.0), 255);
                }
            }
            Body::Text(lines) => {
                let step = ui.pt(16.0);
                let inner = ui.pt(12.0);
                let mut y = by + inner;
                for line in lines {
                    if y + step > by + bh {
                        break;
                    }
                    let t = ui.fit(line, MAC_BODY_PT, bw - 2.0 * inner);
                    self.label(&t, MAC_BODY_PT, Weight::Regular, LABEL.now(), |im| s.blit(im, (bx + inner) as i32, y as i32, 255, 255));
                    y += step;
                }
            }
            Body::Info(text) => {
                let t = ui.fit(text, MAC_BODY_PT, bw - ui.pt(24.0));
                self.label(&t, MAC_BODY_PT, Weight::Regular, SECONDARY, |im| {
                    s.blit(im, (bx + (bw - im.w as f32) / 2.0) as i32, (by + (bh - im.h as f32) / 2.0) as i32, 255, 255)
                });
            }
        }

        // the strip under the body: a PDF's page count, or the movie's or sound's transport
        match &p.body {
            Body::Pdf(v) => {
                let t = g.transport;
                let first = v.scroll().visible(b.h.max(0) as usize).start;
                let label = format!("Page {} of {}", first + 1, v.count());
                self.label(&label, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY, |im| {
                    s.blit(im, t.x + (t.w - im.w as i32) / 2, t.y + (t.h - im.h as i32) / 2, 255, 255)
                });
            }
            Body::Media(m) => self.draw_transport(s, g.transport, &m.transport),
            _ => {}
        }

        // the URL of an item that has a title of its own, centred under the body in secondary grey
        if let Some(item) = p.current().filter(|i| !i.title.is_empty() && !i.url.is_empty()) {
            let c = g.caption;
            let url = ui.fit(&item.url, MAC_FOOTNOTE_PT, c.w as f32);
            self.label(&url, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY, |im| {
                s.blit(im, c.x + (c.w - im.w as i32) / 2, c.y + (c.h - im.h as i32) / 2, 255, 255)
            });
        }

        // the chevrons, in LINK, dimmed at the ends
        for (r, glyph, enabled) in [(g.prev, "\u{2039}", p.index() > 0), (g.next, "\u{203A}", p.index() + 1 < p.count())] {
            let color = if enabled { LINK } else { SECONDARY };
            self.label(glyph, MAC_HEADLINE_PT, Weight::Bold, color, |im| {
                s.blit(im, r.x + (r.w - im.w as i32) / 2, r.y + (r.h - im.h as i32) / 2, 255, 255)
            });
        }
    }

    /// The transport strip of a movie or sound: the play or pause control in LINK, the seek track with its progress in
    /// LINK over the separator grey, and the time on the right in secondary grey.
    fn draw_transport(&self, s: &mut Surface, strip: Rect, t: &Transport) {
        let ui = &self.ui;
        let k = ui.pt(1.0);
        let (play, track) = transport_parts(strip, k);
        let glyph = if t.playing { "\u{2016}" } else { "\u{25B6}" };
        self.label(glyph, MAC_BODY_PT, Weight::Bold, LINK, |im| {
            s.blit(im, play.x + (play.w - im.w as i32) / 2, play.y + (play.h - im.h as i32) / 2, 255, 255)
        });
        s.fill_rrect(track.x as f32, track.y as f32, track.w as f32, track.h as f32, track.h as f32 / 2.0, SEPARATOR_C.now());
        let done = track.w as f32 * t.fraction();
        if done > 0.0 {
            s.fill_rrect(track.x as f32, track.y as f32, done, track.h as f32, track.h as f32 / 2.0, LINK);
        }
        let time = t.label();
        self.label(&time, MAC_FOOTNOTE_PT, Weight::Regular, SECONDARY, |im| {
            s.blit(im, strip.x + strip.w - im.w as i32, strip.y + (strip.h - im.h as i32) / 2, 255, 255)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(url: &str, title: &str) -> PreviewItem {
        PreviewItem { url: url.into(), title: title.into() }
    }

    #[test]
    fn extensions_pick_the_preview_kind() {
        assert_eq!(kind_for("photo.png"), Kind::Image);
        assert_eq!(kind_for("PHOTO.JPG"), Kind::Image, "extensions are case-insensitive");
        assert_eq!(kind_for("notes.md"), Kind::Text, "Markdown conforms to plain text");
        assert_eq!(kind_for("main.rs"), Kind::Text);
        assert_eq!(kind_for("paper.pdf"), Kind::Pdf);
        assert_eq!(kind_for("song.mp3"), Kind::Audio);
        assert_eq!(kind_for("clip.mp4"), Kind::Video, "MPEG-4 movies conform to public.movie");
        assert_eq!(kind_for("clip.mov"), Kind::Video);
        assert_eq!(kind_for("face.ttf"), Kind::Font);
        assert_eq!(kind_for("face.otf"), Kind::Font);
        assert_eq!(kind_for("archive.zip"), Kind::Other);
        assert_eq!(kind_for("README"), Kind::Other, "no extension, no type");
    }

    #[test]
    fn a_directory_is_a_folder_and_is_summarised_by_its_entries() {
        let dir = std::env::temp_dir().join(format!("ql-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), vec![0u8; 2000]).unwrap();
        std::fs::write(dir.join("b.txt"), vec![0u8; 500]).unwrap();
        assert_eq!(kind_for_path(&dir), Kind::Folder, "a directory decides, not its name");
        assert_eq!(folder_summary(&dir), Some((3, 2500, false)), "subfolders count as items, only files add bytes");
        assert_eq!(folder_text(3, 2500, false), format!("3 items, {}", crate::files::fs::size_text(2500)));
        assert_eq!(folder_text(1, 0, false), format!("1 item, {}", crate::files::fs::size_text(0)));
        assert!(folder_text(MAX_FOLDER_SCAN, 0, true).starts_with("More than 10000 items"));
        assert_eq!(folder_summary(&dir.join("missing")), None);
        let body = load_body(Some(&item(&format!("file://{}", dir.display()), "dir")));
        assert!(matches!(body, Body::Info(ref t) if t.starts_with("3 items")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn video_and_font_items_get_an_info_line_and_no_thumbnail_wait() {
        let video = item("file:///x/clip.mp4", "clip");
        assert!(matches!(load_body(Some(&video)), Body::Info(_)));
        assert!(matches!(fallback_body(&video), Body::Info(ref t) if t.starts_with("Video file")));
        let font = item("file:///x/face.ttf", "face");
        assert!(matches!(load_body(Some(&font)), Body::Info(ref t) if t.contains("face.ttf")));
    }

    #[test]
    fn pictures_wait_for_the_thumbnails_service_and_pdfs_and_audio_get_their_line() {
        let dir = std::env::temp_dir().join(format!("ql-kinds-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("paper.pdf");
        std::fs::write(&pdf, b"%PDF-1.4").unwrap();
        let url = format!("file://{}", pdf.display());
        assert!(matches!(load_body(Some(&item(&url, "paper"))), Body::Info(t) if t.starts_with("PDF document")));
        let song = format!("file://{}/song.mp3", dir.display());
        assert!(matches!(fallback_body(&item(&song, "")), Body::Info(t) if t.starts_with("Audio file")));
        let png = format!("file://{}/pic.png", dir.display());
        assert!(matches!(load_body(Some(&item(&png, "pic"))), Body::Loading));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_pdf_that_does_not_parse_says_so_and_a_bad_font_is_not_leaked() {
        let dir = std::env::temp_dir().join(format!("ql-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("broken.pdf");
        std::fs::write(&pdf, b"%PDF-1.4 not really").unwrap();
        let url = format!("file://{}", pdf.display());
        let mut p = Panel::default();
        p.show("app", vec![item(&url, "broken")], 0);
        assert!(matches!(p.body, Body::Info(ref t) if t == "This document cannot be read."));
        let font = dir.join("face.ttf");
        std::fs::write(&font, b"nope").unwrap();
        let furl = format!("file://{}", font.display());
        p.show("app", vec![item(&furl, "face")], 0);
        assert!(matches!(p.body, Body::Info(ref t) if t.contains("cannot be read")));
        assert!(p.font_data.is_empty(), "bytes that are not a font are not kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_thumbnail_answer_fills_the_body_only_for_the_current_item() {
        let mut p = Panel::default();
        p.show("app", vec![item("file:///a.png", "a"), item("file:///b.zip", "b")], 0);
        assert_eq!(p.take_wanted().as_deref(), Some("/a.png"), "the picture is asked for once");
        assert_eq!(p.take_wanted(), None);
        assert!(!p.thumbnail("/b.zip", None), "an answer for another item is dropped");
        let im = Image::new(2, 2);
        assert!(p.thumbnail("/a.png", Some(im)));
        assert!(matches!(p.body, Body::Image(_)));
        assert!(!p.thumbnail("/a.png", None), "the body no longer waits");
        p.step(1);
        // A type with no preview of its own (not an image, PDF or audio file) is not asked for a thumbnail: its
        // line is shown at once, so nothing is waited for.
        assert_eq!(p.take_wanted(), None, "an archive is not asked for a thumbnail");
        assert!(!p.thumbnail("/b.zip", None), "no picture is waited for");
        assert!(matches!(p.body, Body::Info(t) if t == "No preview for .zip files."));
    }

    #[test]
    fn text_lines_keep_the_first_lines_and_drop_control_bytes() {
        let body = "one\ntwo\u{7}x\tt\n".to_string() + &"n\n".repeat(40);
        let lines = text_lines(body.as_bytes());
        assert_eq!(lines.len(), MAX_TEXT_LINES);
        assert_eq!(lines[0], "one");
        assert_eq!(lines[1], "twox\tt");
        let long = "a".repeat(500);
        assert_eq!(text_lines(long.as_bytes())[0].chars().count(), MAX_LINE_CHARS);
    }

    #[test]
    fn only_file_urls_are_read() {
        assert_eq!(local_path("file:///tmp/a.txt"), Some("/tmp/a.txt"));
        assert_eq!(local_path("https://example.com/a.png"), None);
        assert_eq!(local_path("file://relative"), None);
        assert!(matches!(load_body(Some(&item("https://example.com/x.txt", ""))), Body::Info(_)));
        assert!(matches!(load_body(None), Body::Empty));
    }

    #[test]
    fn text_files_load_into_the_body_and_missing_files_say_so() {
        let dir = std::env::temp_dir().join(format!("ql-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("note.txt");
        std::fs::write(&f, "hello\nworld\n").unwrap();
        let url = format!("file://{}", f.display());
        match load_body(Some(&item(&url, "note"))) {
            Body::Text(lines) => assert_eq!(lines, ["hello", "world"]),
            _ => panic!("text body expected"),
        }
        let gone = format!("file://{}/missing.txt", dir.display());
        assert!(matches!(load_body(Some(&item(&gone, ""))), Body::Info(t) if t == "This file cannot be read."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn show_clamps_the_index_and_an_empty_list_hides() {
        let mut p = Panel::default();
        p.show("is.example.files", vec![item("file:///a", "a"), item("file:///b", "b")], 9);
        assert!(p.is_visible());
        assert_eq!(p.index(), 1);
        assert_eq!(p.owner(), Some("is.example.files"));
        p.show("x", vec![], 0);
        assert!(!p.is_visible());
    }

    #[test]
    fn stepping_does_not_wrap_and_reports_moves() {
        let mut p = Panel::default();
        p.show("app", vec![item("file:///a", "a"), item("file:///b", "b"), item("file:///c", "c")], 0);
        assert!(!p.step(-1), "already at the first item");
        assert!(p.step(1));
        assert!(p.step(5));
        assert_eq!(p.index(), 2);
        assert!(!p.step(1), "no wrapping past the last item");
        assert_eq!(p.current().unwrap().title, "c");
    }

    #[test]
    fn user_actions_queue_events_for_the_owner() {
        let mut p = Panel::default();
        p.show("app", vec![item("file:///a", "a"), item("file:///b", "b")], 0);
        p.user_step(1);
        p.user_step(1); // at the end: no move, no event
        let ev = p.drain_events();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].0, "index");
        assert_eq!(ev[0].1.get("index").and_then(Json::as_u64), Some(1));
        p.close_by_user();
        p.close_by_user(); // already hidden: no second event
        let ev = p.drain_events();
        assert_eq!(ev.iter().map(|e| e.0).collect::<Vec<_>>(), ["closed"]);
        assert!(p.drain_events().is_empty());
    }

    #[test]
    fn take_dirty_is_true_once_per_change() {
        let mut p = Panel::default();
        assert!(!p.take_dirty());
        p.show("app", vec![item("file:///a", "a")], 0);
        assert!(p.take_dirty());
        assert!(!p.take_dirty());
    }

    #[test]
    fn pages_are_laid_out_one_below_another_with_a_gap() {
        let s = Scroll::new(vec![100, 100, 100], 10);
        assert_eq!(s.len(), 3);
        assert_eq!(s.top(0), Some(0));
        assert_eq!(s.top(1), Some(110));
        assert_eq!(s.top(2), Some(220));
        assert_eq!(s.top(3), None);
        assert_eq!(s.content_height(), 320, "no gap after the last page");
        assert_eq!(Scroll::default().content_height(), 0);
    }

    #[test]
    fn scrolling_is_held_between_the_top_and_the_bottom() {
        let mut s = Scroll::new(vec![100, 100, 100], 10);
        assert!(!s.scroll_by(-5, 150), "already at the top");
        assert!(s.scroll_by(200, 150));
        assert_eq!(s.offset(), 170, "held at content height minus the viewport");
        assert!(!s.scroll_by(50, 150), "already at the bottom");
        assert!(s.scroll_by(-500, 150));
        assert_eq!(s.offset(), 0);
        let mut short = Scroll::new(vec![100], 10);
        assert!(!short.scroll_by(40, 300), "a document shorter than the viewport does not scroll");
    }

    #[test]
    fn the_visible_pages_follow_the_offset() {
        let mut s = Scroll::new(vec![100, 100, 100], 10);
        assert_eq!(s.visible(150), 0..2, "pages 0 and 1 show from the top");
        s.scroll_by(200, 150);
        assert_eq!(s.visible(150), 1..3, "the first page has scrolled out");
        assert_eq!(Scroll::default().visible(100), 0..0);
        assert_eq!(Scroll::new(vec![50, 50], 0).visible(1000), 0..2);
    }

    #[test]
    fn the_transport_reads_the_position_and_the_track() {
        let t = Transport { playing: true, position_ms: 2_500.0, duration_ms: 10_000.0 };
        assert!((t.fraction() - 0.25).abs() < 1e-6);
        assert_eq!(t.seek_fraction(0.5), 5_000.0);
        assert_eq!(t.seek_fraction(3.0), 10_000.0, "a seek past the end is held at the end");
        assert_eq!(Transport::default().fraction(), 0.0, "an unknown duration shows no progress");
        assert!(!t.toggled().playing, "the play control flips to paused");
        assert!(t.toggled().toggled().playing, "two flips come back to playing");
    }

    #[test]
    fn the_transport_labels_are_clocks() {
        assert_eq!(Transport::clock(0.0), "0:00");
        assert_eq!(Transport::clock(65_000.0), "1:05");
        assert_eq!(Transport::clock(3_723_000.0), "1:02:03", "an hour shows the hours");
        assert_eq!(Transport::clock(-5.0), "0:00", "a negative position reads as the start");
        let t = Transport { playing: false, position_ms: 5_000.0, duration_ms: 60_000.0 };
        assert_eq!(t.label(), "0:05 / 1:00");
    }

    #[test]
    fn the_track_maps_a_pointer_to_a_fraction_and_the_strip_splits_in_two() {
        let track = Rect { x: 100, y: 0, w: 200, h: 4 };
        assert_eq!(track_fraction(track, 100.0), 0.0);
        assert_eq!(track_fraction(track, 200.0), 0.5);
        assert_eq!(track_fraction(track, 900.0), 1.0, "past the end is the end");
        assert_eq!(track_fraction(Rect { x: 0, y: 0, w: 0, h: 4 }, 5.0), 0.0);
        let strip = Rect { x: 0, y: 0, w: 400, h: 28 };
        let (play, track) = transport_parts(strip, 1.0);
        assert_eq!(play.x, 0);
        assert!(track.x > play.x + play.w, "the track starts after the play button");
        assert!(track.x + track.w <= strip.w - 112, "the track stops before the time");
    }

    #[test]
    fn stepping_never_moves_a_pdf_scroll_and_the_geometry_adds_a_strip() {
        let mut p = Panel::default();
        p.show("app", vec![item("file:///a.txt", "a")], 0);
        assert!(!p.scroll_key(KEY_DOWN), "a text item does not scroll");
        assert!(!p.has_strip());
        let plain = geometry(1280.0, 800.0, 1.0);
        let with = geometry_with(1280.0, 800.0, 1.0, Layout { strip: true, ..Layout::default() });
        assert!(with.body.h < plain.body.h, "the strip takes room from the body");
        assert_eq!(with.transport.h > 0, true);
        assert!(with.body.y + with.body.h <= with.transport.y, "the body sits above the strip");
        assert!(with.transport.y + with.transport.h <= with.caption.y);
    }

    #[test]
    fn a_dragged_edge_follows_the_pointer_about_the_centre() {
        let screen = (1280.0, 800.0);
        let current = (560.0, 440.0);
        // the right edge at x = 1000 is 360 from the centre (640): the card is 720 wide
        assert_eq!(size_for_drag(Grip::Horizontal, 1000.0, 400.0, screen, current, 1.0), (720.0, 440.0));
        // the bottom edge at y = 600 is 200 from the centre (400): the card is 400 tall, its width untouched
        assert_eq!(size_for_drag(Grip::Vertical, 900.0, 600.0, screen, current, 1.0), (560.0, 400.0));
        assert_eq!(size_for_drag(Grip::Both, 1000.0, 600.0, screen, current, 1.0), (720.0, 400.0));
    }

    #[test]
    fn a_drag_is_held_between_the_minimum_and_the_screen_less_a_margin() {
        let screen = (1280.0, 800.0);
        assert_eq!(size_for_drag(Grip::Horizontal, 640.0, 400.0, screen, (560.0, 440.0), 1.0).0, MIN_CARD_PT.0);
        assert_eq!(size_for_drag(Grip::Horizontal, 5000.0, 400.0, screen, (560.0, 440.0), 1.0).0, 1280.0 - 32.0);
        assert_eq!(size_for_drag(Grip::Vertical, 640.0, 400.0, screen, (560.0, 440.0), 1.0).1, MIN_CARD_PT.1, "dragged to the centre");
    }

    #[test]
    fn the_grab_zones_are_the_right_and_bottom_edges_and_the_corner() {
        let card = Rect { x: 100, y: 50, w: 400, h: 300 };
        assert_eq!(grip_at(card, 499.0, 200.0, 1.0), Some(Grip::Horizontal), "just inside the right edge");
        assert_eq!(grip_at(card, 300.0, 348.0, 1.0), Some(Grip::Vertical), "just inside the bottom edge");
        assert_eq!(grip_at(card, 499.0, 348.0, 1.0), Some(Grip::Both), "the corner takes both");
        assert_eq!(grip_at(card, 300.0, 200.0, 1.0), None, "the middle of the card is not a grip");
        assert_eq!(grip_at(card, 900.0, 900.0, 1.0), None, "outside the card");
    }

    #[test]
    fn a_full_screen_card_covers_the_screen_and_takes_no_drag() {
        let full = geometry_with(1280.0, 800.0, 1.0, Layout { full: true, ..Layout::default() });
        assert_eq!((full.card.x, full.card.y, full.card.w, full.card.h), (0, 0, 1280, 800));
        assert!(inside(full.full, (full.full.x + 1) as f32, (full.full.y + 1) as f32), "the full-screen button stays on the card");
        let mut p = Panel::default();
        p.show("app", vec![item("file:///a", "a")], 0);
        p.toggle_full();
        assert!(p.layout().full);
        p.begin_drag(Grip::Both);
        assert!(!p.drag_to(10.0, 10.0, (1280.0, 800.0), (560.0, 440.0), 1.0), "no drag while full-screen");
        p.toggle_full();
        assert!(!p.layout().full, "the button goes back to the card");
    }

    #[test]
    fn a_drag_resizes_the_panel_and_keeps_the_size_for_the_next_item() {
        let mut p = Panel::default();
        p.show("app", vec![item("file:///a", "a"), item("file:///b", "b")], 0);
        assert!(!p.drag_to(900.0, 300.0, (1280.0, 800.0), (560.0, 440.0), 1.0), "no drag, the pointer passes on");
        p.begin_drag(Grip::Horizontal);
        assert!(p.drag_to(1000.0, 300.0, (1280.0, 800.0), (560.0, 440.0), 1.0));
        assert_eq!(p.layout().size, Some((720.0, 440.0)));
        assert!(p.end_drag());
        assert!(!p.end_drag(), "a release with no drag is nothing");
        p.step(1);
        assert_eq!(p.layout().size, Some((720.0, 440.0)), "the size stays for the next item");
        p.close_by_user();
        assert!(p.layout().size.is_some(), "and for the next opening");
    }

    #[test]
    fn geometry_centres_the_card_and_keeps_buttons_inside_it() {
        let g = geometry(1280.0, 800.0, 1.0);
        assert_eq!(g.card.w, 560);
        assert_eq!(g.card.x, (1280 - 560) / 2);
        assert!(inside(g.card, 640.0, 400.0));
        assert!(inside(g.close, (g.close.x + 1) as f32, (g.close.y + 1) as f32));
        assert!(inside(g.card, g.next.x as f32, g.next.y as f32));
        let small = geometry(300.0, 300.0, 1.0);
        assert!(small.card.w <= 300 - 32, "a small screen shrinks the card");
        // the caption strip sits at the bottom of the card, below the body
        assert_eq!(g.caption.y + g.caption.h, g.card.y + g.card.h);
        assert!(g.body.y + g.body.h <= g.caption.y);
    }
}
