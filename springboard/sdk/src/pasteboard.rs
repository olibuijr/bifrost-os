//! `NativePasteboard`: pasteboards with typed items. Types are UTType ids (`public.utf8-plain-text`, `public.html`,
//! `public.rtf`, `public.png`, `public.file-url`); the shell maps them to MIME types for the Wayland clipboard
//! (`springboard::shell::pasteboard::mime_for`). The plain-string API (`Ctx::pasteboard_string`) keeps working.
//!
//! A board holds a list of [`PasteboardItem`]s; the plain `set_*`/`data` calls work on the first item. Boards other
//! than the general one (`named`, `drag`, `find`) live inside the app.
//!
//! Several types of one copy go out in a single write: [`NativePasteboard::write_items`] (`writeObjects` with
//! `PasteboardItem`s), [`NativePasteboard::write_transferable`] (a `Transferable` model, one change step) and
//! [`NativePasteboard::declare_types`] (`declareTypes(_:owner:)`, the types promised and produced lazily by a
//! [`PasteboardTypeOwner`]). Reading back: [`NativePasteboard::items`] and [`NativePasteboard::read_objects`]
//! (`readObjects(forClasses:options:)`).
//!
//! parity gap: the shell wire carries a flat (type, data) list, so the general board sends the representations of its
//! items merged (the first item holding a type keeps it, at most 16 items), and the shell publishes only the text-like
//! type to the Wayland selection for now; other types stay inside BifrOSt apps until the presenter offers multi-MIME
//! data sources. Promised data that nobody has read yet is not sent, and another app's write ends this process's
//! promises.

use crate::pasteboard_item::{Found, PasteboardItem};
use crate::pasteboard_promise::{OwnerProvider, PasteboardTypeOwner};
use crate::proto::ToShell;
use crate::transfer::{self, Representations, Transferable};
use crate::Ctx;
use bifrost_ui::uttype;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const STRING: &str = "public.utf8-plain-text";
pub const HTML: &str = "public.html";
pub const RTF: &str = "public.rtf";
pub const PNG: &str = "public.png";
pub const FILE_URL: &str = "public.file-url";

/// Names of the system pasteboards.
pub const GENERAL: &str = "bifrost general";
pub const DRAG: &str = "bifrost drag";
pub const FIND: &str = "bifrost find";

/// The shell's wire limit on items per write.
const MAX_ITEMS: usize = 16;

#[derive(Default)]
struct State {
    change_count: u32,
    items: Vec<PasteboardItem>,
    /// The change count `declare_types` made, while its promises are still this board's contents.
    declared_at: Option<u32>,
}

thread_local! {
    static BOARDS: RefCell<HashMap<String, State>> = RefCell::new(HashMap::new());
}

fn with<R>(name: &str, f: impl FnOnce(&mut State) -> R) -> R {
    BOARDS.with(|b| f(b.borrow_mut().entry(name.to_string()).or_default()))
}

/// Providers are told they are finished outside the board's borrow, so they may use the pasteboard themselves.
fn finish(items: Vec<PasteboardItem>) {
    for item in &items {
        item.finish_providers();
    }
}

