//! The property table (`Prop`) and the `Computed` style: every longhand as a
//! resolved value. `Computed::apply` turns a parsed `Value` into the field of
//! one property, resolving relative units, `currentcolor` and the CSS-wide
//! keywords (`inherit`, `initial`, `unset`).

use super::values::*;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Custom properties (`--name`), inherited as a shared map.
pub type CustomProps = Arc<BTreeMap<String, String>>;

/// The resolved style of one element or generated box.
#[derive(Clone, Debug, PartialEq)]
pub struct Computed {
    pub display: Display,
    pub position: Position,
    pub float: Float,
    pub clear: Clear,
    pub box_sizing: BoxSizing,
    pub visibility: Visibility,
    pub overflow_x: Overflow,
    pub overflow_y: Overflow,
    /// `None` is `auto`.
    pub z_index: Option<i32>,
    pub opacity: f32,

    pub width: Length,
    pub height: Length,
    pub min_width: Length,
    pub min_height: Length,
    /// `Length::None` when unbounded.
    pub max_width: Length,
    pub max_height: Length,
    pub margin_top: Length,
    pub margin_right: Length,
    pub margin_bottom: Length,
    pub margin_left: Length,
    pub padding_top: Length,
    pub padding_right: Length,
    pub padding_bottom: Length,
    pub padding_left: Length,
    pub top: Length,
    pub right: Length,
    pub bottom: Length,
    pub left: Length,

    /// Border widths in px (0 when the style is none or hidden).
    pub border_top_width: f32,
    pub border_right_width: f32,
    pub border_bottom_width: f32,
    pub border_left_width: f32,
    pub border_top_style: BorderStyle,
    pub border_right_style: BorderStyle,
    pub border_bottom_style: BorderStyle,
    pub border_left_style: BorderStyle,
    pub border_top_color: Rgba,
    pub border_right_color: Rgba,
    pub border_bottom_color: Rgba,
    pub border_left_color: Rgba,
    /// Horizontal and vertical radius of each corner.
    pub border_top_left_radius: [Length; 2],
    pub border_top_right_radius: [Length; 2],
    pub border_bottom_right_radius: [Length; 2],
    pub border_bottom_left_radius: [Length; 2],

    pub color: Rgba,
    pub background_color: Rgba,
    pub background_image: Vec<BgImage>,
    pub background_repeat: Vec<(BgRepeatKind, BgRepeatKind)>,
    pub background_position_x: Vec<Length>,
    pub background_position_y: Vec<Length>,
    pub background_size: Vec<BgSize>,
    pub background_clip: Vec<BgBox>,
    pub background_origin: Vec<BgBox>,

    pub font_family: Vec<String>,
    /// Font size in px.
    pub font_size: f32,
    pub font_weight: f32,
    pub font_style: FontStyle,
    pub line_height: LineHeight,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub text_align: TextAlign,
    pub text_decoration_line: DecoLine,
    pub text_decoration_color: Rgba,
    pub text_transform: TextTransform,
    pub white_space: WhiteSpace,
    pub text_indent: Length,
    pub vertical_align: VAlign,
    pub text_overflow: TextOverflow,
    pub word_break: WordBreak,
    pub overflow_wrap: OverflowWrap,
    pub list_style_type: ListStyleType,
    pub text_shadow: Vec<Shadow>,
    pub box_shadow: Vec<Shadow>,
    pub content: Content,
    pub transform: Option<Transform2D>,
    pub transform_origin: [Length; 2],
    pub aspect_ratio: Option<f32>,

    pub flex_direction: FlexDirection,
    pub flex_wrap: FlexWrap,
    pub flex_grow: f32,
    pub flex_shrink: f32,
    pub flex_basis: Length,
    pub justify_content: Align,
    pub align_items: Align,
    pub align_self: Align,
    pub align_content: Align,
    pub justify_items: Align,
    pub justify_self: Align,
    pub order: i32,
    pub row_gap: Length,
    pub column_gap: Length,

    pub grid_template_columns: Vec<TrackItem>,
    pub grid_template_rows: Vec<TrackItem>,
    pub grid_template_areas: Vec<Vec<String>>,
    pub grid_auto_flow: GridAutoFlow,
    pub grid_auto_columns: Vec<TrackSize>,
    pub grid_auto_rows: Vec<TrackSize>,
    pub grid_column_start: GridLine,
    pub grid_column_end: GridLine,
    pub grid_row_start: GridLine,
    pub grid_row_end: GridLine,

    /// Resolved custom properties (shared with the parent when unchanged).
    pub custom: CustomProps,
}

