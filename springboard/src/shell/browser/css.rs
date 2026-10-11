//! A CSS subset: rule parsing (type, class, id and universal selectors with
//! descendant and child combinators; `@media` width queries), the cascade
//! (specificity, source order, `!important`, the `style` attribute) and the
//! handful of properties a text-first layout needs. Anything else is
//! skipped, never an error.

use super::html::{Element, Node};
use super::style::parse::{split_important, MediaEnv, MediaQueryList};

/// The CSS pixel width `@media` width queries are answered for.
pub const MEDIA_WIDTH: f32 = 800.0;
/// The CSS pixel height `@media` height queries are answered for.
pub const MEDIA_HEIGHT: f32 = 1280.0;
const ROOT_PX: f32 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    /// Opaque pixel for the framebuffer; translucent colours are blended over white.
    pub fn pack(self) -> u32 {
        let mix = |c: u8| ((c as u32 * self.a as u32 + 255 * (255 - self.a as u32) + 127) / 255) as u8;
        crate::gfx::pack(mix(self.r), mix(self.g), mix(self.b), 255)
    }
}

const NAMED_COLORS: &[(&str, Color)] = &[
    ("black", Color::rgb(0, 0, 0)),
    ("white", Color::rgb(255, 255, 255)),
    ("red", Color::rgb(255, 0, 0)),
    ("green", Color::rgb(0, 128, 0)),
    ("blue", Color::rgb(0, 0, 255)),
    ("gray", Color::rgb(128, 128, 128)),
    ("grey", Color::rgb(128, 128, 128)),
    ("silver", Color::rgb(192, 192, 192)),
    ("maroon", Color::rgb(128, 0, 0)),
    ("purple", Color::rgb(128, 0, 128)),
    ("fuchsia", Color::rgb(255, 0, 255)),
    ("lime", Color::rgb(0, 255, 0)),
    ("olive", Color::rgb(128, 128, 0)),
    ("yellow", Color::rgb(255, 255, 0)),
    ("navy", Color::rgb(0, 0, 128)),
    ("teal", Color::rgb(0, 128, 128)),
    ("aqua", Color::rgb(0, 255, 255)),
    ("orange", Color::rgb(255, 165, 0)),
    ("brown", Color::rgb(165, 42, 42)),
    ("pink", Color::rgb(255, 192, 203)),
    ("gold", Color::rgb(255, 215, 0)),
    ("darkgray", Color::rgb(169, 169, 169)),
    ("darkgrey", Color::rgb(169, 169, 169)),
    ("lightgray", Color::rgb(211, 211, 211)),
    ("lightgrey", Color::rgb(211, 211, 211)),
    ("whitesmoke", Color::rgb(245, 245, 245)),
    ("darkblue", Color::rgb(0, 0, 139)),
    ("darkred", Color::rgb(139, 0, 0)),
    ("darkgreen", Color::rgb(0, 100, 0)),
    ("crimson", Color::rgb(220, 20, 60)),
    ("transparent", Color { r: 0, g: 0, b: 0, a: 0 }),
];

pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        if !hex.is_ascii() {
            return None;
        }
        let digit = |i: usize, n: usize| -> Option<u8> {
            let v = u8::from_str_radix(hex.get(i..i + n)?, 16).ok()?;
            Some(if n == 1 { v * 17 } else { v })
        };
        return match hex.len() {
            3 => Some(Color::rgb(digit(0, 1)?, digit(1, 1)?, digit(2, 1)?)),
            4 => Some(Color { r: digit(0, 1)?, g: digit(1, 1)?, b: digit(2, 1)?, a: digit(3, 1)? }),
            6 => Some(Color::rgb(digit(0, 2)?, digit(2, 2)?, digit(4, 2)?)),
            8 => Some(Color { r: digit(0, 2)?, g: digit(2, 2)?, b: digit(4, 2)?, a: digit(6, 2)? }),
            _ => None,
        };
    }
    if let Some(args) = s.strip_prefix("rgba(").or_else(|| s.strip_prefix("rgb(")) {
        let args = args.strip_suffix(')')?;
        let parts: Vec<&str> = args.split(|c: char| c == ',' || c == '/' || c.is_ascii_whitespace()).filter(|p| !p.is_empty()).collect();
        if parts.len() < 3 {
            return None;
        }
        let chan = |p: &str| -> Option<u8> {
            match p.strip_suffix('%') {
                Some(n) => Some((n.parse::<f32>().ok()? * 2.55).round().clamp(0.0, 255.0) as u8),
                None => Some(p.parse::<f32>().ok()?.round().clamp(0.0, 255.0) as u8),
            }
        };
        let a = match parts.get(3) {
            Some(p) => match p.strip_suffix('%') {
                Some(n) => (n.parse::<f32>().ok()? * 2.55).round().clamp(0.0, 255.0) as u8,
                None => (p.parse::<f32>().ok()? * 255.0).round().clamp(0.0, 255.0) as u8,
            },
            None => 255,
        };
        return Some(Color { r: chan(parts[0])?, g: chan(parts[1])?, b: chan(parts[2])?, a });
    }
    NAMED_COLORS.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Len {
    Auto,
    Px(f32),
    Pct(f32),
}

