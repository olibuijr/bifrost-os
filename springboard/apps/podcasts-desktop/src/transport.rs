//! Transport maths for the player, on the parity models in bifrost-ui: interval skips (`skip_interval`). The speed menu
//! keeps its rates here too. Times here are milliseconds, as the audio service reports them; the models take seconds.
//! Pure functions: nothing is sent to the service from this module.

use bifrost_app::controls::skip_interval::{Direction, SkipCommand};

/// The rates the speed menu offers, slowest first. The audio service has no rate command yet, so the chosen rate
/// is kept for the session and shown; the engine plays at 1x.
pub const RATES: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// Back skip, in seconds: the mini player's back button, the Now Playing sheet's back button and the system's
/// skip-backward command.
pub const BACK_SKIP_S: f64 = 15.0;
/// Forward skip, in seconds: the mini player's forward button, the sheet's forward button, the system's
/// skip-forward command and the Right arrow key.
pub const FORWARD_SKIP_S: f64 = 30.0;

/// Where a skip lands, in milliseconds. `interval_s` is the preferred interval in seconds. `duration_ms` of zero
/// (or less) means the episode's length is not known yet, so a forward skip is not clamped. A backward skip
/// stops at zero; a forward skip stops at the end when it is known.
pub fn skip_target_ms(backward: bool, interval_s: f64, position_ms: f64, duration_ms: f64) -> f64 {
    let direction = if backward { Direction::Backward } else { Direction::Forward };
    let Ok(cmd) = SkipCommand::new(direction, &[interval_s]) else {
        return position_ms;
    };
    let duration = (duration_ms > 0.0).then_some(duration_ms / 1000.0);
    cmd.target(position_ms / 1000.0, duration) * 1000.0
}

/// The position of `rate` in `RATES`, or `None` when the rate is not one the menu offers.
pub fn rate_index(rate: f64) -> Option<usize> {
    RATES.iter().position(|r| (r - rate).abs() < 1e-9)
}

/// The speed control's label, such as "Speed 1x" or "Speed 0.75x".
pub fn speed_label(rate: f64) -> String {
    format!("Speed {rate}x")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forward_skip_moves_by_the_interval_in_milliseconds() {
        assert_eq!(skip_target_ms(false, FORWARD_SKIP_S, 60_000.0, 600_000.0), 90_000.0);
    }

    #[test]
    fn the_back_skip_is_15_seconds_and_the_forward_skip_is_30() {
        assert_eq!(BACK_SKIP_S, 15.0);
        assert_eq!(FORWARD_SKIP_S, 30.0);
        assert_eq!(skip_target_ms(true, BACK_SKIP_S, 40_000.0, 600_000.0), 25_000.0);
    }

    #[test]
    fn a_backward_skip_stops_at_the_start() {
        assert_eq!(skip_target_ms(true, 15.0, 40_000.0, 600_000.0), 25_000.0);
        assert_eq!(skip_target_ms(true, 15.0, 4_000.0, 600_000.0), 0.0);
    }

    #[test]
    fn a_forward_skip_stops_at_the_known_end() {
        assert_eq!(skip_target_ms(false, 30.0, 590_000.0, 600_000.0), 600_000.0);
    }

    #[test]
    fn an_unknown_length_is_not_clamped_forward() {
        assert_eq!(skip_target_ms(false, 30.0, 590_000.0, 0.0), 620_000.0);
    }

    #[test]
    fn a_nan_position_counts_as_the_start() {
        assert_eq!(skip_target_ms(false, 30.0, f64::NAN, 600_000.0), 30_000.0);
        assert_eq!(skip_target_ms(true, 30.0, f64::NAN, 600_000.0), 0.0);
    }

    #[test]
    fn the_menu_rates_have_positions_and_others_have_none() {
        assert_eq!(rate_index(0.5), Some(0));
        assert_eq!(rate_index(1.0), Some(2));
        assert_eq!(rate_index(2.0), Some(6));
        assert_eq!(rate_index(1.4), None);
    }

    #[test]
    fn the_speed_label_shows_the_rate() {
        assert_eq!(speed_label(1.0), "Speed 1x");
        assert_eq!(speed_label(0.75), "Speed 0.75x");
    }
}
