//! Text layout objects for apps: a typesetter that breaks a styled string into lines, a framesetter that lays the lines
//! out in a shape, and the frame, line and run objects they produce, with the geometry queries an editor needs
//! (offset of a string index, string index at a point, selection rectangles, caret rectangle) and drawing through a
//! [`TextPainter`].
//!
//! * Offsets and ranges are UTF-16 units, the unit of the platform's string range type.
//! * Coordinates are in points, y grows downward. A line's origin is its baseline start; glyph positions are relative
//!   to it (a raised glyph has a negative y).
//! * Widths come from a [`GlyphSource`]; nothing here measures text. One glyph is laid out per grapheme cluster. A
//!   source that shapes text can fold ligatures into the first character's advance.
//! * Line breaks follow `textbreak` (line break opportunities, grapheme clusters). Paragraph styles decide alignment,
//!   indents, tab stops, line height and spacing, the break mode and the base direction. Mixed direction text is
//!   reordered per line with `bidi`; tab stops are measured left to right from the line start. Brackets are not
//!   mirrored in right to left runs.
//! * Not modelled: vertical text, ruby, run delegates (see their own modules), hyphenation.

use crate::bidi;
use crate::paragraph::{Alignment, BaseDirection, LineBreakMode, ParagraphStyle, TabAlignment, DEFAULT_TAB_INTERVAL};
use crate::textbreak::{self, Brk};
use crate::textmetrics::{glyph_advances, FontMetrics, FontSpec, GlyphSource};
use crate::Rect;
use std::ops::Range;
use std::rc::Rc;

// ---- styles ----

/// The kind of line under a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnderlineStyle {
    #[default]
    None,
    Single,
    Double,
    Thick,
}

/// The attributes of a run of text. Colours are `0xAARRGGBB`; `None` means the painter's default colour.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunStyle {
    pub font: FontSpec,
    pub foreground: Option<u32>,
    pub background: Option<u32>,
    /// Raises (positive) or lowers (negative) the baseline of the run, in points.
    pub baseline_offset: f32,
    /// 1 for a superscript, -1 for a subscript, 0 for neither. Shrinks the font and shifts the baseline.
    pub superscript: i8,
    /// Extra space after every character, in points.
    pub kern: f32,
    /// Extra space after every character as a fraction of the font size.
    pub tracking: f32,
    /// The outline width as a percentage of the font size. Negative strokes and fills.
    pub stroke_width: f32,
    pub stroke_color: Option<u32>,
    pub underline: UnderlineStyle,
    pub underline_color: Option<u32>,
    pub strikethrough: bool,
    pub link: Option<String>,
    /// OpenType feature settings as (tag, value); empty keeps the shaper's defaults (kerning and ligatures on).
    pub features: Vec<([u8; 4], u32)>,
}

/// A superscript or subscript draws at this fraction of the font size.
const SCRIPT_SCALE: f32 = 0.7;
/// A superscript or subscript moves the baseline by this fraction of the font size.
const SCRIPT_SHIFT: f32 = 0.4;

impl RunStyle {
    /// The font the run is measured and drawn with: the font, shrunk for scripts.
    pub fn effective_font(&self) -> FontSpec {
        let mut f = self.font.clone();
        if self.superscript != 0 {
            f.size *= SCRIPT_SCALE;
        }
        f
    }

    /// The vertical shift of the run's baseline, y down (negative is up).
    pub fn baseline_shift(&self) -> f32 {
        -(self.baseline_offset + self.superscript as f32 * SCRIPT_SHIFT * self.font.size)
    }

    fn spacing(&self) -> f32 {
        self.kern + self.tracking * self.font.size
    }
}

// ---- painter ----

/// What drawing needs from the outside. The SDK implements it over a surface.
pub trait TextPainter {
    /// The colour of text with no foreground set.
    fn default_color(&self) -> u32;
    fn fill_rect(&mut self, rect: Rect, color: u32);
    /// Draws `text` left to right with its baseline start at (`x`, `baseline`). Fill colour, stroke and font come from
    /// `style`.
    fn draw_text(&mut self, text: &str, x: f32, baseline: f32, style: &RunStyle);
    /// A horizontal rule from `x0` to `x1`, `thickness` tall, centred on `y`.
    fn draw_rule(&mut self, x0: f32, x1: f32, y: f32, thickness: f32, color: u32);
}

// ---- internals ----

/// Options for a typesetter.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TypesetterOptions {
    /// Overrides the base direction of every paragraph.
    pub base_direction: Option<BaseDirection>,
}

#[derive(Clone, Debug)]
struct Cluster {
    /// Character indices.
    chars: Range<usize>,
    style: usize,
    level: u8,
    adv: f32,
    spacing: f32,
    tab: bool,
    hard: bool,
    ws: bool,
    space: bool,
    token: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Justify {
    None,
    Spaces { extra: f32, upto: usize },
    Chars { extra: f32, upto: usize },
}

#[derive(Clone, Copy, Debug)]
struct Ctx {
    para: usize,
    /// Distance of the line start from the leading margin, for tab stops.
    margin: f32,
}

struct Inner {
    chars: Vec<char>,
    /// UTF-16 offset of each character, one entry more than there are characters.
    u16_at: Vec<usize>,
    styles: Vec<RunStyle>,
    char_style: Vec<usize>,
    metrics: Vec<FontMetrics>,
    para_ranges: Vec<Range<usize>>,
    para_styles: Vec<ParagraphStyle>,
    para_rtl: Vec<bool>,
    levels: Vec<u8>,
    grapheme: Vec<bool>,
    breaks: Vec<Brk>,
    adv: Vec<f32>,
    source: Rc<dyn GlyphSource>,
}

fn is_hard(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{85}' | '\u{b}' | '\u{c}' | '\u{2028}' | '\u{2029}')
}

fn ends_paragraph(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2029}' | '\u{85}')
}

fn tab_advance(ps: &ParagraphStyle, x: f32, after: f32) -> f32 {
    let interval = if ps.default_tab_interval > 0.0 { ps.default_tab_interval } else { DEFAULT_TAB_INTERVAL };
    match ps.next_tab(x) {
        Some(t) => {
            let start = match t.alignment {
                TabAlignment::Left => t.location,
                TabAlignment::Right => t.location - after,
                TabAlignment::Center => t.location - after / 2.0,
            };
            (start - x).max(0.0)
        }
        None => ((x / interval).floor() + 1.0) * interval - x,
    }
}

impl Inner {
    fn len(&self) -> usize {
        self.chars.len()
    }

    /// The character index at UTF-16 offset `u`, rounded up to a character start and clamped.
    fn char_at(&self, u: usize) -> usize {
        self.u16_at.partition_point(|&x| x < u).min(self.chars.len())
    }

    fn para_index(&self, ch: usize) -> usize {
        let i = self.para_ranges.partition_point(|r| r.end <= ch);
        i.min(self.para_ranges.len().saturating_sub(1))
    }

    fn cluster_end(&self, i: usize, end: usize) -> usize {
        let mut j = i + 1;
        while j < end && !self.grapheme[j] {
            j += 1;
        }
        j.min(end)
    }

    fn cluster_at(&self, i: usize, end: usize) -> Cluster {
        let j = self.cluster_end(i, end);
        let style = self.char_style[i];
        let c = self.chars[i];
        Cluster {
            chars: i..j,
            style,
            level: self.levels[i],
            adv: self.adv[i..j].iter().sum(),
            spacing: self.styles[style].spacing(),
            tab: c == '\t',
            hard: is_hard(c),
            ws: c.is_whitespace(),
            space: c == ' ' || c == '\u{a0}',
            token: None,
        }
    }

    fn make_clusters(&self, range: Range<usize>) -> Vec<Cluster> {
        let mut out = Vec::new();
        let mut i = range.start;
        while i < range.end {
            let c = self.cluster_at(i, range.end);
            i = c.chars.end;
            out.push(c);
        }
        out
    }

    fn metrics_at(&self, ch: usize) -> FontMetrics {
        if self.chars.is_empty() {
            return FontMetrics::default();
        }
        self.metrics[self.char_style[ch.min(self.chars.len() - 1)]]
    }

