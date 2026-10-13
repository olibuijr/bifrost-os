//! `NativePasteboard`: pasteboards with typed items. Types are UTType ids (`public.utf8-plain-text`, `public.html`,
//! `public.rtf`, `public.png`, `public.file-url`); the shell maps them to MIME types for the Wayland clipboard
//! (`springboard::shell::pasteboard::mime_for`). The plain-string API (`Ctx::pasteboard_string`) keeps working.
//!
//! A board holds a list of [`PasteboardItem`]s; the plain `set_*`/`data` calls work on the first item. Boards other
//! than the general one (`named`, `drag`, `find`) live inside the app.
//!
//! parity gap: the shell wire carries a flat (type, data) list, so only the first item of the general board reaches
//! the shell, and the shell publishes only the text-like type to the Wayland selection for now; other types and items
//! stay inside BifrOSt apps until the presenter offers multi-MIME data sources.

use crate::pasteboard_item::PasteboardItem;
use crate::proto::ToShell;
use crate::Ctx;
use bifrost_ui::uttype;
use std::cell::RefCell;
use std::collections::HashMap;

pub const STRING: &str = "public.utf8-plain-text";
pub const HTML: &str = "public.html";
pub const RTF: &str = "public.rtf";
pub const PNG: &str = "public.png";
pub const FILE_URL: &str = "public.file-url";

/// Names of the system pasteboards.
pub const GENERAL: &str = "bifrost general";
pub const DRAG: &str = "bifrost drag";
pub const FIND: &str = "bifrost find";

#[derive(Default)]
struct State {
    change_count: u32,
    items: Vec<PasteboardItem>,
}

thread_local! {
    static BOARDS: RefCell<HashMap<String, State>> = RefCell::new(HashMap::new());
}

fn with<R>(name: &str, f: impl FnOnce(&mut State) -> R) -> R {
    BOARDS.with(|b| f(b.borrow_mut().entry(name.to_string()).or_default()))
}

/// Applies the shell's `ToApp::Pasteboard` (the general board; the flat list becomes one item).
pub(crate) fn mirror(change_count: u32, items: Vec<(String, Vec<u8>)>) {
    with(GENERAL, |s| {
        s.change_count = change_count;
        s.items = if items.is_empty() { Vec::new() } else { vec![PasteboardItem::from_representations(items)] };
    });
}

/// A named pasteboard. Get it with [`NativePasteboard::general`] or [`NativePasteboard::named`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativePasteboard {
    name: String,
}