impl Default for Computed {
    fn default() -> Computed {
        let pct0 = || Length::Percent(0.0);
        Computed {
            display: Display::Inline,
            position: Position::Static,
            float: Float::None,
            clear: Clear::None,
            box_sizing: BoxSizing::ContentBox,
            visibility: Visibility::Visible,
            overflow_x: Overflow::Visible,
            overflow_y: Overflow::Visible,
            z_index: None,
            opacity: 1.0,
            width: Length::Auto,
            height: Length::Auto,
            min_width: Length::Auto,
            min_height: Length::Auto,
            max_width: Length::None,
            max_height: Length::None,
            margin_top: Length::ZERO,
            margin_right: Length::ZERO,
            margin_bottom: Length::ZERO,
            margin_left: Length::ZERO,
            padding_top: Length::ZERO,
            padding_right: Length::ZERO,
            padding_bottom: Length::ZERO,
            padding_left: Length::ZERO,
            top: Length::Auto,
            right: Length::Auto,
            bottom: Length::Auto,
            left: Length::Auto,
            border_top_width: 3.0,
            border_right_width: 3.0,
            border_bottom_width: 3.0,
            border_left_width: 3.0,
            border_top_style: BorderStyle::None,
            border_right_style: BorderStyle::None,
            border_bottom_style: BorderStyle::None,
            border_left_style: BorderStyle::None,
            border_top_color: Rgba::BLACK,
            border_right_color: Rgba::BLACK,
            border_bottom_color: Rgba::BLACK,
            border_left_color: Rgba::BLACK,
            border_top_left_radius: [Length::ZERO, Length::ZERO],
            border_top_right_radius: [Length::ZERO, Length::ZERO],
            border_bottom_right_radius: [Length::ZERO, Length::ZERO],
            border_bottom_left_radius: [Length::ZERO, Length::ZERO],
            color: Rgba::BLACK,
            background_color: Rgba::TRANSPARENT,
            background_image: vec![BgImage::None],
            background_repeat: vec![(BgRepeatKind::Repeat, BgRepeatKind::Repeat)],
            background_position_x: vec![pct0()],
            background_position_y: vec![pct0()],
            background_size: vec![BgSize::Explicit(Length::Auto, Length::Auto)],
            background_clip: vec![BgBox::BorderBox],
            background_origin: vec![BgBox::PaddingBox],
            font_family: vec!["serif".to_string()],
            font_size: 16.0,
            font_weight: 400.0,
            font_style: FontStyle::Normal,
            line_height: LineHeight::Normal,
            letter_spacing: 0.0,
            word_spacing: 0.0,
            text_align: TextAlign::Start,
            text_decoration_line: DecoLine::default(),
            text_decoration_color: Rgba::BLACK,
            text_transform: TextTransform::None,
            white_space: WhiteSpace::Normal,
            text_indent: Length::ZERO,
            vertical_align: VAlign::Keyword(VAlignKw::Baseline),
            text_overflow: TextOverflow::Clip,
            word_break: WordBreak::Normal,
            overflow_wrap: OverflowWrap::Normal,
            list_style_type: ListStyleType::Disc,
            text_shadow: Vec::new(),
            box_shadow: Vec::new(),
            content: Content::Normal,
            transform: None,
            transform_origin: [Length::Percent(50.0), Length::Percent(50.0)],
            aspect_ratio: None,
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Nowrap,
            flex_grow: 0.0,
            flex_shrink: 1.0,
            flex_basis: Length::Auto,
            justify_content: Align::Normal,
            align_items: Align::Normal,
            align_self: Align::Auto,
            align_content: Align::Normal,
            justify_items: Align::Normal,
            justify_self: Align::Auto,
            order: 0,
            row_gap: Length::ZERO,
            column_gap: Length::ZERO,
            grid_template_columns: Vec::new(),
            grid_template_rows: Vec::new(),
            grid_template_areas: Vec::new(),
            grid_auto_flow: GridAutoFlow::Row,
            grid_auto_columns: Vec::new(),
            grid_auto_rows: Vec::new(),
            grid_column_start: GridLine::Auto,
            grid_column_end: GridLine::Auto,
            grid_row_start: GridLine::Auto,
            grid_row_end: GridLine::Auto,
            custom: Arc::new(BTreeMap::new()),
        }
    }
}

macro_rules! props {
    ($($v:ident, $name:literal, $field:ident, $inh:literal;)*) => {
        /// A longhand property.
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Prop { $($v),* }

        impl Prop {
            /// Every longhand.
            pub const ALL: &'static [Prop] = &[$(Prop::$v),*];

            /// Looks a longhand up by its (lowercase) CSS name.
            pub fn from_name(n: &str) -> Option<Prop> {
                match n { $($name => Some(Prop::$v),)* _ => None }
            }

            /// The CSS name.
            pub fn name(self) -> &'static str {
                match self { $(Prop::$v => $name,)* }
            }

            /// Whether the property inherits by default.
            pub fn inherited(self) -> bool {
                match self { $(Prop::$v => $inh,)* }
            }
        }

        impl Computed {
            /// Copies one property from `src`.
            pub fn copy_prop(&mut self, prop: Prop, src: &Computed) {
                match prop { $(Prop::$v => self.$field = src.$field.clone(),)* }
            }

            /// A fresh style that inherits the inherited properties of `parent`.
            pub fn inherit_from(parent: &Computed) -> Computed {
                let mut c = Computed::default();
                $( if $inh { c.$field = parent.$field.clone(); } )*
                c.custom = parent.custom.clone();
                c
            }
        }
    };
}

