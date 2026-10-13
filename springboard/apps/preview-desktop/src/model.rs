//! Viewer state that needs no drawing: zoom, quarter turns, page navigation, markup in page
//! coordinates, the sidecar file that keeps markup, and the burn-in used by export. Everything here
//! is plain data, so the tests run without a window.

use bifrost_app::kit::{pack, Image};

/// The smallest and largest zoom factor the viewer goes to.
pub const ZOOM_MIN: f32 = 0.05;
pub const ZOOM_MAX: f32 = 8.0;
/// One Zoom In or Zoom Out step.
const ZOOM_STEP: f32 = 1.25;
/// Text pages are cut once they pass this many characters.
pub const TEXT_PAGE_CHARS: usize = 1500;

/// How the picture is scaled into the canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zoom {
    /// The whole page fits the canvas. Fit never enlarges a small picture past its own size.
    Fit,
    /// A fixed factor: 1.0 is actual size.
    Factor(f32),
}

impl Zoom {
    /// The scale factor for a `w` by `h` picture shown in a `view_w` by `view_h` canvas.
    pub fn scale(self, w: usize, h: usize, view_w: usize, view_h: usize) -> f32 {
        match self {
            Zoom::Factor(f) => f.clamp(ZOOM_MIN, ZOOM_MAX),
            Zoom::Fit => {
                if w == 0 || h == 0 || view_w == 0 || view_h == 0 {
                    return 1.0;
                }
                let s = (view_w as f32 / w as f32).min(view_h as f32 / h as f32);
                s.min(1.0).clamp(ZOOM_MIN, ZOOM_MAX)
            }
        }
    }

    /// The zoom one step in or out from the current scale.
    pub fn stepped(self, w: usize, h: usize, view_w: usize, view_h: usize, zoom_in: bool) -> Zoom {
        let current = self.scale(w, h, view_w, view_h);
        let next = if zoom_in { current * ZOOM_STEP } else { current / ZOOM_STEP };
        Zoom::Factor(next.clamp(ZOOM_MIN, ZOOM_MAX))
    }
}

/// Quarter turns clockwise, 0 to 3.
pub fn turn_size(w: usize, h: usize, quarters: u8) -> (usize, usize) {
    if quarters % 2 == 1 {
        (h, w)
    } else {
        (w, h)
    }
}

/// Rotates `img` clockwise by `quarters` quarter turns (the count is taken modulo 4).
pub fn rotate(img: &Image, quarters: u8) -> Image {
    let q = quarters % 4;
    if q == 0 || img.w == 0 || img.h == 0 {
        return img.clone();
    }
    let (w, h) = (img.w, img.h);
    let (ow, oh) = turn_size(w, h, q);
    let mut px = vec![0u32; ow * oh];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = match q {
                1 => (h - 1 - y, x),
                2 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            px[dy * ow + dx] = img.px[y * w + x];
        }
    }
    Image { w: ow, h: oh, px }
}

/// A rectangle in normalised page coordinates (0 to 1 across the unrotated page).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Norm {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Norm {
    /// The rectangle spanned by two corners, in any order.
    pub fn from_corners(a: (f32, f32), b: (f32, f32)) -> Norm {
        let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
        let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
        Norm { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
}

/// Maps a point of the unrotated page to the same point after `quarters` clockwise turns.
pub fn rot_point(u: f32, v: f32, quarters: u8) -> (f32, f32) {
    match quarters % 4 {
        1 => (1.0 - v, u),
        2 => (1.0 - u, 1.0 - v),
        3 => (v, 1.0 - u),
        _ => (u, v),
    }
}

/// The inverse of `rot_point`: a point of the turned picture back on the unrotated page.
pub fn unrot_point(a: f32, b: f32, quarters: u8) -> (f32, f32) {
    match quarters % 4 {
        1 => (b, 1.0 - a),
        2 => (1.0 - a, 1.0 - b),
        3 => (1.0 - b, a),
        _ => (a, b),
    }
}

/// A source-page rectangle as it sits on the turned picture (its corners mapped, then re-spanned).
pub fn rot_rect(r: Norm, quarters: u8) -> Norm {
    let a = rot_point(r.x, r.y, quarters);
    let b = rot_point(r.x + r.w, r.y + r.h, quarters);
    Norm::from_corners(a, b)
}

/// Which page is shown and how. The pan offset is in canvas pixels and resets on a page change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub page: usize,
    pub zoom: Zoom,
    pub quarters: u8,
    pub pan: (f32, f32),
    pub sidebar_scroll: f32,
}

