//! Room reverberation and the direct to reverberant balance of a sound. Sabine's rule gives the reverberation time from
//! the room volume and its total absorption. The critical distance is where direct and reverberant energy are equal,
//! and the medium sets how long sound takes to travel. Pure formulas: no impulse response is generated here.

/// Speed of sound in air at 20 degrees C, in metres per second.
pub const AIR_SPEED: f32 = 343.0;

/// The substance sound travels through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Medium {
    /// Speed of sound in metres per second.
    pub speed: f32,
}

impl Medium {
    /// Returns None unless `speed` is finite and above zero.
    pub fn new(speed: f32) -> Option<Self> {
        (speed.is_finite() && speed > 0.0).then_some(Self { speed })
    }

    /// Air at 20 degrees C.
    pub fn air() -> Self {
        Self { speed: AIR_SPEED }
    }

    /// Seconds for sound to travel `distance` metres. Negative distances count as zero.
    pub fn delay(&self, distance: f32) -> f32 {
        distance.max(0.0) / self.speed
    }
}

/// A room as one volume with one average absorption coefficient over all its surfaces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Room {
    /// Air volume in cubic metres.
    pub volume: f32,
    /// Total surface area in square metres.
    pub surface_area: f32,
    /// Average absorption coefficient of the surfaces, above 0 and at most 1.
    pub absorption: f32,
}

impl Room {
    /// Returns None unless the volume and area are finite and above zero and `absorption` is in (0, 1].
    pub fn new(volume: f32, surface_area: f32, absorption: f32) -> Option<Self> {
        let finite = volume.is_finite() && surface_area.is_finite() && absorption.is_finite();
        let valid = finite
            && volume > 0.0
            && surface_area > 0.0
            && absorption > 0.0
            && absorption <= 1.0;
        valid.then_some(Self { volume, surface_area, absorption })
    }

    /// Sabine reverberation time in seconds, the time for a sound to fall by 60 dB: T = 0.161 V / (S a).
    pub fn reverb_time(&self) -> f32 {
        0.161 * self.volume / (self.surface_area * self.absorption)
    }

    /// Critical distance in metres for an omnidirectional source: 0.057 * sqrt(V / T).
    /// Inside it the direct sound dominates; outside it the reverberant sound does.
    pub fn critical_distance(&self) -> f32 {
        0.057 * (self.volume / self.reverb_time()).sqrt()
    }
}

/// Share of a source's energy that arrives directly, with energy falling as 1 / d^2 on the direct path and
/// staying constant in the reverberant field: Dc^2 / (Dc^2 + d^2). It is 1 at zero distance and 0.5 at the critical
/// distance. A critical distance of zero or less leaves no reverberant field to compete with, so the result is 0
/// for every distance above zero. Negative distances count as zero. The reverberant share is one minus this.
pub fn direct_share(distance: f32, critical: f32) -> f32 {
    let c = critical.max(0.0);
    let d = distance.max(0.0);
    let den = c * c + d * d;
    if den <= 0.0 {
        1.0
    } else {
        c * c / den
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn sabine_reverb_time_for_200_cubic_metres() {
        // S = 100 m^2, a = 0.2: A = 20 m^2 sabins, T = 0.161 * 200 / 20 = 1.61 s.
        let room = Room::new(200.0, 100.0, 0.2).unwrap();
        assert!(close(room.reverb_time(), 1.61, 1e-4));
    }

    #[test]
    fn more_absorption_shortens_the_tail() {
        let dead = Room::new(200.0, 100.0, 0.4).unwrap();
        let live = Room::new(200.0, 100.0, 0.1).unwrap();
        assert!(close(dead.reverb_time(), 0.805, 1e-4));
        assert!(close(live.reverb_time(), 3.22, 1e-4));
    }

    #[test]
    fn critical_distance_matches_the_formula() {
        // V / T = 200 / 1.61 = 124.22, sqrt = 11.1456, times 0.057 = 0.6353 m.
        let room = Room::new(200.0, 100.0, 0.2).unwrap();
        assert!(close(room.critical_distance(), 0.6353, 5e-4));
    }

    #[test]
    fn direct_share_is_half_at_the_critical_distance() {
        assert!(close(direct_share(2.0, 2.0), 0.5, 1e-6));
        assert!(close(direct_share(0.0, 2.0), 1.0, 1e-6));
        // d = 4, Dc = 2: 4 / (4 + 16) = 0.2.
        assert!(close(direct_share(4.0, 2.0), 0.2, 1e-6));
        assert!(close(direct_share(3.0, 0.0), 0.0, 1e-6));
    }

    #[test]
    fn sound_travels_343_metres_in_one_second_in_air() {
        assert!(close(Medium::air().delay(343.0), 1.0, 1e-6));
        assert!(close(Medium::air().delay(34.3), 0.1, 1e-6));
        assert!(close(Medium::air().delay(-5.0), 0.0, 1e-9));
    }

    #[test]
    fn rejects_invalid_room_and_medium() {
        assert!(Room::new(200.0, 100.0, 0.0).is_none());
        assert!(Room::new(200.0, 100.0, 1.5).is_none());
        assert!(Room::new(-1.0, 100.0, 0.2).is_none());
        assert!(Medium::new(0.0).is_none());
        assert!(Medium::new(1480.0).is_some());
    }
}