props! {
    Display, "display", display, false;
    Position, "position", position, false;
    Float, "float", float, false;
    Clear, "clear", clear, false;
    BoxSizing, "box-sizing", box_sizing, false;
    Visibility, "visibility", visibility, true;
    OverflowX, "overflow-x", overflow_x, false;
    OverflowY, "overflow-y", overflow_y, false;
    ZIndex, "z-index", z_index, false;
    Opacity, "opacity", opacity, false;
    Width, "width", width, false;
    Height, "height", height, false;
    MinWidth, "min-width", min_width, false;
    MinHeight, "min-height", min_height, false;
    MaxWidth, "max-width", max_width, false;
    MaxHeight, "max-height", max_height, false;
    MarginTop, "margin-top", margin_top, false;
    MarginRight, "margin-right", margin_right, false;
    MarginBottom, "margin-bottom", margin_bottom, false;
    MarginLeft, "margin-left", margin_left, false;
    PaddingTop, "padding-top", padding_top, false;
    PaddingRight, "padding-right", padding_right, false;
    PaddingBottom, "padding-bottom", padding_bottom, false;
    PaddingLeft, "padding-left", padding_left, false;
    Top, "top", top, false;
    Right, "right", right, false;
    Bottom, "bottom", bottom, false;
    Left, "left", left, false;
    BorderTopWidth, "border-top-width", border_top_width, false;
    BorderRightWidth, "border-right-width", border_right_width, false;
    BorderBottomWidth, "border-bottom-width", border_bottom_width, false;
    BorderLeftWidth, "border-left-width", border_left_width, false;
    BorderTopStyle, "border-top-style", border_top_style, false;
    BorderRightStyle, "border-right-style", border_right_style, false;
    BorderBottomStyle, "border-bottom-style", border_bottom_style, false;
    BorderLeftStyle, "border-left-style", border_left_style, false;
    BorderTopColor, "border-top-color", border_top_color, false;
    BorderRightColor, "border-right-color", border_right_color, false;
    BorderBottomColor, "border-bottom-color", border_bottom_color, false;
    BorderLeftColor, "border-left-color", border_left_color, false;
    BorderTopLeftRadius, "border-top-left-radius", border_top_left_radius, false;
    BorderTopRightRadius, "border-top-right-radius", border_top_right_radius, false;
    BorderBottomRightRadius, "border-bottom-right-radius", border_bottom_right_radius, false;
    BorderBottomLeftRadius, "border-bottom-left-radius", border_bottom_left_radius, false;
    Color, "color", color, true;
    BackgroundColor, "background-color", background_color, false;
    BackgroundImage, "background-image", background_image, false;
    BackgroundRepeat, "background-repeat", background_repeat, false;
    BackgroundPositionX, "background-position-x", background_position_x, false;
    BackgroundPositionY, "background-position-y", background_position_y, false;
    BackgroundSize, "background-size", background_size, false;
    BackgroundClip, "background-clip", background_clip, false;
    BackgroundOrigin, "background-origin", background_origin, false;
    FontFamily, "font-family", font_family, true;
    FontSize, "font-size", font_size, true;
    FontWeight, "font-weight", font_weight, true;
    FontStyle, "font-style", font_style, true;
    LineHeight, "line-height", line_height, true;
    LetterSpacing, "letter-spacing", letter_spacing, true;
    WordSpacing, "word-spacing", word_spacing, true;
    TextAlign, "text-align", text_align, true;
    TextDecorationLine, "text-decoration-line", text_decoration_line, false;
    TextDecorationColor, "text-decoration-color", text_decoration_color, false;
    TextTransform, "text-transform", text_transform, true;
    WhiteSpace, "white-space", white_space, true;
    TextIndent, "text-indent", text_indent, true;
    VerticalAlign, "vertical-align", vertical_align, false;
    TextOverflow, "text-overflow", text_overflow, false;
    WordBreak, "word-break", word_break, true;
    OverflowWrap, "overflow-wrap", overflow_wrap, true;
    ListStyleType, "list-style-type", list_style_type, true;
    TextShadow, "text-shadow", text_shadow, true;
    BoxShadow, "box-shadow", box_shadow, false;
    Content, "content", content, false;
    Transform, "transform", transform, false;
    TransformOrigin, "transform-origin", transform_origin, false;
    AspectRatio, "aspect-ratio", aspect_ratio, false;
    FlexDirection, "flex-direction", flex_direction, false;
    FlexWrap, "flex-wrap", flex_wrap, false;
    FlexGrow, "flex-grow", flex_grow, false;
    FlexShrink, "flex-shrink", flex_shrink, false;
    FlexBasis, "flex-basis", flex_basis, false;
    JustifyContent, "justify-content", justify_content, false;
    AlignItems, "align-items", align_items, false;
    AlignSelf, "align-self", align_self, false;
    AlignContent, "align-content", align_content, false;
    JustifyItems, "justify-items", justify_items, false;
    JustifySelf, "justify-self", justify_self, false;
    Order, "order", order, false;
    RowGap, "row-gap", row_gap, false;
    ColumnGap, "column-gap", column_gap, false;
    GridTemplateColumns, "grid-template-columns", grid_template_columns, false;
    GridTemplateRows, "grid-template-rows", grid_template_rows, false;
    GridTemplateAreas, "grid-template-areas", grid_template_areas, false;
    GridAutoFlow, "grid-auto-flow", grid_auto_flow, false;
    GridAutoColumns, "grid-auto-columns", grid_auto_columns, false;
    GridAutoRows, "grid-auto-rows", grid_auto_rows, false;
    GridColumnStart, "grid-column-start", grid_column_start, false;
    GridColumnEnd, "grid-column-end", grid_column_end, false;
    GridRowStart, "grid-row-start", grid_row_start, false;
    GridRowEnd, "grid-row-end", grid_row_end, false;
}

/// Which "defaults to currentcolor" properties a cascade set explicitly.
#[derive(Clone, Copy, Debug, Default)]
pub struct Explicit {
    border_color: [bool; 4],
    deco_color: bool,
}

impl Explicit {
    fn set(&mut self, prop: Prop, on: bool) {
        match prop {
            Prop::BorderTopColor => self.border_color[0] = on,
            Prop::BorderRightColor => self.border_color[1] = on,
            Prop::BorderBottomColor => self.border_color[2] = on,
            Prop::BorderLeftColor => self.border_color[3] = on,
            Prop::TextDecorationColor => self.deco_color = on,
            _ => {}
        }
    }
}

/// What `apply` needs besides the value.
pub struct ApplyCtx<'a> {
    /// The parent style (the initial style for the root).
    pub parent: &'a Computed,
    /// The initial style (`initial` keyword).
    pub initial: &'a Computed,
    pub root_font_size: f32,
    pub vw: f32,
    pub vh: f32,
}

impl Prop {
    /// Properties that other properties' units depend on are applied first.
    pub fn is_early(self) -> bool {
        matches!(self, Prop::FontSize | Prop::Color)
    }
}

fn kw<E: Kw>(v: &Value) -> E {
    match v {
        Value::Ident(s) => E::from_ident(s).unwrap_or_default(),
        _ => E::default(),
    }
}

fn px_of(l: &Length) -> f32 {
    l.resolve(None).unwrap_or(0.0)
}

