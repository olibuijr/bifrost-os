//! Playback arbitration between participants: which registered app may play on an external output, and which may
//! play when the route cannot carry two audio sources at once. A participant is an app identifier. The arbiter
//! only answers "may this participant play now"; it never starts or stops playback itself.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArbiterError {
    /// The participant is not registered with the arbiter.
    Unknown,
}

#[derive(Clone, Debug, Default)]
pub struct Arbiter {
    participants: Vec<String>,
    preferred_external: Option<String>,
    preferred_non_mixable: Option<String>,
    /// Per-source `AVAudioMixing` state, by participant. Absent means volume 1 and pan 0.
    mixing: Vec<(String, SourceMix)>,
}

/// `AVAudioMixing` state of one source: `volume` from 0 to 1 and `pan` from -1 (left) to 1 (right).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceMix {
    pub volume: f32,
    pub pan: f32,
}

impl Default for SourceMix {
    fn default() -> Self {
        SourceMix { volume: 1.0, pan: 0.0 }
    }
}

impl Arbiter {
    /// `volume` of a source; 1 until set.
    pub fn source_volume(&self, id: &str) -> f32 {
        self.source_mix(id).volume
    }

    /// `pan` of a source; 0 until set.
    pub fn source_pan(&self, id: &str) -> f32 {
        self.source_mix(id).pan
    }

    fn source_mix(&self, id: &str) -> SourceMix {
        self.mixing.iter().find(|(p, _)| p == id).map(|(_, m)| *m).unwrap_or_default()
    }

    fn mix_entry(&mut self, id: &str) -> Result<&mut SourceMix, ArbiterError> {
        if !self.is_registered(id) {
            return Err(ArbiterError::Unknown);
        }
        if let Some(i) = self.mixing.iter().position(|(p, _)| p == id) {
            return Ok(&mut self.mixing[i].1);
        }
        self.mixing.push((id.to_string(), SourceMix::default()));
        Ok(&mut self.mixing.last_mut().expect("just pushed").1)
    }

    /// Sets a source's volume, clamped to 0 to 1 (NaN becomes 0).
    pub fn set_source_volume(&mut self, id: &str, volume: f32) -> Result<(), ArbiterError> {
        self.mix_entry(id)?.volume = if volume.is_nan() { 0.0 } else { volume.clamp(0.0, 1.0) };
        Ok(())
    }

    /// Sets a source's pan, clamped to -1 to 1 (NaN becomes 0).
    pub fn set_source_pan(&mut self, id: &str, pan: f32) -> Result<(), ArbiterError> {
        self.mix_entry(id)?.pan = if pan.is_nan() { 0.0 } else { pan.clamp(-1.0, 1.0) };
        Ok(())
    }

