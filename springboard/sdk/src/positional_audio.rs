//! Positional audio model: a listener and sources placed in space, with the gain and the stereo pan each source gets at
//! the listener's ears. It follows the OpenAL model the reference offers: an inverse-distance attenuation clamped to the
//! source's reference and maximum distances, and an equal-power pan from the direction of the source against the
//! listener's right vector (the cross product of its forward and up vectors).
//!
//! The model is pure. It computes the two channel gains; `audio_scene::Scene` applies them to sample buffers and mixes
//! the sources of a scene to stereo, which the app hands to the audio service.

use std::fmt;

/// A point or a direction in space, in metres. The listener looks down `forward` with `up` above it, as OpenAL does
/// (default: forward -z, up +y, so right is +x).
pub type Vec3 = [f32; 3];

/// A listener: where it is and how it is turned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    pub position: Vec3,
    pub forward: Vec3,
    pub up: Vec3,
}

impl Default for Listener {
    fn default() -> Self {
        Listener { position: [0.0; 3], forward: [0.0, 0.0, -1.0], up: [0.0, 1.0, 0.0] }
    }
}

/// A positioned source and its distance model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub position: Vec3,
    /// Distance at which the gain is 1.0; nearer than this the gain stays at 1.0.
    pub reference_distance: f32,
    /// Distance beyond which the gain stops falling.
    pub max_distance: f32,
    /// How fast the gain falls with distance; 0 means no attenuation.
    pub rolloff: f32,
}

impl Source {
    /// A source with the defaults: reference 1 m, maximum 100 m, rolloff 1.
    pub fn at(position: Vec3) -> Source {
        Source { position, reference_distance: 1.0, max_distance: 100.0, rolloff: 1.0 }
    }
}

/// The two channel gains of a source at the listener, each from 0.0 to 1.0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pan {
    pub left: f32,
    pub right: f32,
}

/// Why a listener or source is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PositionError {
    /// The forward or up vector is zero, or the two are parallel, so the listener has no right side.
    DegenerateOrientation,
    /// The reference distance is not above zero, or the maximum is below it, or the rolloff is negative or not finite.
    BadDistanceModel,
}

impl fmt::Display for PositionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PositionError::DegenerateOrientation => f.write_str("listener forward and up must be non-zero and not parallel"),
            PositionError::BadDistanceModel => f.write_str("source distance model is out of range"),
        }
    }
}

impl std::error::Error for PositionError {}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

fn unit(a: Vec3) -> Option<Vec3> {
    let n = length(a);
    (n > 0.0 && n.is_finite()).then(|| [a[0] / n, a[1] / n, a[2] / n])
}

/// The listener's right vector, unit length, or an error when its orientation is degenerate.
pub fn right_vector(listener: &Listener) -> Result<Vec3, PositionError> {
    let forward = unit(listener.forward).ok_or(PositionError::DegenerateOrientation)?;
    let up = unit(listener.up).ok_or(PositionError::DegenerateOrientation)?;
    let right = cross(forward, up);
    unit(right).ok_or(PositionError::DegenerateOrientation)
}

/// Checks a source's distance model.
pub fn check_source(source: &Source) -> Result<(), PositionError> {
    let ok = source.reference_distance > 0.0
        && source.reference_distance.is_finite()
        && source.max_distance >= source.reference_distance
        && source.max_distance.is_finite()
        && source.rolloff >= 0.0
        && source.rolloff.is_finite();
    if ok {
        Ok(())
    } else {
        Err(PositionError::BadDistanceModel)
    }
}

/// The inverse-distance gain: distance clamped to [reference, max], then `ref / (ref + rolloff * (d - ref))`.
/// One at or inside the reference distance, falling with distance after it, and flat beyond the maximum.
pub fn gain(listener: &Listener, source: &Source) -> Result<f32, PositionError> {
    check_source(source)?;
    let distance = length(sub(source.position, listener.position));
    let d = distance.clamp(source.reference_distance, source.max_distance);
    Ok(source.reference_distance / (source.reference_distance + source.rolloff * (d - source.reference_distance)))
}