/// Applies the shell's `ToApp::Pasteboard` (the general board; the flat list becomes one item). The echo of this
/// process's own `declare_types` keeps its promises; any other change ends them.
pub(crate) fn mirror(change_count: u32, items: Vec<(String, Vec<u8>)>) {
    let old = with(GENERAL, |s| {
        if s.declared_at == Some(change_count) {
            return Vec::new();
        }
        s.declared_at = None;
        s.change_count = change_count;
        let fresh = if items.is_empty() { Vec::new() } else { vec![PasteboardItem::from_representations(items)] };
        std::mem::replace(&mut s.items, fresh)
    });
    finish(old);
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
    /// Sends the general board to the shell: the data that exists, items merged, one write.
    fn push(&self, ctx: &Ctx) {
        if !self.is_general() {
            return;
        }
        let items = with(GENERAL, |s| {
            let mut flat: Vec<(String, Vec<u8>)> = Vec::new();
            for (t, d) in s.items.iter().take(MAX_ITEMS).flat_map(|i| i.available_representations()) {
                if !flat.iter().any(|(have, _)| *have == t) {
                    flat.push((t, d));
                }
            }
            flat
        });
        ctx.send_to_shell(ToShell::PasteboardSet { items });
    }
    /// Empties the board and steps its change count; the dropped items are returned for finishing.
    fn empty(&self) -> (u32, Vec<PasteboardItem>) {
        with(&self.name, |s| {
            s.declared_at = None;
            s.change_count = s.change_count.wrapping_add(1);
            (s.change_count, std::mem::take(&mut s.items))
        })
    }
    /// `clearContents`: empties the pasteboard and returns the new change count.
    pub fn clear_contents(&self, ctx: &Ctx) -> u32 {
        let (n, old) = self.empty();
        finish(old);
        self.push(ctx);
        n
    }
    /// `writeObjects(_:)` for items: appends them. Returns true.
    pub fn write_items(&self, ctx: &Ctx, items: &[PasteboardItem]) -> bool {
        with(&self.name, |s| s.items.extend(items.iter().cloned()));
        self.push(ctx);
        true
    }
    /// `clearContents` then `writeObjects(_:)` as one step: one change count, one message to the shell. Returns the
    /// new change count.
    pub fn replace_items(&self, ctx: &Ctx, items: Vec<PasteboardItem>) -> u32 {
        let (n, old) = self.empty();
        with(&self.name, |s| s.items = items);
        finish(old);
        self.push(ctx);
        n
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
        with(&self.name, |s| s.items.iter().any(|i| i.conforms_to_any(types)))
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
    /// `data(forType:)`: the first item that has `uti`. A promised type is resolved on first read (the provider is
    /// asked once).
    pub fn data(&self, uti: &str) -> Option<Vec<u8>> {
        let found = with(&self.name, |s| s.items.iter().find_map(|i| i.lookup(uti)));
        match found? {
            Found::Data(d) => Some(d),
            Found::Promise(p) => p.resolve(uti),
        }
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
    /// `types`: the UTType ids on the pasteboard, across items, promised ones included, without duplicates.
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

    /// `writeObjects([item])` for a `Transferable`: every representation another app may see, in one write. Returns
    /// the new change count.
    pub fn write_transferable<T: Transferable>(&self, ctx: &Ctx, value: &T) -> u32 {
        self.write_representations(ctx, &T::transfer_representations(), value)
    }

    /// As [`Self::write_transferable`] for representations built on the spot.
    pub fn write_representations<T: 'static>(&self, ctx: &Ctx, reps: &Representations<T>, value: &T) -> u32 {
        self.replace_items(ctx, vec![PasteboardItem::from_item(&transfer::pasteboard_item(reps, value))])
    }

    /// `declareTypes(_:owner:)`: clears the board and announces `types`; `owner` supplies each one the first time it is
    /// read and hears `changed_owner` when the board is cleared or another app writes it. The shell is sent only data
    /// that exists (promised bytes stay here until read); returns the new change count.
    pub fn declare_types(&self, ctx: &Ctx, types: &[&str], owner: Rc<dyn PasteboardTypeOwner>) -> u32 {
        let (n, old) = self.empty();
        let mut item = PasteboardItem::new();
        item.set_data_provider(Rc::new(OwnerProvider(owner)), types);
        with(&self.name, |s| {
            s.items = vec![item];
            s.declared_at = Some(n);
        });
        finish(old);
        self.push(ctx);
        n
    }

    /// `readObjects(forClasses:options:)` for a `Transferable`: the values the board holds, one per item, read through
    /// the best type the model can import.
    pub fn read_objects<T: Transferable>(&self) -> Vec<T> {
        let reps = T::transfer_representations();
        self.items().iter().filter_map(|i| transfer::from_pasteboard_item(&reps, &i.to_item())).collect()
    }

    /// `canReadObject(forClasses:options:)`.
    pub fn can_read_object<T: Transferable>(&self) -> bool {
        let types = self.types();
        let offered: Vec<&str> = types.iter().map(String::as_str).collect();
        T::transfer_representations().best_import(&offered).is_some()
    }
}

/// The standard edit commands a text control offers in its menu and on its key equivalents: cut, copy, paste, select all
/// and delete. Each is enabled or disabled from the facts in [`EditState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCommand {
    Cut,
    Copy,
    Paste,
    SelectAll,
    Delete,
}

