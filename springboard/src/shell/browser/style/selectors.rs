//! Selector matching on the `selectors` crate: the `SelectorImpl`, the
//! parser for style sheet preludes and `selectors::Element` for a
//! `(&Dom, NodeId)` pair, plus the rule-index helpers (bucket key and
//! ancestor hashes) the cascade uses.
//!
//! `selectors` 0.38 is built on cssparser 0.37 while the rest of the engine
//! uses cssparser 0.38, so selector text is cut out of the 0.38 token stream
//! as a string and re-parsed here with the 0.37 crate (`cssparser37`).

use super::super::dom::{Dom, NodeId, NodeKind};
use cssparser37 as cp;
use precomputed_hash::PrecomputedHash;
use selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use selectors::bloom::BloomFilter;
use selectors::context::SelectorCaches;
use selectors::matching::{
    matches_selector, ElementSelectorFlags, MatchingContext, MatchingForInvalidation, MatchingMode,
    NeedsSelectorFlags, QuirksMode,
};
use selectors::parser::{
    Combinator, Component, ParseRelative, Selector, SelectorImpl, SelectorList, SelectorParseErrorKind,
};
use selectors::{Element, OpaqueElement};
use std::fmt;
use std::ptr::NonNull;

/// A string atom used for every selector name type.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Atom(pub String);

impl<'a> From<&'a str> for Atom {
    fn from(s: &'a str) -> Atom {
        Atom(s.to_string())
    }
}

impl AsRef<str> for Atom {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl cp::ToCss for Atom {
    fn to_css<W>(&self, dest: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        cp::serialize_identifier(&self.0, dest)
    }
}

/// FNV-1a over `kind` and the bytes of `s`.
pub fn hash_str(kind: u8, s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5 ^ kind as u32;
    h = h.wrapping_mul(0x0100_0193);
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Hash kind of a tag name (lowercase).
pub const HASH_TAG: u8 = 1;
/// Hash kind of an id.
pub const HASH_ID: u8 = 2;
/// Hash kind of a class.
pub const HASH_CLASS: u8 = 3;

impl PrecomputedHash for Atom {
    fn precomputed_hash(&self) -> u32 {
        hash_str(0, &self.0)
    }
}

/// The `selectors` implementation tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelImpl;

impl SelectorImpl for SelImpl {
    type ExtraMatchingData<'a> = ();
    type AttrValue = Atom;
    type Identifier = Atom;
    type LocalName = Atom;
    type NamespaceUrl = Atom;
    type NamespacePrefix = Atom;
    type BorrowedNamespaceUrl = Atom;
    type BorrowedLocalName = Atom;
    type NonTSPseudoClass = PseudoClass;
    type PseudoElement = PseudoElementKind;
}

/// The supported non tree-structural pseudo-classes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PseudoClass {
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
    Link,
    AnyLink,
    Visited,
    Target,
    Checked,
    Disabled,
    Enabled,
    Required,
    Optional,
    ReadOnly,
    ReadWrite,
    PlaceholderShown,
}

impl PseudoClass {
    fn name(self) -> &'static str {
        match self {
            PseudoClass::Hover => "hover",
            PseudoClass::Active => "active",
            PseudoClass::Focus => "focus",
            PseudoClass::FocusVisible => "focus-visible",
            PseudoClass::FocusWithin => "focus-within",
            PseudoClass::Link => "link",
            PseudoClass::AnyLink => "any-link",
            PseudoClass::Visited => "visited",
            PseudoClass::Target => "target",
            PseudoClass::Checked => "checked",
            PseudoClass::Disabled => "disabled",
            PseudoClass::Enabled => "enabled",
            PseudoClass::Required => "required",
            PseudoClass::Optional => "optional",
            PseudoClass::ReadOnly => "read-only",
            PseudoClass::ReadWrite => "read-write",
            PseudoClass::PlaceholderShown => "placeholder-shown",
        }
    }

