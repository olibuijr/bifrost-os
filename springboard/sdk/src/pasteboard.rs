//! `NativePasteboard`: the general pasteboard with typed data. Types are UTType ids (`public.utf8-plain-text`, `public.html`,
//! `public.rtf`, `public.png`, `public.file-url`); the shell maps them to MIME types for the Wayland clipboard
//! (`springboard::shell::pasteboard::mime_for`). The plain-string API (`Ctx::pasteboard_string`) keeps working.
//!
//! Several types of one copy go out in a single write: [`NativePasteboard::write_items`] (`writeObjects` with
//! `NSPasteboardItem`s), [`NativePasteboard::write_transferable`] (a `Transferable` model), and
//! [`NativePasteboard::declare_types`] (`declareTypes(_:owner:)` with a lazy [`DataProvider`]). Reading back:
//! [`NativePasteboard::pasteboard_items`] and [`NativePasteboard::read_objects`] (`readObjects(forClasses:options:)`).
//!
//! parity gap: the shell publishes only the text-like type to the Wayland selection for now; other types stay
//! inside BifrOSt apps until the presenter offers multi-MIME data sources (shell/pasteboard.rs follow-up). The wire is
//! flat, one representation per type, so the items of one write share it: the first item holding a type keeps it, and
//! `pasteboard_items` returns the board as one item.

use crate::pasteboard_promise::{DataProvider, Promise};
use crate::pasteboard_types::Item;
use crate::proto::ToShell;
use crate::transfer::{self, Representations, Transferable};
use crate::Ctx;
use std::cell::RefCell;
use std::rc::Rc;

pub const STRING: &str = "public.utf8-plain-text";
pub const HTML: &str = "public.html";
pub const RTF: &str = "public.rtf";
pub const PNG: &str = "public.png";
pub const FILE_URL: &str = "public.file-url";

/// The shell's wire limit on items per write.
const MAX_ITEMS: usize = 16;

#[derive(Default)]
struct State {
    change_count: u32,
    items: Vec<(String, Vec<u8>)>,
    /// Promised types (`declareTypes` with an owner) and the change count they were declared at.
    promises: Vec<(String, Promise)>,
    declared_at: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Applies the shell's `ToApp::Pasteboard`. Another app's write ends this process's promises.
pub(crate) fn mirror(change_count: u32, items: Vec<(String, Vec<u8>)>) {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        if change_count != s.declared_at {
            s.promises.clear();
        }
        s.change_count = change_count;
        s.items = items;
    });
}

/// `NativePasteboard`. Get it with [`NativePasteboard::general`].
#[derive(Clone, Copy, Debug)]
pub struct NativePasteboard;

