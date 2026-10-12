//! Sound in a 3D scene: a listener with a position and an orientation, the direction each source is heard from,
//! equal-power left and right gains for that direction, and box occluders that attenuate sound crossing them.
//! Pure geometry and gain math. No samples are processed and no output is opened. Units are metres.
//! The frame is right-handed: x points to the listener's right, y up, z along the listener's forward direction.
//! A listener facing +z with up +y hears a source at +x on its right.

/// A point or direction in the scene.
pub type Vec3 = [f32; 3];

const EPS: f32 = 1e-6;

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

fn scale(a: Vec3, k: f32) -> Vec3 {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn finite(a: Vec3) -> bool {
    a.iter().all(|v| v.is_finite())
}

/// How one source is heard from a listener.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hearing {
    /// Distance in metres.
    pub distance: f32,
    /// Angle in radians from straight ahead: 0 ahead, positive to the right, +-PI behind.
    pub azimuth: f32,
    /// Angle in radians above (positive) or below the horizontal plane.
    pub elevation: f32,
    /// Left to right position in -1..=1, the sine of the azimuth.
    pub pan: f32,
}

/// The point from which sound is heard, with its orientation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Listener {
    position: Vec3,
    forward: Vec3,
    up: Vec3,
    right: Vec3,
}

impl Listener {
    /// Returns None unless every vector is finite, `forward` and `up` are not zero and they are not parallel.
    /// `forward` and `up` need not be unit length or perpendicular: the listener keeps a perpendicular frame.
    pub fn new(position: Vec3, forward: Vec3, up: Vec3) -> Option<Self> {
        if !(finite(position) && finite(forward) && finite(up)) {
            return None;
        }
        if length(forward) < EPS || length(up) < EPS {
            return None;
        }
        let f = scale(forward, 1.0 / length(forward));
        let side = cross(up, f);
        let side_len = length(side);
        if side_len < EPS {
            return None;
        }
        let right = scale(side, 1.0 / side_len);
        let up_true = cross(f, right);
        Some(Self { position, forward: f, up: up_true, right })
    }

    /// The listener's position in the scene.
    pub fn position(&self) -> Vec3 {
        self.position
    }

    /// Where the listener looks, unit length.
    pub fn forward(&self) -> Vec3 {
        self.forward
    }

    /// Direction and distance of a source as the listener hears it. Returns None if the source is not finite.
    pub fn hear(&self, source: Vec3) -> Option<Hearing> {
        if !finite(source) {
            return None;
        }
        let d = sub(source, self.position);
        let distance = length(d);
        if distance < EPS {
            return Some(Hearing { distance: 0.0, azimuth: 0.0, elevation: 0.0, pan: 0.0 });
        }
        let x = dot(d, self.right);
        let y = dot(d, self.up);
        let z = dot(d, self.forward);
        let azimuth = x.atan2(z);
        let elevation = y.atan2((x * x + z * z).sqrt());
        Some(Hearing { distance, azimuth, elevation, pan: azimuth.sin() })
    }
}

/// Equal-power left and right gains for a pan position in -1..=1 (values outside are clamped).
/// Hard left gives (1, 0), centre gives sqrt(1/2) for each side, hard right gives (0, 1). The sum of squares is 1.
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = if pan.is_nan() { 0.0 } else { pan.clamp(-1.0, 1.0) };
    let theta = (p + 1.0) * std::f32::consts::FRAC_PI_4;
    (theta.cos(), theta.sin())
}

/// A box that blocks sound. `transmission` is the gain of sound that passes through it, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Occluder {
    pub min: Vec3,
    pub max: Vec3,
    pub transmission: f32,
}

impl Occluder {
    /// Returns None unless the corners are finite with `min <= max` on every axis and `transmission` is in 0..=1.
    pub fn new(min: Vec3, max: Vec3, transmission: f32) -> Option<Self> {
        if !(finite(min) && finite(max) && transmission.is_finite()) {
            return None;
        }
        if min.iter().zip(max.iter()).any(|(a, b)| a > b) {
            return None;
        }
        if !(0.0..=1.0).contains(&transmission) {
            return None;
        }
        Some(Self { min, max, transmission })
    }

