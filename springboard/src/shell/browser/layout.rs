//! Block and inline layout of a styled tree into a flat display list.
//!
//! Blocks stack vertically with collapsing margins, padding and borders,
//! `margin: auto` centring and `width`/`max-width`; inline content (text,
//! links, images, `<br>`) is broken into lines greedily with `text-align`.
//! Everything is measured through [`Metrics`], so layout is exercised in
//! tests with a fixed-width fake and on the device with the real font.
//! Units are CSS pixels; the painter scales them to the screen.

use super::css::{Align, Color, Display, Kind, Len, ListStyle, Style, Styled, WhiteSpace};
use std::collections::HashMap;

/// How wide text is, in CSS px.
pub trait Metrics {
    fn width(&self, text: &str, size: f32, bold: bool) -> f32;
}

/// Height of a glyph row relative to the font size.
pub const GLYPH_H: f32 = 1.25;
/// Marks text that belongs to no selectable block (list markers).
pub const NO_BLOCK: usize = usize::MAX;
const MARKER_GAP: f32 = 6.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Rect { x: f32, y: f32, w: f32, h: f32, color: Color },
    Text { x: f32, y: f32, w: f32, h: f32, size: f32, text: String, bold: bool, italic: bool, underline: bool, color: Color, link: Option<usize>, block: usize },
    Image { x: f32, y: f32, w: f32, h: f32, src: String, link: Option<usize> },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub items: Vec<Item>,
    /// raw `href`s, indexed by `Item::Text::link`
    pub links: Vec<String>,
    /// the text of each paragraph-like run, for selection and copy
    pub blocks: Vec<String>,
    /// raw `src`s of the images the page shows
    pub images: Vec<String>,
    pub background: Option<Color>,
}

impl Page {
    /// The link under (x, y), if any.
    pub fn link_at(&self, x: f32, y: f32) -> Option<usize> {
        self.items.iter().rev().find_map(|it| match it {
            Item::Text { x: ix, y: iy, w, h, link: Some(l), .. } | Item::Image { x: ix, y: iy, w, h, link: Some(l), .. } if hits(*ix, *iy, *w, *h, x, y, 0.0) => Some(*l),
            _ => None,
        })
    }

    /// The selectable block whose text lies within `slop` px of (x, y).
    pub fn block_at(&self, x: f32, y: f32, slop: f32) -> Option<usize> {
        self.items.iter().find_map(|it| match it {
            Item::Text { x: ix, y: iy, w, h, block, .. } if *block != NO_BLOCK && hits(*ix, *iy, *w, *h, x, y, slop) => Some(*block),
            _ => None,
        })
    }

    /// Every block, paragraphs separated by a blank line.
    pub fn text(&self) -> String {
        self.blocks.iter().filter(|b| !b.is_empty()).cloned().collect::<Vec<_>>().join("\n\n")
    }
}

fn hits(ix: f32, iy: f32, w: f32, h: f32, x: f32, y: f32, slop: f32) -> bool {
    x >= ix - slop && x < ix + w + slop && y >= iy - slop && y < iy + h + slop
}

#[derive(Clone)]
struct Face {
    size: f32,
    bold: bool,
    italic: bool,
    underline: bool,
    color: Color,
    bg: Option<Color>,
    link: Option<usize>,
    lh: f32,
    nowrap: bool,
}

enum Atom {
    Word { text: String, w: f32, face: Face },
    Space { w: f32, face: Face },
    Break { lh: f32 },
    Image { w: f32, h: f32, src: String, link: Option<usize> },
}

struct Line {
    atoms: Vec<(f32, Atom)>,
    width: f32,
    h: f32,
    hard: bool,
}

impl Line {
    fn new() -> Line {
        Line { atoms: vec![], width: 0.0, h: 0.0, hard: false }
    }

    fn push(&mut self, atom: Atom, w: f32, h: f32) {
        self.atoms.push((self.width, atom));
        self.width += w;
        self.h = self.h.max(h);
    }