impl Default for View {
    fn default() -> Self {
        View { page: 0, zoom: Zoom::Fit, quarters: 0, pan: (0.0, 0.0), sidebar_scroll: 0.0 }
    }
}

impl View {
    /// Shows page `page`, clamped to the page count. Returns whether the page changed.
    pub fn go(&mut self, page: usize, count: usize) -> bool {
        let last = count.saturating_sub(1);
        let next = page.min(last);
        let changed = next != self.page;
        self.page = next;
        if changed {
            self.pan = (0.0, 0.0);
        }
        changed
    }

    /// Moves by `delta` pages (negative goes back), clamped. Returns whether the page changed.
    pub fn step(&mut self, delta: i32, count: usize) -> bool {
        let target = (self.page as i64 + delta as i64).max(0) as usize;
        self.go(target, count)
    }

    /// Turns the picture by `delta` quarter turns (negative turns anticlockwise).
    pub fn turn(&mut self, delta: i32) {
        self.quarters = (self.quarters as i32 + delta).rem_euclid(4) as u8;
        self.pan = (0.0, 0.0);
    }
}

/// The colour adjustments of Adjust Color. The defaults leave the picture as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Adjust {
    /// Exposure in stops (EV), 0 is unchanged.
    pub exposure: f32,
    /// Saturation, 1 is unchanged.
    pub saturation: f32,
    /// Contrast, 1 is unchanged.
    pub contrast: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Adjust { exposure: 0.0, saturation: 1.0, contrast: 1.0 }
    }
}

impl Adjust {
    pub fn is_identity(&self) -> bool {
        *self == Adjust::default()
    }

    /// The named filters to chain, each with its one input value, skipping the unchanged ones.
    pub fn settings(&self) -> Vec<(&'static str, f32)> {
        let mut out = Vec::new();
        if self.exposure != 0.0 {
            out.push(("exposure", self.exposure));
        }
        if self.saturation != 1.0 {
            out.push(("saturation", self.saturation));
        }
        if self.contrast != 1.0 {
            out.push(("contrast", self.contrast));
        }
        out
    }
}

/// What a markup mark is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkKind {
    Highlight,
    Note,
    Rect,
    Rounded,
}

impl MarkKind {
    fn code(self) -> &'static str {
        match self {
            MarkKind::Highlight => "highlight",
            MarkKind::Note => "note",
            MarkKind::Rect => "rect",
            MarkKind::Rounded => "rounded",
        }
    }

    fn parse(s: &str) -> Option<MarkKind> {
        match s {
            "highlight" => Some(MarkKind::Highlight),
            "note" => Some(MarkKind::Note),
            "rect" => Some(MarkKind::Rect),
            "rounded" => Some(MarkKind::Rounded),
            _ => None,
        }
    }
}

/// One piece of markup. Its rectangle is in the unrotated page's normalised coordinates, so it
/// survives rotation and zoom.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    pub page: usize,
    pub rect: Norm,
    /// Packed colour (`pack`), straight alpha.
    pub color: u32,
    /// The note text. Empty for the other kinds.
    pub text: String,
}

/// A note's marker on an exported picture, in pixels.
const NOTE_PX: usize = 14;

