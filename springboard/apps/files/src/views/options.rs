//! Files's view settings shared by the four views: gallery mode, sort direction, Group By, icon and text size,
//! column width, inline-expanded folders, thumbnails, and the Show View Options panel (Cmd+J).
use crate::*;
use springboard::gfx::Image;
use std::cell::RefCell;
use std::collections::HashMap;

/// Group By choices (Files: None, Kind, Date Modified).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    None,
    Kind,
    Date,
}

impl Group {
    pub(crate) const ALL: [Group; 3] = [Group::None, Group::Kind, Group::Date];
    pub(crate) fn label(self) -> &'static str {
        match self {
            Group::None => "None",
            Group::Kind => "Kind",
            Group::Date => "Date",
        }
    }
    /// Sort key (and caption) of the group an entry belongs to.
    pub(crate) fn key(self, e: &Entry) -> (u8, String) {
        match self {
            Group::None => (0, String::new()),
            Group::Kind => (0, fs::kind_description(&e.name, e.is_dir, e.kind).to_string()),
            Group::Date => {
                let (n, label) = date_bucket(e.mtime);
                (n, label.to_string())
            }
        }
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Files's date groups: Today, Yesterday, Previous 7 Days, Previous 30 Days, Earlier.
pub(crate) fn date_bucket(mtime: i64) -> (u8, &'static str) {
    if mtime <= 0 {
        return (4, "Earlier");
    }
    let age = now_secs().div_euclid(86400) - mtime.div_euclid(86400);
    match age {
        i64::MIN..=0 => (0, "Today"),
        1 => (1, "Yesterday"),
        2..=7 => (2, "Previous 7 Days"),
        8..=30 => (3, "Previous 30 Days"),
        _ => (4, "Earlier"),
    }
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// "Today at 14:05", "Yesterday at 09:30" or "Oct 11, 2026" (UTC; the SDK has no time zone yet).
pub(crate) fn date_text(mtime: i64) -> String {
    if mtime <= 0 {
        return "—".into();
    }
    let clock = format!("{:02}:{:02}", mtime.rem_euclid(86400) / 3600, mtime.rem_euclid(3600) / 60);
    match date_bucket(mtime) {
        (0, _) => format!("Today at {clock}"),
        (1, _) => format!("Yesterday at {clock}"),
        _ => {
            let (y, m, d) = civil(mtime.div_euclid(86400));
            const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            format!("{} {d}, {y}", MONTHS[(m as usize + 11) % 12])
        }
    }
}

/// A small solid triangle drawn from rects (no glyph needed): `dir` 0 points right, 1 down, 2 up.
pub(crate) fn triangle(s: &mut Surface, ui: &Ui, x: f32, y: f32, dir: u8, color: u32) {
    for k in 0..5 {
        let k = k as f32;
        match dir {
            0 => rect(s, ui, x + k, y + k, 1.2, 9. - 2. * k, color),
            1 => rect(s, ui, x + k, y + k, 9. - 2. * k, 1.2, color),
            _ => rect(s, ui, x + k, y + 4. - k, 9. - 2. * k, 1.2, color),
        }
    }
}

pub(crate) const STRIP_H: f32 = 92.;
pub(crate) const STRIP_CELL: f32 = 76.;
pub(crate) const PANEL_W: f32 = 300.;
pub(crate) const PANEL_H: f32 = 176.;

/// What a click in the View Options panel does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum OptAct {
    Icon(f32),
    Text(f32),
    Group(Group),
    SortBy(Sort),
}

pub(crate) struct ViewOpts {
    /// Gallery view is on (with `grid` and `columns` false).
    pub(crate) gallery: bool,
    /// Sort direction: true is the sort's natural order (A to Z, newest first, largest first).
    pub(crate) asc: bool,
    pub(crate) group: Group,
    /// Icon view: icon side in pt. Text size in pt for icon labels and list names.
    pub(crate) icon: f32,
    pub(crate) text: f32,
    /// Columns view: one column's width in pt, and which separator (1 based) is being dragged, 0 for none.
    pub(crate) col_w: f32,
    pub(crate) resize_k: usize,
    /// The Show View Options panel is open.
    pub(crate) show: bool,
    /// List view: folders opened inline, and the nesting depth of every row (parallel to `rows`).
    pub(crate) expanded: BTreeSet<PathBuf>,
    pub(crate) depth: Vec<usize>,
    /// Decoded thumbnails by (path, mtime, width px, height px); None when the file is not a decodable picture.
    pub(crate) thumbs: RefCell<HashMap<(PathBuf, i64, usize, usize), Option<Image>>>,
}

impl Default for ViewOpts {
    fn default() -> Self {
        ViewOpts {
            gallery: false,
            asc: true,
            group: Group::None,
            icon: GRID_ICON,
            text: MAC_CALLOUT_PT,
            col_w: COLUMN_W,
            resize_k: 0,
            show: false,
            expanded: BTreeSet::new(),
            depth: vec![],
            thumbs: RefCell::new(HashMap::new()),
        }
    }
}

impl Files {
    pub(crate) fn grid_icon(&self) -> f32 {
        self.vo.icon
    }
    pub(crate) fn grid_w(&self) -> f32 {
        self.vo.icon + 40.
    }
    pub(crate) fn grid_h(&self) -> f32 {
        self.vo.icon + 44.
    }
    pub(crate) fn is_list(&self) -> bool {
        !self.grid && !self.columns && !self.vo.gallery
    }
    pub(crate) fn depth_of(&self, i: usize) -> usize {
        self.vo.depth.get(i).copied().unwrap_or(0)
    }
    pub(crate) fn set_gallery(&mut self) {
        self.set_view(false, false);
        self.vo.gallery = true;
    }
    /// Sorts a listing the way the view settings say: by `sort` (reversed when not ascending), grouped if asked.
    pub(crate) fn order(&self, v: &mut Vec<Entry>) {
        fs::sort(v, self.sort);
        if !self.vo.asc {
            v.reverse();
        }
        match self.vo.group {
            Group::None => v.sort_by_key(|e| !e.is_dir),
            g => v.sort_by_cached_key(|e| g.key(e)),
        }
    }
    /// A click on a list column header (or a Sort By choice): the same sort again flips the direction.
    pub(crate) fn sort_by(&mut self, by: Sort) {
        if self.sort == by {
            self.vo.asc = !self.vo.asc;
        } else {
            self.sort = by;
            self.vo.asc = true;
        }
        self.sort_rows();
        self.revision += 1;
    }
    /// Opens or closes a folder row inline (list view).
    pub(crate) fn toggle_expand(&mut self, i: usize) {
        let Some(e) = self.rows.get(i).filter(|e| e.is_dir) else { return };
        let path = e.path.clone();
        if !self.vo.expanded.remove(&path) {
            self.vo.expanded.insert(path);
        }
        self.selection.clear();
        self.refresh_rows();
        self.revision += 1;
    }
    /// `base` with the children of every expanded folder spliced in under it; fills `vo.depth`.
    pub(crate) fn expand_rows(&mut self, base: Vec<Entry>) -> Vec<Entry> {
        let mut depth = vec![];
        if self.vo.expanded.is_empty() || !self.is_list() || self.hits.is_some() {
            depth.resize(base.len(), 0);
            self.vo.depth = depth;
            return base;
        }
        let mut out = vec![];
        self.splice(base, 0, &mut out, &mut depth);
        self.vo.depth = depth;
        out
    }
    fn splice(&self, level: Vec<Entry>, d: usize, out: &mut Vec<Entry>, depth: &mut Vec<usize>) {
        for e in level {
            let open = e.is_dir && d < 8 && self.vo.expanded.contains(&e.path);
            let path = e.path.clone();
            out.push(e);
            depth.push(d);
            if open {
                if let Ok(mut kids) = fs::list(&path) {
                    self.order(&mut kids);
                    self.splice(kids, d + 1, out, depth);
                }
            }
        }
    }
    /// A picture's thumbnail fitted into the box (x, y, w, h in pt) and centred in it; false when the file is not a
    /// decodable picture or is bigger than `max_bytes`.
    pub(crate) fn blit_thumb(&self, s: &mut Surface, ui: &Ui, e: &Entry, b: (f32, f32, f32, f32), max_bytes: u64) -> bool {
        if e.is_dir || e.size > max_bytes || !springboard::imgdec::is_image_name(&e.name) {
            return false;
        }
        let (pw, ph) = (ui.pt(b.2) as usize, ui.pt(b.3) as usize);
        let mut cache = self.vo.thumbs.borrow_mut();
        if cache.len() > 96 {
            cache.clear();
        }
        let img = cache.entry((e.path.clone(), e.mtime, pw, ph)).or_insert_with(|| {
            std::fs::read(&e.path).ok().and_then(|bytes| springboard::imgdec::decode(&bytes)).map(|i| springboard::imgdec::fit(&i, pw, ph))
        });
        match img {
            Some(im) => {
                let x = ui.pt(b.0) as i32 + (pw as i32 - im.w as i32) / 2;
                let y = ui.pt(b.1) as i32 + (ph as i32 - im.h as i32) / 2;
                s.blit(im, x, y, 255, 255);
                true
            }
            None => false,
        }
    }
    /// An entry's icon (folder or file type) in a square of `side` pt, or its picture thumbnail.
    pub(crate) fn paint_entry_icon(&self, ui: &Ui, s: &mut Surface, e: &Entry, x: f32, y: f32, side: f32, thumbs: bool) {
        if thumbs && self.blit_thumb(s, ui, e, (x + 2., y + 2., side - 4., side - 4.), 4 << 20) {
            return;
        }
        if e.is_dir {
            widgets::folder_icon(s, ui, Rect { x, y, w: side, h: side }, folder_emblem(&self.home, &e.path));
        } else {
            icons::entry(s, (ui.pt(x + side * 0.1), ui.pt(y + side * 0.06), ui.pt(side * 0.8), ui.pt(side * 0.86)), e.kind);
        }
    }
    fn centred(&self, s: &mut Surface, ui: &Ui, label: &str, pt: f32, color: u32, cx: f32, y: f32, max: f32) {
        let label = ui.fit(label, pt, ui.pt(max));
        let im = ui.text(&label, pt, Weight::Regular, color);
        s.blit(&im, ui.pt(cx) as i32 - im.w as i32 / 2, ui.pt(y) as i32, 255, 255);
    }

    // ---- gallery ----

    /// The strip's top edge, how many thumbnails fit, and the first one shown (the focus is kept in view).
    fn strip_geom(&self, visible: &[usize]) -> (f32, usize, usize) {
        let cols = (((self.w - (self.side() + 16.) - 8.) / STRIP_CELL).floor().max(1.)) as usize;
        let at = self.focus().and_then(|f| visible.iter().position(|&j| j == f)).unwrap_or(0);
        let first = at.saturating_sub(cols / 2).min(visible.len().saturating_sub(cols));
        (self.bottom() - STRIP_H, cols, first)
    }
    /// A click in the gallery: a strip thumbnail picks that item; the preview area keeps the current one.
    pub(crate) fn gallery_hit(&self, x: f32, y: f32) -> Option<usize> {
        if x < self.side() + 16. || y < HEADER_H || y > self.bottom() {
            return None;
        }
        let visible = self.visible();
        let (top, cols, first) = self.strip_geom(&visible);
        if y >= top {
            let k = ((x - (self.side() + 16.)) / STRIP_CELL).floor();
            if k >= 0. && (k as usize) < cols {
                return visible.get(first + k as usize).copied();
            }
            return None;
        }
        self.focus().filter(|f| visible.contains(f))
    }
    pub(crate) fn paint_gallery(&self, ui: &Ui, s: &mut Surface, visible: &[usize]) {
        let cur = match self.focus().filter(|f| visible.contains(f)).or(visible.first().copied()) {
            Some(i) => i,
            None => return,
        };
        let (top, cols, first) = self.strip_geom(visible);
        let (x0, y0, x1) = (self.side() + 16., HEADER_H + 6., self.w - 16.);
        let y1 = top - 8.;
        s.fill_rrect(ui.pt(x0), ui.pt(y0), ui.pt(x1 - x0), ui.pt((y1 - y0).max(0.)), ui.pt(CARD_R), CARD_BG.now());
        let e = &self.rows[cur];
        let info_h = 56.;
        let box_h = (y1 - y0 - info_h - 16.).max(32.);
        let cx = (x0 + x1) / 2.;
        let shown = self.blit_thumb(s, ui, e, (x0 + 8., y0 + 8., x1 - x0 - 16., box_h), 32 << 20);
        if !shown {
            let side = box_h.min(128.).min(x1 - x0 - 16.);
            self.paint_entry_icon(ui, s, e, cx - side / 2., y0 + 8. + (box_h - side) / 2., side, false);
        }
        let ty = y1 - info_h + 4.;
        self.centred(s, ui, &e.name, MAC_TITLE3_PT, LABEL.now(), cx, ty, x1 - x0 - 32.);
        let detail = if e.is_dir {
            format!("Folder · {}", date_text(e.mtime))
        } else {
            format!("{} · {} · {}", e.kind.label(), fs::size_text(e.size), date_text(e.mtime))
        };
        self.centred(s, ui, &detail, 12., LABEL_2.now(), cx, ty + 24., x1 - x0 - 32.);
        // the thumbnail strip
        rect(s, ui, self.side(), top, self.w - self.side(), 1., SEPARATOR_C.now());
        for k in 0..cols {
            let Some(&i) = visible.get(first + k) else { break };
            let e = &self.rows[i];
            let x = x0 + k as f32 * STRIP_CELL;
            let selected = i == cur;
            if selected {
                s.fill_rrect(ui.pt(x + 2.), ui.pt(top + 6.), ui.pt(STRIP_CELL - 4.), ui.pt(STRIP_H - 10.), ui.pt(SPACE_S), SELECTION.now());
            }
            self.paint_entry_icon(ui, s, e, x + 10., top + 10., 56., true);
            self.centred(s, ui, &e.name, 10., ink(selected), x + STRIP_CELL / 2., top + 68., STRIP_CELL - 8.);
        }
    }

    // ---- columns view: resize and preview ----

    /// The separator (1 based) under a point in the columns view, to start a resize.
    pub(crate) fn column_edge(&self, x: f32, y: f32) -> Option<usize> {
        if !self.columns || y < HEADER_H || y > self.bottom() || x < self.side() + 16. {
            return None;
        }
        let w = self.vo.col_w;
        let rel = x - (self.side() + 16.);
        let k = (rel / w).round();
        let max = self.ancestor_shown() + 1;
        (k >= 1. && (k as usize) <= max && (rel - k * w).abs() <= 4.).then_some(k as usize)
    }
    pub(crate) fn resize_to(&mut self, x: f32) {
        if self.vo.resize_k > 0 {
            self.vo.col_w = ((x - (self.side() + 16.)) / self.vo.resize_k as f32).clamp(140., 360.);
            self.revision += 1;
        }
    }
    /// The preview column right of the open folder's column: the single selected file, large, with its details.
    pub(crate) fn paint_column_preview(&self, ui: &Ui, s: &mut Surface, x: f32) {
        let [i] = self.selection[..] else { return };
        let Some(e) = self.rows.get(i).filter(|e| !e.is_dir) else { return };
        let w = (self.w - x - 16.).min(self.vo.col_w * 1.6);
        if w < 96. {
            return;
        }
        rect(s, ui, x, HEADER_H, 1., self.bottom() - HEADER_H, SEPARATOR_C.now());
        let side = (w - 24.).min(160.);
        let shown = self.blit_thumb(s, ui, e, (x + 12., ROWS_TOP + 8., w - 24., side), 32 << 20);
        if !shown {
            let icon = side.min(96.);
            self.paint_entry_icon(ui, s, e, x + 12. + (w - 24. - icon) / 2., ROWS_TOP + 8., icon, false);
        }
        let y = ROWS_TOP + side + 20.;
        text(s, ui, &e.name, 13., LABEL.now(), x + 12., y, w - 24.);
        text(s, ui, &format!("{} · {}", e.kind.label(), fs::size_text(e.size)), 11., LABEL_2.now(), x + 12., y + 20., w - 24.);
        text(s, ui, &format!("Modified {}", date_text(e.mtime)), 11., LABEL_2.now(), x + 12., y + 38., w - 24.);
    }

    // ---- Show View Options ----

    fn panel_rect(&self) -> Rect {
        let x = (self.w - PANEL_W - 16.).max(self.side() + 16.);
        Rect { x, y: HEADER_H + 6., w: PANEL_W.min(self.w - x - 8.), h: PANEL_H }
    }
    /// The panel's clickable plates, in paint order.
    fn panel_hits(&self) -> Vec<(Rect, OptAct)> {
        let p = self.panel_rect();
        let mut out = vec![];
        let r = |x: f32, y: f32, w: f32| Rect { x, y, w, h: 22. };
        out.push((r(p.x + p.w - 96., p.y + 40., 28.), OptAct::Icon(-16.)));
        out.push((r(p.x + p.w - 44., p.y + 40., 28.), OptAct::Icon(16.)));
        out.push((r(p.x + p.w - 96., p.y + 70., 28.), OptAct::Text(-1.)));
        out.push((r(p.x + p.w - 44., p.y + 70., 28.), OptAct::Text(1.)));
        for (k, g) in Group::ALL.into_iter().enumerate() {
            out.push((r(p.x + 88. + k as f32 * 62., p.y + 100., 58.), OptAct::Group(g)));
        }
        for (k, by) in Sort::ALL.into_iter().enumerate() {
            out.push((r(p.x + 88. + k as f32 * 52., p.y + 134., 48.), OptAct::SortBy(by)));
        }
        out
    }
    /// A click while the panel is open. Returns true when the panel took it (a click outside closes the panel and
    /// falls through to the view).
    pub(crate) fn options_click(&mut self, x: f32, y: f32) -> bool {
        if !self.vo.show {
            return false;
        }
        let p = self.panel_rect();
        let inside = x >= p.x && x < p.x + p.w && y >= p.y && y < p.y + p.h;
        if !inside {
            self.vo.show = false;
            self.revision += 1;
            return false;
        }
        let act = self.panel_hits().into_iter().find(|(r, _)| x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h).map(|(_, a)| a);
        match act {
            Some(OptAct::Icon(d)) => self.vo.icon = (self.vo.icon + d).clamp(32., 128.),
            Some(OptAct::Text(d)) => self.vo.text = (self.vo.text + d).clamp(10., 16.),
            Some(OptAct::Group(g)) => {
                self.vo.group = g;
                self.sort_rows();
            }
            Some(OptAct::SortBy(by)) => {
                self.sort = by;
                self.vo.asc = true;
                self.sort_rows();
            }
            None => {}
        }
        self.scroll = 0.;
        self.revision += 1;
        true
    }
    pub(crate) fn paint_options(&self, ui: &Ui, s: &mut Surface) {
        if !self.vo.show {
            return;
        }
        let p = self.panel_rect();
        s.fill_rrect(ui.pt(p.x - 1.), ui.pt(p.y - 1.), ui.pt(p.w + 2.), ui.pt(p.h + 2.), ui.pt(CARD_R), SEPARATOR_C.now());
        s.fill_rrect(ui.pt(p.x), ui.pt(p.y), ui.pt(p.w), ui.pt(p.h), ui.pt(CARD_R), DETAIL_BG.now());
        text(s, ui, "View Options", MAC_HEADLINE_PT, LABEL.now(), p.x + 16., p.y + 14., p.w - 32.);
        for (name, dy) in [("Icon size", 40.), ("Text size", 70.), ("Group By", 100.), ("Sort By", 134.)] {
            text(s, ui, name, MAC_CALLOUT_PT, LABEL_2.now(), p.x + 16., p.y + dy + 4., 70.);
        }
        text(s, ui, &format!("{}", self.vo.icon as i32), MAC_CALLOUT_PT, LABEL.now(), p.x + p.w - 62., p.y + 44., 18.);
        text(s, ui, &format!("{}", self.vo.text as i32), MAC_CALLOUT_PT, LABEL.now(), p.x + p.w - 62., p.y + 74., 18.);
        for (r, act) in self.panel_hits() {
            let (name, on) = match act {
                OptAct::Icon(d) | OptAct::Text(d) => (if d < 0. { "−" } else { "+" }, false),
                OptAct::Group(g) => (g.label(), g == self.vo.group),
                OptAct::SortBy(by) => (by.label(), by == self.sort),
            };
            s.fill_rrect(ui.pt(r.x), ui.pt(r.y), ui.pt(r.w), ui.pt(r.h), ui.pt(CELL_RADIUS_PT), if on { SELECTION.now() } else { CARD_BG.now() });
            let wpt = ui.measure(name, MAC_CALLOUT_PT, Weight::Regular) / ui.pt(1.);
            text(s, ui, name, MAC_CALLOUT_PT, ink(on), r.x + (r.w - wpt) / 2., r.y + 4., r.w);
        }
    }
}
