//! Text support for the layout engine: the `TextMetrics` trait the engine
//! measures with, a bounded width cache, and white-space collapsing.

use super::super::display::GlyphRun;
use super::super::style::{TextTransform, WhiteSpace};
use std::collections::HashMap;

/// Font measurements the layout engine needs. Implemented by the shell font
/// stack; tests use a fixed-width fake.
pub trait TextMetrics {
    /// Advance width of `text` in px.
    fn width(&self, text: &str, font_size: f32, bold: bool, italic: bool) -> f32;
    /// Distance from the top of a glyph row to the baseline.
    fn ascent(&self, font_size: f32) -> f32 {
        font_size
    }
    /// Distance from the baseline to the bottom of a glyph row.
    fn descent(&self, font_size: f32) -> f32 {
        font_size * 0.25
    }
    /// A rasterised run for painting (origin = top of the glyph row). The
    /// default draws nothing.
    fn glyph_run(
        &self,
        _text: &str,
        _font_size: f32,
        _bold: bool,
        _italic: bool,
    ) -> Option<GlyphRun> {
        None
    }
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Bounded cache of text widths keyed by (hash of text, size, style).
#[derive(Debug)]
pub struct WidthCache {
    map: HashMap<(u64, u32, u8), f32>,
    cap: usize,
}

impl WidthCache {
    pub fn new(cap: usize) -> WidthCache {
        WidthCache {
            map: HashMap::new(),
            cap: cap.max(16),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The width of `text`, measured once per key.
    pub fn width(
        &mut self,
        m: &dyn TextMetrics,
        text: &str,
        size: f32,
        bold: bool,
        italic: bool,
    ) -> f32 {
        let flags = (bold as u8) | ((italic as u8) << 1);
        let key = (
            fnv(text) ^ ((text.len() as u64) << 48),
            size.to_bits(),
            flags,
        );
        if let Some(w) = self.map.get(&key) {
            return *w;
        }
        let w = m.width(text, size, bold, italic);
        if self.map.len() >= self.cap {
            self.map.clear();
        }
        self.map.insert(key, w);
        w
    }
}

/// Whether the mode keeps line feeds as forced breaks.
pub fn keeps_newlines(w: WhiteSpace) -> bool {
    matches!(
        w,
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::PreLine | WhiteSpace::BreakSpaces
    )
}

/// Whether the mode keeps runs of spaces.
pub fn keeps_spaces(w: WhiteSpace) -> bool {
    matches!(
        w,
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
    )
}

/// Whether the mode never wraps at spaces.
pub fn no_wrap(w: WhiteSpace) -> bool {
    matches!(w, WhiteSpace::Nowrap | WhiteSpace::Pre)
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// Appends `src` to `out` with CSS white-space processing and text-transform
/// applied. `prev_space` carries "the last emitted char was a collapsible
/// space" across text nodes.
pub fn collapse_into(
    src: &str,
    ws: WhiteSpace,
    tt: TextTransform,
    prev_space: &mut bool,
    out: &mut String,
) {
    let mut cap_next = true;
    let push = |c: char, out: &mut String, cap_next: &mut bool| match tt {
        TextTransform::Uppercase => out.extend(c.to_uppercase()),
        TextTransform::Lowercase => out.extend(c.to_lowercase()),
        TextTransform::Capitalize => {
            if *cap_next && c.is_alphabetic() {
                out.extend(c.to_uppercase());
                *cap_next = false;
            } else {
                out.push(c);
            }
        }
        TextTransform::None => out.push(c),
    };
    for c in src.chars() {
        if keeps_spaces(ws) {
            match c {
                '\r' => {}
                '\t' => {
                    for _ in 0..4 {
                        out.push(' ');
                    }
                }
                _ => push(c, out, &mut cap_next),
            }
            *prev_space = false;
            if is_ws(c) {
                cap_next = true;
            }
            continue;
        }
        if c == '\n' && ws == WhiteSpace::PreLine {
            while out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
            *prev_space = true;
            cap_next = true;
            continue;
        }
        if is_ws(c) {
            if !*prev_space {
                out.push(' ');
                *prev_space = true;
            }
            cap_next = true;
        } else {
            push(c, out, &mut cap_next);
            *prev_space = false;
        }
    }
}

/// Whether `s` holds only collapsible white space.
pub fn only_space(s: &str) -> bool {
    s.chars().all(is_ws)
}

#[cfg(test)]
pub mod fake {
    use super::*;

    /// Every char is `cw` px wide at size 10 (scales with the font size).
    pub struct Mono(pub f32);

    impl TextMetrics for Mono {
        fn width(&self, text: &str, font_size: f32, _b: bool, _i: bool) -> f32 {
            text.chars().count() as f32 * self.0 * font_size / 10.0
        }
        fn glyph_run(&self, text: &str, font_size: f32, _b: bool, _i: bool) -> Option<GlyphRun> {
            let w = self.width(text, font_size, false, false).ceil().max(1.0) as usize;
            let h = (font_size * 1.25).ceil().max(1.0) as usize;
            Some(GlyphRun::new(w, h, vec![255; w * h]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Mono;
    use super::*;

    fn run(src: &str, ws: WhiteSpace, tt: TextTransform) -> String {
        let mut out = String::new();
        let mut prev = false;
        collapse_into(src, ws, tt, &mut prev, &mut out);
        out
    }

    #[test]
    fn collapses_runs_of_space() {
        assert_eq!(
            run("a  \n\t b", WhiteSpace::Normal, TextTransform::None),
            "a b"
        );
    }

    #[test]
    fn pre_keeps_everything() {
        assert_eq!(
            run("a  b\nc", WhiteSpace::Pre, TextTransform::None),
            "a  b\nc"
        );
    }

    #[test]
    fn pre_line_keeps_newlines_only() {
        assert_eq!(
            run("a  b \n c", WhiteSpace::PreLine, TextTransform::None),
            "a b\nc"
        );
    }

    #[test]
    fn space_state_carries_across_nodes() {
        let mut out = String::new();
        let mut prev = false;
        collapse_into(
            "a ",
            WhiteSpace::Normal,
            TextTransform::None,
            &mut prev,
            &mut out,
        );
        collapse_into(
            " b",
            WhiteSpace::Normal,
            TextTransform::None,
            &mut prev,
            &mut out,
        );
        assert_eq!(out, "a b");
    }

    #[test]
    fn transforms() {
        assert_eq!(
            run("ab cd", WhiteSpace::Normal, TextTransform::Uppercase),
            "AB CD"
        );
        assert_eq!(
            run("ab cd", WhiteSpace::Normal, TextTransform::Capitalize),
            "Ab Cd"
        );
    }

    #[test]
    fn cache_is_bounded_and_correct() {
        let m = Mono(8.0);
        let mut c = WidthCache::new(16);
        assert_eq!(c.width(&m, "abc", 10.0, false, false), 24.0);
        assert_eq!(c.width(&m, "abc", 10.0, false, false), 24.0);
        assert_eq!(c.len(), 1);
        for i in 0..100 {
            c.width(&m, &format!("w{}", i), 10.0, false, false);
        }
        assert!(c.len() <= 16);
    }
}
