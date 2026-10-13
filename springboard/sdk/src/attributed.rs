//! `NativeAttributedString` / `NativeMutableAttributedString` parity plus a rich text editing model (`NativeTextView` rich mode).
//!
//! The classic range methods count characters; the `*_utf16` methods take ranges in UTF-16 units, the unit of the
//! platform's range type, and are what the layout objects in `richlayout` use.
//! Sizes are the Golden Gate text styles ([`SizeClass`], from `render/src/ui.rs` `MAC_*_PT`), colours are named tokens
//! ([`ColorToken`], resolved with `Pair::now()` so they follow Light and Dark). No literal sizes or colours.
use bifrost_render::ui::{self, system};
use bifrost_ui::paragraph::ParagraphStyle;
use bifrost_ui::typeset::UnderlineStyle;
use bifrost_ui::undo::UndoManager;
use std::ops::Range;

/// A Golden Gate text style; the point size comes from the `MAC_*_PT` constants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SizeClass {
    LargeTitle,
    Title1,
    Title2,
    Title3,
    Headline,
    #[default]
    Body,
    Callout,
    Subhead,
    Footnote,
    Caption,
}

impl SizeClass {
    const ALL: [SizeClass; 10] = [
        SizeClass::Body, SizeClass::LargeTitle, SizeClass::Title1, SizeClass::Title2, SizeClass::Title3,
        SizeClass::Headline, SizeClass::Callout, SizeClass::Subhead, SizeClass::Footnote, SizeClass::Caption,
    ];
    pub fn pt(self) -> f32 {
        match self {
            SizeClass::LargeTitle => ui::MAC_LARGE_TITLE_PT,
            SizeClass::Title1 => ui::MAC_TITLE1_PT,
            SizeClass::Title2 => ui::MAC_TITLE2_PT,
            SizeClass::Title3 => ui::MAC_TITLE3_PT,
            SizeClass::Headline => ui::MAC_HEADLINE_PT,
            SizeClass::Body => ui::MAC_BODY_PT,
            SizeClass::Callout => ui::MAC_CALLOUT_PT,
            SizeClass::Subhead => ui::MAC_SUBHEAD_PT,
            SizeClass::Footnote => ui::MAC_FOOTNOTE_PT,
            SizeClass::Caption => ui::MAC_CAPTION1_PT,
        }
    }
    /// The class nearest to `pt` (RTF `\fs` import). Equal sizes resolve to the first listed class (Body for
    /// 13 pt, Footnote for 10 pt), so a Body or Footnote run survives an RTF round trip.
    pub fn nearest(pt: f32) -> SizeClass {
        let mut best = SizeClass::Body;
        let mut d = f32::MAX;
        for c in Self::ALL {
            let e = (c.pt() - pt).abs();
            if e < d {
                best = c;
                d = e;
            }
        }
        best
    }
}

/// Named colours (`NativeColor` semantic names).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorToken {
    Label,
    SecondaryLabel,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
}

impl ColorToken {
    pub const ALL: [ColorToken; 7] = [
        ColorToken::Label, ColorToken::SecondaryLabel, ColorToken::Red, ColorToken::Orange, ColorToken::Yellow,
        ColorToken::Green, ColorToken::Blue,
    ];
    pub fn name(self) -> &'static str {
        match self {
            ColorToken::Label => "label",
            ColorToken::SecondaryLabel => "secondaryLabel",
            ColorToken::Red => "red",
            ColorToken::Orange => "orange",
            ColorToken::Yellow => "yellow",
            ColorToken::Green => "green",
            ColorToken::Blue => "blue",
        }
    }
    pub fn from_name(n: &str) -> Option<ColorToken> {
        Self::ALL.into_iter().find(|c| c.name() == n)
    }
    /// The resolved colour for the current appearance.
    pub fn now(self) -> u32 {
        match self {
            ColorToken::Label => system::LABEL.now(),
            ColorToken::SecondaryLabel => system::SECONDARY_LABEL.now(),
            ColorToken::Red => system::SYSTEM_RED.now(),
            ColorToken::Orange => system::SYSTEM_ORANGE.now(),
            ColorToken::Yellow => system::SYSTEM_YELLOW.now(),
            ColorToken::Green => system::SYSTEM_GREEN.now(),
            ColorToken::Blue => system::SYSTEM_BLUE.now(),
        }
    }
    /// 8-bit RGB of the current appearance (RTF colour table export).
    pub fn rgb(self) -> (u8, u8, u8) {
        let c = self.now();
        ((c >> 16) as u8, (c >> 8) as u8, c as u8)
    }
    /// The token whose current colour is nearest to an RGB triple (RTF colour table import). An exact match wins.
    pub fn nearest_rgb(r: u8, g: u8, b: u8) -> ColorToken {
        let mut best = ColorToken::Label;
        let mut d = i64::MAX;
        for t in Self::ALL {
            let (tr, tg, tb) = t.rgb();
            let e = (tr as i64 - r as i64).pow(2) + (tg as i64 - g as i64).pow(2) + (tb as i64 - b as i64).pow(2);
            if e < d {
                d = e;
                best = t;
            }
        }
        best
    }
}