fn compute_shadow(s: &Shadow, lc: &LenCtx, current: Rgba) -> Shadow {
    Shadow {
        x: s.x.compute(lc),
        y: s.y.compute(lc),
        blur: s.blur.compute(lc),
        spread: s.spread.compute(lc),
        color: Color::Rgba(s.color.resolve(current)),
        inset: s.inset,
    }
}

fn compute_track(t: &TrackSize, lc: &LenCtx) -> TrackSize {
    match t {
        TrackSize::Len(l) => TrackSize::Len(l.compute(lc)),
        TrackSize::FitContent(l) => TrackSize::FitContent(l.compute(lc)),
        TrackSize::MinMax(a, b) => TrackSize::MinMax(Box::new(compute_track(a, lc)), Box::new(compute_track(b, lc))),
        other => other.clone(),
    }
}

fn compute_track_item(t: &TrackItem, lc: &LenCtx) -> TrackItem {
    match t {
        TrackItem::Size(s) => TrackItem::Size(compute_track(s, lc)),
        TrackItem::Repeat(c, items) => TrackItem::Repeat(*c, items.iter().map(|i| compute_track_item(i, lc)).collect()),
        TrackItem::Names(n) => TrackItem::Names(n.clone()),
    }
}

fn compute_image(img: &BgImage, lc: &LenCtx, current: Rgba) -> BgImage {
    match img {
        BgImage::Gradient(g) => BgImage::Gradient(Gradient {
            kind: g.kind,
            repeating: g.repeating,
            stops: g
                .stops
                .iter()
                .map(|s| GradientStop {
                    color: Color::Rgba(s.color.resolve(current)),
                    pos: s.pos.as_ref().map(|p| p.compute(lc)),
                })
                .collect(),
        }),
        other => other.clone(),
    }
}

fn font_size_keyword(s: &str, parent: f32) -> Option<f32> {
    Some(match s {
        "xx-small" => 9.0,
        "x-small" => 10.0,
        "small" => 13.0,
        "medium" => 16.0,
        "large" => 18.0,
        "x-large" => 24.0,
        "xx-large" => 32.0,
        "xxx-large" => 48.0,
        "larger" => parent * 1.2,
        "smaller" => parent / 1.2,
        _ => return None,
    })
}

fn relative_weight(keyword: &str, parent: f32) -> f32 {
    match keyword {
        "bolder" => {
            if parent < 350.0 {
                400.0
            } else if parent < 550.0 {
                700.0
            } else {
                900.0
            }
        }
        _ => {
            if parent < 550.0 {
                100.0
            } else if parent < 750.0 {
                400.0
            } else {
                700.0
            }
        }
    }
}