    fn from_name(n: &str) -> Option<PseudoClass> {
        Some(match n {
            "hover" => PseudoClass::Hover,
            "active" => PseudoClass::Active,
            "focus" => PseudoClass::Focus,
            "focus-visible" => PseudoClass::FocusVisible,
            "focus-within" => PseudoClass::FocusWithin,
            "link" => PseudoClass::Link,
            "any-link" => PseudoClass::AnyLink,
            "visited" => PseudoClass::Visited,
            "target" => PseudoClass::Target,
            "checked" => PseudoClass::Checked,
            "disabled" => PseudoClass::Disabled,
            "enabled" => PseudoClass::Enabled,
            "required" => PseudoClass::Required,
            "optional" => PseudoClass::Optional,
            "read-only" => PseudoClass::ReadOnly,
            "read-write" => PseudoClass::ReadWrite,
            "placeholder-shown" => PseudoClass::PlaceholderShown,
            _ => return None,
        })
    }
}

impl cp::ToCss for PseudoClass {
    fn to_css<W>(&self, dest: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        dest.write_str(":")?;
        dest.write_str(self.name())
    }
}

impl selectors::parser::NonTSPseudoClass for PseudoClass {
    type Impl = SelImpl;

    fn is_active_or_hover(&self) -> bool {
        matches!(self, PseudoClass::Hover | PseudoClass::Active)
    }

    fn is_user_action_state(&self) -> bool {
        matches!(
            self,
            PseudoClass::Hover | PseudoClass::Active | PseudoClass::Focus | PseudoClass::FocusVisible | PseudoClass::FocusWithin
        )
    }
}

/// The supported pseudo-elements.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PseudoElementKind {
    Before,
    After,
}

impl cp::ToCss for PseudoElementKind {
    fn to_css<W>(&self, dest: &mut W) -> fmt::Result
    where
        W: fmt::Write,
    {
        dest.write_str(match self {
            PseudoElementKind::Before => "::before",
            PseudoElementKind::After => "::after",
        })
    }
}

impl selectors::parser::PseudoElement for PseudoElementKind {
    type Impl = SelImpl;

    fn is_before_or_after(&self) -> bool {
        true
    }
}

struct SelParser;

impl<'i> selectors::parser::Parser<'i> for SelParser {
    type Impl = SelImpl;
    type Error = SelectorParseErrorKind<'i>;

    fn parse_is_and_where(&self) -> bool {
        true
    }

    fn parse_non_ts_pseudo_class(
        &self,
        location: cp::SourceLocation,
        name: cp::CowRcStr<'i>,
    ) -> Result<PseudoClass, cp::ParseError<'i, SelectorParseErrorKind<'i>>> {
        let lower = name.to_ascii_lowercase();
        match PseudoClass::from_name(&lower) {
            Some(pc) => Ok(pc),
            None => Err(location.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name))),
        }
    }

    fn parse_pseudo_element(
        &self,
        location: cp::SourceLocation,
        name: cp::CowRcStr<'i>,
    ) -> Result<PseudoElementKind, cp::ParseError<'i, SelectorParseErrorKind<'i>>> {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "before" => Ok(PseudoElementKind::Before),
            "after" => Ok(PseudoElementKind::After),
            _ => Err(location.new_custom_error(SelectorParseErrorKind::UnsupportedPseudoClassOrElement(name))),
        }
    }
}

/// Parses a selector list; `None` when any selector is invalid.
pub fn parse_selector_list(text: &str) -> Option<SelectorList<SelImpl>> {
    let mut input = cp::ParserInput::new(text);
    let mut parser = cp::Parser::new(&mut input);
    SelectorList::parse(&SelParser, &mut parser, ParseRelative::No).ok()
}

/// An element of a `Dom` for selector matching.
#[derive(Clone, Copy)]
pub struct El<'a> {
    pub dom: &'a Dom,
    pub id: NodeId,
}

impl<'a> fmt::Debug for El<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "El({}, {})", self.id.0, self.dom.tag(self.id).unwrap_or("?"))
    }
}

impl<'a> El<'a> {
    /// Wraps `id` when it is an element.
    pub fn new(dom: &'a Dom, id: NodeId) -> Option<El<'a>> {
        dom.tag(id).map(|_| El { dom, id })
    }

    fn wrap(&self, id: NodeId) -> Option<El<'a>> {
        El::new(self.dom, id)
    }

    fn tag(&self) -> &'a str {
        self.dom.tag(self.id).unwrap_or("")
    }

    fn has_attr(&self, name: &str) -> bool {
        self.dom.attrs(self.id).iter().any(|(k, _)| k.eq_ignore_ascii_case(name))
    }

    fn attr_value(&self, name: &str) -> Option<&'a str> {
        self.dom
            .attrs(self.id)
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn is_form_control(&self) -> bool {
        matches!(self.tag(), "input" | "button" | "select" | "textarea" | "optgroup" | "option" | "fieldset")
    }

    fn is_checkable_input(&self) -> bool {
        let ty = self.attr_value("type").unwrap_or("text");
        ty.eq_ignore_ascii_case("checkbox") || ty.eq_ignore_ascii_case("radio")
    }
}

