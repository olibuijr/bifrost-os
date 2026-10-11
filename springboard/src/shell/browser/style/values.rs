//! CSS value types and their parsers: keywords, colors, lengths, `calc()`
//! expressions, shadows, gradients, 2D transforms, generated content and
//! grid track lists. Everything here is pure data plus parsing over
//! cssparser; resolving against an element happens in `computed`.

use cssparser::color::{parse_hash_color, parse_named_color};
use cssparser::{ParseError, Parser, Token};

/// Error type of every value parser (no custom errors).
pub type PErr = ParseError<()>;
/// Result type of every value parser.
pub type PResult<T> = Result<T, PErr>;

/// A parse failure: the declaration is dropped.
pub fn fail<T>() -> PResult<T> {
    Err(ParseError::unexpected_token())
}

/// Parser helpers that pin the error type to `PErr` so closures infer.
pub trait ParserExt<'i> {
    /// `parse_nested_block` with the crate error type.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Parser<'i>) -> PResult<T>) -> PResult<T>;
    /// `parse_comma_separated` with the crate error type.
    fn comma<T>(&mut self, f: impl FnMut(&mut Parser<'i>) -> PResult<T>) -> PResult<Vec<T>>;
}

impl<'i> ParserExt<'i> for Parser<'i> {
    fn nested<T>(&mut self, f: impl FnOnce(&mut Parser<'i>) -> PResult<T>) -> PResult<T> {
        self.parse_nested_block(f)
    }
    fn comma<T>(&mut self, f: impl FnMut(&mut Parser<'i>) -> PResult<T>) -> PResult<Vec<T>> {
        self.parse_comma_separated(f)
    }
}

fn some<T>(o: Option<T>) -> PResult<T> {
    match o {
        Some(v) => Ok(v),
        None => fail(),
    }
}

// ---------------------------------------------------------------------------
// Keyword enums
// ---------------------------------------------------------------------------

/// A property value that is one keyword out of a closed set.
pub trait Kw: Sized + Default {
    /// Parses a keyword, ASCII case-insensitively.
    fn from_ident(s: &str) -> Option<Self>;
}

macro_rules! keyword_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident = $($kw:literal)|+),+ $(,)? } default $def:ident) => {
        $(#[$m])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum $name { $($var),+ }
        impl Default for $name {
            fn default() -> Self { $name::$def }
        }
        impl Kw for $name {
            fn from_ident(s: &str) -> Option<Self> {
                $( $( if s.eq_ignore_ascii_case($kw) { return Some($name::$var); } )+ )+
                None
            }
        }
        impl $name {
            /// The canonical keyword.
            pub fn keyword(self) -> &'static str {
                match self { $( $name::$var => keyword_enum!(@first $($kw),+) ),+ }
            }
        }
    };
    (@first $first:literal $(, $rest:literal)*) => { $first };
}

keyword_enum!(
    /// The `display` property.
    Display {
        None = "none", Block = "block", Inline = "inline", InlineBlock = "inline-block",
        Flex = "flex", InlineFlex = "inline-flex", Grid = "grid", InlineGrid = "inline-grid",
        FlowRoot = "flow-root", ListItem = "list-item", Table = "table",
        InlineTable = "inline-table", TableRow = "table-row", TableCell = "table-cell",
        TableRowGroup = "table-row-group", TableHeaderGroup = "table-header-group",
        TableFooterGroup = "table-footer-group", TableColumn = "table-column",
        TableColumnGroup = "table-column-group", TableCaption = "table-caption",
        Contents = "contents",
    } default Inline
);
keyword_enum!(
    /// The `position` property.
    Position {
        Static = "static", Relative = "relative", Absolute = "absolute",
        Fixed = "fixed", Sticky = "sticky",
    } default Static
);
keyword_enum!(
    /// The `float` property.
    Float { None = "none", Left = "left", Right = "right", InlineStart = "inline-start",
        InlineEnd = "inline-end" } default None
);
keyword_enum!(
    /// The `clear` property.
    Clear { None = "none", Left = "left", Right = "right", Both = "both" } default None
);
keyword_enum!(
    /// The `box-sizing` property.
    BoxSizing { ContentBox = "content-box", BorderBox = "border-box" } default ContentBox
);
keyword_enum!(
    /// The `visibility` property.
    Visibility { Visible = "visible", Hidden = "hidden", Collapse = "collapse" } default Visible
);
keyword_enum!(
    /// The `overflow-x` / `overflow-y` properties.
    Overflow {
        Visible = "visible", Hidden = "hidden", Scroll = "scroll", Auto = "auto",
        Clip = "clip",
    } default Visible
);
keyword_enum!(
    /// Border line styles.
    BorderStyle {
        None = "none", Hidden = "hidden", Solid = "solid", Dashed = "dashed",
        Dotted = "dotted", Double = "double", Groove = "groove", Ridge = "ridge",
        Inset = "inset", Outset = "outset",
    } default None
);
keyword_enum!(
    /// The `font-style` property.
    FontStyle { Normal = "normal", Italic = "italic", Oblique = "oblique" } default Normal
);
keyword_enum!(
    /// The `text-align` property.
    TextAlign {
        Start = "start", End = "end", Left = "left" | "-webkit-left", Right = "right" | "-webkit-right",
        Center = "center" | "-webkit-center", Justify = "justify",
    } default Start
);
keyword_enum!(
    /// The `text-transform` property.
    TextTransform {
        None = "none", Uppercase = "uppercase", Lowercase = "lowercase",
        Capitalize = "capitalize",
    } default None
);
keyword_enum!(
    /// The `white-space` property.
    WhiteSpace {
        Normal = "normal", Nowrap = "nowrap", Pre = "pre", PreWrap = "pre-wrap",
        PreLine = "pre-line", BreakSpaces = "break-spaces",
    } default Normal
);
keyword_enum!(
    /// The `word-break` property.
    WordBreak {
        Normal = "normal", BreakAll = "break-all", KeepAll = "keep-all",
        BreakWord = "break-word",
    } default Normal
);
keyword_enum!(
    /// The `overflow-wrap` property.
    OverflowWrap { Normal = "normal", BreakWord = "break-word", Anywhere = "anywhere" } default Normal
);
keyword_enum!(
    /// The `text-overflow` property.
    TextOverflow { Clip = "clip", Ellipsis = "ellipsis" } default Clip
);
keyword_enum!(
    /// The `flex-direction` property.
    FlexDirection {
        Row = "row", RowReverse = "row-reverse", Column = "column",
        ColumnReverse = "column-reverse",
    } default Row
);
keyword_enum!(
    /// The `flex-wrap` property.
    FlexWrap { Nowrap = "nowrap", Wrap = "wrap", WrapReverse = "wrap-reverse" } default Nowrap
);
keyword_enum!(
    /// Box alignment keywords shared by `justify-*` and `align-*`.
    Align {
        Normal = "normal", Auto = "auto", Stretch = "stretch", FlexStart = "flex-start",
        FlexEnd = "flex-end", Center = "center", Baseline = "baseline" | "first baseline",
        Start = "start", End = "end", SelfStart = "self-start", SelfEnd = "self-end",
        SpaceBetween = "space-between", SpaceAround = "space-around",
        SpaceEvenly = "space-evenly", Left = "left", Right = "right",
    } default Normal
);
keyword_enum!(
    /// The `list-style-type` property.
    ListStyleType {
        Disc = "disc", Circle = "circle", Square = "square", Decimal = "decimal",
        DecimalLeadingZero = "decimal-leading-zero", LowerAlpha = "lower-alpha" | "lower-latin",
        UpperAlpha = "upper-alpha" | "upper-latin", LowerRoman = "lower-roman",
        UpperRoman = "upper-roman", None = "none",
    } default Disc
);
keyword_enum!(
    /// Background repeat of one axis.
    BgRepeatKind { Repeat = "repeat", NoRepeat = "no-repeat", Space = "space", Round = "round" } default Repeat
);
keyword_enum!(
    /// Background box (clip, origin).
    BgBox {
        BorderBox = "border-box", PaddingBox = "padding-box", ContentBox = "content-box",
        Text = "text",
    } default BorderBox
);
keyword_enum!(
    /// Keyword values of `vertical-align`.
    VAlignKw {
        Baseline = "baseline", Sub = "sub", Super = "super", Top = "top", Middle = "middle",
        Bottom = "bottom", TextTop = "text-top", TextBottom = "text-bottom",
    } default Baseline
);
keyword_enum!(
    /// `grid-auto-flow` as one keyword (row/column, dense or not).
    GridAutoFlow {
        Row = "row", Column = "column", RowDense = "row dense" | "dense", ColumnDense = "column dense",
    } default Row
);

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

/// A color with `f32` channels in 0..=1 (not premultiplied).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const BLACK: Rgba = Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE: Rgba = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

    /// From 8-bit channels and a 0..=1 alpha.
    pub fn from_u8(r: u8, g: u8, b: u8, a: f32) -> Rgba {
        Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a }
    }

    /// Packs to `0xAARRGGBB`.
    pub fn to_argb(&self) -> u32 {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u32;
        (q(self.a) << 24) | (q(self.r) << 16) | (q(self.g) << 8) | q(self.b)
    }

    /// The same color with its alpha multiplied by `k`.
    pub fn with_alpha_mul(&self, k: f32) -> Rgba {
        Rgba { a: (self.a * k).clamp(0.0, 1.0), ..*self }
    }

    /// Premultiplied interpolation: `self * (1 - t) + other * t`.
    pub fn mix(&self, other: &Rgba, t: f32) -> Rgba {
        let a = self.a * (1.0 - t) + other.a * t;
        if a <= 0.0 {
            return Rgba::TRANSPARENT;
        }
        let ch = |x: f32, y: f32| (x * self.a * (1.0 - t) + y * other.a * t) / a;
        Rgba { r: ch(self.r, other.r), g: ch(self.g, other.g), b: ch(self.b, other.b), a }
    }
}