    /// The end of the line that starts at character `pos`, in a space `avail` points wide, never past `limit`.
    fn break_line(&self, pos: usize, limit: usize, margin: f32, avail: f32, opportunities: bool) -> usize {
        let ps = &self.para_styles[self.para_index(pos)];
        let end = limit.min(self.len());
        let (mut x, mut ws_tail) = (0.0f32, 0.0f32);
        let mut i = pos;
        let mut last_opp: Option<usize> = None;
        while i < end {
            if i > pos && self.breaks[i] == Brk::Mandatory {
                return i;
            }
            let c = self.cluster_at(i, end);
            let a = if c.tab { tab_advance(ps, margin + x, 0.0) } else { c.adv + c.spacing };
            let new_x = x + a;
            let new_tail = if c.ws { ws_tail + a } else { 0.0 };
            if new_x - new_tail > avail + 0.001 && !c.ws && i > pos {
                if opportunities {
                    if let Some(o) = last_opp {
                        return o;
                    }
                }
                return i;
            }
            x = new_x;
            ws_tail = new_tail;
            i = c.chars.end;
            if i < end && self.breaks[i] == Brk::Allowed {
                last_opp = Some(i);
            }
        }
        end
    }

    fn assemble(&self, rc: &Rc<Inner>, clusters: Vec<Cluster>, chars: Range<usize>, ctx: Ctx, justify: Justify) -> TextLine {
        let n = clusters.len();
        let ps = &self.para_styles[ctx.para.min(self.para_styles.len() - 1)];
        // Advances in logical order.
        let mut adv = Vec::with_capacity(n);
        let mut x = 0.0f32;
        for k in 0..n {
            let c = &clusters[k];
            let mut a = c.adv + if k + 1 < n { c.spacing } else { 0.0 };
            if c.tab {
                let mut after = 0.0f32;
                for d in &clusters[k + 1..] {
                    if d.tab || d.hard {
                        break;
                    }
                    after += d.adv + d.spacing;
                }
                a = tab_advance(ps, ctx.margin + x, after);
            }
            match justify {
                Justify::Spaces { extra, upto } if k < upto && c.space => a += extra,
                Justify::Chars { extra, upto } if k < upto => a += extra,
                _ => {}
            }
            adv.push(a);
            x += a;
        }
        let width = x;
        // Trailing whitespace.
        let mut trailing = 0.0f32;
        for k in (0..n).rev() {
            if clusters[k].ws {
                trailing += adv[k];
            } else {
                break;
            }
        }
        // Visual order.
        let levels: Vec<u8> = clusters.iter().map(|c| c.level).collect();
        let order: Vec<usize> =
            if levels.iter().any(|l| l % 2 == 1) { bidi::visual_order(&levels) } else { (0..n).collect() };
        let mut xs = vec![0.0f32; n];
        let mut vx = 0.0f32;
        for &k in &order {
            xs[k] = vx;
            vx += adv[k];
        }
        // Runs.
        let mut runs: Vec<TextRun> = Vec::new();
        for &k in &order {
            let c = &clusters[k];
            let rtl = c.level % 2 == 1;
            let need_new = match runs.last() {
                Some(r) => r.style_ix != c.style || r.rtl != rtl,
                None => true,
            };
            if need_new {
                let style = self.styles[c.style].clone();
                let m = self.metrics[c.style];
                let shift = style.baseline_shift();
                runs.push(TextRun {
                    style_ix: c.style,
                    style,
                    rtl,
                    lo: usize::MAX,
                    hi: 0,
                    string_indices: Vec::new(),
                    positions: Vec::new(),
                    advances: Vec::new(),
                    natural: Vec::new(),
                    glyph_text: Vec::new(),
                    ascent: (m.ascent - shift).max(0.0),
                    descent: (m.descent + shift).max(0.0),
                    leading: m.leading,
                    metrics: m,
                });
            }
            let text: String = match &c.token {
                Some(t) => t.clone(),
                None => self.chars[c.chars.clone()].iter().collect(),
            };
            let (s16, e16) = (self.u16_at[c.chars.start], self.u16_at[c.chars.end]);
            let shift = self.styles[c.style].baseline_shift();
            if let Some(r) = runs.last_mut() {
                r.lo = r.lo.min(s16);
                r.hi = r.hi.max(e16);
                r.string_indices.push(s16);
                r.positions.push((xs[k], shift));
                r.advances.push(adv[k]);
                r.natural.push(if c.tab { 0.0 } else { c.adv + c.spacing });
                r.glyph_text.push(text);
            }
        }
        // Line metrics.
        let (mut ascent, mut descent, mut leading) = (0.0f32, 0.0f32, 0.0f32);
        if runs.is_empty() {
            let m = self.metrics_at(chars.start);
            ascent = m.ascent;
            descent = m.descent;
            leading = m.leading;
        }
        for r in &runs {
            ascent = ascent.max(r.ascent);
            descent = descent.max(r.descent);
            leading = leading.max(r.leading);
        }
        let boxes = (0..n)
            .map(|k| ClusterBox {
                start16: self.u16_at[clusters[k].chars.start],
                end16: self.u16_at[clusters[k].chars.end],
                x: xs[k],
                w: adv[k],
                rtl: clusters[k].level % 2 == 1,
                hard: clusters[k].hard,
            })
            .collect();
        TextLine {
            inner: Rc::clone(rc),
            chars,
            clusters,
            ctx,
            runs,
            boxes,
            width,
            trailing,
            ascent,
            descent,
            leading,
        }
    }
}

// ---- typesetter ----

/// Breaks a styled string into lines (a typesetter).
#[derive(Clone)]
pub struct Typesetter {
    inner: Rc<Inner>,
}

impl Typesetter {
    /// `spans` give the style of each UTF-16 range (later spans win, text outside every span is plain);
    /// `paragraphs` give the paragraph style of each UTF-16 range (the style covering the first character of a
    /// paragraph is its style).
    pub fn new(
        text: &str,
        spans: &[(Range<usize>, RunStyle)],
        paragraphs: &[(Range<usize>, ParagraphStyle)],
        options: TypesetterOptions,
        source: Rc<dyn GlyphSource>,
    ) -> Typesetter {
        let chars: Vec<char> = text.chars().collect();
        let n = chars.len();
        let mut u16_at = Vec::with_capacity(n + 1);
        let mut u = 0usize;
        for c in &chars {
            u16_at.push(u);
            u += c.len_utf16();
        }
        u16_at.push(u);
        let char_at = |off: usize| u16_at.partition_point(|&x| x < off).min(n);

        let mut styles = vec![RunStyle::default()];
        let mut char_style = vec![0usize; n];
        for (r, st) in spans {
            let ix = match styles.iter().position(|s| s == st) {
                Some(i) => i,
                None => {
                    styles.push(st.clone());
                    styles.len() - 1
                }
            };
            for s in char_style.iter_mut().take(char_at(r.end)).skip(char_at(r.start)) {
                *s = ix;
            }
        }
        let metrics: Vec<FontMetrics> = styles.iter().map(|s| source.metrics(&s.effective_font())).collect();

        // Paragraphs end after a paragraph separator; CR LF is one separator.
        let mut para_ranges: Vec<Range<usize>> = Vec::new();
        let mut start = 0usize;
        let mut i = 0usize;
        while i < n {
            if ends_paragraph(chars[i]) {
                let mut e = i + 1;
                if chars[i] == '\r' && e < n && chars[e] == '\n' {
                    e += 1;
                }
                para_ranges.push(start..e);
                start = e;
                i = e;
            } else {
                i += 1;
            }
        }
        if start < n || para_ranges.is_empty() {
            para_ranges.push(start..n);
        }
        let mut para_styles = Vec::with_capacity(para_ranges.len());
        for r in &para_ranges {
            let first = u16_at[r.start.min(n)];
            let st = paragraphs
                .iter()
                .rev()
                .find(|(pr, _)| pr.start <= first && first < pr.end.max(pr.start + 1))
                .map(|(_, s)| s.clone())
                .unwrap_or_default();
            para_styles.push(st);
        }

        let mut levels = Vec::with_capacity(n);
        let mut para_rtl = Vec::with_capacity(para_ranges.len());
        for (r, ps) in para_ranges.iter().zip(&para_styles) {
            let dir = match options.base_direction.unwrap_or(ps.base_direction) {
                BaseDirection::Natural => bidi::Direction::Natural,
                BaseDirection::LeftToRight => bidi::Direction::LeftToRight,
                BaseDirection::RightToLeft => bidi::Direction::RightToLeft,
            };
            let text: String = chars[r.clone()].iter().collect();
            let p = bidi::Paragraph::new(&text, dir);
            let mut lv = p.levels.clone();
            lv.resize(r.len(), p.level);
            para_rtl.push(p.is_rtl());
            levels.extend(lv);
        }

        let grapheme = textbreak::grapheme_boundaries(&chars);
        let breaks = textbreak::line_breaks(&chars);

        // Natural advances, per run of equal style.
        let mut adv = vec![0.0f32; n];
        let mut a = 0usize;
        while a < n {
            let mut b = a + 1;
            while b < n && char_style[b] == char_style[a] {
                b += 1;
            }
            let font = styles[char_style[a]].effective_font();
            let text: String = chars[a..b].iter().collect();
            let v = glyph_advances(source.as_ref(), &font, &text);
            for (k, w) in v.into_iter().enumerate() {
                let idx = a + k;
                adv[idx] = if grapheme[idx] && !is_hard(chars[idx]) && chars[idx] != '\t' { w } else { 0.0 };
            }
            a = b;
        }

        Typesetter {
            inner: Rc::new(Inner {
                chars,
                u16_at,
                styles,
                char_style,
                metrics,
                para_ranges,
                para_styles,
                para_rtl,
                levels,
                grapheme,
                breaks,
                adv,
                source,
            }),
        }
    }