/// The attributes of one run. `color: None` means the default text colour.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Attrs {
    pub bold: bool,
    pub italic: bool,
    pub size: SizeClass,
    pub color: Option<ColorToken>,
    pub underline: bool,
    pub strikethrough: bool,
    pub link: Option<String>,
    /// A font family by name; `None` is the system font. Layout and the line metrics use the name; drawing uses the
    /// system face until the renderer can select a face per run.
    pub font_family: Option<String>,
    /// A point size overriding `size`.
    pub font_size: Option<f32>,
    /// A literal text colour, in the renderer's colour format, overriding `color`.
    pub foreground: Option<u32>,
    pub background: Option<u32>,
    /// Raises (positive) or lowers the baseline, in points.
    pub baseline_offset: f32,
    /// 1 superscript, -1 subscript, 0 neither.
    pub superscript: i8,
    /// Extra space after each character in points, and as a fraction of the font size.
    pub kern: f32,
    pub tracking: f32,
    /// Outline width as a percentage of the font size; negative strokes and fills.
    pub stroke_width: f32,
    pub stroke_color: Option<u32>,
    pub underline_color: Option<u32>,
    /// `None` with `underline` set is a single line.
    pub underline_style: UnderlineStyle,
    /// The paragraph style, read from the first character of each paragraph.
    pub paragraph: Option<ParagraphStyle>,
}

/// One attribute and its value (`NativeAttributedString.Key` plus value).
#[derive(Clone, Debug, PartialEq)]
pub enum Attribute {
    Bold(bool),
    Italic(bool),
    Size(SizeClass),
    Color(Option<ColorToken>),
    Underline(bool),
    Strikethrough(bool),
    Link(Option<String>),
    FontFamily(Option<String>),
    FontSize(Option<f32>),
    Foreground(Option<u32>),
    Background(Option<u32>),
    BaselineOffset(f32),
    Superscript(i8),
    Kern(f32),
    Tracking(f32),
    StrokeWidth(f32),
    StrokeColor(Option<u32>),
    UnderlineColor(Option<u32>),
    UnderlineStyle(UnderlineStyle),
    Paragraph(Option<ParagraphStyle>),
}

/// Which attribute to enumerate (`enumerateAttribute(_:in:)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeKey {
    Bold,
    Italic,
    Size,
    Color,
    Underline,
    Strikethrough,
    Link,
    FontFamily,
    FontSize,
    Foreground,
    Background,
    BaselineOffset,
    Superscript,
    Kern,
    Tracking,
    StrokeWidth,
    StrokeColor,
    UnderlineColor,
    UnderlineStyle,
    Paragraph,
}