impl NativePasteboard {
    pub fn general() -> Self {
        NativePasteboard
    }
    fn push(ctx: &Ctx) {
        let items = STATE.with(|s| s.borrow().items.clone());
        ctx.send_to_shell(ToShell::PasteboardSet { items });
    }
    fn clear_state() -> u32 {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.items.clear();
            s.promises.clear();
            s.change_count = s.change_count.wrapping_add(1);
            s.change_count
        })
    }
    /// `clearContents`: empties the pasteboard and returns the new change count.
    pub fn clear_contents(&self, ctx: &Ctx) -> u32 {
        let n = Self::clear_state();
        Self::push(ctx);
        n
    }
    /// `setData(_:forType:)`: replaces the data of `uti`.
    pub fn set_data(&self, ctx: &Ctx, bytes: &[u8], uti: &str) {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.items.retain(|(t, _)| t != uti);
            s.promises.retain(|(t, _)| t != uti);
            s.items.push((uti.to_string(), bytes.to_vec()));
        });
        Self::push(ctx);
    }
    /// `setString(_:forType:)` as `public.utf8-plain-text`.
    pub fn set_string(&self, ctx: &Ctx, text: &str) {
        self.set_data(ctx, text.as_bytes(), STRING);
    }
    /// `data(forType:)`. A promised type is resolved on first read (the owner is asked once).
    pub fn data(&self, uti: &str) -> Option<Vec<u8>> {
        let promised = STATE.with(|s| s.borrow().promises.iter().find(|(t, _)| t == uti).map(|(_, p)| p.clone()));
        if let Some(p) = promised {
            return p.resolve(uti);
        }
        STATE.with(|s| s.borrow().items.iter().find(|(t, _)| t == uti).map(|(_, d)| d.clone()))
    }
    /// `string(forType: .string)`; falls back to the plain clipboard text.
    pub fn string(&self) -> Option<String> {
        self.data(STRING).and_then(|d| String::from_utf8(d).ok()).or_else(crate::kit::clipboard::text)
    }
    /// `types`: the UTType ids on the pasteboard, promised ones included.
    pub fn types(&self) -> Vec<String> {
        STATE.with(|s| {
            let s = s.borrow();
            s.items.iter().map(|(t, _)| t.clone()).chain(s.promises.iter().map(|(t, _)| t.clone())).collect()
        })
    }
    /// `changeCount`.
    pub fn change_count(&self) -> u32 {
        STATE.with(|s| s.borrow().change_count)
    }
    /// `writeObjects` for file URLs (`public.file-url`, newline-separated `file://` URLs). Clears first and returns the
    /// new change count.
    pub fn write_objects(&self, ctx: &Ctx, urls: &[&str]) -> u32 {
        let n = self.clear_contents(ctx);
        self.set_data(ctx, urls.join("\n").as_bytes(), FILE_URL);
        n
    }

    /// `writeObjects` for pasteboard items: clears, then publishes every representation of every item in one write
    /// (one change count step, one message to the shell). Returns the new change count.
    pub fn write_items(&self, ctx: &Ctx, items: &[Item]) -> u32 {
        let n = Self::clear_state();
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            for r in items.iter().take(MAX_ITEMS).flat_map(|i| i.representations.iter()) {
                if !s.items.iter().any(|(t, _)| *t == r.uti) {
                    s.items.push((r.uti.clone(), r.data.clone()));
                }
            }
        });
        Self::push(ctx);
        n
    }

    /// `writeObjects([item])` for a `Transferable`: every representation another app may see, in one write.
    pub fn write_transferable<T: Transferable>(&self, ctx: &Ctx, value: &T) -> u32 {
        self.write_representations(ctx, &T::transfer_representations(), value)
    }

    /// As [`Self::write_transferable`] for representations built on the spot.
    pub fn write_representations<T: 'static>(&self, ctx: &Ctx, reps: &Representations<T>, value: &T) -> u32 {
        self.write_items(ctx, &[transfer::pasteboard_item(reps, value)])
    }

    /// `declareTypes(_:owner:)`: clears the board and announces `types`; `owner` supplies each one the first time it is read.
    /// The shell is sent only data that exists (promised bytes stay here until read); returns the new change count.
    pub fn declare_types(&self, ctx: &Ctx, types: &[&str], owner: Rc<dyn DataProvider>) -> u32 {
        let n = Self::clear_state();
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.declared_at = n;
            for t in types {
                s.promises.push((t.to_string(), Promise::new(Rc::clone(&owner))));
            }
        });
        Self::push(ctx);
        n
    }

    /// `pasteboardItems`: the board as one item holding every type (promised types resolved now).
    pub fn pasteboard_items(&self) -> Vec<Item> {
        let item = self.types().iter().filter_map(|t| self.data(t).map(|d| (t.clone(), d))).fold(Item::default(), |i, (t, d)| i.with(&t, &d));
        if item.representations.is_empty() {
            Vec::new()
        } else {
            vec![item]
        }
    }

    /// `readObjects(forClasses:options:)` for a `Transferable`: the values the board holds, read through the best type the
    /// model can import.
    pub fn read_objects<T: Transferable>(&self) -> Vec<T> {
        let reps = T::transfer_representations();
        self.pasteboard_items().iter().filter_map(|i| transfer::from_pasteboard_item(&reps, i)).collect()
    }

    /// `canReadObject(forClasses:options:)`.
    pub fn can_read_object<T: Transferable>(&self) -> bool {
        let types = self.types();
        let offered: Vec<&str> = types.iter().map(String::as_str).collect();
        T::transfer_representations().best_import(&offered).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::{Representations, Visibility};
    use crate::{Configure, Ui};
    use bifrost_render::font::Font;
    use std::cell::Cell;

    fn ctx() -> Ctx {
        let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
        let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
        Ctx::testing(Ui::new(Font::parse(data).unwrap(), 1.0), cfg)
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

    struct Owner(Cell<u32>);
    impl DataProvider for Owner {
        fn provide(&self, uti: &str) -> Option<Vec<u8>> {
            self.0.set(self.0.get() + 1);
            Some(format!("lazy {uti}").into_bytes())
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
    fn the_board_reads_back_as_one_item_and_as_the_model() {
        let ctx = ctx();
        let pb = NativePasteboard::general();
        pb.write_transferable(&ctx, &Note("hi".into()));
        let items = pb.pasteboard_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].data(HTML), Some(&b"<p>hi</p>"[..]));
        assert_eq!(pb.read_objects::<Note>(), vec![Note("hi".into())]);
        assert!(pb.can_read_object::<Note>());
        pb.clear_contents(&ctx);
        assert!(pb.pasteboard_items().is_empty());
        assert!(!pb.can_read_object::<Note>());
    }

    #[test]
    fn declared_types_are_listed_at_once_and_provided_lazily_once() {
        let ctx = ctx();
        let pb = NativePasteboard::general();
        let owner = Rc::new(Owner(Cell::new(0)));
        let before = pb.change_count();
        assert_eq!(pb.declare_types(&ctx, &[STRING, PNG], owner.clone()), before.wrapping_add(1));
        assert_eq!(pb.types(), [STRING, PNG]);
        assert_eq!(owner.0.get(), 0, "nothing is produced until read");
        assert_eq!(pb.data(PNG), Some(b"lazy public.png".to_vec()));
        assert_eq!(pb.data(PNG), Some(b"lazy public.png".to_vec()));
        assert_eq!(owner.0.get(), 1, "resolved once");
    }

    #[test]
    fn another_apps_write_ends_the_promises() {
        let ctx = ctx();
        let pb = NativePasteboard::general();
        pb.declare_types(&ctx, &[STRING], Rc::new(Owner(Cell::new(0))));
        mirror(pb.change_count() + 5, vec![(PNG.into(), vec![1])]);
        assert_eq!(pb.types(), [PNG]);
        assert_eq!(pb.data(STRING), None);
    }
}
