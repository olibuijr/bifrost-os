//! The style engine: an indexed rule set, selector matching with an ancestor
//! bloom filter, the cascade (origin, importance, specificity, order),
//! custom properties with `var()`, inheritance and `::before`/`::after`.

use super::super::dom::{Dom, NodeId};
use super::computed::{ApplyCtx, Computed, CustomProps, Explicit, Prop};
use super::parse::{
    longhands_of, parse_declaration_list, resolve_raw, substitute_vars, DeclBlock, Declaration, FontFace,
    MediaEnv, Origin, Rule, Stylesheet,
};
use super::selectors::{
    ancestor_hashes, bucket_key, hash_str, matches, pseudo_of, BucketKey, El, PseudoElementKind, HASH_CLASS, HASH_ID,
    HASH_TAG,
};
use super::values::{Content, Display, Value};
use ::selectors::context::SelectorCaches;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

/// Counting bloom filter over the tag, id and class names of the ancestors
/// of the element being matched.
pub struct AncestorBloom {
    counts: [u16; 256],
}

fn slots(h: u32) -> (usize, usize) {
    ((h & 255) as usize, ((h >> 8) & 255) as usize)
}

impl AncestorBloom {
    /// An empty filter.
    pub fn new() -> AncestorBloom {
        AncestorBloom { counts: [0; 256] }
    }

    /// The hashes `node` contributes.
    pub fn hashes_for(dom: &Dom, node: NodeId) -> Vec<u32> {
        let mut v = Vec::new();
        if let Some(tag) = dom.tag(node) {
            v.push(hash_str(HASH_TAG, tag));
        }
        if let Some(id) = dom.id(node) {
            v.push(hash_str(HASH_ID, id));
        }
        for c in dom.classes(node) {
            v.push(hash_str(HASH_CLASS, c));
        }
        v
    }

    /// Adds `node`; returns what to pass to `pop` later.
    pub fn push(&mut self, dom: &Dom, node: NodeId) -> Vec<u32> {
        let hs = AncestorBloom::hashes_for(dom, node);
        for h in &hs {
            let (a, b) = slots(*h);
            self.counts[a] += 1;
            self.counts[b] += 1;
        }
        hs
    }

    /// Removes hashes added by `push`.
    pub fn pop(&mut self, hs: &[u32]) {
        for h in hs {
            let (a, b) = slots(*h);
            self.counts[a] = self.counts[a].saturating_sub(1);
            self.counts[b] = self.counts[b].saturating_sub(1);
        }
    }

    /// `false` means the hash is definitely absent.
    pub fn might_contain(&self, h: u32) -> bool {
        let (a, b) = slots(h);
        self.counts[a] > 0 && self.counts[b] > 0
    }
}

impl Default for AncestorBloom {
    fn default() -> Self {
        AncestorBloom::new()
    }
}

/// The computed style of one element and its generated boxes.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeStyle {
    pub computed: Computed,
    /// `::before` box, present when its `content` generates one.
    pub before: Option<Box<Computed>>,
    /// `::after` box.
    pub after: Option<Box<Computed>>,
}

/// Computed styles for a whole document, indexed by node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleTree {
    nodes: Vec<Option<NodeStyle>>,
}

impl StyleTree {
    fn new() -> StyleTree {
        StyleTree { nodes: Vec::new() }
    }

    fn set(&mut self, n: NodeId, style: NodeStyle) {
        let i = n.0 as usize;
        if self.nodes.len() <= i {
            self.nodes.resize(i + 1, None);
        }
        self.nodes[i] = Some(style);
    }

    fn clear(&mut self, n: NodeId) {
        if let Some(slot) = self.nodes.get_mut(n.0 as usize) {
            *slot = None;
        }
    }

    /// The computed style of an element.
    pub fn get(&self, n: NodeId) -> Option<&Computed> {
        self.node(n).map(|s| &s.computed)
    }

    /// The element style with its generated boxes.
    pub fn node(&self, n: NodeId) -> Option<&NodeStyle> {
        self.nodes.get(n.0 as usize).and_then(|s| s.as_ref())
    }

    /// The `::before` style of an element, if it generates one.
    pub fn before(&self, n: NodeId) -> Option<&Computed> {
        self.node(n).and_then(|s| s.before.as_deref())
    }

    /// The `::after` style of an element, if it generates one.
    pub fn after(&self, n: NodeId) -> Option<&Computed> {
        self.node(n).and_then(|s| s.after.as_deref())
    }

    /// Number of styled elements.
    pub fn len(&self) -> usize {
        self.nodes.iter().filter(|n| n.is_some()).count()
    }