    /// The length of the string in UTF-16 units.
    pub fn len(&self) -> usize {
        *self.inner.u16_at.last().unwrap_or(&0)
    }

    pub fn is_empty(&self) -> bool {
        self.inner.chars.is_empty()
    }

    fn snap_start(&self, c: usize) -> usize {
        let mut c = c.min(self.inner.len());
        while c < self.inner.len() && !self.inner.grapheme[c] {
            c += 1;
        }
        c
    }

    fn suggest(&self, start: usize, width: f32, opportunities: bool) -> usize {
        let inn = &self.inner;
        let pos = self.snap_start(inn.char_at(start));
        if pos >= inn.len() {
            return 0;
        }
        let limit = inn.para_ranges[inn.para_index(pos)].end;
        let end = inn.break_line(pos, limit, 0.0, width, opportunities);
        inn.u16_at[end] - inn.u16_at[pos]
    }

    /// The number of UTF-16 units from `start` that fit a line `width` points wide, ending at a line break
    /// opportunity. Trailing spaces hang past the width. A word wider than the line is split between characters. A
    /// hard break ends the line after itself. At least one character is taken while there is text left.
    pub fn suggest_line_break(&self, start: usize, width: f32) -> usize {
        self.suggest(start, width, true)
    }

    /// Like `suggest_line_break`, but the line may end between any two grapheme clusters.
    pub fn suggest_cluster_break(&self, start: usize, width: f32) -> usize {
        self.suggest(start, width, false)
    }

    /// A line holding `range` (UTF-16 units). The line has no indent and no alignment; tabs are measured from its
    /// start.
    pub fn create_line(&self, range: Range<usize>) -> TextLine {
        let inn = &self.inner;
        let s = self.snap_start(inn.char_at(range.start));
        let e = self.snap_start(inn.char_at(range.end)).max(s);
        let para = inn.para_index(s);
        let clusters = inn.make_clusters(s..e);
        inn.assemble(inn, clusters, s..e, Ctx { para, margin: 0.0 }, Justify::None)
    }

    fn line_in_para(&self, chars: Range<usize>, para: usize, margin: f32) -> TextLine {
        let clusters = self.inner.make_clusters(chars.clone());
        self.inner.assemble(&self.inner, clusters, chars, Ctx { para, margin }, Justify::None)
    }

    fn layout_frame(&self, range: Range<usize>, path: &FramePath, unbounded_width: bool) -> Vec<FrameLine> {
        let inn = &self.inner;
        let n = inn.len();
        let start = self.snap_start(inn.char_at(range.start));
        let end = if range.end <= range.start { n } else { self.snap_start(inn.char_at(range.end)).max(start) };
        let bounds = path.bounds();
        let mut out: Vec<FrameLine> = Vec::new();
        let mut y = bounds.y;
        let mut pos = start;
        let mut guard = 0usize;
        while pos < end {
            guard += 1;
            if guard > 1_000_000 {
                break;
            }
            let pi = inn.para_index(pos);
            let ps = &inn.para_styles[pi];
            let prange = inn.para_ranges[pi].clone();
            let first_line = pos == prange.start;
            if first_line && !out.is_empty() {
                y += ps.paragraph_spacing_before;
            }
            let m = inn.metrics_at(pos);
            let est = ps.line_height(m.ascent + m.descent + m.leading).max(1.0);
            if y + est > bounds.bottom() + 0.01 {
                break;
            }
            let Some(extent) = path.extent(y, y + est) else {
                y += est;
                continue;
            };
            let rtl = inn.para_rtl[pi];
            let indent = ps.indent(first_line);
            let w_ext = extent.1 - extent.0;
            let (lo, hi) = if rtl {
                (extent.0 + (w_ext - ps.trailing_edge(w_ext)), extent.1 - indent)
            } else {
                (extent.0 + indent, extent.0 + ps.trailing_edge(w_ext))
            };
            let avail = (hi - lo).max(1.0);
            let limit = prange.end.min(end);
            let mode = ps.line_break_mode;
            let line_end = match mode {
                LineBreakMode::WordWrap => inn.break_line(pos, limit, indent, avail, true),
                LineBreakMode::CharWrap => inn.break_line(pos, limit, indent, avail, false),
                _ => inn.break_line(pos, limit, indent, f32::INFINITY, true),
            };
            let line_end = line_end.max(self.snap_start(pos + 1)).min(limit.max(pos + 1));
            let mut line = self.line_in_para(pos..line_end, pi, indent);
            match mode {
                LineBreakMode::TruncateHead => line = line.truncated_line(avail, Truncation::Start, None),
                LineBreakMode::TruncateTail => line = line.truncated_line(avail, Truncation::End, None),
                LineBreakMode::TruncateMiddle => line = line.truncated_line(avail, Truncation::Middle, None),
                _ => {}
            }
            let wraps = matches!(mode, LineBreakMode::WordWrap | LineBreakMode::CharWrap);
            if ps.alignment == Alignment::Justified && wraps && line_end < prange.end && !line.ends_with_hard_break() {
                if let Some(j) = line.justified_line(1.0, avail) {
                    line = j;
                }
            }
            let factor = if unbounded_width {
                0.0
            } else {
                match ps.alignment {
                    Alignment::Left => 0.0,
                    Alignment::Right => 1.0,
                    Alignment::Center => 0.5,
                    Alignment::Natural | Alignment::Justified => {
                        if rtl {
                            1.0
                        } else {
                            0.0
                        }
                    }
                }
            };
            let pen = line.pen_offset_for_flush(factor, avail);
            let natural = line.ascent + line.descent + line.leading;
            let h = ps.line_height(natural);
            if y + h > bounds.bottom() + 0.01 {
                break;
            }
            let baseline = y + h - line.descent - line.leading;
            out.push(FrameLine { line, origin: (lo + pen, baseline), top: y, height: h });
            y += h + ps.line_spacing;
            if line_end >= prange.end {
                y += ps.paragraph_spacing;
            }
            pos = line_end;
        }
        out
    }
}

// ---- runs ----

/// Flags of a run (`status`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunStatus {
    pub right_to_left: bool,
    /// The string indices of the glyphs are not monotonic. Never set: one glyph per cluster.
    pub non_monotonic: bool,
    pub has_non_identity_matrix: bool,
}