/// A specified color: a literal or `currentcolor`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Color {
    Rgba(Rgba),
    Current,
}

impl Color {
    /// Resolves `currentcolor` against `current`.
    pub fn resolve(&self, current: Rgba) -> Rgba {
        match self {
            Color::Rgba(c) => *c,
            Color::Current => current,
        }
    }
}

/// Parses any supported color.
pub fn parse_color(input: &mut Parser<'_>) -> PResult<Color> {
    let tok = input.next()?.clone();
    match tok {
        Token::Hash(ref h) | Token::IDHash(ref h) => match parse_hash_color(h.as_bytes()) {
            Ok((r, g, b, a)) => Ok(Color::Rgba(Rgba::from_u8(r, g, b, a))),
            Err(()) => fail(),
        },
        Token::Ident(ref name) => {
            let n = name.to_ascii_lowercase();
            match n.as_str() {
                "currentcolor" => Ok(Color::Current),
                "transparent" => Ok(Color::Rgba(Rgba::TRANSPARENT)),
                _ => match parse_named_color(&n) {
                    Ok((r, g, b)) => Ok(Color::Rgba(Rgba::from_u8(r, g, b, 1.0))),
                    Err(()) => fail(),
                },
            }
        }
        Token::Function(ref name) => {
            let n = name.to_ascii_lowercase();
            input.nested(|i| match n.as_str() {
                "rgb" | "rgba" => parse_rgb_args(i),
                "hsl" | "hsla" => parse_hsl_args(i),
                "hwb" => parse_hwb_args(i),
                "color-mix" => parse_color_mix(i),
                _ => fail(),
            })
        }
        _ => fail(),
    }
}

/// A percentage token as percent points: the integer when the source was one (30% is exactly
/// 30.0, where 0.3 * 100.0 in f32 gives 30.000002), else the scaled fraction.
fn percent_points(unit_value: f32, int_value: Option<i32>) -> f32 {
    int_value.map_or(unit_value * 100.0, |i| i as f32)
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn rgb_comp(i: &mut Parser<'_>) -> PResult<f32> {
    let t = i.next()?.clone();
    match t {
        Token::Number { value, .. } => Ok(clamp01(value / 255.0)),
        Token::Percentage { unit_value, .. } => Ok(clamp01(unit_value)),
        Token::Ident(ref s) if s.eq_ignore_ascii_case("none") => Ok(0.0),
        _ => fail(),
    }
}

fn parse_alpha(i: &mut Parser<'_>, comma: bool) -> PResult<f32> {
    let sep = if comma {
        i.try_parse(|i| i.expect_comma()).is_ok()
    } else {
        i.try_parse(|i| i.expect_delim('/')).is_ok()
    };
    if !sep {
        return Ok(1.0);
    }
    let t = i.next()?.clone();
    match t {
        Token::Number { value, .. } => Ok(clamp01(value)),
        Token::Percentage { unit_value, .. } => Ok(clamp01(unit_value)),
        Token::Ident(ref s) if s.eq_ignore_ascii_case("none") => Ok(0.0),
        _ => fail(),
    }
}

fn parse_rgb_args(i: &mut Parser<'_>) -> PResult<Color> {
    let r = rgb_comp(i)?;
    let comma = i.try_parse(|i| i.expect_comma()).is_ok();
    let g = rgb_comp(i)?;
    if comma {
        i.expect_comma()?;
    }
    let b = rgb_comp(i)?;
    let a = parse_alpha(i, comma)?;
    Ok(Color::Rgba(Rgba { r, g, b, a }))
}

/// Parses a hue: a number (degrees) or an angle; returns degrees.
fn parse_hue(i: &mut Parser<'_>) -> PResult<f32> {
    let t = i.next()?.clone();
    match t {
        Token::Number { value, .. } => Ok(value),
        Token::Dimension { value, ref unit, .. } => angle_to_deg(value, unit),
        Token::Ident(ref s) if s.eq_ignore_ascii_case("none") => Ok(0.0),
        _ => fail(),
    }
}

fn angle_to_deg(value: f32, unit: &str) -> PResult<f32> {
    let u = unit.to_ascii_lowercase();
    match u.as_str() {
        "deg" => Ok(value),
        "rad" => Ok(value.to_degrees()),
        "grad" => Ok(value * 0.9),
        "turn" => Ok(value * 360.0),
        _ => fail(),
    }
}

/// Parses an `<angle>` (or a bare `0`) as degrees.
pub fn parse_angle(i: &mut Parser<'_>) -> PResult<f32> {
    let t = i.next()?.clone();
    match t {
        Token::Number { value, .. } if value == 0.0 => Ok(0.0),
        Token::Dimension { value, ref unit, .. } => angle_to_deg(value, unit),
        _ => fail(),
    }
}

fn pct_or_number(i: &mut Parser<'_>) -> PResult<f32> {
    let t = i.next()?.clone();
    match t {
        Token::Number { value, .. } => Ok(clamp01(value / 100.0)),
        Token::Percentage { unit_value, .. } => Ok(clamp01(unit_value)),
        Token::Ident(ref s) if s.eq_ignore_ascii_case("none") => Ok(0.0),
        _ => fail(),
    }
}

/// Converts HSL (hue in degrees, s and l in 0..=1) to RGB.
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let h = h.rem_euclid(360.0) / 360.0;
    let t2 = if l <= 0.5 { l * (s + 1.0) } else { l + s - l * s };
    let t1 = l * 2.0 - t2;
    let f = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t * 6.0 < 1.0 {
            t1 + (t2 - t1) * t * 6.0
        } else if t * 2.0 < 1.0 {
            t2
        } else if t * 3.0 < 2.0 {
            t1 + (t2 - t1) * (2.0 / 3.0 - t) * 6.0
        } else {
            t1
        }
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

fn parse_hsl_args(i: &mut Parser<'_>) -> PResult<Color> {
    let h = parse_hue(i)?;
    let comma = i.try_parse(|i| i.expect_comma()).is_ok();
    let s = pct_or_number(i)?;
    if comma {
        i.expect_comma()?;
    }
    let l = pct_or_number(i)?;
    let a = parse_alpha(i, comma)?;
    let (r, g, b) = hsl_to_rgb(h, s, l);
    Ok(Color::Rgba(Rgba { r: clamp01(r), g: clamp01(g), b: clamp01(b), a }))
}

fn parse_hwb_args(i: &mut Parser<'_>) -> PResult<Color> {
    let h = parse_hue(i)?;
    let w = pct_or_number(i)?;
    let bl = pct_or_number(i)?;
    let a = parse_alpha(i, false)?;
    let (w, bl) = if w + bl > 1.0 { (w / (w + bl), bl / (w + bl)) } else { (w, bl) };
    let (r, g, b) = hsl_to_rgb(h, 1.0, 0.5);
    let f = |c: f32| clamp01(c * (1.0 - w - bl) + w);
    Ok(Color::Rgba(Rgba { r: f(r), g: f(g), b: f(b), a }))
}

fn mix_part(i: &mut Parser<'_>) -> PResult<(Rgba, Option<f32>)> {
    let mut pct = i.try_parse(|i| i.expect_percentage()).ok();
    let c = match parse_color(i)? {
        Color::Rgba(c) => c,
        Color::Current => return fail(),
    };
    if pct.is_none() {
        pct = i.try_parse(|i| i.expect_percentage()).ok();
    }
    Ok((c, pct))
}

fn parse_color_mix(i: &mut Parser<'_>) -> PResult<Color> {
    i.expect_ident_matching("in")?;
    // The interpolation space (and optional hue method) is ignored: sRGB.
    loop {
        if i.try_parse(|i| i.expect_ident_cloned()).is_err() {
            break;
        }
    }
    i.expect_comma()?;
    let (c1, p1) = mix_part(i)?;
    i.expect_comma()?;
    let (c2, p2) = mix_part(i)?;
    let (w1, w2) = match (p1, p2) {
        (None, None) => (0.5, 0.5),
        (Some(a), None) => (a, 1.0 - a),
        (None, Some(b)) => (1.0 - b, b),
        (Some(a), Some(b)) => (a, b),
    };
    let sum = w1 + w2;
    if sum <= 0.0 {
        return fail();
    }
    let t = w2 / sum;
    let mut c = c1.mix(&c2, t);
    if sum < 1.0 {
        c = c.with_alpha_mul(sum);
    }
    Ok(Color::Rgba(c))
}

// ---------------------------------------------------------------------------
// Lengths and calc()
// ---------------------------------------------------------------------------

/// A CSS length unit (and `%`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Vw,
    Vh,
    Vmin,
    Vmax,
    Ch,
    Ex,
    Pt,
    Pc,
    In,
    Cm,
    Mm,
    Q,
    Percent,
}