/// The facts an edit command depends on: whether the text holds any text, whether something is selected, and whether the
/// general board holds text that paste can take.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditState {
    pub has_text: bool,
    pub has_selection: bool,
    pub board_has_text: bool,
}

impl EditCommand {
    /// Whether the command can run now. Cut, copy and delete need a selection, paste needs text on the board, and
    /// select all needs text to select.
    pub fn enabled(self, state: EditState) -> bool {
        match self {
            EditCommand::Cut | EditCommand::Copy | EditCommand::Delete => state.has_selection,
            EditCommand::Paste => state.board_has_text,
            EditCommand::SelectAll => state.has_text,
        }
    }
}

/// Whether a board's types include plain text, which is what a text control pastes.
pub fn offers_text(types: &[String]) -> bool {
    types.iter().any(|t| t == STRING)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pasteboard_promise::PasteboardItemDataProvider;
    use crate::proto::Configure;
    use crate::transfer::Visibility;
    use std::cell::Cell;

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
    fn the_wire_carries_the_items_merged_first_holder_of_a_type_wins() {
        let ctx = ctx();
        let pb = NativePasteboard::general();
        pb.clear_contents(&ctx);
        pb.set_string(&ctx, "hi");
        assert_eq!(pb.items().len(), 1);
        pb.write_items(&ctx, &[item(HTML, "<b>"), item(STRING, "later")]);
        let sent = ctx.take_sent();
        assert!(matches!(sent.last(), Some(ToShell::PasteboardSet { items })
            if items.len() == 2 && items[0] == (STRING.to_string(), b"hi".to_vec()) && items[1].0 == HTML));
        assert_eq!(pb.items().len(), 3);
    }

    #[derive(Debug, PartialEq)]
    struct Note(String);

    fn text(n: &Note) -> Vec<u8> {
        n.0.as_bytes().to_vec()
    }
    fn read(b: &[u8]) -> Option<Note> {
        String::from_utf8(b.to_vec()).ok().map(Note)
    }
    fn html(n: &Note) -> Vec<u8> {
        format!("<p>{}</p>", n.0).into_bytes()
    }

    impl Transferable for Note {
        fn transfer_representations() -> Representations<Note> {
            Representations::new()
                .data(STRING, Visibility::All, text, read)
                .proxy(HTML, Visibility::All, html)
                .proxy("is.bifrost.note", Visibility::OwnProcess, text)
        }
    }

    #[test]
    fn a_transferable_goes_out_as_all_its_types_in_one_write_and_change_count_step() {
        let ctx = ctx();
        let pb = NativePasteboard::general();
        let before = pb.change_count();
        let n = pb.write_transferable(&ctx, &Note("hi".into()));
        assert_eq!(n, before.wrapping_add(1));
        assert_eq!(pb.change_count(), n);
        assert_eq!(pb.types(), [STRING, HTML], "own-process types stay home");
        let sent = ctx.take_sent();
        assert_eq!(sent.len(), 1, "one message to the shell");
        assert!(matches!(&sent[0], ToShell::PasteboardSet { items } if items.len() == 2));
    }

    #[test]
    fn the_board_reads_back_as_an_item_and_as_the_model() {
        let ctx = ctx();
        let pb = NativePasteboard::general();
        pb.write_transferable(&ctx, &Note("hi".into()));
        let items = pb.items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].data(HTML), Some(b"<p>hi</p>".to_vec()));
        assert_eq!(pb.read_objects::<Note>(), vec![Note("hi".into())]);
        assert!(pb.can_read_object::<Note>());
        pb.clear_contents(&ctx);
        assert!(pb.items().is_empty());
        assert!(!pb.can_read_object::<Note>());
    }

    struct Owner {
        asked: Cell<u32>,
        changed: Cell<u32>,
    }

    impl Owner {
        fn new() -> Rc<Owner> {
            Rc::new(Owner { asked: Cell::new(0), changed: Cell::new(0) })
        }
    }

    impl PasteboardTypeOwner for Owner {
        fn provide_data(&self, uti: &str) -> Option<Vec<u8>> {
            self.asked.set(self.asked.get() + 1);
            Some(format!("lazy {uti}").into_bytes())
        }
        fn changed_owner(&self) {
            self.changed.set(self.changed.get() + 1);
        }
    }

    #[test]
    fn declared_types_are_listed_at_once_and_provided_lazily_once() {
        let ctx = ctx();
        let pb = NativePasteboard::general();
        let owner = Owner::new();
        let before = pb.change_count();
        assert_eq!(pb.declare_types(&ctx, &[STRING, PNG], owner.clone()), before.wrapping_add(1));
        assert_eq!(pb.types(), [STRING, PNG]);
        assert_eq!(owner.asked.get(), 0, "nothing is produced until read");
        assert!(matches!(ctx.take_sent().last(), Some(ToShell::PasteboardSet { items }) if items.is_empty()), "promised bytes are not sent");
        assert_eq!(pb.data(PNG), Some(b"lazy public.png".to_vec()));
        assert_eq!(pb.data(PNG), Some(b"lazy public.png".to_vec()));
        assert_eq!(owner.asked.get(), 1, "resolved once");
    }

    #[test]
    fn the_owner_hears_of_a_clear_once() {
        let ctx = ctx();
        let pb = NativePasteboard::named("t-owner");
        let owner = Owner::new();
        pb.declare_types(&ctx, &[HTML], owner.clone());
        assert_eq!(pb.types(), [HTML]);
        pb.clear_contents(&ctx);
        pb.clear_contents(&ctx);
        assert_eq!(owner.changed.get(), 1);
        assert!(pb.types().is_empty());
    }

    #[test]
    fn another_apps_write_ends_the_promises() {
        let ctx = ctx();
        let pb = NativePasteboard::general();
        let owner = Owner::new();
        let n = pb.declare_types(&ctx, &[STRING], owner.clone());
        mirror(n, vec![(PNG.into(), vec![2])]);
        assert_eq!(pb.types(), [STRING], "the echo of the board's own declare keeps the promise");
        mirror(n + 5, vec![(PNG.into(), vec![1])]);
        assert_eq!(pb.types(), [PNG]);
        assert_eq!(pb.data(STRING), None);
        assert_eq!(owner.changed.get(), 1);
    }

    struct Recording(Cell<u32>);

    impl PasteboardItemDataProvider for Recording {
        fn provide_data(&self, _uti: &str) -> Option<Vec<u8>> {
            Some(b"x".to_vec())
        }
        fn finished_with_data_provider(&self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn clearing_the_board_finishes_an_items_provider_once() {
        let ctx = ctx();
        let pb = NativePasteboard::named("t-finish");
        let provider = Rc::new(Recording(Cell::new(0)));
        let mut promised = PasteboardItem::new();
        promised.set_data_provider(provider.clone(), &[PNG]);
        pb.write_items(&ctx, &[promised]);
        assert_eq!(pb.clear_contents(&ctx), 1);
        pb.clear_contents(&ctx);
        assert_eq!(provider.0.get(), 1);
    }
}