/// Size of a run or a line.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TypographicBounds {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
}

/// A run of glyphs that share one style and one direction, in visual order.
#[derive(Clone, Debug)]
pub struct TextRun {
    style_ix: usize,
    style: RunStyle,
    rtl: bool,
    lo: usize,
    hi: usize,
    string_indices: Vec<usize>,
    positions: Vec<(f32, f32)>,
    advances: Vec<f32>,
    natural: Vec<f32>,
    glyph_text: Vec<String>,
    ascent: f32,
    descent: f32,
    leading: f32,
    metrics: FontMetrics,
}

impl TextRun {
    pub fn glyph_count(&self) -> usize {
        self.glyph_text.len()
    }

    /// The attributes of the run.
    pub fn attributes(&self) -> &RunStyle {
        &self.style
    }

    pub fn status(&self) -> RunStatus {
        RunStatus { right_to_left: self.rtl, non_monotonic: false, has_non_identity_matrix: false }
    }

    /// The UTF-16 range the run covers.
    pub fn string_range(&self) -> Range<usize> {
        if self.lo > self.hi {
            0..0
        } else {
            self.lo..self.hi
        }
    }

    /// The UTF-16 index each glyph maps to.
    pub fn string_indices(&self) -> &[usize] {
        &self.string_indices
    }

    /// The position of each glyph relative to the line origin.
    pub fn positions(&self) -> &[(f32, f32)] {
        &self.positions
    }

    pub fn advances(&self) -> &[f32] {
        &self.advances
    }

    /// The width and vertical extent of the glyphs in `glyphs` (an empty range or one past the end is clamped).
    pub fn typographic_bounds(&self, glyphs: Range<usize>) -> TypographicBounds {
        let e = glyphs.end.min(self.advances.len());
        let s = glyphs.start.min(e);
        TypographicBounds {
            width: self.advances[s..e].iter().sum(),
            ascent: self.ascent,
            descent: self.descent,
            leading: self.leading,
        }
    }

    /// Draws the glyphs in `glyphs` of a run on the line whose origin is (`ox`, `oy`).
    pub fn draw(&self, p: &mut dyn TextPainter, ox: f32, oy: f32, glyphs: Range<usize>) {
        let e = glyphs.end.min(self.glyph_count());
        let s = glyphs.start.min(e);
        if s >= e {
            return;
        }
        let style = &self.style;
        let fg = style.foreground.unwrap_or_else(|| p.default_color());
        let x0 = ox + self.positions[s].0;
        let x1 = ox + self.positions[e - 1].0 + self.advances[e - 1];
        if let Some(bg) = style.background {
            p.fill_rect(Rect { x: x0, y: oy - self.ascent, w: x1 - x0, h: self.ascent + self.descent }, bg);
        }
        let mut k = s;
        while k < e {
            let mut j = k + 1;
            while j < e {
                let expect = self.positions[j - 1].0 + self.natural[j - 1];
                if (self.positions[j].0 - expect).abs() < 0.01 && self.positions[j].1 == self.positions[k].1 {
                    j += 1;
                } else {
                    break;
                }
            }
            let text: String = self.glyph_text[k..j].iter().flat_map(|t| t.chars()).filter(|c| !c.is_control()).collect();
            if !text.is_empty() {
                p.draw_text(&text, ox + self.positions[k].0, oy + self.positions[k].1, style);
            }
            k = j;
        }
        let shift = style.baseline_shift();
        let thickness = self.metrics.underline_thickness.max(0.5);
        if style.underline != UnderlineStyle::None {
            let color = style.underline_color.unwrap_or(fg);
            let y = oy + shift + self.metrics.underline_position;
            match style.underline {
                UnderlineStyle::Thick => p.draw_rule(x0, x1, y, thickness * 2.0, color),
                UnderlineStyle::Double => {
                    p.draw_rule(x0, x1, y, thickness, color);
                    p.draw_rule(x0, x1, y + thickness * 2.0, thickness, color);
                }
                _ => p.draw_rule(x0, x1, y, thickness, color),
            }
        }
        if style.strikethrough {
            p.draw_rule(x0, x1, oy + shift - self.ascent * 0.3, thickness, fg);
        }
    }
}

// ---- lines ----

/// How a truncated line loses text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Truncation {
    Start,
    End,
    Middle,
}

#[derive(Clone, Copy, Debug)]
struct ClusterBox {
    start16: usize,
    end16: usize,
    x: f32,
    w: f32,
    rtl: bool,
    hard: bool,
}

/// One line of text.
#[derive(Clone)]
pub struct TextLine {
    inner: Rc<Inner>,
    chars: Range<usize>,
    clusters: Vec<Cluster>,
    ctx: Ctx,
    runs: Vec<TextRun>,
    boxes: Vec<ClusterBox>,
    width: f32,
    trailing: f32,
    ascent: f32,
    descent: f32,
    leading: f32,
}

impl TextLine {
    /// The UTF-16 range of the line.
    pub fn string_range(&self) -> Range<usize> {
        self.inner.u16_at[self.chars.start]..self.inner.u16_at[self.chars.end]
    }

    pub fn glyph_count(&self) -> usize {
        self.runs.iter().map(|r| r.glyph_count()).sum()
    }

    /// The runs, in visual order.
    pub fn runs(&self) -> &[TextRun] {
        &self.runs
    }

    pub fn typographic_bounds(&self) -> TypographicBounds {
        TypographicBounds { width: self.width, ascent: self.ascent, descent: self.descent, leading: self.leading }
    }

    pub fn trailing_whitespace_width(&self) -> f32 {
        self.trailing
    }

    /// The box around the drawn text relative to the line origin: from the start over the width without trailing
    /// whitespace, from the ascent above the baseline to the descent below it. There are no outlines here, so this
    /// is the typographic box.
    pub fn image_bounds(&self) -> Rect {
        Rect { x: 0.0, y: -self.ascent, w: (self.width - self.trailing).max(0.0), h: self.ascent + self.descent }
    }

    /// How far to move the line to flush it in `flush_width`: 0 for flush left, 1 for flush right, 0.5 for centred.
    /// Trailing whitespace does not count. A line wider than `flush_width` is not moved.
    pub fn pen_offset_for_flush(&self, flush_factor: f32, flush_width: f32) -> f32 {
        (flush_factor * (flush_width - (self.width - self.trailing))).max(0.0)
    }

    fn leading_edge(b: &ClusterBox) -> f32 {
        if b.rtl {
            b.x + b.w
        } else {
            b.x
        }
    }

    fn trailing_edge(b: &ClusterBox) -> f32 {
        if b.rtl {
            b.x
        } else {
            b.x + b.w
        }
    }

    /// The x of the caret at UTF-16 `index`: (primary, secondary). They differ where text of two directions meet:
    /// the primary is the leading edge of the character at the index, the secondary the trailing edge of the one
    /// before it. An index inside a cluster maps to the start of the cluster.
    pub fn offset_for_string_index(&self, index: usize) -> (f32, f32) {
        let Some(last) = self.boxes.last() else {
            return (0.0, 0.0);
        };
        if index >= last.end16 {
            let t = Self::trailing_edge(last);
            return (t, t);
        }
        let k = self.boxes.iter().position(|b| index < b.end16).unwrap_or(0);
        let lead = Self::leading_edge(&self.boxes[k]);
        let prior = if k > 0 && index <= self.boxes[k].start16 { Self::trailing_edge(&self.boxes[k - 1]) } else { lead };
        (lead, prior)
    }

