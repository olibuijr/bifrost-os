//! Menu-bar extras (`NativeStatusBar` and `NativeStatusItem`): an app puts a title or symbol in the menu bar's extras area, with
//! an `NativeMenu` that opens under it. The shell draws both; a chosen entry arrives in `App::status_item_action`.
//!
//! ```ignore
//! let item = StatusBar::system().status_item(StatusItemLength::Variable);
//! item.set_title(&ctx, "42%");
//! item.set_menu(&ctx, &[MenuItem::new(1, "Open"), MenuItem::separator(), MenuItem::new(2, "Quit")]);
//! ```
//!
//! Needs no manifest permission. An app may keep up to [`MAX_ITEMS`] items; the shell ignores more. The items go when the
//! app quits.

pub use crate::proto::extras::{MAX_STATUS_ITEMS_PER_APP as MAX_ITEMS, MAX_STATUS_MENU as MAX_MENU_ITEMS};
use crate::proto::StatusMenuItem;
use crate::proto::ToShell;
use crate::Ctx;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(1);

/// `NativeStatusItem.squareLength` / `variableLength`. The shell sizes every item to its content, so `Square` is accepted
/// and drawn like `Variable` (recorded gap in docs/bifrost-ui.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusItemLength {
    Variable,
    Square,
}

/// `NativeMenuItem` of a status item's menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem(StatusMenuItem);

impl MenuItem {
    /// `NativeMenuItem(title:action:keyEquivalent:)`; `id` comes back in `App::status_item_action`.
    pub fn new(id: u32, title: &str) -> Self {
        MenuItem(StatusMenuItem { id, title: title.into(), enabled: true, checked: false, separator: false })
    }
    /// `NativeMenuItem.separator()`.
    pub fn separator() -> Self {
        MenuItem(StatusMenuItem { separator: true, ..Default::default() })
    }
    /// `isEnabled`.
    pub fn enabled(mut self, v: bool) -> Self {
        self.0.enabled = v;
        self
    }
    /// `state == .on` (a check mark).
    pub fn checked(mut self, v: bool) -> Self {
        self.0.checked = v;
        self
    }
}

/// `NativeStatusBar`.
#[derive(Clone, Copy, Debug)]
pub struct StatusBar(());

impl StatusBar {
    /// `NativeStatusBar.system`.
    pub fn system() -> Self {
        StatusBar(())
    }
    /// `statusItem(withLength:)`. Nothing shows until a title or symbol is set.
    pub fn status_item(&self, _length: StatusItemLength) -> StatusItem {
        StatusItem { id: NEXT.fetch_add(1, Ordering::Relaxed), state: RefCell::new(State { visible: true, ..Default::default() }) }
    }
}

#[derive(Default)]
struct State {
    visible: bool,
    title: String,
    symbol: String,
    menu: Vec<StatusMenuItem>,
}

/// `NativeStatusItem`. Every setter sends the whole item to the shell.
pub struct StatusItem {
    id: u32,
    state: RefCell<State>,
}

impl StatusItem {
    /// The id `App::status_item_action` reports.
    pub fn id(&self) -> u32 {
        self.id
    }
    /// `button.title` (at most 64 characters).
    pub fn set_title(&self, ctx: &Ctx, title: &str) {
        self.state.borrow_mut().title = title.into();
        self.push(ctx);
    }
    /// `button.image` as a symbol name of the shell's symbol set (empty clears it).
    pub fn set_symbol(&self, ctx: &Ctx, symbol: &str) {
        self.state.borrow_mut().symbol = symbol.into();
        self.push(ctx);
    }
    /// `menu`: replaces the menu (at most [`MAX_MENU_ITEMS`] entries).
    pub fn set_menu(&self, ctx: &Ctx, menu: &[MenuItem]) {
        self.state.borrow_mut().menu = menu.iter().map(|m| m.0.clone()).collect();
        self.push(ctx);
    }
    /// `isVisible`.
    pub fn set_visible(&self, ctx: &Ctx, visible: bool) {
        self.state.borrow_mut().visible = visible;
        self.push(ctx);
    }
    /// `NativeStatusBar.removeStatusItem`.
    pub fn remove(self, ctx: &Ctx) {
        ctx.send_to_shell(ToShell::StatusItemRemove { item: self.id });
    }
    fn push(&self, ctx: &Ctx) {
        let s = self.state.borrow();
        ctx.send_to_shell(ToShell::StatusItemSet { item: self.id, visible: s.visible, title: s.title.clone(), symbol: s.symbol.clone(), menu: s.menu.clone() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setters_send_the_whole_item() {
        let ctx = crate::runtime::test_ctx();
        let item = StatusBar::system().status_item(StatusItemLength::Variable);
        item.set_title(&ctx, "42%");
        item.set_symbol(&ctx, "bolt");
        item.set_menu(&ctx, &[MenuItem::new(1, "Open").checked(true), MenuItem::separator(), MenuItem::new(2, "Quit").enabled(false)]);
        item.set_visible(&ctx, false);
        let id = item.id();
        let sent = ctx.take_sent();
        assert_eq!(sent.len(), 4);
        assert_eq!(
            sent[2],
            ToShell::StatusItemSet {
                item: id,
                visible: true,
                title: "42%".into(),
                symbol: "bolt".into(),
                menu: vec![
                    StatusMenuItem { id: 1, title: "Open".into(), enabled: true, checked: true, separator: false },
                    StatusMenuItem { id: 0, title: String::new(), enabled: false, checked: false, separator: true },
                    StatusMenuItem { id: 2, title: "Quit".into(), enabled: false, checked: false, separator: false },
                ]
            }
        );
        assert!(matches!(&sent[3], ToShell::StatusItemSet { visible: false, .. }));
        item.remove(&ctx);
        assert_eq!(ctx.take_sent(), [ToShell::StatusItemRemove { item: id }]);
    }

    #[test]
    fn items_get_distinct_ids() {
        let bar = StatusBar::system();
        assert_ne!(bar.status_item(StatusItemLength::Square).id(), bar.status_item(StatusItemLength::Variable).id());
    }
}

/// The system menu extras (Search, Wi-Fi, Battery, Clock and the rest) as the shell lists them: ids, labels, default
/// visibility and clock options. Shared with the shell through `bifrost_ui::menu_extras`; an app reads it, it does not
/// draw these items (the shell owns them).
pub mod system {
    pub use bifrost_ui::menu_extras::{ClockStyle, SystemExtra};
}
