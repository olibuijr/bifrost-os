//! `NativePasteboard`: the general pasteboard with typed data. Types are UTType ids (`public.utf8-plain-text`, `public.html`,
//! `public.rtf`, `public.png`, `public.file-url`); the shell maps them to MIME types for the Wayland clipboard
//! (`springboard::shell::pasteboard::mime_for`). The plain-string API (`Ctx::pasteboard_string`) keeps working.
//!
//! parity gap: the shell publishes only the text-like type to the Wayland selection for now; other types stay
//! inside BifrOSt apps until the presenter offers multi-MIME data sources.

use crate::proto::ToShell;
use crate::Ctx;
use std::cell::RefCell;

pub const STRING: &str = "public.utf8-plain-text";
pub const HTML: &str = "public.html";
pub const RTF: &str = "public.rtf";
pub const PNG: &str = "public.png";
pub const FILE_URL: &str = "public.file-url";

#[derive(Default)]
struct State {
    change_count: u32,
    items: Vec<(String, Vec<u8>)>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Applies the shell's `ToApp::Pasteboard`.
pub(crate) fn mirror(change_count: u32, items: Vec<(String, Vec<u8>)>) {
    STATE.with(|s| *s.borrow_mut() = State { change_count, items });
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
    /// `clearContents`: empties the pasteboard and returns the new change count.
    pub fn clear_contents(&self, ctx: &Ctx) -> u32 {
        let n = STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.items.clear();
            s.change_count = s.change_count.wrapping_add(1);
            s.change_count
        });
        Self::push(ctx);
        n
    }
    /// `setData(_:forType:)`: replaces the data of `uti`.
    pub fn set_data(&self, ctx: &Ctx, bytes: &[u8], uti: &str) {
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.items.retain(|(t, _)| t != uti);
            s.items.push((uti.to_string(), bytes.to_vec()));
        });
        Self::push(ctx);
    }
    /// `setString(_:forType:)` as `public.utf8-plain-text`.
    pub fn set_string(&self, ctx: &Ctx, text: &str) {
        self.set_data(ctx, text.as_bytes(), STRING);
    }
    /// `data(forType:)`.
    pub fn data(&self, uti: &str) -> Option<Vec<u8>> {
        STATE.with(|s| s.borrow().items.iter().find(|(t, _)| t == uti).map(|(_, d)| d.clone()))
    }
    /// `string(forType: .string)`; falls back to the plain clipboard text.
    pub fn string(&self) -> Option<String> {
        self.data(STRING).and_then(|d| String::from_utf8(d).ok()).or_else(crate::kit::clipboard::text)
    }
    /// `types`: the UTType ids on the pasteboard.
    pub fn types(&self) -> Vec<String> {
        STATE.with(|s| s.borrow().items.iter().map(|(t, _)| t.clone()).collect())
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
}