impl Computed {
    /// Applies one declared value to `prop`. `explicit` tracks the
    /// properties whose default is `currentcolor`.
    pub fn apply(&mut self, prop: Prop, v: &Value, cx: &ApplyCtx<'_>, explicit: &mut Explicit) {
        match v {
            Value::Inherit => {
                self.copy_prop(prop, cx.parent);
                explicit.set(prop, true);
                return;
            }
            Value::Initial => {
                self.copy_prop(prop, cx.initial);
                explicit.set(prop, false);
                return;
            }
            Value::Unset => {
                if prop.inherited() {
                    self.copy_prop(prop, cx.parent);
                } else {
                    self.copy_prop(prop, cx.initial);
                }
                explicit.set(prop, prop.inherited());
                return;
            }
            _ => {}
        }
        let lc = LenCtx {
            font_size: if prop == Prop::FontSize { cx.parent.font_size } else { self.font_size },
            root_font_size: cx.root_font_size,
            vw: cx.vw,
            vh: cx.vh,
        };
        let len = |v: &Value| -> Length {
            match v {
                Value::Length(l) => l.compute(&lc),
                _ => Length::Auto,
            }
        };
        let pair = |v: &Value| -> [Length; 2] {
            match v {
                Value::Pair(a, b) => [a.compute(&lc), b.compute(&lc)],
                _ => [Length::ZERO, Length::ZERO],
            }
        };
        let current = self.color;
        let resolve_color = |v: &Value| -> Rgba {
            match v {
                Value::Color(c) => c.resolve(current),
                _ => current,
            }
        };
        match prop {
            Prop::Display => self.display = kw(v),
            Prop::Position => self.position = kw(v),
            Prop::Float => self.float = kw(v),
            Prop::Clear => self.clear = kw(v),
            Prop::BoxSizing => self.box_sizing = kw(v),
            Prop::Visibility => self.visibility = kw(v),
            Prop::OverflowX => self.overflow_x = kw(v),
            Prop::OverflowY => self.overflow_y = kw(v),
            Prop::ZIndex => {
                self.z_index = match v {
                    Value::Int(n) => Some(*n),
                    _ => None,
                }
            }
            Prop::Opacity => {
                self.opacity = match v {
                    Value::Number(n) => n.clamp(0.0, 1.0),
                    _ => 1.0,
                }
            }
            Prop::Width => self.width = len(v),
            Prop::Height => self.height = len(v),
            Prop::MinWidth => self.min_width = len(v),
            Prop::MinHeight => self.min_height = len(v),
            Prop::MaxWidth => self.max_width = len(v),
            Prop::MaxHeight => self.max_height = len(v),
            Prop::MarginTop => self.margin_top = len(v),
            Prop::MarginRight => self.margin_right = len(v),
            Prop::MarginBottom => self.margin_bottom = len(v),
            Prop::MarginLeft => self.margin_left = len(v),
            Prop::PaddingTop => self.padding_top = len(v),
            Prop::PaddingRight => self.padding_right = len(v),
            Prop::PaddingBottom => self.padding_bottom = len(v),
            Prop::PaddingLeft => self.padding_left = len(v),
            Prop::Top => self.top = len(v),
            Prop::Right => self.right = len(v),
            Prop::Bottom => self.bottom = len(v),
            Prop::Left => self.left = len(v),
            Prop::BorderTopWidth => self.border_top_width = px_of(&len(v)).max(0.0),
            Prop::BorderRightWidth => self.border_right_width = px_of(&len(v)).max(0.0),
            Prop::BorderBottomWidth => self.border_bottom_width = px_of(&len(v)).max(0.0),
            Prop::BorderLeftWidth => self.border_left_width = px_of(&len(v)).max(0.0),
            Prop::BorderTopStyle => self.border_top_style = kw(v),
            Prop::BorderRightStyle => self.border_right_style = kw(v),
            Prop::BorderBottomStyle => self.border_bottom_style = kw(v),
            Prop::BorderLeftStyle => self.border_left_style = kw(v),
            Prop::BorderTopColor => {
                self.border_top_color = resolve_color(v);
                explicit.set(prop, true);
            }
            Prop::BorderRightColor => {
                self.border_right_color = resolve_color(v);
                explicit.set(prop, true);
            }
            Prop::BorderBottomColor => {
                self.border_bottom_color = resolve_color(v);
                explicit.set(prop, true);
            }
            Prop::BorderLeftColor => {
                self.border_left_color = resolve_color(v);
                explicit.set(prop, true);
            }
            Prop::BorderTopLeftRadius => self.border_top_left_radius = pair(v),
            Prop::BorderTopRightRadius => self.border_top_right_radius = pair(v),
            Prop::BorderBottomRightRadius => self.border_bottom_right_radius = pair(v),
            Prop::BorderBottomLeftRadius => self.border_bottom_left_radius = pair(v),
            Prop::Color => {
                self.color = match v {
                    Value::Color(Color::Rgba(c)) => *c,
                    _ => cx.parent.color,
                }
            }
            Prop::BackgroundColor => self.background_color = resolve_color(v),
            Prop::BackgroundImage => {
                if let Value::Images(images) = v {
                    self.background_image = images.iter().map(|i| compute_image(i, &lc, current)).collect();
                }
            }
            Prop::BackgroundRepeat => {
                if let Value::Repeats(r) = v {
                    self.background_repeat = r.clone();
                }
            }
            Prop::BackgroundPositionX => {
                if let Value::Lengths(l) = v {
                    self.background_position_x = l.iter().map(|l| l.compute(&lc)).collect();
                }
            }
            Prop::BackgroundPositionY => {
                if let Value::Lengths(l) = v {
                    self.background_position_y = l.iter().map(|l| l.compute(&lc)).collect();
                }
            }
            Prop::BackgroundSize => {
                if let Value::Sizes(s) = v {
                    self.background_size = s
                        .iter()
                        .map(|s| match s {
                            BgSize::Explicit(a, b) => BgSize::Explicit(a.compute(&lc), b.compute(&lc)),
                            other => other.clone(),
                        })
                        .collect();
                }
            }
            Prop::BackgroundClip => {
                if let Value::Boxes(b) = v {
                    self.background_clip = b.clone();
                }
            }
            Prop::BackgroundOrigin => {
                if let Value::Boxes(b) = v {
                    self.background_origin = b.clone();
                }
            }
            Prop::FontFamily => {
                if let Value::Families(f) = v {
                    self.font_family = f.clone();
                }
            }
            Prop::FontSize => {
                let parent = cx.parent.font_size;
                self.font_size = match v {
                    Value::Length(l) => match l.compute(&lc) {
                        Length::Percent(p) => parent * p / 100.0,
                        Length::Calc(c) => c.resolve(parent),
                        other => px_of(&other),
                    },
                    Value::Ident(s) => font_size_keyword(s, parent).unwrap_or(parent),
                    _ => parent,
                }
                .max(0.0);
            }
            Prop::FontWeight => {
                self.font_weight = match v {
                    Value::Number(n) => n.clamp(1.0, 1000.0),
                    Value::Ident(s) => match s.as_str() {
                        "normal" => 400.0,
                        "bold" => 700.0,
                        other => relative_weight(other, cx.parent.font_weight),
                    },
                    _ => 400.0,
                }
            }
            Prop::FontStyle => self.font_style = kw(v),
            Prop::LineHeight => {
                self.line_height = match v {
                    Value::Number(n) => LineHeight::Number(*n),
                    Value::Length(l) => match l.compute(&lc) {
                        Length::Percent(p) => LineHeight::Px(self.font_size * p / 100.0),
                        Length::Calc(c) => LineHeight::Px(c.resolve(self.font_size)),
                        other => LineHeight::Px(px_of(&other)),
                    },
                    _ => LineHeight::Normal,
                }
            }
            Prop::LetterSpacing => {
                self.letter_spacing = match v {
                    Value::Length(_) => px_of(&len(v)),
                    _ => 0.0,
                }
            }
            Prop::WordSpacing => {
                self.word_spacing = match v {
                    Value::Length(_) => px_of(&len(v)),
                    _ => 0.0,
                }
            }
            Prop::TextAlign => self.text_align = kw(v),
            Prop::TextDecorationLine => {
                if let Value::DecoLine(d) = v {
                    self.text_decoration_line = *d;
                }
            }
            Prop::TextDecorationColor => {
                self.text_decoration_color = resolve_color(v);
                explicit.set(prop, true);
            }
            Prop::TextTransform => self.text_transform = kw(v),
            Prop::WhiteSpace => self.white_space = kw(v),
            Prop::TextIndent => self.text_indent = len(v),
            Prop::VerticalAlign => {
                self.vertical_align = match v {
                    Value::Length(_) => VAlign::Length(len(v)),
                    other => VAlign::Keyword(kw(other)),
                }
            }
            Prop::TextOverflow => self.text_overflow = kw(v),
            Prop::WordBreak => self.word_break = kw(v),
            Prop::OverflowWrap => self.overflow_wrap = kw(v),
            Prop::ListStyleType => self.list_style_type = kw(v),
            Prop::TextShadow => {
                if let Value::Shadows(s) = v {
                    self.text_shadow = s.iter().map(|s| compute_shadow(s, &lc, current)).collect();
                }
            }
            Prop::BoxShadow => {
                if let Value::Shadows(s) = v {
                    self.box_shadow = s.iter().map(|s| compute_shadow(s, &lc, current)).collect();
                }
            }
            Prop::Content => {
                if let Value::Content(c) = v {
                    self.content = c.clone();
                }
            }
            Prop::Transform => {
                if let Value::Transform(fns) = v {
                    self.transform = if fns.is_empty() {
                        None
                    } else {
                        let t = Transform2D::from_fns(fns, &lc);
                        if t.is_identity() {
                            None
                        } else {
                            Some(t)
                        }
                    };
                }
            }
            Prop::TransformOrigin => self.transform_origin = pair(v),
            Prop::AspectRatio => {
                if let Value::Aspect(a) = v {
                    self.aspect_ratio = *a;
                }
            }
            Prop::FlexDirection => self.flex_direction = kw(v),
            Prop::FlexWrap => self.flex_wrap = kw(v),
            Prop::FlexGrow => {
                self.flex_grow = match v {
                    Value::Number(n) => n.max(0.0),
                    _ => 0.0,
                }
            }
            Prop::FlexShrink => {
                self.flex_shrink = match v {
                    Value::Number(n) => n.max(0.0),
                    _ => 1.0,
                }
            }
            Prop::FlexBasis => self.flex_basis = len(v),
            Prop::JustifyContent => self.justify_content = kw(v),
            Prop::AlignItems => self.align_items = kw(v),
            Prop::AlignSelf => self.align_self = kw(v),
            Prop::AlignContent => self.align_content = kw(v),
            Prop::JustifyItems => self.justify_items = kw(v),
            Prop::JustifySelf => self.justify_self = kw(v),
            Prop::Order => {
                self.order = match v {
                    Value::Int(n) => *n,
                    _ => 0,
                }
            }
            Prop::RowGap => self.row_gap = len(v),
            Prop::ColumnGap => self.column_gap = len(v),
            Prop::GridTemplateColumns => {
                if let Value::Tracks(t) = v {
                    self.grid_template_columns = t.iter().map(|i| compute_track_item(i, &lc)).collect();
                }
            }
            Prop::GridTemplateRows => {
                if let Value::Tracks(t) = v {
                    self.grid_template_rows = t.iter().map(|i| compute_track_item(i, &lc)).collect();
                }
            }
            Prop::GridTemplateAreas => {
                if let Value::Areas(a) = v {
                    self.grid_template_areas = a.clone();
                }
            }
            Prop::GridAutoFlow => self.grid_auto_flow = kw(v),
            Prop::GridAutoColumns => {
                if let Value::TrackSizes(t) = v {
                    self.grid_auto_columns = t.iter().map(|t| compute_track(t, &lc)).collect();
                }
            }
            Prop::GridAutoRows => {
                if let Value::TrackSizes(t) = v {
                    self.grid_auto_rows = t.iter().map(|t| compute_track(t, &lc)).collect();
                }
            }
            Prop::GridColumnStart => self.grid_column_start = line(v),
            Prop::GridColumnEnd => self.grid_column_end = line(v),
            Prop::GridRowStart => self.grid_row_start = line(v),
            Prop::GridRowEnd => self.grid_row_end = line(v),
        }
        // An explicit non-inherit value of the color properties is not `initial`.
        if matches!(v, Value::Color(_)) {
            explicit.set(prop, true);
        }
    }