    /// Drops the space a wrap leaves at the end of the line.
    fn trim(&mut self) {
        if let Some((x, Atom::Space { .. })) = self.atoms.last() {
            self.width = *x;
            self.atoms.pop();
        }
    }
}

struct Marker {
    text: String,
    size: f32,
    color: Color,
    /// content edge of the list item
    x: f32,
}

struct Lay<'a> {
    m: &'a dyn Metrics,
    sizes: &'a HashMap<String, (usize, usize)>,
    items: Vec<Item>,
    links: Vec<String>,
    blocks: Vec<String>,
    images: Vec<String>,
    y: f32,
    /// the bottom margin waiting to collapse with the next block's top margin
    margin: f32,
    marker: Option<Marker>,
    cur_link: Option<usize>,
    avail: f32,
}

fn collapse(a: f32, b: f32) -> f32 {
    if a >= 0.0 && b >= 0.0 {
        a.max(b)
    } else if a < 0.0 && b < 0.0 {
        a.min(b)
    } else {
        a + b
    }
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\n' | '\t' | '\r' | '\x0c')
}

fn usable_href(h: &str) -> bool {
    let h = h.trim().to_ascii_lowercase();
    !(h.is_empty() || h.starts_with('#') || h.starts_with("javascript:") || h.starts_with("mailto:") || h.starts_with("tel:"))
}

/// Lays `tree` out in a viewport `width` px wide. `sizes` maps an image
/// `src` to its decoded size once known.
pub fn layout(tree: &[Styled], m: &dyn Metrics, width: f32, sizes: &HashMap<String, (usize, usize)>) -> Page {
    let mut l = Lay { m, sizes, items: vec![], links: vec![], blocks: vec![], images: vec![], y: 0.0, margin: 0.0, marker: None, cur_link: None, avail: width };
    l.children(tree, 0.0, width, None);
    let height = l.y + l.margin.max(0.0);
    Page { width, height, items: l.items, links: l.links, blocks: l.blocks, images: l.images, background: background(tree) }
}

fn background(tree: &[Styled]) -> Option<Color> {
    for n in tree {
        if matches!(n.tag(), Some("html" | "body")) {
            if let Some(c) = n.style.background.or_else(|| background(n.children())) {
                return Some(c);
            }
        }
    }
    None
}

/// Whether `n` takes part in an inline run rather than being a block.
fn is_inline(n: &Styled) -> bool {
    match n.tag() {
        None => true,
        Some("img" | "br") => true,
        Some(_) => n.style.display == Display::Inline,
    }
}