/// Highlight colour: system yellow at about 45 per cent.
pub const HIGHLIGHT: u32 = pack(255, 214, 10, 115);
/// Note and shape colour: system blue.
pub const MARK_BLUE: u32 = pack(10, 132, 255, 255);

const SIDECAR_HEADER: &str = "# viewer markup 1";

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
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
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// The sidecar text for `marks`: a header line, then one tab-separated line per mark.
pub fn to_sidecar(marks: &[Mark]) -> String {
    let mut out = String::from(SIDECAR_HEADER);
    out.push('\n');
    for m in marks {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{:08x}\t{}\n",
            m.kind.code(),
            m.page,
            m.rect.x,
            m.rect.y,
            m.rect.w,
            m.rect.h,
            m.color,
            escape(&m.text)
        ));
    }
    out
}

/// Reads sidecar text back. A missing header or a malformed line is skipped, not an error, so a
/// damaged file keeps whatever marks it still has.
pub fn parse_sidecar(text: &str) -> Vec<Mark> {
    let mut lines = text.lines();
    if lines.next() != Some(SIDECAR_HEADER) {
        return Vec::new();
    }
    lines.filter_map(parse_line).collect()
}

fn parse_line(line: &str) -> Option<Mark> {
    let mut f = line.splitn(8, '\t');
    let kind = MarkKind::parse(f.next()?)?;
    let page = f.next()?.parse::<usize>().ok()?;
    let x = f.next()?.parse::<f32>().ok()?;
    let y = f.next()?.parse::<f32>().ok()?;
    let w = f.next()?.parse::<f32>().ok()?;
    let h = f.next()?.parse::<f32>().ok()?;
    let color = u32::from_str_radix(f.next()?, 16).ok()?;
    let text = unescape(f.next().unwrap_or(""));
    if ![x, y, w, h].iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(Mark { kind, page, rect: Norm { x, y, w, h }, color, text })
}

/// The file name for a document's sidecar: a 64-bit FNV-1a hash of its path, so the same file
/// always finds its markup and two files never share a sidecar by name.
pub fn sidecar_name(path: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}.markup")
}

/// Source-over of a straight-alpha colour `src` onto a premultiplied pixel `dst`.
fn over(dst: u32, src: u32) -> u32 {
    let sa = (src >> 24) & 0xff;
    let inv = 255 - sa;
    let ch = |shift: u32| {
        let s = (src >> shift) & 0xff;
        let d = (dst >> shift) & 0xff;
        ((s * sa + d * inv) / 255).min(255)
    };
    let da = (dst >> 24) & 0xff;
    let a = ((sa * 255 + da * inv) / 255).min(255);
    ch(0) | (ch(8) << 8) | (ch(16) << 16) | (a << 24)
}

/// Draws the marks of `page` onto a picture that is `quarters` turns from the page. Highlights
/// are tinted, rectangles and rounded rectangles get a 2 pixel outline, notes get a small marker.
/// The picture is the unrotated page scaled to its own size, so this runs on the export copy.
pub fn burn(img: &mut Image, marks: &[Mark], page: usize, quarters: u8) {
    let (w, h) = (img.w, img.h);
    for m in marks.iter().filter(|m| m.page == page) {
        let r = rot_rect(m.rect, quarters);
        let x0 = ((r.x * w as f32).round() as i64).clamp(0, w as i64) as usize;
        let y0 = ((r.y * h as f32).round() as i64).clamp(0, h as i64) as usize;
        let mut x1 = (((r.x + r.w) * w as f32).round() as i64).clamp(0, w as i64) as usize;
        let mut y1 = (((r.y + r.h) * h as f32).round() as i64).clamp(0, h as i64) as usize;
        if m.kind == MarkKind::Note {
            // a note is a small marker, whatever its zero-size anchor
            x1 = (x0 + NOTE_PX).min(w);
            y1 = (y0 + NOTE_PX).min(h);
        }
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        match m.kind {
            MarkKind::Highlight => {
                for y in y0..y1 {
                    for x in x0..x1 {
                        let i = y * w + x;
                        img.px[i] = over(img.px[i], m.color);
                    }
                }
            }
            MarkKind::Rect | MarkKind::Rounded | MarkKind::Note => {
                let c = if m.kind == MarkKind::Note { MARK_BLUE } else { m.color };
                let t = 2usize;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let edge = x < x0 + t || x + t >= x1 || y < y0 + t || y + t >= y1;
                        if edge {
                            let i = y * w + x;
                            img.px[i] = over(img.px[i], c);
                        }
                    }
                }
            }
        }
    }
}

