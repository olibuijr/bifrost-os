//! Bar appearance objects (`NativeBarAppearance`, `NativeNavigationBarAppearance`, `NativeTabBarAppearance`) and the appearance proxy
//! (`NativeAppearance`): a process-wide standard appearance that every bar resolves against, with an optional per-bar
//! appearance on top. A field that is `None` is not set and inherits from the next level. Colours are packed RGBA as
//! the renderer packs them (`bifrost_render::gfx::pack`).
use std::sync::{OnceLock, RwLock};

/// What a bar paints behind its items (`configureWith...Background`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    /// The system bar material. The default.
    Default,
    /// A solid colour that is not translucent.
    Opaque,
    /// Nothing behind the items: content shows through the bar.
    Transparent,
}

/// Text attributes of a bar title or an item title.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextAttributes {
    pub color: Option<u32>,
    pub size: Option<f32>,
    pub weight: Option<u16>,
}

impl TextAttributes {
    /// The fields set here win; the rest come from `base`.
    pub fn over(&self, base: &TextAttributes) -> TextAttributes {
        TextAttributes {
            color: self.color.or(base.color),
            size: self.size.or(base.size),
            weight: self.weight.or(base.weight),
        }
    }
}

/// The background, separator and tint of a bar (`NativeBarAppearance`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BarAppearance {
    pub background: Option<Background>,
    /// The colour behind the items when the background is opaque.
    pub background_color: Option<u32>,
    /// The separator line between the bar and the content.
    pub shadow_color: Option<u32>,
    /// The colour of the bar's controls (`tintColor`).
    pub tint: Option<u32>,
}

impl BarAppearance {
    /// `configureWithDefaultBackground`: the system material, with the colours cleared.
    pub fn configure_with_default_background(&mut self) {
        self.background = Some(Background::Default);
        self.background_color = None;
    }

    /// `configureWithOpaqueBackground`: a solid colour.
    pub fn configure_with_opaque_background(&mut self, color: u32) {
        self.background = Some(Background::Opaque);
        self.background_color = Some(color);
    }

    /// `configureWithTransparentBackground`: no background, and no separator.
    pub fn configure_with_transparent_background(&mut self) {
        self.background = Some(Background::Transparent);
        self.background_color = None;
        self.shadow_color = Some(0);
    }

    /// The fields set here win; the rest come from `base`.
    pub fn over(&self, base: &BarAppearance) -> BarAppearance {
        BarAppearance {
            background: self.background.or(base.background),
            background_color: self.background_color.or(base.background_color),
            shadow_color: self.shadow_color.or(base.shadow_color),
            tint: self.tint.or(base.tint),
        }
    }

    /// The background the bar paints with: the one set, else the system material.
    pub fn effective_background(&self) -> Background {
        self.background.unwrap_or(Background::Default)
    }
}

/// The appearance of a navigation bar (`NativeNavigationBarAppearance`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NavigationBarAppearance {
    pub bar: BarAppearance,
    pub title: TextAttributes,
    pub large_title: TextAttributes,
}

impl NavigationBarAppearance {
    pub fn configure_with_default_background(&mut self) {
        self.bar.configure_with_default_background();
    }

    pub fn configure_with_opaque_background(&mut self, color: u32) {
        self.bar.configure_with_opaque_background(color);
    }

    pub fn configure_with_transparent_background(&mut self) {
        self.bar.configure_with_transparent_background();
    }

    pub fn over(&self, base: &NavigationBarAppearance) -> NavigationBarAppearance {
        NavigationBarAppearance {
            bar: self.bar.over(&base.bar),
            title: self.title.over(&base.title),
            large_title: self.large_title.over(&base.large_title),
        }
    }
}

/// One state of a tab bar item: the normal or the selected look.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ItemState {
    pub icon_color: Option<u32>,
    pub title: TextAttributes,
}

impl ItemState {
    pub fn over(&self, base: &ItemState) -> ItemState {
        ItemState { icon_color: self.icon_color.or(base.icon_color), title: self.title.over(&base.title) }
    }
}

/// The appearance of a tab bar (`NativeTabBarAppearance`). The items use one layout here; the normal and selected states
/// are separate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TabBarAppearance {
    pub bar: BarAppearance,
    pub normal: ItemState,
    pub selected: ItemState,
}

impl TabBarAppearance {
    pub fn configure_with_default_background(&mut self) {
        self.bar.configure_with_default_background();
    }

    pub fn configure_with_opaque_background(&mut self, color: u32) {
        self.bar.configure_with_opaque_background(color);
    }

    pub fn configure_with_transparent_background(&mut self) {
        self.bar.configure_with_transparent_background();
    }

    /// The item state for a tab that is or is not selected.
    pub fn item_state(&self, selected: bool) -> &ItemState {
        if selected {
            &self.selected
        } else {
            &self.normal
        }
    }

    pub fn over(&self, base: &TabBarAppearance) -> TabBarAppearance {
        TabBarAppearance {
            bar: self.bar.over(&base.bar),
            normal: self.normal.over(&base.normal),
            selected: self.selected.over(&base.selected),
        }
    }
}

/// The appearance proxies: the standard navigation and tab bar appearance. A bar with its own appearance draws with
/// that over the standard one; a bar without one draws with the standard one as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Standard {
    pub navigation: NavigationBarAppearance,
    pub tab: TabBarAppearance,
}

