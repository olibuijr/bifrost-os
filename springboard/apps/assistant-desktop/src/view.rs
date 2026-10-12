//! The Assistant window, Golden Gate style: a source-list sidebar of conversations, the transcript under a unified
//! toolbar, a composer at the bottom and a Settings sheet. Geometry is in points for the kit and in pixels
//! (`ui.pt`) for drawing. Colours are `Pair` tokens resolved with `.now()`, so Light and Dark both work.

use crate::api::{self, Event, Running};
use crate::connect::{self, Link};
use crate::model::{self, Block, Conversation, Message, Span};
use crate::safety;
use crate::session;
use bifrost_app::controls::guardrail::Guardrail;
use bifrost_app::controls::text_classifier::TextClassifier;
use bifrost_app::controls::nav::{Toolbar, ToolbarItem, ToolbarLayout};
use bifrost_app::controls::sidebar::{SidebarItem, SourceList};
use bifrost_app::controls::{Node, Rect, Role};
use bifrost_app::keychain::{self, Error as KeyError};
use bifrost_app::controls::prefs_layout::m;
use bifrost_app::kit::ui::{CARD_BG, CONTENT_BG, FIELD_BG, LABEL, LABEL_2, LABEL_3, SEPARATOR_C, WHITE};
use bifrost_app::kit::ui::{SPACE_L, SPACE_M, SPACE_S, SPACE_XL, SPACE_XS, SPACE_XXL, SPACE_XXS};
use bifrost_app::kit::ui::{MAC_BODY_PT, MAC_FOOTNOTE_PT, MAC_TITLE1_PT};
use bifrost_app::kit::{clipboard, colors, Scroller, Surface, TextCache, Ui, Weight};
use bifrost_app::layout::{traffic_lights_trailing, UNIFIED_BAR_PT};
use bifrost_app::widgets::{self, CAPSULE_PT, TOOLBAR_GAP_PT};
use bifrost_app::dispatch::{DispatchQueue, Qos};
use bifrost_app::{Ctx, KeyInput, TouchKind};
use bifrost_core::json::Json;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

const SIDE_PT: f32 = 220.0;
const SIDE_FOLD_PT: f32 = 560.0;
const SIDE_ROW_PT: f32 = 32.0;
const SIDE_HEADER_PT: f32 = 30.0;
const SIDE_SCROLL: f32 = -(UNIFIED_BAR_PT + 4.0);
/// The transcript column never gets wider than this (a readable line), with a margin either side.
const COLUMN_PT: f32 = 720.0;
const MARGIN_PT: f32 = 24.0;
const GAP_PT: f32 = 16.0;
const BUBBLE_PAD_X_PT: f32 = 12.0;
const BUBBLE_PAD_Y_PT: f32 = 8.0;
const FIELD_MIN_PT: f32 = 36.0;
const FIELD_MAX_LINES: usize = 7;
const SEND_PT: f32 = 26.0;
const ROW_FIRST: u32 = 10;
/// The sheet panel and the column minimum, in points (the sheet is a fixed panel centred on the window).
const SHEET_W_PT: f32 = 420.0;
const SHEET_H_PT: f32 = 318.0;
const MIN_COLUMN_PT: f32 = 120.0;
/// Settings sheet rows: the first model row and the Model label, measured from the panel top.
const MODELS_Y_PT: f32 = 146.0;
const MODEL_LABEL_Y_PT: f32 = 126.0;
/// Glyphs and text drawn on the accent colour (systemBlue), never a background.
const ON_ACCENT: u32 = WHITE;

const B_NEW: u32 = 1;
const B_SETTINGS: u32 = 2;
const B_TITLE: u32 = 3;
const N_COMPOSER: u64 = 20;

pub const MENU_NEW: u32 = 1;
pub const MENU_DELETE: u32 = 2;
pub const MENU_SETTINGS: u32 = 3;
pub const MENU_COPY: u32 = 4;
pub const MENU_DIAGNOSTICS: u32 = 5;
/// The Diagnostics sheet: a panel centred on the window, with its lines and two buttons along the bottom.
const DIAG_W_PT: f32 = 460.0;
const DIAG_H_PT: f32 = 380.0;
const DIAG_BUTTON_W_PT: f32 = 120.0;
const DIAG_BUTTON_H_PT: f32 = 28.0;
/// evdev V (paste chord)
const KEY_V: u32 = 47;

fn unix_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

// Layout of message text ---------------------------------------------------------------------------------------

struct Word {
    x: f32,
    y: f32,
    w: f32,
    text: String,
    bold: bool,
    code: bool,
}

#[derive(Default)]
struct Laid {
    h: f32,
    used_w: f32,
    words: Vec<Word>,
    /// code block backgrounds: x, y, w, h
    boxes: Vec<[f32; 4]>,
}

struct Cached {
    width: i32,
    len: usize,
    laid: Laid,
}

/// User text is shown as typed: lines, no Markdown.
fn plain(text: &str) -> Vec<Block> {
    vec![Block::Para(text.lines().map(|l| vec![Span { text: l.to_string(), bold: false, code: false }]).collect())]
}

fn weight(bold: bool) -> Weight {
    if bold { Weight::Semibold } else { Weight::Regular }
}

/// Breaks one long code line at `max` px without collapsing its spaces.
fn char_wrap(ui: &Ui, line: &str, max: f32) -> Vec<String> {
    let mut rows = vec![];
    let (mut cur, mut w) = (String::new(), 0.0);
    for ch in line.chars() {
        let cw = ui.measure(&ch.to_string(), MAC_BODY_PT, Weight::Regular);
        if w + cw > max && !cur.is_empty() {
            rows.push(std::mem::take(&mut cur));
            w = 0.0;
        }
        cur.push(ch);
        w += cw;
    }
    rows.push(cur);
    rows
}

fn lay_out(ui: &Ui, blocks: &[Block], width: f32) -> Laid {
    let pt = MAC_BODY_PT;
    let (lh, gap) = (ui.pt(pt * 1.4), ui.pt(SPACE_S));
    let mut out = Laid::default();
    let mut y = 0.0f32;
    for block in blocks {
        match block {
            Block::Para(lines) => {
                for line in lines {
                    let mut x = 0.0f32;
                    for span in line {
                        for word in span.text.split_inclusive(' ') {
                            let wt = weight(span.bold);
                            let (full, bare) = (ui.measure(word, pt, wt), ui.measure(word.trim_end(), pt, wt));
                            if x > 0.0 && x + bare > width {
                                x = 0.0;
                                y += lh;
                            }
                            out.words.push(Word { x, y, w: bare, text: word.trim_end().to_string(), bold: span.bold, code: span.code });
                            x += full;
                            out.used_w = out.used_w.max(x.min(width));
                        }
                    }
                    y += lh;
                }
                y += gap;
            }
            Block::Code(lines) => {
                let pad = ui.pt(SPACE_S);
                let mut rows: Vec<String> = vec![];
                for l in lines {
                    rows.extend(char_wrap(ui, l, width - 2.0 * pad));
                }
                let h = rows.len().max(1) as f32 * lh + 2.0 * pad;
                out.boxes.push([0.0, y, width, h]);
                for (i, r) in rows.into_iter().enumerate() {
                    if !r.trim().is_empty() {
                        let w = ui.measure(&r, pt, Weight::Regular);
                        out.words.push(Word { x: pad, y: y + pad + i as f32 * lh, w, text: r, bold: false, code: false });
                    }
                }
                out.used_w = width;
                y += h + gap;
            }
        }
    }
    out.h = if blocks.is_empty() { lh } else { (y - gap).max(lh) };
    out
}

