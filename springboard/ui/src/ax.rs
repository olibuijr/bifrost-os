//! The element map: every addressable thing in the OS and in apps, named by code and never by hand.
//!
//! An address is `scope/path/role-or-name`, built from the labels the user sees, for example
//! `os/settings/general/software-update` or `app/is.olibuijr.notes/button/new-note`. Each producer (a Settings page spec, the
//! menu bar, the dock, an app's semantic nodes) walks what it already has and calls [`Map::add`]; the map slugs the name and keeps
//! addresses unique. Agents list the map, resolve a name to one address, and press it. parity: `AXUIElement` identifiers
//! and the accessibility hierarchy (`NativeAccessibility`, `NativeAccessibility`), see docs/agent-api.md.
use crate::{Node, Rect, Role};

/// Lower-case ASCII words joined by `-`: "Software Update" is `software-update`, "Wi-Fi" is `wi-fi`, "Ólafur" is `lafur`.
/// Icelandic letters fold to their ASCII base first so names stay readable.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.chars() {
        let c = match c {
            'á' | 'Á' => 'a', 'é' | 'É' => 'e', 'í' | 'Í' => 'i', 'ó' | 'Ó' | 'ö' | 'Ö' => 'o', 'ú' | 'Ú' => 'u', 'ý' | 'Ý' => 'y',
            'ð' | 'Ð' => 'd', 'þ' | 'Þ' => 't', 'æ' | 'Æ' => 'a',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else {
            gap = true;
        }
    }
    out
}

/// The role word used in addresses and listings.
pub fn role_name(role: Role) -> &'static str {
    match role {
        Role::Button => "button",
        Role::TextField => "field",
        Role::List => "list",
        Role::Row => "row",
        Role::Toggle => "toggle",
        Role::Checkbox => "checkbox",
        Role::Slider => "slider",
        Role::SegmentedControl => "segmented-control",
        Role::StaticText => "text",
        Role::MenuItem => "menu-item",
        Role::Window => "window",
        Role::ProgressIndicator => "progress-indicator",
    }
}

/// the announcement-requested accessibility notification priority: low, medium (default) and high. On the
/// AT-SPI bus they map to the `announcement` event's politeness: low and medium are polite, high is assertive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Priority {
    Low = 0,
    #[default]
    Medium = 1,
    High = 2,
}

impl Priority {
    pub fn from_wire(v: u8) -> Option<Priority> {
        Some(match v {
            0 => Priority::Low,
            1 => Priority::Medium,
            2 => Priority::High,
            _ => return None,
        })
    }
    /// `AtspiLive` politeness: 1 polite, 2 assertive.
    pub fn politeness(self) -> i32 {
        if self == Priority::High { 2 } else { 1 }
    }
}

/// Text attributes of one text element (`AXSelectedText`, `AXSelectedTextRange`, `AXVisibleCharacterRange`, the insertion
/// point and the character count). Offsets are Unicode scalar (char) offsets, which is what AT-SPI `Text` uses. A secure
/// field (`isSecureTextEntry`) carries only its length: the element's value is masked and no offset reveals content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct AxText {
    /// the `Node::id` this describes
    pub id: u64,
    /// the insertion point (moving end of the selection)
    pub caret: u32,
    pub sel_start: u32,
    pub sel_end: u32,
    /// `AXVisibleCharacterRange`: the part not scrolled out of the field
    pub vis_start: u32,
    pub vis_end: u32,
    pub secure: bool,
    /// characters in the field (for a secure field, the masked length)
    pub len: u32,
}

impl AxText {
    /// `AXSelectedTextRange`: `(start, end)`; empty when only a caret.
    pub fn selected_range(&self) -> (u32, u32) {
        (self.sel_start, self.sel_end)
    }
    /// `AXSelectedText` of `text` (the element's value). A secure field answers an empty string.
    pub fn selected_text(&self, text: &str) -> String {
        if self.secure {
            return String::new();
        }
        text.chars().skip(self.sel_start as usize).take(self.sel_end.saturating_sub(self.sel_start) as usize).collect()
    }
}

/// The label/title/help split (`accessibilityTitle`, `accessibilityHelp`); `Node::label` stays `accessibilityLabel`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AxMeta {
    pub id: u64,
    /// `accessibilityTitle`: the visible title of the control
    pub title: String,
    /// `accessibilityHelp`: what the control does, spoken after the label
    pub help: String,
}