impl Len {
    /// Pixels against a containing size; `auto` is 0.
    pub fn resolve(self, of: f32) -> f32 {
        match self {
            Len::Auto => 0.0,
            Len::Px(v) => v,
            Len::Pct(p) => of * p / 100.0,
        }
    }
}

/// A length token; `em` is the font size `em` units refer to.
pub fn parse_len(s: &str, em: f32) -> Option<Len> {
    let s = s.trim().to_ascii_lowercase();
    if s == "auto" {
        return Some(Len::Auto);
    }
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+')).unwrap_or(s.len());
    let n: f32 = s[..split].parse().ok()?;
    match &s[split..] {
        "" | "px" => Some(Len::Px(n)),
        "pt" => Some(Len::Px(n * 4.0 / 3.0)),
        "em" => Some(Len::Px(n * em)),
        "ex" => Some(Len::Px(n * em / 2.0)),
        "rem" => Some(Len::Px(n * ROOT_PX)),
        "%" => Some(Len::Pct(n)),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Display {
    None,
    Block,
    Inline,
    ListItem,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WhiteSpace {
    Normal,
    Pre,
    PreWrap,
    NoWrap,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ListStyle {
    Disc,
    Decimal,
    None,
}

/// (width px, colour)
pub type Border = Option<(f32, Color)>;

#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub display: Display,
    pub color: Color,
    pub background: Option<Color>,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub align: Align,
    pub white_space: WhiteSpace,
    pub list_style: ListStyle,
    /// multiple of the font size
    pub line_height: f32,
    /// top, right, bottom, left
    pub margin: [Len; 4],
    pub padding: [Len; 4],
    pub border: [Border; 4],
    pub width: Len,
    pub max_width: Len,
}

impl Style {
    pub fn root() -> Style {
        Style {
            display: Display::Inline,
            color: Color::BLACK,
            background: None,
            font_size: ROOT_PX,
            bold: false,
            italic: false,
            underline: false,
            align: Align::Left,
            white_space: WhiteSpace::Normal,
            list_style: ListStyle::Disc,
            line_height: 1.3,
            margin: [Len::Px(0.0); 4],
            padding: [Len::Px(0.0); 4],
            border: [None; 4],
            width: Len::Auto,
            max_width: Len::Auto,
        }
    }

    /// A child's starting style: inherited properties kept, the rest reset.
    fn inherit(&self) -> Style {
        Style { background: None, margin: [Len::Px(0.0); 4], padding: [Len::Px(0.0); 4], border: [None; 4], width: Len::Auto, max_width: Len::Auto, display: Display::Inline, ..self.clone() }
    }

    /// The style of a text node inside an element styled `self`.
    fn for_text(&self) -> Style {
        let mut s = self.inherit();
        s.background = if self.display == Display::Inline { self.background } else { None };
        s
    }
}

// --- declarations -----------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Decl {
    pub name: String,
    pub value: String,
    pub important: bool,
}

/// Splits at `sep` outside parentheses and quotes.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let (mut out, mut depth, mut quote, mut start) = (vec![], 0i32, None::<char>, 0);
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '(') => depth += 1,
            (None, ')') => depth -= 1,
            (None, c) if c == sep && depth <= 0 => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Splits a value at whitespace outside parentheses (`rgb(1, 2, 3) 4px` is two tokens).
fn tokens(s: &str) -> Vec<&str> {
    let (mut out, mut depth, mut start) = (vec![], 0i32, None::<usize>);
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if c.is_ascii_whitespace() && depth <= 0 {
            if let Some(st) = start.take() {
                out.push(&s[st..i]);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(&s[st..]);
    }
    out
}

pub fn parse_decls(body: &str) -> Vec<Decl> {
    split_top(body, ';')
        .into_iter()
        .filter_map(|d| {
            let (name, value) = d.split_once(':')?;
            let name = name.trim().to_ascii_lowercase();
            let (value, important) = split_important(value.trim());
            let lower = value.to_ascii_lowercase();
            if name.is_empty() || value.is_empty() || ["var(", "calc(", "env(", "min(", "max(", "clamp("].iter().any(|f| lower.contains(f)) {
                return None;
            }
            Some(Decl { name, value: value.to_string(), important })
        })
        .collect()
}

fn font_size(v: &str, parent: f32) -> Option<f32> {
    let size = match v.trim().to_ascii_lowercase().as_str() {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "smaller" => parent / 1.2,
        "larger" => parent * 1.2,
        other => match parse_len(other, parent)? {
            Len::Px(p) => p,
            Len::Pct(p) => parent * p / 100.0,
            Len::Auto => return None,
        },
    };
    Some(size.clamp(6.0, 120.0))
}

/// 1 to 4 box values to (top, right, bottom, left).
fn box_values(v: &str, em: f32) -> Option<[Len; 4]> {
    let l: Vec<Len> = tokens(v).into_iter().map(|t| parse_len(t, em)).collect::<Option<_>>()?;
    match l.as_slice() {
        [a] => Some([*a, *a, *a, *a]),
        [v, h] => Some([*v, *h, *v, *h]),
        [t, h, b] => Some([*t, *h, *b, *h]),
        [t, r, b, l] => Some([*t, *r, *b, *l]),
        _ => None,
    }
}

fn border_value(v: &str, em: f32, current: Color) -> Option<Border> {
    let (mut width, mut color, mut visible) = (None, None, true);
    for t in tokens(v) {
        let lower = t.to_ascii_lowercase();
        match lower.as_str() {
            "none" | "hidden" => visible = false,
            "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" => {}
            "thin" => width = Some(1.0),
            "medium" => width = Some(3.0),
            "thick" => width = Some(5.0),
            "currentcolor" => color = Some(current),
            _ => match parse_len(t, em) {
                Some(Len::Px(p)) => width = Some(p),
                _ => color = parse_color(t).or(color),
            },
        }
    }
    let w = width.unwrap_or(1.0);
    Some((visible && w > 0.0).then_some((w, color.unwrap_or(current))))
}

fn parse_display(v: &str) -> Display {
    match v.trim().to_ascii_lowercase().as_str() {
        "none" => Display::None,
        "inline" | "inline-block" | "inline-flex" | "inline-grid" | "table-cell" | "contents" => Display::Inline,
        "list-item" => Display::ListItem,
        _ => Display::Block,
    }
}

fn apply(st: &mut Style, parent: &Style, name: &str, value: &str) {
    let lower = value.to_ascii_lowercase();
    if lower == "inherit" || lower == "initial" || lower == "unset" {
        return;
    }
    let em = st.font_size;
    match name {
        "display" => st.display = parse_display(value),
        "color" => {
            if let Some(c) = parse_color(value) {
                st.color = c;
            } else if lower == "currentcolor" {
                st.color = parent.color;
            }
        }
        "background" | "background-color" => {
            let found = tokens(value).into_iter().find_map(parse_color);
            match found {
                Some(c) if c.a == 0 => st.background = None,
                Some(c) => st.background = Some(c),
                None if lower == "none" || lower == "transparent" => st.background = None,
                None => {}
            }
        }
        "font-weight" => {
            st.bold = match lower.as_str() {
                "bold" | "bolder" => true,
                "normal" | "lighter" => false,
                n => n.parse::<u32>().map_or(st.bold, |w| w >= 600),
            }
        }
        "font-style" => st.italic = lower == "italic" || lower == "oblique",
        "text-decoration" | "text-decoration-line" => {
            if lower.contains("underline") {
                st.underline = true;
            } else if lower.contains("none") {
                st.underline = false;
            }
        }
        "text-align" => {
            st.align = match lower.as_str() {
                "center" => Align::Center,
                "right" | "end" => Align::Right,
                _ => Align::Left,
            }
        }
        "white-space" => {
            st.white_space = match lower.as_str() {
                "pre" => WhiteSpace::Pre,
                "pre-wrap" | "pre-line" | "break-spaces" => WhiteSpace::PreWrap,
                "nowrap" => WhiteSpace::NoWrap,
                _ => WhiteSpace::Normal,
            }
        }
        "list-style" | "list-style-type" => {
            st.list_style = if lower.contains("none") {
                ListStyle::None
            } else if lower.contains("decimal") || lower.contains("roman") || lower.contains("alpha") {
                ListStyle::Decimal
            } else {
                ListStyle::Disc
            }
        }
        "line-height" => {
            if lower == "normal" {
                st.line_height = 1.3;
            } else if let Ok(n) = lower.parse::<f32>() {
                st.line_height = n.clamp(1.0, 3.0);
            } else {
                match parse_len(value, em) {
                    Some(Len::Px(p)) => st.line_height = (p / em).clamp(1.0, 3.0),
                    Some(Len::Pct(p)) => st.line_height = (p / 100.0).clamp(1.0, 3.0),
                    _ => {}
                }
            }
        }
        "margin" => {
            if let Some(b) = box_values(value, em) {
                st.margin = b;
            }
        }
        "padding" => {
            if let Some(b) = box_values(value, em) {
                st.padding = b.map(|l| if l == Len::Auto { Len::Px(0.0) } else { l });
            }
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top" | "padding-right" | "padding-bottom" | "padding-left" => {
            if let Some(l) = parse_len(value, em) {
                let side = match name.rsplit('-').next() {
                    Some("top") => 0,
                    Some("right") => 1,
                    Some("bottom") => 2,
                    _ => 3,
                };
                if name.starts_with("margin") {
                    st.margin[side] = l;
                } else {
                    st.padding[side] = if l == Len::Auto { Len::Px(0.0) } else { l };
                }
            }
        }
        "border" => {
            if let Some(b) = border_value(value, em, st.color) {
                st.border = [b; 4];
            }
        }
        "border-top" | "border-right" | "border-bottom" | "border-left" => {
            if let Some(b) = border_value(value, em, st.color) {
                let side = match name {
                    "border-top" => 0,
                    "border-right" => 1,
                    "border-bottom" => 2,
                    _ => 3,
                };
                st.border[side] = b;
            }
        }
        "width" => {
            if let Some(l) = parse_len(value, em) {
                st.width = l;
            }
        }
        "max-width" => {
            st.max_width = if lower == "none" { Len::Auto } else { parse_len(value, em).unwrap_or(Len::Auto) };
        }
        _ => {}
    }
}

// --- selectors and rules ----------------------------------------------------

#[derive(Clone, Debug, PartialEq, Default)]
struct Compound {
    tag: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

impl Compound {
    fn matches(&self, e: &Element) -> bool {
        self.tag.as_ref().is_none_or(|t| *t == e.tag) && self.id.as_deref().is_none_or(|i| e.attr("id") == Some(i)) && self.classes.iter().all(|c| e.has_class(c))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Comb {
    Descendant,
    Child,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    /// each compound with how it relates to the one before it
    parts: Vec<(Comb, Compound)>,
    pub specificity: u32,
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '-' || c == '_'
}

/// None for anything outside the supported subset (pseudo-classes, attribute
/// selectors, sibling combinators): such rules are dropped whole.
pub fn parse_selector(s: &str) -> Option<Selector> {
    let cs: Vec<char> = s.trim().chars().collect();
    let (mut parts, mut cur, mut has) = (Vec::new(), Compound::default(), false);
    let (mut comb, mut lead) = (Comb::Descendant, Comb::Descendant);
    let mut i = 0;
    let ident = |i: &mut usize| -> String {
        let st = *i;
        while *i < cs.len() && is_ident(cs[*i]) {
            *i += 1;
        }
        cs[st..*i].iter().collect()
    };
    while i < cs.len() {
        let c = cs[i];
        match c {
            c if c.is_whitespace() => {
                if has {
                    parts.push((lead, std::mem::take(&mut cur)));
                    has = false;
                }
                i += 1;
            }
            '>' => {
                if has {
                    parts.push((lead, std::mem::take(&mut cur)));
                    has = false;
                }
                comb = Comb::Child;
                i += 1;
            }
            '*' | '.' | '#' | 'a'..='z' | 'A'..='Z' | '_' => {
                if !has {
                    lead = comb;
                    comb = Comb::Descendant;
                    has = true;
                }
                match c {
                    '*' => i += 1,
                    '.' => {
                        i += 1;
                        let n = ident(&mut i);
                        if n.is_empty() {
                            return None;
                        }
                        cur.classes.push(n);
                    }
                    '#' => {
                        i += 1;
                        let n = ident(&mut i);
                        if n.is_empty() {
                            return None;
                        }
                        cur.id = Some(n);
                    }
                    _ => cur.tag = Some(ident(&mut i).to_ascii_lowercase()),
                }
            }
            _ => return None,
        }
    }
    if has {
        parts.push((lead, cur));
    }
    if parts.is_empty() {
        return None;
    }
    let specificity = parts.iter().map(|(_, c)| c.id.is_some() as u32 * 10_000 + c.classes.len() as u32 * 100 + c.tag.is_some() as u32).sum();
    Some(Selector { parts, specificity })
}

impl Selector {
    /// `chain` is the element's ancestors from the root down, ending with the element itself.
    pub fn matches(&self, chain: &[&Element]) -> bool {
        match_parts(&self.parts, chain)
    }
}

fn match_parts(parts: &[(Comb, Compound)], chain: &[&Element]) -> bool {
    let Some(((comb, last), rest)) = parts.split_last() else { return true };
    let Some((el, above)) = chain.split_last() else { return false };
    if !last.matches(el) {
        return false;
    }
    if rest.is_empty() {
        return true;
    }
    match comb {
        Comb::Child => match_parts(rest, above),
        Comb::Descendant => (1..=above.len()).any(|n| match_parts(rest, &above[..n])),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub decls: Vec<Decl>,
}

/// Index just past the string literal opening at `s[i]` (a quote), honouring backslash escapes.
fn skip_string(s: &[u8], i: usize) -> usize {
    let quote = s[i];
    let mut j = i + 1;
    while j < s.len() {
        match s[j] {
            b'\\' => j += 2,
            c if c == quote => return j + 1,
            _ => j += 1,
        }
    }
    s.len()
}

/// Index just past an unquoted `url(...)` starting at `s[i]`, or `None` when `s[i..]` is not one.
fn skip_bare_url(s: &[u8], i: usize) -> Option<usize> {
    let head = s.get(i..i + 4)?;
    if !head.eq_ignore_ascii_case(b"url(") {
        return None;
    }
    let inner = i + 4 + s[i + 4..].iter().take_while(|b| b.is_ascii_whitespace()).count();
    if matches!(s.get(inner), Some(b'"' | b'\'')) {
        return None;
    }
    Some(s[inner..].iter().position(|&b| b == b')').map_or(s.len(), |p| inner + p + 1))
}

fn strip_comments(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut copied) = (0, 0);
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => i = skip_string(bytes, i),
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                out.push_str(&s[copied..i]);
                i = s[i + 2..].find("*/").map_or(s.len(), |e| i + 2 + e + 2);
                copied = i;
            }
            _ => i = skip_bare_url(bytes, i).unwrap_or(i + 1),
        }
    }
    out.push_str(&s[copied.min(s.len())..]);
    out
}

/// Index of the `}` matching the `{` at `open`, or the end of `s`; braces inside strings and `url()` do not count.
fn matching_brace(s: &str, open: usize) -> usize {
    let bytes = s.as_bytes();
    let (mut depth, mut i) = (0, open);
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                i = skip_string(bytes, i);
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {
                if let Some(end) = skip_bare_url(bytes, i) {
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }
    s.len()
}

/// Whether an `@media` prelude (`@media ...`) holds at the fixed viewport.
fn media_applies(prelude: &str) -> bool {
    let query = prelude.trim_start().get("@media".len()..).unwrap_or("");
    MediaQueryList::parse(query).matches(&MediaEnv { width: MEDIA_WIDTH, height: MEDIA_HEIGHT, dark: false })
}

fn parse_rules(src: &str, rules: &mut Vec<Rule>) {
    let mut rest = src.trim_start();
    while !rest.is_empty() {
        if rest.starts_with('@') {
            let (semi, brace) = (rest.find(';'), rest.find('{'));
            match (semi, brace) {
                (Some(s), b) if b.is_none_or(|b| s < b) => rest = &rest[s + 1..],
                (_, Some(b)) => {
                    let end = matching_brace(rest, b);
                    if rest[..b].trim_start().to_ascii_lowercase().starts_with("@media") && media_applies(&rest[..b]) {
                        parse_rules(&rest[b + 1..end], rules);
                    }
                    rest = &rest[(end + 1).min(rest.len())..];
                }
                _ => break,
            }
        } else {
            let Some(b) = rest.find('{') else { break };
            let end = matching_brace(rest, b);
            let selectors: Vec<Selector> = split_top(&rest[..b], ',').into_iter().filter_map(parse_selector).collect();
            if !selectors.is_empty() {
                rules.push(Rule { selectors, decls: parse_decls(&rest[b + 1..end]) });
            }
            rest = &rest[(end + 1).min(rest.len())..];
        }
        rest = rest.trim_start();
    }
}

pub fn parse_stylesheet(src: &str) -> Vec<Rule> {
    let mut rules = vec![];
    parse_rules(&strip_comments(src), &mut rules);
    rules
}

/// The browser's own defaults, a stylesheet like any other.
pub const USER_AGENT: &str = "
html, body { display: block }
body { margin: 8px }
head, script, style, title, meta, link, base, noscript, template, svg, iframe, canvas, video, audio, object, embed, select, textarea, input, option, datalist, map, area, source, track, param, dialog { display: none }
div, section, article, aside, header, footer, nav, main, figure, figcaption, form, fieldset, details, summary, address, hgroup, menu, dir { display: block }
center { display: block; text-align: center }
p { display: block; margin: 1em 0 }
h1 { display: block; font-size: 2em; font-weight: bold; margin: .67em 0 }
h2 { display: block; font-size: 1.5em; font-weight: bold; margin: .83em 0 }
h3 { display: block; font-size: 1.17em; font-weight: bold; margin: 1em 0 }
h4 { display: block; font-weight: bold; margin: 1.33em 0 }
h5 { display: block; font-size: .83em; font-weight: bold; margin: 1.67em 0 }
h6 { display: block; font-size: .67em; font-weight: bold; margin: 2.33em 0 }
b, strong, th { font-weight: bold }
i, em, cite, var, dfn { font-style: italic }
u, ins { text-decoration: underline }
a { color: #0a58ca; text-decoration: underline }
small { font-size: .83em }
big { font-size: 1.17em }
sub, sup { font-size: .75em }
code, kbd, samp, tt { background: #eef0f2 }
mark { background: #fff3a3 }
pre { display: block; white-space: pre; margin: 1em 0; padding: 8px; background: #f3f4f6 }
blockquote { display: block; margin: 1em 0; padding: 0 0 0 16px; border-left: 3px solid #c8ccd0; color: #444 }
ul, ol { display: block; margin: 1em 0; padding: 0 0 0 32px }
ul ul, ol ol, ul ol, ol ul { margin: 0 }
li { display: list-item }
dl { display: block; margin: 1em 0 }
dt { display: block; font-weight: bold }
dd { display: block; margin: 0 0 0 32px }
hr { display: block; margin: 8px 0; border-top: 1px solid #c8ccd0 }
table, tr { display: block }
caption { display: block; text-align: center }
td, th { display: inline }
";

// --- the styled tree --------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Styled {
    pub style: Style,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Text(String),
    Element { tag: String, attrs: Vec<(String, String)>, children: Vec<Styled> },
}

impl Styled {
    pub fn tag(&self) -> Option<&str> {
        match &self.kind {
            Kind::Element { tag, .. } => Some(tag),
            Kind::Text(_) => None,
        }
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        match &self.kind {
            Kind::Element { attrs, .. } => attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()),
            Kind::Text(_) => None,
        }
    }

    pub fn children(&self) -> &[Styled] {
        match &self.kind {
            Kind::Element { children, .. } => children,
            Kind::Text(_) => &[],
        }
    }
}

/// Computes styles for `nodes` under `rules` (user-agent rules first, then
/// the page's, in source order). Elements with `display: none` are dropped.
pub fn style_tree(nodes: &[Node], rules: &[Rule]) -> Vec<Styled> {
    let mut chain = Vec::new();
    build(nodes, &Style::root(), rules, &mut chain)
}

fn build<'a>(nodes: &'a [Node], parent: &Style, rules: &[Rule], chain: &mut Vec<&'a Element>) -> Vec<Styled> {
    let mut out = Vec::with_capacity(nodes.len());
    for n in nodes {
        match n {
            Node::Text(t) => out.push(Styled { style: parent.for_text(), kind: Kind::Text(t.clone()) }),
            Node::Element(e) => {
                chain.push(e);
                let mut st = parent.inherit();
                cascade(e, chain, &mut st, parent, rules);
                if st.display != Display::None {
                    let children = build(&e.children, &st, rules, chain);
                    if st.display == Display::Inline && children.iter().any(|c| matches!(c.style.display, Display::Block | Display::ListItem) && c.tag().is_some()) {
                        st.display = Display::Block;
                    }
                    out.push(Styled { style: st, kind: Kind::Element { tag: e.tag.clone(), attrs: e.attrs.clone(), children } });
                }
                chain.pop();
            }
        }
    }
    out
}

fn cascade(e: &Element, chain: &[&Element], st: &mut Style, parent: &Style, rules: &[Rule]) {
    let inline = e.attr("style").map(parse_decls).unwrap_or_default();
    let mut hits: Vec<((bool, u32, usize), &Decl)> = Vec::new();
    for (order, rule) in rules.iter().enumerate() {
        let spec = rule.selectors.iter().filter(|s| s.matches(chain)).map(|s| s.specificity).max();
        if let Some(spec) = spec {
            hits.extend(rule.decls.iter().map(|d| ((d.important, spec, order), d)));
        }
    }
    hits.extend(inline.iter().map(|d| ((d.important, u32::MAX, 0), d)));
    hits.sort_by_key(|(k, _)| *k);
    // font-size first, so em lengths in the other declarations see the final size
    for (_, d) in hits.iter().filter(|(_, d)| d.name == "font-size") {
        if let Some(px) = font_size(&d.value, parent.font_size) {
            st.font_size = px;
        }
    }
    for (_, d) in hits.iter().filter(|(_, d)| d.name != "font-size") {
        apply(st, parent, &d.name, &d.value);
    }
    if e.attr("hidden").is_some() {
        st.display = Display::None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::browser::html;

    fn styled(src: &str, css: &str) -> Vec<Styled> {
        let mut rules = parse_stylesheet(USER_AGENT);
        rules.extend(parse_stylesheet(css));
        style_tree(&html::parse(src), &rules)
    }

    #[test]
    fn colours_parse_in_every_notation() {
        assert_eq!(parse_color("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("#00ff80"), Some(Color::rgb(0, 255, 128)));
        assert_eq!(parse_color("rgb(10, 20, 30)"), Some(Color::rgb(10, 20, 30)));
        assert_eq!(parse_color("rgba(0,0,0,.5)").map(|c| c.a), Some(128));
        assert_eq!(parse_color("Navy"), Some(Color::rgb(0, 0, 128)));
        assert_eq!(parse_color("#12"), None);
        assert_eq!(parse_color("nonsense"), None);
    }

    #[test]
    fn translucent_colours_pack_over_white() {
        let half = Color { r: 0, g: 0, b: 0, a: 128 };
        assert_eq!(half.pack() & 0xff, 127);
        assert_eq!(Color::rgb(1, 2, 3).pack() >> 24, 255);
    }

    #[test]
    fn lengths_resolve_units() {
        assert_eq!(parse_len("12px", 16.0), Some(Len::Px(12.0)));
        assert_eq!(parse_len("1.5em", 10.0), Some(Len::Px(15.0)));
        assert_eq!(parse_len("2rem", 10.0), Some(Len::Px(32.0)));
        assert_eq!(parse_len("50%", 10.0), Some(Len::Pct(50.0)));
        assert_eq!(parse_len("auto", 10.0), Some(Len::Auto));
        assert_eq!(parse_len("3vw", 10.0), None);
        assert_eq!(Len::Pct(25.0).resolve(200.0), 50.0);
    }

    #[test]
    fn rules_parse_with_comments_comma_lists_and_at_rules() {
        let css = "/* c */ a, .b > p { color: red; margin: 0 !important } @font-face { font-family: x } @import url(x.css); h1 { color: blue }";
        let rules = parse_stylesheet(css);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selectors.len(), 2);
        assert!(rules[0].decls[1].important);
    }

    #[test]
    fn unsupported_selectors_drop_their_rule() {
        assert!(parse_selector("a:hover").is_none());
        assert!(parse_selector("a[href]").is_none());
        assert!(parse_selector("a + b").is_none());
        assert_eq!(parse_stylesheet("a:hover, b { color: red }")[0].selectors.len(), 1);
    }

    #[test]
    fn selectors_match_with_combinators() {
        let tree = html::parse("<div class=box><section><p id=x class='a b'>t</p></section></div>");
        let div = tree[0].element().unwrap();
        let section = div.children[0].element().unwrap();
        let p = section.children[0].element().unwrap();
        let chain = [div, section, p];
        assert!(parse_selector("p").unwrap().matches(&chain));
        assert!(parse_selector(".box p").unwrap().matches(&chain));
        assert!(parse_selector("div p.a.b").unwrap().matches(&chain));
        assert!(parse_selector("section > p#x").unwrap().matches(&chain));
        assert!(!parse_selector("div > p").unwrap().matches(&chain));
        assert!(!parse_selector("p.c").unwrap().matches(&chain));
        assert!(parse_selector("*").unwrap().matches(&chain));
    }

    #[test]
    fn specificity_orders_ids_over_classes_over_tags() {
        let s = |x: &str| parse_selector(x).unwrap().specificity;
        assert!(s("#a") > s(".a.b.c"));
        assert!(s(".a") > s("div p"));
        assert!(s("div p") > s("p"));
    }

    #[test]
    fn the_cascade_prefers_specificity_then_order_then_important() {
        let css = "p { color: red } .x { color: blue } p { color: green } .y { color: #111 !important } p.y { color: #222 }";
        let t = styled("<p class=x>a</p><p class='x y'>b</p><p>c</p>", css);
        assert_eq!(t[0].style.color, Color::rgb(0, 0, 255));
        assert_eq!(t[1].style.color, Color::rgb(0x11, 0x11, 0x11));
        assert_eq!(t[2].style.color, Color::rgb(0, 128, 0));
    }

    #[test]
    fn the_style_attribute_beats_sheet_rules() {
        let t = styled("<p style='color: red; margin: 4px'>a</p>", "#x { color: blue } p { color: green }");
        assert_eq!(t[0].style.color, Color::rgb(255, 0, 0));
        assert_eq!(t[0].style.margin, [Len::Px(4.0); 4]);
    }

    #[test]
    fn text_properties_inherit_box_properties_do_not() {
        let t = styled("<div><span>x</span></div>", "div { color: red; font-size: 20px; margin: 9px; background: #ccc; font-weight: bold }");
        let span = &t[0].children()[0];
        assert_eq!(span.style.color, Color::rgb(255, 0, 0));
        assert_eq!(span.style.font_size, 20.0);
        assert!(span.style.bold);
        assert_eq!(span.style.margin, [Len::Px(0.0); 4]);
        assert_eq!(span.style.background, None);
    }

    #[test]
    fn em_units_use_the_elements_own_font_size() {
        let t = styled("<p>x</p>", "p { font-size: 20px; margin-top: 2em }");
        assert_eq!(t[0].style.margin[0], Len::Px(40.0));
        let t = styled("<div><h1>x</h1></div>", "div { font-size: 10px }");
        assert_eq!(t[0].children()[0].style.font_size, 20.0);
    }

    #[test]
    fn margin_shorthand_expands() {
        let t = styled("<p>x</p>", "p { margin: 1px 2px 3px } ");
        assert_eq!(t[0].style.margin, [Len::Px(1.0), Len::Px(2.0), Len::Px(3.0), Len::Px(2.0)]);
        let t = styled("<p>x</p>", "p { margin: 0 auto; padding-left: 7px }");
        assert_eq!(t[0].style.margin, [Len::Px(0.0), Len::Auto, Len::Px(0.0), Len::Auto]);
        assert_eq!(t[0].style.padding[3], Len::Px(7.0));
    }

    #[test]
    fn display_none_removes_the_subtree() {
        let t = styled("<div>a</div><p hidden>b</p><script>x</script><div style='display:none'>c</div>", "");
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn an_inline_with_a_block_child_becomes_a_block() {
        let t = styled("<a href=x><div>card</div></a><a href=y>plain</a>", "");
        assert_eq!(t[0].style.display, Display::Block);
        assert_eq!(t[1].style.display, Display::Inline);
    }

    #[test]
    fn user_agent_sheet_styles_headings_links_and_lists() {
        let t = styled("<h1>t</h1><a href=x>l</a><ul><li>i</li></ul>", "");
        assert!(t[0].style.bold && t[0].style.font_size == 32.0);
        assert!(t[1].style.underline && t[1].style.color == Color::rgb(0x0a, 0x58, 0xca));
        assert_eq!(t[2].children()[0].style.display, Display::ListItem);
    }

    #[test]
    fn media_queries_follow_the_viewport() {
        let css = "@media (max-width: 600px) { p { color: red } } @media (min-width: 700px) { p { color: blue } } @media print { p { color: green } }";
        let t = styled("<p>x</p>", css);
        assert_eq!(t[0].style.color, Color::rgb(0, 0, 255));
    }

    #[test]
    fn borders_and_backgrounds_parse() {
        let t = styled("<p>x</p>", "p { border-bottom: 2px solid #f00; background: url(a.png) #eee no-repeat; border-top: none }");
        assert_eq!(t[0].style.border[2], Some((2.0, Color::rgb(255, 0, 0))));
        assert_eq!(t[0].style.border[0], None);
        assert_eq!(t[0].style.background, Some(Color::rgb(0xee, 0xee, 0xee)));
    }

    #[test]
    fn declarations_with_functions_we_cannot_evaluate_are_skipped() {
        let d = parse_decls("width: calc(100% - 2px); color: var(--c); margin: 1px; background: rgb(1,2,3);");
        let names: Vec<_> = d.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["margin", "background"]);
    }

    #[test]
    fn media_queries_follow_not_only_and_comma_lists() {
        let n = |css: &str| parse_stylesheet(css).len();
        assert_eq!(n("@media not print { p { color: red } }"), 1);
        assert_eq!(n("@media print, (min-width: 100px) { p { color: red } }"), 1);
        assert_eq!(n("@media print, (min-width: 9000px) { p { color: red } }"), 0);
        assert_eq!(n("@media only screen and (max-width: 900px) { p { color: red } }"), 1);
        assert_eq!(n("@media not all and (max-width: 900px) { p { color: red } }"), 0);
    }

    #[test]
    fn important_inside_a_string_is_not_a_flag() {
        let d = parse_decls("content: \"a !important\"; color: red ! important");
        assert!(!d[0].important);
        assert_eq!(d[0].value, "\"a !important\"");
        assert!(d[1].important);
        assert_eq!(d[1].value, "red");
    }

    #[test]
    fn braces_and_comment_markers_in_strings_and_urls_are_literal() {
        let r = parse_stylesheet("a { content: \"}/*\" } b { color: red } c { background: url(x/*y}.png) } d { color: blue }");
        assert_eq!(r.len(), 4);
        assert_eq!(r[1].decls[0].value, "red");
        assert_eq!(r[3].decls[0].value, "blue");
    }
}
