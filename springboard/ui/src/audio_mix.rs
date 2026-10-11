//! Per-track volume over a timeline: a level held from a time on, and linear ramps over a span. This is the pure
//! model behind audio mix input parameters. No samples are processed here. Times are seconds.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixError {
    /// A time is negative or not finite.
    InvalidTime,
    /// A volume is not finite. Finite volumes outside 0 to 1 are clamped, not refused.
    InvalidVolume,
}

/// The volume of a track that has no level set.
pub const FULL_VOLUME: f32 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    /// The level holds from `at` until the next step.
    Hold { at: f64, volume: f32 },
    /// Moves linearly from `from` at `start` to `to` at `end`, then holds `to`.
    Ramp { start: f64, end: f64, from: f32, to: f32 },
}

impl Step {
    fn start(&self) -> f64 {
        match *self {
            Step::Hold { at, .. } => at,
            Step::Ramp { start, .. } => start,
        }
    }
}

fn check_time(t: f64) -> Result<f64, MixError> {
    if t.is_finite() && t >= 0.0 {
        Ok(t)
    } else {
        Err(MixError::InvalidTime)
    }
}

fn check_volume(v: f32) -> Result<f32, MixError> {
    if v.is_finite() {
        Ok(v.clamp(0.0, 1.0))
    } else {
        Err(MixError::InvalidVolume)
    }
}

/// The volume changes of one track, kept in time order. A later change wins from its own start onward, so a
/// level set in the middle of a ramp ends the ramp there.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackVolume {
    pub track: u32,
    steps: Vec<Step>,
}

impl TrackVolume {
    pub fn new(track: u32) -> Self {
        Self { track, steps: Vec::new() }
    }
    /// Sets the level from `at` onward. A change at the same time as an earlier one replaces it.
    pub fn set_volume(&mut self, volume: f32, at: f64) -> Result<(), MixError> {
        let at = check_time(at)?;
        let volume = check_volume(volume)?;
        self.put(Step::Hold { at, volume });
        Ok(())
    }
    /// Ramps linearly from `from` at `start` to `to` at `end`, then holds `to`. An empty or reversed span is a
    /// jump to `to` at `start`.
    pub fn set_volume_ramp(&mut self, from: f32, to: f32, start: f64, end: f64) -> Result<(), MixError> {
        let start = check_time(start)?;
        let end = check_time(end)?;
        let from = check_volume(from)?;
        let to = check_volume(to)?;
        if end <= start {
            self.put(Step::Hold { at: start, volume: to });
        } else {
            self.put(Step::Ramp { start, end, from, to });
        }
        Ok(())
    }
    /// The volume at time `t`: the latest change that has started, interpolated while a ramp runs.
    pub fn volume_at(&self, t: f64) -> f32 {
        let Some(step) = self.steps.iter().rev().find(|s| s.start() <= t) else {
            return FULL_VOLUME;
        };
        match *step {
            Step::Hold { volume, .. } => volume,
            Step::Ramp { start, end, from, to } => {
                if t >= end {
                    to
                } else {
                    let k = ((t - start) / (end - start)) as f32;
                    from + (to - from) * k
                }
            }
        }
    }
    fn put(&mut self, step: Step) {
        let at = step.start();
        match self.steps.binary_search_by(|s| s.start().total_cmp(&at)) {
            Ok(i) => self.steps[i] = step,
            Err(i) => self.steps.insert(i, step),
        }
    }
}

/// The input parameters of a mix: one entry per track, created on first use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioMix {
    inputs: Vec<TrackVolume>,
}

