//! Opacity over time for each video track of a composition: a level that holds, linear ramps, and the cross
//! dissolve between two clips. This is the pure model behind layer opacity ramps. No frames are composed here.
//! Times are integer ticks, as in the composition model, so every ramp boundary is exact.

use crate::composition::TimeRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionError {
    /// The range is empty, or starts before tick 0.
    InvalidRange,
    /// An opacity is not finite. Finite values outside 0 to 1 are clamped, not refused.
    InvalidOpacity,
}

/// The opacity of a track that has no started ramp.
pub const FULL_OPACITY: f32 = 1.0;

/// Moves linearly from `from` at the range start to `to` at its end, then holds `to`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpacityRamp {
    pub range: TimeRange,
    pub from: f32,
    pub to: f32,
}

impl OpacityRamp {
    /// The value at tick `t`. Callers only ask for ticks at or after the range start.
    fn value_at(&self, t: i64) -> f32 {
        if t >= self.range.end() {
            self.to
        } else {
            let k = (t - self.range.start) as f32 / self.range.duration as f32;
            self.from + (self.to - self.from) * k
        }
    }
}

fn check_opacity(v: f32) -> Result<f32, TransitionError> {
    if v.is_finite() {
        Ok(v.clamp(0.0, 1.0))
    } else {
        Err(TransitionError::InvalidOpacity)
    }
}

/// The opacity changes of one track, kept in start order. A ramp that starts later wins from its start onward,
/// so a ramp set over the middle of another one ends that one there.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerOpacity {
    pub track: u32,
    ramps: Vec<OpacityRamp>,
}

impl LayerOpacity {
    pub fn new(track: u32) -> Self {
        Self { track, ramps: Vec::new() }
    }
    pub fn ramps(&self) -> &[OpacityRamp] {
        &self.ramps
    }
    /// Ramps from `from` to `to` over `range`, then holds `to`. A ramp that starts on the same tick as an
    /// earlier one replaces it.
    pub fn set_ramp(&mut self, from: f32, to: f32, range: TimeRange) -> Result<(), TransitionError> {
        if range.duration <= 0 || range.start < 0 {
            return Err(TransitionError::InvalidRange);
        }
        let ramp = OpacityRamp { range, from: check_opacity(from)?, to: check_opacity(to)? };
        match self.ramps.binary_search_by(|r| r.range.start.cmp(&range.start)) {
            Ok(i) => self.ramps[i] = ramp,
            Err(i) => self.ramps.insert(i, ramp),
        }
        Ok(())
    }
    /// Holds `value` from tick `at` until a later ramp.
    pub fn set_opacity(&mut self, value: f32, at: i64) -> Result<(), TransitionError> {
        let v = check_opacity(value)?;
        self.set_ramp(v, v, TimeRange::new(at, 1))
    }
    /// The opacity at tick `t`: the latest ramp that has started, interpolated while it runs and held after it
    /// ends. A track with no started ramp is fully opaque.
    pub fn opacity_at(&self, t: i64) -> f32 {
        match self.ramps.iter().rev().find(|r| r.range.start <= t) {
            Some(r) => r.value_at(t),
            None => FULL_OPACITY,
        }
    }
}

/// The opacity parameters of a composition: one entry per track, created on first use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoMix {
    layers: Vec<LayerOpacity>,
}