    /// The UTF-16 index of the character boundary nearest to x (relative to the line origin). `None` for an empty
    /// line. Never past a trailing line break.
    pub fn string_index_for_position(&self, px: f32) -> Option<usize> {
        if self.boxes.is_empty() {
            return None;
        }
        let cap = match self.boxes.last() {
            Some(b) if b.hard => b.start16,
            Some(b) => b.end16,
            None => 0,
        };
        let left = self.boxes.iter().min_by(|a, b| a.x.total_cmp(&b.x))?;
        let right = self.boxes.iter().max_by(|a, b| (a.x + a.w).total_cmp(&(b.x + b.w)))?;
        let idx = if px < left.x {
            if left.rtl { left.end16 } else { left.start16 }
        } else if px >= right.x + right.w {
            if right.rtl { right.start16 } else { right.end16 }
        } else {
            let b = self.boxes.iter().find(|b| px >= b.x && px < b.x + b.w).unwrap_or(left);
            let left_half = px < b.x + b.w / 2.0;
            if left_half != b.rtl { b.start16 } else { b.end16 }
        };
        Some(idx.min(cap))
    }

    /// The x extents (relative to the line origin) of the text in the UTF-16 `range`, sorted and merged.
    pub fn x_spans_for_range(&self, range: Range<usize>) -> Vec<(f32, f32)> {
        let mut v: Vec<(f32, f32)> = self
            .boxes
            .iter()
            .filter(|b| b.w > 0.0 && b.start16 < range.end && b.end16 > range.start)
            .map(|b| (b.x, b.x + b.w))
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out: Vec<(f32, f32)> = Vec::new();
        for s in v {
            match out.last_mut() {
                Some(l) if s.0 <= l.1 + 0.01 => l.1 = l.1.max(s.1),
                _ => out.push(s),
            }
        }
        out
    }

    fn ends_with_hard_break(&self) -> bool {
        self.clusters.last().is_some_and(|c| c.hard)
    }

    fn rebuild(&self, clusters: Vec<Cluster>, justify: Justify) -> TextLine {
        self.inner.assemble(&self.inner, clusters, self.chars.clone(), self.ctx, justify)
    }

    /// The line cut to `width`, the removed text replaced by `token` (default an ellipsis). The result keeps the
    /// string range of the line; the token maps to the removed text. A line that fits is returned as it is.
    pub fn truncated_line(&self, width: f32, kind: Truncation, token: Option<&str>) -> TextLine {
        if self.width - self.trailing <= width + 0.001 {
            return self.clone();
        }
        let m = self.clusters.len() - usize::from(self.ends_with_hard_break());
        if m == 0 {
            return self.clone();
        }
        let token = token.unwrap_or("\u{2026}").to_string();
        let build = |k: usize| -> TextLine {
            let (p, s) = match kind {
                Truncation::End => (k, 0),
                Truncation::Start => (0, k),
                Truncation::Middle => (k.div_ceil(2), k / 2),
            };
            let from = &self.clusters[p];
            let style = from.style;
            let font = self.inner.styles[style].effective_font();
            let adv: f32 = glyph_advances(self.inner.source.as_ref(), &font, &token).iter().sum();
            let tok = Cluster {
                chars: from.chars.start..self.clusters[m - s - 1].chars.end,
                style,
                level: from.level,
                adv,
                spacing: 0.0,
                tab: false,
                hard: false,
                ws: false,
                space: false,
                token: Some(token.clone()),
            };
            let mut cl: Vec<Cluster> = self.clusters[..p].to_vec();
            cl.push(tok);
            cl.extend_from_slice(&self.clusters[m - s..]);
            self.rebuild(cl, Justify::None)
        };
        // The largest number of kept clusters, below m, whose line fits.
        let (mut lo, mut hi) = (0usize, m - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            let l = build(mid);
            if l.width - l.trailing <= width + 0.001 {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        build(lo)
    }

    /// The line stretched to `width`. The extra space goes to the spaces (to every gap between characters when the
    /// line has none), `factor` (0 to 1) scales how much of it is used. `None` when the line is wider than `width`.
    pub fn justified_line(&self, factor: f32, width: f32) -> Option<TextLine> {
        let content = self.width - self.trailing;
        let free = width - content;
        if free < 0.0 {
            return None;
        }
        let extra_total = free * factor.clamp(0.0, 1.0);
        let last = self.clusters.iter().rposition(|c| !c.ws)?;
        let spaces = self.clusters[..last].iter().filter(|c| c.space).count();
        let justify = if spaces > 0 {
            Justify::Spaces { extra: extra_total / spaces as f32, upto: last }
        } else if last > 0 {
            Justify::Chars { extra: extra_total / last as f32, upto: last }
        } else {
            return Some(self.clone());
        };
        Some(self.rebuild(self.clusters.clone(), justify))
    }

    /// Draws every run with the line origin at (`x`, `y`).
    pub fn draw(&self, p: &mut dyn TextPainter, x: f32, y: f32) {
        for r in &self.runs {
            r.draw(p, x, y, 0..r.glyph_count());
        }
    }
}

// ---- frames ----

/// The shape a frame is laid out in.
#[derive(Clone, Debug, PartialEq)]
pub enum FramePath {
    Rect(Rect),
    /// The ellipse inscribed in the rectangle.
    Ellipse(Rect),
    /// A closed polygon.
    Polygon(Vec<(f32, f32)>),
}

impl FramePath {
    pub fn bounds(&self) -> Rect {
        match self {
            FramePath::Rect(r) | FramePath::Ellipse(r) => *r,
            FramePath::Polygon(p) => {
                if p.is_empty() {
                    return Rect::default();
                }
                let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
                for &(x, y) in p {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x);
                    y1 = y1.max(y);
                }
                Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
            }
        }
    }

    /// The horizontal extent `(left, right)` free for a line occupying the band `y0..y1`. `None` when the band is
    /// outside the shape or has no room.
    pub fn extent(&self, y0: f32, y1: f32) -> Option<(f32, f32)> {
        let b = self.bounds();
        if y0 < b.y - 0.01 || y1 > b.bottom() + 0.01 {
            return None;
        }
        match self {
            FramePath::Rect(r) => Some((r.x, r.right())),
            FramePath::Ellipse(r) => {
                let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                let (rx, ry) = (r.w / 2.0, r.h / 2.0);
                let dy = (y0 - cy).abs().max((y1 - cy).abs());
                if ry <= 0.0 || dy >= ry {
                    return None;
                }
                let half = rx * (1.0 - (dy / ry) * (dy / ry)).sqrt();
                Some((cx - half, cx + half))
            }
            FramePath::Polygon(p) => {
                let mut lo = f32::MIN;
                let mut hi = f32::MAX;
                for y in [y0 + 0.001, (y0 + y1) / 2.0, y1 - 0.001] {
                    let (a, b) = Self::widest_span(p, y)?;
                    lo = lo.max(a);
                    hi = hi.min(b);
                }
                (hi > lo).then_some((lo, hi))
            }
        }
    }

    fn widest_span(p: &[(f32, f32)], y: f32) -> Option<(f32, f32)> {
        let mut xs: Vec<f32> = Vec::new();
        for i in 0..p.len() {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            if (a.1 <= y && y < b.1) || (b.1 <= y && y < a.1) {
                xs.push(a.0 + (y - a.1) * (b.0 - a.0) / (b.1 - a.1));
            }
        }
        xs.sort_by(|a, b| a.total_cmp(b));
        let mut best: Option<(f32, f32)> = None;
        for pair in xs.chunks(2) {
            if let [a, b] = pair {
                if best.map_or(true, |(l, r)| b - a > r - l) {
                    best = Some((*a, *b));
                }
            }
        }
        best
    }
}

/// A line placed in a frame.
#[derive(Clone)]
pub struct FrameLine {
    pub line: TextLine,
    /// The start of the baseline.
    pub origin: (f32, f32),
    /// The top of the line box and its height.
    pub top: f32,
    pub height: f32,
}

/// Laid-out lines in a shape.
pub struct Frame {
    path: FramePath,
    lines: Vec<FrameLine>,
    visible: Range<usize>,
    total: usize,
    caret_after_break: Option<(f32, f32, f32)>,
}

impl Frame {
    pub fn path(&self) -> &FramePath {
        &self.path
    }

    pub fn lines(&self) -> &[FrameLine] {
        &self.lines
    }

    /// The start of the baseline of each line.
    pub fn line_origins(&self) -> Vec<(f32, f32)> {
        self.lines.iter().map(|l| l.origin).collect()
    }