    /// Whether no element is styled.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

struct IndexedRule {
    rule: Rule,
    origin: Origin,
    order: u32,
    ancestors: Vec<u32>,
    pseudo: Option<PseudoElementKind>,
}

struct Entry<'a> {
    rank: u8,
    spec: u32,
    order: u32,
    decls: &'a [Declaration],
}

fn push_block<'a>(out: &mut Vec<Entry<'a>>, block: &'a DeclBlock, origin: Origin, spec: u32, order: u32) {
    let (normal, important) = match origin {
        Origin::UserAgent => (0, 3),
        Origin::Author => (1, 2),
    };
    if !block.normal.is_empty() {
        out.push(Entry { rank: normal, spec, order, decls: &block.normal });
    }
    if !block.important.is_empty() {
        out.push(Entry { rank: important, spec, order, decls: &block.important });
    }
}

/// The style engine: rules from all sheets that match the media environment,
/// bucketed by the id, class or tag of their subject.
pub struct StyleEngine {
    env: MediaEnv,
    rules: Vec<IndexedRule>,
    by_id: HashMap<String, Vec<u32>>,
    by_class: HashMap<String, Vec<u32>>,
    by_tag: HashMap<String, Vec<u32>>,
    universal: Vec<u32>,
    initial: Computed,
    font_faces: Vec<FontFace>,
}

impl StyleEngine {
    /// Builds the engine. Sheets are in cascade order of their origin
    /// (later sheets win ties); `@media` rules that do not match `env` are
    /// dropped.
    pub fn new(sheets: Vec<Stylesheet>, env: MediaEnv) -> StyleEngine {
        let mut engine = StyleEngine {
            env,
            rules: Vec::new(),
            by_id: HashMap::new(),
            by_class: HashMap::new(),
            by_tag: HashMap::new(),
            universal: Vec::new(),
            initial: Computed::default(),
            font_faces: Vec::new(),
        };
        let mut order = 0u32;
        for sheet in &sheets {
            for rule in &sheet.rules {
                if !rule.media.iter().all(|m| m.matches(&env)) {
                    continue;
                }
                let idx = engine.rules.len() as u32;
                match bucket_key(&rule.selector) {
                    BucketKey::Id(k) => engine.by_id.entry(k).or_default().push(idx),
                    BucketKey::Class(k) => engine.by_class.entry(k).or_default().push(idx),
                    BucketKey::Tag(k) => engine.by_tag.entry(k).or_default().push(idx),
                    BucketKey::Universal => engine.universal.push(idx),
                }
                engine.rules.push(IndexedRule {
                    rule: rule.clone(),
                    origin: sheet.origin,
                    order,
                    ancestors: ancestor_hashes(&rule.selector),
                    pseudo: pseudo_of(&rule.selector),
                });
                order += 1;
            }
            engine.font_faces.extend(sheet.font_faces.iter().cloned());
        }
        engine
    }

    /// The `@font-face` rules of all sheets.
    pub fn font_faces(&self) -> &[FontFace] {
        &self.font_faces
    }

    /// The media environment the engine was built for.
    pub fn env(&self) -> MediaEnv {
        self.env
    }

    /// Number of active style rules.
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Computes the style of every element of `dom`.
    pub fn compute(&self, dom: &Dom) -> StyleTree {
        let mut tree = StyleTree::new();
        let mut bloom = AncestorBloom::new();
        let mut caches = SelectorCaches::default();
        self.compute_subtree(dom, &mut tree, dom.root(), &mut bloom, &mut caches);
        tree
    }

