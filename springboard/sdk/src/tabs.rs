//! Tabs and bar appearance for apps (`NativeTabBarController`, `NativeTab`, `NativeTabGroup`, `NativeSearchTab`, `NativeAppearance`).
//!
//! An app builds a [`TabBarController`] from [`Tab`]s: screens, groups of screens, and a search tab. The shell's
//! window hands the controller its bounds; [`TabBarController::layout`] gives the content area and either the bar
//! along the bottom (compact width) or the sidebar along the leading edge (wide width). Pointer taps go to
//! [`TabBarController::tap`], and keyboard navigation to [`TabBarController::step`] and
//! [`TabBarController::select_index`]. Bars draw with the process-wide standard appearance, read with
//! [`read_standard`], or set with [`update_standard`]; a bar with its own [`NavigationBarAppearance`] or
//! [`TabBarAppearance`] draws with that over the standard one.
//!
//! The drawing of the bar and the sidebar is done by the renderer that owns the window; this module is the model.

pub use bifrost_ui::bar_appearance::{
    read_standard, standard, update_standard, Background, BarAppearance, ItemState, NavigationBarAppearance,
    Standard, TabBarAppearance, TextAttributes,
};
pub use bifrost_ui::tab_controller::{
    Presentation, Tab, TabBarController, TabKind, TabLayout, Tap, SIDEBAR_MIN_WIDTH_PT, SIDEBAR_WIDTH_PT, TAB_BAR_PT,
};