impl<'a> Element for El<'a> {
    type Impl = SelImpl;

    fn opaque(&self) -> OpaqueElement {
        let ptr = NonNull::<()>::new((self.id.0 as usize + 1) as *mut ()).expect("non-zero");
        OpaqueElement::from_non_null_ptr(ptr)
    }

    fn parent_element(&self) -> Option<Self> {
        self.dom.parent(self.id).and_then(|p| self.wrap(p))
    }

    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }

    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }

    fn is_pseudo_element(&self) -> bool {
        false
    }

    fn prev_sibling_element(&self) -> Option<Self> {
        let mut cur = self.dom.prev_sibling(self.id);
        while let Some(n) = cur {
            if let Some(e) = self.wrap(n) {
                return Some(e);
            }
            cur = self.dom.prev_sibling(n);
        }
        None
    }

    fn next_sibling_element(&self) -> Option<Self> {
        let mut cur = self.dom.next_sibling(self.id);
        while let Some(n) = cur {
            if let Some(e) = self.wrap(n) {
                return Some(e);
            }
            cur = self.dom.next_sibling(n);
        }
        None
    }

    fn first_element_child(&self) -> Option<Self> {
        let mut cur = self.dom.first_child(self.id);
        while let Some(n) = cur {
            if let Some(e) = self.wrap(n) {
                return Some(e);
            }
            cur = self.dom.next_sibling(n);
        }
        None
    }

    fn is_html_element_in_html_document(&self) -> bool {
        true
    }

    fn has_local_name(&self, local_name: &Atom) -> bool {
        self.tag().eq_ignore_ascii_case(&local_name.0)
    }

    fn has_namespace(&self, _ns: &Atom) -> bool {
        true
    }

    fn is_same_type(&self, other: &Self) -> bool {
        self.tag() == other.tag()
    }

    fn attr_matches(
        &self,
        ns: &NamespaceConstraint<&Atom>,
        local_name: &Atom,
        operation: &AttrSelectorOperation<&Atom>,
    ) -> bool {
        if let NamespaceConstraint::Specific(url) = ns {
            if !url.0.is_empty() {
                return false;
            }
        }
        self.dom
            .attrs(self.id)
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case(&local_name.0) && operation.eval_str(v))
    }

    fn match_non_ts_pseudo_class(&self, pc: &PseudoClass, _context: &mut MatchingContext<SelImpl>) -> bool {
        match pc {
            PseudoClass::Hover
            | PseudoClass::Active
            | PseudoClass::Focus
            | PseudoClass::FocusVisible
            | PseudoClass::FocusWithin
            | PseudoClass::Visited
            | PseudoClass::Target => false,
            PseudoClass::Link | PseudoClass::AnyLink => self.is_link(),
            PseudoClass::Checked => match self.tag() {
                "input" => self.is_checkable_input() && self.has_attr("checked"),
                "option" => self.has_attr("selected"),
                _ => false,
            },
            PseudoClass::Disabled => self.is_form_control() && self.has_attr("disabled"),
            PseudoClass::Enabled => self.is_form_control() && !self.has_attr("disabled"),
            PseudoClass::Required => matches!(self.tag(), "input" | "select" | "textarea") && self.has_attr("required"),
            PseudoClass::Optional => matches!(self.tag(), "input" | "select" | "textarea") && !self.has_attr("required"),
            PseudoClass::ReadOnly => !(matches!(self.tag(), "input" | "textarea") && !self.has_attr("readonly") && !self.has_attr("disabled")),
            PseudoClass::ReadWrite => matches!(self.tag(), "input" | "textarea") && !self.has_attr("readonly") && !self.has_attr("disabled"),
            PseudoClass::PlaceholderShown => {
                matches!(self.tag(), "input" | "textarea")
                    && self.has_attr("placeholder")
                    && self.attr_value("value").map_or(true, |v| v.is_empty())
            }
        }
    }

    fn match_pseudo_element(&self, _pe: &PseudoElementKind, _context: &mut MatchingContext<SelImpl>) -> bool {
        true
    }

    fn apply_selector_flags(&self, _flags: ElementSelectorFlags) {}

    fn is_link(&self) -> bool {
        matches!(self.tag(), "a" | "area") && self.has_attr("href")
    }

    fn is_html_slot_element(&self) -> bool {
        false
    }

    fn has_id(&self, id: &Atom, case_sensitivity: CaseSensitivity) -> bool {
        match self.dom.id(self.id) {
            Some(v) => case_sensitivity.eq(id.0.as_bytes(), v.as_bytes()),
            None => false,
        }
    }

    fn has_class(&self, name: &Atom, case_sensitivity: CaseSensitivity) -> bool {
        match case_sensitivity {
            CaseSensitivity::CaseSensitive => self.dom.has_class(self.id, &name.0),
            CaseSensitivity::AsciiCaseInsensitive => self.dom.classes(self.id).any(|c| c.eq_ignore_ascii_case(&name.0)),
        }
    }

    fn has_custom_state(&self, _name: &Atom) -> bool {
        false
    }

    fn imported_part(&self, _name: &Atom) -> Option<Atom> {
        None
    }

    fn is_part(&self, _name: &Atom) -> bool {
        false
    }

    fn is_empty(&self) -> bool {
        self.dom.children(self.id).all(|c| match self.dom.kind(c) {
            NodeKind::Element { .. } => false,
            NodeKind::Text(t) => t.is_empty(),
            _ => true,
        })
    }

    fn is_root(&self) -> bool {
        self.dom.parent(self.id) == Some(self.dom.root())
    }

    fn add_element_unique_hashes(&self, _filter: &mut BloomFilter) -> bool {
        false
    }
}