/// Char offset of byte `b` in `s` (clamped to `s` and rounded down to a character boundary).
pub fn char_offset(s: &str, b: usize) -> u32 {
    let mut b = b.min(s.len());
    while !s.is_char_boundary(b) {
        b -= 1;
    }
    s[..b].chars().count() as u32
}

/// Builds the [`AxText`] of a text element from byte offsets: `anchor` and `head` are the selection ends, `visible` the
/// shown byte range.
pub fn text_ax(id: u64, text: &str, anchor: usize, head: usize, visible: (usize, usize), secure: bool) -> AxText {
    let (a, h) = (char_offset(text, anchor), char_offset(text, head));
    let vs = char_offset(text, visible.0);
    AxText { id, caret: h, sel_start: a.min(h), sel_end: a.max(h), vis_start: vs, vis_end: char_offset(text, visible.1).max(vs), secure, len: text.chars().count() as u32 }
}

/// What a secure field exposes as its value: one bullet per character.
pub fn masked(len: u32) -> String {
    "\u{2022}".repeat(len as usize)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub address: String,
    /// the label as the user reads it
    pub name: String,
    /// "page", "row", "toggle", "slider", "button", "field", "menu", "item", "app", ...
    pub role: String,
    pub value: String,
    pub enabled: bool,
    /// window-local points when the element is on screen
    pub bounds: Option<Rect>,
    /// the producer's own handle for acting on it (a node id, a row index); empty when the address alone is enough
    pub handle: String,
}

#[derive(Debug, PartialEq)]
pub enum Resolve<'a> {
    One(&'a Element),
    None,
    /// the query matched several: the candidates' addresses
    Ambiguous(Vec<&'a str>),
}

#[derive(Default, Debug)]
pub struct Map {
    elements: Vec<Element>,
}

impl Map {
    pub fn new() -> Self {
        Map::default()
    }

    pub fn elements(&self) -> &[Element] {
        &self.elements
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Adds `name` under `parent` (an address, or a scope such as `os/settings`) and returns its address. A repeated name under
    /// the same parent becomes `name-2`, `name-3`, in the order added, so a list with two identical rows stays addressable.
    pub fn add(&mut self, parent: &str, role: &str, name: &str, value: &str, enabled: bool, bounds: Option<Rect>, handle: &str) -> String {
        let base = match slug(name) {
            s if s.is_empty() => role.to_string(),
            s => s,
        };
        let mut address = format!("{parent}/{base}");
        let mut n = 2;
        while self.elements.iter().any(|e| e.address == address) {
            address = format!("{parent}/{base}-{n}");
            n += 1;
        }
        self.elements.push(Element { address: address.clone(), name: name.into(), role: role.into(), value: value.into(), enabled, bounds, handle: handle.into() });
        address
    }

    /// An app's semantic nodes under `scope`, grouped by role: `scope/button/new-note`. The handle is the node id.
    pub fn add_nodes(&mut self, scope: &str, nodes: &[Node]) {
        for n in nodes {
            let parent = format!("{scope}/{}", role_name(n.role));
            self.add(&parent, role_name(n.role), &n.label, &n.value, n.enabled, Some(n.bounds), &n.id.to_string());
        }
    }

    pub fn get(&self, address: &str) -> Option<&Element> {
        self.elements.iter().find(|e| e.address == address)
    }

    /// Everything at or below `prefix` (`os/settings` lists all of Settings), in the order added.
    pub fn under<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        let prefix = prefix.trim().trim_end_matches('/');
        let dir = format!("{prefix}/");
        // an empty prefix is the whole map
        self.elements.iter().filter(move |e| prefix.is_empty() || e.address == prefix || e.address.starts_with(&dir))
    }

    /// Finds one element from what an agent typed: the full address, else the unique address that ends with the query
    /// (`general/software-update`), else the unique element whose name slugs to the query (`software-update`).
    pub fn resolve(&self, query: &str) -> Resolve<'_> {
        // an exact address wins (an app id has dots, which slugging would turn into dashes)
        if let Some(e) = self.get(query.trim().trim_matches('/')) {
            return Resolve::One(e);
        }
        // "General > Software Update" and "general/software-update" are the same path
        let normal = query.split(['>', '/']).map(slug).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("/");
        let q = if normal.is_empty() { query.trim().trim_matches('/') } else { normal.as_str() };
        if let Some(e) = self.get(q) {
            return Resolve::One(e);
        }
        let by_suffix: Vec<&Element> = self.elements.iter().filter(|e| e.address.ends_with(&format!("/{q}"))).collect();
        let hits = if by_suffix.is_empty() {
            let s = slug(q);
            self.elements.iter().filter(|e| !s.is_empty() && slug(&e.name) == s).collect()
        } else {
            by_suffix
        };
        match hits.as_slice() {
            [] => Resolve::None,
            [one] => Resolve::One(one),
            many => Resolve::Ambiguous(many.iter().map(|e| e.address.as_str()).collect()),
        }
    }

    /// One line per element: `address<TAB>role<TAB>name<TAB>value<TAB>enabled`. The listing agents read.
    pub fn listing(&self, prefix: &str) -> String {
        let mut out = String::new();
        for e in self.under(prefix) {
            out.push_str(&format!("{}\t{}\t{}\t{}\t{}\n", e.address, e.role, e.name, e.value, if e.enabled { "enabled" } else { "disabled" }));
        }
        out
    }
}