fn put(s: &mut Surface, labels: &mut TextCache, ui: &Ui, text: &str, pt: f32, wt: Weight, color: u32, x: f32, y: f32) {
    if text.is_empty() {
        return;
    }
    let img = labels.get(ui, text, pt, wt, color);
    s.blit(img, x as i32, y as i32, 255, 255);
}

/// Text centred vertically on `cy`.
fn put_mid(s: &mut Surface, labels: &mut TextCache, ui: &Ui, text: &str, pt: f32, wt: Weight, color: u32, x: f32, cy: f32) {
    if text.is_empty() {
        return;
    }
    let img = labels.get(ui, text, pt, wt, color);
    let top = cy - img.h as f32 / 2.0;
    s.blit(img, x as i32, top as i32, 255, 255);
}

fn put_centred(s: &mut Surface, labels: &mut TextCache, ui: &Ui, text: &str, pt: f32, wt: Weight, color: u32, cx: f32, cy: f32) {
    if text.is_empty() {
        return;
    }
    let img = labels.get(ui, text, pt, wt, color);
    let (x, y) = (cx - img.w as f32 / 2.0, cy - img.h as f32 / 2.0);
    s.blit(img, x as i32, y as i32, 255, 255);
}

// The window ---------------------------------------------------------------------------------------------------

struct Sheet {
    key: String,
    model: usize,
}

struct SheetGeo {
    panel: (f32, f32, f32, f32),
    field: (f32, f32, f32, f32),
    models: [(f32, f32, f32, f32); 3],
    cancel: (f32, f32, f32, f32),
    save: (f32, f32, f32, f32),
}

/// The Diagnostics sheet: the panel, the Close button and the Run Evaluation button, in pixels.
struct DiagGeo {
    panel: (f32, f32, f32, f32),
    close: (f32, f32, f32, f32),
    run: (f32, f32, f32, f32),
}

#[derive(Clone, Copy)]
struct Geo {
    side: f32,
    cont_x: f32,
    cont_w: f32,
    top: f32,
    bottom: f32,
    /// composer field: x, y, w, h
    field: (f32, f32, f32, f32),
    /// send / stop button centre and radius
    send: (f32, f32, f32),
}