impl VideoMix {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn layers(&self) -> &[LayerOpacity] {
        &self.layers
    }
    /// The parameters for a track, created the first time it is asked for.
    pub fn layer(&mut self, track: u32) -> &mut LayerOpacity {
        let i = match self.layers.iter().position(|l| l.track == track) {
            Some(i) => i,
            None => {
                self.layers.push(LayerOpacity::new(track));
                self.layers.len() - 1
            }
        };
        &mut self.layers[i]
    }
    /// The opacity of a track at tick `t`. A track with no parameters is fully opaque.
    pub fn opacity(&self, track: u32, t: i64) -> f32 {
        self.layers.iter().find(|l| l.track == track).map_or(FULL_OPACITY, |l| l.opacity_at(t))
    }
    /// Dissolves from `outgoing` to `incoming` over `range`. The outgoing track fades from opaque to clear and
    /// the incoming track rises from clear to opaque. The outgoing track stays clear after the range. Before the
    /// range the incoming track has no ramp, so it is opaque, as any track without parameters is.
    pub fn cross_dissolve(&mut self, outgoing: u32, incoming: u32, range: TimeRange) -> Result<(), TransitionError> {
        self.layer(outgoing).set_ramp(1.0, 0.0, range)?;
        self.layer(incoming).set_ramp(0.0, 1.0, range)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_track_without_parameters_is_opaque() {
        let mix = VideoMix::new();
        assert_eq!(mix.opacity(3, 50), 1.0);
    }

    #[test]
    fn ramp_is_linear_inside_the_range_and_holds_after_it() {
        let mut mix = VideoMix::new();
        mix.layer(1).set_ramp(1.0, 0.0, TimeRange::new(100, 100)).unwrap();
        assert_eq!(mix.opacity(1, 50), 1.0); // before the ramp starts
        assert_eq!(mix.opacity(1, 100), 1.0);
        assert_eq!(mix.opacity(1, 150), 0.5); // halfway through 100 ticks
        assert_eq!(mix.opacity(1, 200), 0.0); // the range end is the target
        assert_eq!(mix.opacity(1, 300), 0.0); // held after the end
    }

    #[test]
    fn invalid_ranges_and_opacities_are_refused_or_clamped() {
        let mut layer = LayerOpacity::new(1);
        assert_eq!(layer.set_ramp(1.0, 0.0, TimeRange::new(5, 0)), Err(TransitionError::InvalidRange));
        assert_eq!(layer.set_ramp(1.0, 0.0, TimeRange::new(-5, 10)), Err(TransitionError::InvalidRange));
        assert_eq!(layer.set_ramp(f32::NAN, 0.0, TimeRange::new(0, 10)), Err(TransitionError::InvalidOpacity));
        // Finite values outside 0 to 1 are clamped: 2.0 becomes 1.0 and -1.0 becomes 0.0.
        layer.set_ramp(2.0, -1.0, TimeRange::new(0, 10)).unwrap();
        assert_eq!(layer.opacity_at(0), 1.0);
        assert_eq!(layer.opacity_at(10), 0.0);
    }

    #[test]
    fn cross_dissolve_fades_the_outgoing_clip_and_raises_the_incoming_one() {
        let mut mix = VideoMix::new();
        mix.cross_dissolve(7, 8, TimeRange::new(0, 100)).unwrap();
        assert_eq!(mix.opacity(7, 25), 0.75);
        assert_eq!(mix.opacity(8, 25), 0.25);
        assert_eq!(mix.opacity(7, 100), 0.0);
        assert_eq!(mix.opacity(8, 100), 1.0);
        // The outgoing clip stays clear after the dissolve.
        assert_eq!(mix.opacity(7, 400), 0.0);
    }

    #[test]
    fn a_later_ramp_ends_the_earlier_one_where_it_starts() {
        let mut layer = LayerOpacity::new(1);
        layer.set_ramp(1.0, 0.0, TimeRange::new(0, 100)).unwrap();
        layer.set_ramp(0.5, 0.5, TimeRange::new(50, 10)).unwrap();
        assert!(near(layer.opacity_at(40), 0.6)); // still the first ramp: 1 - 0.4
        assert_eq!(layer.opacity_at(55), 0.5); // the second ramp, which is flat at 0.5
        assert_eq!(layer.opacity_at(70), 0.5); // held after the second ramp ends
    }

    #[test]
    fn set_opacity_holds_a_level_from_its_tick() {
        let mut layer = LayerOpacity::new(2);
        layer.set_opacity(0.3, 10).unwrap();
        assert_eq!(layer.opacity_at(5), 1.0);
        assert_eq!(layer.opacity_at(10), 0.3);
        assert_eq!(layer.opacity_at(500), 0.3);
    }
}