    /// Final adjustments once every declaration is applied: `currentcolor`
    /// defaults, border widths of absent borders, blockification.
    pub fn finalize(&mut self, explicit: &Explicit) {
        let c = self.color;
        if !explicit.border_color[0] {
            self.border_top_color = c;
        }
        if !explicit.border_color[1] {
            self.border_right_color = c;
        }
        if !explicit.border_color[2] {
            self.border_bottom_color = c;
        }
        if !explicit.border_color[3] {
            self.border_left_color = c;
        }
        if !explicit.deco_color {
            self.text_decoration_color = c;
        }
        let none = |s: BorderStyle| matches!(s, BorderStyle::None | BorderStyle::Hidden);
        if none(self.border_top_style) {
            self.border_top_width = 0.0;
        }
        if none(self.border_right_style) {
            self.border_right_width = 0.0;
        }
        if none(self.border_bottom_style) {
            self.border_bottom_width = 0.0;
        }
        if none(self.border_left_style) {
            self.border_left_width = 0.0;
        }
        if matches!(self.position, Position::Absolute | Position::Fixed) {
            self.float = Float::None;
        }
        if self.float != Float::None || matches!(self.position, Position::Absolute | Position::Fixed) {
            self.display = blockify(self.display);
        }
    }

    /// `[top, right, bottom, left]` margins.
    pub fn margin(&self) -> [&Length; 4] {
        [&self.margin_top, &self.margin_right, &self.margin_bottom, &self.margin_left]
    }

    /// `[top, right, bottom, left]` paddings.
    pub fn padding(&self) -> [&Length; 4] {
        [&self.padding_top, &self.padding_right, &self.padding_bottom, &self.padding_left]
    }

    /// `[top, right, bottom, left]` inset offsets.
    pub fn inset(&self) -> [&Length; 4] {
        [&self.top, &self.right, &self.bottom, &self.left]
    }

    /// `[top, right, bottom, left]` border widths in px.
    pub fn border_widths(&self) -> [f32; 4] {
        [self.border_top_width, self.border_right_width, self.border_bottom_width, self.border_left_width]
    }