impl Standard {
    /// The navigation bar appearance a bar draws with, given its own (if any).
    pub fn resolve_navigation(&self, local: Option<&NavigationBarAppearance>) -> NavigationBarAppearance {
        match local {
            Some(l) => l.over(&self.navigation),
            None => self.navigation,
        }
    }

    /// The tab bar appearance a bar draws with, given its own (if any).
    pub fn resolve_tab(&self, local: Option<&TabBarAppearance>) -> TabBarAppearance {
        match local {
            Some(l) => l.over(&self.tab),
            None => self.tab,
        }
    }
}

/// The process-wide standard appearance. Bars read it when they draw; apps change it with `update_standard`.
pub fn standard() -> &'static RwLock<Standard> {
    static STANDARD: OnceLock<RwLock<Standard>> = OnceLock::new();
    STANDARD.get_or_init(|| RwLock::new(Standard::default()))
}

/// Runs `f` on the standard appearance for writing (`NativeAppearance` proxy setters). A poisoned lock still updates.
pub fn update_standard(f: impl FnOnce(&mut Standard)) {
    let mut guard = standard().write().unwrap_or_else(|e| e.into_inner());
    f(&mut *guard);
}

/// A copy of the standard appearance, for drawing. A poisoned lock still reads.
pub fn read_standard() -> Standard {
    *standard().read().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: u32 = 0xFF_00_00_FF;
    const BLUE: u32 = 0xFF_FF_00_00;

    #[test]
    fn opaque_background_sets_the_colour_and_default_clears_it() {
        let mut bar = BarAppearance::default();
        assert_eq!(bar.effective_background(), Background::Default);
        bar.configure_with_opaque_background(RED);
        assert_eq!(bar.background, Some(Background::Opaque));
        assert_eq!(bar.background_color, Some(RED));
        bar.configure_with_default_background();
        assert_eq!(bar.background, Some(Background::Default));
        assert_eq!(bar.background_color, None);
    }

    #[test]
    fn transparent_background_drops_the_separator() {
        let mut bar = BarAppearance { shadow_color: Some(RED), ..Default::default() };
        bar.configure_with_transparent_background();
        assert_eq!(bar.effective_background(), Background::Transparent);
        assert_eq!(bar.shadow_color, Some(0));
    }

    #[test]
    fn set_fields_win_and_unset_fields_inherit() {
        let base = BarAppearance { background: Some(Background::Opaque), tint: Some(BLUE), ..Default::default() };
        let local = BarAppearance { tint: Some(RED), ..Default::default() };
        let merged = local.over(&base);
        assert_eq!(merged.tint, Some(RED));
        assert_eq!(merged.background, Some(Background::Opaque));
        assert_eq!(merged.shadow_color, None);
    }

    #[test]
    fn text_attributes_merge_field_by_field() {
        let base = TextAttributes { color: Some(BLUE), size: Some(17.0), weight: None };
        let local = TextAttributes { size: Some(22.0), ..Default::default() };
        let merged = local.over(&base);
        assert_eq!(merged, TextAttributes { color: Some(BLUE), size: Some(22.0), weight: None });
    }

    #[test]
    fn a_bar_without_its_own_appearance_draws_with_the_standard_one() {
        let mut s = Standard::default();
        s.navigation.title.color = Some(BLUE);
        s.tab.selected.icon_color = Some(RED);
        assert_eq!(s.resolve_navigation(None).title.color, Some(BLUE));
        assert_eq!(s.resolve_tab(None).selected.icon_color, Some(RED));
    }

    #[test]
    fn a_local_appearance_overrides_only_what_it_sets() {
        let mut s = Standard::default();
        s.navigation.title = TextAttributes { color: Some(BLUE), size: Some(17.0), weight: Some(600) };
        let mut local = NavigationBarAppearance::default();
        local.title.color = Some(RED);
        let resolved = s.resolve_navigation(Some(&local));
        assert_eq!(resolved.title, TextAttributes { color: Some(RED), size: Some(17.0), weight: Some(600) });
    }

    #[test]
    fn tab_item_state_follows_selection() {
        let mut tab = TabBarAppearance::default();
        tab.normal.icon_color = Some(BLUE);
        tab.selected.icon_color = Some(RED);
        assert_eq!(tab.item_state(false).icon_color, Some(BLUE));
        assert_eq!(tab.item_state(true).icon_color, Some(RED));
    }

    #[test]
    fn navigation_configure_helpers_touch_the_bar() {
        let mut nav = NavigationBarAppearance::default();
        nav.configure_with_opaque_background(RED);
        assert_eq!(nav.bar.background, Some(Background::Opaque));
        let mut tab = TabBarAppearance::default();
        tab.configure_with_transparent_background();
        assert_eq!(tab.bar.effective_background(), Background::Transparent);
    }

    #[test]
    fn the_process_wide_standard_is_read_back_after_an_update() {
        // The one test that touches the global; it restores the default so other tests see a clean state.
        update_standard(|s| s.navigation.title.color = Some(RED));
        assert_eq!(read_standard().navigation.title.color, Some(RED));
        update_standard(|s| *s = Standard::default());
        assert_eq!(read_standard(), Standard::default());
    }
}
