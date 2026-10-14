//! Playback speeds for Voice Memos: the steps the Recording menu's Playback Speed submenu offers, and how a speed is
//! named. The audio service plays at the chosen speed (`audio::Client::set_rate`); this module only holds the steps.

/// The speeds the menu offers, slowest first. 1 is normal.
pub const RATES: [f32; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// The speed a new playback starts at.
pub const NORMAL: f32 = 1.0;

/// Whether `rate` is the speed of the step at `index` of `RATES` (speeds are compared to a hundredth).
pub fn is_step(rate: f32, index: usize) -> bool {
    RATES.get(index).is_some_and(|step| (step - rate).abs() < 0.005)
}

/// The label of a speed: `1x`, `1.5x`.
pub fn label(rate: f32) -> String {
    if (rate - rate.round()).abs() < 0.005 {
        format!("{}x", rate.round() as i32)
    } else {
        format!("{rate}x")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_run_from_half_to_double_speed_with_normal_among_them() {
        assert_eq!(RATES.first(), Some(&0.5));
        assert_eq!(RATES.last(), Some(&2.0));
        assert!(RATES.windows(2).all(|w| w[0] < w[1]), "slowest first");
        assert!(is_step(NORMAL, 2), "normal is the third step");
    }

    #[test]
    fn a_step_matches_only_its_own_speed() {
        assert!(is_step(1.25, 3));
        assert!(!is_step(1.25, 2));
        assert!(!is_step(1.0, 99), "no step past the end");
    }

    #[test]
    fn speeds_are_named_as_the_menu_shows_them() {
        assert_eq!(label(1.0), "1x");
        assert_eq!(label(2.0), "2x");
        assert_eq!(label(1.5), "1.5x");
        assert_eq!(label(0.75), "0.75x");
    }
}
