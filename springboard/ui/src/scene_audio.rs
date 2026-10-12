//! Positional audio players for a 3D scene: a player attached to a point, and the left and right gain each
//! listener hears from it. The gain falls off with distance (inverse-distance rule), is multiplied by the occlusion
//! of any box between the two points, and is panned from the listener's azimuth with the equal-power law.
//! Pure gain math. No samples are played and no output is opened. Units are metres.
//! It uses the frame and pan rules of `sound_space`: a source on the listener's right has pan 1.

use crate::sound_space::{occlusion_gain, pan_gains, Listener, Occluder, Vec3};

/// Gain of a source at `distance` metres. Full gain up to `reference` metres, then
/// `reference / (reference + rolloff * (distance - reference))`, so the gain halves when the distance has grown by
/// `reference / rolloff`. A rolloff of 0 means no falloff. Distances inside the reference are clamped to it.
/// Returns 1 for a NaN distance or a non-positive reference.
pub fn distance_gain(distance: f32, reference: f32, rolloff: f32) -> f32 {
    if distance.is_nan() || !(reference > 0.0) || !rolloff.is_finite() || rolloff < 0.0 {
        return 1.0;
    }
    let d = distance.max(reference);
    reference / (reference + rolloff * (d - reference))
}

/// The left and right output gain of one player as one listener hears it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gains {
    pub left: f32,
    pub right: f32,
}

/// A player that plays a source at a point in the scene. A positional player is attenuated and panned by its
/// distance and direction. A non-positional player plays at the same level to both ears, as a plain stereo source.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioPlayer {
    position: Vec3,
    volume: f32,
    positional: bool,
    reference_distance: f32,
    rolloff: f32,
    playing: bool,
}

impl AudioPlayer {
    /// Returns None unless the position is finite and `reference_distance` is a positive finite number and
    /// `rolloff` is a finite non-negative number. `volume` is clamped to 0..=1 (NaN gives 0). The player starts
    /// stopped.
    pub fn new(position: Vec3, reference_distance: f32, rolloff: f32, volume: f32) -> Option<Self> {
        if !position.iter().all(|v| v.is_finite()) {
            return None;
        }
        if !(reference_distance.is_finite() && reference_distance > 0.0) {
            return None;
        }
        if !(rolloff.is_finite() && rolloff >= 0.0) {
            return None;
        }
        let volume = if volume.is_nan() { 0.0 } else { volume.clamp(0.0, 1.0) };
        Some(Self { position, volume, positional: true, reference_distance, rolloff, playing: false })
    }

    /// Switches between a positional player and a plain stereo one.
    pub fn set_positional(&mut self, positional: bool) {
        self.positional = positional;
    }

    /// Moves the player. Returns false, and keeps the old position, if the point is not finite.
    pub fn set_position(&mut self, position: Vec3) -> bool {
        if !position.iter().all(|v| v.is_finite()) {
            return false;
        }
        self.position = position;
        true
    }

    /// The point the player is attached to.
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// Sets the volume, clamped to 0..=1 (NaN gives 0).
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = if volume.is_nan() { 0.0 } else { volume.clamp(0.0, 1.0) };
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn play(&mut self) {
        self.playing = true;
    }

