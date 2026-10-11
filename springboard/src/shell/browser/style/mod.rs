//! CSS engine: parse style sheets, match selectors over the `Dom`, run the
//! cascade and produce a `Computed` style per element.
//!
//! * `values`    value types, color / length / `calc()` parsing
//! * `parse`     style sheets, `@media`, `@supports`, `@font-face`, shorthands, `var()`
//! * `selectors` the `selectors` crate glue (`Element` for `(&Dom, NodeId)`)
//! * `computed`  the property table and the `Computed` style
//! * `cascade`   `StyleEngine`: rule index, ancestor bloom filter, cascade
//! * `ua`        the user-agent style sheet
//!
//! Needs two Cargo dependencies besides `cssparser` 0.38 and `selectors`
//! 0.38: `cssparser37 = { package = "cssparser", version = "0.37" }` (the
//! version `selectors` 0.38 is built on) and `precomputed-hash = "0.1"`.

pub mod cascade;
pub mod computed;
pub mod parse;
pub mod selectors;
pub mod ua;
pub mod values;

pub use cascade::{AncestorBloom, NodeStyle, StyleEngine, StyleTree};
pub use computed::{BgLayer, Computed, Prop};
pub use parse::{
    parse_declaration_list, parse_stylesheet, FontFace, FontSrc, MediaEnv, MediaQueryList, Origin, Stylesheet,
};
pub use ua::{ua_stylesheet, UA_CSS};
pub use values::{
    Align, BgBox, BgImage, BgRepeatKind, BgSize, BorderStyle, BoxSizing, Calc, Clear, Color, Content, ContentItem,
    DecoLine, Display, FlexDirection, FlexWrap, Float, FontStyle, Gradient, GradientKind, GradientStop, GridAutoFlow,
    GridLine, LenCtx, Length, LineHeight, ListStyleType, Overflow, Position, Rgba, Shadow, TextAlign, TextOverflow,
    TextTransform, TrackItem, TrackSize, Transform2D, TransformFn, Unit, VAlign, VAlignKw, Visibility, WhiteSpace,
};