/// Parses a unit name.
pub fn unit_from(s: &str) -> Option<Unit> {
    let l = s.to_ascii_lowercase();
    Some(match l.as_str() {
        "px" => Unit::Px,
        "em" => Unit::Em,
        "rem" => Unit::Rem,
        "vw" | "dvw" | "svw" | "lvw" => Unit::Vw,
        "vh" | "dvh" | "svh" | "lvh" => Unit::Vh,
        "vmin" | "dvmin" | "svmin" | "lvmin" => Unit::Vmin,
        "vmax" | "dvmax" | "svmax" | "lvmax" => Unit::Vmax,
        "ch" => Unit::Ch,
        "ex" => Unit::Ex,
        "pt" => Unit::Pt,
        "pc" => Unit::Pc,
        "in" => Unit::In,
        "cm" => Unit::Cm,
        "mm" => Unit::Mm,
        "q" => Unit::Q,
        _ => return None,
    })
}

/// What relative units resolve against.
#[derive(Clone, Copy, Debug)]
pub struct LenCtx {
    /// Font size of the element (px) for `em`, `ch`, `ex`.
    pub font_size: f32,
    /// Font size of the root element (px) for `rem`.
    pub root_font_size: f32,
    /// Viewport width in px (`100vw`).
    pub vw: f32,
    /// Viewport height in px (`100vh`).
    pub vh: f32,
}

impl LenCtx {
    /// A context with a 16px font and a 1000x1000 viewport, for absolute units.
    pub const ABSOLUTE: LenCtx = LenCtx { font_size: 16.0, root_font_size: 16.0, vw: 1000.0, vh: 1000.0 };

    /// Converts a dimension to px; `None` for `%`.
    pub fn unit_to_px(&self, v: f32, u: Unit) -> Option<f32> {
        Some(match u {
            Unit::Px => v,
            Unit::Em => v * self.font_size,
            Unit::Rem => v * self.root_font_size,
            Unit::Vw => v * self.vw / 100.0,
            Unit::Vh => v * self.vh / 100.0,
            Unit::Vmin => v * self.vw.min(self.vh) / 100.0,
            Unit::Vmax => v * self.vw.max(self.vh) / 100.0,
            Unit::Ch | Unit::Ex => v * self.font_size * 0.5,
            Unit::Pt => v * 96.0 / 72.0,
            Unit::Pc => v * 16.0,
            Unit::In => v * 96.0,
            Unit::Cm => v * 96.0 / 2.54,
            Unit::Mm => v * 96.0 / 25.4,
            Unit::Q => v * 96.0 / 101.6,
            Unit::Percent => return None,
        })
    }
}

/// A math expression: `calc()`, `min()`, `max()`, `clamp()`.
#[derive(Clone, PartialEq, Debug)]
pub enum Calc {
    Num(f32),
    Dim(f32, Unit),
    Add(Box<Calc>, Box<Calc>),
    Sub(Box<Calc>, Box<Calc>),
    Mul(Box<Calc>, Box<Calc>),
    Div(Box<Calc>, Box<Calc>),
    Min(Vec<Calc>),
    Max(Vec<Calc>),
    Clamp(Box<Calc>, Box<Calc>, Box<Calc>),
}

impl Calc {
    /// Replaces every dimension by px or `%` leaves using `ctx`.
    pub fn compute(&self, ctx: &LenCtx) -> Calc {
        match self {
            Calc::Num(n) => Calc::Num(*n),
            Calc::Dim(v, Unit::Percent) => Calc::Dim(*v, Unit::Percent),
            Calc::Dim(v, u) => Calc::Dim(ctx.unit_to_px(*v, *u).unwrap_or(0.0), Unit::Px),
            Calc::Add(a, b) => Calc::Add(Box::new(a.compute(ctx)), Box::new(b.compute(ctx))),
            Calc::Sub(a, b) => Calc::Sub(Box::new(a.compute(ctx)), Box::new(b.compute(ctx))),
            Calc::Mul(a, b) => Calc::Mul(Box::new(a.compute(ctx)), Box::new(b.compute(ctx))),
            Calc::Div(a, b) => Calc::Div(Box::new(a.compute(ctx)), Box::new(b.compute(ctx))),
            Calc::Min(v) => Calc::Min(v.iter().map(|c| c.compute(ctx)).collect()),
            Calc::Max(v) => Calc::Max(v.iter().map(|c| c.compute(ctx)).collect()),
            Calc::Clamp(a, b, c) => Calc::Clamp(
                Box::new(a.compute(ctx)),
                Box::new(b.compute(ctx)),
                Box::new(c.compute(ctx)),
            ),
        }
    }

    /// Whether any leaf is a percentage.
    pub fn has_percent(&self) -> bool {
        match self {
            Calc::Num(_) => false,
            Calc::Dim(_, u) => *u == Unit::Percent,
            Calc::Add(a, b) | Calc::Sub(a, b) | Calc::Mul(a, b) | Calc::Div(a, b) => {
                a.has_percent() || b.has_percent()
            }
            Calc::Min(v) | Calc::Max(v) => v.iter().any(|c| c.has_percent()),
            Calc::Clamp(a, b, c) => a.has_percent() || b.has_percent() || c.has_percent(),
        }
    }

    /// Evaluates to px; `%` leaves resolve against `basis`.
    pub fn resolve(&self, basis: f32) -> f32 {
        match self {
            Calc::Num(n) => *n,
            Calc::Dim(v, Unit::Percent) => v * basis / 100.0,
            Calc::Dim(v, u) => LenCtx::ABSOLUTE.unit_to_px(*v, *u).unwrap_or(0.0),
            Calc::Add(a, b) => a.resolve(basis) + b.resolve(basis),
            Calc::Sub(a, b) => a.resolve(basis) - b.resolve(basis),
            Calc::Mul(a, b) => a.resolve(basis) * b.resolve(basis),
            Calc::Div(a, b) => {
                let d = b.resolve(basis);
                if d == 0.0 {
                    0.0
                } else {
                    a.resolve(basis) / d
                }
            }
            Calc::Min(v) => v.iter().map(|c| c.resolve(basis)).fold(f32::INFINITY, f32::min),
            Calc::Max(v) => v.iter().map(|c| c.resolve(basis)).fold(f32::NEG_INFINITY, f32::max),
            Calc::Clamp(lo, val, hi) => {
                let (lo, val, hi) = (lo.resolve(basis), val.resolve(basis), hi.resolve(basis));
                lo.max(val.min(hi))
            }
        }
    }
}

/// A length-like value. Specified values may hold `Dim` (unresolved units);
/// after `compute` only `Px`, `Percent`, `Calc` (px and `%` leaves) and the
/// keywords remain.
#[derive(Clone, PartialEq, Debug)]
pub enum Length {
    Auto,
    /// `none` (for `max-width` and friends).
    None,
    MinContent,
    MaxContent,
    FitContent,
    Px(f32),
    Percent(f32),
    /// Unresolved dimension (specified values only).
    Dim(f32, Unit),
    Calc(Box<Calc>),
}

impl Length {
    pub const ZERO: Length = Length::Px(0.0);

    /// Resolves relative units against `ctx`.
    pub fn compute(&self, ctx: &LenCtx) -> Length {
        match self {
            Length::Dim(v, Unit::Percent) => Length::Percent(*v),
            Length::Dim(v, u) => Length::Px(ctx.unit_to_px(*v, *u).unwrap_or(0.0)),
            Length::Calc(c) => {
                let c = c.compute(ctx);
                if c.has_percent() {
                    Length::Calc(Box::new(c))
                } else {
                    Length::Px(c.resolve(0.0))
                }
            }
            other => other.clone(),
        }
    }

    /// The px value given a percentage basis; `None` for keywords, or for a
    /// percentage when `basis` is `None`.
    pub fn resolve(&self, basis: Option<f32>) -> Option<f32> {
        match self {
            Length::Px(v) => Some(*v),
            Length::Percent(p) => basis.map(|b| p * b / 100.0),
            Length::Calc(c) => {
                if c.has_percent() && basis.is_none() {
                    None
                } else {
                    Some(c.resolve(basis.unwrap_or(0.0)))
                }
            }
            Length::Dim(v, u) => match LenCtx::ABSOLUTE.unit_to_px(*v, *u) {
                Some(px) => Some(px),
                None => basis.map(|b| v * b / 100.0),
            },
            _ => None,
        }
    }

    /// Whether this is `auto`.
    pub fn is_auto(&self) -> bool {
        matches!(self, Length::Auto)
    }
}

/// Which keywords `parse_length_kw` accepts besides lengths.
#[derive(Clone, Copy, Default)]
pub struct LenKw {
    pub auto: bool,
    pub none: bool,
    pub content: bool,
}

impl LenKw {
    pub const NONE: LenKw = LenKw { auto: false, none: false, content: false };
    pub const AUTO: LenKw = LenKw { auto: true, none: false, content: false };
    /// `width`/`height`: auto plus content sizing keywords.
    pub const SIZE: LenKw = LenKw { auto: true, none: false, content: true };
    /// `max-width`/`max-height`: none plus content sizing keywords.
    pub const MAX: LenKw = LenKw { auto: false, none: true, content: true };
}

/// Parses a length, a percentage, `0`, or a math function.
pub fn parse_length_pct(input: &mut Parser<'_>) -> PResult<Length> {
    parse_length_kw(input, LenKw::NONE)
}