    /// Re-styles after DOM changes. Every dirty node's parent subtree is
    /// recomputed (sibling selectors and inheritance both flow from there);
    /// detached nodes lose their style.
    pub fn recompute(&self, dom: &Dom, tree: &mut StyleTree, dirty: &[NodeId]) {
        enum Target {
            /// Restyle the children (and below) of this element.
            Children(NodeId),
            /// Restyle this root element and everything below.
            Subtree(NodeId),
        }
        impl Target {
            fn node(&self) -> NodeId {
                match self {
                    Target::Children(n) | Target::Subtree(n) => *n,
                }
            }
        }
        if dirty.contains(&dom.root()) {
            *tree = self.compute(dom);
            return;
        }
        let mut targets: Vec<Target> = Vec::new();
        let mut seen: HashSet<(bool, NodeId)> = HashSet::new();
        for &d in dirty {
            if dom.parent(d).is_none() {
                tree.clear(d);
                for x in dom.descendants(d) {
                    tree.clear(x);
                }
                continue;
            }
            let target = match dom.parent(d) {
                Some(p) if dom.tag(p).is_some() => Target::Children(p),
                Some(_) if dom.tag(d).is_some() => Target::Subtree(d),
                _ => continue,
            };
            let key = (matches!(target, Target::Subtree(_)), target.node());
            if seen.insert(key) {
                targets.push(target);
            }
        }
        let nodes: HashSet<NodeId> = targets.iter().map(|t| t.node()).collect();
        let mut caches = SelectorCaches::default();
        for t in &targets {
            let mut covered = false;
            let mut cur = dom.parent(t.node());
            while let Some(p) = cur {
                if nodes.contains(&p) {
                    covered = true;
                    break;
                }
                cur = dom.parent(p);
            }
            if covered {
                continue;
            }
            match t {
                Target::Subtree(r) => {
                    let mut bloom = AncestorBloom::new();
                    self.compute_subtree(dom, tree, *r, &mut bloom, &mut caches);
                }
                Target::Children(p) => {
                    let mut bloom = self.bloom_with_ancestors(dom, *p);
                    let kids: Vec<NodeId> = dom.children(*p).collect();
                    for c in kids {
                        self.compute_subtree(dom, tree, c, &mut bloom, &mut caches);
                    }
                }
            }
        }
    }

    fn bloom_with_ancestors(&self, dom: &Dom, el: NodeId) -> AncestorBloom {
        let mut chain = vec![el];
        let mut cur = dom.parent(el);
        while let Some(p) = cur {
            chain.push(p);
            cur = dom.parent(p);
        }
        let mut bloom = AncestorBloom::new();
        for n in chain.into_iter().rev() {
            if dom.tag(n).is_some() {
                bloom.push(dom, n);
            }
        }
        bloom
    }