impl Attrs {
    pub fn set(&mut self, a: &Attribute) {
        match a.clone() {
            Attribute::Bold(v) => self.bold = v,
            Attribute::Italic(v) => self.italic = v,
            Attribute::Size(v) => self.size = v,
            Attribute::Color(v) => self.color = v,
            Attribute::Underline(v) => self.underline = v,
            Attribute::Strikethrough(v) => self.strikethrough = v,
            Attribute::Link(v) => self.link = v,
            Attribute::FontFamily(v) => self.font_family = v,
            Attribute::FontSize(v) => self.font_size = v,
            Attribute::Foreground(v) => self.foreground = v,
            Attribute::Background(v) => self.background = v,
            Attribute::BaselineOffset(v) => self.baseline_offset = v,
            Attribute::Superscript(v) => self.superscript = v,
            Attribute::Kern(v) => self.kern = v,
            Attribute::Tracking(v) => self.tracking = v,
            Attribute::StrokeWidth(v) => self.stroke_width = v,
            Attribute::StrokeColor(v) => self.stroke_color = v,
            Attribute::UnderlineColor(v) => self.underline_color = v,
            Attribute::UnderlineStyle(v) => self.underline_style = v,
            Attribute::Paragraph(v) => self.paragraph = v,
        }
    }
    pub fn get(&self, k: AttributeKey) -> Attribute {
        match k {
            AttributeKey::Bold => Attribute::Bold(self.bold),
            AttributeKey::Italic => Attribute::Italic(self.italic),
            AttributeKey::Size => Attribute::Size(self.size),
            AttributeKey::Color => Attribute::Color(self.color),
            AttributeKey::Underline => Attribute::Underline(self.underline),
            AttributeKey::Strikethrough => Attribute::Strikethrough(self.strikethrough),
            AttributeKey::Link => Attribute::Link(self.link.clone()),
            AttributeKey::FontFamily => Attribute::FontFamily(self.font_family.clone()),
            AttributeKey::FontSize => Attribute::FontSize(self.font_size),
            AttributeKey::Foreground => Attribute::Foreground(self.foreground),
            AttributeKey::Background => Attribute::Background(self.background),
            AttributeKey::BaselineOffset => Attribute::BaselineOffset(self.baseline_offset),
            AttributeKey::Superscript => Attribute::Superscript(self.superscript),
            AttributeKey::Kern => Attribute::Kern(self.kern),
            AttributeKey::Tracking => Attribute::Tracking(self.tracking),
            AttributeKey::StrokeWidth => Attribute::StrokeWidth(self.stroke_width),
            AttributeKey::StrokeColor => Attribute::StrokeColor(self.stroke_color),
            AttributeKey::UnderlineColor => Attribute::UnderlineColor(self.underline_color),
            AttributeKey::UnderlineStyle => Attribute::UnderlineStyle(self.underline_style),
            AttributeKey::Paragraph => Attribute::Paragraph(self.paragraph.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Run {
    text: String,
    attrs: Attrs,
}

/// A string with per-range attributes. Runs are kept merged, so equal content compares equal.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct AttributedString {
    runs: Vec<Run>,
}

fn char_to_byte(s: &str, c: usize) -> usize {
    s.char_indices().nth(c).map_or(s.len(), |(i, _)| i)
}

/// One run of an [`AttributedString`] as the accessibility text export sees it (`AXTextAttributedString`). The range is in
/// UTF-16 units, the unit of the platform's range type, not in chars.
#[derive(Clone, Debug, PartialEq)]
pub struct AxAttrRun {
    /// first UTF-16 unit of the run
    pub utf16_start: usize,
    /// UTF-16 units in the run
    pub utf16_len: usize,
    pub attrs: Attrs,
}

impl AttributedString {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn plain(text: &str) -> Self {
        Self::with_attrs(text, Attrs::default())
    }
    pub fn with_attrs(text: &str, attrs: Attrs) -> Self {
        let mut s = Self::new();
        s.append(text, attrs);
        s
    }
    /// Number of characters.
    pub fn len(&self) -> usize {
        self.runs.iter().map(|r| r.text.chars().count()).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
    /// The plain text (`string`).
    pub fn string(&self) -> String {
        self.runs.iter().map(|r| r.text.as_str()).collect()
    }
    /// Attributes of the character at `index` (`attributes(at:)`); the last run's for `index == len`.
    pub fn attributes_at(&self, index: usize) -> Attrs {
        let mut pos = 0;
        for r in &self.runs {
            let n = r.text.chars().count();
            if index < pos + n {
                return r.attrs.clone();
            }
            pos += n;
        }
        self.runs.last().map(|r| r.attrs.clone()).unwrap_or_default()
    }
    pub fn append(&mut self, text: &str, attrs: Attrs) {
        if text.is_empty() {
            return;
        }
        self.runs.push(Run { text: text.to_string(), attrs });
        self.normalize();
    }
    pub fn append_attributed(&mut self, other: &AttributedString) {
        self.runs.extend(other.runs.iter().cloned());
        self.normalize();
    }
    fn normalize(&mut self) {
        let mut out: Vec<Run> = Vec::new();
        for r in self.runs.drain(..) {
            if r.text.is_empty() {
                continue;
            }
            match out.last_mut() {
                Some(l) if l.attrs == r.attrs => l.text.push_str(&r.text),
                _ => out.push(r),
            }
        }
        self.runs = out;
    }
    /// Splits runs so `at` (a character index) is a run boundary.
    fn split_at(&mut self, at: usize) {
        let mut pos = 0;
        for i in 0..self.runs.len() {
            let n = self.runs[i].text.chars().count();
            if at > pos && at < pos + n {
                let b = char_to_byte(&self.runs[i].text, at - pos);
                let tail = self.runs[i].text.split_off(b);
                let attrs = self.runs[i].attrs.clone();
                self.runs.insert(i + 1, Run { text: tail, attrs });
                return;
            }
            pos += n;
        }
    }
    fn clamp(&self, r: Range<usize>) -> Range<usize> {
        let n = self.len();
        let s = r.start.min(n);
        s..r.end.clamp(s, n)
    }
    /// `attributedSubstring(from:)`.
    pub fn substring(&self, range: Range<usize>) -> AttributedString {
        let range = self.clamp(range);
        let mut c = self.clone();
        c.split_at(range.start);
        c.split_at(range.end);
        let mut out = AttributedString::new();
        let mut pos = 0;
        for r in &c.runs {
            let n = r.text.chars().count();
            if pos >= range.start && pos + n <= range.end {
                out.runs.push(r.clone());
            }
            pos += n;
        }
        out.normalize();
        out
    }
    /// `replaceCharacters(in:with:)` for plain text: the new text takes the attributes of the character before the
    /// range (or the first replaced one at the start), as `NativeTextView` typing does.
    pub fn replace_characters(&mut self, range: Range<usize>, with: &str) {
        let range = self.clamp(range);
        let attrs = if range.start > 0 { self.attributes_at(range.start - 1) } else { self.attributes_at(range.start) };
        let ins = AttributedString::with_attrs(with, attrs);
        self.replace_attributed(range, &ins);
    }
    /// `replaceCharacters(in:with: NativeAttributedString)`.
    pub fn replace_attributed(&mut self, range: Range<usize>, with: &AttributedString) {
        let range = self.clamp(range);
        self.split_at(range.start);
        self.split_at(range.end);
        let mut pos = 0;
        let mut out: Vec<Run> = Vec::new();
        let mut inserted = false;
        for r in self.runs.drain(..) {
            let n = r.text.chars().count();
            if pos >= range.start && pos + n <= range.end {
                // inside the replaced range: dropped; the replacement goes in at its place
                if !inserted {
                    out.extend(with.runs.iter().cloned());
                    inserted = true;
                }
            } else {
                if pos >= range.end && !inserted {
                    out.extend(with.runs.iter().cloned());
                    inserted = true;
                }
                out.push(r);
            }
            pos += n;
        }
        if !inserted {
            out.extend(with.runs.iter().cloned());
        }
        self.runs = out;
        self.normalize();
    }
    /// `addAttribute(_:value:range:)`.
    pub fn add_attribute(&mut self, attr: Attribute, range: Range<usize>) {
        let range = self.clamp(range);
        if range.is_empty() {
            return;
        }
        self.split_at(range.start);
        self.split_at(range.end);
        let mut pos = 0;
        for r in &mut self.runs {
            let n = r.text.chars().count();
            if pos >= range.start && pos + n <= range.end {
                r.attrs.set(&attr);
            }
            pos += n;
        }
        self.normalize();
    }
    /// `removeAttribute(_:range:)`: resets the key to its default.
    pub fn remove_attribute(&mut self, key: AttributeKey, range: Range<usize>) {
        let d = Attrs::default().get(key);
        self.add_attribute(d, range);
    }
    /// `enumerateAttribute(_:in:)`: calls `f` for each maximal range over which `key` has one value.
    pub fn enumerate_attribute(&self, key: AttributeKey, range: Range<usize>, mut f: impl FnMut(Range<usize>, Attribute)) {
        let range = self.clamp(range);
        let mut pos = 0;
        let mut cur: Option<(usize, usize, Attribute)> = None;
        for r in &self.runs {
            let n = r.text.chars().count();
            let (s, e) = (pos.max(range.start), (pos + n).min(range.end));
            pos += n;
            if s >= e {
                continue;
            }
            let v = r.attrs.get(key);
            cur = match cur.take() {
                Some((cs, _, cv)) if cv == v => Some((cs, e, cv)),
                Some((cs, ce, cv)) => {
                    f(cs..ce, cv);
                    Some((s, e, v))
                }
                None => Some((s, e, v)),
            };
        }
        if let Some((cs, ce, cv)) = cur {
            f(cs..ce, cv);
        }
    }
    /// True when the whole non-empty `range` has `value` for `key` (used for toggles).
    pub fn all(&self, range: Range<usize>, key: AttributeKey, value: &Attribute) -> bool {
        let mut ok = true;
        let mut any = false;
        self.enumerate_attribute(key, range, |_, v| {
            any = true;
            ok &= &v == value;
        });
        ok && any
    }
    /// Runs as `(text, attrs)` for drawing.
    pub fn runs(&self) -> impl Iterator<Item = (&str, &Attrs)> {
        self.runs.iter().map(|r| (r.text.as_str(), &r.attrs))
    }
    /// The runs for accessibility: each with its UTF-16 start and length. Runs are merged on every edit, so an
    /// attribute change always starts a new run.
    pub fn ax_runs(&self) -> Vec<AxAttrRun> {
        let mut out = Vec::with_capacity(self.runs.len());
        let mut at = 0usize;
        for r in &self.runs {
            let n = r.text.encode_utf16().count();
            if n == 0 {
                continue;
            }
            out.push(AxAttrRun { utf16_start: at, utf16_len: n, attrs: r.attrs.clone() });
            at += n;
        }
        out
    }

    // ---- RTF (basic subset: \b \i \ul \strike \fs, colour table, hyperlink fields, \par, \' and \u escapes) ----

    pub fn to_rtf(&self) -> String {
        let mut out = String::from("{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 SF Pro;}}{\\colortbl;");
        for t in ColorToken::ALL {
            let (r, g, b) = t.rgb();
            out.push_str(&format!("\\red{r}\\green{g}\\blue{b};"));
        }
        out.push_str("}\n");
        for (text, a) in self.runs() {
            let mut open = String::new();
            if a.bold {
                open.push_str("\\b");
            }
            if a.italic {
                open.push_str("\\i");
            }
            if a.underline {
                open.push_str("\\ul");
            }
            if a.strikethrough {
                open.push_str("\\strike");
            }
            open.push_str(&format!("\\fs{}", (a.size.pt() * 2.0).round() as i32));
            if let Some(c) = a.color {
                let i = ColorToken::ALL.iter().position(|t| *t == c).unwrap_or(0) + 1;
                open.push_str(&format!("\\cf{i}"));
            }
            if let Some(url) = &a.link {
                out.push_str(&format!("{{\\field{{\\*\\fldinst HYPERLINK \"{}\"}}{{\\fldrslt ", rtf_escape(url)));
            }
            out.push_str(&format!("{{{open} {}}}", rtf_escape(text)));
            if a.link.is_some() {
                out.push_str("}}");
            }
        }
        out.push('}');
        out
    }

    pub fn from_rtf(rtf: &str) -> AttributedString {
        let b: Vec<char> = rtf.chars().collect();
        let mut colors: Vec<(u8, u8, u8)> = Vec::new();
        // (attrs, skip, link) per group level
        let mut stack: Vec<(Attrs, bool, Option<String>)> = Vec::new();
        let (mut cur, mut skip, mut link): (Attrs, bool, Option<String>) = (Attrs::default(), false, None);
        let mut out = AttributedString::new();
        let mut ctbl_depth: Option<usize> = None;
        let mut fld_depth: Option<usize> = None;
        let mut fldinst = String::new();
        let (mut cr, mut cg, mut cb) = (0u8, 0u8, 0u8);
        let mut i = 0;
        while i < b.len() {
            let ch = b[i];
            match ch {
                '{' => {
                    stack.push((cur.clone(), skip, link.clone()));
                    i += 1;
                    let rest: String = b[i..(i + 12).min(b.len())].iter().collect();
                    if rest.starts_with("\\fonttbl") {
                        skip = true;
                    } else if rest.starts_with("\\colortbl") {
                        ctbl_depth = Some(stack.len());
                    } else if rest.starts_with("\\*\\fldinst") {
                        fld_depth = Some(stack.len());
                        fldinst.clear();
                        i += "\\*\\fldinst".len();
                    } else if rest.starts_with("\\*") {
                        skip = true;
                    }
                }
                '}' => {
                    if fld_depth == Some(stack.len()) {
                        fld_depth = None;
                        if let Some(p) = fldinst.find('"') {
                            let url = fldinst[p + 1..].split('"').next().unwrap_or("");
                            // the group state restored below is the {\field ...} level; the link applies to fldrslt
                            link = Some(url.to_string());
                            if let Some(top) = stack.last_mut() {
                                top.2 = link.clone();
                            }
                        }
                    }
                    if ctbl_depth == Some(stack.len()) {
                        ctbl_depth = None;
                    }
                    if let Some(s) = stack.pop() {
                        (cur, skip, link) = s;
                    }
                    i += 1;
                }
                '\\' => {
                    i += 1;
                    if i >= b.len() {
                        break;
                    }
                    let c = b[i];
                    if c == '\'' {
                        let h: String = b[(i + 1).min(b.len())..(i + 3).min(b.len())].iter().collect();
                        i += 3;
                        if let Ok(v) = u8::from_str_radix(&h, 16) {
                            put(&mut out, &cur, &link, skip, fld_depth.is_some(), &mut fldinst, &(v as char).to_string());
                        }
                    } else if c.is_ascii_alphabetic() {
                        let s = i;
                        while i < b.len() && b[i].is_ascii_alphabetic() {
                            i += 1;
                        }
                        let word: String = b[s..i].iter().collect();
                        let ns = i;
                        if i < b.len() && b[i] == '-' {
                            i += 1;
                        }
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                        let num: Option<i32> = b[ns..i].iter().collect::<String>().parse().ok();
                        if i < b.len() && b[i] == ' ' {
                            i += 1;
                        }
                        let on = num != Some(0);
                        let fld = fld_depth.is_some();
                        match word.as_str() {
                            "b" => cur.bold = on,
                            "i" => cur.italic = on,
                            "ul" => cur.underline = on,
                            "ulnone" => cur.underline = false,
                            "strike" => cur.strikethrough = on,
                            "plain" => cur = Attrs::default(),
                            "fs" => {
                                if let Some(n) = num {
                                    cur.size = SizeClass::nearest(n as f32 / 2.0);
                                }
                            }
                            "red" => cr = num.unwrap_or(0).clamp(0, 255) as u8,
                            "green" => cg = num.unwrap_or(0).clamp(0, 255) as u8,
                            "blue" => cb = num.unwrap_or(0).clamp(0, 255) as u8,
                            "cf" => {
                                let n = num.unwrap_or(0);
                                cur.color = if n >= 1 {
                                    colors.get(n as usize).map(|c| ColorToken::nearest_rgb(c.0, c.1, c.2))
                                } else {
                                    None
                                };
                            }
                            "par" | "line" => put(&mut out, &cur, &link, skip, fld, &mut fldinst, "\n"),
                            "tab" => put(&mut out, &cur, &link, skip, fld, &mut fldinst, "\t"),
                            "u" => {
                                if let Some(n) = num {
                                    let cp = if n < 0 { (n + 65536) as u32 } else { n as u32 };
                                    // a surrogate pair arrives as two \u items; pair them up
                                    if (0xD800..0xDC00).contains(&cp) {
                                        let lo = pending_low_surrogate(&b, i);
                                        if let Some((lo, ni)) = lo {
                                            let full = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                            if let Some(ch) = char::from_u32(full) {
                                                put(&mut out, &cur, &link, skip, fld, &mut fldinst, &ch.to_string());
                                            }
                                            i = ni;
                                            continue;
                                        }
                                    } else if let Some(ch) = char::from_u32(cp) {
                                        put(&mut out, &cur, &link, skip, fld, &mut fldinst, &ch.to_string());
                                    }
                                    if i < b.len() && !matches!(b[i], '\\' | '{' | '}') {
                                        i += 1; // the ANSI fallback character
                                    }
                                }
                            }
                            _ => {}
                        }
                    } else {
                        i += 1;
                        if matches!(c, '\\' | '{' | '}') {
                            put(&mut out, &cur, &link, skip, fld_depth.is_some(), &mut fldinst, &c.to_string());
                        }
                    }
                }
                ';' if ctbl_depth.is_some() => {
                    colors.push((cr, cg, cb));
                    (cr, cg, cb) = (0, 0, 0);
                    i += 1;
                }
                '\r' | '\n' => i += 1,
                ch => {
                    if ctbl_depth.is_none() {
                        put(&mut out, &cur, &link, skip, fld_depth.is_some(), &mut fldinst, &ch.to_string());
                    }
                    i += 1;
                }
            }
        }
        out
    }

    // ---- HTML (basic subset: b strong i em u s del strike a span[data-color] h1-h3 small br p) ----

    pub fn to_html(&self) -> String {
        let mut out = String::new();
        for (text, a) in self.runs() {
            let mut close: Vec<&str> = Vec::new();
            let mut open = String::new();
            let head = match a.size {
                SizeClass::LargeTitle | SizeClass::Title1 => Some("h1"),
                SizeClass::Title2 => Some("h2"),
                SizeClass::Title3 => Some("h3"),
                SizeClass::Footnote | SizeClass::Caption => Some("small"),
                _ => None,
            };
            if let Some(h) = head {
                open.push_str(&format!("<{h}>"));
                close.push(h);
            }
            if let Some(url) = &a.link {
                open.push_str(&format!("<a href=\"{}\">", html_escape(url)));
                close.push("a");
            }
            if let Some(c) = a.color {
                open.push_str(&format!("<span data-color=\"{}\">", c.name()));
                close.push("span");
            }
            if a.bold {
                open.push_str("<b>");
                close.push("b");
            }
            if a.italic {
                open.push_str("<i>");
                close.push("i");
            }
            if a.underline {
                open.push_str("<u>");
                close.push("u");
            }
            if a.strikethrough {
                open.push_str("<s>");
                close.push("s");
            }
            out.push_str(&open);
            out.push_str(&html_escape(text).replace('\n', "<br>"));
            for t in close.iter().rev() {
                out.push_str(&format!("</{t}>"));
            }
        }
        out
    }

    pub fn from_html(html: &str) -> AttributedString {
        let mut out = AttributedString::new();
        let mut stack: Vec<(String, Attrs)> = Vec::new();
        let mut cur = Attrs::default();
        let mut rest = html;
        while !rest.is_empty() {
            if let Some(stripped) = rest.strip_prefix('<') {
                let end = stripped.find('>').unwrap_or(stripped.len());
                let tag = &stripped[..end];
                rest = stripped.get(end + 1..).unwrap_or("");
                let closing = tag.starts_with('/');
                let body = tag.trim_start_matches('/').trim_end_matches('/');
                let name = body.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                if closing {
                    if let Some(p) = stack.iter().rposition(|(n, _)| *n == name) {
                        cur = stack[p].1.clone();
                        stack.truncate(p);
                    }
                    if matches!(name.as_str(), "p" | "h1" | "h2" | "h3") {
                        out.append("\n", cur.clone());
                    }
                    continue;
                }
                let prev = cur.clone();
                let mut known = true;
                match name.as_str() {
                    "b" | "strong" => cur.bold = true,
                    "i" | "em" => cur.italic = true,
                    "u" => cur.underline = true,
                    "s" | "del" | "strike" => cur.strikethrough = true,
                    "small" => cur.size = SizeClass::Footnote,
                    "h1" => {
                        cur.size = SizeClass::Title1;
                        cur.bold = true;
                    }
                    "h2" => {
                        cur.size = SizeClass::Title2;
                        cur.bold = true;
                    }
                    "h3" => {
                        cur.size = SizeClass::Title3;
                        cur.bold = true;
                    }
                    "a" => cur.link = html_attr(body, "href").map(|s| html_unescape(&s)),
                    "span" => cur.color = html_attr(body, "data-color").and_then(|c| ColorToken::from_name(&c)),
                    "br" => {
                        out.append("\n", cur.clone());
                        known = false;
                    }
                    "p" => {}
                    _ => known = false,
                }
                if known {
                    stack.push((name, prev));
                }
            } else {
                let end = rest.find('<').unwrap_or(rest.len());
                let t = html_unescape(&rest[..end]);
                if !t.is_empty() {
                    out.append(&t, cur.clone());
                }
                rest = &rest[end..];
            }
        }
        out
    }
}

/// After a high surrogate `\uN?`, reads the following `\uM?` low surrogate. Returns it and the index after it.
fn pending_low_surrogate(b: &[char], mut i: usize) -> Option<(u32, usize)> {
    if i < b.len() && !matches!(b[i], '\\' | '{' | '}') {
        i += 1; // fallback char of the high surrogate
    }
    if !(i + 1 < b.len() && b[i] == '\\' && b[i + 1] == 'u') {
        return None;
    }
    let mut j = i + 2;
    let s = j;
    if j < b.len() && b[j] == '-' {
        j += 1;
    }
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    let n: i32 = b[s..j].iter().collect::<String>().parse().ok()?;
    let cp = if n < 0 { (n + 65536) as u32 } else { n as u32 };
    if !(0xDC00..0xE000).contains(&cp) {
        return None;
    }
    if j < b.len() && !matches!(b[j], '\\' | '{' | '}') {
        j += 1;
    }
    Some((cp, j))
}

fn put(out: &mut AttributedString, a: &Attrs, link: &Option<String>, skip: bool, in_fld: bool, fld: &mut String, s: &str) {
    if in_fld {
        fld.push_str(s);
    } else if !skip {
        let mut a = a.clone();
        if let Some(l) = link {
            a.link = Some(l.clone());
            a.underline = true;
        }
        out.append(s, a);
    }
}

fn rtf_escape(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '\\' | '{' | '}' => {
                o.push('\\');
                o.push(c);
            }
            '\n' => o.push_str("\\par "),
            '\t' => o.push_str("\\tab "),
            c if (c as u32) < 128 => o.push(c),
            c => {
                let mut u = [0u16; 2];
                for unit in c.encode_utf16(&mut u) {
                    o.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
    o
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn html_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&nbsp;", " ").replace("&amp;", "&")
}

fn html_attr(tag: &str, name: &str) -> Option<String> {
    let p = tag.find(&format!("{name}=\""))? + name.len() + 2;
    let e = tag[p..].find('"')?;
    Some(tag[p..p + e].to_string())
}

// ---- rich text editing ----

/// Editing state of a rich text view: content, selection and the attributes typed next.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct RichText {
    pub content: AttributedString,
    /// Selection in characters; `start == end` is a caret.
    pub selection: Range<usize>,
    /// Attributes for the next typed character when the selection is a caret (`typingAttributes`).
    pub typing: Attrs,
}

/// The traits `Cmd+B/I/U` toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trait {
    Bold,
    Italic,
    Underline,
}

/// `NativeTextView` with `isRichText = true`: every edit registers with the view's [`UndoManager`].
pub struct RichTextView {
    pub text: RichText,
    pub undo: UndoManager<RichText>,
}

impl RichTextView {
    pub fn new(content: AttributedString) -> Self {
        let end = content.len();
        RichTextView { text: RichText { content, selection: end..end, typing: Attrs::default() }, undo: UndoManager::new(100) }
    }
    fn edit(&mut self, name: &str, f: impl FnOnce(&mut RichText)) {
        let before = self.text.clone();
        f(&mut self.text);
        let after = self.text.clone();
        if before == after {
            return;
        }
        self.undo.register(name, move |t| *t = before.clone(), move |t| *t = after.clone());
    }
    /// Moves the selection; a caret picks up the attributes of the character before it as typing attributes.
    pub fn set_selection(&mut self, range: Range<usize>) {
        let n = self.text.content.len();
        let s = range.start.min(n);
        self.text.selection = s..range.end.clamp(s, n);
        if n > 0 {
            self.text.typing = self.text.content.attributes_at(s.saturating_sub(1));
        }
    }
    /// Types `s` over the selection with the typing attributes (`insertText`).
    pub fn insert_text(&mut self, s: &str) {
        self.edit("Typing", |t| {
            let r = t.selection.clone();
            let ins = AttributedString::with_attrs(s, t.typing.clone());
            t.content.replace_attributed(r.clone(), &ins);
            let c = r.start + s.chars().count();
            t.selection = c..c;
        });
    }
    /// Backspace: deletes the selection, or the character before the caret.
    pub fn delete_backward(&mut self) {
        self.edit("Delete", |t| {
            let mut r = t.selection.clone();
            if r.is_empty() && r.start > 0 {
                r = r.start - 1..r.start;
            }
            t.content.replace_attributed(r.clone(), &AttributedString::new());
            t.selection = r.start..r.start;
        });
    }
    /// Toggles a trait over the selection (all on turns it off, otherwise on), or on the typing attributes at a caret.
    pub fn toggle(&mut self, tr: Trait) {
        let (name, key, mk): (&str, AttributeKey, fn(bool) -> Attribute) = match tr {
            Trait::Bold => ("Bold", AttributeKey::Bold, Attribute::Bold),
            Trait::Italic => ("Italic", AttributeKey::Italic, Attribute::Italic),
            Trait::Underline => ("Underline", AttributeKey::Underline, Attribute::Underline),
        };
        self.edit(name, |t| {
            if t.selection.is_empty() {
                let on = t.typing.get(key) != mk(true);
                t.typing.set(&mk(on));
            } else {
                let on = !t.content.all(t.selection.clone(), key, &mk(true));
                t.content.add_attribute(mk(on), t.selection.clone());
            }
        });
    }
    /// Command key handling: `Cmd+B`, `Cmd+I`, `Cmd+U`, `Cmd+Z`, `Cmd+Shift+Z`. Returns true when handled.
    pub fn key_command(&mut self, ch: char, shift: bool) -> bool {
        match ch.to_ascii_lowercase() {
            'b' => self.toggle(Trait::Bold),
            'i' => self.toggle(Trait::Italic),
            'u' => self.toggle(Trait::Underline),
            'z' if shift => {
                self.undo.redo(&mut self.text);
            }
            'z' => {
                self.undo.undo(&mut self.text);
            }
            _ => return false,
        }
        true
    }
    /// Lays the content out in `width` px with the typesetter (paragraph styles, tabs, kerning, baseline and the other
    /// attributes apply). The layout answers caret, selection and hit-testing queries in pixels.
    pub fn layout(&self, ui: &ui::Ui, width: i32) -> crate::richlayout::TextLayout {
        crate::richlayout::TextLayout::new(&self.text.content, ui, width as f32)
    }
    /// Draws the content wrapped to `width` px at (`x`, `y`) with the selection highlighted and, when `focused`, the
    /// caret; returns the height used. Bold uses `Weight::Bold`; links, underline and strikethrough are lines in the
    /// run colour. There is no italic face in the system font, so italic lives in the model and in RTF/HTML export
    /// but is drawn upright (documented gap).
    pub fn draw(&self, ui: &ui::Ui, s: &mut bifrost_render::gfx::Surface, x: i32, y: i32, width: i32, focused: bool) -> i32 {
        let layout = self.layout(ui, width);
        let sel = self.text.selection.clone();
        let caret = (focused && sel.is_empty()).then_some(sel.start);
        layout.draw(ui, s, x, y, Some(sel), caret);
        layout.height_px()
    }
    /// The character index under the pixel (`px`, `py`), measured from the content origin (`mouseDown` hit-testing).
    pub fn index_at_point(&self, ui: &ui::Ui, width: i32, px: i32, py: i32) -> usize {
        self.layout(ui, width).index_at_px(px as f32, py as f32)
    }
    /// Selects from `anchor` to the character under the pixel, as a drag does.
    pub fn drag_selection(&mut self, ui: &ui::Ui, width: i32, anchor: usize, px: i32, py: i32) {
        let at = self.index_at_point(ui, width, px, py);
        self.set_selection(anchor.min(at)..anchor.max(at));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold() -> Attrs {
        Attrs { bold: true, ..Default::default() }
    }

    #[test]
    fn add_attribute_splits_and_merges_runs() {
        let mut s = AttributedString::plain("hello world");
        s.add_attribute(Attribute::Bold(true), 0..5);
        assert_eq!(s.runs().count(), 2);
        assert_eq!(s.attributes_at(2), bold());
        s.add_attribute(Attribute::Bold(true), 5..11);
        assert_eq!(s.runs().count(), 1, "equal neighbours merge");
        s.remove_attribute(AttributeKey::Bold, 0..11);
        assert_eq!(s, AttributedString::plain("hello world"));
    }

    #[test]
    fn replace_characters_inherits_attributes() {
        let mut s = AttributedString::with_attrs("bold", bold());
        s.append(" plain", Attrs::default());
        s.replace_characters(1..3, "OO");
        assert_eq!(s.string(), "bOOd plain");
        assert_eq!(s.attributes_at(1), bold());
        assert!(!s.attributes_at(8).bold);
        assert_eq!(s.substring(0..4).string(), "bOOd");
    }

    #[test]
    fn enumerate_attribute_reports_maximal_ranges() {
        let mut s = AttributedString::plain("abcdef");
        s.add_attribute(Attribute::Underline(true), 2..4);
        let mut seen = Vec::new();
        s.enumerate_attribute(AttributeKey::Underline, 0..6, |r, v| seen.push((r, v)));
        assert_eq!(
            seen,
            vec![(0..2, Attribute::Underline(false)), (2..4, Attribute::Underline(true)), (4..6, Attribute::Underline(false))]
        );
        let mut sub = Vec::new();
        s.enumerate_attribute(AttributeKey::Underline, 3..5, |r, v| sub.push((r, v)));
        assert_eq!(sub, vec![(3..4, Attribute::Underline(true)), (4..5, Attribute::Underline(false))]);
    }

    #[test]
    fn rtf_round_trip() {
        let mut s = AttributedString::plain("a {b} \\c\nd \u{e9}x");
        s.add_attribute(Attribute::Bold(true), 0..3);
        s.add_attribute(Attribute::Italic(true), 2..5);
        s.add_attribute(Attribute::Strikethrough(true), 8..9);
        s.add_attribute(Attribute::Size(SizeClass::Title2), 9..10);
        s.add_attribute(Attribute::Color(Some(ColorToken::Red)), 10..11);
        let back = AttributedString::from_rtf(&s.to_rtf());
        assert_eq!(back.string(), s.string());
        assert_eq!(back, s);
    }

    #[test]
    fn rtf_link_round_trip() {
        let mut s = AttributedString::plain("see site now");
        s.add_attribute(Attribute::Link(Some("https://example.com/".into())), 4..8);
        let back = AttributedString::from_rtf(&s.to_rtf());
        assert_eq!(back.string(), "see site now");
        assert_eq!(back.attributes_at(5).link.as_deref(), Some("https://example.com/"));
        assert_eq!(back.attributes_at(0).link, None);
    }

    #[test]
    fn html_round_trip() {
        let mut s = AttributedString::plain("a<b & c\nline");
        s.add_attribute(Attribute::Bold(true), 0..3);
        s.add_attribute(Attribute::Underline(true), 2..6);
        s.add_attribute(Attribute::Link(Some("https://x.test/?a=1&b=2".into())), 7..8);
        let html = s.to_html();
        assert!(html.contains("&lt;") && html.contains("<br>"));
        assert_eq!(AttributedString::from_html(&html), s);
    }

    #[test]
    fn html_import_reads_common_tags() {
        let s = AttributedString::from_html("<p>Hi <strong>there</strong> <em>you</em></p>");
        assert_eq!(s.string(), "Hi there you\n");
        assert!(s.attributes_at(3).bold && s.attributes_at(9).italic && !s.attributes_at(0).bold);
    }

    #[test]
    fn size_classes_use_the_mac_styles() {
        assert_eq!(SizeClass::Body.pt(), ui::MAC_BODY_PT);
        assert_eq!(SizeClass::nearest(26.0), SizeClass::LargeTitle);
        assert_eq!(SizeClass::nearest(13.0), SizeClass::Body);
    }

    #[test]
    fn cmd_b_toggles_selection_and_undoes() {
        let mut v = RichTextView::new(AttributedString::plain("hello"));
        v.set_selection(0..5);
        assert!(v.key_command('b', false));
        assert!(v.text.content.attributes_at(0).bold);
        v.key_command('b', false);
        assert!(!v.text.content.attributes_at(0).bold, "second press clears");
        v.key_command('z', false);
        assert!(v.text.content.attributes_at(0).bold, "undo restores bold");
        v.key_command('z', true);
        assert!(!v.text.content.attributes_at(0).bold, "redo");
        assert!(!v.key_command('q', false));
    }

    #[test]
    fn typing_uses_typing_attributes_and_undoes() {
        let mut v = RichTextView::new(AttributedString::plain("ab"));
        v.toggle(Trait::Underline);
        v.insert_text("c");
        assert_eq!(v.text.content.string(), "abc");
        assert!(v.text.content.attributes_at(2).underline && !v.text.content.attributes_at(0).underline);
        v.delete_backward();
        assert_eq!(v.text.content.string(), "ab");
        v.key_command('z', false);
        assert_eq!(v.text.content.string(), "abc");
        v.key_command('z', false);
        v.key_command('z', false);
        assert_eq!(v.text.content.string(), "ab");
        assert!(!v.undo.can_undo());
    }

    #[test]
    fn ax_runs_of_a_plain_string_is_one_run() {
        let s = AttributedString::plain("abc");
        assert_eq!(s.ax_runs(), vec![AxAttrRun { utf16_start: 0, utf16_len: 3, attrs: Attrs::default() }]);
    }

    #[test]
    fn ax_runs_count_utf16_units_and_keep_attributes() {
        // "a" (1 unit) is bold, the emoji (2 units) is plain, "b" (1 unit) is a link
        let mut s = AttributedString::with_attrs("a", bold());
        s.append("\u{1F600}", Attrs::default());
        let link = Attrs { link: Some("https://example.org/".to_string()), ..Default::default() };
        s.append("b", link.clone());
        let runs = s.ax_runs();
        assert_eq!(runs.len(), 3);
        assert_eq!((runs[0].utf16_start, runs[0].utf16_len), (0, 1));
        assert!(runs[0].attrs.bold);
        assert_eq!((runs[1].utf16_start, runs[1].utf16_len), (1, 2));
        assert_eq!((runs[2].utf16_start, runs[2].utf16_len), (3, 1));
        assert_eq!(runs[2].attrs, link);
    }

    #[test]
    fn ax_runs_merge_equal_neighbours_and_skip_empty_text() {
        let mut s = AttributedString::plain("ab");
        s.append("c", Attrs::default());
        assert_eq!(s.ax_runs().len(), 1);
        assert_eq!(s.ax_runs()[0].utf16_len, 3);
        assert!(AttributedString::new().ax_runs().is_empty());
    }
}