impl AudioMix {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn inputs(&self) -> &[TrackVolume] {
        &self.inputs
    }
    /// The parameters for a track, created the first time it is asked for.
    pub fn input(&mut self, track: u32) -> &mut TrackVolume {
        let i = match self.inputs.iter().position(|i| i.track == track) {
            Some(i) => i,
            None => {
                self.inputs.push(TrackVolume::new(track));
                self.inputs.len() - 1
            }
        };
        &mut self.inputs[i]
    }
    /// The volume of a track at time `t`. A track with no parameters plays at full volume.
    pub fn volume(&self, track: u32, t: f64) -> f32 {
        self.inputs.iter().find(|i| i.track == track).map_or(FULL_VOLUME, |i| i.volume_at(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_track_without_parameters_is_full_volume_and_a_set_level_holds() {
        let mut mix = AudioMix::new();
        assert_eq!(mix.volume(3, 10.0), FULL_VOLUME);
        mix.input(3).set_volume(0.5, 2.0).unwrap();
        assert!(close(mix.volume(3, 1.9), FULL_VOLUME), "nothing is set before the level's time");
        assert!(close(mix.volume(3, 2.0), 0.5));
        assert!(close(mix.volume(3, 100.0), 0.5), "the level holds until another change");
    }

    #[test]
    fn a_ramp_interpolates_across_its_span_and_then_holds_its_end_value() {
        let mut t = TrackVolume::new(1);
        t.set_volume_ramp(1.0, 0.0, 0.0, 4.0).unwrap();
        assert!(close(t.volume_at(0.0), 1.0));
        assert!(close(t.volume_at(2.0), 0.5));
        assert!(close(t.volume_at(4.0), 0.0));
        assert!(close(t.volume_at(9.0), 0.0));
    }

    #[test]
    fn a_level_set_inside_a_ramp_ends_the_ramp_from_its_own_time() {
        let mut t = TrackVolume::new(1);
        t.set_volume_ramp(1.0, 0.0, 0.0, 10.0).unwrap();
        t.set_volume(0.2, 5.0).unwrap();
        assert!(close(t.volume_at(4.0), 0.6), "the ramp runs until the level starts");
        assert!(close(t.volume_at(5.0), 0.2));
        assert!(close(t.volume_at(9.0), 0.2), "the ramp no longer applies");
    }

    #[test]
    fn an_empty_ramp_is_a_jump_at_its_start() {
        let mut t = TrackVolume::new(1);
        t.set_volume(0.8, 1.0).unwrap();
        t.set_volume_ramp(0.2, 0.6, 3.0, 3.0).unwrap();
        assert!(close(t.volume_at(2.9), 0.8));
        assert!(close(t.volume_at(3.0), 0.6));
    }

    #[test]
    fn invalid_times_and_volumes_are_refused_and_out_of_range_volumes_are_clamped() {
        let mut t = TrackVolume::new(1);
        assert_eq!(t.set_volume(0.5, -1.0), Err(MixError::InvalidTime));
        assert_eq!(t.set_volume(0.5, f64::NAN), Err(MixError::InvalidTime));
        assert_eq!(t.set_volume(f32::NAN, 0.0), Err(MixError::InvalidVolume));
        assert_eq!(t.set_volume_ramp(0.5, 0.5, 0.0, f64::INFINITY), Err(MixError::InvalidTime));
        assert!(close(t.volume_at(0.0), FULL_VOLUME), "a refused change leaves the track alone");
        t.set_volume(1.5, 0.0).unwrap();
        assert!(close(t.volume_at(0.0), 1.0));
        t.set_volume(-0.25, 1.0).unwrap();
        assert!(close(t.volume_at(1.0), 0.0));
    }

    #[test]
    fn a_mix_keeps_one_entry_per_track_and_a_later_change_at_the_same_time_replaces_the_earlier() {
        let mut mix = AudioMix::new();
        mix.input(1).set_volume(0.3, 1.0).unwrap();
        mix.input(1).set_volume(0.7, 1.0).unwrap();
        mix.input(2).set_volume(0.1, 0.0).unwrap();
        assert_eq!(mix.inputs().len(), 2);
        assert!(close(mix.volume(1, 1.0), 0.7));
        assert!(close(mix.volume(2, 5.0), 0.1));
        assert_eq!(mix.inputs()[0].track, 1);
    }
}