impl Lay<'_> {
    fn children(&mut self, kids: &[Styled], x: f32, w: f32, list: Option<(bool, i64)>) {
        let mut run: Vec<&Styled> = vec![];
        let (ordered, mut counter) = list.unwrap_or((false, 1));
        for k in kids {
            if is_inline(k) {
                run.push(k);
                continue;
            }
            self.flush(&mut run, x, w);
            let marker = (k.style.display == Display::ListItem).then(|| {
                let n = counter;
                counter += 1;
                match k.style.list_style {
                    ListStyle::None => None,
                    ListStyle::Decimal => Some(format!("{n}.")),
                    ListStyle::Disc if ordered => Some(format!("{n}.")),
                    ListStyle::Disc => Some("\u{2022}".to_string()),
                }
            });
            self.block(k, x, w, marker.flatten());
        }
        self.flush(&mut run, x, w);
    }

    fn flush(&mut self, run: &mut Vec<&Styled>, x: f32, w: f32) {
        if run.is_empty() {
            return;
        }
        let align = run[0].style.align;
        self.avail = w;
        let mut atoms = Vec::new();
        for n in run.drain(..) {
            self.collect(n, &mut atoms, self.cur_link);
        }
        self.flow(atoms, x, w, align);
    }

    fn block(&mut self, n: &Styled, x: f32, w: f32, marker: Option<String>) {
        let st = &n.style;
        let pad: [f32; 4] = std::array::from_fn(|i| st.padding[i].resolve(w).max(0.0));
        let bw: [f32; 4] = std::array::from_fn(|i| st.border[i].map_or(0.0, |(t, _)| t));
        let (mt, mb) = (st.margin[0].resolve(w), st.margin[2].resolve(w));
        let extra = pad[1] + pad[3] + bw[1] + bw[3];
        let (ml, mr) = (st.margin[3], st.margin[1]);
        let (ml_fixed, mr_fixed) = (ml.resolve(w), mr.resolve(w));
        let avail_box = (w - ml_fixed - mr_fixed).max(0.0);
        let mut box_w = match st.width {
            Len::Auto => avail_box,
            l => l.resolve(w).min(avail_box),
        };
        if st.max_width != Len::Auto {
            box_w = box_w.min(st.max_width.resolve(w));
        }
        let free = (w - box_w - ml_fixed - mr_fixed).max(0.0);
        let left = match (ml, mr) {
            (Len::Auto, Len::Auto) => free / 2.0,
            (Len::Auto, _) => free,
            _ => ml_fixed,
        };
        let x0 = x + left;
        let (cx, cw) = (x0 + bw[3] + pad[3], (box_w - extra).max(0.0));

        let top_open = pad[0] == 0.0 && bw[0] == 0.0 && st.background.is_none();
        if top_open {
            self.margin = collapse(self.margin, mt);
        } else {
            self.y += collapse(self.margin, mt);
            self.margin = 0.0;
        }
        let top = self.y;
        let bg_at = self.items.len();
        if let Some(color) = st.background {
            self.items.push(Item::Rect { x: x0, y: top, w: box_w, h: 0.0, color });
        }
        self.y += bw[0] + pad[0];
        if let Some(text) = marker {
            self.marker = Some(Marker { text, size: st.font_size, color: st.color, x: cx });
        }
        let saved_link = self.cur_link;
        if n.tag() == Some("a") {
            if let Some(h) = n.attr("href").filter(|h| usable_href(h)) {
                self.links.push(h.to_string());
                self.cur_link = Some(self.links.len() - 1);
            }
        }
        let list = match n.tag() {
            Some(t @ ("ul" | "ol")) => Some((t == "ol", n.attr("start").and_then(|s| s.trim().parse().ok()).unwrap_or(1))),
            _ => None,
        };
        self.children(n.children(), cx, cw, list);
        self.cur_link = saved_link;
        if let Some(mk) = self.marker.take() {
            // an empty list item still shows its bullet
            let y = top + bw[0] + pad[0];
            self.emit_marker(&mk, y, mk.size * st.line_height);
        }
        let carried = if pad[2] + bw[2] > 0.0 {
            self.y += self.margin + pad[2] + bw[2];
            self.margin = 0.0;
            mb
        } else {
            collapse(self.margin, mb)
        };
        let bottom = self.y;
        if st.background.is_some() {
            if let Item::Rect { h, .. } = &mut self.items[bg_at] {
                *h = bottom - top;
            }
        }
        let sides = [
            (0, x0, top, box_w, bw[0]),
            (1, x0 + box_w - bw[1], top, bw[1], bottom - top),
            (2, x0, bottom - bw[2], box_w, bw[2]),
            (3, x0, top, bw[3], bottom - top),
        ];
        for (i, rx, ry, rw, rh) in sides {
            if let Some((_, color)) = st.border[i].filter(|(t, _)| *t > 0.0) {
                self.items.push(Item::Rect { x: rx, y: ry, w: rw, h: rh, color });
            }
        }
        self.margin = carried;
    }

    fn emit_marker(&mut self, mk: &Marker, line_top: f32, line_h: f32) {
        let w = self.m.width(&mk.text, mk.size, false);
        let h = mk.size * GLYPH_H;
        self.items.push(Item::Text {
            x: mk.x - w - MARKER_GAP,
            y: line_top + (line_h - h) / 2.0,
            w,
            h,
            size: mk.size,
            text: mk.text.clone(),
            bold: false,
            italic: false,
            underline: false,
            color: mk.color,
            link: None,
            block: NO_BLOCK,
        });
    }

    // --- inline content ---

    fn face(st: &Style, link: Option<usize>) -> Face {
        Face {
            size: st.font_size,
            bold: st.bold,
            italic: st.italic,
            underline: st.underline,
            color: st.color,
            bg: st.background,
            link,
            lh: st.font_size * st.line_height,
            nowrap: matches!(st.white_space, WhiteSpace::NoWrap | WhiteSpace::Pre),
        }
    }

    fn collect(&mut self, n: &Styled, atoms: &mut Vec<Atom>, link: Option<usize>) {
        match &n.kind {
            Kind::Text(t) => self.text_atoms(t, &n.style, link, atoms),
            Kind::Element { tag, children, .. } => {
                match tag.as_str() {
                    "br" => {
                        atoms.push(Atom::Break { lh: n.style.font_size * n.style.line_height });
                        return;
                    }
                    "img" => {
                        if let Some(a) = self.image_atom(n, link) {
                            atoms.push(a);
                        }
                        return;
                    }
                    "td" | "th" if !atoms.is_empty() => {
                        let face = Self::face(&n.style, link);
                        let w = self.m.width(" ", face.size, face.bold);
                        atoms.push(Atom::Space { w, face });
                    }
                    _ => {}
                }
                let link = if tag == "a" {
                    match n.attr("href").filter(|h| usable_href(h)) {
                        Some(h) => {
                            self.links.push(h.to_string());
                            Some(self.links.len() - 1)
                        }
                        None => link,
                    }
                } else {
                    link
                };
                for c in children {
                    self.collect(c, atoms, link);
                }
            }
        }
    }

    fn word(&self, text: &str, face: &Face) -> Atom {
        Atom::Word { w: self.m.width(text, face.size, face.bold), text: text.to_string(), face: face.clone() }
    }

    fn text_atoms(&mut self, t: &str, st: &Style, link: Option<usize>, atoms: &mut Vec<Atom>) {
        let face = Self::face(st, link);
        let space = |s: &Self| Atom::Space { w: s.m.width(" ", face.size, face.bold), face: face.clone() };
        let collapse_into = |s: &Self, text: &str, atoms: &mut Vec<Atom>| {
            let (mut word, mut in_space) = (String::new(), false);
            for c in text.chars() {
                if is_ws(c) {
                    if !word.is_empty() {
                        atoms.push(s.word(&word, &face));
                        word.clear();
                    }
                    if !in_space {
                        atoms.push(space(s));
                        in_space = true;
                    }
                } else {
                    word.push(c);
                    in_space = false;
                }
            }
            if !word.is_empty() {
                atoms.push(s.word(&word, &face));
            }
        };
        match st.white_space {
            WhiteSpace::Normal | WhiteSpace::NoWrap => collapse_into(self, t, atoms),
            WhiteSpace::PreWrap => {
                for (i, line) in t.split('\n').enumerate() {
                    if i > 0 {
                        atoms.push(Atom::Break { lh: face.lh });
                    }
                    collapse_into(self, line, atoms);
                }
            }
            WhiteSpace::Pre => {
                for (i, line) in t.split('\n').enumerate() {
                    if i > 0 {
                        atoms.push(Atom::Break { lh: face.lh });
                    }
                    let line = line.trim_end_matches('\r').replace('\t', "    ");
                    if !line.is_empty() {
                        atoms.push(self.word(&line, &face));
                    }
                }
            }
        }
    }

    fn image_atom(&mut self, n: &Styled, link: Option<usize>) -> Option<Atom> {
        let usable = |s: &&str| !s.is_empty() && !s.starts_with("data:");
        let src = n.attr("src").filter(usable).or_else(|| n.attr("data-src").filter(usable))?;
        if !self.images.iter().any(|i| i == src) {
            self.images.push(src.to_string());
        }
        let nat = self.sizes.get(src).map(|&(w, h)| (w as f32, h as f32));
        let attr = |name: &str| n.attr(name).filter(|v| !v.trim().ends_with('%')).and_then(|v| v.trim().trim_end_matches("px").parse::<f32>().ok());
        let cw = match n.style.width {
            Len::Px(p) => Some(p),
            Len::Pct(p) => Some(self.avail * p / 100.0),
            Len::Auto => attr("width"),
        };
        let (mut w, mut h) = match (cw, attr("height"), nat) {
            (Some(w), Some(h), _) => (w, h),
            (Some(w), None, Some((nw, nh))) => (w, w * nh / nw),
            (Some(w), None, None) => (w, w * 0.6),
            (None, Some(h), Some((nw, nh))) => (h * nw / nh, h),
            (None, Some(h), None) => (h * 1.5, h),
            (None, None, Some((nw, nh))) => (nw, nh),
            (None, None, None) => (0.0, 0.0),
        };
        if w > self.avail && w > 0.0 {
            h *= self.avail / w;
            w = self.avail;
        }
        (w >= 1.0 && h >= 1.0).then(|| Atom::Image { w, h, src: src.to_string(), link })
    }

    /// Splits a word that is wider than a whole line into line-sized pieces.
    fn split_word(&self, text: &str, face: &Face, w: f32) -> Vec<String> {
        let (mut parts, mut cur) = (Vec::new(), String::new());
        for c in text.chars() {
            cur.push(c);
            if self.m.width(&cur, face.size, face.bold) > w && cur.chars().count() > 1 {
                cur.pop();
                parts.push(std::mem::take(&mut cur));
                cur.push(c);
            }
        }
        if !cur.is_empty() {
            parts.push(cur);
        }
        parts
    }

    fn flow(&mut self, atoms: Vec<Atom>, x: f32, w: f32, align: Align) {
        if atoms.iter().all(|a| matches!(a, Atom::Space { .. })) {
            return;
        }
        self.y += std::mem::take(&mut self.margin);
        let mut lines: Vec<Line> = vec![];
        let mut cur = Line::new();
        for atom in atoms {
            match atom {
                Atom::Break { lh } => {
                    cur.h = cur.h.max(lh);
                    cur.hard = true;
                    cur.trim();
                    lines.push(std::mem::replace(&mut cur, Line::new()));
                }
                Atom::Space { w: sw, face } => {
                    if cur.atoms.is_empty() || matches!(cur.atoms.last(), Some((_, Atom::Space { .. }))) {
                        continue;
                    }
                    let h = face.lh;
                    cur.push(Atom::Space { w: sw, face }, sw, h);
                }
                Atom::Word { text, w: aw, face } => {
                    let wrap = !face.nowrap;
                    if wrap && cur.width + aw > w + 0.01 && !cur.atoms.is_empty() {
                        cur.trim();
                        lines.push(std::mem::replace(&mut cur, Line::new()));
                    }
                    if wrap && aw > w + 0.01 && w > 0.0 {
                        let mut parts = self.split_word(&text, &face, w);
                        let last = parts.pop().unwrap_or_default();
                        for p in parts {
                            let mut l = Line::new();
                            let pw = self.m.width(&p, face.size, face.bold);
                            l.push(Atom::Word { text: p, w: pw, face: face.clone() }, pw, face.lh);
                            lines.push(l);
                        }
                        let lw = self.m.width(&last, face.size, face.bold);
                        cur.push(Atom::Word { text: last, w: lw, face: face.clone() }, lw, face.lh);
                    } else {
                        let h = face.lh;
                        cur.push(Atom::Word { text, w: aw, face }, aw, h);
                    }
                }
                Atom::Image { w: iw, h: ih, src, link } => {
                    if cur.width + iw > w + 0.01 && !cur.atoms.is_empty() {
                        cur.trim();
                        lines.push(std::mem::replace(&mut cur, Line::new()));
                    }
                    cur.push(Atom::Image { w: iw, h: ih, src, link }, iw, ih);
                }
            }
        }
        cur.trim();
        if !cur.atoms.is_empty() {
            lines.push(cur);
        }

        let block = self.blocks.len();
        self.blocks.push(String::new());
        for line in lines {
            let free = (w - line.width).max(0.0);
            let off = match align {
                Align::Left => 0.0,
                Align::Center => free / 2.0,
                Align::Right => free,
            };
            let y0 = self.y;
            if let Some(mk) = self.marker.take() {
                self.emit_marker(&mk, y0, line.h.max(mk.size));
            }
            for (ax, atom) in line.atoms {
                let ax = x + off + ax;
                match atom {
                    Atom::Word { text, w: aw, face } => {
                        let th = face.size * GLYPH_H;
                        let ty = y0 + (line.h - th) / 2.0;
                        if let Some(color) = face.bg {
                            self.items.push(Item::Rect { x: ax, y: ty, w: aw, h: th, color });
                        }
                        self.blocks[block].push_str(&text);
                        self.items.push(Item::Text { x: ax, y: ty, w: aw, h: th, size: face.size, text, bold: face.bold, italic: face.italic, underline: face.underline, color: face.color, link: face.link, block });
                    }
                    Atom::Space { .. } => self.blocks[block].push(' '),
                    Atom::Image { w: iw, h: ih, src, link } => {
                        self.items.push(Item::Image { x: ax, y: y0 + line.h - ih, w: iw, h: ih, src, link });
                    }
                    Atom::Break { .. } => {}
                }
            }
            self.blocks[block].push(if line.hard { '\n' } else { ' ' });
            self.y += line.h;
        }
        let trimmed = self.blocks[block].trim_end().len();
        self.blocks[block].truncate(trimmed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::browser::{css, html};

    /// Every glyph is half its font size wide.
    struct Fixed;
    impl Metrics for Fixed {
        fn width(&self, text: &str, size: f32, _bold: bool) -> f32 {
            text.chars().count() as f32 * size * 0.5
        }
    }

    fn page_at(src: &str, css_text: &str, width: f32, sizes: &HashMap<String, (usize, usize)>) -> Page {
        let mut rules = css::parse_stylesheet(css::USER_AGENT);
        rules.extend(css::parse_stylesheet(&format!("body {{ margin: 0 }} {css_text}")));
        layout(&css::style_tree(&html::parse(src), &rules), &Fixed, width, sizes)
    }

    fn page(src: &str, css_text: &str, width: f32) -> Page {
        page_at(src, css_text, width, &HashMap::new())
    }

    fn texts(p: &Page) -> Vec<(&str, f32, f32)> {
        p.items.iter().filter_map(|i| if let Item::Text { text, x, y, .. } = i { Some((text.as_str(), *x, *y)) } else { None }).collect()
    }

    #[test]
    fn blocks_stack_and_margins_collapse() {
        let p = page("<p>one</p><p>two</p>", "p { margin: 16px 0 }", 400.0);
        let t = texts(&p);
        let line = 16.0 * 1.3;
        assert!((t[0].2 - (16.0 + (line - 20.0) / 2.0)).abs() < 0.01, "first margin is not collapsed away: {t:?}");
        assert!((t[1].2 - t[0].2 - (line + 16.0)).abs() < 0.01, "16px between paragraphs, not 32: {t:?}");
        assert!((p.height - (16.0 + 2.0 * line + 16.0 + 16.0)).abs() < 0.01);
    }

    #[test]
    fn a_child_margin_collapses_through_a_plain_parent() {
        let p = page("<div><p>x</p></div>", "div { margin: 8px 0 } p { margin: 16px 0 }", 400.0);
        assert!((texts(&p)[0].2 - (16.0 + (20.8 - 20.0) / 2.0)).abs() < 0.01);
    }

    #[test]
    fn text_wraps_at_the_width() {
        let p = page("<p>aaaa bbbb cccc dddd</p>", "p { margin: 0 }", 100.0);
        let t = texts(&p);
        assert_eq!(t.iter().map(|x| x.0).collect::<Vec<_>>(), ["aaaa", "bbbb", "cccc", "dddd"]);
        assert_eq!(t[0].2, t[1].2);
        assert!(t[2].2 > t[0].2, "third word starts the second line");
        assert_eq!(t[2].1, 0.0);
        assert_eq!(t[1].1, 40.0);
        assert_eq!(p.blocks, ["aaaa bbbb cccc dddd"]);
    }

    #[test]
    fn a_word_wider_than_the_line_is_split() {
        let p = page("<p>abcdefghijkl</p>", "p { margin: 0 }", 40.0);
        let t = texts(&p);
        assert!(t.len() >= 3);
        assert!(t.iter().all(|(s, ..)| s.len() <= 5));
        assert_eq!(t.iter().map(|x| x.0).collect::<String>(), "abcdefghijkl");
    }

    #[test]
    fn br_ends_a_line_and_blank_ones_count() {
        let p = page("<p>a<br>b<br><br>c</p>", "p { margin: 0 }", 400.0);
        let ys: Vec<f32> = texts(&p).iter().map(|t| t.2).collect();
        assert!(ys[1] > ys[0]);
        assert!((ys[2] - ys[1] - 2.0 * 20.8).abs() < 0.01, "an empty line between b and c: {ys:?}");
        assert_eq!(p.blocks[0], "a\nb\n\nc");
    }

    #[test]
    fn links_are_registered_and_hit_testable() {
        let p = page("<p>go <a href='/next'>here</a> now <a href='#top'>skip</a></p>", "p { margin: 0 }", 400.0);
        assert_eq!(p.links, ["/next"]);
        let (_, x, y) = texts(&p).into_iter().find(|t| t.0 == "here").unwrap();
        assert_eq!(p.link_at(x + 2.0, y + 2.0), Some(0));
        assert_eq!(p.link_at(0.0, 0.0), None, "plain text is not a link");
        let (_, sx, sy) = texts(&p).into_iter().find(|t| t.0 == "skip").unwrap();
        assert_eq!(p.link_at(sx + 1.0, sy + 1.0), None, "fragment links are inert");
    }

    #[test]
    fn a_block_link_covers_its_contents() {
        let p = page("<a href='/card'><div>card text</div></a>", "", 400.0);
        assert_eq!(p.links, ["/card"]);
        let (_, x, y) = texts(&p)[0];
        assert_eq!(p.link_at(x + 1.0, y + 1.0), Some(0));
    }

    #[test]
    fn bold_italic_and_colour_reach_the_text_items() {
        let p = page("<p>a <b>b</b> <i style='color:red'>c</i></p>", "", 400.0);
        let flags: Vec<(bool, bool)> = p.items.iter().filter_map(|i| if let Item::Text { bold, italic, .. } = i { Some((*bold, *italic)) } else { None }).collect();
        assert_eq!(flags, [(false, false), (true, false), (false, true)]);
        assert!(matches!(&p.items[2], Item::Text { color, .. } if *color == Color::rgb(255, 0, 0)));
    }

    #[test]
    fn lists_get_bullets_and_numbers() {
        let p = page("<ul><li>a</li><li>b</li></ul><ol start=3><li>c</li><li>d</li></ol>", "", 400.0);
        let marks: Vec<&str> = p.items.iter().filter_map(|i| if let Item::Text { text, block, .. } = i { (*block == NO_BLOCK).then_some(text.as_str()) } else { None }).collect();
        assert_eq!(marks, ["\u{2022}", "\u{2022}", "3.", "4."]);
        let (_, ax, _) = texts(&p).into_iter().find(|t| t.0 == "a").unwrap();
        assert_eq!(ax, 32.0, "items indent by the list padding");
    }

    #[test]
    fn list_style_none_hides_markers() {
        let p = page("<ul style='list-style:none'><li>a</li></ul>", "", 400.0);
        assert!(p.items.iter().all(|i| !matches!(i, Item::Text { block, .. } if *block == NO_BLOCK)));
    }

    #[test]
    fn centred_text_and_auto_margins() {
        let p = page("<p style='text-align:center'>abcd</p>", "p { margin: 0 }", 100.0);
        assert_eq!(texts(&p)[0].1, 34.0);
        let p = page("<div style='width:100px;margin:0 auto'><p>ab</p></div>", "p { margin: 0 }", 400.0);
        assert_eq!(texts(&p)[0].1, 150.0);
        let p = page("<div style='max-width:200px;margin:0 auto'>ab</div>", "", 600.0);
        assert_eq!(texts(&p)[0].1, 200.0);
    }

    #[test]
    fn backgrounds_borders_and_padding_wrap_the_content() {
        let p = page("<div>hi</div>", "div { background: #eee; padding: 10px; border-left: 4px solid #f00; margin: 5px }", 200.0);
        let Item::Rect { x, y, w, h, .. } = p.items[0] else { panic!("background first") };
        assert_eq!((x, y, w), (5.0, 5.0, 190.0));
        assert!((h - (20.0 + 20.8)).abs() < 0.01, "padding top+bottom plus one line, got {h}");
        let (_, tx, ty) = texts(&p)[0];
        assert_eq!(tx, 5.0 + 4.0 + 10.0);
        assert!(ty > 15.0);
        assert!(matches!(p.items.last(), Some(Item::Rect { w, .. }) if *w == 4.0), "left border drawn");
        assert!((p.height - (5.0 + 40.8 + 5.0)).abs() < 0.01);
    }

    #[test]
    fn images_use_attributes_known_sizes_and_the_viewport() {
        let sizes: HashMap<String, (usize, usize)> = [("a.png".to_string(), (400usize, 200usize))].into();
        let p = page_at("<img src=a.png><img src=b.png width=50 height=10><img src=a.png width=100>", "", 300.0, &sizes);
        let dims: Vec<(f32, f32)> = p.items.iter().filter_map(|i| if let Item::Image { w, h, .. } = i { Some((*w, *h)) } else { None }).collect();
        assert_eq!(dims, [(300.0, 150.0), (50.0, 10.0), (100.0, 50.0)]);
        assert_eq!(p.images, ["a.png", "b.png"]);
    }

    #[test]
    fn unsized_unloaded_images_take_no_room_but_are_requested() {
        let p = page("<img src=x.png>", "", 300.0);
        assert!(p.items.is_empty());
        assert_eq!(p.images, ["x.png"]);
    }

    #[test]
    fn preformatted_text_keeps_lines_and_spaces() {
        let p = page("<pre>a  b\n  c</pre>", "pre { margin: 0; padding: 0 }", 400.0);
        let t = texts(&p);
        assert_eq!(t[0].0, "a  b");
        assert_eq!(t[1].0, "  c");
        assert!(t[1].2 > t[0].2);
    }

    #[test]
    fn whitespace_between_blocks_makes_no_lines() {
        let p = page("<div>a</div>\n  <div>b</div>\n", "", 400.0);
        assert_eq!(p.blocks, ["a", "b"]);
    }

    #[test]
    fn blocks_are_selectable_by_position() {
        let p = page("<p>first</p><p>second</p>", "", 400.0);
        let t = texts(&p);
        assert_eq!(p.block_at(t[0].1 + 1.0, t[0].2 + 1.0, 0.0), Some(0));
        assert_eq!(p.block_at(t[1].1 + 1.0, t[1].2 + 1.0, 0.0), Some(1));
        assert_eq!(p.block_at(399.0, 1000.0, 4.0), None);
        assert_eq!(p.text(), "first\n\nsecond");
    }

    #[test]
    fn the_body_background_becomes_the_page_background() {
        let p = page("<body><p>x</p></body>", "body { background: #123456 }", 400.0);
        assert_eq!(p.background, Some(Color::rgb(0x12, 0x34, 0x56)));
    }

    #[test]
    fn table_cells_are_separated() {
        let p = page("<table><tr><td>ab</td><td>cd</td></tr></table>", "", 400.0);
        let t = texts(&p);
        assert!(t[1].1 > t[0].1 + 16.0, "cells do not run together: {t:?}");
    }
}
