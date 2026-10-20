//! Text input for on-screen keyboards and input methods: `zwp_text_input_v3` (what applications use),
//! `zwp_input_method_v2` (what a keyboard or IME process binds) and `zwp_virtual_keyboard_v1` (key events from a
//! process that is not a device).
//!
//! Smithay relays the protocols. This module adds what the compositor decides:
//!
//! - **Who may bind the privileged globals.** An input method grabs the keyboard and sees every field's surrounding
//!   text, and a virtual keyboard types into any window, so both are offered only to clients of the compositor's own
//!   user (`Trust`). `BIFROST_WM_INPUT_METHOD=off` removes all three globals.
//! - **Candidate popups.** An input method's popup surface is placed below the text cursor, flipped above the cursor
//!   when it would leave the output (`place_popup`), drawn above the windows, hit-tested for pointer and touch, and
//!   given frame callbacks.
//! - **Show and hide.** Smithay sends the input method `activate` when the focused client enables a text input and
//!   `deactivate` when it disables it, loses keyboard focus or goes away; a keyboard process shows itself on
//!   `activate` and hides on `deactivate`. `WmState::text_input_active` is the same fact for the compositor.

use smithay::{
    delegate_input_method_manager, delegate_text_input_manager, delegate_virtual_keyboard_manager,
    desktop::{
        utils::{bbox_from_surface_tree, under_from_surface_tree},
        WindowSurfaceType,
    },
    reexports::wayland_server::{protocol::wl_surface::WlSurface, Client, DisplayHandle},
    utils::{Logical, Point, Rectangle},
    wayland::{
        input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface},
        text_input::{TextInputManagerState, TextInputSeat},
        virtual_keyboard::VirtualKeyboardManagerState,
    },
};

use crate::{state::{ClientState, WmState}, wm::Rect};

/// Environment variable that selects the `Trust` policy.
pub const POLICY_ENV: &str = "BIFROST_WM_INPUT_METHOD";

/// Which clients may bind the input-method and virtual-keyboard globals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trust {
    /// No input method and no virtual keyboard; text input is not offered either (it would have no counterpart).
    Off,
    /// Clients running as the compositor's user.
    SameUser,
}

impl Trust {
    /// The policy a setting names. Unset, empty and unknown values give `SameUser`: a typo must not open the
    /// globals further or close them silently.
    pub fn parse(value: Option<&str>) -> Trust {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("off" | "0" | "no" | "false" | "none") => Trust::Off,
            _ => Trust::SameUser,
        }
    }

    /// Whether a client with `client_uid` (`None`: its credentials could not be read) may bind the globals while the
    /// compositor runs as `own_uid` (`None`: unknown). Unknown on either side admits nobody.
    pub fn admits(self, client_uid: Option<u32>, own_uid: Option<u32>) -> bool {
        match (self, client_uid, own_uid) {
            (Trust::Off, _, _) => false,
            (Trust::SameUser, Some(c), Some(o)) => c == o,
            (Trust::SameUser, _, _) => false,
        }
    }
}

/// The uid this process runs as, read from the owner of `/proc/self`.
pub(crate) fn own_uid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// The global filter of the privileged protocols: true for the clients `trust` admits.
fn client_filter(trust: Trust, own: Option<u32>) -> impl for<'c> Fn(&'c Client) -> bool + Send + Sync + 'static {
    move |client: &Client| trust.admits(client.get_data::<ClientState>().and_then(|data| data.peer_uid), own)
}

/// Where a popup of `size` (w, h) goes for a text cursor at `cursor`, inside `bounds` (all output pixels): left edges
/// aligned, just below the cursor; above it when the bottom would leave `bounds`; kept inside `bounds` horizontally.
/// A popup taller than the room above and below is pinned to the bottom edge. Returns the top-left corner.
pub fn place_popup(cursor: Rect, size: (i32, i32), bounds: Rect) -> (i32, i32) {
    let (w, h) = (size.0.max(0), size.1.max(0));
    let right = bounds.x + bounds.w;
    let bottom = bounds.y + bounds.h;
    let x = cursor.x.min(right - w).max(bounds.x);
    let below = cursor.y + cursor.h;
    let y = if below + h <= bottom {
        below
    } else if cursor.y - h >= bounds.y {
        cursor.y - h
    } else {
        (bottom - h).max(bounds.y)
    };
    (x, y)
}