/// Like `parse_length_pct` but also accepts the keywords in `kw`.
pub fn parse_length_kw(input: &mut Parser<'_>, kw: LenKw) -> PResult<Length> {
    let tok = input.next()?.clone();
    match tok {
        Token::Dimension { value, ref unit, .. } => Ok(Length::Dim(value, some(unit_from(unit))?)),
        Token::Percentage { unit_value, int_value, .. } => Ok(Length::Dim(percent_points(unit_value, int_value), Unit::Percent)),
        Token::Number { value, .. } if value == 0.0 => Ok(Length::Px(0.0)),
        Token::Ident(ref s) => {
            let l = s.to_ascii_lowercase();
            match l.as_str() {
                "auto" if kw.auto => Ok(Length::Auto),
                "none" if kw.none => Ok(Length::None),
                "min-content" if kw.content => Ok(Length::MinContent),
                "max-content" if kw.content => Ok(Length::MaxContent),
                "fit-content" | "-webkit-fit-content" | "-moz-available" if kw.content => {
                    Ok(Length::FitContent)
                }
                _ => fail(),
            }
        }
        Token::Function(ref name) => {
            let n = name.to_ascii_lowercase();
            let calc = input.nested(|i| math_fn_body(&n, i))?;
            Ok(Length::Calc(Box::new(calc)))
        }
        _ => fail(),
    }
}

/// Parses a non-negative-agnostic `<number>`.
pub fn parse_number(input: &mut Parser<'_>) -> PResult<f32> {
    let tok = input.next()?.clone();
    match tok {
        Token::Number { value, .. } => Ok(value),
        Token::Function(ref name) if name.eq_ignore_ascii_case("calc") => {
            let c = input.nested(parse_sum)?;
            Ok(c.resolve(0.0))
        }
        _ => fail(),
    }
}

fn math_fn_body(name: &str, i: &mut Parser<'_>) -> PResult<Calc> {
    match name {
        "calc" | "-webkit-calc" | "-moz-calc" => parse_sum(i),
        "min" => Ok(Calc::Min(i.comma(parse_sum)?)),
        "max" => Ok(Calc::Max(i.comma(parse_sum)?)),
        "clamp" => {
            let mut v = i.comma(parse_sum)?;
            if v.len() != 3 {
                return fail();
            }
            let hi = v.pop().unwrap();
            let val = v.pop().unwrap();
            let lo = v.pop().unwrap();
            Ok(Calc::Clamp(Box::new(lo), Box::new(val), Box::new(hi)))
        }
        _ => fail(),
    }
}

fn parse_sum(i: &mut Parser<'_>) -> PResult<Calc> {
    let mut lhs = parse_product(i)?;
    loop {
        let st = i.state();
        let t = i.next().ok().cloned();
        match t {
            Some(Token::Delim('+')) => {
                let rhs = parse_product(i)?;
                lhs = Calc::Add(Box::new(lhs), Box::new(rhs));
            }
            Some(Token::Delim('-')) => {
                let rhs = parse_product(i)?;
                lhs = Calc::Sub(Box::new(lhs), Box::new(rhs));
            }
            _ => {
                i.reset(&st);
                break;
            }
        }
    }
    Ok(lhs)
}

fn parse_product(i: &mut Parser<'_>) -> PResult<Calc> {
    let mut lhs = parse_unary(i)?;
    loop {
        let st = i.state();
        let t = i.next().ok().cloned();
        match t {
            Some(Token::Delim('*')) => {
                let rhs = parse_unary(i)?;
                lhs = Calc::Mul(Box::new(lhs), Box::new(rhs));
            }
            Some(Token::Delim('/')) => {
                let rhs = parse_unary(i)?;
                lhs = Calc::Div(Box::new(lhs), Box::new(rhs));
            }
            _ => {
                i.reset(&st);
                break;
            }
        }
    }
    Ok(lhs)
}

fn parse_unary(i: &mut Parser<'_>) -> PResult<Calc> {
    let t = i.next()?.clone();
    match t {
        Token::Number { value, .. } => Ok(Calc::Num(value)),
        Token::Dimension { value, ref unit, .. } => Ok(Calc::Dim(value, some(unit_from(unit))?)),
        Token::Percentage { unit_value, int_value, .. } => Ok(Calc::Dim(percent_points(unit_value, int_value), Unit::Percent)),
        Token::ParenthesisBlock => i.nested(parse_sum),
        Token::Function(ref name) => {
            let n = name.to_ascii_lowercase();
            i.nested(|i| math_fn_body(&n, i))
        }
        Token::Ident(ref s) if s.eq_ignore_ascii_case("pi") => Ok(Calc::Num(std::f32::consts::PI)),
        Token::Ident(ref s) if s.eq_ignore_ascii_case("e") => Ok(Calc::Num(std::f32::consts::E)),
        _ => fail(),
    }
}

// ---------------------------------------------------------------------------
// Backgrounds, shadows, transforms, content, grid
// ---------------------------------------------------------------------------

/// One gradient color stop.
#[derive(Clone, PartialEq, Debug)]
pub struct GradientStop {
    pub color: Color,
    pub pos: Option<Length>,
}

/// Gradient geometry.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GradientKind {
    /// Angle in degrees, 0 = up, clockwise.
    Linear(f32),
    Radial,
}

/// A gradient image.
#[derive(Clone, PartialEq, Debug)]
pub struct Gradient {
    pub kind: GradientKind,
    pub repeating: bool,
    pub stops: Vec<GradientStop>,
}

/// A `background-image` layer.
#[derive(Clone, PartialEq, Debug)]
pub enum BgImage {
    None,
    Url(String),
    Gradient(Gradient),
}

/// A `background-size` layer.
#[derive(Clone, PartialEq, Debug)]
pub enum BgSize {
    Cover,
    Contain,
    Explicit(Length, Length),
}

/// A box shadow or text shadow. Lengths are `Px` after compute.
#[derive(Clone, PartialEq, Debug)]
pub struct Shadow {
    pub x: Length,
    pub y: Length,
    pub blur: Length,
    pub spread: Length,
    pub color: Color,
    pub inset: bool,
}

impl Shadow {
    /// `(x, y, blur, spread)` in px (computed shadows only).
    pub fn px(&self) -> (f32, f32, f32, f32) {
        let f = |l: &Length| l.resolve(None).unwrap_or(0.0);
        (f(&self.x), f(&self.y), f(&self.blur), f(&self.spread))
    }
}

/// One function of a `transform` list.
#[derive(Clone, PartialEq, Debug)]
pub enum TransformFn {
    Translate(Length, Length),
    Scale(f32, f32),
    /// Degrees.
    Rotate(f32),
    /// Degrees.
    Skew(f32, f32),
    Matrix([f32; 6]),
}

/// A composed 2D transform `[a b c d e f]` (CSS matrix order) plus a
/// percentage translation that resolves against the border box.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Transform2D {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
    /// Translation in percent of the box width / height, already carried
    /// through the linear part.
    pub ep: f32,
    pub fp: f32,
}

impl Transform2D {
    pub const IDENTITY: Transform2D =
        Transform2D { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0, ep: 0.0, fp: 0.0 };

    /// Composes the list left to right (`T = T1 * T2 * ...`).
    pub fn from_fns(fns: &[TransformFn], ctx: &LenCtx) -> Transform2D {
        let mut t = Transform2D::IDENTITY;
        for f in fns {
            match f {
                TransformFn::Translate(x, y) => {
                    let x = x.compute(ctx);
                    let y = y.compute(ctx);
                    let (xp, xpx) = split_pct(&x);
                    let (yp, ypx) = split_pct(&y);
                    t.e += t.a * xpx + t.c * ypx;
                    t.f += t.b * xpx + t.d * ypx;
                    t.ep += t.a * xp + t.c * yp;
                    t.fp += t.b * xp + t.d * yp;
                }
                TransformFn::Scale(sx, sy) => t = t.mul_linear(*sx, 0.0, 0.0, *sy, 0.0, 0.0),
                TransformFn::Rotate(deg) => {
                    let (s, c) = deg.to_radians().sin_cos();
                    t = t.mul_linear(c, s, -s, c, 0.0, 0.0);
                }
                TransformFn::Skew(ax, ay) => {
                    t = t.mul_linear(1.0, ay.to_radians().tan(), ax.to_radians().tan(), 1.0, 0.0, 0.0)
                }
                TransformFn::Matrix(m) => t = t.mul_linear(m[0], m[1], m[2], m[3], m[4], m[5]),
            }
        }
        t
    }

    fn mul_linear(&self, a1: f32, b1: f32, c1: f32, d1: f32, e1: f32, f1: f32) -> Transform2D {
        Transform2D {
            a: self.a * a1 + self.c * b1,
            b: self.b * a1 + self.d * b1,
            c: self.a * c1 + self.c * d1,
            d: self.b * c1 + self.d * d1,
            e: self.a * e1 + self.c * f1 + self.e,
            f: self.b * e1 + self.d * f1 + self.f,
            ep: self.ep,
            fp: self.fp,
        }
    }

    /// The `[a b c d e f]` matrix for a box of `w` x `h` px.
    pub fn resolve(&self, w: f32, h: f32) -> [f32; 6] {
        [self.a, self.b, self.c, self.d, self.e + self.ep * w / 100.0, self.f + self.fp * h / 100.0]
    }

    /// Whether this is the identity.
    pub fn is_identity(&self) -> bool {
        *self == Transform2D::IDENTITY
    }
}