/// Cuts a run of text parts into pages of about `limit` characters, at part boundaries. A part
/// longer than the limit becomes a page of its own.
pub fn chunk_text(parts: &[&str], limit: usize) -> Vec<String> {
    let mut pages = Vec::new();
    let mut cur = String::new();
    for part in parts.iter().filter(|p| !p.trim().is_empty()) {
        if !cur.is_empty() && cur.chars().count() + part.chars().count() > limit {
            pages.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push_str("\n\n");
        }
        cur.push_str(part.trim());
    }
    if !cur.is_empty() {
        pages.push(cur);
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize) -> Image {
        // each pixel gets a distinct value so a rotation mistake shows up
        Image { w, h, px: (0..w * h).map(|i| pack(i as u8, 0, 0, 255)).collect() }
    }

    #[test]
    fn fit_scales_down_and_never_enlarges() {
        assert_eq!(Zoom::Fit.scale(2000, 1000, 1000, 1000), 0.5);
        assert_eq!(Zoom::Fit.scale(100, 100, 1000, 1000), 1.0);
        assert_eq!(Zoom::Fit.scale(0, 0, 10, 10), 1.0);
    }

    #[test]
    fn stepping_zoom_is_clamped() {
        let z = Zoom::Factor(ZOOM_MAX).stepped(10, 10, 10, 10, true);
        assert_eq!(z, Zoom::Factor(ZOOM_MAX));
        let z = Zoom::Factor(ZOOM_MIN).stepped(10, 10, 10, 10, false);
        assert_eq!(z, Zoom::Factor(ZOOM_MIN));
        let z = Zoom::Fit.stepped(100, 100, 50, 50, true);
        assert_eq!(z, Zoom::Factor(0.5 * ZOOM_STEP));
    }

    #[test]
    fn four_quarter_turns_return_the_original() {
        let src = img(3, 2);
        let mut cur = src.clone();
        for _ in 0..4 {
            cur = rotate(&cur, 1);
        }
        assert_eq!(cur, src);
    }

    #[test]
    fn quarter_turn_swaps_the_size_and_moves_the_top_left_pixel() {
        let src = img(2, 3);
        let out = rotate(&src, 1);
        assert_eq!((out.w, out.h), (3, 2));
        // the top-left source pixel (0,0) ends at the top-right corner after a clockwise turn
        assert_eq!(out.px[out.w - 1], src.px[0]);
        assert_eq!(rotate(&src, 5), rotate(&src, 1));
    }

    #[test]
    fn point_mapping_round_trips_for_every_turn() {
        for q in 0..4u8 {
            let (a, b) = rot_point(0.2, 0.7, q);
            let (u, v) = unrot_point(a, b, q);
            assert!((u - 0.2).abs() < 1e-6 && (v - 0.7).abs() < 1e-6, "turn {q}");
        }
    }

    #[test]
    fn rect_mapping_matches_the_rotated_picture() {
        let r = Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.25 };
        let turned = rot_rect(r, 1);
        assert!((turned.x - 0.75).abs() < 1e-6 && (turned.y - 0.0).abs() < 1e-6);
        assert!((turned.w - 0.25).abs() < 1e-6 && (turned.h - 0.5).abs() < 1e-6);
        let back = Norm::from_corners(unrot_point(turned.x, turned.y, 1), unrot_point(turned.x + turned.w, turned.y + turned.h, 1));
        assert!((back.x - 0.0).abs() < 1e-6 && (back.w - 0.5).abs() < 1e-6);
    }

    #[test]
    fn navigation_clamps_and_resets_pan() {
        let mut v = View::default();
        v.pan = (5.0, 5.0);
        assert!(!v.step(-1, 3));
        assert!(v.step(10, 3));
        assert_eq!(v.page, 2);
        assert_eq!(v.pan, (0.0, 0.0));
        v.turn(-1);
        assert_eq!(v.quarters, 3);
    }

    #[test]
    fn default_adjust_adds_no_filters() {
        assert!(Adjust::default().is_identity());
        assert!(Adjust::default().settings().is_empty());
        let a = Adjust { exposure: 0.5, saturation: 1.0, contrast: 1.2 };
        assert_eq!(a.settings(), vec![("exposure", 0.5), ("contrast", 1.2)]);
    }

    #[test]
    fn sidecar_round_trips_text_with_tabs_and_newlines() {
        let marks = vec![
            Mark { kind: MarkKind::Note, page: 2, rect: Norm { x: 0.1, y: 0.2, w: 0.0, h: 0.0 }, color: MARK_BLUE, text: "a\tb\nc\\d".into() },
            Mark { kind: MarkKind::Highlight, page: 0, rect: Norm { x: 0.3, y: 0.4, w: 0.2, h: 0.05 }, color: HIGHLIGHT, text: String::new() },
        ];
        let text = to_sidecar(&marks);
        assert_eq!(parse_sidecar(&text), marks);
    }

    #[test]
    fn sidecar_skips_bad_lines_and_wrong_headers() {
        assert!(parse_sidecar("not a viewer file\nrect\t0\t0\t0\t1\t1\tff0000ff\t").is_empty());
        let text = format!("{SIDECAR_HEADER}\nbogus line\nrect\t0\tnope\t0\t1\t1\tff0000ff\t\nrect\t1\t0\t0\t1\t1\tff0000ff\tok\n");
        let got = parse_sidecar(&text);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].page, 1);
    }

    #[test]
    fn sidecar_name_is_stable_and_distinct() {
        assert_eq!(sidecar_name("/a/b.png"), sidecar_name("/a/b.png"));
        assert_ne!(sidecar_name("/a/b.png"), sidecar_name("/a/c.png"));
        assert!(sidecar_name("/a/b.png").ends_with(".markup"));
    }

    #[test]
    fn burn_outlines_a_rectangle_and_tints_a_highlight() {
        let mut picture = Image { w: 10, h: 10, px: vec![pack(255, 255, 255, 255); 100] };
        let marks = vec![
            Mark { kind: MarkKind::Rect, page: 0, rect: Norm { x: 0.0, y: 0.0, w: 0.5, h: 0.5 }, color: MARK_BLUE, text: String::new() },
            Mark { kind: MarkKind::Highlight, page: 1, rect: Norm { x: 0.0, y: 0.0, w: 1.0, h: 1.0 }, color: HIGHLIGHT, text: String::new() },
        ];
        burn(&mut picture, &marks, 0, 0);
        assert_ne!(picture.px[0], pack(255, 255, 255, 255));
        // the interior of the outline is untouched
        assert_eq!(picture.px[2 * 10 + 2], pack(255, 255, 255, 255));
        // the highlight on page 1 is not drawn on page 0
        assert_eq!(picture.px[99], pack(255, 255, 255, 255));
    }

    #[test]
    fn text_chunks_break_at_parts_and_skip_blanks() {
        let parts = ["alpha", "  ", "beta", "gamma"];
        assert_eq!(chunk_text(&parts, 100), vec!["alpha\n\nbeta\n\ngamma".to_string()]);
        let pages = chunk_text(&parts, 9);
        assert_eq!(pages, vec!["alpha".to_string(), "beta\n\ngamma".to_string()]);
        assert!(chunk_text(&[], 10).is_empty());
    }
}