impl NativePasteboard {
    pub fn general() -> Self {
        Self::named(GENERAL)
    }
    /// The pasteboard with the given name.
    pub fn named(name: &str) -> Self {
        NativePasteboard { name: name.to_string() }
    }
    /// The drag pasteboard.
    pub fn drag() -> Self {
        Self::named(DRAG)
    }
    /// The find pasteboard.
    pub fn find() -> Self {
        Self::named(FIND)
    }
    /// `name`.
    pub fn name(&self) -> &str {
        &self.name
    }
    fn is_general(&self) -> bool {
        self.name == GENERAL
    }
    fn push(&self, ctx: &Ctx) {
        if !self.is_general() {
            return;
        }
        let items = with(GENERAL, |s| s.items.first().map(|i| i.representations().to_vec()).unwrap_or_default());
        ctx.send_to_shell(ToShell::PasteboardSet { items });
    }
    /// `clearContents`: empties the pasteboard and returns the new change count.
    pub fn clear_contents(&self, ctx: &Ctx) -> u32 {
        let n = with(&self.name, |s| {
            s.items.clear();
            s.change_count = s.change_count.wrapping_add(1);
            s.change_count
        });
        self.push(ctx);
        n
    }
    /// `writeObjects(_:)` for items: appends them. Returns true.
    pub fn write_items(&self, ctx: &Ctx, items: &[PasteboardItem]) -> bool {
        with(&self.name, |s| s.items.extend(items.iter().cloned()));
        self.push(ctx);
        true
    }
    /// `pasteboardItems`.
    pub fn items(&self) -> Vec<PasteboardItem> {
        with(&self.name, |s| s.items.clone())
    }
    /// `readObjects(forClasses:options:)` by type: the items with a type conforming to one of `types`.
    pub fn read_items(&self, types: &[&str]) -> Vec<PasteboardItem> {
        self.items().into_iter().filter(|i| i.conforms_to_any(types)).collect()
    }
    /// `availableType(from:)`: the first of `types` that some type on the pasteboard conforms to.
    pub fn available_type(&self, types: &[&str]) -> Option<String> {
        let have = self.types();
        types.iter().find(|want| have.iter().any(|h| uttype::conforms(h, want))).map(|t| t.to_string())
    }
    /// `canReadItem(withDataConformingToTypes:)`.
    pub fn can_read_item(&self, types: &[&str]) -> bool {
        self.items().iter().any(|i| i.conforms_to_any(types))
    }
    /// `setData(_:forType:)`: replaces the data of `uti` on the first item.
    pub fn set_data(&self, ctx: &Ctx, bytes: &[u8], uti: &str) {
        with(&self.name, |s| {
            if s.items.is_empty() {
                s.items.push(PasteboardItem::new());
            }
            s.items[0].set_data(bytes, uti);
        });
        self.push(ctx);
    }
    /// `setString(_:forType:)` as `public.utf8-plain-text`.
    pub fn set_string(&self, ctx: &Ctx, text: &str) {
        self.set_data(ctx, text.as_bytes(), STRING);
    }
    /// `data(forType:)`: the first item that has `uti`.
    pub fn data(&self, uti: &str) -> Option<Vec<u8>> {
        with(&self.name, |s| s.items.iter().find_map(|i| i.data(uti).map(|d| d.to_vec())))
    }
    /// `string(forType: .string)`; the general board falls back to the plain clipboard text.
    pub fn string(&self) -> Option<String> {
        let own = self.data(STRING).and_then(|d| String::from_utf8(d).ok());
        if self.is_general() {
            own.or_else(crate::kit::clipboard::text)
        } else {
            own
        }
    }
    /// `types`: the UTType ids on the pasteboard, across items, without duplicates.
    pub fn types(&self) -> Vec<String> {
        with(&self.name, |s| {
            let mut out: Vec<String> = Vec::new();
            for t in s.items.iter().flat_map(|i| i.types()) {
                if !out.contains(&t) {
                    out.push(t);
                }
            }
            out
        })
    }
    /// `changeCount`.
    pub fn change_count(&self) -> u32 {
        with(&self.name, |s| s.change_count)
    }
    /// `writeObjects` for file URLs (`public.file-url`, newline-separated `file://` URLs). Clears first and returns the
    /// new change count.
    pub fn write_objects(&self, ctx: &Ctx, urls: &[&str]) -> u32 {
        let n = self.clear_contents(ctx);
        self.set_data(ctx, urls.join("\n").as_bytes(), FILE_URL);
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::Configure;

    fn ctx() -> Ctx {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(bifrost_render::ui::Ui::new(bifrost_render::font::Font::parse(data).unwrap(), 1.0), cfg)
    }

    fn item(uti: &str, text: &str) -> PasteboardItem {
        let mut i = PasteboardItem::new();
        i.set_string(text, uti);
        i
    }

    #[test]
    fn two_items_with_different_types_round_trip() {
        let ctx = ctx();
        let pb = NativePasteboard::named("t-two");
        pb.clear_contents(&ctx);
        pb.write_items(&ctx, &[item(STRING, "one"), item(HTML, "<i>two</i>")]);
        let items = pb.items();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].string(HTML).as_deref(), Some("<i>two</i>"));
        assert_eq!(pb.types(), [STRING, HTML]);
        assert_eq!(pb.read_items(&[HTML]).len(), 1);
    }

    #[test]
    fn available_type_picks_the_first_conforming_type() {
        let ctx = ctx();
        let pb = NativePasteboard::named("t-avail");
        pb.write_items(&ctx, &[item(STRING, "x")]);
        assert_eq!(pb.available_type(&[PNG, "public.text", STRING]).as_deref(), Some("public.text"));
        assert_eq!(pb.available_type(&[PNG]), None);
        assert!(pb.can_read_item(&["public.text"]));
        assert!(!pb.can_read_item(&[PNG]));
    }

    #[test]
    fn named_pasteboards_are_isolated_and_count_changes() {
        let ctx = ctx();
        let (a, b) = (NativePasteboard::named("t-a"), NativePasteboard::drag());
        a.set_string(&ctx, "in a");
        assert_eq!(b.string(), None);
        let n = a.change_count();
        assert_eq!(a.clear_contents(&ctx), n + 1);
        assert_eq!(a.clear_contents(&ctx), n + 2);
        assert_eq!(b.change_count(), 0);
        assert!(ctx.take_sent().is_empty(), "named boards stay off the wire");
    }

    #[test]
    fn legacy_set_string_is_one_item_and_the_wire_sends_the_first_item() {
        let ctx = ctx();
        let pb = NativePasteboard::general();
        pb.clear_contents(&ctx);
        pb.set_string(&ctx, "hi");
        assert_eq!(pb.items().len(), 1);
        pb.write_items(&ctx, &[item(HTML, "<b>")]);
        let sent = ctx.take_sent();
        assert!(matches!(sent.last(), Some(ToShell::PasteboardSet { items }) if items.len() == 1 && items[0].0 == STRING));
        assert_eq!(pb.items().len(), 2);
    }
}