    pub fn stop(&mut self) {
        self.playing = false;
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The left and right gain this player gives the listener, after distance, occlusion and pan. A stopped player
    /// gives zero on both sides.
    pub fn mix(&self, listener: &Listener, occluders: &[Occluder]) -> Gains {
        if !self.playing {
            return Gains { left: 0.0, right: 0.0 };
        }
        if !self.positional {
            let (l, r) = pan_gains(0.0);
            return Gains { left: l * self.volume, right: r * self.volume };
        }
        let heard = match listener.hear(self.position) {
            Some(h) => h,
            None => return Gains { left: 0.0, right: 0.0 },
        };
        let level = self.volume
            * distance_gain(heard.distance, self.reference_distance, self.rolloff)
            * occlusion_gain(listener.position(), self.position, occluders);
        let (l, r) = pan_gains(heard.pan);
        Gains { left: l * level, right: r * level }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    fn facing_z() -> Listener {
        Listener::new([0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]).unwrap()
    }

    #[test]
    fn inverse_distance_halves_at_one_reference_plus_one_rolloff_step() {
        // reference 1, rolloff 1: at 3 m the denominator is 1 + 1 * 2 = 3.
        assert!(close(distance_gain(3.0, 1.0, 1.0), 1.0 / 3.0, 1e-6));
        // Inside the reference the gain stays at full.
        assert!(close(distance_gain(0.5, 1.0, 1.0), 1.0, 1e-6));
        // Rolloff 0 means no falloff at any distance.
        assert!(close(distance_gain(100.0, 1.0, 0.0), 1.0, 1e-6));
        // A degenerate reference gives full gain instead of a division by zero.
        assert!(close(distance_gain(5.0, 0.0, 1.0), 1.0, 1e-6));
    }

    #[test]
    fn source_on_the_right_at_two_metres_is_only_heard_in_the_right_ear() {
        let mut p = AudioPlayer::new([2.0, 0.0, 0.0], 1.0, 1.0, 1.0).unwrap();
        p.play();
        // Distance gain at 2 m is 1 / (1 + 1) = 0.5. Pan 1 gives (0, 1).
        let g = p.mix(&facing_z(), &[]);
        assert!(close(g.left, 0.0, 1e-5));
        assert!(close(g.right, 0.5, 1e-5));
    }

    #[test]
    fn occluder_between_listener_and_source_scales_both_ears_by_its_transmission() {
        let mut p = AudioPlayer::new([0.0, 0.0, 2.0], 1.0, 1.0, 1.0).unwrap();
        p.play();
        // Source straight ahead at 2 m: distance gain 0.5, pan 0 gives sqrt(1/2) per ear.
        // A box across the path with transmission 0.5 halves the level: 0.5 * 0.5 * sqrt(1/2).
        let wall = Occluder::new([-1.0, -1.0, 0.5], [1.0, 1.0, 1.5], 0.5).unwrap();
        let g = p.mix(&facing_z(), &[wall]);
        let expected = 0.25 * std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(g.left, expected, 1e-5));
        assert!(close(g.right, expected, 1e-5));
    }

    #[test]
    fn stopped_player_is_silent_and_volume_scales_the_mix() {
        let mut p = AudioPlayer::new([0.0, 0.0, 1.0], 1.0, 0.0, 0.5).unwrap();
        // Stopped by default.
        assert_eq!(p.mix(&facing_z(), &[]), Gains { left: 0.0, right: 0.0 });
        p.play();
        // Ahead at 1 m with no falloff: volume 0.5 times sqrt(1/2) per ear.
        let g = p.mix(&facing_z(), &[]);
        assert!(close(g.left, 0.5 * std::f32::consts::FRAC_1_SQRT_2, 1e-5));
        assert!(close(g.right, 0.5 * std::f32::consts::FRAC_1_SQRT_2, 1e-5));
        p.stop();
        assert!(!p.is_playing());
    }

    #[test]
    fn non_positional_player_ignores_distance_and_direction() {
        let mut p = AudioPlayer::new([50.0, 0.0, 0.0], 1.0, 1.0, 1.0).unwrap();
        p.set_positional(false);
        p.play();
        let g = p.mix(&facing_z(), &[]);
        assert!(close(g.left, std::f32::consts::FRAC_1_SQRT_2, 1e-5));
        assert!(close(g.right, std::f32::consts::FRAC_1_SQRT_2, 1e-5));
    }

    #[test]
    fn invalid_values_are_rejected_or_clamped() {
        assert!(AudioPlayer::new([f32::NAN, 0.0, 0.0], 1.0, 1.0, 1.0).is_none());
        assert!(AudioPlayer::new([0.0, 0.0, 0.0], 0.0, 1.0, 1.0).is_none());
        assert!(AudioPlayer::new([0.0, 0.0, 0.0], 1.0, -1.0, 1.0).is_none());
        let mut p = AudioPlayer::new([0.0, 0.0, 0.0], 1.0, 1.0, 3.0).unwrap();
        assert!(close(p.volume(), 1.0, 1e-6));
        p.set_volume(f32::NAN);
        assert!(close(p.volume(), 0.0, 1e-6));
        assert!(!p.set_position([0.0, f32::INFINITY, 0.0]));
        assert_eq!(p.position(), [0.0, 0.0, 0.0]);
    }
}
