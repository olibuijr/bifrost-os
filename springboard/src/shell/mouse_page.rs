//! Settings > Mouse: the pointer choices of a desktop with a mouse, apart from the Trackpad pane. Tracking and
//! scrolling speed, double-click speed, pointer acceleration, natural scrolling and which side is the secondary
//! button. Tap to Click and the two-finger secondary click belong to the Trackpad pane only. The pane's settings link is
//! `SETTINGS_URL`.
use super::list::{Ends, Group, Item};
use super::options::{speed_slider, Options};
use super::pages::{Action, Pick, Slider, Toggle};

/// Settings URL of the pane.
pub const SETTINGS_URL: &str = "bifrost-settings:mouse";
/// Longest and shortest gap between two clicks that still make a double-click, in ms.
pub const DOUBLE_CLICK_RANGE_MS: (u32, u32) = (200, 900);
/// The gap a new account starts with.
pub const DOUBLE_CLICK_DEFAULT_MS: u32 = 400;
/// Secondary Click choices, as `secondary_index` numbers them.
pub const SECONDARY_BUTTON: [&str; 2] = ["Click on Right Side", "Click on Left Side"];

/// The Double-Click Speed slider position (0..1, Fast on the right) for a gap in ms.
pub fn double_click_slider(ms: u32) -> f32 {
    let (lo, hi) = DOUBLE_CLICK_RANGE_MS;
    ((hi - clamp_double_click(ms)) as f32 / (hi - lo) as f32).clamp(0.0, 1.0)
}

/// The double-click gap for a slider position in 0..1.
pub fn double_click_from_slider(v: f32) -> u32 {
    let (lo, hi) = DOUBLE_CLICK_RANGE_MS;
    hi - (v.clamp(0.0, 1.0) * (hi - lo) as f32).round() as u32
}

/// Clamps a stored gap into the offered range.
pub fn clamp_double_click(ms: u32) -> u32 {
    ms.clamp(DOUBLE_CLICK_RANGE_MS.0, DOUBLE_CLICK_RANGE_MS.1)
}

/// Index into `SECONDARY_BUTTON` for the buttons being swapped or not.
pub fn secondary_index(swap_buttons: bool) -> usize {
    usize::from(swap_buttons)
}

/// Whether choice `k` of `SECONDARY_BUTTON` swaps the buttons.
pub fn swaps_buttons(k: usize) -> bool {
    k == 1
}

/// The Mouse page's groups for the user's options.
pub fn groups(o: &Options) -> Vec<Group> {
    vec![
        Group::new(vec![Item::slider(Ends::Speed, speed_slider(o.mouse_speed), Action::Slider(Slider::MouseSpeed))]).header("TRACKING SPEED"),
        Group::new(vec![Item::new("Pointer Acceleration").switch(o.mouse_accel, Action::Toggle(Toggle::MouseAcceleration))])
            .footer("Moving the mouse faster takes the pointer further."),
        Group::new(vec![Item::slider(Ends::Speed, speed_slider(o.mouse_scroll), Action::Slider(Slider::MouseScroll))]).header("SCROLLING SPEED"),
        Group::new(vec![Item::slider(Ends::Speed, double_click_slider(o.double_click_ms), Action::Slider(Slider::MouseDoubleClick))]).header("DOUBLE-CLICK SPEED"),
        Group::new(vec![Item::new("Natural Scrolling").switch(o.natural_scroll, Action::Toggle(Toggle::NaturalScroll))]).footer("The content moves with the wheel, as it does on a touch screen."),
        Group::new(vec![Item::new("Secondary Click").popup(
            SECONDARY_BUTTON.iter().map(|l| l.to_string()).collect(),
            secondary_index(o.swap_buttons),
            Action::Pick(Pick::MouseSecondary(secondary_index(o.swap_buttons))),
        )]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::prefs::Prefs;

    fn labels(gs: &[Group]) -> Vec<String> {
        gs.iter().flat_map(|g| g.items.iter().map(|i| i.label.clone())).collect()
    }

    #[test]
    fn double_click_slider_round_trips_and_fast_is_a_short_gap() {
        assert_eq!(double_click_from_slider(0.0), DOUBLE_CLICK_RANGE_MS.1);
        assert_eq!(double_click_from_slider(1.0), DOUBLE_CLICK_RANGE_MS.0);
        assert!(double_click_from_slider(0.8) < double_click_from_slider(0.2));
        for ms in [200, 400, 650, 900] {
            assert_eq!(double_click_from_slider(double_click_slider(ms)), ms);
        }
        assert_eq!(clamp_double_click(5), 200);
        assert_eq!(clamp_double_click(9000), 900);
    }

    #[test]
    fn secondary_choice_maps_to_swapped_buttons() {
        assert_eq!(secondary_index(false), 0);
        assert_eq!(secondary_index(true), 1);
        assert!(!swaps_buttons(0));
        assert!(swaps_buttons(1));
    }

    #[test]
    fn page_lists_the_mouse_rows_and_none_of_the_trackpad_ones() {
        let gs = groups(&Options::default());
        let l = labels(&gs);
        for want in ["Pointer Acceleration", "Natural Scrolling", "Secondary Click"] {
            assert!(l.iter().any(|x| x == want), "{want} missing in {l:?}");
        }
        let secondary = gs[5].items[0].popup.as_ref().expect("a pop-up button");
        assert_eq!(secondary.choices, ["Click on Right Side", "Click on Left Side"]);
        assert!(!l.iter().any(|x| x == "Tap to Click" || x.contains("Two Fingers") || x.contains("Corner")), "{l:?}");
        let headers: Vec<_> = gs.iter().filter_map(|g| g.header.clone()).collect();
        assert_eq!(headers, ["TRACKING SPEED", "SCROLLING SPEED", "DOUBLE-CLICK SPEED"]);
    }

    #[test]
    fn rows_reflect_the_options() {
        let o = Options { swap_buttons: true, mouse_accel: false, double_click_ms: 900, ..Options::default() };
        let gs = groups(&o);
        assert_eq!(gs[5].items[0].accessory, crate::shell::list::Accessory::Popup);
        assert_eq!(gs[5].items[0].popup.as_ref().map(|p| p.selected), Some(1), "swapped buttons: Click on Left Side");
        assert_eq!(gs[1].items[0].accessory, crate::shell::list::Accessory::Switch(false));
        assert_eq!(gs[3].items[0].action, Action::Slider(Slider::MouseDoubleClick));
        assert_eq!(gs[5].items[0].action, Action::Pick(Pick::MouseSecondary(1)));
        assert_eq!(crate::shell::pages::chosen(&gs[5].items[0].action, 0), Some(Action::Pick(Pick::MouseSecondary(0))), "the menu's choice replaces the index");
    }

    #[test]
    fn mouse_options_persist_through_save_and_load() {
        let mut p = Prefs::memory();
        let o = Options { mouse_speed: 2.0, mouse_scroll: 0.5, double_click_ms: 700, mouse_accel: false, swap_buttons: true, ..Options::default() };
        o.save(&mut p);
        assert_eq!(p.get("double_click_ms"), Some("700"));
        assert_eq!(p.get("mouse_acceleration"), Some("0"));
        assert_eq!(Options::load(&p), o);
        p.set("double_click_ms", "5").unwrap();
        assert_eq!(Options::load(&p).double_click_ms, DOUBLE_CLICK_RANGE_MS.0);
    }

    #[test]
    fn pane_is_found_by_its_words() {
        let k = crate::shell::pages::keywords("mouse");
        for w in ["pointer", "wheel", "double-click"] {
            assert!(k.contains(w), "{w} missing in {k:?}");
        }
    }
}