    /// The UTF-16 range of the text that fits in the frame.
    pub fn visible_string_range(&self) -> Range<usize> {
        self.visible.clone()
    }

    /// True when the frame holds all the text it was asked to lay out.
    pub fn fits_all(&self, requested_end: usize) -> bool {
        self.visible.end >= requested_end.min(self.total)
    }

    /// Draws every line with the frame's coordinates offset by (`ox`, `oy`).
    pub fn draw(&self, p: &mut dyn TextPainter, ox: f32, oy: f32) {
        for l in &self.lines {
            l.line.draw(p, ox + l.origin.0, oy + l.origin.1);
        }
    }

    fn line_for_index(&self, index: usize) -> Option<usize> {
        for (i, l) in self.lines.iter().enumerate() {
            let r = l.line.string_range();
            if index < r.end || (i + 1 == self.lines.len() && index <= r.end) {
                return Some(i);
            }
        }
        None
    }

    /// The UTF-16 index of the character boundary nearest to (x, y). `None` for a frame with no lines.
    pub fn string_index_for_point(&self, x: f32, y: f32) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        let mut pick = self.lines.len() - 1;
        for (i, l) in self.lines.iter().enumerate() {
            if y < l.top + l.height {
                pick = i;
                break;
            }
        }
        let l = &self.lines[pick];
        l.line.string_index_for_position(x - l.origin.0).or_else(|| Some(l.line.string_range().start))
    }

    /// The caret at UTF-16 `index`: x, top and height, with a zero-width rectangle. After a trailing line break the
    /// caret sits on the empty line below.
    pub fn caret_rect(&self, index: usize) -> Option<Rect> {
        if index >= self.total {
            if let Some((x, y, h)) = self.caret_after_break {
                return Some(Rect { x, y, w: 0.0, h });
            }
        }
        let i = self.line_for_index(index)?;
        let l = &self.lines[i];
        let (primary, _) = l.line.offset_for_string_index(index);
        Some(Rect { x: l.origin.0 + primary, y: l.top, w: 0.0, h: l.height })
    }

    /// One rectangle per run of selected text on each line.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let mut out = Vec::new();
        for l in &self.lines {
            let r = l.line.string_range();
            if range.start >= r.end || range.end <= r.start {
                continue;
            }
            for (a, b) in l.line.x_spans_for_range(range.clone()) {
                out.push(Rect { x: l.origin.0 + a, y: l.top, w: b - a, h: l.height });
            }
        }
        out
    }

    /// The box around the text: the widest line over the stack of line boxes.
    pub fn used_rect(&self) -> Rect {
        let mut r: Option<Rect> = None;
        for l in &self.lines {
            let b = Rect {
                x: l.origin.0,
                y: l.top,
                w: (l.line.width - l.line.trailing).max(0.0),
                h: l.height,
            };
            r = Some(match r {
                Some(a) => a.union(b),
                None => b,
            });
        }
        r.unwrap_or_default()
    }
}

// ---- framesetter ----

/// Lays a typesetter's text out in shapes (a framesetter).
#[derive(Clone)]
pub struct Framesetter {
    ts: Typesetter,
}

impl Framesetter {
    pub fn new(ts: Typesetter) -> Self {
        Framesetter { ts }
    }

    pub fn typesetter(&self) -> &Typesetter {
        &self.ts
    }

    fn make_frame(&self, range: Range<usize>, path: FramePath, unbounded_width: bool) -> Frame {
        let lines = self.ts.layout_frame(range.clone(), &path, unbounded_width);
        let total = self.ts.len();
        let first = range.start.min(total);
        let visible = match (lines.first(), lines.last()) {
            (Some(a), Some(b)) => a.line.string_range().start..b.line.string_range().end,
            _ => first..first,
        };
        let caret_after_break = match lines.last() {
            Some(l) if l.line.ends_with_hard_break() && l.line.string_range().end >= total => {
                let ps = &self.ts.inner.para_styles[l.line.ctx.para];
                let m = self.ts.inner.metrics_at(self.ts.inner.len().saturating_sub(1));
                let h = ps.line_height(m.ascent + m.descent + m.leading);
                let b = path.bounds();
                Some((b.x, l.top + l.height + ps.line_spacing, h))
            }
            _ => None,
        };
        Frame { path, lines, visible, total, caret_after_break }
    }

    /// A frame holding the UTF-16 `range` in `path`. An empty range means everything from its start.
    pub fn create_frame(&self, range: Range<usize>, path: FramePath) -> Frame {
        self.make_frame(range, path, false)
    }