    fn compute_subtree(
        &self,
        dom: &Dom,
        tree: &mut StyleTree,
        start: NodeId,
        bloom: &mut AncestorBloom,
        caches: &mut SelectorCaches,
    ) {
        enum Frame {
            Enter(NodeId),
            Exit(Vec<u32>),
        }
        let root_el = dom.document_element();
        let mut stack = vec![Frame::Enter(start)];
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Exit(hs) => bloom.pop(&hs),
                Frame::Enter(n) => {
                    if dom.tag(n).is_some() {
                        let style = {
                            let parent = match dom.parent(n).and_then(|p| tree.get(p)) {
                                Some(p) => p,
                                None => &self.initial,
                            };
                            let root_fs = match root_el {
                                Some(r) if r != n => tree.get(r).map_or(16.0, |c| c.font_size),
                                _ => 16.0,
                            };
                            self.style_element(dom, n, parent, root_fs, bloom, caches)
                        };
                        tree.set(n, style);
                        let hs = bloom.push(dom, n);
                        stack.push(Frame::Exit(hs));
                    } else {
                        tree.clear(n);
                    }
                    let kids: Vec<NodeId> = dom.children(n).collect();
                    for c in kids.into_iter().rev() {
                        stack.push(Frame::Enter(c));
                    }
                }
            }
        }
    }

    fn collect<'a>(
        &'a self,
        dom: &Dom,
        el: &El<'_>,
        bloom: &AncestorBloom,
        caches: &mut SelectorCaches,
        main: &mut Vec<Entry<'a>>,
        pseudo: &mut [Vec<Entry<'a>>; 2],
    ) {
        let node = el.id;
        let mut candidates: Vec<u32> = Vec::new();
        if let Some(id) = dom.id(node) {
            if let Some(v) = self.by_id.get(id) {
                candidates.extend_from_slice(v);
            }
        }
        for c in dom.classes(node) {
            if let Some(v) = self.by_class.get(c) {
                candidates.extend_from_slice(v);
            }
        }
        if let Some(tag) = dom.tag(node) {
            if let Some(v) = self.by_tag.get(tag) {
                candidates.extend_from_slice(v);
            }
        }
        candidates.extend_from_slice(&self.universal);
        candidates.sort_unstable();
        candidates.dedup();
        for ri in candidates {
            let r = &self.rules[ri as usize];
            if !r.ancestors.iter().all(|h| bloom.might_contain(*h)) {
                continue;
            }
            if !matches(&r.rule.selector, el, caches, r.pseudo.is_some()) {
                continue;
            }
            let target = match r.pseudo {
                None => &mut *main,
                Some(PseudoElementKind::Before) => &mut pseudo[0],
                Some(PseudoElementKind::After) => &mut pseudo[1],
            };
            push_block(target, &r.rule.block, r.origin, r.rule.specificity, r.order);
        }
    }

    fn style_element(
        &self,
        dom: &Dom,
        node: NodeId,
        parent: &Computed,
        root_fs: f32,
        bloom: &AncestorBloom,
        caches: &mut SelectorCaches,
    ) -> NodeStyle {
        let el = match El::new(dom, node) {
            Some(el) => el,
            None => {
                return NodeStyle { computed: Computed::inherit_from(parent), before: None, after: None };
            }
        };
        let inline: Option<DeclBlock> = dom.attr(node, "style").map(parse_declaration_list);
        let mut main: Vec<Entry<'_>> = Vec::new();
        let mut pseudo: [Vec<Entry<'_>>; 2] = [Vec::new(), Vec::new()];
        self.collect(dom, &el, bloom, caches, &mut main, &mut pseudo);
        if let Some(b) = &inline {
            push_block(&mut main, b, Origin::Author, u32::MAX, u32::MAX);
        }
        let computed = self.cascade(main, parent, root_fs);
        let [before, after] = pseudo;
        let before = self.pseudo_style(before, &computed, root_fs);
        let after = self.pseudo_style(after, &computed, root_fs);
        NodeStyle { computed, before, after }
    }

    fn pseudo_style(&self, entries: Vec<Entry<'_>>, element: &Computed, root_fs: f32) -> Option<Box<Computed>> {
        if entries.is_empty() {
            return None;
        }
        let c = self.cascade(entries, element, root_fs);
        match c.content {
            Content::Items(_) => {}
            _ => return None,
        }
        if c.display == Display::None {
            return None;
        }
        Some(Box::new(c))
    }

    fn cascade(&self, mut entries: Vec<Entry<'_>>, parent: &Computed, root_fs: f32) -> Computed {
        entries.sort_by_key(|e| (e.rank, e.spec, e.order));

        // Custom properties: the winner per name, then var() resolution.
        let mut own: BTreeMap<String, String> = BTreeMap::new();
        for e in &entries {
            for d in e.decls {
                if let Declaration::Custom(name, value) = d {
                    own.insert(name.clone(), value.clone());
                }
            }
        }
        let custom: CustomProps =
            if own.is_empty() { parent.custom.clone() } else { Arc::new(resolve_custom(&parent.custom, &own)) };

        // Flatten to longhands in cascade order.
        let mut items: Vec<(Prop, Cow<'_, Value>)> = Vec::new();
        for e in &entries {
            for d in e.decls {
                match d {
                    Declaration::Longhand(p, v) => items.push((*p, Cow::Borrowed(v))),
                    Declaration::Raw(name, text) => match resolve_raw(name, text, &custom) {
                        Some(list) => {
                            for (p, v) in list {
                                items.push((p, Cow::Owned(v)));
                            }
                        }
                        None => {
                            // Invalid at computed-value time: behaves as `unset`.
                            for p in longhands_of(name) {
                                items.push((p, Cow::Owned(Value::Unset)));
                            }
                        }
                    },
                    Declaration::Custom(..) => {}
                }
            }
        }

        let mut c = Computed::inherit_from(parent);
        c.custom = custom;
        let mut explicit = Explicit::default();
        let cx = ApplyCtx {
            parent,
            initial: &self.initial,
            root_font_size: root_fs,
            vw: self.env.width,
            vh: self.env.height,
        };
        for (p, v) in &items {
            if p.is_early() {
                c.apply(*p, v, &cx, &mut explicit);
            }
        }
        for (p, v) in &items {
            if !p.is_early() {
                c.apply(*p, v, &cx, &mut explicit);
            }
        }
        c.finalize(&explicit);
        c
    }

    #[cfg(any(test, feature = "testkit"))]
    #[cfg(test)]
    fn bucket_sizes(&self) -> (usize, usize, usize, usize) {
        let n = |m: &HashMap<String, Vec<u32>>| m.values().map(|v| v.len()).sum();
        (n(&self.by_id), n(&self.by_class), n(&self.by_tag), self.universal.len())
    }
}

/// Resolves the declared custom properties on top of the inherited map.
fn resolve_custom(parent: &CustomProps, own: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut map: BTreeMap<String, String> = (**parent).clone();
    let mut resolved: HashMap<String, Option<String>> = HashMap::new();
    let mut stack: Vec<String> = Vec::new();
    for name in own.keys() {
        resolve_one(name, own, parent, &mut resolved, &mut stack);
    }
    for (name, value) in resolved {
        match value {
            Some(v) => {
                map.insert(name, v);
            }
            None => {
                map.remove(&name);
            }
        }
    }
    map
}

fn resolve_one(
    name: &str,
    own: &BTreeMap<String, String>,
    parent: &CustomProps,
    resolved: &mut HashMap<String, Option<String>>,
    stack: &mut Vec<String>,
) -> Option<String> {
    if let Some(r) = resolved.get(name) {
        return r.clone();
    }
    if stack.iter().any(|s| s == name) {
        return None;
    }
    let raw = own.get(name)?;
    stack.push(name.to_string());
    let value = match raw.trim().to_ascii_lowercase().as_str() {
        "initial" => None,
        "inherit" | "unset" | "revert" | "revert-layer" => parent.get(name).cloned(),
        _ => substitute_vars(raw, &mut |n| {
            if own.contains_key(n) {
                resolve_one(n, own, parent, resolved, stack)
            } else {
                parent.get(n).cloned()
            }
        }),
    };
    stack.pop();
    resolved.insert(name.to_string(), value.clone());
    value
}

#[cfg(test)]
mod tests {
    use super::super::super::dom::parse_html;
    use super::super::parse::parse_stylesheet;
    use super::super::values::*;
    use super::*;

    fn env() -> MediaEnv {
        MediaEnv { width: 800.0, height: 600.0, dark: false }
    }

    fn engine_with(ua: &str, author: &str, env: MediaEnv) -> StyleEngine {
        StyleEngine::new(
            vec![parse_stylesheet(ua, Origin::UserAgent), parse_stylesheet(author, Origin::Author)],
            env,
        )
    }

    fn styled(body: &str, ua: &str, author: &str) -> (Dom, StyleTree) {
        let dom = parse_html(&format!("<html><body>{}</body></html>", body));
        let tree = engine_with(ua, author, env()).compute(&dom);
        (dom, tree)
    }

    fn of<'a>(dom: &Dom, tree: &'a StyleTree, id: &str) -> &'a Computed {
        tree.get(dom.get_element_by_id(id).unwrap_or_else(|| panic!("no #{}", id))).expect("styled")
    }

    fn red() -> Rgba {
        Rgba::from_u8(255, 0, 0, 1.0)
    }
    fn blue() -> Rgba {
        Rgba::from_u8(0, 0, 255, 1.0)
    }
    fn green() -> Rgba {
        Rgba::from_u8(0, 128, 0, 1.0)
    }

    #[test]
    fn author_beats_user_agent() {
        let (dom, tree) = styled("<p id=a>x</p>", "p { color: red }", "p { color: blue }");
        assert_eq!(of(&dom, &tree, "a").color, blue());
    }

    #[test]
    fn important_reverses_origins() {
        let (dom, tree) = styled("<p id=a>x</p>", "p { color: red !important }", "p { color: blue }");
        assert_eq!(of(&dom, &tree, "a").color, red());
        let (dom, tree) = styled("<p id=a>x</p>", "p { color: red !important }", "p { color: blue !important }");
        assert_eq!(of(&dom, &tree, "a").color, red());
        let (dom, tree) = styled("<p id=a>x</p>", "p { color: red }", "p { color: blue !important }");
        assert_eq!(of(&dom, &tree, "a").color, blue());
    }

    #[test]
    fn specificity_then_source_order() {
        let (dom, tree) = styled(
            "<p id=a class=b>x</p>",
            "",
            "p { color: red } .b { color: blue } #a { color: green } p { color: red }",
        );
        assert_eq!(of(&dom, &tree, "a").color, green());
        let (dom, tree) = styled("<p id=a class=b>x</p>", "", ".b { color: red } p.b { color: blue } .b { color: green }");
        assert_eq!(of(&dom, &tree, "a").color, blue());
        let (dom, tree) = styled("<p id=a>x</p>", "", "p { color: red } p { color: blue }");
        assert_eq!(of(&dom, &tree, "a").color, blue());
    }

    #[test]
    fn important_within_author_by_specificity() {
        let (dom, tree) = styled("<p id=a>x</p>", "", "#a { color: blue } p { color: red !important }");
        assert_eq!(of(&dom, &tree, "a").color, red());
    }

    #[test]
    fn inline_style_wins_unless_important() {
        let (dom, tree) = styled("<p id=a style='color: red'>x</p>", "", "#a { color: blue }");
        assert_eq!(of(&dom, &tree, "a").color, red());
        let (dom, tree) = styled("<p id=a style='color: red'>x</p>", "", "#a { color: blue !important }");
        assert_eq!(of(&dom, &tree, "a").color, blue());
        let (dom, tree) = styled("<p id=a style='color: red !important'>x</p>", "", "#a { color: blue !important }");
        assert_eq!(of(&dom, &tree, "a").color, red());
    }

    #[test]
    fn inheritance_of_inherited_and_non_inherited() {
        let (dom, tree) = styled(
            "<div id=d><span id=s>x</span></div>",
            "",
            "div { color: red; font-size: 20px; width: 100px; margin-top: 7px }",
        );
        let s = of(&dom, &tree, "s");
        assert_eq!(s.color, red());
        assert_eq!(s.font_size, 20.0);
        assert_eq!(s.width, Length::Auto);
        assert_eq!(s.margin_top, Length::ZERO);
    }

    #[test]
    fn css_wide_keywords_in_cascade() {
        let (dom, tree) = styled(
            "<div id=d><span id=s>x</span></div>",
            "",
            "div { width: 100px; color: red; padding-top: 3px } span { width: inherit; color: initial; padding-top: unset }",
        );
        let s = of(&dom, &tree, "s");
        assert_eq!(s.width, Length::Px(100.0));
        assert_eq!(s.color, Rgba::BLACK);
        assert_eq!(s.padding_top, Length::ZERO);
        let (dom, tree) = styled("<div id=d><span id=s>x</span></div>", "", "div { color: red } span { color: unset }");
        assert_eq!(of(&dom, &tree, "s").color, red());
    }

    #[test]
    fn relative_units_resolve_in_cascade() {
        let (dom, tree) = styled(
            "<div id=d><p id=p>x</p></div>",
            "",
            "html { font-size: 10px } div { font-size: 20px } p { margin-top: 2em; padding-top: 2rem; width: 10vw; height: 50vh }",
        );
        let p = of(&dom, &tree, "p");
        assert_eq!(p.margin_top, Length::Px(40.0));
        assert_eq!(p.padding_top, Length::Px(20.0));
        assert_eq!(p.width, Length::Px(80.0));
        assert_eq!(p.height, Length::Px(300.0));
    }

    #[test]
    fn calc_in_cascade_keeps_percent() {
        let (dom, tree) = styled("<p id=p>x</p>", "", "p { width: calc(100% - 2em); font-size: 10px }");
        let w = &of(&dom, &tree, "p").width;
        assert_eq!(w.resolve(Some(200.0)), Some(180.0));
    }

    #[test]
    fn custom_properties_and_var() {
        let (dom, tree) = styled(
            "<div id=d><p id=p>x</p></div>",
            "",
            ":root { --c: red; --gap: 4px } p { color: var(--c); margin-top: var(--gap); padding-top: var(--none, 9px) }",
        );
        let p = of(&dom, &tree, "p");
        assert_eq!(p.color, red());
        assert_eq!(p.margin_top, Length::Px(4.0));
        assert_eq!(p.padding_top, Length::Px(9.0));
        assert_eq!(p.custom.get("--c").map(|s| s.as_str()), Some("red"));
    }

    #[test]
    fn custom_property_inheritance_and_override() {
        let (dom, tree) = styled(
            "<div id=d><section id=s><p id=p>x</p></section><p id=q>y</p></div>",
            "",
            "div { --c: red } section { --c: blue } p { color: var(--c) }",
        );
        assert_eq!(of(&dom, &tree, "p").color, blue());
        assert_eq!(of(&dom, &tree, "q").color, red());
    }

    #[test]
    fn custom_property_references_and_cycles() {
        let (dom, tree) = styled(
            "<p id=p>x</p>",
            "",
            "p { --a: 5px; --b: calc(var(--a) * 2); margin-top: var(--b); --x: var(--y); --y: var(--x); color: var(--x, blue) }",
        );
        let p = of(&dom, &tree, "p");
        assert_eq!(p.margin_top, Length::Px(10.0));
        assert!(p.custom.get("--x").is_none());
        assert_eq!(p.color, blue());
    }

    #[test]
    fn var_in_shorthand_and_invalid_at_computed_time() {
        let (dom, tree) = styled(
            "<p id=p>x</p><p id=q>y</p>",
            "",
            "#p { --m: 1px 2px; margin: var(--m) 3px 4px } #q { color: red } #q { color: var(--nope) }",
        );
        let p = of(&dom, &tree, "p");
        assert_eq!(p.margin_top, Length::Px(1.0));
        assert_eq!(p.margin_right, Length::Px(2.0));
        assert_eq!(p.margin_bottom, Length::Px(3.0));
        assert_eq!(p.margin_left, Length::Px(4.0));
        // `color: var(--nope)` is invalid at computed-value time: unset -> inherited.
        assert_eq!(of(&dom, &tree, "q").color, Rgba::BLACK);
    }

    #[test]
    fn custom_property_important_and_initial() {
        let (dom, tree) = styled(
            "<div id=d><p id=p>x</p></div>",
            "",
            "div { --c: red !important } div { --c: blue } p { --c: initial; color: var(--c, green) }",
        );
        assert_eq!(of(&dom, &tree, "d").custom.get("--c").map(|s| s.as_str()), Some("red"));
        assert_eq!(of(&dom, &tree, "p").color, green());
    }

    #[test]
    fn media_queries_filter_rules_at_construction() {
        let css = "@media (min-width: 900px) { p { color: red } } @media (max-width: 900px) { p { color: blue } }";
        let (dom, tree) = styled("<p id=a>x</p>", "", css);
        assert_eq!(of(&dom, &tree, "a").color, blue());
        let dom = parse_html("<p id=a>x</p>");
        let wide = engine_with("", css, MediaEnv { width: 1000.0, height: 600.0, dark: false }).compute(&dom);
        assert_eq!(of(&dom, &wide, "a").color, red());
        let dark = engine_with(
            "",
            "p { color: black } @media (prefers-color-scheme: dark) { p { color: white } }",
            MediaEnv { width: 1000.0, height: 600.0, dark: true },
        )
        .compute(&dom);
        assert_eq!(of(&dom, &dark, "a").color, Rgba::WHITE);
    }

    #[test]
    fn viewport_units_follow_the_environment() {
        let dom = parse_html("<p id=a>x</p>");
        let t = engine_with("", "p { width: 50vw; height: 10vh }", MediaEnv { width: 1000.0, height: 500.0, dark: false })
            .compute(&dom);
        assert_eq!(of(&dom, &t, "a").width, Length::Px(500.0));
        assert_eq!(of(&dom, &t, "a").height, Length::Px(50.0));
    }

    #[test]
    fn generated_content_before_and_after() {
        let (dom, tree) = styled(
            "<p id=a>x</p><p id=b>y</p><p id=c>z</p>",
            "",
            "p { color: red } #a::before { content: \"[\"; color: blue } #a::after { content: \"]\" } #b::before { content: none } #c::before { color: blue } #c:after { content: \"\"; display: none }",
        );
        let a = dom.get_element_by_id("a").unwrap();
        let before = tree.before(a).expect("before");
        assert_eq!(before.content, Content::Items(vec![ContentItem::Str("[".into())]));
        assert_eq!(before.color, blue());
        assert_eq!(before.display, Display::Inline);
        let after = tree.after(a).expect("after");
        assert_eq!(after.color, red());
        let b = dom.get_element_by_id("b").unwrap();
        assert!(tree.before(b).is_none());
        let c = dom.get_element_by_id("c").unwrap();
        assert!(tree.before(c).is_none());
        assert!(tree.after(c).is_none());
    }

    #[test]
    fn pseudo_rules_do_not_style_the_element() {
        let (dom, tree) = styled("<p id=a>x</p>", "", "p::before { content: \"x\"; color: red }");
        assert_eq!(of(&dom, &tree, "a").color, Rgba::BLACK);
    }

    #[test]
    fn rules_are_indexed_into_buckets() {
        let e = engine_with("", "#a { color: red } .b { color: red } p { color: red } * { color: red } [x] { color: red } .c .d { color: red }", env());
        assert_eq!(e.rule_count(), 6);
        assert_eq!(e.bucket_sizes(), (1, 2, 1, 2));
    }

    #[test]
    fn descendant_selectors_with_bloom_prefilter() {
        let (dom, tree) = styled(
            "<div class=a><section><p id=in>x</p></section></div><div class=z><p id=out>y</p></div>",
            "",
            ".a p { color: red } .a > p { color: blue } section p { margin-top: 5px } #nothere p { color: green }",
        );
        assert_eq!(of(&dom, &tree, "in").color, red());
        assert_eq!(of(&dom, &tree, "in").margin_top, Length::Px(5.0));
        assert_eq!(of(&dom, &tree, "out").color, Rgba::BLACK);
        assert_eq!(of(&dom, &tree, "out").margin_top, Length::ZERO);
    }

    #[test]
    fn bloom_push_pop() {
        let dom = parse_html("<div id=x class='a b'><p>t</p></div>");
        let x = dom.get_element_by_id("x").unwrap();
        let mut b = AncestorBloom::new();
        assert!(!b.might_contain(hash_str(HASH_CLASS, "a")));
        let hs = b.push(&dom, x);
        assert!(b.might_contain(hash_str(HASH_CLASS, "a")));
        assert!(b.might_contain(hash_str(HASH_ID, "x")));
        assert!(b.might_contain(hash_str(HASH_TAG, "div")));
        b.pop(&hs);
        assert!(!b.might_contain(hash_str(HASH_CLASS, "a")));
    }

    #[test]
    fn sibling_and_structural_selectors_in_cascade() {
        let (dom, tree) = styled(
            "<ul><li id=a>1</li><li id=b>2</li><li id=c>3</li></ul>",
            "",
            "li:first-child { color: red } li + li { color: blue } li:last-child { color: green } li:nth-child(2) { margin-top: 3px }",
        );
        assert_eq!(of(&dom, &tree, "a").color, red());
        assert_eq!(of(&dom, &tree, "b").color, blue());
        assert_eq!(of(&dom, &tree, "c").color, green());
        assert_eq!(of(&dom, &tree, "b").margin_top, Length::Px(3.0));
    }

    #[test]
    fn border_colors_follow_current_color() {
        let (dom, tree) = styled("<p id=a>x</p>", "", "p { border: 2px solid; color: red }");
        let a = of(&dom, &tree, "a");
        assert_eq!(a.border_top_color, red());
        assert_eq!(a.border_top_width, 2.0);
        let (dom, tree) = styled("<p id=a>x</p>", "", "p { color: red; border: 2px solid blue; border-left-color: green }");
        let a = of(&dom, &tree, "a");
        assert_eq!(a.border_top_color, blue());
        assert_eq!(a.border_left_color, green());
    }

    #[test]
    fn font_faces_are_collected_across_sheets() {
        let e = engine_with("", "@font-face { font-family: X; src: url(x.woff2) }", env());
        assert_eq!(e.font_faces().len(), 1);
        assert_eq!(e.font_faces()[0].family, "X");
    }

    #[test]
    fn recompute_matches_a_fresh_compute_after_mutation() {
        let mut dom = parse_html(
            "<div id=w><p id=a class=x>1</p><p id=b>2</p><p id=c>3</p></div><section id=s><span id=t>t</span></section>",
        );
        let engine = engine_with("", ".y { color: red } .y + p { color: blue } p.x { margin-top: 5px } #w { font-size: 20px } #w p { padding-top: 1em }", env());
        let mut tree = engine.compute(&dom);
        let a = dom.get_element_by_id("a").unwrap();
        dom.set_attr(a, "class", "y");
        engine.recompute(&dom, &mut tree, &[a]);
        assert_eq!(tree, engine.compute(&dom));
        assert_eq!(of(&dom, &tree, "a").color, red());
        assert_eq!(of(&dom, &tree, "b").color, blue());
        assert_eq!(of(&dom, &tree, "a").margin_top, Length::ZERO);

        let w = dom.get_element_by_id("w").unwrap();
        dom.set_attr(w, "style", "font-size: 40px");
        engine.recompute(&dom, &mut tree, &[w]);
        assert_eq!(tree, engine.compute(&dom));
    }

    #[test]
    fn recompute_handles_new_and_removed_nodes() {
        let mut dom = parse_html("<div id=w><p id=a>1</p></div>");
        let engine = engine_with("", "p { color: red } .n { color: blue }", env());
        let mut tree = engine.compute(&dom);
        let w = dom.get_element_by_id("w").unwrap();
        let n = dom.create_element("p");
        dom.set_attr(n, "class", "n");
        dom.append_child(w, n);
        engine.recompute(&dom, &mut tree, &[n]);
        assert_eq!(tree.get(n).unwrap().color, blue());
        assert_eq!(tree, engine.compute(&dom));
        let a = dom.get_element_by_id("a").unwrap();
        dom.remove(a);
        engine.recompute(&dom, &mut tree, &[a, w]);
        assert!(tree.get(a).is_none());
    }

    #[test]
    fn recompute_root_restyles_everything() {
        let mut dom = parse_html("<p id=a>1</p>");
        let engine = engine_with("", "html.d p { color: red }", env());
        let mut tree = engine.compute(&dom);
        let root = dom.document_element().unwrap();
        dom.set_attr(root, "class", "d");
        engine.recompute(&dom, &mut tree, &[root]);
        assert_eq!(of(&dom, &tree, "a").color, red());
        assert_eq!(tree, engine.compute(&dom));
    }

    #[test]
    fn tree_lookups() {
        let (dom, tree) = styled("<p id=a>x</p>", "", "");
        assert!(tree.get(dom.root()).is_none());
        assert!(!tree.is_empty());
        assert!(tree.len() >= 4);
        let text = dom.first_child(dom.get_element_by_id("a").unwrap()).unwrap();
        assert!(tree.get(text).is_none());
    }
}
