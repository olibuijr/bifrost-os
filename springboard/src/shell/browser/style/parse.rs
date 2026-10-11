//! Style sheet parsing: rules, `@media`, `@supports`, `@font-face`,
//! declaration blocks, property value parsing with shorthand expansion, and
//! `var()` substitution.

use super::computed::Prop;
use super::selectors::{parse_selector_list, SelImpl};
use super::values::*;
use cssparser::{
    AtRuleParser, BasicParseErrorKind, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};
use ::selectors::parser::{Selector, SelectorList};
use std::collections::BTreeMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Public data
// ---------------------------------------------------------------------------

/// Where a style sheet comes from; the cascade orders origins.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    UserAgent,
    Author,
}

/// The environment `@media` queries are evaluated against.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MediaEnv {
    pub width: f32,
    pub height: f32,
    pub dark: bool,
}

/// One declaration of a block.
#[derive(Clone, PartialEq, Debug)]
pub enum Declaration {
    /// A fully parsed longhand.
    Longhand(Prop, Value),
    /// A custom property: name (with `--`) and raw text.
    Custom(String, String),
    /// A property whose value contains `var()`: name and raw text, parsed
    /// at cascade time after substitution.
    Raw(String, String),
}

/// The declarations of one rule split by importance.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DeclBlock {
    pub normal: Vec<Declaration>,
    pub important: Vec<Declaration>,
}

impl DeclBlock {
    /// Whether the block has no declarations.
    pub fn is_empty(&self) -> bool {
        self.normal.is_empty() && self.important.is_empty()
    }
}

/// One style rule for one selector (a selector list is split into rules).
#[derive(Clone, Debug)]
pub struct Rule {
    pub selector: Selector<SelImpl>,
    pub specificity: u32,
    pub block: Arc<DeclBlock>,
    /// All enclosing `@media` conditions; every one must match.
    pub media: Vec<Arc<MediaQueryList>>,
}

/// A source of an `@font-face`.
#[derive(Clone, PartialEq, Debug)]
pub enum FontSrc {
    Url { url: String, format: Option<String> },
    Local(String),
}

/// A collected `@font-face` rule.
#[derive(Clone, PartialEq, Debug)]
pub struct FontFace {
    pub family: String,
    pub sources: Vec<FontSrc>,
    pub weight: f32,
    pub style: FontStyle,
    pub unicode_range: Option<String>,
}

/// A parsed style sheet.
#[derive(Clone, Debug)]
pub struct Stylesheet {
    pub origin: Origin,
    pub rules: Vec<Rule>,
    pub font_faces: Vec<FontFace>,
}

impl Stylesheet {
    /// The style rules in source order.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// The collected `@font-face` rules.
    pub fn font_faces(&self) -> &[FontFace] {
        &self.font_faces
    }
}

/// Parses a style sheet. Invalid rules and declarations are dropped.
pub fn parse_stylesheet(src: &str, origin: Origin) -> Stylesheet {
    let mut parser = Parser::new(src);
    let mut top = TopLevel::default();
    {
        let it = StyleSheetParser::new(&mut parser, &mut top);
        for _ in it {}
    }
    Stylesheet { origin, rules: top.rules, font_faces: top.font_faces }
}

/// Parses a declaration list such as a `style` attribute.
pub fn parse_declaration_list(text: &str) -> DeclBlock {
    let mut parser = Parser::new(text);
    let mut dp = DeclParser::default();
    {
        let it: RuleBodyParser<'_, '_, DeclParser, (), ()> = RuleBodyParser::new(&mut parser, &mut dp);
        for _ in it {}
    }
    dp.block
}

// ---------------------------------------------------------------------------
// Declaration block parser
// ---------------------------------------------------------------------------

#[derive(Default)]
struct DeclParser {
    block: DeclBlock,
}

pub(crate) fn split_important(s: &str) -> (&str, bool) {
    let t = s.trim_end();
    if t.len() >= 9 {
        if let Some(tail) = t.get(t.len() - 9..) {
            if tail.eq_ignore_ascii_case("important") {
                if let Some(rest) = t[..t.len() - 9].trim_end().strip_suffix('!') {
                    return (rest.trim_end(), true);
                }
            }
        }
    }
    (t, false)
}

/// Whether `text` contains a `var(` function.
pub fn has_var(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() < 4 {
        return false;
    }
    (0..=b.len() - 4).any(|i| b[i..i + 4].eq_ignore_ascii_case(b"var("))
}

impl DeclParser {
    fn add(&mut self, d: Declaration, important: bool) {
        if important {
            self.block.important.push(d);
        } else {
            self.block.normal.push(d);
        }
    }

    fn push(&mut self, name: &str, text: &str, important: bool) -> bool {
        if name.starts_with("--") {
            self.add(Declaration::Custom(name.to_string(), text.trim().to_string()), important);
            return true;
        }
        let lname = name.to_ascii_lowercase();
        if has_var(text) {
            if is_known_property(&lname) {
                self.add(Declaration::Raw(lname, text.trim().to_string()), important);
                return true;
            }
            return false;
        }
        match parse_property(&lname, text) {
            Some(list) => {
                for (p, v) in list {
                    self.add(Declaration::Longhand(p, v), important);
                }
                true
            }
            None => false,
        }
    }
}

impl<'i> DeclarationParser<'i> for DeclParser {
    type Declaration = ();
    type Error = ();

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<(), ParseError<()>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        let raw = input.slice_from(start);
        let (text, important) = split_important(raw);
        if self.push(&name, text, important) {
            Ok(())
        } else {
            Err(ParseError::unexpected_token())
        }
    }
}