    /// `[top, right, bottom, left]` border styles.
    pub fn border_styles(&self) -> [BorderStyle; 4] {
        [self.border_top_style, self.border_right_style, self.border_bottom_style, self.border_left_style]
    }

    /// `[top, right, bottom, left]` border colors.
    pub fn border_colors(&self) -> [Rgba; 4] {
        [self.border_top_color, self.border_right_color, self.border_bottom_color, self.border_left_color]
    }

    /// Corner radii `[top-left, top-right, bottom-right, bottom-left]`.
    pub fn border_radii(&self) -> [&[Length; 2]; 4] {
        [
            &self.border_top_left_radius,
            &self.border_top_right_radius,
            &self.border_bottom_right_radius,
            &self.border_bottom_left_radius,
        ]
    }

    /// The background layers, one per `background-image` entry, with the
    /// other lists cycled as the CSS spec requires.
    pub fn background_layers(&self) -> Vec<BgLayer> {
        let n = self.background_image.len();
        (0..n)
            .map(|i| BgLayer {
                image: self.background_image[i].clone(),
                repeat: cycle(&self.background_repeat, i, (BgRepeatKind::Repeat, BgRepeatKind::Repeat)),
                position: (
                    cycle(&self.background_position_x, i, Length::Percent(0.0)),
                    cycle(&self.background_position_y, i, Length::Percent(0.0)),
                ),
                size: cycle(&self.background_size, i, BgSize::Explicit(Length::Auto, Length::Auto)),
                clip: cycle(&self.background_clip, i, BgBox::BorderBox),
                origin: cycle(&self.background_origin, i, BgBox::PaddingBox),
            })
            .collect()
    }

    /// Whether the element generates a box.
    pub fn is_displayed(&self) -> bool {
        self.display != Display::None
    }
}

/// One resolved background layer.
#[derive(Clone, Debug, PartialEq)]
pub struct BgLayer {
    pub image: BgImage,
    pub repeat: (BgRepeatKind, BgRepeatKind),
    pub position: (Length, Length),
    pub size: BgSize,
    pub clip: BgBox,
    pub origin: BgBox,
}

fn cycle<T: Clone>(list: &[T], i: usize, default: T) -> T {
    if list.is_empty() {
        default
    } else {
        list[i % list.len()].clone()
    }
}

fn line(v: &Value) -> GridLine {
    match v {
        Value::Line(l) => l.clone(),
        _ => GridLine::Auto,
    }
}