/// Splits a computed length into `(percent, px)` parts.
fn split_pct(l: &Length) -> (f32, f32) {
    match l {
        Length::Px(v) => (0.0, *v),
        Length::Percent(p) => (*p, 0.0),
        // Mixed calc: take the px part at 0 basis and the percent part separately.
        Length::Calc(c) => (c.resolve(100.0) - c.resolve(0.0), c.resolve(0.0)),
        _ => (0.0, 0.0),
    }
}

/// One item of the `content` property.
#[derive(Clone, PartialEq, Debug)]
pub enum ContentItem {
    Str(String),
    /// `attr(name)`.
    Attr(String),
    OpenQuote,
    CloseQuote,
    Url(String),
}

/// The `content` property.
#[derive(Clone, PartialEq, Debug)]
pub enum Content {
    Normal,
    None,
    Items(Vec<ContentItem>),
}

/// A grid track size.
#[derive(Clone, PartialEq, Debug)]
pub enum TrackSize {
    Fr(f32),
    Len(Length),
    MinContent,
    MaxContent,
    FitContent(Length),
    MinMax(Box<TrackSize>, Box<TrackSize>),
}

/// `repeat()` count.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RepeatCount {
    Count(u32),
    AutoFill,
    AutoFit,
}

/// One entry of a track list.
#[derive(Clone, PartialEq, Debug)]
pub enum TrackItem {
    Size(TrackSize),
    Repeat(RepeatCount, Vec<TrackItem>),
    Names(Vec<String>),
}

/// A grid line reference.
#[derive(Clone, PartialEq, Debug)]
pub enum GridLine {
    Auto,
    Line(i32),
    Span(u32),
    Named(String),
}

/// `line-height`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LineHeight {
    Normal,
    /// Multiplier of the element's font size (inherited as a number).
    Number(f32),
    Px(f32),
}

/// `vertical-align`.
#[derive(Clone, PartialEq, Debug)]
pub enum VAlign {
    Keyword(VAlignKw),
    Length(Length),
}

/// `text-decoration-line` flags.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct DecoLine {
    pub underline: bool,
    pub overline: bool,
    pub line_through: bool,
}

/// A parsed, not yet cascaded longhand value.
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Inherit,
    Initial,
    Unset,
    /// A keyword for a `Kw` enum (also `auto`, `bolder`, size keywords).
    Ident(String),
    Number(f32),
    Int(i32),
    Length(Length),
    /// Two lengths (border radius corner, transform origin).
    Pair(Length, Length),
    Color(Color),
    Families(Vec<String>),
    DecoLine(DecoLine),
    Images(Vec<BgImage>),
    Repeats(Vec<(BgRepeatKind, BgRepeatKind)>),
    Lengths(Vec<Length>),
    Sizes(Vec<BgSize>),
    Boxes(Vec<BgBox>),
    Shadows(Vec<Shadow>),
    Content(Content),
    Transform(Vec<TransformFn>),
    Tracks(Vec<TrackItem>),
    TrackSizes(Vec<TrackSize>),
    Areas(Vec<Vec<String>>),
    Line(GridLine),
    Aspect(Option<f32>),
}

/// Parses one keyword of enum `E` and returns it as `Value::Ident`.
pub fn parse_kw<E: Kw>(input: &mut Parser<'_>) -> PResult<Value> {
    let id = input.expect_ident()?.clone();
    match E::from_ident(&id) {
        Some(_) => Ok(Value::Ident(id.to_ascii_lowercase())),
        None => fail(),
    }
}

/// Parses a gradient or `url()` or `none` background image.
pub fn parse_bg_image(input: &mut Parser<'_>) -> PResult<BgImage> {
    let tok = input.next()?.clone();
    match tok {
        Token::Ident(ref s) if s.eq_ignore_ascii_case("none") => Ok(BgImage::None),
        Token::UnquotedUrl(ref u) => Ok(BgImage::Url(u.to_string())),
        Token::Function(ref name) => {
            let n = name.to_ascii_lowercase();
            match n.as_str() {
                "url" | "src" => {
                    let u = input.nested(|i| Ok(i.expect_string()?.to_string()))?;
                    Ok(BgImage::Url(u))
                }
                "linear-gradient" | "repeating-linear-gradient" => {
                    let g = input.nested(|i| parse_linear_gradient(i, n.starts_with("rep")))?;
                    Ok(BgImage::Gradient(g))
                }
                "radial-gradient" | "repeating-radial-gradient" => {
                    let g = input.nested(|i| parse_radial_gradient(i, n.starts_with("rep")))?;
                    Ok(BgImage::Gradient(g))
                }
                _ => fail(),
            }
        }
        _ => fail(),
    }
}

fn parse_stops(i: &mut Parser<'_>) -> PResult<Vec<GradientStop>> {
    let stops = i.comma(|i| {
        let color = parse_color(i)?;
        let p1 = i.try_parse(parse_length_pct).ok();
        let p2 = i.try_parse(parse_length_pct).ok();
        let mut out = vec![GradientStop { color, pos: p1 }];
        if let Some(p2) = p2 {
            out.push(GradientStop { color, pos: Some(p2) });
        }
        Ok(out)
    })?;
    let stops: Vec<GradientStop> = stops.into_iter().flatten().collect();
    if stops.len() < 2 {
        return fail();
    }
    Ok(stops)
}

fn parse_linear_gradient(i: &mut Parser<'_>, repeating: bool) -> PResult<Gradient> {
    let mut angle = 180.0;
    if let Ok(a) = i.try_parse(parse_angle) {
        angle = a;
        i.expect_comma()?;
    } else if i.try_parse(|i| i.expect_ident_matching("to")).is_ok() {
        let mut up = false;
        let mut down = false;
        let mut left = false;
        let mut right = false;
        for _ in 0..2 {
            let id = match i.try_parse(|i| i.expect_ident_cloned()) {
                Ok(id) => id,
                Err(_) => break,
            };
            match id.to_ascii_lowercase().as_str() {
                "top" => up = true,
                "bottom" => down = true,
                "left" => left = true,
                "right" => right = true,
                _ => return fail(),
            }
        }
        angle = match (up, down, left, right) {
            (true, false, false, false) => 0.0,
            (false, false, false, true) => 90.0,
            (false, true, false, false) => 180.0,
            (false, false, true, false) => 270.0,
            (true, false, false, true) => 45.0,
            (false, true, false, true) => 135.0,
            (false, true, true, false) => 225.0,
            (true, false, true, false) => 315.0,
            _ => return fail(),
        };
        i.expect_comma()?;
    }
    let stops = parse_stops(i)?;
    Ok(Gradient { kind: GradientKind::Linear(angle), repeating, stops })
}

fn parse_radial_gradient(i: &mut Parser<'_>, repeating: bool) -> PResult<Gradient> {
    // Skip an optional shape / size / position prefix up to the first comma
    // unless the first item already is a color stop.
    let st = i.state();
    if parse_color(i).is_ok() {
        i.reset(&st);
    } else {
        i.reset(&st);
        loop {
            let t = i.next()?.clone();
            if t == Token::Comma {
                break;
            }
        }
    }
    let stops = parse_stops(i)?;
    Ok(Gradient { kind: GradientKind::Radial, repeating, stops })
}

/// Parses one shadow (`box-shadow` / `text-shadow` item).
pub fn parse_shadow(i: &mut Parser<'_>) -> PResult<Shadow> {
    let mut inset = false;
    let mut color = None;
    let mut lens: Vec<Length> = Vec::new();
    loop {
        if i.try_parse(|i| i.expect_ident_matching("inset")).is_ok() {
            inset = true;
            continue;
        }
        if color.is_none() {
            if let Ok(c) = i.try_parse(parse_color) {
                color = Some(c);
                continue;
            }
        }
        if lens.len() < 4 {
            if let Ok(l) = i.try_parse(parse_length_pct) {
                lens.push(l);
                continue;
            }
        }
        break;
    }
    if lens.len() < 2 {
        return fail();
    }
    let mut it = lens.into_iter();
    let x = it.next().unwrap();
    let y = it.next().unwrap();
    let blur = it.next().unwrap_or(Length::ZERO);
    let spread = it.next().unwrap_or(Length::ZERO);
    Ok(Shadow { x, y, blur, spread, color: color.unwrap_or(Color::Current), inset })
}

/// Parses a `box-shadow` / `text-shadow` list (`none` gives an empty list).
pub fn parse_shadow_list(i: &mut Parser<'_>) -> PResult<Vec<Shadow>> {
    if i.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(Vec::new());
    }
    i.comma(parse_shadow)
}

fn num_or_pct(i: &mut Parser<'_>) -> PResult<f32> {
    let t = i.next()?.clone();
    match t {
        Token::Number { value, .. } => Ok(value),
        Token::Percentage { unit_value, .. } => Ok(unit_value),
        _ => fail(),
    }
}