fn inside((x, y, w, h): (f32, f32, f32, f32), px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

/// A pair of connected sockets: the request thread writes to one, the loop reads the other (`wake_fd`).
fn wake_pair() -> Option<(UnixStream, UnixStream)> {
    let (rx, tx) = UnixStream::pair().ok()?;
    rx.set_nonblocking(true).ok()?;
    tx.set_nonblocking(true).ok()?;
    Some((rx, tx))
}

pub struct Assistant {
    dir: PathBuf,
    convs: Vec<Conversation>,
    cur: Option<usize>,
    w: u32,
    h: u32,
    model: String,
    key: Option<String>,
    get_req: Option<u32>,
    key_req: Option<u32>,
    key_pending: Option<String>,
    composer: String,
    tx: Sender<(u64, Event)>,
    rx: Receiver<(u64, Event)>,
    gen: u64,
    streaming: Option<Running>,
    stream_conv: String,
    error: Option<String>,
    sheet: Option<Sheet>,
    scroll: Scroller,
    follow: bool,
    labels: TextCache,
    laid: Vec<Option<Cached>>,
    laid_conv: String,
    press: Option<(f32, f32, bool, bool)>,
    appearance: u32,
    dirty: bool,
    wake_rx: Option<UnixStream>,
    wake_tx: Option<UnixStream>,
    /// The network path, asked once at start and then kept up to date by the shell's events.
    link: Link,
    /// The pasteboard write of a "Copy Answer", until its reply arrives.
    copy_req: Option<u32>,
    /// Blocked phrases and the length limit that every answer passes before it is shown or saved.
    guard: Guardrail,
    /// Routes a request to a skill (a system prompt).
    router: Option<TextClassifier>,
    /// The text of the answer in progress as the model sent it, before the guardrail screens it.
    stream_raw: String,
    /// The skill the last request was routed to, `None` for general.
    last_skill: Option<&'static str>,
    diag_open: bool,
    diag_running: bool,
    diag_lines: Vec<String>,
    diag_tx: Sender<Vec<String>>,
    diag_rx: Receiver<Vec<String>>,
}

impl Assistant {
    pub fn new(ctx: &Ctx) -> Self {
        let dir = ctx.data_dir.clone();
        let (tx, rx) = channel();
        let (diag_tx, diag_rx) = channel();
        let get_req = Some(keychain::get(ctx, model::KEY_SERVICE, model::KEY_ACCOUNT));
        let link = Link::new(ctx);
        let (wake_rx, wake_tx) = match wake_pair() {
            Some((rx, tx)) => (Some(rx), Some(tx)),
            None => (None, None),
        };
        let mut a = Assistant {
            convs: model::load(&dir),
            model: model::load_model(&dir),
            guard: safety::load_guardrail(&dir),
            router: safety::router(),
            stream_raw: String::new(),
            last_skill: None,
            diag_open: false,
            diag_running: false,
            diag_lines: Vec::new(),
            diag_tx,
            diag_rx,
            dir,
            cur: None,
            w: ctx.cfg.w,
            h: ctx.cfg.h,
            key: None,
            get_req,
            key_req: None,
            key_pending: None,
            composer: String::new(),
            tx,
            rx,
            gen: 0,
            streaming: None,
            stream_conv: String::new(),
            error: None,
            sheet: None,
            scroll: Scroller::new(),
            follow: true,
            labels: TextCache::default(),
            laid: Vec::new(),
            laid_conv: String::new(),
            press: None,
            appearance: LABEL.now(),
            dirty: true,
            wake_rx,
            wake_tx,
            link,
            copy_req: None,
        };
        a.new_chat();
        a
    }

    pub fn set_size(&mut self, w: u32, h: u32) {
        self.w = w;
        self.h = h;
        self.dirty = true;
    }

    // conversations ---------------------------------------------------------------------------------------------

    fn current(&self) -> Option<&Conversation> {
        self.cur.and_then(|i| self.convs.get(i))
    }

    /// Selects the empty conversation at the top, or makes one.
    pub fn new_chat(&mut self) {
        if !self.convs.first().is_some_and(|c| c.messages.is_empty()) {
            self.convs.insert(0, Conversation::new(unix_ms().to_string(), unix_ms()));
        }
        self.select(0);
    }

    fn select(&mut self, i: usize) {
        self.cur = Some(i);
        self.error = None;
        self.follow = true;
        self.scroll.jump_top();
        self.dirty = true;
    }

    pub fn can_delete(&self) -> bool {
        self.current().is_some_and(|c| !c.messages.is_empty())
    }

    pub fn delete_current(&mut self) {
        let Some(i) = self.cur else { return };
        if self.streaming.is_some() && self.convs.get(i).is_some_and(|c| c.id == self.stream_conv) {
            self.stop();
        }
        if i < self.convs.len() {
            self.convs.remove(i);
        }
        self.persist();
        if self.convs.is_empty() {
            self.new_chat();
        } else {
            self.select(i.min(self.convs.len() - 1));
        }
    }

    fn persist(&self) {
        let saved: Vec<Conversation> = self.convs.iter().filter(|c| !c.messages.is_empty()).cloned().collect();
        let _ = model::save(&self.dir, &saved);
    }

    pub fn flush(&self) {
        self.persist();
    }

    // sending ---------------------------------------------------------------------------------------------------

    pub fn streaming(&self) -> bool {
        self.streaming.is_some()
    }

    fn send(&mut self) {
        if self.streaming.is_some() {
            return;
        }
        let text = self.composer.trim().to_string();
        if text.is_empty() {
            return;
        }
        // The request goes out with the instructions of its skill, if a skill word is in it.
        let skill = safety::route(self.router.as_ref(), &text);
        let system = skill.and_then(safety::skill_prompt);
        // Offline: keep the composer text and say why, instead of sending a request that cannot leave the machine.
        if self.link.offline() {
            self.error = Some(connect::OFFLINE.into());
            self.dirty = true;
            return;
        }
        let Some(key) = self.key.clone() else {
            self.open_sheet();
            return;
        };
        if self.cur.is_none() {
            self.new_chat();
        }
        let Some(i) = self.cur else { return };
        let mut conv = self.convs.remove(i);
        conv.messages.push(Message { user: true, text });
        conv.retitle();
        conv.updated = unix_ms();
        // The request is a session of this conversation under the skill's profile, fitted to the context budget.
        let request = session::request(&session::session(&conv.messages, session::profile(skill, system)));
        conv.messages.push(Message { user: false, text: String::new() });
        self.stream_conv = conv.id.clone();
        self.convs.insert(0, conv);
        self.cur = Some(0);
        self.composer.clear();
        self.error = None;
        self.follow = true;
        self.gen += 1;
        self.stream_raw.clear();
        self.last_skill = skill;
        let wake = self.wake_tx.as_ref().and_then(|w| w.try_clone().ok());
        self.streaming = Some(api::spawn(self.gen, key, self.model.clone(), request.system, request.messages, self.tx.clone(), wake));
        self.persist();
        self.dirty = true;
    }

    pub fn stop(&mut self) {
        if let Some(r) = self.streaming.take() {
            r.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            self.gen += 1;
            self.finish(None);
        }
    }

    fn finish(&mut self, error: Option<String>) {
        self.streaming = None;
        let id = std::mem::take(&mut self.stream_conv);
        let shown = safety::screen_final(&self.guard, &std::mem::take(&mut self.stream_raw));
        if let Some(c) = self.convs.iter_mut().find(|c| c.id == id) {
            if let Some(last) = c.messages.last_mut().filter(|m| !m.user) {
                last.text = shown;
            }
            if c.messages.last().is_some_and(|m| !m.user && m.text.trim().is_empty()) {
                c.messages.pop();
            }
        }
        if self.current().is_some_and(|c| c.id == id) {
            self.error = error;
        }
        self.persist();
        self.dirty = true;
    }

    /// Applies what the request thread sent; true when the screen changed. Also steps the scroller.
    pub fn tick(&mut self, t: f64) -> bool {
        if let Some(mut r) = self.wake_rx.as_ref() {
            let mut buf = [0u8; 64];
            while r.read(&mut buf).is_ok_and(|n| n > 0) {}
        }
        while let Ok(lines) = self.diag_rx.try_recv() {
            self.diag_lines = lines;
            self.diag_running = false;
            self.dirty = true;
        }
        while let Ok((g, ev)) = self.rx.try_recv() {
            if g != self.gen {
                continue;
            }
            self.dirty = true;
            match ev {
                Event::Delta(d) => {
                    // The answer shows only what the guardrail lets through, so it is screened as it grows.
                    self.stream_raw.push_str(&d);
                    let shown = safety::screen_partial(&self.guard, &self.stream_raw);
                    let id = self.stream_conv.clone();
                    if let Some(m) = self.convs.iter_mut().find(|c| c.id == id).and_then(|c| c.messages.last_mut()) {
                        m.text = shown;
                    }
                }
                Event::Done => self.finish(None),
                Event::Error(e) => self.finish(Some(e)),
            }
        }
        if self.scroll.tick(t) {
            self.dirty = true;
        }
        if LABEL.now() != self.appearance || bifrost_app::kit::appearance::fading() {
            self.appearance = LABEL.now();
            self.dirty = true;
        }
        std::mem::take(&mut self.dirty)
    }


    // settings and the keychain ---------------------------------------------------------------------------------

    pub fn open_sheet(&mut self) {
        self.diag_open = false;
        let model = model::MODELS.iter().position(|(id, _)| *id == self.model).unwrap_or(0);
        self.sheet = Some(Sheet { key: String::new(), model });
        self.dirty = true;
    }

    fn save_sheet(&mut self, ctx: &Ctx) {
        let Some(sheet) = self.sheet.take() else { return };
        self.model = model::MODELS[sheet.model].0.to_string();
        let _ = model::save_model(&self.dir, &self.model);
        let key = sheet.key.trim().to_string();
        if !key.is_empty() {
            self.key = Some(key.clone());
            // update first; a missing item answers ItemNotFound and the reply handler adds it
            self.key_req = Some(keychain::update(ctx, model::KEY_SERVICE, model::KEY_ACCOUNT, key.as_bytes()));
            self.key_pending = Some(key);
        }
        self.dirty = true;
    }

    /// The answer to a keychain call made above.
    pub fn keychain(&mut self, ctx: &Ctx, reply: keychain::Reply) -> bool {
        if self.get_req == Some(reply.request) {
            self.get_req = None;
            if let Ok(bytes) = reply.result {
                self.key = String::from_utf8(bytes).ok().filter(|k| !k.is_empty());
            }
            self.dirty = true;
        } else if self.key_req == Some(reply.request) {
            self.key_req = None;
            match (reply.result, self.key_pending.take()) {
                (Err(KeyError::ItemNotFound), Some(key)) => {
                    self.key_req = Some(keychain::add(ctx, model::KEY_SERVICE, model::KEY_ACCOUNT, key.as_bytes()));
                }
                (Err(e), _) => ctx.log(&format!("keychain: {e:?}")),
                _ => {}
            }
        }
        self.dirty
    }

    // geometry --------------------------------------------------------------------------------------------------

    fn w_pt(&self, ui: &Ui) -> f32 {
        self.w as f32 / ui.pt(1.0)
    }

    fn side_pt(&self, ui: &Ui) -> f32 {
        if self.w_pt(ui) < SIDE_FOLD_PT { 0.0 } else { SIDE_PT }
    }

    fn composer_lines(&self, ui: &Ui, width: f32) -> Vec<String> {
        let mut lines = vec![];
        for seg in self.composer.split('\n') {
            let mut wrapped = ui.wrap(seg, MAC_BODY_PT, Weight::Regular, width);
            if wrapped.is_empty() {
                wrapped.push(String::new());
            }
            // wrap() drops trailing spaces; keep them so the caret follows what was typed
            let trailing = seg.len() - seg.trim_end_matches(' ').len();
            if let Some(last) = wrapped.last_mut() {
                last.push_str(&" ".repeat(trailing));
            }
            lines.extend(wrapped);
        }
        lines
    }

    fn geo(&self, ui: &Ui) -> Geo {
        let k = ui.pt(1.0);
        let side = ui.pt(self.side_pt(ui));
        let det_w = self.w as f32 - side;
        let cont_w = (det_w - 2.0 * ui.pt(MARGIN_PT)).min(ui.pt(COLUMN_PT)).max(ui.pt(MIN_COLUMN_PT));
        let cont_x = side + (det_w - cont_w) / 2.0;
        let inner = cont_w - ui.pt(SPACE_L) - ui.pt(SEND_PT + 8.0);
        let lines = self.composer_lines(ui, inner).len().clamp(1, FIELD_MAX_LINES);
        let lh = ui.pt(MAC_BODY_PT * 1.35);
        let fh = (2.0 * ui.pt(SPACE_S) + lines as f32 * lh).max(ui.pt(FIELD_MIN_PT));
        let fy = self.h as f32 - ui.pt(SPACE_L) - fh;
        let field = (cont_x, fy, cont_w, fh);
        let r = ui.pt(SEND_PT) / 2.0;
        let send = (cont_x + cont_w - r - ui.pt(5.0), fy + fh - r - ui.pt(5.0), r);
        let _ = k;
        Geo { side, cont_x, cont_w, top: ui.pt(UNIFIED_BAR_PT), bottom: fy - ui.pt(SPACE_M), field, send }
    }

    fn sheet_geo(&self, ui: &Ui) -> SheetGeo {
        let (pw, ph) = (ui.pt(SHEET_W_PT).min(self.w as f32 - ui.pt(SPACE_XXL)), ui.pt(SHEET_H_PT).min(self.h as f32 - ui.pt(SPACE_XXL)));
        let (px, py) = ((self.w as f32 - pw) / 2.0, (self.h as f32 - ph) / 2.0);
        let m = ui.pt(SPACE_XL);
        let field = (px + m, py + ui.pt(84.0), pw - 2.0 * m, ui.pt(28.0));
        let row = ui.pt(28.0);
        let y0 = py + ui.pt(MODELS_Y_PT);
        let models = [0, 1, 2].map(|i| (px + m, y0 + i as f32 * row, pw - 2.0 * m, row));
        let (bw, bh) = (ui.pt(76.0), ui.pt(28.0));
        let by = py + ph - m - bh;
        let save = (px + pw - m - bw, by, bw, bh);
        let cancel = (save.0 - ui.pt(SPACE_S) - bw, by, bw, bh);
        SheetGeo { panel: (px, py, pw, ph), field, models, cancel, save }
    }

    // toolbar ---------------------------------------------------------------------------------------------------

    fn bars(&self, ui: &Ui) -> Vec<(Toolbar, Rect)> {
        let side = self.side_pt(ui);
        let lead = traffic_lights_trailing() + TOOLBAR_GAP_PT;
        let band = |x: f32, w: f32| Rect { x, y: (UNIFIED_BAR_PT - CAPSULE_PT) / 2.0, w: w.max(0.0), h: CAPSULE_PT };
        let mut bars = vec![];
        if side > 0.0 {
            bars.push((Toolbar::new(vec![ToolbarItem::flexible_space(), ToolbarItem::icon(B_NEW, "New Chat", "square.and.pencil")]), band(lead, side - lead - 8.0)));
        }
        let x0 = if side > 0.0 { side + 12.0 } else { lead };
        let mut items = vec![];
        if side == 0.0 {
            items.push(ToolbarItem::icon(B_NEW, "New Chat", "square.and.pencil"));
        }
        let buttons = if side == 0.0 { 2.0 } else { 1.0 };
        let w = self.w_pt(ui) - x0 - 12.0;
        items.push(ToolbarItem::custom(B_TITLE, "Title", (w - buttons * CAPSULE_PT - 2.0 * TOOLBAR_GAP_PT).max(40.0)));
        items.push(ToolbarItem::flexible_space());
        items.push(ToolbarItem::icon(B_SETTINGS, "Settings", "ellipsis.circle"));
        bars.push((Toolbar::new(items), band(x0, w)));
        bars
    }

    fn layouts(&self, ui: &Ui) -> Vec<(Toolbar, ToolbarLayout)> {
        let item_w = |it: &ToolbarItem| {
            if it.symbol.is_some() {
                CAPSULE_PT
            } else {
                (ui.measure(&it.label, MAC_BODY_PT, Weight::Regular) / ui.pt(1.0) + 24.0).max(CAPSULE_PT)
            }
        };
        self.bars(ui).into_iter().map(|(bar, r)| {
            let l = bar.layout_with(r, item_w, CAPSULE_PT, TOOLBAR_GAP_PT);
            (bar, l)
        }).collect()
    }

    fn bar_hit(&self, ui: &Ui, x: f32, y: f32) -> Option<u32> {
        let (x, y) = (x / ui.pt(1.0), y / ui.pt(1.0));
        if y >= UNIFIED_BAR_PT {
            return None;
        }
        self.layouts(ui).iter().find_map(|(_, l)| l.hit(x, y)).filter(|id| *id != B_TITLE)
    }

    fn sidebar(&self) -> SourceList {
        let mut items = vec![SidebarItem::header("Chats")];
        for (i, c) in self.convs.iter().enumerate() {
            items.push(SidebarItem::row(ROW_FIRST + i as u32, &c.title));
        }
        let mut list = SourceList::new(items, SIDE_ROW_PT, SIDE_HEADER_PT);
        list.set_selected(self.cur.map(|i| ROW_FIRST + i as u32));
        list
    }

    fn sidebar_bounds(&self, ui: &Ui) -> Rect {
        Rect { x: 0.0, y: 0.0, w: self.side_pt(ui), h: self.h as f32 / ui.pt(1.0) }
    }

    /// Every toolbar button and the composer as accessible controls (`App::controls`), in points.
    pub fn controls(&self, ui: &Ui) -> Vec<Node> {
        let mut nodes = vec![];
        for (bar, l) in self.layouts(ui) {
            for (id, r) in l.visible {
                if id == B_TITLE {
                    continue;
                }
                if let Some(item) = bar.items.iter().find(|i| i.id == id) {
                    nodes.push(Node { id: id as u64, role: Role::Button, label: item.label.clone(), value: String::new(), bounds: r, enabled: item.enabled });
                }
            }
        }
        let (x, y, w, h) = self.geo(ui).field;
        let k = ui.pt(1.0);
        nodes.push(Node { id: N_COMPOSER, role: Role::TextField, label: "Message".into(), value: self.composer.clone(), bounds: Rect { x: x / k, y: y / k, w: w / k, h: h / k }, enabled: true });
        nodes
    }

    pub fn activate(&mut self, id: u64) -> bool {
        match id {
            x if x == B_NEW as u64 => self.new_chat(),
            x if x == B_SETTINGS as u64 => self.open_sheet(),
            _ => return false,
        }
        true
    }

    // input -----------------------------------------------------------------------------------------------------

    pub fn menu(&mut self, ctx: &Ctx, id: u32) -> bool {
        match id {
            MENU_NEW => self.new_chat(),
            MENU_DELETE => self.delete_current(),
            MENU_SETTINGS => self.open_sheet(),
            MENU_DIAGNOSTICS => self.open_diagnostics(),
            MENU_COPY => return self.copy_answer(ctx),
            _ => return false,
        }
        true
    }

    /// The newest answer of the open conversation, once it has finished streaming.
    fn last_answer(&self) -> Option<&str> {
        self.current()?.messages.iter().rev().find(|m| !m.user && !m.text.trim().is_empty()).map(|m| m.text.as_str())
    }

    pub fn can_copy(&self) -> bool {
        self.streaming.is_none() && self.last_answer().is_some()
    }

    /// Copies the newest answer to the general pasteboard as HTML and plain text (one user action, one service call).
    pub fn copy_answer(&mut self, ctx: &Ctx) -> bool {
        if !self.can_copy() {
            return false;
        }
        let Some(text) = self.last_answer().map(str::to_string) else { return false };
        self.copy_req = Some(connect::copy_answer(ctx, &text));
        true
    }

    /// The reply to a service call the chat made: the copy, or the network path. True when it was one of ours.
    pub fn service_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> bool {
        if self.copy_req == Some(request) {
            self.copy_req = None;
            if let Err(e) = result {
                ctx.log(&format!("copy answer: {e}"));
            }
            return true;
        }
        if self.link.reply(request, result) {
            self.dirty = true;
            return true;
        }
        false
    }

    /// A shell event for the chat; true when it changed what is shown (the network path).
    pub fn service_event(&mut self, service: &str, event: &str, body: &Json) -> bool {
        let changed = self.link.event(service, event, body);
        self.dirty |= changed;
        changed
    }

    pub fn paste(&mut self) -> bool {
        let Some(text) = clipboard::text() else { return false };
        match &mut self.sheet {
            Some(sheet) => sheet.key.push_str(text.trim()),
            None => self.composer.push_str(&text),
        }
        self.dirty = true;
        true
    }

    pub fn key(&mut self, ctx: &Ctx, k: KeyInput) -> bool {
        use bifrost_app::proto::{CHORD_CTRL, CHORD_META};
        if let Some((key, mods)) = k.as_chord() {
            return key == KEY_V && mods & (CHORD_CTRL | CHORD_META) != 0 && self.paste();
        }
        // The Diagnostics sheet takes Escape to close and Return to run; other keys do nothing while it is open.
        if self.diag_open {
            match k {
                KeyInput::Code(1) => self.diag_open = false,
                KeyInput::Code(13) => self.run_diagnostics(),
                _ => return false,
            }
            self.dirty = true;
            return true;
        }
        self.dirty = true;
        match k {
            KeyInput::Text(s) => {
                let s: String = s.chars().filter(|c| !c.is_control() || *c == '\n').collect();
                match &mut self.sheet {
                    Some(sheet) => sheet.key.push_str(s.trim_matches('\n')),
                    // Shift+Return arrives as a newline; a plain Return is Code(13) and sends
                    None => self.composer.push_str(&s),
                }
            }
            KeyInput::Code(13) => {
                if self.sheet.is_some() {
                    self.save_sheet(ctx);
                } else {
                    self.send();
                }
            }
            KeyInput::Code(8) => {
                match &mut self.sheet {
                    Some(sheet) => sheet.key.pop(),
                    None => self.composer.pop(),
                };
            }
            KeyInput::Code(1) => {
                if self.sheet.take().is_none() {
                    return false;
                }
            }
            KeyInput::Code(_) => return false,
        }
        true
    }

    pub fn touch(&mut self, ctx: &Ctx, kind: TouchKind, x: f32, y: f32, t: f64) -> bool {
        let ui = &ctx.ui;
        let g = self.geo(ui);
        match kind {
            TouchKind::Down => {
                let in_scroll = self.sheet.is_none() && !self.diag_open && x >= g.side && y > g.top && y < g.bottom;
                if in_scroll {
                    self.scroll.down(y, t);
                }
                self.press = Some((x, y, in_scroll, false));
                false
            }
            TouchKind::Move => {
                let Some((dx, dy, in_scroll, moved)) = self.press else { return false };
                let far = (x - dx).hypot(y - dy) > ui.pt(6.0);
                if in_scroll && (moved || far) {
                    self.press = Some((dx, dy, true, true));
                    self.scroll.moved(y, t);
                    self.follow = false;
                    return true;
                }
                false
            }
            TouchKind::Up | TouchKind::Cancel => {
                let Some((_, _, in_scroll, moved)) = self.press.take() else { return false };
                if moved {
                    self.scroll.up(y, t);
                    return true;
                }
                if in_scroll {
                    self.scroll.up(y, t);
                }
                if matches!(kind, TouchKind::Cancel) {
                    return false;
                }
                self.click(ctx, x, y)
            }
        }
    }

    fn click(&mut self, ctx: &Ctx, x: f32, y: f32) -> bool {
        let ui = &ctx.ui;
        self.dirty = true;
        if self.diag_open {
            let dg = self.diag_geo(ui);
            if inside(dg.run, x, y) {
                self.run_diagnostics();
            } else if inside(dg.close, x, y) || !inside(dg.panel, x, y) {
                self.diag_open = false;
            }
            return true;
        }
        if self.sheet.is_some() {
            let sg = self.sheet_geo(ui);
            if inside(sg.save, x, y) {
                self.save_sheet(ctx);
            } else if inside(sg.cancel, x, y) || !inside(sg.panel, x, y) {
                self.sheet = None;
            } else if let Some(i) = sg.models.iter().position(|r| inside(*r, x, y)) {
                if let Some(sheet) = &mut self.sheet {
                    sheet.model = i;
                }
            }
            return true;
        }
        if let Some(id) = self.bar_hit(ui, x, y) {
            return self.activate(id as u64);
        }
        let g = self.geo(ui);
        if x < g.side && y >= ui.pt(UNIFIED_BAR_PT) {
            let hit = self.sidebar().hit(self.sidebar_bounds(ui), SIDE_SCROLL, x / ui.pt(1.0), y / ui.pt(1.0));
            if let Some(id) = hit {
                let i = (id - ROW_FIRST) as usize;
                if i < self.convs.len() {
                    self.select(i);
                }
            }
            return true;
        }
        let (cx, cy, r) = g.send;
        if (x - cx).hypot(y - cy) <= r + ui.pt(SPACE_XXS) {
            if self.streaming.is_some() {
                self.stop();
            } else {
                self.send();
            }
            return true;
        }
        false
    }

    // drawing ---------------------------------------------------------------------------------------------------

    /// Lays out every message of the open conversation again when its text or the width changed.
    fn ensure_laid(&mut self, ui: &Ui, cont_w: f32) {
        let Some(i) = self.cur else { return };
        let Some(conv) = self.convs.get(i) else { return };
        if self.laid_conv != conv.id {
            self.laid_conv = conv.id.clone();
            self.laid.clear();
        }
        self.laid.resize_with(conv.messages.len(), || None);
        let user_w = (cont_w * 0.75).floor();
        for (m, slot) in conv.messages.iter().zip(self.laid.iter_mut()) {
            let width = if m.user { user_w - 2.0 * ui.pt(BUBBLE_PAD_X_PT) } else { cont_w };
            let stale = slot.as_ref().map_or(true, |c| c.width != width as i32 || c.len != m.text.len());
            if stale {
                let blocks = if m.user { plain(&m.text) } else { model::parse(&m.text) };
                *slot = Some(Cached { width: width as i32, len: m.text.len(), laid: lay_out(ui, &blocks, width) });
            }
        }
    }

    pub fn draw(&mut self, s: &mut Surface, ui: &Ui) {
        let g = self.geo(ui);
        s.fill_rect(0, 0, s.w, s.h, CONTENT_BG.now());
        self.ensure_laid(ui, g.cont_w);
        self.draw_transcript(s, ui, &g);
        // composer area over the transcript's tail
        let det_x = g.side as usize;
        s.fill_rect(det_x, g.bottom as usize, s.w.saturating_sub(det_x), s.h.saturating_sub(g.bottom as usize), CONTENT_BG.now());
        self.draw_composer(s, ui, &g);
        self.draw_chrome(s, ui, &g);
        if self.sheet.is_some() {
            self.draw_sheet(s, ui);
        }
        if self.diag_open {
            self.draw_diagnostics(s, ui);
        }
    }

    fn draw_transcript(&mut self, s: &mut Surface, ui: &Ui, g: &Geo) {
        let Some(conv) = self.current() else { return };
        let (view_h, empty) = (g.bottom - g.top, conv.messages.is_empty());
        if empty {
            let mid = g.top + view_h / 2.0;
            let cx = g.cont_x + g.cont_w / 2.0;
            put_centred(s, &mut self.labels, ui, "How can I help?", MAC_TITLE1_PT, Weight::Semibold, LABEL.now(), cx, mid - ui.pt(10.0));
            if self.link.offline() {
                put_centred(s, &mut self.labels, ui, connect::OFFLINE_HINT, MAC_BODY_PT, Weight::Regular, LABEL_2.now(), cx, mid + ui.pt(18.0));
            } else if self.key.is_none() && self.get_req.is_none() {
                put_centred(s, &mut self.labels, ui, "Add your API key in Settings to start.", MAC_BODY_PT, Weight::Regular, LABEL_2.now(), cx, mid + ui.pt(18.0));
            }
            return;
        }
        let users: Vec<bool> = conv.messages.iter().map(|m| m.user).collect();
        let waiting: Vec<bool> = conv.messages.iter().map(|m| m.text.is_empty()).collect();
        let (pad_x, pad_y, gap) = (ui.pt(BUBBLE_PAD_X_PT), ui.pt(BUBBLE_PAD_Y_PT), ui.pt(GAP_PT));
        let err_lines = self.error.as_ref().map(|e| ui.wrap(e, MAC_BODY_PT, Weight::Regular, g.cont_w - 2.0 * pad_x));
        let lh = ui.pt(MAC_BODY_PT * 1.4);
        let err_h = err_lines.as_ref().map_or(0.0, |l| l.len() as f32 * lh + 2.0 * pad_y);
        let mut total = gap;
        for (i, user) in users.iter().enumerate() {
            let h = self.laid.get(i).and_then(|c| c.as_ref()).map_or(0.0, |c| c.laid.h);
            total += if *user { h + 2.0 * pad_y } else { h } + gap;
        }
        if err_lines.is_some() {
            total += err_h + gap;
        }
        let max = (total - view_h).max(0.0);
        self.scroll.set_max(max);
        if self.follow {
            self.scroll.jump_to(max);
        } else if self.scroll.offset() >= max - 1.0 && !self.scroll.dragging() {
            self.follow = self.streaming.is_some();
        }
        let mut y = g.top + gap - self.scroll.offset();
        let (blue, white) = (colors::SYSTEM_BLUE.now(), ON_ACCENT);
        let (fg, field_bg) = (LABEL.now(), FIELD_BG.now());
        let (top, bottom) = (g.top - lh, g.bottom);
        for (i, user) in users.iter().enumerate() {
            let Some(c) = self.laid.get(i).and_then(|c| c.as_ref()) else { continue };
            let laid = &c.laid;
            let (ox, oy, bh) = if *user {
                let bw = laid.used_w + 2.0 * pad_x;
                let bx = g.cont_x + g.cont_w - bw;
                let bh = laid.h + 2.0 * pad_y;
                if y + bh >= top && y <= bottom {
                    s.fill_rrect(bx, y, bw, bh, ui.pt(m::CARD_RADIUS), blue);
                }
                (bx + pad_x, y + pad_y, bh)
            } else {
                (g.cont_x, y, laid.h)
            };
            if oy + laid.h >= top && oy <= bottom {
                for b in &laid.boxes {
                    s.fill_rrect(ox + b[0], oy + b[1], b[2], b[3], ui.pt(m::SELECTION_RADIUS), field_bg);
                }
                for w in &laid.words {
                    let wy = oy + w.y;
                    if wy + lh < top || wy > bottom {
                        continue;
                    }
                    if w.code {
                        s.fill_rrect(ox + w.x - ui.pt(SPACE_XXS), wy, w.w + ui.pt(SPACE_XS), lh, ui.pt(m::ICON_RADIUS), field_bg);
                    }
                    let color = if *user { white } else { fg };
                    put(s, &mut self.labels, ui, &w.text, MAC_BODY_PT, weight(w.bold), color, ox + w.x, wy + (lh - ui.pt(MAC_BODY_PT * 1.2)) / 2.0);
                }
                if waiting[i] && !*user {
                    put(s, &mut self.labels, ui, "…", MAC_BODY_PT, Weight::Regular, LABEL_3.now(), ox, oy);
                }
            }
            y += bh + gap;
        }
        if let Some(lines) = err_lines {
            if y + err_h >= top && y <= bottom {
                s.fill_rrect(g.cont_x, y, g.cont_w, err_h, ui.pt(m::CARD_RADIUS), CARD_BG.now());
                let red = colors::SYSTEM_RED.now();
                for (i, l) in lines.iter().enumerate() {
                    put(s, &mut self.labels, ui, l, MAC_BODY_PT, Weight::Regular, red, g.cont_x + pad_x, y + pad_y + i as f32 * lh);
                }
            }
        }
    }

    fn draw_composer(&mut self, s: &mut Surface, ui: &Ui, g: &Geo) {
        let (x, y, w, h) = g.field;
        let r = ui.pt(m::CARD_RADIUS).min(h / 2.0);
        s.fill_rrect(x, y, w, h, r, FIELD_BG.now());
        let pad = ui.pt(SPACE_M);
        let inner = w - ui.pt(SPACE_L) - ui.pt(SEND_PT + 8.0);
        let lh = ui.pt(MAC_BODY_PT * 1.35);
        if self.composer.is_empty() {
            put_mid(s, &mut self.labels, ui, "Message Assistant", MAC_BODY_PT, Weight::Regular, LABEL_3.now(), x + pad, y + h / 2.0);
        } else {
            let lines = self.composer_lines(ui, inner);
            let shown = lines.len().min(FIELD_MAX_LINES);
            let first = lines.len() - shown;
            for (i, l) in lines[first..].iter().enumerate() {
                put(s, &mut self.labels, ui, l.trim_end(), MAC_BODY_PT, Weight::Regular, LABEL.now(), x + pad, y + ui.pt(SPACE_S) + i as f32 * lh);
            }
            if self.sheet.is_none() {
                let last = lines.last().map_or("", String::as_str);
                let cx = x + pad + ui.measure(last, MAC_BODY_PT, Weight::Regular);
                let cy = y + ui.pt(SPACE_S) + (shown - 1) as f32 * lh;
                s.fill_rect(cx as usize, cy as usize, (ui.pt(1.0) as usize).max(1), lh as usize, colors::CONTROL_ACCENT.now());
            }
        }
        let (cx, cy, rad) = g.send;
        let busy = self.streaming.is_some();
        let on = busy || !self.composer.trim().is_empty();
        s.fill_rrect(cx - rad, cy - rad, 2.0 * rad, 2.0 * rad, rad, if on { colors::SYSTEM_BLUE.now() } else { LABEL_3.now() });
        let white = ON_ACCENT;
        if busy {
            let q = rad * 0.5;
            s.fill_rrect(cx - q / 2.0 - q / 4.0, cy - q / 2.0 - q / 4.0, q * 1.5, q * 1.5, ui.pt(SPACE_XXS), white);
        } else {
            let (t, a) = (ui.pt(1.8), rad * 0.5);
            springboard::shell::icons::line(s, (cx, cy + a), (cx, cy - a), t, white);
            springboard::shell::icons::line(s, (cx, cy - a), (cx - a * 0.8, cy - a * 0.1), t, white);
            springboard::shell::icons::line(s, (cx, cy - a), (cx + a * 0.8, cy - a * 0.1), t, white);
        }
    }

    fn draw_chrome(&mut self, s: &mut Surface, ui: &Ui, g: &Geo) {
        let side = self.side_pt(ui);
        if side > 0.0 {
            widgets::source_list(s, ui, self.sidebar_bounds(ui), &self.sidebar(), SIDE_SCROLL, None);
            s.fill_rect(g.side as usize - 1, 0, 1, self.h as usize, SEPARATOR_C.now());
        }
        widgets::toolbar_band(s, ui, Rect { x: side, y: 0.0, w: self.w_pt(ui) - side, h: UNIFIED_BAR_PT });
        let title = self.current().map_or("New Chat", |c| c.title.as_str()).to_string();
        for (bar, r) in self.bars(ui) {
            let placed = widgets::toolbar(s, ui, r, &bar, None);
            if let Some(t) = placed.rect_of(B_TITLE) {
                let fitted = ui.fit(&title, MAC_BODY_PT, ui.pt(t.w));
                put_mid(s, &mut self.labels, ui, &fitted, MAC_BODY_PT, Weight::Semibold, LABEL.now(), ui.pt(t.x), ui.pt(t.y + t.h / 2.0));
            }
        }
    }

    fn draw_sheet(&mut self, s: &mut Surface, ui: &Ui) {
        let sg = self.sheet_geo(ui);
        let Some(sheet) = &self.sheet else { return };
        let (px, py, pw, ph) = sg.panel;
        // a hairline shadow ring, then the panel
        s.fill_rrect(px - 1.0, py - 1.0, pw + 2.0, ph + 2.0, ui.pt(m::CARD_RADIUS + 1.0), SEPARATOR_C.now());
        s.fill_rrect(px, py, pw, ph, ui.pt(m::CARD_RADIUS), CARD_BG.now());
        let m = ui.pt(SPACE_XL);
        let (label, label2, fg) = (LABEL.now(), LABEL_2.now(), LABEL.now());
        put(s, &mut self.labels, ui, "Settings", MAC_TITLE1_PT, Weight::Semibold, label, px + m, py + ui.pt(SPACE_XL));
        put(s, &mut self.labels, ui, "Anthropic API key", MAC_FOOTNOTE_PT, Weight::Regular, label2, px + m, py + ui.pt(64.0));
        let (fx, fy, fw, fh) = sg.field;
        s.fill_rrect(fx, fy, fw, fh, ui.pt(m::ICON_RADIUS), FIELD_BG.now());
        let shown = if sheet.key.is_empty() {
            None
        } else {
            let n = sheet.key.chars().count();
            let tail: String = sheet.key.chars().skip(n.saturating_sub(4)).collect();
            Some(format!("{}{}", "•".repeat(n.saturating_sub(4).min(32)), tail))
        };
        match shown {
            Some(t) => {
                let t = ui.fit(&t, MAC_BODY_PT, fw - ui.pt(SPACE_L));
                put_mid(s, &mut self.labels, ui, &t, MAC_BODY_PT, Weight::Regular, fg, fx + ui.pt(SPACE_S), fy + fh / 2.0);
            }
            None => {
                let hint = if self.key.is_some() { "Saved in the Keychain. Paste a new key to replace it" } else { "sk-ant-…  (paste with Cmd+V)" };
                put_mid(s, &mut self.labels, ui, hint, MAC_BODY_PT, Weight::Regular, LABEL_3.now(), fx + ui.pt(SPACE_S), fy + fh / 2.0);
            }
        }
        put(s, &mut self.labels, ui, "Model", MAC_FOOTNOTE_PT, Weight::Regular, label2, px + m, py + ui.pt(MODEL_LABEL_Y_PT));
        let selected = sheet.model;
        for (i, (_, name)) in model::MODELS.iter().enumerate() {
            let (rx, ry, rw, rh) = sg.models[i];
            let cy = ry + rh / 2.0;
            let rad = ui.pt(7.0);
            s.fill_rrect(rx, cy - rad, 2.0 * rad, 2.0 * rad, rad, LABEL_3.now());
            if i == selected {
                s.fill_rrect(rx, cy - rad, 2.0 * rad, 2.0 * rad, rad, colors::SYSTEM_BLUE.now());
                let dot = ui.pt(6.0);
                s.fill_rrect(rx + rad - dot / 2.0, cy - dot / 2.0, dot, dot, dot / 2.0, ON_ACCENT);
            }
            put_mid(s, &mut self.labels, ui, name, MAC_BODY_PT, Weight::Regular, fg, rx + 2.0 * rad + ui.pt(SPACE_S), cy);
            let _ = rw;
        }
        let (cx, cy, cw, ch) = sg.cancel;
        s.fill_rrect(cx, cy, cw, ch, ui.pt(m::ICON_RADIUS), FIELD_BG.now());
        put_centred(s, &mut self.labels, ui, "Cancel", MAC_BODY_PT, Weight::Regular, fg, cx + cw / 2.0, cy + ch / 2.0);
        let (bx, by, bw, bh) = sg.save;
        s.fill_rrect(bx, by, bw, bh, ui.pt(m::ICON_RADIUS), colors::SYSTEM_BLUE.now());
        put_centred(s, &mut self.labels, ui, "Save", MAC_BODY_PT, Weight::Semibold, ON_ACCENT, bx + bw / 2.0, by + bh / 2.0);
    }

    // diagnostics -----------------------------------------------------------------------------------------------

    fn open_diagnostics(&mut self) {
        self.sheet = None;
        self.diag_open = true;
        self.dirty = true;
    }

    /// Scores the answers of the open chat on a background thread; the result arrives on `diag_rx` and wakes the loop.
    fn run_diagnostics(&mut self) {
        if self.diag_running {
            return;
        }
        let pairs = self.current().map_or_else(Vec::new, |c| safety::exchanges(&c.messages));
        self.dirty = true;
        if pairs.is_empty() {
            self.diag_lines = vec!["This chat has no answers to evaluate yet.".into()];
            return;
        }
        self.diag_running = true;
        self.diag_lines = vec!["Running the evaluation. Each answer is scored by the model when a key is set.".into()];
        let guard = self.guard.clone();
        let key = self.key.clone();
        let model = self.model.clone();
        let tx = self.diag_tx.clone();
        let wake = self.wake_tx.as_ref().and_then(|w| w.try_clone().ok());
        DispatchQueue::global(Qos::UserInitiated).async_(move || {
            let result = safety::run_evaluation(&guard, key.as_deref(), &model, &pairs);
            let _ = tx.send(safety::report_lines(&result));
            if let Some(mut w) = wake {
                let _ = w.write(&[1]);
            }
        });
    }

    /// The sheet's text: what is guarded and how the last request was routed, then the evaluation's report.
    fn diagnostics_lines(&self) -> Vec<String> {
        let mut lines = vec![
            format!("Guardrail: answers are cut at {} characters, and phrases listed in {} are masked.", safety::MAX_ANSWER_CHARS, safety::BLOCKED_FILE),
            format!("Blocked phrases loaded: {}.", self.guard.phrases().len()),
            format!("Last request routed to: {}.", self.last_skill.unwrap_or("General")),
            "Evaluation: the newest answers of this chat, each scored from 0 to 1 by the checks and the model judge.".into(),
        ];
        if self.key.is_none() {
            lines.push("Add an API key in Settings to add the model judge (Helpfulness and Clarity).".into());
        }
        lines.extend(self.diag_lines.iter().cloned());
        lines
    }

    fn diag_geo(&self, ui: &Ui) -> DiagGeo {
        let (pw, ph) = (ui.pt(DIAG_W_PT).min(self.w as f32 - ui.pt(SPACE_XXL)), ui.pt(DIAG_H_PT).min(self.h as f32 - ui.pt(SPACE_XXL)));
        let (px, py) = ((self.w as f32 - pw) / 2.0, (self.h as f32 - ph) / 2.0);
        let m = ui.pt(SPACE_XL);
        let (bw, bh) = (ui.pt(DIAG_BUTTON_W_PT), ui.pt(DIAG_BUTTON_H_PT));
        let by = py + ph - m - bh;
        let run = (px + pw - m - bw, by, bw, bh);
        let close = (run.0 - ui.pt(SPACE_S) - bw, by, bw, bh);
        DiagGeo { panel: (px, py, pw, ph), close, run }
    }

    fn draw_diagnostics(&mut self, s: &mut Surface, ui: &Ui) {
        let dg = self.diag_geo(ui);
        let (px, py, pw, ph) = dg.panel;
        s.fill_rrect(px - 1.0, py - 1.0, pw + 2.0, ph + 2.0, ui.pt(m::CARD_RADIUS + 1.0), SEPARATOR_C.now());
        s.fill_rrect(px, py, pw, ph, ui.pt(m::CARD_RADIUS), CARD_BG.now());
        let inset = ui.pt(SPACE_XL);
        let label = LABEL.now();
        put(s, &mut self.labels, ui, "Diagnostics", MAC_TITLE1_PT, Weight::Semibold, label, px + inset, py + ui.pt(SPACE_XL));
        let lh = ui.pt(MAC_BODY_PT * 1.4);
        let width = pw - 2.0 * inset;
        let bottom = dg.run.1 - ui.pt(SPACE_M);
        let mut y = py + ui.pt(64.0);
        for line in self.diagnostics_lines() {
            for row in ui.wrap(&line, MAC_BODY_PT, Weight::Regular, width) {
                if y + lh > bottom {
                    break;
                }
                put(s, &mut self.labels, ui, &row, MAC_BODY_PT, Weight::Regular, label, px + inset, y);
                y += lh;
            }
            y += ui.pt(SPACE_S);
        }
        let (cx, cy, cw, ch) = dg.close;
        s.fill_rrect(cx, cy, cw, ch, ui.pt(m::ICON_RADIUS), FIELD_BG.now());
        put_centred(s, &mut self.labels, ui, "Close", MAC_BODY_PT, Weight::Regular, label, cx + cw / 2.0, cy + ch / 2.0);
        let (bx, by, bw, bh) = dg.run;
        let (fill, text) = if self.diag_running { (LABEL_3.now(), "Running…") } else { (colors::SYSTEM_BLUE.now(), "Run Evaluation") };
        s.fill_rrect(bx, by, bw, bh, ui.pt(m::ICON_RADIUS), fill);
        put_centred(s, &mut self.labels, ui, text, MAC_BODY_PT, Weight::Semibold, ON_ACCENT, bx + bw / 2.0, by + bh / 2.0);
    }

    /// The loop wakes on `wake_fd` while a reply streams; polling is only the fallback without the pipe.
    pub fn wake_fd(&self) -> Option<i32> {
        self.wake_rx.as_ref().map(AsRawFd::as_raw_fd)
    }

    pub fn next_wake(&self, t: f64) -> Option<f64> {
        ((self.wake_rx.is_none() && self.streaming.is_some()) || self.scroll.animating()).then_some(t + 33.0)
    }
}