    /// The size needed for the UTF-16 `range` within the constraints (infinite means unconstrained), and the range
    /// that fits. Lines are flush left when the width is unconstrained.
    pub fn suggest_frame_size(&self, range: Range<usize>, max_width: f32, max_height: f32) -> ((f32, f32), Range<usize>) {
        let unbounded = !max_width.is_finite();
        let w = if unbounded { 1.0e7 } else { max_width.max(0.0) };
        let h = if max_height.is_finite() { max_height.max(0.0) } else { f32::INFINITY };
        let frame = self.make_frame(range, FramePath::Rect(Rect { x: 0.0, y: 0.0, w, h }), unbounded);
        let mut width = 0.0f32;
        let mut bottom = 0.0f32;
        for l in frame.lines() {
            width = width.max(l.origin.0 + (l.line.width - l.line.trailing).max(0.0));
            bottom = bottom.max(l.top + l.height);
        }
        ((width, bottom), frame.visible_string_range())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paragraph::TextTab;
    use crate::textmetrics::FnSource;

    /// Ten points per character at size 10; ascent 8, descent 2.
    fn source() -> Rc<dyn GlyphSource> {
        Rc::new(FnSource {
            measure: Box::new(|f, t| t.chars().count() as f32 * f.size),
            metrics: Box::new(|f| FontMetrics {
                ascent: f.size * 0.8,
                descent: f.size * 0.2,
                leading: 0.0,
                underline_position: 1.0,
                underline_thickness: 1.0,
            }),
        })
    }

    fn plain() -> RunStyle {
        RunStyle { font: FontSpec::new(10.0), ..Default::default() }
    }

    fn ts(text: &str) -> Typesetter {
        let n = text.encode_utf16().count();
        Typesetter::new(text, &[(0..n, plain())], &[], TypesetterOptions::default(), source())
    }

    fn ts_para(text: &str, ps: ParagraphStyle) -> Typesetter {
        let n = text.encode_utf16().count();
        Typesetter::new(text, &[(0..n, plain())], &[(0..n.max(1), ps)], TypesetterOptions::default(), source())
    }

    fn rect(w: f32, h: f32) -> FramePath {
        FramePath::Rect(Rect { x: 0.0, y: 0.0, w, h })
    }

    fn line_text(l: &TextLine) -> String {
        l.runs().iter().flat_map(|r| r.glyph_text.iter().cloned()).collect()
    }

    #[test]
    fn suggests_breaks_at_opportunities_and_clusters() {
        let t = ts("aa bb cc dd");
        assert_eq!(t.suggest_line_break(0, 50.0), 6);
        assert_eq!(t.suggest_line_break(6, 50.0), 5);
        assert_eq!(ts("abcdefg").suggest_line_break(0, 30.0), 3);
        assert_eq!(t.suggest_cluster_break(0, 50.0), 6);
        assert_eq!(ts("ab\ncd").suggest_line_break(0, 100.0), 3);
        assert_eq!(ts("ab").suggest_line_break(2, 100.0), 0);
        // The first character is always taken.
        assert_eq!(ts("abc").suggest_line_break(0, 1.0), 1);
    }

    #[test]
    fn utf16_offsets_count_surrogate_pairs() {
        let t = ts("a\u{1f600}b");
        assert_eq!(t.len(), 4);
        let line = t.create_line(0..4);
        assert_eq!(line.glyph_count(), 3);
        assert_eq!(line.runs()[0].string_indices(), &[0, 1, 3]);
        assert_eq!(line.string_range(), 0..4);
    }

    #[test]
    fn line_geometry_and_hit_testing() {
        let line = ts("hello").create_line(0..5);
        let b = line.typographic_bounds();
        assert_eq!((b.width, b.ascent, b.descent), (50.0, 8.0, 2.0));
        assert_eq!(line.offset_for_string_index(2), (20.0, 20.0));
        assert_eq!(line.offset_for_string_index(5), (50.0, 50.0));
        assert_eq!(line.string_index_for_position(24.0), Some(2));
        assert_eq!(line.string_index_for_position(26.0), Some(3));
        assert_eq!(line.string_index_for_position(-5.0), Some(0));
        assert_eq!(line.string_index_for_position(500.0), Some(5));
        assert_eq!(line.image_bounds(), Rect { x: 0.0, y: -8.0, w: 50.0, h: 10.0 });
        assert_eq!(line.runs()[0].positions()[1], (10.0, 0.0));
        assert_eq!(line.runs()[0].typographic_bounds(1..3).width, 20.0);
    }

    #[test]
    fn trailing_whitespace_and_flush_offsets() {
        let line = ts("ab  ").create_line(0..4);
        assert_eq!(line.typographic_bounds().width, 40.0);
        assert_eq!(line.trailing_whitespace_width(), 20.0);
        assert_eq!(line.pen_offset_for_flush(1.0, 100.0), 80.0);
        assert_eq!(line.pen_offset_for_flush(0.5, 100.0), 40.0);
        assert_eq!(line.pen_offset_for_flush(1.0, 10.0), 0.0);
    }

    #[test]
    fn truncation_replaces_removed_text_with_a_token() {
        let line = ts("hello world").create_line(0..11);
        let tail = line.truncated_line(60.0, Truncation::End, None);
        assert_eq!(line_text(&tail), "hello\u{2026}");
        assert_eq!(tail.typographic_bounds().width, 60.0);
        assert_eq!(tail.string_range(), 0..11);
        assert_eq!(line_text(&line.truncated_line(60.0, Truncation::Start, None)), "\u{2026}world");
        assert_eq!(line_text(&line.truncated_line(60.0, Truncation::Middle, None)), "hel\u{2026}ld");
        assert_eq!(line_text(&line.truncated_line(5.0, Truncation::End, None)), "\u{2026}");
        assert_eq!(line_text(&line.truncated_line(500.0, Truncation::End, None)), "hello world");
        assert_eq!(line_text(&line.truncated_line(50.0, Truncation::End, Some("--"))), "hel--");
    }

    #[test]
    fn justified_line_stretches_the_spaces() {
        let line = ts("aa bb").create_line(0..5);
        let j = line.justified_line(1.0, 70.0).unwrap();
        assert_eq!(j.typographic_bounds().width, 70.0);
        assert_eq!(j.runs()[0].positions()[3].0, 50.0);
        assert!(line.justified_line(1.0, 40.0).is_none());
        // No spaces: the gaps between characters grow.
        let k = ts("abc").create_line(0..3).justified_line(1.0, 36.0).unwrap();
        assert_eq!(k.runs()[0].positions()[2].0, 26.0);
        // Factor zero changes nothing.
        assert_eq!(line.justified_line(0.0, 90.0).unwrap().typographic_bounds().width, 50.0);
    }

    #[test]
    fn tabs_advance_to_default_and_explicit_stops() {
        let line = ts("a\tb").create_line(0..3);
        assert_eq!(line.runs()[0].positions()[2].0, 28.0);
        let ps = ParagraphStyle {
            tab_stops: vec![TextTab { alignment: TabAlignment::Right, location: 100.0 }],
            ..Default::default()
        };
        let l2 = ts_para("a\tbb", ps).create_line(0..4);
        // A right tab ends the following text on the stop: "bb" is 20 wide, so it starts at 80.
        assert_eq!(l2.runs()[0].positions()[2].0, 80.0);
    }

    #[test]
    fn kerning_and_baseline_attributes_change_the_layout() {
        let style = RunStyle { kern: 2.0, ..plain() };
        let t = Typesetter::new("abc", &[(0..3, style)], &[], TypesetterOptions::default(), source());
        let l = t.create_line(0..3);
        // No spacing after the last character of the line.
        assert_eq!(l.typographic_bounds().width, 34.0);
        let sup = RunStyle { superscript: 1, ..plain() };
        let t2 = Typesetter::new("ab", &[(0..2, sup)], &[], TypesetterOptions::default(), source());
        let l2 = t2.create_line(0..2);
        assert_eq!(l2.typographic_bounds().width, 14.0);
        assert!(l2.runs()[0].positions()[0].1 < 0.0);
        assert!(l2.typographic_bounds().ascent > 5.6);
    }

    #[test]
    fn runs_split_at_style_changes() {
        let bold = RunStyle { font: FontSpec::new(10.0).with_weight(700), ..Default::default() };
        let t = Typesetter::new("abcd", &[(0..2, plain()), (2..4, bold.clone())], &[], TypesetterOptions::default(), source());
        let l = t.create_line(0..4);
        assert_eq!(l.runs().len(), 2);
        assert_eq!(l.runs()[1].attributes(), &bold);
        assert_eq!(l.runs()[1].string_range(), 2..4);
        assert!(!l.runs()[0].status().right_to_left);
    }

    #[test]
    fn right_to_left_text_is_reordered() {
        let l = ts("\u{5d0}\u{5d1}").create_line(0..2);
        assert_eq!(l.runs().len(), 1);
        assert!(l.runs()[0].status().right_to_left);
        assert_eq!(l.runs()[0].string_indices(), &[1, 0]);
        // The index 0 is at the right edge.
        assert_eq!(l.offset_for_string_index(0).0, 20.0);
        assert_eq!(l.string_index_for_position(3.0), Some(2));
    }

    #[test]
    fn frame_wraps_lines_and_reports_the_visible_range() {
        let fs = Framesetter::new(ts("aa bb cc dd"));
        let f = fs.create_frame(0..0, rect(50.0, 100.0));
        assert_eq!(f.lines().len(), 2);
        assert_eq!(f.line_origins(), vec![(0.0, 8.0), (0.0, 18.0)]);
        assert_eq!(f.visible_string_range(), 0..11);
        assert_eq!(f.lines()[0].line.string_range(), 0..6);
        // A frame one line tall shows the first line only.
        let short = fs.create_frame(0..0, rect(50.0, 10.0));
        assert_eq!(short.lines().len(), 1);
        assert_eq!(short.visible_string_range(), 0..6);
        assert!(!short.fits_all(11));
        assert!(f.fits_all(11));
    }

    #[test]
    fn frame_hit_testing_caret_and_selection() {
        let f = Framesetter::new(ts("aa bb cc dd")).create_frame(0..0, rect(50.0, 100.0));
        assert_eq!(f.string_index_for_point(100.0, 15.0), Some(11));
        assert_eq!(f.string_index_for_point(0.0, 12.0), Some(6));
        assert_eq!(f.string_index_for_point(0.0, 5.0), Some(0));
        assert_eq!(f.string_index_for_point(0.0, 500.0), Some(6));
        // A soft wrap index belongs to the next line.
        assert_eq!(f.caret_rect(6), Some(Rect { x: 0.0, y: 10.0, w: 0.0, h: 10.0 }));
        assert_eq!(f.caret_rect(2), Some(Rect { x: 20.0, y: 0.0, w: 0.0, h: 10.0 }));
        assert_eq!(f.caret_rect(11), Some(Rect { x: 50.0, y: 10.0, w: 0.0, h: 10.0 }));
        assert_eq!(f.selection_rects(3..8), vec![
            Rect { x: 30.0, y: 0.0, w: 30.0, h: 10.0 },
            Rect { x: 0.0, y: 10.0, w: 20.0, h: 10.0 },
        ]);
    }

    #[test]
    fn caret_after_a_trailing_break_is_on_the_next_line() {
        let f = Framesetter::new(ts("ab\n")).create_frame(0..0, rect(100.0, 100.0));
        assert_eq!(f.lines().len(), 1);
        assert_eq!(f.caret_rect(3), Some(Rect { x: 0.0, y: 10.0, w: 0.0, h: 10.0 }));
        assert_eq!(f.caret_rect(2), Some(Rect { x: 20.0, y: 0.0, w: 0.0, h: 10.0 }));
        // A click past the end of the first line stays before the break.
        assert_eq!(f.string_index_for_point(300.0, 5.0), Some(2));
    }

    #[test]
    fn paragraph_style_aligns_indents_and_spaces() {
        let right = ParagraphStyle { alignment: Alignment::Right, ..Default::default() };
        let f = Framesetter::new(ts_para("aa", right)).create_frame(0..0, rect(100.0, 100.0));
        assert_eq!(f.line_origins()[0].0, 80.0);
        let center = ParagraphStyle { alignment: Alignment::Center, ..Default::default() };
        let f = Framesetter::new(ts_para("aa", center)).create_frame(0..0, rect(100.0, 100.0));
        assert_eq!(f.line_origins()[0].0, 40.0);
        let indent = ParagraphStyle { first_line_head_indent: 30.0, head_indent: 10.0, ..Default::default() };
        let f = Framesetter::new(ts_para("aaaa bbbb cccc", indent)).create_frame(0..0, rect(80.0, 100.0));
        let o = f.line_origins();
        assert_eq!(o[0].0, 30.0);
        assert_eq!(o[1].0, 10.0);
        // The first line has 50 points left: "aaaa " fits, "bbbb" does not.
        assert_eq!(f.lines()[0].line.string_range(), 0..5);
    }

    #[test]
    fn line_height_and_spacing() {
        let ps = ParagraphStyle {
            line_height_multiple: 2.0,
            paragraph_spacing: 5.0,
            paragraph_spacing_before: 3.0,
            ..Default::default()
        };
        let f = Framesetter::new(ts_para("a\nb", ps)).create_frame(0..0, rect(100.0, 100.0));
        let l = f.lines();
        // Height 20: baseline = top + 20 - descent.
        assert_eq!(l[0].origin.1, 18.0);
        assert_eq!(l[0].height, 20.0);
        // Second paragraph: 20 + 5 after the first, 3 before the second.
        assert_eq!(l[1].top, 28.0);
        let spaced = ParagraphStyle { line_spacing: 4.0, ..Default::default() };
        let f2 = Framesetter::new(ts_para("a\nb", spaced)).create_frame(0..0, rect(100.0, 100.0));
        assert_eq!(f2.lines()[1].top, 14.0);
    }

    #[test]
    fn justified_paragraph_fills_all_but_the_last_line() {
        let ps = ParagraphStyle { alignment: Alignment::Justified, ..Default::default() };
        let f = Framesetter::new(ts_para("aa bb cc dd", ps)).create_frame(0..0, rect(60.0, 100.0));
        // "aa bb " is 50 wide without the trailing space; stretched to 60 the second "b" ends at 60.
        let first = &f.lines()[0].line;
        assert_eq!(first.typographic_bounds().width - first.trailing_whitespace_width(), 60.0);
        let last = &f.lines()[f.lines().len() - 1].line;
        assert_eq!(last.typographic_bounds().width, 50.0);
    }

    #[test]
    fn break_modes() {
        let tail = ParagraphStyle { line_break_mode: LineBreakMode::TruncateTail, ..Default::default() };
        let f = Framesetter::new(ts_para("hello world", tail)).create_frame(0..0, rect(60.0, 100.0));
        assert_eq!(f.lines().len(), 1);
        assert_eq!(line_text(&f.lines()[0].line), "hello\u{2026}");
        let chars = ParagraphStyle { line_break_mode: LineBreakMode::CharWrap, ..Default::default() };
        let f = Framesetter::new(ts_para("aa bb cc dd", chars)).create_frame(0..0, rect(50.0, 100.0));
        assert_eq!(f.lines()[0].line.string_range(), 0..6);
        let clip = ParagraphStyle { line_break_mode: LineBreakMode::Clip, ..Default::default() };
        let f = Framesetter::new(ts_para("hello world", clip)).create_frame(0..0, rect(60.0, 100.0));
        assert_eq!(f.lines().len(), 1);
        assert_eq!(f.lines()[0].line.string_range(), 0..11);
    }

    #[test]
    fn suggested_frame_size() {
        let fs = Framesetter::new(ts("aa bb cc dd"));
        let ((w, h), range) = fs.suggest_frame_size(0..0, 50.0, f32::INFINITY);
        assert_eq!((w, h), (50.0, 20.0));
        assert_eq!(range, 0..11);
        let ((w, h), _) = fs.suggest_frame_size(0..0, f32::INFINITY, f32::INFINITY);
        assert_eq!((w, h), (110.0, 10.0));
        let (_, range) = fs.suggest_frame_size(0..0, 50.0, 10.0);
        assert_eq!(range, 0..6);
    }

    #[test]
    fn shapes_limit_the_line_extent() {
        let ell = FramePath::Ellipse(Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 });
        let (a, b) = ell.extent(10.0, 30.0).unwrap();
        assert!(a > 15.0 && b < 85.0, "{a} {b}");
        let mid = ell.extent(40.0, 60.0).unwrap();
        assert!(mid.0 < 10.0 && mid.1 > 90.0);
        assert!(ell.extent(0.0, 200.0).is_none());
        let tri = FramePath::Polygon(vec![(0.0, 0.0), (100.0, 0.0), (50.0, 100.0)]);
        let (a, b) = tri.extent(0.0, 10.0).unwrap();
        assert!(a > 0.0 && a < 10.0 && b > 90.0 && b < 100.0, "{a} {b}");
        assert_eq!(FramePath::Rect(Rect { x: 5.0, y: 0.0, w: 10.0, h: 10.0 }).extent(0.0, 10.0), Some((5.0, 15.0)));
        assert_eq!(tri.bounds(), Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 });
    }

    #[derive(Default)]
    struct Log {
        texts: Vec<(String, f32, f32)>,
        rects: Vec<(Rect, u32)>,
        rules: Vec<(f32, f32, f32, u32)>,
    }

    impl TextPainter for Log {
        fn default_color(&self) -> u32 {
            1
        }
        fn fill_rect(&mut self, rect: Rect, color: u32) {
            self.rects.push((rect, color));
        }
        fn draw_text(&mut self, text: &str, x: f32, baseline: f32, _: &RunStyle) {
            self.texts.push((text.to_string(), x, baseline));
        }
        fn draw_rule(&mut self, x0: f32, x1: f32, y: f32, _t: f32, color: u32) {
            self.rules.push((x0, x1, y, color));
        }
    }

    #[test]
    fn drawing_coalesces_glyphs_and_adds_decorations() {
        let style = RunStyle { underline: UnderlineStyle::Single, background: Some(7), underline_color: Some(9), ..plain() };
        let t = Typesetter::new("ab cd", &[(0..5, style)], &[], TypesetterOptions::default(), source());
        let mut log = Log::default();
        Framesetter::new(t).create_frame(0..0, rect(100.0, 100.0)).draw(&mut log, 5.0, 100.0);
        assert_eq!(log.texts, vec![("ab cd".to_string(), 5.0, 108.0)]);
        assert_eq!(log.rects.len(), 1);
        assert_eq!(log.rects[0].1, 7);
        assert_eq!(log.rules, vec![(5.0, 55.0, 109.0, 9)]);
        // A tab splits the text into two draws.
        let t = ts("a\tb");
        let mut log = Log::default();
        t.create_line(0..3).draw(&mut log, 0.0, 0.0);
        assert_eq!(log.texts, vec![("a".to_string(), 0.0, 0.0), ("b".to_string(), 28.0, 0.0)]);
    }

    #[test]
    fn empty_text_has_an_empty_line_and_no_frame_lines() {
        let t = ts("");
        assert!(t.is_empty());
        let l = t.create_line(0..0);
        assert_eq!(l.glyph_count(), 0);
        assert_eq!(l.string_index_for_position(3.0), None);
        let f = Framesetter::new(t).create_frame(0..0, rect(10.0, 10.0));
        assert!(f.lines().is_empty());
        assert_eq!(f.string_index_for_point(0.0, 0.0), None);
    }
}