impl<'i> AtRuleParser<'i> for DeclParser {
    type Prelude = ();
    type AtRule = ();
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for DeclParser {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, (), ()> for DeclParser {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Top level rules
// ---------------------------------------------------------------------------

#[derive(Default)]
struct TopLevel {
    rules: Vec<Rule>,
    font_faces: Vec<FontFace>,
    media: Vec<Arc<MediaQueryList>>,
}

enum Pre {
    Media(MediaQueryList),
    Supports(bool),
    FontFace,
    Layer,
    Skip,
}

impl<'i> AtRuleParser<'i> for TopLevel {
    type Prelude = Pre;
    type AtRule = ();
    type Error = ();

    fn parse_prelude(&mut self, name: CowRcStr<'i>, input: &mut Parser<'i>) -> Result<Pre, ParseError<()>> {
        let n = name.to_ascii_lowercase();
        match n.as_str() {
            "media" => Ok(Pre::Media(parse_media_query_list(input))),
            "supports" => {
                let ok = parse_supports(input).unwrap_or(false);
                while input.next().is_ok() {}
                Ok(Pre::Supports(ok))
            }
            "font-face" => Ok(Pre::FontFace),
            "layer" => {
                while input.next().is_ok() {}
                Ok(Pre::Layer)
            }
            "container" | "scope" | "starting-style" => {
                while input.next().is_ok() {}
                Ok(Pre::Skip)
            }
            _ => Err(ParseError::from_basic_kind(BasicParseErrorKind::AtRuleInvalid)),
        }
    }

    fn rule_without_block(&mut self, _prelude: Pre, _start: &ParserState) -> Result<(), ()> {
        Ok(())
    }

    fn parse_block(&mut self, prelude: Pre, _start: &ParserState, input: &mut Parser<'i>) -> Result<(), ParseError<()>> {
        match prelude {
            Pre::Media(list) => {
                self.media.push(Arc::new(list));
                {
                    let it = StyleSheetParser::new(input, &mut *self);
                    for _ in it {}
                }
                self.media.pop();
            }
            Pre::Supports(true) | Pre::Layer => {
                let it = StyleSheetParser::new(input, &mut *self);
                for _ in it {}
            }
            Pre::Supports(false) | Pre::Skip => {}
            Pre::FontFace => {
                let mut dp = DescriptorParser::default();
                {
                    let it: RuleBodyParser<'_, '_, DescriptorParser, (), ()> = RuleBodyParser::new(input, &mut dp);
                    for _ in it {}
                }
                if let Some(ff) = build_font_face(&dp.items) {
                    self.font_faces.push(ff);
                }
            }
        }
        Ok(())
    }
}

impl<'i> QualifiedRuleParser<'i> for TopLevel {
    type Prelude = SelectorList<SelImpl>;
    type QualifiedRule = ();
    type Error = ();

    fn parse_prelude(&mut self, input: &mut Parser<'i>) -> Result<SelectorList<SelImpl>, ParseError<()>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        let text = input.slice_from(start);
        match parse_selector_list(text) {
            Some(list) => Ok(list),
            None => Err(ParseError::unexpected_token()),
        }
    }

    fn parse_block(
        &mut self,
        prelude: SelectorList<SelImpl>,
        _start: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<(), ParseError<()>> {
        let mut dp = DeclParser::default();
        {
            let it: RuleBodyParser<'_, '_, DeclParser, (), ()> = RuleBodyParser::new(input, &mut dp);
            for _ in it {}
        }
        if dp.block.is_empty() {
            return Ok(());
        }
        let block = Arc::new(dp.block);
        for sel in prelude.slice() {
            self.rules.push(Rule {
                selector: sel.clone(),
                specificity: sel.specificity(),
                block: block.clone(),
                media: self.media.clone(),
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// @font-face
// ---------------------------------------------------------------------------

#[derive(Default)]
struct DescriptorParser {
    items: Vec<(String, String)>,
}

impl<'i> DeclarationParser<'i> for DescriptorParser {
    type Declaration = ();
    type Error = ();

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _start: &ParserState,
    ) -> Result<(), ParseError<()>> {
        let start = input.position();
        while input.next_including_whitespace_and_comments().is_ok() {}
        let (text, _) = split_important(input.slice_from(start));
        self.items.push((name.to_ascii_lowercase(), text.trim().to_string()));
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for DescriptorParser {
    type Prelude = ();
    type AtRule = ();
    type Error = ();
}

impl<'i> QualifiedRuleParser<'i> for DescriptorParser {
    type Prelude = ();
    type QualifiedRule = ();
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, (), ()> for DescriptorParser {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

fn build_font_face(items: &[(String, String)]) -> Option<FontFace> {
    let mut family = None;
    let mut sources = Vec::new();
    let mut weight = 400.0;
    let mut style = FontStyle::Normal;
    let mut unicode_range = None;
    for (name, text) in items {
        match name.as_str() {
            "font-family" => {
                let mut p = Parser::new(text);
                if let Ok(mut f) = p.parse_entirely(parse_font_family) {
                    if !f.is_empty() {
                        family = Some(f.remove(0));
                    }
                }
            }
            "src" => sources = parse_font_src(text),
            "font-weight" => {
                let first = text.split_whitespace().next().unwrap_or("");
                weight = match first {
                    "normal" => 400.0,
                    "bold" => 700.0,
                    n => n.parse::<f32>().unwrap_or(400.0),
                };
            }
            "font-style" => {
                let first = text.split_whitespace().next().unwrap_or("");
                style = FontStyle::from_ident(first).unwrap_or(FontStyle::Normal);
            }
            "unicode-range" => unicode_range = Some(text.clone()),
            _ => {}
        }
    }
    let family = family?;
    if sources.is_empty() {
        return None;
    }
    Some(FontFace { family, sources, weight, style, unicode_range })
}

fn parse_font_src(text: &str) -> Vec<FontSrc> {
    let mut p = Parser::new(text);
    let r = p.comma(|i| -> PResult<FontSrc> {
        let mut src: Option<FontSrc> = None;
        let mut fmt: Option<String> = None;
        while !i.is_exhausted() {
            let t = i.next()?.clone();
            match t {
                Token::UnquotedUrl(ref u) => src = Some(FontSrc::Url { url: u.to_string(), format: None }),
                Token::Function(ref n) => {
                    let n = n.to_ascii_lowercase();
                    match n.as_str() {
                        "url" => {
                            let u = i.nested(|i| Ok(i.expect_string()?.to_string()))?;
                            src = Some(FontSrc::Url { url: u, format: None });
                        }
                        "local" => {
                            let name = i.nested(|i| {
                                let t = i.next()?.clone();
                                match t {
                                    Token::QuotedString(ref s) => Ok(s.to_string()),
                                    Token::Ident(ref s) => {
                                        let mut name = s.to_string();
                                        while let Ok(x) = i.try_parse(|i| i.expect_ident_cloned()) {
                                            name.push(' ');
                                            name.push_str(&x);
                                        }
                                        Ok(name)
                                    }
                                    _ => fail(),
                                }
                            })?;
                            src = Some(FontSrc::Local(name));
                        }
                        "format" => {
                            fmt = Some(i.nested(|i| {
                                let t = i.next()?.clone();
                                match t {
                                    Token::QuotedString(ref s) => Ok(s.to_string()),
                                    Token::Ident(ref s) => Ok(s.to_string()),
                                    _ => fail(),
                                }
                            })?);
                        }
                        _ => {
                            i.nested(|i| {
                                while i.next().is_ok() {}
                                Ok(())
                            })?;
                        }
                    }
                }
                _ => {}
            }
        }
        match src {
            Some(FontSrc::Url { url, .. }) => Ok(FontSrc::Url { url, format: fmt }),
            Some(local) => Ok(local),
            None => fail(),
        }
    });
    r.unwrap_or_default()
}

// ---------------------------------------------------------------------------
// @media
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Cmp {
    Eq,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    fn test(self, actual: f32, target: f32) -> bool {
        match self {
            Cmp::Eq => (actual - target).abs() < 0.001,
            Cmp::Lt => actual < target,
            Cmp::Le => actual <= target,
            Cmp::Gt => actual > target,
            Cmp::Ge => actual >= target,
        }
    }

    fn flip(self) -> Cmp {
        match self {
            Cmp::Lt => Cmp::Gt,
            Cmp::Le => Cmp::Ge,
            Cmp::Gt => Cmp::Lt,
            Cmp::Ge => Cmp::Le,
            Cmp::Eq => Cmp::Eq,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
enum Feat {
    Width(Cmp, f32),
    Height(Cmp, f32),
    Aspect(Cmp, f32),
    Resolution(Cmp, f32),
    Scheme(bool),
    Portrait(bool),
    Const(bool),
}

#[derive(Clone, PartialEq, Debug)]
enum MediaCond {
    Feature(Feat),
    Not(Box<MediaCond>),
    And(Vec<MediaCond>),
    Or(Vec<MediaCond>),
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MediaType {
    All,
    Screen,
    Print,
    Other,
}

#[derive(Clone, PartialEq, Debug)]
struct MediaQuery {
    negated: bool,
    media_type: MediaType,
    cond: Option<MediaCond>,
}

/// A comma separated list of media queries; matches when any query does.
#[derive(Clone, PartialEq, Debug)]
pub struct MediaQueryList(Vec<MediaQuery>);

impl MediaQueryList {
    /// Parses the text between `@media` and `{`; a bad query becomes `not all`.
    pub fn parse(text: &str) -> Self {
        parse_media_query_list(&mut Parser::new(text))
    }

    /// Whether the list matches `env`.
    pub fn matches(&self, env: &MediaEnv) -> bool {
        self.0.iter().any(|q| q.matches(env))
    }
}

impl MediaQuery {
    fn matches(&self, env: &MediaEnv) -> bool {
        let type_ok = match self.media_type {
            MediaType::All | MediaType::Screen => true,
            MediaType::Print | MediaType::Other => false,
        };
        let cond_ok = self.cond.as_ref().map_or(true, |c| c.matches(env));
        (type_ok && cond_ok) != self.negated
    }
}

impl MediaCond {
    fn matches(&self, env: &MediaEnv) -> bool {
        match self {
            MediaCond::Feature(f) => f.matches(env),
            MediaCond::Not(c) => !c.matches(env),
            MediaCond::And(v) => v.iter().all(|c| c.matches(env)),
            MediaCond::Or(v) => v.iter().any(|c| c.matches(env)),
            MediaCond::Unknown => false,
        }
    }
}

impl Feat {
    fn matches(&self, env: &MediaEnv) -> bool {
        match self {
            Feat::Width(c, v) => c.test(env.width, *v),
            Feat::Height(c, v) => c.test(env.height, *v),
            Feat::Aspect(c, v) => c.test(env.width / env.height.max(1.0), *v),
            Feat::Resolution(c, v) => c.test(1.0, *v),
            Feat::Scheme(dark) => env.dark == *dark,
            Feat::Portrait(p) => (env.height >= env.width) == *p,
            Feat::Const(b) => *b,
        }
    }
}

/// Parses the prelude of `@media`. A bad query becomes `not all`.
fn parse_media_query_list(input: &mut Parser<'_>) -> MediaQueryList {
    let never = MediaQuery { negated: true, media_type: MediaType::All, cond: None };
    if input.is_exhausted() {
        return MediaQueryList(vec![MediaQuery { negated: false, media_type: MediaType::All, cond: None }]);
    }
    let r = input.comma(|i| {
        let q = parse_media_query(i);
        // Drain whatever a failed query left behind.
        while i.next().is_ok() {}
        Ok(q.unwrap_or_else(|_| MediaQuery { negated: true, media_type: MediaType::All, cond: None }))
    });
    MediaQueryList(r.unwrap_or_else(|_| vec![never]))
}

fn media_type_from(s: &str) -> MediaType {
    match s.to_ascii_lowercase().as_str() {
        "all" => MediaType::All,
        "screen" => MediaType::Screen,
        "print" => MediaType::Print,
        _ => MediaType::Other,
    }
}

fn parse_media_query(i: &mut Parser<'_>) -> PResult<MediaQuery> {
    let st = i.state();
    let first = i.next()?.clone();
    let mut negated = false;
    let type_tok = match first {
        Token::Ident(ref s) if s.eq_ignore_ascii_case("not") => {
            if let Ok(t) = i.try_parse(|i| i.expect_ident_cloned()) {
                negated = true;
                Some(t)
            } else {
                i.reset(&st);
                None
            }
        }
        Token::Ident(ref s) if s.eq_ignore_ascii_case("only") => Some(i.expect_ident_cloned()?),
        Token::Ident(ref s) => Some(s.clone()),
        _ => {
            i.reset(&st);
            None
        }
    };
    match type_tok {
        Some(t) => {
            let media_type = media_type_from(&t);
            let cond = if i.try_parse(|i| i.expect_ident_matching("and")).is_ok() {
                Some(parse_media_cond(i, false)?)
            } else {
                None
            };
            Ok(MediaQuery { negated, media_type, cond })
        }
        None => {
            let cond = parse_media_cond(i, true)?;
            Ok(MediaQuery { negated: false, media_type: MediaType::All, cond: Some(cond) })
        }
    }
}

fn parse_media_cond(i: &mut Parser<'_>, allow_or: bool) -> PResult<MediaCond> {
    if i.try_parse(|i| i.expect_ident_matching("not")).is_ok() {
        return Ok(MediaCond::Not(Box::new(parse_media_in_parens(i)?)));
    }
    let first = parse_media_in_parens(i)?;
    let mut ands = vec![first.clone()];
    let mut ors = vec![first];
    loop {
        if i.try_parse(|i| i.expect_ident_matching("and")).is_ok() {
            ands.push(parse_media_in_parens(i)?);
        } else if allow_or && i.try_parse(|i| i.expect_ident_matching("or")).is_ok() {
            ors.push(parse_media_in_parens(i)?);
        } else {
            break;
        }
    }
    if ors.len() > 1 {
        Ok(MediaCond::Or(ors))
    } else if ands.len() > 1 {
        Ok(MediaCond::And(ands))
    } else {
        Ok(ands.pop().unwrap())
    }
}

fn parse_media_in_parens(i: &mut Parser<'_>) -> PResult<MediaCond> {
    let t = i.next()?.clone();
    match t {
        Token::ParenthesisBlock => i.nested(|i| {
            if let Ok(c) = i.try_parse(|i| -> PResult<MediaCond> {
                let c = parse_media_cond(i, true)?;
                i.expect_exhausted()?;
                Ok(c)
            }) {
                return Ok(c);
            }
            parse_media_feature(i)
        }),
        Token::Function(_) => {
            i.nested(|i| {
                while i.next().is_ok() {}
                Ok(())
            })?;
            Ok(MediaCond::Unknown)
        }
        _ => fail(),
    }
}

#[derive(Clone, Debug)]
enum FeatVal {
    Px(f32),
    Num(f32),
    Ratio(f32),
    Res(f32),
    Ident(String),
}

fn parse_feat_val(i: &mut Parser<'_>) -> PResult<FeatVal> {
    let st = i.state();
    let t = i.next()?.clone();
    match t {
        Token::Dimension { value, ref unit, .. } => {
            let u = unit.to_ascii_lowercase();
            match u.as_str() {
                "dppx" | "x" => Ok(FeatVal::Res(value)),
                "dpi" => Ok(FeatVal::Res(value / 96.0)),
                "dpcm" => Ok(FeatVal::Res(value * 2.54 / 96.0)),
                _ => {
                    i.reset(&st);
                    let l = parse_length_pct(i)?;
                    let ctx = LenCtx { font_size: 16.0, root_font_size: 16.0, vw: 0.0, vh: 0.0 };
                    match l.compute(&ctx) {
                        Length::Px(px) => Ok(FeatVal::Px(px)),
                        _ => fail(),
                    }
                }
            }
        }
        Token::Number { value, .. } => {
            if i.try_parse(|i| i.expect_delim('/')).is_ok() {
                let d = i.expect_number()?;
                if d == 0.0 {
                    return fail();
                }
                Ok(FeatVal::Ratio(value / d))
            } else if value == 0.0 {
                Ok(FeatVal::Px(0.0))
            } else {
                Ok(FeatVal::Num(value))
            }
        }
        Token::Ident(ref s) => Ok(FeatVal::Ident(s.to_ascii_lowercase())),
        Token::Function(_) => {
            i.reset(&st);
            let l = parse_length_pct(i)?;
            let ctx = LenCtx { font_size: 16.0, root_font_size: 16.0, vw: 0.0, vh: 0.0 };
            match l.compute(&ctx) {
                Length::Px(px) => Ok(FeatVal::Px(px)),
                _ => fail(),
            }
        }
        _ => fail(),
    }
}

fn parse_cmp(i: &mut Parser<'_>) -> PResult<Cmp> {
    let t = i.next()?.clone();
    match t {
        Token::Delim('=') => Ok(Cmp::Eq),
        Token::Delim('<') => {
            if i.try_parse(|i| i.expect_delim('=')).is_ok() {
                Ok(Cmp::Le)
            } else {
                Ok(Cmp::Lt)
            }
        }
        Token::Delim('>') => {
            if i.try_parse(|i| i.expect_delim('=')).is_ok() {
                Ok(Cmp::Ge)
            } else {
                Ok(Cmp::Gt)
            }
        }
        _ => fail(),
    }
}

fn build_feature(name: &str, cmp: Cmp, v: FeatVal) -> MediaCond {
    let (base, cmp) = if let Some(r) = name.strip_prefix("min-") {
        (r, Cmp::Ge)
    } else if let Some(r) = name.strip_prefix("max-") {
        (r, Cmp::Le)
    } else {
        (name, cmp)
    };
    let f = match (base, v) {
        ("width", FeatVal::Px(px)) | ("device-width", FeatVal::Px(px)) => Feat::Width(cmp, px),
        ("height", FeatVal::Px(px)) | ("device-height", FeatVal::Px(px)) => Feat::Height(cmp, px),
        ("aspect-ratio", FeatVal::Ratio(r)) | ("device-aspect-ratio", FeatVal::Ratio(r)) => Feat::Aspect(cmp, r),
        ("aspect-ratio", FeatVal::Num(r)) => Feat::Aspect(cmp, r),
        ("resolution", FeatVal::Res(r)) => Feat::Resolution(cmp, r),
        ("prefers-color-scheme", FeatVal::Ident(s)) => match s.as_str() {
            "dark" => Feat::Scheme(true),
            "light" => Feat::Scheme(false),
            _ => return MediaCond::Unknown,
        },
        ("orientation", FeatVal::Ident(s)) => match s.as_str() {
            "portrait" => Feat::Portrait(true),
            "landscape" => Feat::Portrait(false),
            _ => return MediaCond::Unknown,
        },
        ("prefers-reduced-motion", FeatVal::Ident(s)) => Feat::Const(s == "no-preference"),
        ("prefers-contrast", FeatVal::Ident(s)) => Feat::Const(s == "no-preference"),
        ("prefers-reduced-transparency", FeatVal::Ident(s)) => Feat::Const(s == "no-preference"),
        ("forced-colors", FeatVal::Ident(s)) => Feat::Const(s == "none"),
        ("hover", FeatVal::Ident(s)) | ("any-hover", FeatVal::Ident(s)) => Feat::Const(s == "none"),
        ("pointer", FeatVal::Ident(s)) | ("any-pointer", FeatVal::Ident(s)) => Feat::Const(s == "coarse"),
        ("display-mode", FeatVal::Ident(s)) => Feat::Const(s == "browser"),
        ("scripting", FeatVal::Ident(s)) => Feat::Const(s == "enabled"),
        ("update", FeatVal::Ident(s)) => Feat::Const(s == "fast"),
        ("color", FeatVal::Num(n)) => Feat::Const(Cmp::test(cmp, 8.0, n)),
        ("color", FeatVal::Px(n)) => Feat::Const(Cmp::test(cmp, 8.0, n)),
        _ => return MediaCond::Unknown,
    };
    MediaCond::Feature(f)
}

fn boolean_feature(name: &str) -> MediaCond {
    match name {
        "color" | "width" | "height" | "orientation" | "pointer" | "any-pointer" | "aspect-ratio" => {
            MediaCond::Feature(Feat::Const(true))
        }
        _ => MediaCond::Feature(Feat::Const(false)),
    }
}

fn parse_media_feature(i: &mut Parser<'_>) -> PResult<MediaCond> {
    let st = i.state();
    let t0 = i.next()?.clone();
    if let Token::Ident(ref name) = t0 {
        let name = name.to_ascii_lowercase();
        if i.try_parse(|i| i.expect_colon()).is_ok() {
            let v = parse_feat_val(i)?;
            return Ok(build_feature(&name, Cmp::Eq, v));
        }
        if let Ok(op) = i.try_parse(parse_cmp) {
            let v = parse_feat_val(i)?;
            return Ok(build_feature(&name, op, v));
        }
        if i.is_exhausted() {
            return Ok(boolean_feature(&name));
        }
        return fail();
    }
    // value op name [op value]
    i.reset(&st);
    let v1 = parse_feat_val(i)?;
    let op1 = parse_cmp(i)?;
    let name = i.expect_ident()?.to_ascii_lowercase();
    // `v1 op1 name` reads as `name flip(op1) v1`.
    let first = build_feature(&name, op1.flip(), v1);
    if let Ok(op2) = i.try_parse(parse_cmp) {
        let v2 = parse_feat_val(i)?;
        let second = build_feature(&name, op2, v2);
        return Ok(MediaCond::And(vec![first, second]));
    }
    Ok(first)
}

// ---------------------------------------------------------------------------
// @supports
// ---------------------------------------------------------------------------

fn parse_supports(i: &mut Parser<'_>) -> PResult<bool> {
    if i.try_parse(|i| i.expect_ident_matching("not")).is_ok() {
        return Ok(!parse_supports_in_parens(i)?);
    }
    let first = parse_supports_in_parens(i)?;
    let mut and = first;
    let mut or = and;
    let mut saw_or = false;
    loop {
        if i.try_parse(|i| i.expect_ident_matching("and")).is_ok() {
            let r = parse_supports_in_parens(i)?;
            and = and && r;
            or = and;
        } else if i.try_parse(|i| i.expect_ident_matching("or")).is_ok() {
            saw_or = true;
            let r = parse_supports_in_parens(i)?;
            or = or || r;
        } else {
            break;
        }
    }
    Ok(if saw_or { or } else { and })
}

fn parse_supports_in_parens(i: &mut Parser<'_>) -> PResult<bool> {
    let t = i.next()?.clone();
    match t {
        Token::ParenthesisBlock => i.nested(|i| {
            if let Ok(b) = i.try_parse(|i| -> PResult<bool> {
                let b = parse_supports(i)?;
                i.expect_exhausted()?;
                Ok(b)
            }) {
                return Ok(b);
            }
            let name = i.expect_ident()?.to_ascii_lowercase();
            i.expect_colon()?;
            while i.next_including_whitespace_and_comments().is_ok() {}
            Ok(name.starts_with("--") || is_known_property(&name))
        }),
        Token::Function(ref name) => {
            let is_selector = name.eq_ignore_ascii_case("selector");
            let text = i.nested(|i| {
                let start = i.position();
                while i.next_including_whitespace_and_comments().is_ok() {}
                Ok(i.slice_from(start).to_string())
            })?;
            Ok(is_selector && parse_selector_list(&text).is_some())
        }
        _ => fail(),
    }
}

// ---------------------------------------------------------------------------
// Property parsing
// ---------------------------------------------------------------------------

const IGNORED_PREFIXES: &[&str] = &[
    "transition",
    "animation",
    "outline",
    "scroll",
    "overscroll",
    "-webkit-",
    "-moz-",
    "-ms-",
];

const IGNORED: &[&str] = &[
    "cursor",
    "will-change",
    "pointer-events",
    "user-select",
    "touch-action",
    "appearance",
    "resize",
    "object-fit",
    "object-position",
    "text-rendering",
    "text-size-adjust",
    "text-wrap",
    "tab-size",
    "caret-color",
    "accent-color",
    "color-scheme",
    "contain",
    "content-visibility",
    "isolation",
    "mix-blend-mode",
    "filter",
    "backdrop-filter",
    "clip-path",
    "mask",
    "fill",
    "stroke",
    "quotes",
    "table-layout",
    "border-collapse",
    "border-spacing",
    "caption-side",
    "empty-cells",
    "font-variant",
    "font-variant-numeric",
    "font-stretch",
    "font-feature-settings",
    "font-kerning",
    "font-display",
    "hyphens",
    "orphans",
    "widows",
    "page-break-inside",
    "break-inside",
    "text-decoration-style",
    "text-decoration-thickness",
    "text-underline-offset",
    "list-style-position",
    "list-style-image",
    "columns",
    "column-count",
    "column-width",
    "column-rule",
    "image-rendering",
    "perspective",
    "transform-style",
    "backface-visibility",
    "transform-box",
    "paint-order",
    "shape-rendering",
    "unicode-bidi",
    "direction",
    "writing-mode",
    "all",
];

fn is_ignored(name: &str) -> bool {
    IGNORED.contains(&name) || IGNORED_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Maps vendor and logical aliases to the canonical property name.
fn canonical(name: &str) -> &str {
    match name {
        "word-wrap" => "overflow-wrap",
        "-webkit-transform" => "transform",
        "-webkit-transform-origin" => "transform-origin",
        "-webkit-box-shadow" => "box-shadow",
        "-webkit-box-sizing" | "-moz-box-sizing" => "box-sizing",
        "grid-gap" => "gap",
        "grid-row-gap" => "row-gap",
        "grid-column-gap" => "column-gap",
        "column-gap" => "column-gap",
        "inline-size" => "width",
        "block-size" => "height",
        "min-inline-size" => "min-width",
        "min-block-size" => "min-height",
        "max-inline-size" => "max-width",
        "max-block-size" => "max-height",
        "margin-inline-start" => "margin-left",
        "margin-inline-end" => "margin-right",
        "margin-block-start" => "margin-top",
        "margin-block-end" => "margin-bottom",
        "padding-inline-start" => "padding-left",
        "padding-inline-end" => "padding-right",
        "padding-block-start" => "padding-top",
        "padding-block-end" => "padding-bottom",
        "inset-inline-start" => "left",
        "inset-inline-end" => "right",
        "inset-block-start" => "top",
        "inset-block-end" => "bottom",
        other => other,
    }
}

const MARGIN: [Prop; 4] = [Prop::MarginTop, Prop::MarginRight, Prop::MarginBottom, Prop::MarginLeft];
const PADDING: [Prop; 4] = [Prop::PaddingTop, Prop::PaddingRight, Prop::PaddingBottom, Prop::PaddingLeft];
const INSET: [Prop; 4] = [Prop::Top, Prop::Right, Prop::Bottom, Prop::Left];
const BORDER_WIDTH: [Prop; 4] =
    [Prop::BorderTopWidth, Prop::BorderRightWidth, Prop::BorderBottomWidth, Prop::BorderLeftWidth];
const BORDER_STYLE: [Prop; 4] =
    [Prop::BorderTopStyle, Prop::BorderRightStyle, Prop::BorderBottomStyle, Prop::BorderLeftStyle];
const BORDER_COLOR: [Prop; 4] =
    [Prop::BorderTopColor, Prop::BorderRightColor, Prop::BorderBottomColor, Prop::BorderLeftColor];
const BORDER_RADIUS: [Prop; 4] = [
    Prop::BorderTopLeftRadius,
    Prop::BorderTopRightRadius,
    Prop::BorderBottomRightRadius,
    Prop::BorderBottomLeftRadius,
];
const BORDER_ALL: [Prop; 12] = [
    Prop::BorderTopWidth,
    Prop::BorderRightWidth,
    Prop::BorderBottomWidth,
    Prop::BorderLeftWidth,
    Prop::BorderTopStyle,
    Prop::BorderRightStyle,
    Prop::BorderBottomStyle,
    Prop::BorderLeftStyle,
    Prop::BorderTopColor,
    Prop::BorderRightColor,
    Prop::BorderBottomColor,
    Prop::BorderLeftColor,
];
const BORDER_TOP: [Prop; 3] = [Prop::BorderTopWidth, Prop::BorderTopStyle, Prop::BorderTopColor];
const BORDER_RIGHT: [Prop; 3] = [Prop::BorderRightWidth, Prop::BorderRightStyle, Prop::BorderRightColor];
const BORDER_BOTTOM: [Prop; 3] = [Prop::BorderBottomWidth, Prop::BorderBottomStyle, Prop::BorderBottomColor];
const BORDER_LEFT: [Prop; 3] = [Prop::BorderLeftWidth, Prop::BorderLeftStyle, Prop::BorderLeftColor];
const BACKGROUND: [Prop; 8] = [
    Prop::BackgroundColor,
    Prop::BackgroundImage,
    Prop::BackgroundRepeat,
    Prop::BackgroundPositionX,
    Prop::BackgroundPositionY,
    Prop::BackgroundSize,
    Prop::BackgroundOrigin,
    Prop::BackgroundClip,
];
const BG_POSITION: [Prop; 2] = [Prop::BackgroundPositionX, Prop::BackgroundPositionY];
const FONT: [Prop; 5] =
    [Prop::FontStyle, Prop::FontWeight, Prop::FontSize, Prop::LineHeight, Prop::FontFamily];
const FLEX: [Prop; 3] = [Prop::FlexGrow, Prop::FlexShrink, Prop::FlexBasis];
const FLEX_FLOW: [Prop; 2] = [Prop::FlexDirection, Prop::FlexWrap];
const GAP: [Prop; 2] = [Prop::RowGap, Prop::ColumnGap];
const GRID_TEMPLATE: [Prop; 3] = [Prop::GridTemplateRows, Prop::GridTemplateColumns, Prop::GridTemplateAreas];
const GRID_AREA: [Prop; 4] =
    [Prop::GridRowStart, Prop::GridColumnStart, Prop::GridRowEnd, Prop::GridColumnEnd];
const GRID_ROW: [Prop; 2] = [Prop::GridRowStart, Prop::GridRowEnd];
const GRID_COLUMN: [Prop; 2] = [Prop::GridColumnStart, Prop::GridColumnEnd];
const PLACE_ITEMS: [Prop; 2] = [Prop::AlignItems, Prop::JustifyItems];
const PLACE_CONTENT: [Prop; 2] = [Prop::AlignContent, Prop::JustifyContent];
const PLACE_SELF: [Prop; 2] = [Prop::AlignSelf, Prop::JustifySelf];
const OVERFLOW: [Prop; 2] = [Prop::OverflowX, Prop::OverflowY];
const TEXT_DECORATION: [Prop; 2] = [Prop::TextDecorationLine, Prop::TextDecorationColor];
const LIST_STYLE: [Prop; 1] = [Prop::ListStyleType];
const MARGIN_BLOCK: [Prop; 2] = [Prop::MarginTop, Prop::MarginBottom];
const MARGIN_INLINE: [Prop; 2] = [Prop::MarginLeft, Prop::MarginRight];
const PADDING_BLOCK: [Prop; 2] = [Prop::PaddingTop, Prop::PaddingBottom];
const PADDING_INLINE: [Prop; 2] = [Prop::PaddingLeft, Prop::PaddingRight];
const INSET_BLOCK: [Prop; 2] = [Prop::Top, Prop::Bottom];
const INSET_INLINE: [Prop; 2] = [Prop::Left, Prop::Right];

/// The longhands a shorthand sets (all of them for the CSS-wide keywords).
fn shorthand_longhands(name: &str) -> Option<&'static [Prop]> {
    Some(match name {
        "margin" => &MARGIN[..],
        "padding" => &PADDING[..],
        "inset" => &INSET[..],
        "margin-block" => &MARGIN_BLOCK[..],
        "margin-inline" => &MARGIN_INLINE[..],
        "padding-block" => &PADDING_BLOCK[..],
        "padding-inline" => &PADDING_INLINE[..],
        "inset-block" => &INSET_BLOCK[..],
        "inset-inline" => &INSET_INLINE[..],
        "border" => &BORDER_ALL[..],
        "border-width" => &BORDER_WIDTH[..],
        "border-style" => &BORDER_STYLE[..],
        "border-color" => &BORDER_COLOR[..],
        "border-radius" => &BORDER_RADIUS[..],
        "border-top" => &BORDER_TOP[..],
        "border-right" => &BORDER_RIGHT[..],
        "border-bottom" => &BORDER_BOTTOM[..],
        "border-left" => &BORDER_LEFT[..],
        "background" => &BACKGROUND[..],
        "background-position" => &BG_POSITION[..],
        "font" => &FONT[..],
        "flex" => &FLEX[..],
        "flex-flow" => &FLEX_FLOW[..],
        "gap" => &GAP[..],
        "grid-template" | "grid" => &GRID_TEMPLATE[..],
        "grid-area" => &GRID_AREA[..],
        "grid-row" => &GRID_ROW[..],
        "grid-column" => &GRID_COLUMN[..],
        "place-items" => &PLACE_ITEMS[..],
        "place-content" => &PLACE_CONTENT[..],
        "place-self" => &PLACE_SELF[..],
        "overflow" => &OVERFLOW[..],
        "text-decoration" => &TEXT_DECORATION[..],
        "list-style" => &LIST_STYLE[..],
        _ => return None,
    })
}

/// The longhands that `name` (a longhand or shorthand) sets; empty when unknown.
pub fn longhands_of(name: &str) -> Vec<Prop> {
    let name = canonical(name);
    if let Some(p) = Prop::from_name(name) {
        return vec![p];
    }
    shorthand_longhands(name).map(|l| l.to_vec()).unwrap_or_default()
}

/// Whether `name` is a property this engine understands (or deliberately
/// ignores). `@supports` uses this.
pub fn is_known_property(name: &str) -> bool {
    let name = canonical(name);
    Prop::from_name(name).is_some() || shorthand_longhands(name).is_some() || is_ignored(name)
}

/// Parses `name: text` into longhand values. `None` when the declaration is
/// invalid; `Some(vec![])` when the property is known but ignored. `name`
/// must be lowercase. `text` must not contain `!important`.
pub fn parse_property(name: &str, text: &str) -> Option<Vec<(Prop, Value)>> {
    let name = canonical(name);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let wide = match trimmed.to_ascii_lowercase().as_str() {
        "inherit" => Some(Value::Inherit),
        "initial" => Some(Value::Initial),
        "unset" | "revert" | "revert-layer" => Some(Value::Unset),
        _ => None,
    };
    if let Some(v) = wide {
        if let Some(p) = Prop::from_name(name) {
            return Some(vec![(p, v)]);
        }
        if let Some(list) = shorthand_longhands(name) {
            return Some(list.iter().map(|p| (*p, v.clone())).collect());
        }
        return if is_ignored(name) { Some(Vec::new()) } else { None };
    }
    if Prop::from_name(name).is_none() && shorthand_longhands(name).is_none() {
        return if is_ignored(name) { Some(Vec::new()) } else { None };
    }
    let mut input = Parser::new(trimmed);
    input.parse_entirely(|i| parse_known(name, i)).ok()
}

fn zip(props: &[Prop], values: Vec<Value>) -> Vec<(Prop, Value)> {
    props.iter().copied().zip(values).collect()
}

fn expand4<T: Clone>(v: &[T]) -> [T; 4] {
    match v.len() {
        1 => [v[0].clone(), v[0].clone(), v[0].clone(), v[0].clone()],
        2 => [v[0].clone(), v[1].clone(), v[0].clone(), v[1].clone()],
        3 => [v[0].clone(), v[1].clone(), v[2].clone(), v[1].clone()],
        _ => [v[0].clone(), v[1].clone(), v[2].clone(), v[3].clone()],
    }
}

fn parse_up_to4<T>(i: &mut Parser<'_>, f: fn(&mut Parser<'_>) -> PResult<T>) -> PResult<Vec<T>> {
    let mut out = vec![f(i)?];
    while out.len() < 4 {
        match i.try_parse(f) {
            Ok(v) => out.push(v),
            Err(_) => break,
        }
    }
    Ok(out)
}

fn kind_auto(i: &mut Parser<'_>) -> PResult<Length> {
    parse_length_kw(i, LenKw::AUTO)
}

fn kind_plain(i: &mut Parser<'_>) -> PResult<Length> {
    parse_length_pct(i)
}

fn parse_border_width(i: &mut Parser<'_>) -> PResult<Length> {
    if let Ok(id) = i.try_parse(|i| i.expect_ident_cloned()) {
        return match id.to_ascii_lowercase().as_str() {
            "thin" => Ok(Length::Px(1.0)),
            "medium" => Ok(Length::Px(3.0)),
            "thick" => Ok(Length::Px(5.0)),
            _ => fail(),
        };
    }
    let l = parse_length_pct(i)?;
    match l {
        Length::Dim(v, _) if v < 0.0 => fail(),
        Length::Percent(_) | Length::Dim(_, Unit::Percent) => fail(),
        other => Ok(other),
    }
}

fn parse_border_style(i: &mut Parser<'_>) -> PResult<Value> {
    parse_kw::<BorderStyle>(i)
}

fn color_value(i: &mut Parser<'_>) -> PResult<Value> {
    Ok(Value::Color(parse_color(i)?))
}

fn box_shorthand(
    i: &mut Parser<'_>,
    props: &[Prop; 4],
    f: fn(&mut Parser<'_>) -> PResult<Length>,
) -> PResult<Vec<(Prop, Value)>> {
    let v = parse_up_to4(i, f)?;
    let e = expand4(&v);
    Ok(zip(props, e.into_iter().map(Value::Length).collect()))
}

fn pair_shorthand(
    i: &mut Parser<'_>,
    props: &[Prop; 2],
    f: fn(&mut Parser<'_>) -> PResult<Length>,
) -> PResult<Vec<(Prop, Value)>> {
    let a = f(i)?;
    let b = i.try_parse(f).unwrap_or_else(|_| a.clone());
    Ok(zip(props, vec![Value::Length(a), Value::Length(b)]))
}

type Parts = (Option<Length>, Option<Value>, Option<Value>);

fn parse_border_parts(i: &mut Parser<'_>) -> PResult<Parts> {
    let mut width = None;
    let mut style = None;
    let mut color = None;
    loop {
        if width.is_none() {
            if let Ok(w) = i.try_parse(parse_border_width) {
                width = Some(w);
                continue;
            }
        }
        if style.is_none() {
            if let Ok(s) = i.try_parse(parse_border_style) {
                style = Some(s);
                continue;
            }
        }
        if color.is_none() {
            if let Ok(c) = i.try_parse(color_value) {
                color = Some(c);
                continue;
            }
        }
        break;
    }
    if width.is_none() && style.is_none() && color.is_none() {
        return fail();
    }
    Ok((width, style, color))
}

fn border_side(parts: Parts, props: &[Prop]) -> Vec<(Prop, Value)> {
    let (w, s, c) = parts;
    vec![
        (props[0], w.map(Value::Length).unwrap_or(Value::Initial)),
        (props[1], s.unwrap_or(Value::Initial)),
        (props[2], c.unwrap_or(Value::Initial)),
    ]
}

fn parse_radius(i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    let h = parse_up_to4(i, kind_plain)?;
    let v = if i.try_parse(|i| i.expect_delim('/')).is_ok() { parse_up_to4(i, kind_plain)? } else { h.clone() };
    let h = expand4(&h);
    let v = expand4(&v);
    let values = h.into_iter().zip(v).map(|(a, b)| Value::Pair(a, b)).collect();
    Ok(zip(&BORDER_RADIUS, values))
}

fn parse_repeat_style(i: &mut Parser<'_>) -> PResult<(BgRepeatKind, BgRepeatKind)> {
    let id = i.expect_ident()?.to_ascii_lowercase();
    match id.as_str() {
        "repeat-x" => return Ok((BgRepeatKind::Repeat, BgRepeatKind::NoRepeat)),
        "repeat-y" => return Ok((BgRepeatKind::NoRepeat, BgRepeatKind::Repeat)),
        _ => {}
    }
    let a = match BgRepeatKind::from_ident(&id) {
        Some(a) => a,
        None => return fail(),
    };
    let b = i
        .try_parse(|i| -> PResult<BgRepeatKind> {
            let id = i.expect_ident()?.to_ascii_lowercase();
            match BgRepeatKind::from_ident(&id) {
                Some(b) => Ok(b),
                None => fail(),
            }
        })
        .unwrap_or(a);
    Ok((a, b))
}

fn parse_bg_size(i: &mut Parser<'_>) -> PResult<BgSize> {
    if let Ok(id) = i.try_parse(|i| i.expect_ident_cloned()) {
        return match id.to_ascii_lowercase().as_str() {
            "cover" => Ok(BgSize::Cover),
            "contain" => Ok(BgSize::Contain),
            "auto" => {
                let b = i.try_parse(kind_auto).unwrap_or(Length::Auto);
                Ok(BgSize::Explicit(Length::Auto, b))
            }
            _ => fail(),
        };
    }
    let a = parse_length_pct(i)?;
    let b = i.try_parse(kind_auto).unwrap_or(Length::Auto);
    Ok(BgSize::Explicit(a, b))
}

fn parse_bg_box(i: &mut Parser<'_>) -> PResult<BgBox> {
    let id = i.expect_ident()?.to_ascii_lowercase();
    match BgBox::from_ident(&id) {
        Some(b) => Ok(b),
        None => fail(),
    }
}

struct BgLayerParts {
    image: BgImage,
    repeat: (BgRepeatKind, BgRepeatKind),
    pos: (Length, Length),
    size: BgSize,
    origin: BgBox,
    clip: BgBox,
    color: Option<Color>,
}

fn parse_bg_layer(i: &mut Parser<'_>) -> PResult<BgLayerParts> {
    let pct0 = Length::Dim(0.0, Unit::Percent);
    let mut layer = BgLayerParts {
        image: BgImage::None,
        repeat: (BgRepeatKind::Repeat, BgRepeatKind::Repeat),
        pos: (pct0.clone(), pct0),
        size: BgSize::Explicit(Length::Auto, Length::Auto),
        origin: BgBox::PaddingBox,
        clip: BgBox::BorderBox,
        color: None,
    };
    let (mut has_image, mut has_repeat, mut has_pos, mut has_color) = (false, false, false, false);
    let mut boxes: Vec<BgBox> = Vec::new();
    let mut progress = false;
    loop {
        if !has_image {
            if let Ok(img) = i.try_parse(parse_bg_image) {
                layer.image = img;
                has_image = true;
                progress = true;
                continue;
            }
        }
        if !has_repeat {
            if let Ok(r) = i.try_parse(parse_repeat_style) {
                layer.repeat = r;
                has_repeat = true;
                progress = true;
                continue;
            }
        }
        if i.try_parse(|i| -> PResult<()> {
            let id = i.expect_ident()?.to_ascii_lowercase();
            match id.as_str() {
                "scroll" | "fixed" | "local" => Ok(()),
                _ => fail(),
            }
        })
        .is_ok()
        {
            progress = true;
            continue;
        }
        if boxes.len() < 2 {
            if let Ok(b) = i.try_parse(parse_bg_box) {
                boxes.push(b);
                progress = true;
                continue;
            }
        }
        if !has_color {
            if let Ok(c) = i.try_parse(parse_color) {
                layer.color = Some(c);
                has_color = true;
                progress = true;
                continue;
            }
        }
        if !has_pos {
            if let Ok(p) = i.try_parse(parse_position) {
                layer.pos = p;
                has_pos = true;
                progress = true;
                if i.try_parse(|i| i.expect_delim('/')).is_ok() {
                    layer.size = parse_bg_size(i)?;
                }
                continue;
            }
        }
        break;
    }
    if !progress {
        return fail();
    }
    match boxes.len() {
        1 => {
            layer.origin = boxes[0];
            layer.clip = boxes[0];
        }
        2 => {
            layer.origin = boxes[0];
            layer.clip = boxes[1];
        }
        _ => {}
    }
    Ok(layer)
}

fn parse_background(i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    let layers = i.comma(parse_bg_layer)?;
    let color = layers.last().and_then(|l| l.color).unwrap_or(Color::Rgba(Rgba::TRANSPARENT));
    let images = layers.iter().map(|l| l.image.clone()).collect();
    let repeats = layers.iter().map(|l| l.repeat).collect();
    let xs = layers.iter().map(|l| l.pos.0.clone()).collect();
    let ys = layers.iter().map(|l| l.pos.1.clone()).collect();
    let sizes = layers.iter().map(|l| l.size.clone()).collect();
    let origins = layers.iter().map(|l| l.origin).collect();
    let clips = layers.iter().map(|l| l.clip).collect();
    Ok(zip(
        &BACKGROUND,
        vec![
            Value::Color(color),
            Value::Images(images),
            Value::Repeats(repeats),
            Value::Lengths(xs),
            Value::Lengths(ys),
            Value::Sizes(sizes),
            Value::Boxes(origins),
            Value::Boxes(clips),
        ],
    ))
}

fn parse_font_size(i: &mut Parser<'_>) -> PResult<Value> {
    if let Ok(id) = i.try_parse(|i| i.expect_ident_cloned()) {
        let l = id.to_ascii_lowercase();
        return match l.as_str() {
            "xx-small" | "x-small" | "small" | "medium" | "large" | "x-large" | "xx-large" | "xxx-large"
            | "larger" | "smaller" => Ok(Value::Ident(l.clone())),
            _ => fail(),
        };
    }
    let l = parse_length_pct(i)?;
    if let Length::Dim(v, _) = l {
        if v < 0.0 {
            return fail();
        }
    }
    Ok(Value::Length(l))
}

fn parse_font_weight(i: &mut Parser<'_>) -> PResult<Value> {
    if let Ok(id) = i.try_parse(|i| i.expect_ident_cloned()) {
        let l = id.to_ascii_lowercase();
        return match l.as_str() {
            "normal" | "bold" | "bolder" | "lighter" => Ok(Value::Ident(l.clone())),
            _ => fail(),
        };
    }
    let n = parse_number(i)?;
    if (1.0..=1000.0).contains(&n) {
        Ok(Value::Number(n))
    } else {
        fail()
    }
}

fn parse_line_height(i: &mut Parser<'_>) -> PResult<Value> {
    if i.try_parse(|i| i.expect_ident_matching("normal")).is_ok() {
        return Ok(Value::Ident("normal".to_string()));
    }
    if let Ok(n) = i.try_parse(|i| i.expect_number()) {
        return if n >= 0.0 { Ok(Value::Number(n)) } else { fail() };
    }
    let l = parse_length_pct(i)?;
    if let Length::Dim(v, _) = l {
        if v < 0.0 {
            return fail();
        }
    }
    Ok(Value::Length(l))
}

fn parse_font_shorthand(i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    let mut style = None;
    let mut weight = None;
    for _ in 0..4 {
        let st = i.state();
        if let Ok(id) = i.try_parse(|i| i.expect_ident_cloned()) {
            let l = id.to_ascii_lowercase();
            match l.as_str() {
                "normal" | "small-caps" | "ultra-condensed" | "extra-condensed" | "condensed"
                | "semi-condensed" | "semi-expanded" | "expanded" | "extra-expanded" | "ultra-expanded" => continue,
                "italic" | "oblique" => {
                    style = Some(Value::Ident(l.clone()));
                    continue;
                }
                "bold" | "bolder" | "lighter" => {
                    weight = Some(Value::Ident(l.clone()));
                    continue;
                }
                _ => {
                    i.reset(&st);
                }
            }
        }
        if let Ok(n) = i.try_parse(|i| i.expect_number()) {
            if (1.0..=1000.0).contains(&n) {
                weight = Some(Value::Number(n));
                continue;
            }
            return fail();
        }
        break;
    }
    let size = parse_font_size(i)?;
    let line_height = if i.try_parse(|i| i.expect_delim('/')).is_ok() {
        parse_line_height(i)?
    } else {
        Value::Ident("normal".to_string())
    };
    let family = parse_font_family(i)?;
    Ok(zip(
        &FONT,
        vec![
            style.unwrap_or_else(|| Value::Ident("normal".to_string())),
            weight.unwrap_or(Value::Number(400.0)),
            size,
            line_height,
            Value::Families(family),
        ],
    ))
}

fn parse_flex(i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    if let Ok(id) = i.try_parse(|i| i.expect_ident_cloned()) {
        let (g, s, b) = match id.to_ascii_lowercase().as_str() {
            "none" => (0.0, 0.0, Length::Auto),
            "auto" => (1.0, 1.0, Length::Auto),
            "initial" => (0.0, 1.0, Length::Auto),
            _ => return fail(),
        };
        return Ok(zip(&FLEX, vec![Value::Number(g), Value::Number(s), Value::Length(b)]));
    }
    let mut nums: Vec<f32> = Vec::new();
    let mut basis: Option<Length> = None;
    loop {
        if nums.len() < 2 {
            if let Ok(n) = i.try_parse(|i| i.expect_number()) {
                if n < 0.0 {
                    return fail();
                }
                nums.push(n);
                continue;
            }
        }
        if basis.is_none() {
            if let Ok(l) = i.try_parse(|i| parse_length_kw(i, LenKw::SIZE)) {
                basis = Some(l);
                continue;
            }
        }
        break;
    }
    if nums.is_empty() && basis.is_none() {
        return fail();
    }
    let grow = nums.first().copied().unwrap_or(1.0);
    let shrink = nums.get(1).copied().unwrap_or(1.0);
    let basis = basis.unwrap_or(Length::Px(0.0));
    Ok(zip(&FLEX, vec![Value::Number(grow), Value::Number(shrink), Value::Length(basis)]))
}

fn parse_flex_flow(i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    let mut dir = None;
    let mut wrap = None;
    loop {
        if dir.is_none() {
            if let Ok(v) = i.try_parse(parse_kw::<FlexDirection>) {
                dir = Some(v);
                continue;
            }
        }
        if wrap.is_none() {
            if let Ok(v) = i.try_parse(parse_kw::<FlexWrap>) {
                wrap = Some(v);
                continue;
            }
        }
        break;
    }
    if dir.is_none() && wrap.is_none() {
        return fail();
    }
    Ok(zip(&FLEX_FLOW, vec![dir.unwrap_or(Value::Initial), wrap.unwrap_or(Value::Initial)]))
}

fn parse_gap_value(i: &mut Parser<'_>) -> PResult<Length> {
    if i.try_parse(|i| i.expect_ident_matching("normal")).is_ok() {
        return Ok(Length::Px(0.0));
    }
    parse_length_pct(i)
}

fn parse_place(i: &mut Parser<'_>, props: &[Prop; 2]) -> PResult<Vec<(Prop, Value)>> {
    let a = parse_kw::<Align>(i)?;
    let b = i.try_parse(parse_kw::<Align>).unwrap_or_else(|_| a.clone());
    Ok(zip(props, vec![a, b]))
}

fn parse_overflow(i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    let a = parse_kw::<Overflow>(i)?;
    let b = i.try_parse(parse_kw::<Overflow>).unwrap_or_else(|_| a.clone());
    Ok(zip(&OVERFLOW, vec![a, b]))
}

fn parse_text_decoration_line(i: &mut Parser<'_>) -> PResult<DecoLine> {
    if i.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(DecoLine::default());
    }
    let mut d = DecoLine::default();
    let mut any = false;
    loop {
        let st = i.state();
        let id = match i.try_parse(|i| i.expect_ident_cloned()) {
            Ok(id) => id,
            Err(_) => break,
        };
        match id.to_ascii_lowercase().as_str() {
            "underline" => d.underline = true,
            "overline" => d.overline = true,
            "line-through" => d.line_through = true,
            "blink" => {}
            _ => {
                i.reset(&st);
                break;
            }
        }
        any = true;
    }
    if any {
        Ok(d)
    } else {
        fail()
    }
}

fn parse_text_decoration(i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    let mut line: Option<DecoLine> = None;
    let mut color = None;
    let mut progress = false;
    loop {
        if line.is_none() {
            if let Ok(l) = i.try_parse(parse_text_decoration_line) {
                line = Some(l);
                progress = true;
                continue;
            }
        }
        if color.is_none() {
            if let Ok(c) = i.try_parse(parse_color) {
                color = Some(c);
                progress = true;
                continue;
            }
        }
        // style and thickness are accepted and ignored
        if i.try_parse(|i| -> PResult<()> {
            let t = i.next()?.clone();
            match t {
                Token::Ident(ref s) => match s.to_ascii_lowercase().as_str() {
                    "solid" | "double" | "dotted" | "dashed" | "wavy" | "auto" | "from-font" => Ok(()),
                    _ => fail(),
                },
                Token::Dimension { .. } | Token::Percentage { .. } => Ok(()),
                _ => fail(),
            }
        })
        .is_ok()
        {
            progress = true;
            continue;
        }
        break;
    }
    if !progress {
        return fail();
    }
    Ok(zip(
        &TEXT_DECORATION,
        vec![
            Value::DecoLine(line.unwrap_or_default()),
            color.map(Value::Color).unwrap_or(Value::Initial),
        ],
    ))
}

fn parse_list_style(i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    let mut ty = None;
    let mut progress = false;
    loop {
        if ty.is_none() {
            if let Ok(v) = i.try_parse(parse_kw::<ListStyleType>) {
                ty = Some(v);
                progress = true;
                continue;
            }
        }
        if i.try_parse(|i| -> PResult<()> {
            let t = i.next()?.clone();
            match t {
                Token::Ident(ref s) if s.eq_ignore_ascii_case("inside") || s.eq_ignore_ascii_case("outside") => Ok(()),
                Token::UnquotedUrl(_) => Ok(()),
                Token::Function(ref n) if n.eq_ignore_ascii_case("url") => {
                    i.nested(|i| {
                        while i.next().is_ok() {}
                        Ok(())
                    })?;
                    Ok(())
                }
                _ => fail(),
            }
        })
        .is_ok()
        {
            progress = true;
            continue;
        }
        break;
    }
    if !progress {
        return fail();
    }
    Ok(zip(&LIST_STYLE, vec![ty.unwrap_or(Value::Initial)]))
}

fn parse_grid_areas_strings(i: &mut Parser<'_>) -> PResult<Vec<Vec<String>>> {
    let mut rows = Vec::new();
    while let Ok(s) = i.try_parse(|i| i.expect_string_cloned()) {
        rows.push(s.split_whitespace().map(|c| c.to_string()).collect::<Vec<_>>());
    }
    if rows.is_empty() {
        return fail();
    }
    Ok(rows)
}

fn parse_grid_template(i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    if i.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
        return Ok(zip(
            &GRID_TEMPLATE,
            vec![Value::Tracks(Vec::new()), Value::Tracks(Vec::new()), Value::Areas(Vec::new())],
        ));
    }
    let first_is_string = {
        let st = i.state();
        let r = matches!(i.next(), Ok(Token::QuotedString(_)));
        i.reset(&st);
        r
    };
    let mut rows: Vec<TrackItem> = Vec::new();
    let mut areas: Vec<Vec<String>> = Vec::new();
    if first_is_string {
        // "a b" 100px "c d" auto / 1fr 1fr
        while let Ok(s) = i.try_parse(|i| i.expect_string_cloned()) {
            areas.push(s.split_whitespace().map(|c| c.to_string()).collect());
            let size = i
                .try_parse(|i| -> PResult<Value> {
                    let l = parse_track_list(i)?;
                    Ok(Value::Tracks(l))
                })
                .ok();
            match size {
                Some(Value::Tracks(mut items)) => rows.append(&mut items),
                _ => rows.push(TrackItem::Size(TrackSize::Len(Length::Auto))),
            }
        }
    } else {
        rows = parse_track_list(i)?;
    }
    let cols = if i.try_parse(|i| i.expect_delim('/')).is_ok() { parse_track_list(i)? } else { Vec::new() };
    Ok(zip(&GRID_TEMPLATE, vec![Value::Tracks(rows), Value::Tracks(cols), Value::Areas(areas)]))
}

fn parse_grid_lines(i: &mut Parser<'_>, max: usize) -> PResult<Vec<GridLine>> {
    let mut v = vec![parse_grid_line(i)?];
    while v.len() < max {
        if i.try_parse(|i| i.expect_delim('/')).is_err() {
            break;
        }
        v.push(parse_grid_line(i)?);
    }
    Ok(v)
}

fn named_or_auto(l: &GridLine) -> GridLine {
    match l {
        GridLine::Named(n) => GridLine::Named(n.clone()),
        _ => GridLine::Auto,
    }
}

fn parse_grid_area(i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    let v = parse_grid_lines(i, 4)?;
    let rs = v[0].clone();
    let cs = v.get(1).cloned().unwrap_or_else(|| named_or_auto(&rs));
    let re = v.get(2).cloned().unwrap_or_else(|| named_or_auto(&rs));
    let ce = v.get(3).cloned().unwrap_or_else(|| named_or_auto(&cs));
    Ok(zip(&GRID_AREA, vec![Value::Line(rs), Value::Line(cs), Value::Line(re), Value::Line(ce)]))
}

fn parse_grid_span(i: &mut Parser<'_>, props: &[Prop; 2]) -> PResult<Vec<(Prop, Value)>> {
    let v = parse_grid_lines(i, 2)?;
    let start = v[0].clone();
    let end = v.get(1).cloned().unwrap_or_else(|| named_or_auto(&start));
    Ok(zip(props, vec![Value::Line(start), Value::Line(end)]))
}

fn parse_aspect_ratio(i: &mut Parser<'_>) -> PResult<Value> {
    let auto = i.try_parse(|i| i.expect_ident_matching("auto")).is_ok();
    match i.try_parse(|i| i.expect_number()) {
        Ok(w) => {
            let h = if i.try_parse(|i| i.expect_delim('/')).is_ok() { i.expect_number()? } else { 1.0 };
            if w <= 0.0 || h <= 0.0 {
                return fail();
            }
            Ok(Value::Aspect(Some(w / h)))
        }
        Err(_) if auto => Ok(Value::Aspect(None)),
        Err(_) => fail(),
    }
}

fn parse_bg_pos_axis(i: &mut Parser<'_>, horizontal: bool) -> PResult<Length> {
    if let Ok(id) = i.try_parse(|i| i.expect_ident_cloned()) {
        let pct = |v: f32| Length::Dim(v, Unit::Percent);
        return match (id.to_ascii_lowercase().as_str(), horizontal) {
            ("left", true) | ("top", false) => Ok(pct(0.0)),
            ("center", _) => Ok(pct(50.0)),
            ("right", true) | ("bottom", false) => Ok(pct(100.0)),
            _ => fail(),
        };
    }
    parse_length_pct(i)
}

fn parse_longhand(p: Prop, i: &mut Parser<'_>) -> PResult<Value> {
    match p {
        Prop::Display => parse_kw::<Display>(i),
        Prop::Position => parse_kw::<Position>(i),
        Prop::Float => parse_kw::<Float>(i),
        Prop::Clear => parse_kw::<Clear>(i),
        Prop::BoxSizing => parse_kw::<BoxSizing>(i),
        Prop::Visibility => parse_kw::<Visibility>(i),
        Prop::OverflowX | Prop::OverflowY => parse_kw::<Overflow>(i),
        Prop::ZIndex => {
            if i.try_parse(|i| i.expect_ident_matching("auto")).is_ok() {
                Ok(Value::Ident("auto".to_string()))
            } else {
                Ok(Value::Int(i.expect_integer()?))
            }
        }
        Prop::Opacity => {
            let t = i.next()?.clone();
            match t {
                Token::Number { value, .. } => Ok(Value::Number(value.clamp(0.0, 1.0))),
                Token::Percentage { unit_value, .. } => Ok(Value::Number(unit_value.clamp(0.0, 1.0))),
                _ => fail(),
            }
        }
        Prop::Width | Prop::Height | Prop::FlexBasis => Ok(Value::Length(parse_length_kw(i, LenKw::SIZE)?)),
        Prop::MinWidth | Prop::MinHeight => Ok(Value::Length(parse_length_kw(i, LenKw::SIZE)?)),
        Prop::MaxWidth | Prop::MaxHeight => Ok(Value::Length(parse_length_kw(i, LenKw::MAX)?)),
        Prop::MarginTop
        | Prop::MarginRight
        | Prop::MarginBottom
        | Prop::MarginLeft
        | Prop::Top
        | Prop::Right
        | Prop::Bottom
        | Prop::Left => Ok(Value::Length(parse_length_kw(i, LenKw::AUTO)?)),
        Prop::PaddingTop | Prop::PaddingRight | Prop::PaddingBottom | Prop::PaddingLeft | Prop::TextIndent => {
            Ok(Value::Length(parse_length_pct(i)?))
        }
        Prop::RowGap | Prop::ColumnGap => Ok(Value::Length(parse_gap_value(i)?)),
        Prop::BorderTopWidth | Prop::BorderRightWidth | Prop::BorderBottomWidth | Prop::BorderLeftWidth => {
            Ok(Value::Length(parse_border_width(i)?))
        }
        Prop::BorderTopStyle | Prop::BorderRightStyle | Prop::BorderBottomStyle | Prop::BorderLeftStyle => {
            parse_kw::<BorderStyle>(i)
        }
        Prop::BorderTopColor
        | Prop::BorderRightColor
        | Prop::BorderBottomColor
        | Prop::BorderLeftColor
        | Prop::Color
        | Prop::BackgroundColor
        | Prop::TextDecorationColor => color_value(i),
        Prop::BorderTopLeftRadius
        | Prop::BorderTopRightRadius
        | Prop::BorderBottomRightRadius
        | Prop::BorderBottomLeftRadius => {
            let a = parse_length_pct(i)?;
            let b = i.try_parse(parse_length_pct).unwrap_or_else(|_| a.clone());
            Ok(Value::Pair(a, b))
        }
        Prop::BackgroundImage => Ok(Value::Images(i.comma(parse_bg_image)?)),
        Prop::BackgroundRepeat => Ok(Value::Repeats(i.comma(parse_repeat_style)?)),
        Prop::BackgroundPositionX => Ok(Value::Lengths(i.comma(|i| parse_bg_pos_axis(i, true))?)),
        Prop::BackgroundPositionY => Ok(Value::Lengths(i.comma(|i| parse_bg_pos_axis(i, false))?)),
        Prop::BackgroundSize => Ok(Value::Sizes(i.comma(parse_bg_size)?)),
        Prop::BackgroundClip | Prop::BackgroundOrigin => Ok(Value::Boxes(i.comma(parse_bg_box)?)),
        Prop::FontFamily => Ok(Value::Families(parse_font_family(i)?)),
        Prop::FontSize => parse_font_size(i),
        Prop::FontWeight => parse_font_weight(i),
        Prop::FontStyle => parse_kw::<FontStyle>(i),
        Prop::LineHeight => parse_line_height(i),
        Prop::LetterSpacing | Prop::WordSpacing => {
            if i.try_parse(|i| i.expect_ident_matching("normal")).is_ok() {
                Ok(Value::Ident("normal".to_string()))
            } else {
                Ok(Value::Length(parse_length_pct(i)?))
            }
        }
        Prop::TextAlign => parse_kw::<TextAlign>(i),
        Prop::TextDecorationLine => Ok(Value::DecoLine(parse_text_decoration_line(i)?)),
        Prop::TextTransform => parse_kw::<TextTransform>(i),
        Prop::WhiteSpace => parse_kw::<WhiteSpace>(i),
        Prop::VerticalAlign => {
            if let Ok(v) = i.try_parse(parse_kw::<VAlignKw>) {
                Ok(v)
            } else {
                Ok(Value::Length(parse_length_pct(i)?))
            }
        }
        Prop::TextOverflow => parse_kw::<TextOverflow>(i),
        Prop::WordBreak => parse_kw::<WordBreak>(i),
        Prop::OverflowWrap => parse_kw::<OverflowWrap>(i),
        Prop::ListStyleType => parse_kw::<ListStyleType>(i),
        Prop::TextShadow | Prop::BoxShadow => Ok(Value::Shadows(parse_shadow_list(i)?)),
        Prop::Content => Ok(Value::Content(parse_content(i)?)),
        Prop::Transform => Ok(Value::Transform(parse_transform(i)?)),
        Prop::TransformOrigin => {
            let (x, y) = parse_position(i)?;
            let _ = i.try_parse(parse_length_pct);
            Ok(Value::Pair(x, y))
        }
        Prop::AspectRatio => parse_aspect_ratio(i),
        Prop::FlexDirection => parse_kw::<FlexDirection>(i),
        Prop::FlexWrap => parse_kw::<FlexWrap>(i),
        Prop::FlexGrow | Prop::FlexShrink => {
            let n = i.expect_number()?;
            if n < 0.0 {
                fail()
            } else {
                Ok(Value::Number(n))
            }
        }
        Prop::JustifyContent
        | Prop::AlignItems
        | Prop::AlignSelf
        | Prop::AlignContent
        | Prop::JustifyItems
        | Prop::JustifySelf => parse_kw::<Align>(i),
        Prop::Order => Ok(Value::Int(i.expect_integer()?)),
        Prop::GridTemplateColumns | Prop::GridTemplateRows => Ok(Value::Tracks(parse_track_list(i)?)),
        Prop::GridTemplateAreas => {
            if i.try_parse(|i| i.expect_ident_matching("none")).is_ok() {
                Ok(Value::Areas(Vec::new()))
            } else {
                Ok(Value::Areas(parse_grid_areas_strings(i)?))
            }
        }
        Prop::GridAutoFlow => {
            let mut words: Vec<String> = Vec::new();
            while let Ok(id) = i.try_parse(|i| i.expect_ident_cloned()) {
                words.push(id.to_ascii_lowercase());
            }
            let key = match words.as_slice() {
                [a] if a == "row" || a == "column" => a.clone(),
                [a] if a == "dense" => "row dense".to_string(),
                [a, b] if (a == "row" || a == "column") && b == "dense" => format!("{} dense", a),
                [a, b] if a == "dense" && (b == "row" || b == "column") => format!("{} dense", b),
                _ => return fail(),
            };
            Ok(Value::Ident(key))
        }
        Prop::GridAutoColumns | Prop::GridAutoRows => Ok(Value::TrackSizes(parse_track_sizes(i)?)),
        Prop::GridColumnStart | Prop::GridColumnEnd | Prop::GridRowStart | Prop::GridRowEnd => {
            Ok(Value::Line(parse_grid_line(i)?))
        }
    }
}

fn parse_known(name: &str, i: &mut Parser<'_>) -> PResult<Vec<(Prop, Value)>> {
    if let Some(p) = Prop::from_name(name) {
        return Ok(vec![(p, parse_longhand(p, i)?)]);
    }
    match name {
        "margin" => box_shorthand(i, &MARGIN, kind_auto),
        "padding" => box_shorthand(i, &PADDING, kind_plain),
        "inset" => box_shorthand(i, &INSET, kind_auto),
        "margin-block" => pair_shorthand(i, &MARGIN_BLOCK, kind_auto),
        "margin-inline" => pair_shorthand(i, &MARGIN_INLINE, kind_auto),
        "padding-block" => pair_shorthand(i, &PADDING_BLOCK, kind_plain),
        "padding-inline" => pair_shorthand(i, &PADDING_INLINE, kind_plain),
        "inset-block" => pair_shorthand(i, &INSET_BLOCK, kind_auto),
        "inset-inline" => pair_shorthand(i, &INSET_INLINE, kind_auto),
        "border-width" => {
            let v = parse_up_to4(i, parse_border_width)?;
            Ok(zip(&BORDER_WIDTH, expand4(&v).into_iter().map(Value::Length).collect()))
        }
        "border-style" => {
            let v = parse_up_to4(i, parse_border_style)?;
            Ok(zip(&BORDER_STYLE, expand4(&v).into_iter().collect()))
        }
        "border-color" => {
            let v = parse_up_to4(i, color_value)?;
            Ok(zip(&BORDER_COLOR, expand4(&v).into_iter().collect()))
        }
        "border" => {
            let (w, s, c) = parse_border_parts(i)?;
            let mut out = Vec::new();
            for side in [&BORDER_TOP, &BORDER_RIGHT, &BORDER_BOTTOM, &BORDER_LEFT] {
                out.extend(border_side((w.clone(), s.clone(), c.clone()), &side[..]));
            }
            Ok(out)
        }
        "border-top" => Ok(border_side(parse_border_parts(i)?, &BORDER_TOP)),
        "border-right" => Ok(border_side(parse_border_parts(i)?, &BORDER_RIGHT)),
        "border-bottom" => Ok(border_side(parse_border_parts(i)?, &BORDER_BOTTOM)),
        "border-left" => Ok(border_side(parse_border_parts(i)?, &BORDER_LEFT)),
        "border-radius" => parse_radius(i),
        "background" => parse_background(i),
        "background-position" => {
            let list = i.comma(parse_position)?;
            let xs = list.iter().map(|p| p.0.clone()).collect();
            let ys = list.iter().map(|p| p.1.clone()).collect();
            Ok(zip(&BG_POSITION, vec![Value::Lengths(xs), Value::Lengths(ys)]))
        }
        "font" => parse_font_shorthand(i),
        "flex" => parse_flex(i),
        "flex-flow" => parse_flex_flow(i),
        "gap" => pair_shorthand(i, &GAP, parse_gap_value),
        "grid-template" | "grid" => parse_grid_template(i),
        "grid-area" => parse_grid_area(i),
        "grid-row" => parse_grid_span(i, &GRID_ROW),
        "grid-column" => parse_grid_span(i, &GRID_COLUMN),
        "place-items" => parse_place(i, &PLACE_ITEMS),
        "place-content" => parse_place(i, &PLACE_CONTENT),
        "place-self" => parse_place(i, &PLACE_SELF),
        "overflow" => parse_overflow(i),
        "text-decoration" => parse_text_decoration(i),
        "list-style" => parse_list_style(i),
        _ => fail(),
    }
}

// ---------------------------------------------------------------------------
// var() substitution
// ---------------------------------------------------------------------------

/// Replaces every `var(--name, fallback)` in `text` using `lookup`. `None`
/// when a variable has no value and no fallback (the declaration is invalid
/// at computed-value time).
pub fn substitute_vars(text: &str, lookup: &mut dyn FnMut(&str) -> Option<String>) -> Option<String> {
    substitute_depth(text, lookup, 0)
}

fn substitute_depth(text: &str, lookup: &mut dyn FnMut(&str) -> Option<String>, depth: u32) -> Option<String> {
    if depth > 16 {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            // skip strings
            let q = b;
            i += 1;
            while i < bytes.len() && bytes[i] != q {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if i + 4 <= bytes.len() && bytes[i..i + 4].eq_ignore_ascii_case(b"var(") {
            let open = i + 4;
            let mut level = 1;
            let mut j = open;
            let mut comma = None;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => level += 1,
                    b')' => {
                        level -= 1;
                        if level == 0 {
                            break;
                        }
                    }
                    b',' if level == 1 && comma.is_none() => comma = Some(j),
                    _ => {}
                }
                j += 1;
            }
            if level != 0 {
                return None;
            }
            let (name, fallback) = match comma {
                Some(c) => (text[open..c].trim(), Some(text[c + 1..j].trim())),
                None => (text[open..j].trim(), None),
            };
            out.push_str(&text[last..i]);
            let value = match lookup(name) {
                Some(v) => v,
                None => match fallback {
                    Some(f) => substitute_depth(f, lookup, depth + 1)?,
                    None => return None,
                },
            };
            out.push_str(&value);
            i = j + 1;
            last = i;
            continue;
        }
        i += 1;
    }
    out.push_str(&text[last.min(text.len())..]);
    Some(out)
}

/// Resolves the `var()`s of a `Raw` declaration against `vars` and parses it.
pub fn resolve_raw(name: &str, text: &str, vars: &BTreeMap<String, String>) -> Option<Vec<(Prop, Value)>> {
    let substituted = substitute_vars(text, &mut |n| vars.get(n).cloned())?;
    parse_property(name, &substituted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(name: &str, text: &str) -> Vec<(Prop, Value)> {
        parse_property(name, text).unwrap_or_else(|| panic!("{} : {} failed", name, text))
    }

    fn get(list: &[(Prop, Value)], p: Prop) -> Value {
        list.iter().find(|(q, _)| *q == p).map(|(_, v)| v.clone()).unwrap_or_else(|| panic!("{:?} missing", p))
    }

    fn px(v: f32) -> Length {
        Length::Dim(v, Unit::Px)
    }

    #[test]
    fn margin_shorthand_expands() {
        let l = props("margin", "1px 2px 3px 4px");
        assert_eq!(get(&l, Prop::MarginTop), Value::Length(px(1.0)));
        assert_eq!(get(&l, Prop::MarginRight), Value::Length(px(2.0)));
        assert_eq!(get(&l, Prop::MarginBottom), Value::Length(px(3.0)));
        assert_eq!(get(&l, Prop::MarginLeft), Value::Length(px(4.0)));
        let l = props("margin", "0 auto");
        assert_eq!(get(&l, Prop::MarginTop), Value::Length(Length::Px(0.0)));
        assert_eq!(get(&l, Prop::MarginLeft), Value::Length(Length::Auto));
        let l = props("margin", "1px 2px 3px");
        assert_eq!(get(&l, Prop::MarginLeft), Value::Length(px(2.0)));
        assert!(parse_property("margin", "1px 2px 3px 4px 5px").is_none());
    }

    #[test]
    fn padding_rejects_auto() {
        assert!(parse_property("padding", "auto").is_none());
        let l = props("padding", "5%");
        assert_eq!(get(&l, Prop::PaddingTop), Value::Length(Length::Dim(5.0, Unit::Percent)));
    }

    #[test]
    fn logical_properties_map_to_physical() {
        let l = props("margin-inline", "4px 8px");
        assert_eq!(get(&l, Prop::MarginLeft), Value::Length(px(4.0)));
        assert_eq!(get(&l, Prop::MarginRight), Value::Length(px(8.0)));
        let l = props("padding-block-start", "3px");
        assert_eq!(l[0].0, Prop::PaddingTop);
        let l = props("inset", "0");
        assert_eq!(l.len(), 4);
    }

    #[test]
    fn border_shorthand() {
        let l = props("border", "2px solid red");
        assert_eq!(l.len(), 12);
        assert_eq!(get(&l, Prop::BorderTopWidth), Value::Length(px(2.0)));
        assert_eq!(get(&l, Prop::BorderLeftStyle), Value::Ident("solid".into()));
        assert!(matches!(get(&l, Prop::BorderBottomColor), Value::Color(Color::Rgba(_))));
        let l = props("border", "solid");
        assert_eq!(get(&l, Prop::BorderTopWidth), Value::Initial);
        assert_eq!(get(&l, Prop::BorderTopColor), Value::Initial);
        let l = props("border-bottom", "1px dashed #fff");
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].0, Prop::BorderBottomWidth);
        let l = props("border", "thin dotted");
        assert_eq!(get(&l, Prop::BorderTopWidth), Value::Length(Length::Px(1.0)));
        assert!(parse_property("border", "2px 3px").is_none());
    }

    #[test]
    fn border_width_style_color() {
        let l = props("border-width", "1px 2px");
        assert_eq!(get(&l, Prop::BorderBottomWidth), Value::Length(px(1.0)));
        assert_eq!(get(&l, Prop::BorderLeftWidth), Value::Length(px(2.0)));
        let l = props("border-style", "solid none");
        assert_eq!(get(&l, Prop::BorderRightStyle), Value::Ident("none".into()));
        let l = props("border-color", "red blue");
        assert!(matches!(get(&l, Prop::BorderTopColor), Value::Color(_)));
    }

    #[test]
    fn border_radius_shorthand() {
        let l = props("border-radius", "4px");
        assert_eq!(get(&l, Prop::BorderTopLeftRadius), Value::Pair(px(4.0), px(4.0)));
        let l = props("border-radius", "10px 20px / 5px");
        assert_eq!(get(&l, Prop::BorderTopLeftRadius), Value::Pair(px(10.0), px(5.0)));
        assert_eq!(get(&l, Prop::BorderTopRightRadius), Value::Pair(px(20.0), px(5.0)));
        let l = props("border-radius", "50%");
        assert_eq!(
            get(&l, Prop::BorderBottomRightRadius),
            Value::Pair(Length::Dim(50.0, Unit::Percent), Length::Dim(50.0, Unit::Percent))
        );
    }

    #[test]
    fn background_shorthand() {
        let l = props("background", "url(a.png) no-repeat center / cover #fff");
        assert_eq!(get(&l, Prop::BackgroundImage), Value::Images(vec![BgImage::Url("a.png".into())]));
        assert_eq!(
            get(&l, Prop::BackgroundRepeat),
            Value::Repeats(vec![(BgRepeatKind::NoRepeat, BgRepeatKind::NoRepeat)])
        );
        assert_eq!(get(&l, Prop::BackgroundSize), Value::Sizes(vec![BgSize::Cover]));
        assert_eq!(
            get(&l, Prop::BackgroundColor),
            Value::Color(Color::Rgba(Rgba::from_u8(255, 255, 255, 1.0)))
        );
        let l = props("background", "red");
        assert_eq!(get(&l, Prop::BackgroundImage), Value::Images(vec![BgImage::None]));
        let l = props("background", "linear-gradient(red, blue), blue");
        match get(&l, Prop::BackgroundImage) {
            Value::Images(v) => assert_eq!(v.len(), 2),
            _ => panic!(),
        }
        assert!(parse_property("background", "bogus").is_none());
    }

    #[test]
    fn background_position_shorthand() {
        let l = props("background-position", "right 10px");
        assert_eq!(get(&l, Prop::BackgroundPositionX), Value::Lengths(vec![Length::Dim(100.0, Unit::Percent)]));
        assert_eq!(get(&l, Prop::BackgroundPositionY), Value::Lengths(vec![px(10.0)]));
    }

    #[test]
    fn font_shorthand() {
        let l = props("font", "italic bold 16px/1.5 \"Helvetica Neue\", Arial, sans-serif");
        assert_eq!(get(&l, Prop::FontStyle), Value::Ident("italic".into()));
        assert_eq!(get(&l, Prop::FontWeight), Value::Ident("bold".into()));
        assert_eq!(get(&l, Prop::FontSize), Value::Length(px(16.0)));
        assert_eq!(get(&l, Prop::LineHeight), Value::Number(1.5));
        match get(&l, Prop::FontFamily) {
            Value::Families(f) => assert_eq!(f, vec!["Helvetica Neue", "Arial", "sans-serif"]),
            _ => panic!(),
        }
        let l = props("font", "12px serif");
        assert_eq!(get(&l, Prop::FontWeight), Value::Number(400.0));
        assert_eq!(get(&l, Prop::LineHeight), Value::Ident("normal".into()));
        let l = props("font", "600 1.2em sans-serif");
        assert_eq!(get(&l, Prop::FontWeight), Value::Number(600.0));
        assert!(parse_property("font", "bold").is_none());
    }

    #[test]
    fn flex_shorthand() {
        let l = props("flex", "1");
        assert_eq!(get(&l, Prop::FlexGrow), Value::Number(1.0));
        assert_eq!(get(&l, Prop::FlexShrink), Value::Number(1.0));
        assert_eq!(get(&l, Prop::FlexBasis), Value::Length(Length::Px(0.0)));
        let l = props("flex", "0 0 100px");
        assert_eq!(get(&l, Prop::FlexGrow), Value::Number(0.0));
        assert_eq!(get(&l, Prop::FlexBasis), Value::Length(px(100.0)));
        let l = props("flex", "none");
        assert_eq!(get(&l, Prop::FlexShrink), Value::Number(0.0));
        assert_eq!(get(&l, Prop::FlexBasis), Value::Length(Length::Auto));
        let l = props("flex", "auto");
        assert_eq!(get(&l, Prop::FlexGrow), Value::Number(1.0));
        let l = props("flex", "2 30%");
        assert_eq!(get(&l, Prop::FlexGrow), Value::Number(2.0));
        assert_eq!(get(&l, Prop::FlexShrink), Value::Number(1.0));
        assert_eq!(get(&l, Prop::FlexBasis), Value::Length(Length::Dim(30.0, Unit::Percent)));
        let l = props("flex", "1 1 0%");
        assert_eq!(get(&l, Prop::FlexBasis), Value::Length(Length::Dim(0.0, Unit::Percent)));
    }

    #[test]
    fn flex_flow_and_gap_and_place() {
        let l = props("flex-flow", "column wrap");
        assert_eq!(get(&l, Prop::FlexDirection), Value::Ident("column".into()));
        assert_eq!(get(&l, Prop::FlexWrap), Value::Ident("wrap".into()));
        let l = props("gap", "10px 20px");
        assert_eq!(get(&l, Prop::RowGap), Value::Length(px(10.0)));
        assert_eq!(get(&l, Prop::ColumnGap), Value::Length(px(20.0)));
        let l = props("gap", "8px");
        assert_eq!(get(&l, Prop::ColumnGap), Value::Length(px(8.0)));
        let l = props("grid-gap", "4px");
        assert_eq!(l.len(), 2);
        let l = props("place-items", "center");
        assert_eq!(get(&l, Prop::AlignItems), Value::Ident("center".into()));
        assert_eq!(get(&l, Prop::JustifyItems), Value::Ident("center".into()));
    }

    #[test]
    fn grid_shorthands() {
        let l = props("grid-template", "100px 1fr / repeat(2, 1fr)");
        match get(&l, Prop::GridTemplateRows) {
            Value::Tracks(t) => assert_eq!(t.len(), 2),
            _ => panic!(),
        }
        match get(&l, Prop::GridTemplateColumns) {
            Value::Tracks(t) => assert_eq!(t.len(), 1),
            _ => panic!(),
        }
        let l = props("grid-template", "\"a a\" 50px \"b c\" auto / 1fr 2fr");
        assert_eq!(
            get(&l, Prop::GridTemplateAreas),
            Value::Areas(vec![vec!["a".to_string(), "a".to_string()], vec!["b".to_string(), "c".to_string()]])
        );
        match get(&l, Prop::GridTemplateRows) {
            Value::Tracks(t) => assert_eq!(t.len(), 2),
            _ => panic!(),
        }
        let l = props("grid-area", "1 / 2 / 3 / 4");
        assert_eq!(get(&l, Prop::GridRowStart), Value::Line(GridLine::Line(1)));
        assert_eq!(get(&l, Prop::GridColumnStart), Value::Line(GridLine::Line(2)));
        assert_eq!(get(&l, Prop::GridRowEnd), Value::Line(GridLine::Line(3)));
        assert_eq!(get(&l, Prop::GridColumnEnd), Value::Line(GridLine::Line(4)));
        let l = props("grid-column", "1 / span 2");
        assert_eq!(get(&l, Prop::GridColumnEnd), Value::Line(GridLine::Span(2)));
        let l = props("grid-area", "main");
        assert_eq!(get(&l, Prop::GridColumnEnd), Value::Line(GridLine::Named("main".into())));
        let l = props("grid-auto-flow", "column dense");
        assert_eq!(get(&l, Prop::GridAutoFlow), Value::Ident("column dense".into()));
    }

    #[test]
    fn overflow_and_text_decoration_and_list_style() {
        let l = props("overflow", "hidden auto");
        assert_eq!(get(&l, Prop::OverflowX), Value::Ident("hidden".into()));
        assert_eq!(get(&l, Prop::OverflowY), Value::Ident("auto".into()));
        let l = props("overflow", "scroll");
        assert_eq!(get(&l, Prop::OverflowY), Value::Ident("scroll".into()));
        let l = props("text-decoration", "underline dotted red");
        assert_eq!(
            get(&l, Prop::TextDecorationLine),
            Value::DecoLine(DecoLine { underline: true, overline: false, line_through: false })
        );
        assert!(matches!(get(&l, Prop::TextDecorationColor), Value::Color(_)));
        let l = props("text-decoration", "none");
        assert_eq!(get(&l, Prop::TextDecorationLine), Value::DecoLine(DecoLine::default()));
        let l = props("list-style", "none");
        assert_eq!(get(&l, Prop::ListStyleType), Value::Ident("none".into()));
        let l = props("list-style", "square inside");
        assert_eq!(get(&l, Prop::ListStyleType), Value::Ident("square".into()));
    }

    #[test]
    fn transition_and_unknown_are_handled() {
        assert_eq!(parse_property("transition", "all 0.3s ease"), Some(Vec::new()));
        assert_eq!(parse_property("animation-name", "spin"), Some(Vec::new()));
        assert_eq!(parse_property("cursor", "pointer"), Some(Vec::new()));
        assert_eq!(parse_property("-webkit-tap-highlight-color", "red"), Some(Vec::new()));
        assert!(parse_property("nonsense", "1").is_none());
        assert!(parse_property("display", "bogus").is_none());
        assert!(parse_property("width", "").is_none());
    }

    #[test]
    fn css_wide_keywords_expand_to_longhands() {
        let l = props("margin", "inherit");
        assert_eq!(l.len(), 4);
        assert!(l.iter().all(|(_, v)| *v == Value::Inherit));
        let l = props("color", "INITIAL");
        assert_eq!(l, vec![(Prop::Color, Value::Initial)]);
        let l = props("display", "unset");
        assert_eq!(l, vec![(Prop::Display, Value::Unset)]);
    }

    #[test]
    fn longhand_values() {
        assert_eq!(props("display", "flex"), vec![(Prop::Display, Value::Ident("flex".into()))]);
        assert_eq!(props("z-index", "-2"), vec![(Prop::ZIndex, Value::Int(-2))]);
        assert_eq!(props("z-index", "auto"), vec![(Prop::ZIndex, Value::Ident("auto".into()))]);
        assert_eq!(props("opacity", "0.5"), vec![(Prop::Opacity, Value::Number(0.5))]);
        assert_eq!(props("opacity", "50%"), vec![(Prop::Opacity, Value::Number(0.5))]);
        assert_eq!(props("width", "calc(100% - 10px)").len(), 1);
        assert_eq!(props("max-width", "none"), vec![(Prop::MaxWidth, Value::Length(Length::None))]);
        assert_eq!(props("aspect-ratio", "16 / 9"), vec![(Prop::AspectRatio, Value::Aspect(Some(16.0 / 9.0)))]);
        assert_eq!(props("letter-spacing", "normal"), vec![(Prop::LetterSpacing, Value::Ident("normal".into()))]);
        assert_eq!(props("white-space", "pre-wrap"), vec![(Prop::WhiteSpace, Value::Ident("pre-wrap".into()))]);
        assert_eq!(props("text-align", "center"), vec![(Prop::TextAlign, Value::Ident("center".into()))]);
        assert_eq!(props("order", "-1"), vec![(Prop::Order, Value::Int(-1))]);
        assert_eq!(props("word-wrap", "break-word"), vec![(Prop::OverflowWrap, Value::Ident("break-word".into()))]);
        assert!(parse_property("opacity", "bogus").is_none());
        assert!(parse_property("flex-grow", "-1").is_none());
    }

    #[test]
    fn decl_block_splits_importance_and_vars() {
        let b = parse_declaration_list("color: red; margin: 0 !important; width: var(--w, 10px); --x: 4px; bogus: 1; ;");
        assert_eq!(b.important.len(), 4);
        assert!(b.normal.iter().any(|d| matches!(d, Declaration::Longhand(Prop::Color, _))));
        assert!(b.normal.iter().any(|d| matches!(d, Declaration::Raw(n, _) if n == "width")));
        assert!(b.normal.iter().any(|d| matches!(d, Declaration::Custom(n, v) if n == "--x" && v == "4px")));
        assert_eq!(b.normal.len(), 3);
    }

    #[test]
    fn important_detection() {
        assert_eq!(split_important("red !important"), ("red", true));
        assert_eq!(split_important("red ! IMPORTANT  "), ("red", true));
        assert_eq!(split_important("red"), ("red", false));
        assert_eq!(split_important("é"), ("é", false));
    }

    #[test]
    fn var_substitution() {
        let mut vars = BTreeMap::new();
        vars.insert("--a".to_string(), "10px".to_string());
        vars.insert("--b".to_string(), "red".to_string());
        let sub = |t: &str| substitute_vars(t, &mut |n| vars.get(n).cloned());
        assert_eq!(sub("var(--a)"), Some("10px".to_string()));
        assert_eq!(sub("calc(var(--a) * 2)"), Some("calc(10px * 2)".to_string()));
        assert_eq!(sub("var(--missing, 5px)"), Some("5px".to_string()));
        assert_eq!(sub("var(--missing, var(--a))"), Some("10px".to_string()));
        assert_eq!(sub("var(--missing)"), None);
        assert_eq!(sub("1px solid var(--b)"), Some("1px solid red".to_string()));
        assert_eq!(sub("var(--missing, 1px, 2px)"), Some("1px, 2px".to_string()));
        assert_eq!(sub("\"var(--a)\""), Some("\"var(--a)\"".to_string()));
        assert_eq!(sub("none"), Some("none".to_string()));
    }

    #[test]
    fn resolve_raw_parses_after_substitution() {
        let mut vars = BTreeMap::new();
        vars.insert("--gap".to_string(), "4px 8px".to_string());
        let l = resolve_raw("gap", "var(--gap)", &vars).unwrap();
        assert_eq!(get(&l, Prop::ColumnGap), Value::Length(px(8.0)));
        assert!(resolve_raw("gap", "var(--nope)", &vars).is_none());
    }

    fn env() -> MediaEnv {
        MediaEnv { width: 800.0, height: 1280.0, dark: false }
    }

    fn media(q: &str) -> bool {
        let mut p = Parser::new(q);
        parse_media_query_list(&mut p).matches(&env())
    }

    #[test]
    fn media_width_queries() {
        assert!(media("(min-width: 600px)"));
        assert!(!media("(min-width: 900px)"));
        assert!(media("(max-width: 800px)"));
        assert!(!media("(max-width: 799px)"));
        assert!(media("(width: 800px)"));
        assert!(media("(min-height: 1000px)"));
        assert!(media("(min-width: 40em)"));
        assert!(media("(width >= 600px)"));
        assert!(media("(600px <= width <= 900px)"));
        assert!(!media("(900px <= width)"));
        assert!(media("(width > 100px) and (width < 1000px)"));
    }

    #[test]
    fn media_logic_and_types() {
        assert!(media("screen"));
        assert!(media("all"));
        assert!(!media("print"));
        assert!(media("screen and (min-width: 600px)"));
        assert!(!media("print and (min-width: 600px)"));
        assert!(media("not print"));
        assert!(!media("not screen"));
        assert!(media("print, (min-width: 600px)"));
        assert!(media("(min-width: 900px) or (max-width: 900px)"));
        assert!(!media("(min-width: 900px) and (max-width: 900px)"));
        assert!(media("not (min-width: 900px)"));
        assert!(media("only screen and (max-width: 1000px)"));
        assert!(!media("(unknown-feature: 3)"));
        assert!(media(""));
    }

    #[test]
    fn media_scheme_orientation_and_misc() {
        assert!(!media("(prefers-color-scheme: dark)"));
        assert!(media("(prefers-color-scheme: light)"));
        let mut p = Parser::new("(prefers-color-scheme: dark)");
        assert!(parse_media_query_list(&mut p).matches(&MediaEnv { dark: true, ..env() }));
        assert!(media("(orientation: portrait)"));
        assert!(!media("(orientation: landscape)"));
        assert!(media("(prefers-reduced-motion: no-preference)"));
        assert!(media("(hover: none)"));
        assert!(media("(pointer: coarse)"));
        assert!(!media("(hover: hover)"));
        assert!(media("(min-resolution: 1dppx)"));
        assert!(!media("(min-resolution: 2dppx)"));
    }

    fn supports(q: &str) -> bool {
        let mut p = Parser::new(q);
        parse_supports(&mut p).unwrap_or(false)
    }

    #[test]
    fn supports_checks_known_properties() {
        assert!(supports("(display: grid)"));
        assert!(supports("(margin: 0)"));
        assert!(supports("(--custom: 1)"));
        assert!(!supports("(bogus-prop: 1)"));
        assert!(supports("not (bogus-prop: 1)"));
        assert!(supports("(display: grid) and (display: flex)"));
        assert!(!supports("(display: grid) and (bogus: 1)"));
        assert!(supports("(bogus: 1) or (display: flex)"));
        assert!(supports("selector(a > b)"));
        assert!(!supports("selector(a >>> )"));
    }

    #[test]
    fn stylesheet_rules_and_specificity() {
        let sheet = parse_stylesheet(
            "a, b.c { color: red } #x { margin: 0 } @import url(x.css); p {} bad{{ ",
            Origin::Author,
        );
        assert_eq!(sheet.rules().len(), 3);
        assert!(sheet.rules()[1].specificity > sheet.rules()[0].specificity);
        assert_eq!(sheet.rules()[0].block.normal.len(), 1);
    }

    #[test]
    fn stylesheet_media_nesting_and_supports() {
        let sheet = parse_stylesheet(
            "@media (min-width: 100px) { a { color: red } @media (max-width: 900px) { b { color: blue } } }
             @supports (display: grid) { c { display: grid } }
             @supports (bogus: 1) { d { color: red } }
             e { color: green }",
            Origin::Author,
        );
        let r = sheet.rules();
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].media.len(), 1);
        assert_eq!(r[1].media.len(), 2);
        assert_eq!(r[2].media.len(), 0);
        assert_eq!(r[3].media.len(), 0);
    }

    #[test]
    fn stylesheet_layer_and_unknown_at_rules() {
        let sheet = parse_stylesheet(
            "@layer a, b; @layer base { p { color: red } } @keyframes x { from { opacity: 0 } to { opacity: 1 } } @page { margin: 0 } q { color: blue }",
            Origin::Author,
        );
        assert_eq!(sheet.rules().len(), 2);
    }

    #[test]
    fn font_face_collected() {
        let sheet = parse_stylesheet(
            "@font-face { font-family: \"My Font\"; src: url(a.woff2) format(\"woff2\"), url(a.ttf), local(Arial); font-weight: 700; font-style: italic; unicode-range: U+0000-00FF }
             @font-face { font-family: Broken }",
            Origin::Author,
        );
        assert_eq!(sheet.font_faces().len(), 1);
        let ff = &sheet.font_faces()[0];
        assert_eq!(ff.family, "My Font");
        assert_eq!(ff.weight, 700.0);
        assert_eq!(ff.style, FontStyle::Italic);
        assert_eq!(ff.sources.len(), 3);
        assert_eq!(ff.sources[0], FontSrc::Url { url: "a.woff2".into(), format: Some("woff2".into()) });
        assert_eq!(ff.sources[2], FontSrc::Local("Arial".into()));
        assert!(ff.unicode_range.is_some());
    }

    #[test]
    fn pseudo_element_rules_parse() {
        let sheet = parse_stylesheet("a::before { content: \"x\" } p:after { content: \"y\" } a:hover { color: red }", Origin::Author);
        assert_eq!(sheet.rules().len(), 3);
        assert!(sheet.rules()[0].selector.has_pseudo_element());
        assert!(sheet.rules()[1].selector.has_pseudo_element());
        assert!(!sheet.rules()[2].selector.has_pseudo_element());
    }
}