fn parse_transform_fn(name: &str, i: &mut Parser<'_>) -> PResult<Option<TransformFn>> {
    let l = name.to_ascii_lowercase();
    let f = match l.as_str() {
        "translate" | "translate3d" => {
            let x = parse_length_pct(i)?;
            let y = if i.try_parse(|i| i.expect_comma()).is_ok() { parse_length_pct(i)? } else { Length::ZERO };
            if l == "translate3d" {
                i.expect_comma()?;
                parse_length_pct(i)?;
            }
            TransformFn::Translate(x, y)
        }
        "translatex" => TransformFn::Translate(parse_length_pct(i)?, Length::ZERO),
        "translatey" => TransformFn::Translate(Length::ZERO, parse_length_pct(i)?),
        "translatez" => {
            parse_length_pct(i)?;
            return Ok(None);
        }
        "scale" | "scale3d" => {
            let x = num_or_pct(i)?;
            let y = if i.try_parse(|i| i.expect_comma()).is_ok() { num_or_pct(i)? } else { x };
            if l == "scale3d" {
                i.expect_comma()?;
                num_or_pct(i)?;
            }
            TransformFn::Scale(x, y)
        }
        "scalex" => TransformFn::Scale(num_or_pct(i)?, 1.0),
        "scaley" => TransformFn::Scale(1.0, num_or_pct(i)?),
        "scalez" => {
            num_or_pct(i)?;
            return Ok(None);
        }
        "rotate" | "rotatez" => TransformFn::Rotate(parse_angle(i)?),
        "skew" => {
            let x = parse_angle(i)?;
            let y = if i.try_parse(|i| i.expect_comma()).is_ok() { parse_angle(i)? } else { 0.0 };
            TransformFn::Skew(x, y)
        }
        "skewx" => TransformFn::Skew(parse_angle(i)?, 0.0),
        "skewy" => TransformFn::Skew(0.0, parse_angle(i)?),
        "matrix" => {
            let v = i.comma(parse_number)?;
            if v.len() != 6 {
                return fail();
            }
            TransformFn::Matrix([v[0], v[1], v[2], v[3], v[4], v[5]])
        }
        "matrix3d" => {
            let v = i.comma(parse_number)?;
            if v.len() != 16 {
                return fail();
            }
            TransformFn::Matrix([v[0], v[1], v[4], v[5], v[12], v[13]])
        }
        "perspective" => {
            let _ = i.try_parse(parse_length_pct);
            return Ok(None);
        }
        _ => return fail(),
    };
    Ok(Some(f))
}

/// Parses a `transform` value (`none` gives an empty list).
pub fn parse_transform(input: &mut Parser<'_>) -> PResult<Vec<TransformFn>> {
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    loop {
        if input.is_exhausted() {
            break;
        }
        let name = input.expect_function()?.clone();
        let f = input.nested(|i| parse_transform_fn(&name, i))?;
        if let Some(f) = f {
            out.push(f);
        }
    }
    if out.is_empty() && !input.is_exhausted() {
        return fail();
    }
    Ok(out)
}

/// Parses `content`.
pub fn parse_content(input: &mut Parser<'_>) -> PResult<Content> {
    if input.try_parse(|i| i.expect_ident_matching("normal")).is_ok() {
        return Ok(Content::Normal);
    }
    if input.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(Content::None);
    }
    let mut items = Vec::new();
    loop {
        if input.is_exhausted() {
            break;
        }
        let tok = input.next()?.clone();
        match tok {
            Token::QuotedString(ref s) => items.push(ContentItem::Str(s.to_string())),
            Token::UnquotedUrl(ref u) => items.push(ContentItem::Url(u.to_string())),
            Token::Ident(ref s) => match s.to_ascii_lowercase().as_str() {
                "open-quote" => items.push(ContentItem::OpenQuote),
                "close-quote" => items.push(ContentItem::CloseQuote),
                "no-open-quote" | "no-close-quote" => {}
                _ => return fail(),
            },
            Token::Function(ref name) => {
                let n = name.to_ascii_lowercase();
                match n.as_str() {
                    "attr" => {
                        let a = input.nested(|i| Ok(i.expect_ident()?.to_string()))?;
                        items.push(ContentItem::Attr(a));
                    }
                    "url" => {
                        let u = input.nested(|i| Ok(i.expect_string()?.to_string()))?;
                        items.push(ContentItem::Url(u));
                    }
                    "counter" | "counters" => {
                        input.nested(|i| {
                            while i.next().is_ok() {}
                            Ok(())
                        })?;
                    }
                    _ => return fail(),
                }
            }
            _ => return fail(),
        }
    }
    Ok(Content::Items(items))
}

fn parse_track_size(i: &mut Parser<'_>) -> PResult<TrackSize> {
    let st = i.state();
    let t = i.next()?.clone();
    match t {
        Token::Dimension { value, ref unit, .. } if unit.eq_ignore_ascii_case("fr") => Ok(TrackSize::Fr(value)),
        Token::Ident(ref s) => match s.to_ascii_lowercase().as_str() {
            "auto" => Ok(TrackSize::Len(Length::Auto)),
            "min-content" => Ok(TrackSize::MinContent),
            "max-content" => Ok(TrackSize::MaxContent),
            _ => fail(),
        },
        Token::Function(ref name) if name.eq_ignore_ascii_case("minmax") => {
            let (a, b) = i.nested(|i| {
                let a = parse_track_size(i)?;
                i.expect_comma()?;
                let b = parse_track_size(i)?;
                Ok((a, b))
            })?;
            Ok(TrackSize::MinMax(Box::new(a), Box::new(b)))
        }
        Token::Function(ref name) if name.eq_ignore_ascii_case("fit-content") => {
            let l = i.nested(parse_length_pct)?;
            Ok(TrackSize::FitContent(l))
        }
        _ => {
            i.reset(&st);
            Ok(TrackSize::Len(parse_length_pct(i)?))
        }
    }
}

/// Parses a track list (`grid-template-columns` value). `none` gives empty.
pub fn parse_track_list(i: &mut Parser<'_>) -> PResult<Vec<TrackItem>> {
    if i.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    loop {
        if i.is_exhausted() {
            break;
        }
        let st = i.state();
        match parse_track_item(i) {
            Ok(item) => out.push(item),
            Err(_) => {
                i.reset(&st);
                break;
            }
        }
    }
    if out.is_empty() {
        return fail();
    }
    Ok(out)
}

fn parse_track_item(i: &mut Parser<'_>) -> PResult<TrackItem> {
    let st = i.state();
    let t = i.next()?.clone();
    match t {
        Token::SquareBracketBlock => {
            let names = i.nested(|i| {
                let mut v = Vec::new();
                while let Ok(id) = i.try_parse(|i| i.expect_ident_cloned()) {
                    v.push(id.to_string());
                }
                Ok(v)
            })?;
            Ok(TrackItem::Names(names))
        }
        Token::Function(ref name) if name.eq_ignore_ascii_case("repeat") => {
            let (count, items) = i.nested(|i| {
                let c = i.next()?.clone();
                let count = match c {
                    Token::Number { int_value: Some(n), .. } if n > 0 => RepeatCount::Count(n as u32),
                    Token::Ident(ref s) if s.eq_ignore_ascii_case("auto-fill") => RepeatCount::AutoFill,
                    Token::Ident(ref s) if s.eq_ignore_ascii_case("auto-fit") => RepeatCount::AutoFit,
                    _ => return fail(),
                };
                i.expect_comma()?;
                let mut items = Vec::new();
                loop {
                    if i.is_exhausted() {
                        break;
                    }
                    items.push(parse_track_item(i)?);
                }
                if items.is_empty() {
                    return fail();
                }
                Ok((count, items))
            })?;
            Ok(TrackItem::Repeat(count, items))
        }
        _ => {
            i.reset(&st);
            Ok(TrackItem::Size(parse_track_size(i)?))
        }
    }
}

/// Parses a list of track sizes (`grid-auto-rows`).
pub fn parse_track_sizes(i: &mut Parser<'_>) -> PResult<Vec<TrackSize>> {
    let mut out = Vec::new();
    loop {
        if i.is_exhausted() {
            break;
        }
        out.push(parse_track_size(i)?);
    }
    if out.is_empty() {
        return fail();
    }
    Ok(out)
}

/// Parses a single grid line (`grid-column-start` etc.).
pub fn parse_grid_line(i: &mut Parser<'_>) -> PResult<GridLine> {
    if i.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
        return Ok(GridLine::Auto);
    }
    if i.try_parse(|i| i.expect_ident_matching("span")).is_ok() {
        if let Ok(n) = i.try_parse(|i| i.expect_integer()) {
            return if n > 0 { Ok(GridLine::Span(n as u32)) } else { fail() };
        }
        // `span <ident>` is treated as span 1.
        i.expect_ident()?;
        return Ok(GridLine::Span(1));
    }
    if let Ok(n) = i.try_parse(|i| i.expect_integer()) {
        if n == 0 {
            return fail();
        }
        return Ok(GridLine::Line(n));
    }
    let id = i.expect_ident()?.to_string();
    Ok(GridLine::Named(id))
}

/// Parses `font-family`.
pub fn parse_font_family(i: &mut Parser<'_>) -> PResult<Vec<String>> {
    i.comma(|i| {
        let t = i.next()?.clone();
        match t {
            Token::QuotedString(ref s) => Ok(s.to_string()),
            Token::Ident(ref first) => {
                let mut name = first.to_string();
                while let Ok(id) = i.try_parse(|i| i.expect_ident_cloned()) {
                    name.push(' ');
                    name.push_str(&id);
                }
                Ok(name)
            }
            _ => fail(),
        }
    })
}