/// Matches `selector` against `el`. With `pseudo` the selector must carry a
/// pseudo-element, which is then ignored (the originating element is
/// matched).
pub fn matches(selector: &Selector<SelImpl>, el: &El<'_>, caches: &mut SelectorCaches, pseudo: bool) -> bool {
    let mode = if pseudo { MatchingMode::ForStatelessPseudoElement } else { MatchingMode::Normal };
    let mut context = MatchingContext::new(
        mode,
        None,
        caches,
        QuirksMode::NoQuirks,
        NeedsSelectorFlags::No,
        MatchingForInvalidation::No,
    );
    matches_selector(selector, 0, None, el, &mut context)
}

/// The pseudo-element a selector targets, if any.
pub fn pseudo_of(selector: &Selector<SelImpl>) -> Option<PseudoElementKind> {
    selector.pseudo_element().copied()
}

/// How the rule index buckets a selector: by the most specific simple
/// selector of its subject compound.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum BucketKey {
    Id(String),
    Class(String),
    Tag(String),
    Universal,
}

/// Picks the bucket of a selector (id over class over tag).
pub fn bucket_key(selector: &Selector<SelImpl>) -> BucketKey {
    let mut best = BucketKey::Universal;
    for c in selector.iter_raw_match_order() {
        match c {
            Component::ID(id) => return BucketKey::Id(id.0.clone()),
            Component::Class(cl) => {
                if !matches!(best, BucketKey::Class(_)) {
                    best = BucketKey::Class(cl.0.clone());
                }
            }
            Component::LocalName(n) => {
                if matches!(best, BucketKey::Universal) {
                    best = BucketKey::Tag(n.lower_name.0.clone());
                }
            }
            Component::Combinator(Combinator::PseudoElement) => best = BucketKey::Universal,
            Component::Combinator(_) => break,
            _ => {}
        }
    }
    best
}

