//! Transport maths for the player, on the parity models in bifrost-ui: interval skips (`skip_interval`) and the
//! playback rate (`playback_rate`). Times here are milliseconds, as the audio service reports them; the models
//! take seconds. Pure functions: nothing is sent to the service from this module.

use bifrost_app::controls::playback_rate::RateCommand;
use bifrost_app::controls::skip_interval::{Direction, SkipCommand};

/// The rates the player offers, slowest first. The audio service has no rate command yet, so the chosen rate
/// is kept for the session and shown; the engine plays at 1x.
pub const RATES: [f64; 7] = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

/// Skip interval of the mini player's back and forward buttons, in seconds.
pub const PLAYER_SKIP_S: f64 = 30.0;
/// Skip interval of the Now Playing sheet's back button, in seconds (its forward button uses `PLAYER_SKIP_S`).
pub const SHEET_BACK_S: f64 = 15.0;

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

/// The rate after one press of the speed control: the next faster rate in `RATES`, wrapping to the slowest
/// after the fastest. A rate that is not in the list resolves to the nearest one first.
pub fn next_rate(current: f64) -> f64 {
    let Ok(cmd) = RateCommand::new(&RATES) else {
        return current;
    };
    match cmd.step(current, true) {
        Some(next) if next != current => next,
        _ => cmd.rates().first().copied().unwrap_or(current),
    }
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
        assert_eq!(skip_target_ms(false, 30.0, 60_000.0, 600_000.0), 90_000.0);
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
    fn the_speed_control_cycles_through_the_rates_and_wraps() {
        assert_eq!(next_rate(1.0), 1.25);
        assert_eq!(next_rate(0.5), 0.75);
        assert_eq!(next_rate(2.0), 0.5);
    }

    #[test]
    fn an_unlisted_rate_steps_from_the_rate_it_resolves_to() {
        // 1.4 resolves to 1.5 in RATES, so the next press goes to 1.75.
        assert_eq!(next_rate(1.4), 1.75);
    }

    #[test]
    fn the_speed_label_shows_the_rate() {
        assert_eq!(speed_label(1.0), "Speed 1x");
        assert_eq!(speed_label(0.75), "Speed 0.75x");
    }
}
