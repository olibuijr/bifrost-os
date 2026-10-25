//! Every chrome surface id of the desktop shell in one place. A chrome surface is an overlay (layer-shell) surface the
//! shell cuts from its own frame (`Shell::desktop_chrome_frames`) and the window server stacks by its rank
//! (`bare::wayland::chrome_z`); the id names the surface across frames, so two features that share an id fight over
//! one surface (Look Up once took 43 while a deep submenu could count up into the accessibility ids). Single surfaces
//! have one id each; the cascading submenus have a reserved range, one id per level. A new surface takes a free id
//! here and joins `SINGLES`: the compile-time check below refuses a duplicate or an id inside the submenu range.
#![cfg_attr(not(feature = "desktop-wayland"), allow(dead_code))]

/// The menu bar.
pub const BAR: u32 = 0;
/// The open menu (the first level; its submenus are `FLYOUT_FIRST` onwards).
pub const POPUP: u32 = 1;
/// The Dock.
pub const DOCK: u32 = 2;
/// Search.
pub const SEARCH: u32 = 3;
/// Control Center's panel.
pub const CONTROL_CENTER: u32 = 4;
/// Notification Center's panel.
pub const NOTIFICATION_CENTER: u32 = 5;
/// The assistant panel.
pub const ASSISTANT: u32 = 6;
/// Quick Look.
pub const QUICK_LOOK: u32 = 7;
/// The Apps grid.
pub const APP_GRID: u32 = 8;
/// Overview.
pub const OVERVIEW: u32 = 9;
/// A desktop switch's sweep.
pub const SWEEP: u32 = 10;
/// The screenshot session's capture layer.
pub const CAPTURE: u32 = 11;
/// The screenshot thumbnail.
pub const SHOT: u32 = 12;
/// The Cmd+Tab strip.
pub const SWITCHER: u32 = 13;
/// The system menu's card (Force Quit, About, Log Out).
pub const UTILITY: u32 = 14;
/// The system sheets and alerts (the whole screen).
pub const MODAL: u32 = 15;
/// A tablet's on-screen keyboard.
pub const KEYBOARD: u32 = 16;
/// The first cascading submenu; a submenu of a submenu is the next id, up to `FLYOUT_SLOTS` levels.
pub const FLYOUT_FIRST: u32 = 17;
/// How many submenu levels have a surface of their own (ids `FLYOUT_FIRST..FLYOUT_END`). Deeper levels are not
/// exported as surfaces.
pub const FLYOUT_SLOTS: u32 = 23;
/// The first id after the submenu range.
pub const FLYOUT_END: u32 = FLYOUT_FIRST + FLYOUT_SLOTS;
/// The notification banner.
pub const BANNER: u32 = 40;
/// The accessibility announcement caption.
pub const CAPTION: u32 = 41;
/// The screen reader's focus ring.
pub const FOCUS_RING: u32 = 42;
/// The Look Up popover.
pub const LOOK_UP: u32 = 43;
/// The title bar or toolbar a full-screen window reveals under the menu bar while the bar is shown over it
/// (`Shell::desk_reveal`).
pub const REVEAL: u32 = 44;

/// Every single-surface id; none may repeat or fall in the submenu range.
pub const SINGLES: [u32; 22] = [
    BAR,
    POPUP,
    DOCK,
    SEARCH,
    CONTROL_CENTER,
    NOTIFICATION_CENTER,
    ASSISTANT,
    QUICK_LOOK,
    APP_GRID,
    OVERVIEW,
    SWEEP,
    CAPTURE,
    SHOT,
    SWITCHER,
    UTILITY,
    MODAL,
    KEYBOARD,
    BANNER,
    CAPTION,
    FOCUS_RING,
    LOOK_UP,
    REVEAL,
];

/// Whether `id` is one of the cascading submenus' surfaces.
pub const fn is_flyout(id: u32) -> bool {
    id >= FLYOUT_FIRST && id < FLYOUT_END
}

/// True when the single ids are distinct and none is in the submenu range.
const fn ids_are_disjoint() -> bool {
    let mut i = 0;
    while i < SINGLES.len() {
        if is_flyout(SINGLES[i]) {
            return false;
        }
        let mut j = i + 1;
        while j < SINGLES.len() {
            if SINGLES[i] == SINGLES[j] {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

const _: () = assert!(ids_are_disjoint(), "two chrome surfaces share an id, or one is inside the submenu range");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_ids_are_distinct_and_clear_of_the_submenu_range() {
        assert!(ids_are_disjoint());
        for id in SINGLES {
            assert!(!is_flyout(id), "{id} is in the submenu range");
        }
        assert!(is_flyout(FLYOUT_FIRST) && is_flyout(FLYOUT_END - 1) && !is_flyout(FLYOUT_END));
        // the deepest submenu stops short of the banner and the accessibility surfaces
        assert!(FLYOUT_END <= BANNER && FLYOUT_END <= CAPTION && FLYOUT_END <= FOCUS_RING && FLYOUT_END <= LOOK_UP);
        let mut sorted = SINGLES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), SINGLES.len());
    }
}