    /// True when the segment from `a` to `b` passes through the box (slab test).
    fn blocks(&self, a: Vec3, b: Vec3) -> bool {
        let d = sub(b, a);
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for i in 0..3 {
            if d[i].abs() < EPS {
                if a[i] < self.min[i] || a[i] > self.max[i] {
                    return false;
                }
            } else {
                let inv = 1.0 / d[i];
                let mut near = (self.min[i] - a[i]) * inv;
                let mut far = (self.max[i] - a[i]) * inv;
                if near > far {
                    std::mem::swap(&mut near, &mut far);
                }
                t0 = t0.max(near);
                t1 = t1.min(far);
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    }
}

/// Gain of the direct path from `source` to `listener` after every occluder it crosses. Each crossed occluder
/// multiplies the gain by its transmission, so two walls at 0.5 give 0.25. With no crossed occluder the result is 1.
pub fn occlusion_gain(listener: Vec3, source: Vec3, occluders: &[Occluder]) -> f32 {
    occluders
        .iter()
        .filter(|o| o.blocks(listener, source))
        .fold(1.0f32, |gain, o| gain * o.transmission)
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
    fn source_on_the_right_has_a_quarter_turn_and_full_pan() {
        let h = facing_z().hear([2.0, 0.0, 0.0]).unwrap();
        assert!(close(h.distance, 2.0, 1e-5));
        assert!(close(h.azimuth, std::f32::consts::FRAC_PI_2, 1e-5));
        assert!(close(h.pan, 1.0, 1e-5));
        assert!(close(h.elevation, 0.0, 1e-5));
    }

    #[test]
    fn source_behind_has_azimuth_pi_and_distance_four() {
        let h = facing_z().hear([0.0, 0.0, -4.0]).unwrap();
        assert!(close(h.distance, 4.0, 1e-5));
        assert!(close(h.azimuth.abs(), std::f32::consts::PI, 1e-4));
        assert!(close(h.pan, 0.0, 1e-4));
    }

    #[test]
    fn elevation_of_a_3_4_5_offset_is_atan_3_over_4() {
        let h = facing_z().hear([0.0, 3.0, 4.0]).unwrap();
        assert!(close(h.distance, 5.0, 1e-5));
        assert!(close(h.elevation, 0.6435011, 1e-5));
        assert!(close(h.azimuth, 0.0, 1e-5));
    }

    #[test]
    fn turned_listener_hears_the_side_it_faces_as_right() {
        // Facing +x with up +y: the right-hand side is -z, so a source at -z is on the right.
        let l = Listener::new([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]).unwrap();
        let h = l.hear([0.0, 0.0, -1.0]).unwrap();
        assert!(close(h.azimuth, std::f32::consts::FRAC_PI_2, 1e-5));
        assert!(close(h.pan, 1.0, 1e-5));
    }

    #[test]
    fn equal_power_pan_law() {
        let (l, r) = pan_gains(0.0);
        assert!(close(l, 0.70710677, 1e-6));
        assert!(close(r, 0.70710677, 1e-6));
        let (l, r) = pan_gains(-1.0);
        assert!(close(l, 1.0, 1e-6) && close(r, 0.0, 1e-6));
        let (l, r) = pan_gains(1.0);
        assert!(close(l, 0.0, 1e-6) && close(r, 1.0, 1e-6));
        assert_eq!(pan_gains(5.0), pan_gains(1.0));
    }

    #[test]
    fn occluder_blocks_only_paths_that_cross_it_and_stacks() {
        let wall = Occluder::new([-1.0, -1.0, 2.0], [1.0, 1.0, 3.0], 0.25).unwrap();
        let second = Occluder::new([-1.0, -1.0, 4.0], [1.0, 1.0, 5.0], 0.5).unwrap();
        // Straight path along z crosses both boxes.
        assert!(close(occlusion_gain([0.0; 3], [0.0, 0.0, 6.0], &[wall, second]), 0.125, 1e-6));
        // Path at x = 5 passes z = 2 at x = 1.6667, outside the wall.
        assert!(close(occlusion_gain([0.0; 3], [5.0, 0.0, 6.0], &[wall, second]), 1.0, 1e-6));
    }

    #[test]
    fn rejects_bad_listener_and_occluder_input() {
        assert!(Listener::new([0.0; 3], [0.0; 3], [0.0, 1.0, 0.0]).is_none());
        assert!(Listener::new([0.0; 3], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0]).is_none());
        assert!(Occluder::new([0.0; 3], [1.0; 3], 1.5).is_none());
        assert!(Occluder::new([2.0, 0.0, 0.0], [1.0, 1.0, 1.0], 0.5).is_none());
    }
}