    /// Mixes interleaved stereo sources into one stereo block of `frames` frames. Each source is scaled by its volume
    /// and panned with an equal-power law that leaves the centre at unity. Sources from unregistered participants are
    /// skipped, a short source is padded with silence, and the sum is clamped to -1..1 so overlapping loud sources
    /// cannot wrap when converted to integers.
    pub fn mix(&self, sources: &[(&str, &[f32])], frames: usize) -> Vec<f32> {
        let mut out = vec![0.0_f32; frames * 2];
        for (id, samples) in sources {
            if !self.is_registered(id) {
                continue;
            }
            let m = self.source_mix(id);
            let (l, r) = crate::panner::PannerPosition::new(m.pan, 0.0).stereo_gains();
            let gl = (l * std::f32::consts::SQRT_2).min(1.0) * m.volume;
            let gr = (r * std::f32::consts::SQRT_2).min(1.0) * m.volume;
            for f in 0..frames.min(samples.len() / 2) {
                out[f * 2] += samples[f * 2] * gl;
                out[f * 2 + 1] += samples[f * 2 + 1] * gr;
            }
        }
        out.iter_mut().for_each(|s| *s = s.clamp(-1.0, 1.0));
        out
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a participant. Returns false when the identifier is empty or already registered.
    pub fn register(&mut self, id: &str) -> bool {
        let id = id.trim();
        if id.is_empty() || self.is_registered(id) {
            return false;
        }
        self.participants.push(id.to_string());
        true
    }

    /// Removes a participant and any preference that named it. Returns false when it was not registered.
    pub fn unregister(&mut self, id: &str) -> bool {
        let before = self.participants.len();
        self.participants.retain(|p| p != id);
        self.mixing.retain(|(p, _)| p != id);
        if self.preferred_external.as_deref() == Some(id) {
            self.preferred_external = None;
        }
        if self.preferred_non_mixable.as_deref() == Some(id) {
            self.preferred_non_mixable = None;
        }
        self.participants.len() != before
    }

    pub fn is_registered(&self, id: &str) -> bool {
        self.participants.iter().any(|p| p == id)
    }

    /// Participants in registration order.
    pub fn participants(&self) -> &[String] {
        &self.participants
    }

    /// Sets the participant with priority on external outputs. `None` clears the preference.
    pub fn set_preferred_external(&mut self, id: Option<&str>) -> Result<(), ArbiterError> {
        self.preferred_external = checked(self, id)?;
        Ok(())
    }

    /// Sets the participant with priority when the route cannot mix. `None` clears the preference.
    pub fn set_preferred_non_mixable(&mut self, id: Option<&str>) -> Result<(), ArbiterError> {
        self.preferred_non_mixable = checked(self, id)?;
        Ok(())
    }

    pub fn preferred_external(&self) -> Option<&str> {
        self.preferred_external.as_deref()
    }

    pub fn preferred_non_mixable(&self) -> Option<&str> {
        self.preferred_non_mixable.as_deref()
    }

    /// True when `id` may play on an external output: no preference is set, or `id` is the preference.
    /// An unregistered identifier is never allowed.
    pub fn may_play_external(&self, id: &str) -> bool {
        self.is_registered(id) && self.preferred_external.as_deref().map_or(true, |p| p == id)
    }

    /// True when `id` may play over a route that cannot mix sources. Same rule as `may_play_external`.
    pub fn may_play_non_mixable(&self, id: &str) -> bool {
        self.is_registered(id) && self.preferred_non_mixable.as_deref().map_or(true, |p| p == id)
    }
}

/// Validates an optional identifier against the registered participants. `None` passes through.
fn checked(arbiter: &Arbiter, id: Option<&str>) -> Result<Option<String>, ArbiterError> {
    match id {
        None => Ok(None),
        Some(id) if arbiter.is_registered(id) => Ok(Some(id.to_string())),
        Some(_) => Err(ArbiterError::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_rejects_empty_and_duplicate_identifiers() {
        let mut a = Arbiter::new();
        assert!(a.register("is.olibuijr.music"));
        assert!(!a.register("is.olibuijr.music"));
        assert!(!a.register("   "));
        assert_eq!(a.participants(), &["is.olibuijr.music".to_string()]);
    }

    #[test]
    fn without_a_preference_every_registered_participant_may_play() {
        let mut a = Arbiter::new();
        a.register("music");
        a.register("podcasts");
        assert!(a.may_play_external("music") && a.may_play_external("podcasts"));
        assert!(a.may_play_non_mixable("podcasts"));
        assert!(!a.may_play_external("unknown"), "unregistered identifiers never play");
    }

    #[test]
    fn a_preferred_participant_wins_the_external_output() {
        let mut a = Arbiter::new();
        a.register("music");
        a.register("podcasts");
        a.set_preferred_external(Some("podcasts")).unwrap();
        assert!(a.may_play_external("podcasts"));
        assert!(!a.may_play_external("music"));
        assert_eq!(a.preferred_external(), Some("podcasts"));
        assert!(a.may_play_non_mixable("music"), "the non-mixable preference is separate");
    }

    #[test]
    fn preferences_must_name_a_registered_participant() {
        let mut a = Arbiter::new();
        a.register("music");
        assert_eq!(a.set_preferred_non_mixable(Some("ghost")), Err(ArbiterError::Unknown));
        assert_eq!(a.preferred_non_mixable(), None);
        a.set_preferred_non_mixable(Some("music")).unwrap();
        assert!(a.may_play_non_mixable("music"));
        assert!(!a.may_play_non_mixable("other"), "unregistered identifiers never play");
        a.set_preferred_non_mixable(None).unwrap();
        assert_eq!(a.preferred_non_mixable(), None);
    }

    #[test]
    fn sources_default_to_full_volume_and_centre_and_clamp_their_settings() {
        let mut a = Arbiter::new();
        a.register("music");
        assert_eq!((a.source_volume("music"), a.source_pan("music")), (1.0, 0.0));
        a.set_source_volume("music", 3.0).unwrap();
        a.set_source_pan("music", -2.0).unwrap();
        assert_eq!((a.source_volume("music"), a.source_pan("music")), (1.0, -1.0));
        a.set_source_volume("music", f32::NAN).unwrap();
        assert_eq!(a.source_volume("music"), 0.0);
        assert_eq!(a.set_source_volume("ghost", 0.5), Err(ArbiterError::Unknown));
    }

    #[test]
    fn mix_applies_each_sources_own_volume_and_pan() {
        let mut a = Arbiter::new();
        a.register("music");
        a.register("podcasts");
        a.set_source_volume("music", 0.5).unwrap();
        a.set_source_pan("podcasts", 1.0).unwrap();
        let music = [0.4_f32, 0.4];
        let talk = [0.2_f32, 0.2];
        let out = a.mix(&[("music", &music), ("podcasts", &talk)], 1);
        assert!((out[0] - 0.2).abs() < 1e-6, "music left at half volume, podcast hard right");
        assert!((out[1] - (0.2 + 0.2)).abs() < 1e-6);
    }

    #[test]
    fn mix_clamps_the_sum_and_skips_unknown_sources() {
        let mut a = Arbiter::new();
        a.register("a");
        a.register("b");
        let loud = [0.9_f32, -0.9];
        let out = a.mix(&[("a", &loud), ("b", &loud), ("ghost", &loud)], 2);
        assert_eq!(out, vec![1.0, -1.0, 0.0, 0.0], "clamped, and the short sources end in silence");
    }

    #[test]
    fn unregistering_forgets_the_source_mix() {
        let mut a = Arbiter::new();
        a.register("music");
        a.set_source_volume("music", 0.2).unwrap();
        a.unregister("music");
        a.register("music");
        assert_eq!(a.source_volume("music"), 1.0);
    }

    #[test]
    fn unregistering_clears_the_preferences_that_named_it() {
        let mut a = Arbiter::new();
        a.register("music");
        a.register("podcasts");
        a.set_preferred_external(Some("music")).unwrap();
        a.set_preferred_non_mixable(Some("music")).unwrap();
        assert!(a.unregister("music"));
        assert!(!a.unregister("music"));
        assert_eq!((a.preferred_external(), a.preferred_non_mixable()), (None, None));
        assert!(a.may_play_external("podcasts"));
    }
}