/// The equal-power pan of a source at the listener, times its gain. A source straight ahead, or on the listener, gets
/// the same gain in both ears; one to the right gets only the right ear, and one to the left only the left.
pub fn pan(listener: &Listener, source: &Source) -> Result<Pan, PositionError> {
    let g = gain(listener, source)?;
    let right = right_vector(listener)?;
    let to_source = sub(source.position, listener.position);
    // -1 at the left ear, +1 at the right ear; a source on the listener is centred
    let x = unit(to_source).map_or(0.0, |dir| dot(dir, right).clamp(-1.0, 1.0));
    let angle = (x + 1.0) * std::f32::consts::FRAC_PI_4;
    Ok(Pan { left: g * angle.cos(), right: g * angle.sin() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn right_is_plus_x_for_the_default_listener() {
        let r = right_vector(&Listener::default()).unwrap();
        assert!(near(r[0], 1.0) && near(r[1], 0.0) && near(r[2], 0.0));
    }

    #[test]
    fn a_degenerate_orientation_is_refused() {
        let zero = Listener { forward: [0.0; 3], ..Listener::default() };
        assert_eq!(right_vector(&zero), Err(PositionError::DegenerateOrientation));
        let parallel = Listener { forward: [0.0, 1.0, 0.0], up: [0.0, 2.0, 0.0], ..Listener::default() };
        assert_eq!(right_vector(&parallel), Err(PositionError::DegenerateOrientation));
    }

    #[test]
    fn gain_is_one_inside_the_reference_and_falls_with_distance() {
        let l = Listener::default();
        assert!(near(gain(&l, &Source::at([0.5, 0.0, 0.0])).unwrap(), 1.0), "inside the reference distance");
        assert!(near(gain(&l, &Source::at([0.0, 0.0, -2.0])).unwrap(), 0.5), "twice the reference halves the gain");
        assert!(near(gain(&l, &Source::at([0.0, 0.0, -4.0])).unwrap(), 0.25));
    }

    #[test]
    fn gain_stops_falling_beyond_the_maximum_distance() {
        let l = Listener::default();
        let mut s = Source::at([0.0, 0.0, -100.0]);
        s.max_distance = 10.0;
        let at_max = gain(&l, &s).unwrap();
        s.position = [0.0, 0.0, -1000.0];
        assert!(near(gain(&l, &s).unwrap(), at_max), "clamped at the maximum");
        assert!(at_max < 1.0);
    }

    #[test]
    fn a_zero_rolloff_gives_no_attenuation() {
        let l = Listener::default();
        let mut s = Source::at([0.0, 0.0, -50.0]);
        s.rolloff = 0.0;
        assert!(near(gain(&l, &s).unwrap(), 1.0));
    }

    #[test]
    fn a_source_ahead_is_centred_and_one_to_each_side_is_panned_fully() {
        let l = Listener::default();
        let ahead = pan(&l, &Source::at([0.0, 0.0, -1.0])).unwrap();
        assert!(near(ahead.left, ahead.right), "equal power in both ears");
        let right = pan(&l, &Source::at([1.0, 0.0, 0.0])).unwrap();
        assert!(near(right.left, 0.0) && near(right.right, 1.0));
        let left = pan(&l, &Source::at([-1.0, 0.0, 0.0])).unwrap();
        assert!(near(left.left, 1.0) && near(left.right, 0.0));
    }

    #[test]
    fn the_pan_follows_the_listener_turning() {
        // turned to face +x, the right vector becomes +z (x cross y): a source at +z is then on the right
        let turned = Listener { forward: [1.0, 0.0, 0.0], ..Listener::default() };
        assert!(near(right_vector(&turned).unwrap()[2], 1.0));
        let p = pan(&turned, &Source::at([0.0, 0.0, 1.0])).unwrap();
        assert!(near(p.left, 0.0) && near(p.right, 1.0));
    }

    #[test]
    fn a_source_on_the_listener_is_centred_at_full_gain() {
        let p = pan(&Listener::default(), &Source::at([0.0, 0.0, 0.0])).unwrap();
        assert!(near(p.left, p.right));
        assert!(near(p.left, std::f32::consts::FRAC_1_SQRT_2));
    }

    #[test]
    fn a_bad_distance_model_is_refused() {
        let l = Listener::default();
        let mut s = Source::at([0.0, 0.0, -1.0]);
        s.reference_distance = 0.0;
        assert_eq!(gain(&l, &s), Err(PositionError::BadDistanceModel));
        let mut s = Source::at([0.0, 0.0, -1.0]);
        s.max_distance = 0.5;
        assert_eq!(pan(&l, &s), Err(PositionError::BadDistanceModel));
    }
}
