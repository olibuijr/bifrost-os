//! Smart transitions between songs: the crossfade the music player follows when one track ends and the next begins.
//! Pure: it plans where the fade sits and gives the two gains at each moment. The audio thread applies the gains to
//! the outgoing and incoming samples; nothing here reads samples.

/// The longest crossfade the settings offer, in milliseconds.
pub const MAX_FADE_MS: u32 = 12_000;

/// Where a crossfade sits in the outgoing track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// When the fade starts, in milliseconds from the start of the outgoing track.
    pub start_ms: u64,
    /// How long the fade lasts, in milliseconds.
    pub fade_ms: u32,
}

/// Equal-power gains at progress `t` (0 to 1, clamped; NaN counts as 0): `(outgoing, incoming)`. The squares of the
/// two gains add to one, so the loudness stays even through the fade.
pub fn gains(t: f32) -> (f32, f32) {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    let angle = t * std::f32::consts::FRAC_PI_2;
    (angle.cos(), angle.sin())
}

/// The crossfade for a track of `track_ms` followed by another. The requested length is capped at `MAX_FADE_MS` and
/// at half the track, so the fade never reaches back to the track's start, and it ends with the track. `None` when
/// the request is 0 (gapless: the next track just starts) or the track has no room for a fade.
pub fn plan(track_ms: u64, requested_ms: u32) -> Option<Plan> {
    let fade = (requested_ms.min(MAX_FADE_MS) as u64).min(track_ms / 2);
    if fade == 0 {
        return None;
    }
    Some(Plan { start_ms: track_ms - fade, fade_ms: fade as u32 })
}

/// The `(outgoing, incoming)` gains at `position_ms` into the outgoing track: the outgoing track at full level and the
/// incoming one silent before the fade, the reverse after it, and `gains` across it.
pub fn gains_at(plan: Plan, position_ms: u64) -> (f32, f32) {
    if position_ms <= plan.start_ms {
        return (1.0, 0.0);
    }
    let into = position_ms - plan.start_ms;
    if into >= plan.fade_ms as u64 {
        return (0.0, 1.0);
    }
    gains(into as f32 / plan.fade_ms as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f32, f32), b: (f32, f32)) -> bool {
        (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5
    }

    #[test]
    fn the_fade_ends_with_the_track() {
        assert_eq!(plan(180_000, 3_000), Some(Plan { start_ms: 177_000, fade_ms: 3_000 }));
    }

    #[test]
    fn the_request_is_capped_at_the_maximum_and_at_half_the_track() {
        assert_eq!(plan(300_000, 99_000).map(|p| p.fade_ms), Some(MAX_FADE_MS));
        assert_eq!(plan(10_000, 60_000), Some(Plan { start_ms: 5_000, fade_ms: 5_000 }));
    }

    #[test]
    fn gapless_or_too_short_tracks_have_no_fade() {
        assert_eq!(plan(180_000, 0), None);
        assert_eq!(plan(1, 3_000), None);
        assert_eq!(plan(0, 3_000), None);
    }

    #[test]
    fn gains_are_equal_power_at_the_ends_and_the_middle() {
        assert!(close(gains(0.0), (1.0, 0.0)));
        assert!(close(gains(1.0), (0.0, 1.0)));
        assert!(close(gains(f32::NAN), (1.0, 0.0)));
        assert!(close(gains(2.0), (0.0, 1.0)));
        let (a, b) = gains(0.5);
        assert!((a * a + b * b - 1.0).abs() < 1e-5, "squares must sum to one");
        assert!((a - b).abs() < 1e-5, "the midpoint is equal");
    }

    #[test]
    fn gains_at_hold_outside_the_fade_and_follow_it_inside() {
        let p = Plan { start_ms: 1_000, fade_ms: 1_000 };
        assert_eq!(gains_at(p, 0), (1.0, 0.0));
        assert_eq!(gains_at(p, 1_000), (1.0, 0.0));
        assert_eq!(gains_at(p, 2_000), (0.0, 1.0));
        assert_eq!(gains_at(p, 9_000), (0.0, 1.0));
        assert!(close(gains_at(p, 1_500), gains(0.5)));
    }
}