/// The blockified counterpart of a display type.
pub fn blockify(d: Display) -> Display {
    match d {
        Display::Inline | Display::InlineBlock => Display::Block,
        Display::InlineFlex => Display::Flex,
        Display::InlineGrid => Display::Grid,
        Display::InlineTable => Display::Table,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx<'a>(parent: &'a Computed, initial: &'a Computed) -> ApplyCtx<'a> {
        ApplyCtx { parent, initial, root_font_size: 16.0, vw: 800.0, vh: 600.0 }
    }

    fn apply(c: &mut Computed, parent: &Computed, prop: Prop, v: Value) {
        let init = Computed::default();
        let mut ex = Explicit::default();
        c.apply(prop, &v, &cx(parent, &init), &mut ex);
    }

    #[test]
    fn prop_table_lookup() {
        assert_eq!(Prop::from_name("margin-top"), Some(Prop::MarginTop));
        assert_eq!(Prop::from_name("nope"), None);
        assert_eq!(Prop::MarginTop.name(), "margin-top");
        assert!(Prop::Color.inherited());
        assert!(!Prop::Width.inherited());
        for p in Prop::ALL {
            assert_eq!(Prop::from_name(p.name()), Some(*p));
        }
    }

    #[test]
    fn inherit_from_copies_only_inherited() {
        let mut parent = Computed::default();
        parent.color = Rgba::WHITE;
        parent.font_size = 20.0;
        parent.width = Length::Px(100.0);
        parent.margin_top = Length::Px(9.0);
        let c = Computed::inherit_from(&parent);
        assert_eq!(c.color, Rgba::WHITE);
        assert_eq!(c.font_size, 20.0);
        assert_eq!(c.width, Length::Auto);
        assert_eq!(c.margin_top, Length::ZERO);
    }

    #[test]
    fn em_resolves_against_own_font_size_but_font_size_against_parent() {
        let mut parent = Computed::default();
        parent.font_size = 10.0;
        let mut c = Computed::inherit_from(&parent);
        apply(&mut c, &parent, Prop::FontSize, Value::Length(Length::Dim(2.0, Unit::Em)));
        assert_eq!(c.font_size, 20.0);
        apply(&mut c, &parent, Prop::MarginTop, Value::Length(Length::Dim(1.5, Unit::Em)));
        assert_eq!(c.margin_top, Length::Px(30.0));
        apply(&mut c, &parent, Prop::FontSize, Value::Length(Length::Dim(150.0, Unit::Percent)));
        assert_eq!(c.font_size, 15.0);
    }

    #[test]
    fn font_size_keywords_and_relative_weight() {
        let mut parent = Computed::default();
        parent.font_weight = 400.0;
        let mut c = Computed::inherit_from(&parent);
        apply(&mut c, &parent, Prop::FontSize, Value::Ident("large".into()));
        assert_eq!(c.font_size, 18.0);
        apply(&mut c, &parent, Prop::FontWeight, Value::Ident("bolder".into()));
        assert_eq!(c.font_weight, 700.0);
        apply(&mut c, &parent, Prop::FontWeight, Value::Ident("bold".into()));
        assert_eq!(c.font_weight, 700.0);
        apply(&mut c, &parent, Prop::FontWeight, Value::Number(300.0));
        assert_eq!(c.font_weight, 300.0);
    }

    #[test]
    fn css_wide_keywords() {
        let mut parent = Computed::default();
        parent.width = Length::Px(77.0);
        parent.color = Rgba::WHITE;
        let mut c = Computed::inherit_from(&parent);
        apply(&mut c, &parent, Prop::Width, Value::Inherit);
        assert_eq!(c.width, Length::Px(77.0));
        apply(&mut c, &parent, Prop::Width, Value::Initial);
        assert_eq!(c.width, Length::Auto);
        c.color = Rgba::BLACK;
        apply(&mut c, &parent, Prop::Color, Value::Unset);
        assert_eq!(c.color, Rgba::WHITE);
        c.margin_top = Length::Px(4.0);
        apply(&mut c, &parent, Prop::MarginTop, Value::Unset);
        assert_eq!(c.margin_top, Length::ZERO);
    }

    #[test]
    fn currentcolor_follows_own_color_in_finalize() {
        let parent = Computed::default();
        let mut c = Computed::inherit_from(&parent);
        let init = Computed::default();
        let mut ex = Explicit::default();
        c.apply(Prop::Color, &Value::Color(Color::Rgba(Rgba::WHITE)), &cx(&parent, &init), &mut ex);
        c.apply(Prop::BorderTopStyle, &Value::Ident("solid".into()), &cx(&parent, &init), &mut ex);
        c.apply(Prop::BorderTopWidth, &Value::Length(Length::Dim(2.0, Unit::Px)), &cx(&parent, &init), &mut ex);
        c.apply(Prop::BorderLeftColor, &Value::Color(Color::Rgba(Rgba::from_u8(255, 0, 0, 1.0))), &cx(&parent, &init), &mut ex);
        c.finalize(&ex);
        assert_eq!(c.border_top_color, Rgba::WHITE);
        assert_eq!(c.border_left_color, Rgba::from_u8(255, 0, 0, 1.0));
        assert_eq!(c.border_top_width, 2.0);
        assert_eq!(c.border_right_width, 0.0);
        assert_eq!(c.text_decoration_color, Rgba::WHITE);
    }

    #[test]
    fn background_color_currentcolor() {
        let parent = Computed::default();
        let mut c = Computed::inherit_from(&parent);
        c.color = Rgba::WHITE;
        apply(&mut c, &parent, Prop::BackgroundColor, Value::Color(Color::Current));
        assert_eq!(c.background_color, Rgba::WHITE);
    }

    #[test]
    fn blockification_of_floats_and_absolute() {
        let mut c = Computed::default();
        c.float = Float::Left;
        c.display = Display::Inline;
        c.finalize(&Explicit::default());
        assert_eq!(c.display, Display::Block);
        let mut c = Computed::default();
        c.position = Position::Absolute;
        c.float = Float::Right;
        c.display = Display::InlineFlex;
        c.finalize(&Explicit::default());
        assert_eq!(c.display, Display::Flex);
        assert_eq!(c.float, Float::None);
    }

    #[test]
    fn line_height_forms() {
        let parent = Computed::default();
        let mut c = Computed::inherit_from(&parent);
        c.font_size = 20.0;
        apply(&mut c, &parent, Prop::LineHeight, Value::Number(1.5));
        assert_eq!(c.line_height, LineHeight::Number(1.5));
        apply(&mut c, &parent, Prop::LineHeight, Value::Length(Length::Dim(150.0, Unit::Percent)));
        assert_eq!(c.line_height, LineHeight::Px(30.0));
        apply(&mut c, &parent, Prop::LineHeight, Value::Length(Length::Dim(2.0, Unit::Em)));
        assert_eq!(c.line_height, LineHeight::Px(40.0));
        apply(&mut c, &parent, Prop::LineHeight, Value::Ident("normal".into()));
        assert_eq!(c.line_height, LineHeight::Normal);
    }

    #[test]
    fn transform_and_opacity() {
        let parent = Computed::default();
        let mut c = Computed::default();
        apply(&mut c, &parent, Prop::Transform, Value::Transform(vec![TransformFn::Scale(2.0, 2.0)]));
        assert_eq!(c.transform.unwrap().resolve(1.0, 1.0), [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
        apply(&mut c, &parent, Prop::Transform, Value::Transform(vec![]));
        assert!(c.transform.is_none());
        apply(&mut c, &parent, Prop::Opacity, Value::Number(1.7));
        assert_eq!(c.opacity, 1.0);
        apply(&mut c, &parent, Prop::ZIndex, Value::Int(-3));
        assert_eq!(c.z_index, Some(-3));
        apply(&mut c, &parent, Prop::ZIndex, Value::Ident("auto".into()));
        assert_eq!(c.z_index, None);
    }

    #[test]
    fn background_layers_cycle() {
        let mut c = Computed::default();
        c.background_image = vec![BgImage::None, BgImage::Url("a".into())];
        c.background_clip = vec![BgBox::ContentBox];
        let layers = c.background_layers();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[1].clip, BgBox::ContentBox);
        assert_eq!(layers[1].image, BgImage::Url("a".into()));
    }

    #[test]
    fn shadows_resolve_currentcolor_and_units() {
        let parent = Computed::default();
        let mut c = Computed::inherit_from(&parent);
        c.color = Rgba::WHITE;
        c.font_size = 10.0;
        let s = Shadow {
            x: Length::Dim(1.0, Unit::Em),
            y: Length::Px(2.0),
            blur: Length::ZERO,
            spread: Length::ZERO,
            color: Color::Current,
            inset: false,
        };
        apply(&mut c, &parent, Prop::BoxShadow, Value::Shadows(vec![s]));
        assert_eq!(c.box_shadow[0].px(), (10.0, 2.0, 0.0, 0.0));
        assert_eq!(c.box_shadow[0].color, Color::Rgba(Rgba::WHITE));
    }
}
