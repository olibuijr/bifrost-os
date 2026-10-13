//! Text content: a string or an attributed string drawn into a box with a font size, weight and colour, optional
//! word wrapping, an alignment mode and a truncation mode. `layout_lines` is the pure line breaker; `TextContent::render`
//! draws the lines with the UI font.
use super::raster::blit_over;
use crate::attributed::{Attrs, AttributedString};
use bifrost_render::gfx::Image;
use bifrost_render::ui::{Ui, Weight};

/// Line height as a multiple of the point size.
const LINE_HEIGHT: f32 = 1.25;
const ELLIPSIS: char = '\u{2026}';

/// Horizontal placement of each line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    /// The writing direction's leading edge (left; right-to-left text is not mirrored here).
    #[default]
    Natural,
    Left,
    Right,
    Center,
    /// Wrapped lines other than the last of a paragraph are spread to the full width.
    Justified,
}

/// Which part of an over-long line is replaced by an ellipsis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Truncation {
    /// Over-long text is clipped by the box.
    #[default]
    None,
    Start,
    Middle,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextSource {
    Plain(String),
    Attributed(AttributedString),
}

impl TextSource {
    pub fn string(&self) -> String {
        match self {
            TextSource::Plain(s) => s.clone(),
            TextSource::Attributed(a) => a.string(),
        }
    }
}

/// One laid-out line.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLine {
    pub text: String,
    /// The character range of the source this line shows unchanged; `None` for a truncated line.
    pub range: Option<(usize, usize)>,
    /// The last line of its paragraph.
    pub last: bool,
}

fn sub(chars: &[char], a: usize, b: usize) -> String {
    chars[a.min(chars.len())..b.min(chars.len())].iter().collect()
}

/// Breaks `text` into lines no wider than `max_w`. Newlines always break. With `wrapped` a paragraph breaks at spaces
/// (and inside a word wider than the box); without it each paragraph is one line. A line wider than the box is
/// truncated by `trunc`. When more than `max_lines` lines result, the rest is dropped; with a truncation mode the last
/// kept line carries the rest of the text, truncated.
pub fn layout_lines(measure: &dyn Fn(&str) -> f32, text: &str, max_w: f32, wrapped: bool, trunc: Truncation, max_lines: usize) -> Vec<TextLine> {
    let chars: Vec<char> = text.chars().collect();
    let mut lines: Vec<TextLine> = Vec::new();
    let mut start = 0usize;
    while start <= chars.len() {
        let end = chars[start..].iter().position(|&c| c == '\n').map_or(chars.len(), |p| start + p);
        let ranges = if wrapped { wrap(measure, &chars, start, end, max_w) } else { vec![(start, end)] };
        let n = ranges.len();
        for (i, (a, b)) in ranges.into_iter().enumerate() {
            lines.push(TextLine { text: sub(&chars, a, b), range: Some((a, b)), last: i + 1 == n });
        }
        if end >= chars.len() {
            break;
        }
        start = end + 1;
    }
    if text.is_empty() {
        return Vec::new();
    }
    if !wrapped && trunc != Truncation::None {
        for l in &mut lines {
            if measure(&l.text) > max_w {
                l.text = truncate(measure, &l.text, max_w, trunc);
                l.range = None;
            }
        }
    }
    let max_lines = max_lines.max(1);
    if lines.len() > max_lines {
        if trunc != Truncation::None && wrapped {
            let rest: Vec<&str> = lines[max_lines - 1..].iter().map(|l| l.text.as_str()).collect();
            let joined = rest.join(" ");
            let cut = truncate(measure, &joined, max_w, trunc);
            // The ellipsis must show even when the tail happens to fit.
            let text = if cut == joined { format!("{}{}", joined.trim_end(), ELLIPSIS) } else { cut };
            lines.truncate(max_lines);
            lines[max_lines - 1] = TextLine { text, range: None, last: true };
        } else {
            lines.truncate(max_lines);
        }
    }
    lines
}

fn wrap(measure: &dyn Fn(&str) -> f32, chars: &[char], a: usize, b: usize, max_w: f32) -> Vec<(usize, usize)> {
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut i = a;
    while i < b {
        if chars[i] == ' ' {
            i += 1;
            continue;
        }
        let s = i;
        while i < b && chars[i] != ' ' {
            i += 1;
        }
        words.push((s, i));
    }
    if words.is_empty() {
        return vec![(a, a)];
    }
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let (mut ls, mut le) = words[0];
    for &(s, e) in &words[1..] {
        if measure(&sub(chars, ls, e)) <= max_w {
            le = e;
        } else {
            ranges.push((ls, le));
            ls = s;
            le = e;
        }
    }
    ranges.push((ls, le));
    // A single word wider than the box breaks between characters.
    let mut out = Vec::new();
    for (s, e) in ranges {
        if measure(&sub(chars, s, e)) <= max_w || e - s < 2 {
            out.push((s, e));
            continue;
        }
        let mut from = s;
        while from < e {
            let mut to = from + 1;
            while to < e && measure(&sub(chars, from, to + 1)) <= max_w {
                to += 1;
            }
            out.push((from, to));
            from = to;
        }
    }
    out
}