/// Parses `<position>` (1 to 4 values) into `(x, y)` lengths where
/// percentages are positions in the box.
pub fn parse_position(i: &mut Parser<'_>) -> PResult<(Length, Length)> {
    #[derive(Clone, PartialEq)]
    enum P {
        L,
        R,
        T,
        B,
        C,
        Len(Length),
    }
    let mut items: Vec<P> = Vec::new();
    while items.len() < 4 {
        let st = i.state();
        let t = match i.next() {
            Ok(t) => t.clone(),
            Err(_) => break,
        };
        let p = match t {
            Token::Ident(ref s) => match s.to_ascii_lowercase().as_str() {
                "left" => P::L,
                "right" => P::R,
                "top" => P::T,
                "bottom" => P::B,
                "center" => P::C,
                _ => {
                    i.reset(&st);
                    break;
                }
            },
            _ => {
                i.reset(&st);
                match parse_length_pct(i) {
                    Ok(l) => P::Len(l),
                    Err(_) => {
                        i.reset(&st);
                        break;
                    }
                }
            }
        };
        items.push(p);
    }
    let pct = |v: f32| Length::Dim(v, Unit::Percent);
    let kw_pos = |p: &P| -> Option<f32> {
        match p {
            P::L | P::T => Some(0.0),
            P::R | P::B => Some(100.0),
            P::C => Some(50.0),
            P::Len(_) => None,
        }
    };
    match items.len() {
        1 => match &items[0] {
            P::Len(l) => Ok((l.clone(), pct(50.0))),
            P::T | P::B => Ok((pct(50.0), pct(kw_pos(&items[0]).unwrap()))),
            p => Ok((pct(kw_pos(p).unwrap()), pct(50.0))),
        },
        2 => {
            let (a, b) = (&items[0], &items[1]);
            let vertical_first = matches!(a, P::T | P::B) || matches!(b, P::L | P::R);
            let (x, y) = if vertical_first { (b, a) } else { (a, b) };
            let conv = |p: &P| match p {
                P::Len(l) => l.clone(),
                other => pct(kw_pos(other).unwrap()),
            };
            if matches!(x, P::T | P::B) || matches!(y, P::L | P::R) {
                return fail();
            }
            Ok((conv(x), conv(y)))
        }
        3 | 4 => {
            // keyword [offset] keyword [offset]
            let mut x: Option<Length> = None;
            let mut y: Option<Length> = None;
            let mut idx = 0;
            while idx < items.len() {
                let key = items[idx].clone();
                let offset = match items.get(idx + 1) {
                    Some(P::Len(l)) => {
                        idx += 1;
                        Some(l.clone())
                    }
                    _ => None,
                };
                idx += 1;
                let base = match kw_pos(&key) {
                    Some(b) => b,
                    None => return fail(),
                };
                let from_end = matches!(key, P::R | P::B);
                let mk = |off: Option<Length>| -> Length {
                    match off {
                        None => pct(base),
                        Some(l) if !from_end => match l {
                            Length::Dim(v, Unit::Percent) => pct(base + v),
                            other => Length::Calc(Box::new(Calc::Add(
                                Box::new(Calc::Dim(base, Unit::Percent)),
                                Box::new(length_to_calc(&other)),
                            ))),
                        },
                        Some(l) => Length::Calc(Box::new(Calc::Sub(
                            Box::new(Calc::Dim(100.0, Unit::Percent)),
                            Box::new(length_to_calc(&l)),
                        ))),
                    }
                };
                match key {
                    P::L | P::R => x = Some(mk(offset)),
                    P::T | P::B => y = Some(mk(offset)),
                    P::C => {
                        if x.is_none() {
                            x = Some(pct(50.0))
                        } else {
                            y = Some(pct(50.0))
                        }
                    }
                    P::Len(_) => return fail(),
                }
            }
            Ok((x.unwrap_or_else(|| pct(50.0)), y.unwrap_or_else(|| pct(50.0))))
        }
        _ => fail(),
    }
}