// --- recording: the kit's draw helpers publish what they draw, so every app is mapped without writing code ---------------------
use std::cell::RefCell;
thread_local! {
    static FRAME: RefCell<Vec<Node>> = const { RefCell::new(Vec::new()) };
}

/// Starts a frame: forgets what the last one recorded.
pub fn begin_frame() {
    FRAME.with(|f| f.borrow_mut().clear());
}

/// Publishes one drawn element. `bounds` are window-local points. The id is stable across frames: a hash of the role and label,
/// plus how many identical ones were drawn before it in this frame.
pub fn record(role: Role, label: &str, value: &str, bounds: Rect, enabled: bool) {
    FRAME.with(|f| {
        let mut f = f.borrow_mut();
        let same = f.iter().filter(|n| n.role == role && n.label == label).count();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in (role as u32).to_le_bytes().iter().chain(label.as_bytes()).chain(&(same as u32).to_le_bytes()) {
            h = (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3);
        }
        if f.len() < 400 {
            f.push(Node { id: h, role, label: label.into(), value: value.into(), bounds, enabled });
        }
    });
}

/// What the frame drew, in drawing order.
pub fn take_frame() -> Vec<Node> {
    FRAME.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn recording_gives_stable_ids_and_numbers_repeats() {
        use super::*;
        let r = Rect { x: 1., y: 2., w: 3., h: 4. };
        begin_frame();
        record(Role::Button, "Save", "", r, true);
        record(Role::Button, "Save", "", r, true);
        let a = take_frame();
        begin_frame();
        record(Role::Button, "Save", "", r, true);
        record(Role::Button, "Save", "", r, true);
        let b = take_frame();
        assert_eq!(a, b, "the same frame records the same ids");
        assert_ne!(a[0].id, a[1].id);
        assert!(take_frame().is_empty(), "taking empties the recorder");
    }

    use super::*;

    fn node(id: u64, role: Role, label: &str) -> Node {
        Node { id, role, label: label.into(), value: String::new(), bounds: Rect { x: 0., y: 0., w: 10., h: 10. }, enabled: true }
    }

    #[test]
    fn slugs_are_readable_ascii_words() {
        assert_eq!(slug("Software Update"), "software-update");
        assert_eq!(slug("  Wi-Fi  "), "wi-fi");
        assert_eq!(slug("Desktop & Dock"), "desktop-dock");
        assert_eq!(slug("Þjónusta og Ísland"), "tjonusta-og-island");
        assert_eq!(slug("…"), "");
    }

    #[test]
    fn repeated_names_get_numbered_in_order() {
        let mut m = Map::new();
        assert_eq!(m.add("os/x", "row", "Name", "", true, None, ""), "os/x/name");
        assert_eq!(m.add("os/x", "row", "Name", "", true, None, ""), "os/x/name-2");
        assert_eq!(m.add("os/x", "row", "Name", "", true, None, ""), "os/x/name-3");
        assert_eq!(m.add("os/y", "row", "Name", "", true, None, ""), "os/y/name");
        assert_eq!(m.add("os/x", "button", "", "", true, None, ""), "os/x/button", "an unnamed element falls back to its role");
    }

    #[test]
    fn nodes_are_grouped_by_role_under_their_scope() {
        let mut m = Map::new();
        m.add_nodes("app/is.olibuijr.notes", &[node(7, Role::Button, "New Note"), node(9, Role::TextField, "Search"), node(10, Role::Button, "New Note")]);
        assert_eq!(m.get("app/is.olibuijr.notes/button/new-note").unwrap().handle, "7");
        assert_eq!(m.get("app/is.olibuijr.notes/button/new-note-2").unwrap().handle, "10");
        assert_eq!(m.get("app/is.olibuijr.notes/field/search").unwrap().role, "field");
    }

    #[test]
    fn resolve_takes_an_address_a_suffix_or_a_unique_name() {
        let mut m = Map::new();
        m.add("os/settings/general", "row", "Software Update", "", true, None, "");
        m.add("os/settings/general", "row", "About", "", true, None, "");
        m.add("os/settings/privacy", "row", "About", "", true, None, "");
        assert!(matches!(m.resolve("os/settings/general/software-update"), Resolve::One(e) if e.name == "Software Update"));
        assert!(matches!(m.resolve("general/about"), Resolve::One(e) if e.address == "os/settings/general/about"));
        assert!(matches!(m.resolve("Software Update"), Resolve::One(_)), "a typed name works");
        assert_eq!(m.resolve("about"), Resolve::Ambiguous(vec!["os/settings/general/about", "os/settings/privacy/about"]));
        assert!(matches!(m.resolve("General > Software Update"), Resolve::One(e) if e.name == "Software Update"));
        assert_eq!(m.resolve("nothing"), Resolve::None);
    }

    #[test]
    fn an_exact_address_with_dots_resolves() {
        let mut m = Map::new();
        m.add_nodes("app/is.olibuijr.files.desktop", &[node(1, Role::Button, "Change view")]);
        assert!(matches!(m.resolve("app/is.olibuijr.files.desktop/button/change-view"), Resolve::One(e) if e.handle == "1"));
        assert!(matches!(m.resolve("button/change-view"), Resolve::One(_)), "a suffix still works");
    }

    #[test]
    fn under_is_a_subtree_not_a_string_prefix() {
        let mut m = Map::new();
        m.add("os/settings", "row", "General", "", true, None, "");
        m.add("os/settings2", "row", "General", "", true, None, "");
        let all: Vec<_> = m.under("os/settings").map(|e| e.address.as_str()).collect();
        assert_eq!(all, ["os/settings/general"]);
        assert_eq!(m.under("").count(), 2, "an empty prefix lists everything");
        assert!(m.listing("os/settings").starts_with("os/settings/general\trow\tGeneral\t\tenabled\n"));
    }
}