/// `s` shortened with an ellipsis so it fits `max_w`, removing characters from the `mode` end. A string that already
/// fits is returned as it is.
pub fn truncate(measure: &dyn Fn(&str) -> f32, s: &str, max_w: f32, mode: Truncation) -> String {
    if mode == Truncation::None || measure(s) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let make = |k: usize| -> String {
        match mode {
            Truncation::Start => format!("{}{}", ELLIPSIS, sub(&chars, n - k, n)),
            Truncation::Middle => format!("{}{}{}", sub(&chars, 0, k.div_ceil(2)), ELLIPSIS, sub(&chars, n - k / 2, n)),
            _ => format!("{}{}", sub(&chars, 0, k), ELLIPSIS),
        }
    };
    // The longest kept length whose result fits (width grows with the kept length).
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if measure(&make(mid)) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    make(lo)
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextContent {
    pub source: TextSource,
    /// Point size of plain text; attributed runs bring their own size.
    pub pt: f32,
    pub weight: Weight,
    /// Packed premultiplied colour of plain text and of runs without a colour.
    pub color: u32,
    pub wrapped: bool,
    pub align: TextAlign,
    pub truncation: Truncation,
}

impl TextContent {
    pub fn new(text: &str, pt: f32, color: u32) -> Self {
        TextContent { source: TextSource::Plain(text.into()), pt, weight: Weight::Regular, color, wrapped: false, align: TextAlign::default(), truncation: Truncation::None }
    }

    pub fn attributed(text: AttributedString, color: u32) -> Self {
        let mut t = TextContent::new("", bifrost_render::ui::MAC_BODY_PT, color);
        t.source = TextSource::Attributed(text);
        t
    }

    /// Line height in device pixels.
    fn line_h(&self, ui: &Ui) -> usize {
        let pt = match &self.source {
            TextSource::Attributed(a) => {
                let tallest = a.runs().map(|(_, at)| at.size.pt()).fold(0.0f32, f32::max);
                if tallest > 0.0 { tallest } else { self.pt }
            }
            TextSource::Plain(_) => self.pt,
        };
        (ui.pt(pt * LINE_HEIGHT)).round().max(1.0) as usize
    }

    /// The lines that fit a box `w` x `h` device pixels.
    pub fn lines(&self, ui: &Ui, w: usize, h: usize) -> Vec<TextLine> {
        let lh = self.line_h(ui);
        let max_lines = (h / lh).max(1);
        let measure = |s: &str| ui.measure(s, self.pt, self.weight);
        layout_lines(&measure, &self.source.string(), w as f32, self.wrapped, self.truncation, max_lines)
    }

    /// Draws into a `w` x `h` device pixel image, top aligned.
    pub fn render(&self, ui: &Ui, w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        if w == 0 || h == 0 {
            return img;
        }
        let lh = self.line_h(ui);
        for (i, line) in self.lines(ui, w, h).iter().enumerate() {
            let y = (i * lh) as i32;
            let pieces = self.pieces(ui, line);
            let lw: usize = pieces.iter().map(|p| p.0.w).sum();
            let free = w as i32 - lw as i32;
            let word_gaps = line.text.split(' ').filter(|s| !s.is_empty()).count().saturating_sub(1);
            if self.align == TextAlign::Justified && self.wrapped && !line.last && word_gaps > 0 && line.range.is_some() {
                self.justified(ui, line, w, y, &mut img);
                continue;
            }
            let x = match self.align {
                TextAlign::Right => free,
                TextAlign::Center => free / 2,
                _ => 0,
            };
            let mut cx = x;
            for (piece, deco) in &pieces {
                blit_over(&mut img, piece, cx, y);
                for (frac, color) in deco {
                    let ly = y + (lh as f32 * frac) as i32;
                    let thickness = (ui.pt(1.0).round() as i32).max(1);
                    for dy in 0..thickness {
                        for dx in 0..piece.w as i32 {
                            put(&mut img, cx + dx, ly + dy, *color);
                        }
                    }
                }
                cx += piece.w as i32;
            }
        }
        img
    }

    /// Draws a justified line word by word with the free space shared over the gaps.
    fn justified(&self, ui: &Ui, line: &TextLine, w: usize, y: i32, img: &mut Image) {
        let words: Vec<&str> = line.text.split(' ').filter(|s| !s.is_empty()).collect();
        let imgs: Vec<Image> = words.iter().map(|wd| ui.text(wd, self.pt, self.weight, self.color)).collect();
        let total: i32 = imgs.iter().map(|i| i.w as i32).sum();
        let gap = (w as i32 - total).max(0) as f32 / (words.len() - 1) as f32;
        let mut x = 0.0f32;
        for i in &imgs {
            blit_over(img, i, x.round() as i32, y);
            x += i.w as f32 + gap;
        }
    }

    /// The rendered segments of a line, each with the decoration lines (vertical fraction of the line, colour) to draw.
    fn pieces(&self, ui: &Ui, line: &TextLine) -> Vec<(Image, Vec<(f32, u32)>)> {
        let attributed = match (&self.source, line.range) {
            (TextSource::Attributed(a), Some(r)) => Some((a, r)),
            _ => None,
        };
        let Some((a, (from, to))) = attributed else {
            return vec![(ui.text(&line.text, self.pt, self.weight, self.color), Vec::new())];
        };
        // Group the line's characters into runs of equal attributes.
        let mut runs: Vec<(String, Attrs)> = Vec::new();
        let mut pos = 0usize;
        for (text, attrs) in a.runs() {
            for c in text.chars() {
                if pos >= from && pos < to {
                    match runs.last_mut() {
                        Some(last) if last.1 == *attrs => last.0.push(c),
                        _ => runs.push((c.to_string(), attrs.clone())),
                    }
                }
                pos += 1;
            }
        }
        runs.into_iter()
            .map(|(text, at)| {
                let color = at.color.map_or(self.color, |t| t.now());
                let weight = if at.bold { Weight::Bold } else { self.weight };
                let mut deco = Vec::new();
                if at.underline {
                    deco.push((0.9, color));
                }
                if at.strikethrough {
                    deco.push((0.5, color));
                }
                (ui.text(&text, at.size.pt(), weight, color), deco)
            })
            .collect()
    }
}

fn put(img: &mut Image, x: i32, y: i32, color: u32) {
    if x >= 0 && y >= 0 && (x as usize) < img.w && (y as usize) < img.h {
        let p = &mut img.px[y as usize * img.w + x as usize];
        *p = bifrost_render::gfx::over(*p, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character is 10 wide.
    fn m(s: &str) -> f32 {
        s.chars().count() as f32 * 10.0
    }

    fn texts(lines: &[TextLine]) -> Vec<&str> {
        lines.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn wrapping_breaks_at_spaces() {
        let l = layout_lines(&m, "aa bb cc dd", 50.0, true, Truncation::None, 99);
        assert_eq!(texts(&l), ["aa bb", "cc dd"]);
        assert!(!l[0].last && l[1].last);
        assert_eq!(l[1].range, Some((6, 11)));
    }

    #[test]
    fn unwrapped_text_is_one_line_per_paragraph() {
        let l = layout_lines(&m, "aa bb cc dd\nxy", 50.0, false, Truncation::None, 99);
        assert_eq!(texts(&l), ["aa bb cc dd", "xy"]);
    }

    #[test]
    fn a_word_wider_than_the_box_breaks_between_characters() {
        let l = layout_lines(&m, "abcdefgh", 30.0, true, Truncation::None, 99);
        assert_eq!(texts(&l), ["abc", "def", "gh"]);
    }

    #[test]
    fn end_start_and_middle_truncation_fit_with_an_ellipsis() {
        let s = "abcdefghij";
        let e = truncate(&m, s, 50.0, Truncation::End);
        let st = truncate(&m, s, 50.0, Truncation::Start);
        let mid = truncate(&m, s, 50.0, Truncation::Middle);
        assert_eq!(e, "abcd\u{2026}");
        assert_eq!(st, "\u{2026}ghij");
        assert_eq!(mid, "ab\u{2026}ij");
        assert!(m(&mid) <= 50.0);
        assert_eq!(truncate(&m, "abc", 50.0, Truncation::End), "abc");
        assert_eq!(truncate(&m, s, 50.0, Truncation::None), s);
    }

    #[test]
    fn an_unwrapped_long_line_is_truncated_only_when_a_mode_is_set() {
        let l = layout_lines(&m, "abcdefghij", 50.0, false, Truncation::End, 5);
        assert_eq!(texts(&l), ["abcd\u{2026}"]);
        assert_eq!(l[0].range, None);
        let l = layout_lines(&m, "abcdefghij", 50.0, false, Truncation::None, 5);
        assert_eq!(texts(&l), ["abcdefghij"]);
    }

    #[test]
    fn too_many_wrapped_lines_end_in_a_truncated_last_line() {
        let l = layout_lines(&m, "aa bb cc dd ee ff", 50.0, true, Truncation::End, 2);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].text, "aa bb");
        assert!(l[1].text.ends_with('\u{2026}'), "{:?}", l[1].text);
        assert!(m(&l[1].text) <= 50.0);
        let clipped = layout_lines(&m, "aa bb cc dd ee ff", 50.0, true, Truncation::None, 2);
        assert_eq!(texts(&clipped), ["aa bb", "cc dd"]);
    }

    #[test]
    fn empty_text_has_no_lines_and_blank_lines_are_kept() {
        assert!(layout_lines(&m, "", 50.0, true, Truncation::None, 9).is_empty());
        assert_eq!(texts(&layout_lines(&m, "a\n\nb", 50.0, true, Truncation::None, 9)), ["a", "", "b"]);
    }
}