/// The globals and the popups of the input method.
pub struct TextInputProtocols {
    _text_input: Option<TextInputManagerState>,
    _input_method: Option<InputMethodManagerState>,
    _virtual_keyboard: Option<VirtualKeyboardManagerState>,
    popups: Vec<PopupSurface>,
}

impl TextInputProtocols {
    /// Creates the globals the `BIFROST_WM_INPUT_METHOD` policy allows.
    pub fn new(dh: &DisplayHandle) -> Self {
        let trust = Trust::parse(std::env::var(POLICY_ENV).ok().as_deref());
        Self::with_trust(dh, trust)
    }

    pub fn with_trust(dh: &DisplayHandle, trust: Trust) -> Self {
        if trust == Trust::Off {
            return TextInputProtocols { _text_input: None, _input_method: None, _virtual_keyboard: None, popups: Vec::new() };
        }
        let own = own_uid();
        TextInputProtocols {
            _text_input: Some(TextInputManagerState::new::<WmState>(dh)),
            _input_method: Some(InputMethodManagerState::new::<WmState, _>(dh, client_filter(trust, own))),
            _virtual_keyboard: Some(VirtualKeyboardManagerState::new::<WmState, _>(dh, client_filter(trust, own))),
            popups: Vec::new(),
        }
    }

    /// Starts tracking `popup` (an input method's candidate window), replacing a stale entry for the same popup.
    fn track(&mut self, popup: PopupSurface) {
        self.popups.retain(|p| p.alive() && *p != popup);
        self.popups.push(popup);
    }

    fn untrack(&mut self, popup: &PopupSurface) {
        self.popups.retain(|p| p.alive() && p != popup);
    }

    /// The surfaces of the live popups, bottom first.
    pub fn popup_surfaces(&self) -> impl Iterator<Item = &WlSurface> {
        self.popups.iter().filter(|p| p.alive()).map(|p| p.wl_surface())
    }
}

impl WmState {
    /// Whether the focused client has a text input enabled, that is, whether an input method should be showing.
    pub fn text_input_active(&self) -> bool {
        let mut active = false;
        self.seat.text_input().with_active_text_input(|_, _| active = true);
        active
    }

    /// The input method popups that have a parent window on screen, each with the output position of its surface
    /// origin, bottom first.
    pub fn input_popups(&self) -> Vec<(WlSurface, Point<i32, Logical>)> {
        let Some(output) = self.output() else { return Vec::new() };
        let Some(area) = self.space.output_geometry(&output) else { return Vec::new() };
        let bounds = Rect::new(area.loc.x, area.loc.y, area.size.w, area.size.h);
        self.text_input
            .popups
            .iter()
            .filter(|p| p.alive())
            .filter_map(|p| {
                let parent = p.get_parent()?;
                let window = self.window_for_surface(&parent.surface)?;
                let loc = self.space.element_location(&window)?;
                let origin = loc - window.geometry().loc;
                let cursor = p.text_input_rectangle();
                let size = bbox_from_surface_tree(p.wl_surface(), (0, 0)).size;
                let cursor = Rect::new(origin.x + cursor.loc.x, origin.y + cursor.loc.y, cursor.size.w, cursor.size.h);
                let (x, y) = place_popup(cursor, (size.w, size.h), bounds);
                Some((p.wl_surface().clone(), Point::from((x, y))))
            })
            .collect()
    }

    /// The popup surface under `pos` (output pixels) and that surface's origin, topmost first.
    pub fn input_popup_under(&self, pos: Point<f64, Logical>) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.input_popups()
            .into_iter()
            .rev()
            .find_map(|(surface, origin)| {
                under_from_surface_tree(&surface, pos, origin, WindowSurfaceType::ALL).map(|(s, p)| (s, p.to_f64()))
            })
    }
}