#[cfg(test)]
mod text_tests {
    use super::*;

    #[test]
    fn selected_text_follows_the_caret() {
        // "héllo wörld": drag back from the 'w' to the second character
        let t = "h\u{e9}llo w\u{f6}rld";
        let a = text_ax(1, t, t.find('w').unwrap(), 3, (0, t.len()), false);
        assert_eq!(a.caret, 2, "the caret is the moving end");
        assert_eq!(a.selected_range(), (2, 6));
        assert_eq!(a.selected_text(t), "llo ");
        let caret_only = text_ax(1, t, 4, 4, (0, t.len()), false);
        assert_eq!(caret_only.selected_range(), (3, 3));
        assert_eq!(caret_only.selected_text(t), "");
    }

    #[test]
    fn password_fields_never_expose_text() {
        let a = text_ax(2, "hunter2", 0, 7, (0, 7), true);
        assert_eq!(a.selected_text("hunter2"), "", "AXSelectedText is empty for a secure field");
        assert_eq!(a.len, 7);
        assert_eq!(masked(a.len).chars().count(), 7);
        assert!(masked(a.len).chars().all(|c| c == '\u{2022}'));
    }

    #[test]
    fn visible_range_is_in_characters_and_never_inverted() {
        let t = "\u{e9}\u{e9}\u{e9}\u{e9}";
        let a = text_ax(3, t, 0, 0, (2, 6), false);
        assert_eq!((a.vis_start, a.vis_end), (1, 3));
        assert_eq!(text_ax(3, t, 0, 0, (6, 2), false).vis_end, 3);
        assert_eq!(char_offset(t, 3), 1, "a byte inside a character rounds down");
    }

    #[test]
    fn announcement_priority_maps_to_politeness() {
        assert_eq!(Priority::from_wire(2), Some(Priority::High));
        assert_eq!(Priority::from_wire(3), None);
        assert_eq!((Priority::Low.politeness(), Priority::Medium.politeness(), Priority::High.politeness()), (1, 1, 2));
    }
}