fn length_to_calc(l: &Length) -> Calc {
    match l {
        Length::Dim(v, u) => Calc::Dim(*v, *u),
        Length::Px(v) => Calc::Dim(*v, Unit::Px),
        Length::Percent(p) => Calc::Dim(*p, Unit::Percent),
        Length::Calc(c) => (**c).clone(),
        _ => Calc::Num(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p<T>(src: &str, f: impl FnOnce(&mut Parser<'_>) -> PResult<T>) -> Option<T> {
        let mut input = Parser::new(src);
        input.parse_entirely(f).ok()
    }

    fn color(src: &str) -> Option<Color> {
        p(src, parse_color)
    }

    fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
        Color::Rgba(Rgba::from_u8(r, g, b, a))
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn hex_colors() {
        assert_eq!(color("#fff"), Some(rgba(255, 255, 255, 1.0)));
        assert_eq!(color("#ff0000"), Some(rgba(255, 0, 0, 1.0)));
        assert_eq!(color("#00ff0080"), Some(rgba(0, 255, 0, 128.0 / 255.0)));
        assert_eq!(color("#ggg"), None);
    }

    #[test]
    fn named_and_keyword_colors() {
        assert_eq!(color("red"), Some(rgba(255, 0, 0, 1.0)));
        assert_eq!(color("RebeccaPurple"), Some(rgba(102, 51, 153, 1.0)));
        assert_eq!(color("transparent"), Some(Color::Rgba(Rgba::TRANSPARENT)));
        assert_eq!(color("currentColor"), Some(Color::Current));
        assert_eq!(color("notacolor"), None);
    }

    #[test]
    fn rgb_functions() {
        assert_eq!(color("rgb(255, 0, 0)"), Some(rgba(255, 0, 0, 1.0)));
        assert_eq!(color("rgb(0 128 255)"), Some(rgba(0, 128, 255, 1.0)));
        assert_eq!(color("rgba(0, 0, 0, 0.5)"), Some(rgba(0, 0, 0, 0.5)));
        assert_eq!(color("rgb(0 0 0 / 50%)"), Some(rgba(0, 0, 0, 0.5)));
        assert_eq!(color("rgb(100%, 0%, 0%)"), Some(rgba(255, 0, 0, 1.0)));
    }

    #[test]
    fn hsl_functions() {
        let c = match color("hsl(120, 100%, 50%)") {
            Some(Color::Rgba(c)) => c,
            _ => panic!(),
        };
        assert!(near(c.r, 0.0) && near(c.g, 1.0) && near(c.b, 0.0));
        let c = match color("hsl(0 0% 100% / 0.5)") {
            Some(Color::Rgba(c)) => c,
            _ => panic!(),
        };
        assert!(near(c.r, 1.0) && near(c.a, 0.5));
        let c = match color("hsla(240deg, 100%, 50%, 1)") {
            Some(Color::Rgba(c)) => c,
            _ => panic!(),
        };
        assert!(near(c.b, 1.0) && near(c.r, 0.0));
    }

    #[test]
    fn color_mix_basic() {
        let c = match color("color-mix(in srgb, red 50%, blue)") {
            Some(Color::Rgba(c)) => c,
            _ => panic!(),
        };
        assert!(near(c.r, 0.5) && near(c.b, 0.5) && near(c.g, 0.0));
        let c = match color("color-mix(in srgb, white, black)") {
            Some(Color::Rgba(c)) => c,
            _ => panic!(),
        };
        assert!(near(c.r, 0.5));
        let c = match color("color-mix(in srgb, red 25%, blue 75%)") {
            Some(Color::Rgba(c)) => c,
            _ => panic!(),
        };
        assert!(near(c.r, 0.25) && near(c.b, 0.75));
    }

    #[test]
    fn rgba_packing_and_mix() {
        assert_eq!(Rgba::from_u8(255, 0, 128, 1.0).to_argb(), 0xFFFF0080);
        let m = Rgba::BLACK.mix(&Rgba::WHITE, 0.5);
        assert!(near(m.r, 0.5));
    }

    fn len(src: &str) -> Option<Length> {
        p(src, parse_length_pct)
    }

    #[test]
    fn lengths_and_units() {
        assert_eq!(len("10px"), Some(Length::Dim(10.0, Unit::Px)));
        assert_eq!(len("0"), Some(Length::Px(0.0)));
        assert_eq!(len("50%"), Some(Length::Dim(50.0, Unit::Percent)));
        assert_eq!(len("1.5em"), Some(Length::Dim(1.5, Unit::Em)));
        assert_eq!(len("10"), None);
        assert_eq!(len("10foo"), None);
    }

    fn ctx() -> LenCtx {
        LenCtx { font_size: 20.0, root_font_size: 10.0, vw: 800.0, vh: 600.0 }
    }

    #[test]
    fn unit_resolution() {
        let c = ctx();
        let f = |s: &str| len(s).unwrap().compute(&c);
        assert_eq!(f("2em"), Length::Px(40.0));
        assert_eq!(f("2rem"), Length::Px(20.0));
        assert_eq!(f("10vw"), Length::Px(80.0));
        assert_eq!(f("10vh"), Length::Px(60.0));
        assert_eq!(f("10vmin"), Length::Px(60.0));
        assert_eq!(f("10vmax"), Length::Px(80.0));
        assert_eq!(f("4ch"), Length::Px(40.0));
        assert_eq!(f("72pt"), Length::Px(96.0));
        assert_eq!(f("1in"), Length::Px(96.0));
        assert_eq!(f("25%"), Length::Percent(25.0));
        match f("2.54cm") {
            Length::Px(v) => assert!(near(v, 96.0)),
            _ => panic!(),
        }
    }

    #[test]
    fn calc_expressions() {
        let c = ctx();
        let f = |s: &str| len(s).unwrap().compute(&c);
        assert_eq!(f("calc(10px + 2em)"), Length::Px(50.0));
        assert_eq!(f("calc(2 * (3px + 1px))"), Length::Px(8.0));
        assert_eq!(f("calc(100px / 4)"), Length::Px(25.0));
        assert_eq!(f("min(10px, 5px)"), Length::Px(5.0));
        assert_eq!(f("max(10px, 5px, 20px)"), Length::Px(20.0));
        assert_eq!(f("clamp(10px, 5px, 20px)"), Length::Px(10.0));
        assert_eq!(f("clamp(10px, 50px, 20px)"), Length::Px(20.0));
        assert_eq!(f("clamp(10px, 15px, 20px)"), Length::Px(15.0));
    }

    #[test]
    fn calc_with_percent_stays_resolvable() {
        let c = ctx();
        let l = len("calc(100% - 20px)").unwrap().compute(&c);
        assert!(matches!(l, Length::Calc(_)));
        assert_eq!(l.resolve(Some(200.0)), Some(180.0));
        assert_eq!(l.resolve(None), None);
        let m = len("min(50%, 300px)").unwrap().compute(&c);
        assert_eq!(m.resolve(Some(400.0)), Some(200.0));
        assert_eq!(m.resolve(Some(1000.0)), Some(300.0));
    }

    #[test]
    fn calc_rejects_garbage() {
        assert_eq!(len("calc(10px +)"), None);
        assert_eq!(len("calc()"), None);
        assert_eq!(len("clamp(1px, 2px)"), None);
    }

    #[test]
    fn length_keywords() {
        assert_eq!(p("auto", |i| parse_length_kw(i, LenKw::AUTO)), Some(Length::Auto));
        assert_eq!(p("auto", parse_length_pct), None);
        assert_eq!(p("none", |i| parse_length_kw(i, LenKw::MAX)), Some(Length::None));
        assert_eq!(p("fit-content", |i| parse_length_kw(i, LenKw::SIZE)), Some(Length::FitContent));
    }

    #[test]
    fn keyword_enums_parse() {
        assert_eq!(Display::from_ident("FLEX"), Some(Display::Flex));
        assert_eq!(Display::from_ident("inline-block"), Some(Display::InlineBlock));
        assert_eq!(Display::from_ident("bogus"), None);
        assert_eq!(ListStyleType::from_ident("lower-latin"), Some(ListStyleType::LowerAlpha));
        assert_eq!(Display::default(), Display::Inline);
        assert_eq!(Display::Grid.keyword(), "grid");
    }

    #[test]
    fn shadows() {
        let s = p("2px 4px 6px rgba(0,0,0,.5)", parse_shadow).unwrap();
        assert_eq!(s.x, Length::Dim(2.0, Unit::Px));
        assert_eq!(s.blur, Length::Dim(6.0, Unit::Px));
        assert!(!s.inset);
        let s = p("inset 0 0 0 1px red", parse_shadow).unwrap();
        assert!(s.inset);
        assert_eq!(s.spread, Length::Dim(1.0, Unit::Px));
        let l = p("0 1px red, 0 2px blue", parse_shadow_list).unwrap();
        assert_eq!(l.len(), 2);
        assert!(p("none", parse_shadow_list).unwrap().is_empty());
        assert!(p("1px", parse_shadow).is_none());
    }

    #[test]
    fn gradients() {
        match p("linear-gradient(to right, red, blue)", parse_bg_image).unwrap() {
            BgImage::Gradient(g) => {
                assert_eq!(g.kind, GradientKind::Linear(90.0));
                assert_eq!(g.stops.len(), 2);
            }
            _ => panic!(),
        }
        match p("linear-gradient(45deg, red 0%, blue 100%)", parse_bg_image).unwrap() {
            BgImage::Gradient(g) => assert_eq!(g.kind, GradientKind::Linear(45.0)),
            _ => panic!(),
        }
        match p("linear-gradient(red, blue)", parse_bg_image).unwrap() {
            BgImage::Gradient(g) => assert_eq!(g.kind, GradientKind::Linear(180.0)),
            _ => panic!(),
        }
        assert!(matches!(
            p("radial-gradient(circle at center, red, blue)", parse_bg_image),
            Some(BgImage::Gradient(_))
        ));
        assert_eq!(p("url(a.png)", parse_bg_image), Some(BgImage::Url("a.png".into())));
        assert_eq!(p("url('b.png')", parse_bg_image), Some(BgImage::Url("b.png".into())));
        assert_eq!(p("none", parse_bg_image), Some(BgImage::None));
    }

    #[test]
    fn transforms_compose() {
        let fns = p("translate(10px, 20px) scale(2)", parse_transform).unwrap();
        let t = Transform2D::from_fns(&fns, &ctx());
        assert_eq!(t.resolve(100.0, 100.0), [2.0, 0.0, 0.0, 2.0, 10.0, 20.0]);
        let fns = p("scale(2) translate(10px, 0)", parse_transform).unwrap();
        let t = Transform2D::from_fns(&fns, &ctx());
        assert_eq!(t.resolve(100.0, 100.0), [2.0, 0.0, 0.0, 2.0, 20.0, 0.0]);
    }

    #[test]
    fn transform_rotate_and_percent() {
        let fns = p("rotate(90deg)", parse_transform).unwrap();
        let t = Transform2D::from_fns(&fns, &ctx()).resolve(10.0, 10.0);
        assert!(near(t[0], 0.0) && near(t[1], 1.0) && near(t[2], -1.0) && near(t[3], 0.0));
        let fns = p("translate(-50%, 10px)", parse_transform).unwrap();
        let t = Transform2D::from_fns(&fns, &ctx()).resolve(200.0, 100.0);
        assert!(near(t[4], -100.0) && near(t[5], 10.0));
        let fns = p("matrix(1, 0, 0, 1, 5, 6)", parse_transform).unwrap();
        assert_eq!(Transform2D::from_fns(&fns, &ctx()).resolve(1.0, 1.0), [1.0, 0.0, 0.0, 1.0, 5.0, 6.0]);
        assert!(p("none", parse_transform).unwrap().is_empty());
        assert!(p("bogus(1)", parse_transform).is_none());
        assert!(Transform2D::IDENTITY.is_identity());
    }

    #[test]
    fn content_values() {
        assert_eq!(p("none", parse_content), Some(Content::None));
        assert_eq!(p("normal", parse_content), Some(Content::Normal));
        assert_eq!(
            p("\"a\" attr(title) open-quote", parse_content),
            Some(Content::Items(vec![
                ContentItem::Str("a".into()),
                ContentItem::Attr("title".into()),
                ContentItem::OpenQuote
            ]))
        );
        assert_eq!(p("\"\"", parse_content), Some(Content::Items(vec![ContentItem::Str(String::new())])));
    }

    #[test]
    fn grid_tracks() {
        let t = p("1fr 100px auto", parse_track_list).unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0], TrackItem::Size(TrackSize::Fr(1.0)));
        let t = p("repeat(3, 1fr)", parse_track_list).unwrap();
        assert_eq!(t, vec![TrackItem::Repeat(RepeatCount::Count(3), vec![TrackItem::Size(TrackSize::Fr(1.0))])]);
        let t = p("repeat(auto-fill, minmax(100px, 1fr))", parse_track_list).unwrap();
        assert!(matches!(&t[0], TrackItem::Repeat(RepeatCount::AutoFill, _)));
        let t = p("[a] 1fr [b c] 2fr", parse_track_list).unwrap();
        assert_eq!(t.len(), 4);
        assert!(p("none", parse_track_list).unwrap().is_empty());
    }

    #[test]
    fn grid_lines() {
        assert_eq!(p("auto", parse_grid_line), Some(GridLine::Auto));
        assert_eq!(p("3", parse_grid_line), Some(GridLine::Line(3)));
        assert_eq!(p("-1", parse_grid_line), Some(GridLine::Line(-1)));
        assert_eq!(p("span 2", parse_grid_line), Some(GridLine::Span(2)));
        assert_eq!(p("main", parse_grid_line), Some(GridLine::Named("main".into())));
        assert_eq!(p("0", parse_grid_line), None);
    }

    #[test]
    fn font_families() {
        assert_eq!(
            p("\"Helvetica Neue\", Arial, sans-serif", parse_font_family),
            Some(vec!["Helvetica Neue".into(), "Arial".into(), "sans-serif".into()])
        );
        assert_eq!(p("Times New Roman", parse_font_family), Some(vec!["Times New Roman".into()]));
    }

    #[test]
    fn positions() {
        let pct = |v: f32| Length::Dim(v, Unit::Percent);
        assert_eq!(p("center", parse_position), Some((pct(50.0), pct(50.0))));
        assert_eq!(p("left top", parse_position), Some((pct(0.0), pct(0.0))));
        assert_eq!(p("top left", parse_position), Some((pct(0.0), pct(0.0))));
        assert_eq!(p("right 10px", parse_position), Some((pct(100.0), Length::Dim(10.0, Unit::Px))));
        assert_eq!(p("20px", parse_position), Some((Length::Dim(20.0, Unit::Px), pct(50.0))));
        assert_eq!(p("bottom", parse_position), Some((pct(50.0), pct(100.0))));
        assert_eq!(p("left 50%", parse_position), Some((pct(0.0), pct(50.0))));
    }

    #[test]
    fn angles() {
        assert_eq!(p("90deg", parse_angle), Some(90.0));
        assert_eq!(p("0.5turn", parse_angle), Some(180.0));
        assert_eq!(p("0", parse_angle), Some(0.0));
        assert!(near(p("3.14159265rad", parse_angle).unwrap(), 180.0));
        assert_eq!(p("10px", parse_angle), None);
    }
}