/// Hashes of the tag, id and class names a selector requires of its
/// ancestors (for the ancestor bloom filter pre-check).
pub fn ancestor_hashes(selector: &Selector<SelImpl>) -> Vec<u32> {
    let mut out = Vec::new();
    let mut ancestor = false;
    for c in selector.iter_raw_match_order() {
        match c {
            Component::Combinator(Combinator::Descendant) | Component::Combinator(Combinator::Child) => {
                ancestor = true
            }
            Component::Combinator(Combinator::NextSibling) | Component::Combinator(Combinator::LaterSibling) => {
                ancestor = false
            }
            Component::ID(id) if ancestor => out.push(hash_str(HASH_ID, &id.0)),
            Component::Class(cl) if ancestor => out.push(hash_str(HASH_CLASS, &cl.0)),
            Component::LocalName(n) if ancestor => out.push(hash_str(HASH_TAG, &n.lower_name.0)),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::super::dom::parse_html;
    use super::*;

    const PAGE: &str = "<html><body>
        <div id=wrap class='a b'>
          <p class=first>one</p>
          <p class='second hi' data-x='foo-bar' title='Hello World'>two</p>
          <span></span>
          <ul><li>1</li><li class=mid>2</li><li>3</li></ul>
          <input id=cb type=checkbox checked>
          <input id=off disabled>
          <a id=lnk href='/x'>link</a><a id=nolink>no</a>
        </div></body></html>";

    fn check(dom: &Dom, selector: &str, id: &str) -> bool {
        let list = parse_selector_list(selector).unwrap_or_else(|| panic!("bad selector {}", selector));
        let node = dom.get_element_by_id(id).unwrap_or_else(|| panic!("no #{}", id));
        let el = El::new(dom, node).unwrap();
        let mut caches = SelectorCaches::default();
        list.slice().iter().any(|s| matches(s, &el, &mut caches, false))
    }

    fn count(dom: &Dom, selector: &str) -> usize {
        let list = parse_selector_list(selector).unwrap();
        let mut caches = SelectorCaches::default();
        let mut n = 0;
        for id in dom.descendants(dom.root()) {
            if let Some(el) = El::new(dom, id) {
                if list.slice().iter().any(|s| matches(s, &el, &mut caches, false)) {
                    n += 1;
                }
            }
        }
        n
    }

    #[test]
    fn type_class_id() {
        let dom = parse_html(PAGE);
        assert!(check(&dom, "div", "wrap"));
        assert!(check(&dom, "#wrap", "wrap"));
        assert!(check(&dom, ".a", "wrap"));
        assert!(check(&dom, "div.a.b#wrap", "wrap"));
        assert!(!check(&dom, "span.a", "wrap"));
        assert_eq!(count(&dom, "p"), 2);
        assert_eq!(count(&dom, "*"), count(&dom, "html, body, div, p, span, ul, li, input, a, head"));
    }

    #[test]
    fn attribute_selectors() {
        let dom = parse_html(PAGE);
        assert!(check(&dom, "[data-x]", "lnk") == false);
        assert_eq!(count(&dom, "[data-x]"), 1);
        assert_eq!(count(&dom, "[data-x=foo-bar]"), 1);
        assert_eq!(count(&dom, "[data-x^=foo]"), 1);
        assert_eq!(count(&dom, "[data-x$=bar]"), 1);
        assert_eq!(count(&dom, "[data-x*=o-b]"), 1);
        assert_eq!(count(&dom, "[data-x|=foo]"), 1);
        assert_eq!(count(&dom, "[title~=World]"), 1);
        assert_eq!(count(&dom, "[title='hello world' i]"), 1);
        assert_eq!(count(&dom, "a[href]"), 1);
    }

    #[test]
    fn combinators() {
        let dom = parse_html(PAGE);
        assert_eq!(count(&dom, "div p"), 2);
        assert_eq!(count(&dom, "div > p"), 2);
        assert_eq!(count(&dom, "body > p"), 0);
        assert_eq!(count(&dom, "p + p"), 1);
        assert_eq!(count(&dom, "p + span"), 1);
        assert_eq!(count(&dom, "p ~ ul"), 1);
        assert_eq!(count(&dom, "ul > li + li"), 2);
        assert_eq!(count(&dom, "#wrap li"), 3);
        assert_eq!(count(&dom, "body div ul li.mid"), 1);
    }

    #[test]
    fn structural_pseudo_classes() {
        let dom = parse_html(PAGE);
        assert_eq!(count(&dom, "li:first-child"), 1);
        assert_eq!(count(&dom, "li:last-child"), 1);
        assert_eq!(count(&dom, "li:nth-child(2)"), 1);
        assert_eq!(count(&dom, "li:nth-child(odd)"), 2);
        assert_eq!(count(&dom, "li:nth-child(2n+1)"), 2);
        assert_eq!(count(&dom, "li:nth-last-child(1)"), 1);
        assert_eq!(count(&dom, "p:first-of-type"), 1);
        assert_eq!(count(&dom, "span:empty"), 1);
        assert_eq!(count(&dom, "ul:empty"), 0);
        assert_eq!(count(&dom, ":root"), 1);
        assert!(check(&dom, "li:only-child", "wrap") == false);
    }

    #[test]
    fn logical_pseudo_classes() {
        let dom = parse_html(PAGE);
        assert_eq!(count(&dom, "li:not(.mid)"), 2);
        assert_eq!(count(&dom, ":is(p, span)"), 3);
        assert_eq!(count(&dom, ":where(li, a)"), 5);
        assert_eq!(count(&dom, "p:not(.first):not(.zzz)"), 1);
        assert_eq!(count(&dom, "div :is(.first, .mid)"), 2);
    }

    #[test]
    fn form_and_link_pseudo_classes() {
        let dom = parse_html(PAGE);
        assert!(check(&dom, ":checked", "cb"));
        assert!(!check(&dom, ":checked", "off"));
        assert!(check(&dom, ":disabled", "off"));
        assert!(check(&dom, ":enabled", "cb"));
        assert!(check(&dom, ":link", "lnk"));
        assert!(!check(&dom, ":link", "nolink"));
        assert!(!check(&dom, ":hover", "lnk"));
        assert_eq!(count(&dom, "a:not(:link)"), 1);
    }

    #[test]
    fn pseudo_elements_parse_and_match_originating_element() {
        let list = parse_selector_list("p::before, p:after").unwrap();
        assert_eq!(list.slice().len(), 2);
        for s in list.slice() {
            assert!(s.has_pseudo_element());
        }
        assert_eq!(pseudo_of(&list.slice()[0]), Some(PseudoElementKind::Before));
        assert_eq!(pseudo_of(&list.slice()[1]), Some(PseudoElementKind::After));
        let dom = parse_html(PAGE);
        let first = dom.elements_by_tag("p")[0];
        let el = El::new(&dom, first).unwrap();
        let mut caches = SelectorCaches::default();
        assert!(matches(&list.slice()[0], &el, &mut caches, true));
        let span = dom.elements_by_tag("span")[0];
        let el = El::new(&dom, span).unwrap();
        assert!(!matches(&list.slice()[0], &el, &mut caches, true));
    }

    #[test]
    fn invalid_selectors_are_rejected() {
        assert!(parse_selector_list("p >").is_none());
        assert!(parse_selector_list("").is_none());
        assert!(parse_selector_list("p:bogus-class").is_none());
        assert!(parse_selector_list("p::bogus").is_none());
        assert!(parse_selector_list("a, ").is_none());
    }

    #[test]
    fn specificity_orders_as_css() {
        let s = |t: &str| parse_selector_list(t).unwrap().slice()[0].specificity();
        assert!(s("#a") > s(".a.b.c"));
        assert!(s(".a") > s("p"));
        assert!(s("p.a") > s(".a"));
        assert!(s("p::before") > s("p"));
        assert_eq!(s("*"), 0);
        assert_eq!(s(":where(.a)"), 0);
        assert_eq!(s(":is(.a, #b)"), s("#b"));
    }

    #[test]
    fn bucket_keys() {
        let k = |t: &str| bucket_key(&parse_selector_list(t).unwrap().slice()[0]);
        assert_eq!(k("div.a"), BucketKey::Class("a".into()));
        assert_eq!(k("div#x.a"), BucketKey::Id("x".into()));
        assert_eq!(k("DIV"), BucketKey::Tag("div".into()));
        assert_eq!(k("*"), BucketKey::Universal);
        assert_eq!(k(".a .b"), BucketKey::Class("b".into()));
        assert_eq!(k(".a > p"), BucketKey::Tag("p".into()));
        assert_eq!(k("[x]"), BucketKey::Universal);
        assert_eq!(k("a::before"), BucketKey::Tag("a".into()));
        assert_eq!(k(".x p::after"), BucketKey::Tag("p".into()));
    }

    #[test]
    fn ancestor_hash_collection() {
        let h = |t: &str| ancestor_hashes(&parse_selector_list(t).unwrap().slice()[0]);
        assert!(h("p").is_empty());
        assert_eq!(h(".a .b"), vec![hash_str(HASH_CLASS, "a")]);
        assert_eq!(h("#x > div p"), vec![hash_str(HASH_TAG, "div"), hash_str(HASH_ID, "x")]);
        assert!(h(".a + .b").is_empty());
        assert_eq!(h(".a > .b + .c"), vec![hash_str(HASH_CLASS, "a")]);
        assert_eq!(h(".a .b + .c"), vec![hash_str(HASH_CLASS, "a")]);
    }
}