impl InputMethodHandler for WmState {
    fn new_popup(&mut self, surface: PopupSurface) {
        self.text_input.track(surface);
        self.dirty = true;
    }

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        self.text_input.untrack(&surface);
        self.dirty = true;
    }

    fn popup_repositioned(&mut self, _surface: PopupSurface) {
        // the position is computed from the cursor rectangle on every frame (`input_popups`)
        self.dirty = true;
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, Logical> {
        self.window_for_surface(parent)
            .and_then(|window| {
                let loc = self.space.element_location(&window)?;
                let geo = window.geometry();
                Some(Rectangle::new(loc - geo.loc, geo.size))
            })
            .unwrap_or_default()
    }
}

delegate_text_input_manager!(WmState);
delegate_input_method_manager!(WmState);
delegate_virtual_keyboard_manager!(WmState);

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect::new(0, 0, 1200, 1920);

    #[test]
    fn the_policy_defaults_to_the_same_user_and_off_closes_everything() {
        assert_eq!(Trust::parse(None), Trust::SameUser);
        assert_eq!(Trust::parse(Some("")), Trust::SameUser);
        assert_eq!(Trust::parse(Some("same-user")), Trust::SameUser);
        assert_eq!(Trust::parse(Some("garbage")), Trust::SameUser, "a typo never opens the globals further");
        for off in ["off", " OFF ", "0", "no", "false", "none"] {
            assert_eq!(Trust::parse(Some(off)), Trust::Off, "{off:?}");
        }
    }

    #[test]
    fn only_the_compositors_own_user_binds_the_privileged_globals() {
        assert!(Trust::SameUser.admits(Some(1000), Some(1000)));
        assert!(!Trust::SameUser.admits(Some(1001), Some(1000)), "another user");
        assert!(!Trust::SameUser.admits(Some(0), Some(1000)), "root is not special-cased");
        assert!(!Trust::SameUser.admits(None, Some(1000)), "unreadable credentials");
        assert!(!Trust::SameUser.admits(Some(1000), None), "unknown own uid fails closed");
        assert!(!Trust::Off.admits(Some(1000), Some(1000)));
    }

    #[test]
    fn this_process_knows_its_uid() {
        assert!(own_uid().is_some(), "/proc/self is readable");
    }

    #[test]
    fn a_popup_sits_below_the_cursor_with_the_left_edges_aligned() {
        let cursor = Rect::new(300, 400, 2, 24);
        assert_eq!(place_popup(cursor, (240, 120), SCREEN), (300, 424));
    }

    #[test]
    fn a_popup_that_would_leave_the_bottom_goes_above_the_cursor() {
        let cursor = Rect::new(300, 1850, 2, 24);
        assert_eq!(place_popup(cursor, (240, 120), SCREEN), (300, 1730));
    }

    #[test]
    fn a_popup_is_kept_inside_the_output_horizontally() {
        let right = Rect::new(1150, 400, 2, 24);
        assert_eq!(place_popup(right, (240, 120), SCREEN).0, 960, "pushed left so the right edge touches the output");
        let left = Rect::new(-30, 400, 2, 24);
        assert_eq!(place_popup(left, (240, 120), SCREEN).0, 0);
    }

    #[test]
    fn a_popup_with_no_room_above_or_below_is_pinned_to_the_bottom_edge() {
        let tall = Rect::new(10, 50, 2, 24);
        assert_eq!(place_popup(tall, (240, 1900), SCREEN), (10, 20));
        let huge = Rect::new(10, 50, 2, 24);
        assert_eq!(place_popup(huge, (240, 5000), SCREEN), (10, 0), "taller than the output: the top edge wins");
    }

    #[test]
    fn placement_respects_an_output_that_does_not_start_at_the_origin() {
        let bounds = Rect::new(1200, 0, 800, 600);
        assert_eq!(place_popup(Rect::new(1190, 100, 2, 20), (100, 50), bounds), (1200, 120));
        assert_eq!(place_popup(Rect::new(1990, 100, 2, 20), (100, 50), bounds).0, 1900);
    }

    #[test]
    fn a_zero_sized_popup_goes_right_under_the_cursor() {
        assert_eq!(place_popup(Rect::new(5, 5, 1, 10), (0, 0), SCREEN), (5, 15));
        assert_eq!(place_popup(Rect::new(5, 5, 1, 10), (-4, -4), SCREEN), (5, 15), "a negative size counts as zero");
    }
}
