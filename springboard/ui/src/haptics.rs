//! App-facing haptic requests: named patterns (tap, double tap, warning, success, selection tick) and custom
//! pulse sequences, turned into the force-feedback commands a device plays (rumble magnitudes over time). The shell
//! routes a request to the first connected device that can rumble and honours the person's haptics setting.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Tap,
    DoubleTap,
    Selection,
    Success,
    Warning,
    Failure,
}

/// One pulse: start offset, length and strength (0.0 to 1.0) of the strong and weak motors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pulse {
    pub at_ms: u32,
    pub duration_ms: u32,
    pub strong: f32,
    pub weak: f32,
}

impl Pattern {
    pub fn pulses(self) -> Vec<Pulse> {
        let p = |at_ms, duration_ms, strong, weak| Pulse { at_ms, duration_ms, strong, weak };
        match self {
            Pattern::Tap => vec![p(0, 20, 0.6, 0.3)],
            Pattern::DoubleTap => vec![p(0, 20, 0.6, 0.3), p(80, 20, 0.6, 0.3)],
            Pattern::Selection => vec![p(0, 8, 0.0, 0.4)],
            Pattern::Success => vec![p(0, 20, 0.4, 0.4), p(70, 40, 0.8, 0.5)],
            Pattern::Warning => vec![p(0, 60, 0.8, 0.6), p(120, 60, 0.8, 0.6)],
            Pattern::Failure => vec![p(0, 40, 0.9, 0.5), p(90, 40, 0.9, 0.5), p(180, 90, 1.0, 0.7)],
        }
    }
}

/// The person's haptics setting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    /// 0.0 to 1.0, scales every pulse.
    pub intensity: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: true, intensity: 1.0 }
    }
}

/// The pulses to play for `pulses` under `settings`: nothing when off, strengths scaled and clamped, pulses of zero
/// length or zero strength dropped, and the sequence capped at two seconds so a request cannot rumble indefinitely.
pub fn plan(pulses: &[Pulse], settings: Settings) -> Vec<Pulse> {
    if !settings.enabled || settings.intensity <= 0.0 {
        return vec![];
    }
    let k = settings.intensity.clamp(0.0, 1.0);
    pulses
        .iter()
        .filter(|p| p.duration_ms > 0 && p.at_ms < 2000)
        .map(|p| Pulse {
            at_ms: p.at_ms,
            duration_ms: p.duration_ms.min(2000 - p.at_ms),
            strong: (p.strong * k).clamp(0.0, 1.0),
            weak: (p.weak * k).clamp(0.0, 1.0),
        })
        .filter(|p| p.strong > 0.0 || p.weak > 0.0)
        .collect()
}

/// A rumble sample for the device driver at time `t_ms`: the strongest pulse covering it, as 16-bit magnitudes.
pub fn magnitudes_at(plan: &[Pulse], t_ms: u32) -> (u16, u16) {
    let mut strong = 0.0f32;
    let mut weak = 0.0f32;
    for p in plan {
        if t_ms >= p.at_ms && t_ms < p.at_ms + p.duration_ms {
            strong = strong.max(p.strong);
            weak = weak.max(p.weak);
        }
    }
    ((strong * 65535.0).round() as u16, (weak * 65535.0).round() as u16)
}

/// Total length of a plan in milliseconds.
pub fn length_ms(plan: &[Pulse]) -> u32 {
    plan.iter().map(|p| p.at_ms + p.duration_ms).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_have_ordered_pulses() {
        for p in [Pattern::Tap, Pattern::DoubleTap, Pattern::Selection, Pattern::Success, Pattern::Warning, Pattern::Failure] {
            let v = p.pulses();
            assert!(!v.is_empty());
            assert!(v.windows(2).all(|w| w[0].at_ms + w[0].duration_ms <= w[1].at_ms), "{p:?}");
        }
        assert_eq!(Pattern::DoubleTap.pulses().len(), 2);
    }

    #[test]
    fn settings_scale_or_silence() {
        let tap = Pattern::Tap.pulses();
        assert!(plan(&tap, Settings { enabled: false, intensity: 1.0 }).is_empty());
        assert!(plan(&tap, Settings { enabled: true, intensity: 0.0 }).is_empty());
        let half = plan(&tap, Settings { enabled: true, intensity: 0.5 });
        assert!((half[0].strong - 0.3).abs() < 1e-6);
        assert_eq!(plan(&tap, Settings::default()), tap);
    }

    #[test]
    fn long_and_empty_pulses_are_trimmed() {
        let v = [
            Pulse { at_ms: 1900, duration_ms: 500, strong: 1.0, weak: 0.0 },
            Pulse { at_ms: 5000, duration_ms: 10, strong: 1.0, weak: 1.0 },
            Pulse { at_ms: 0, duration_ms: 0, strong: 1.0, weak: 1.0 },
            Pulse { at_ms: 10, duration_ms: 10, strong: 0.0, weak: 0.0 },
        ];
        let p = plan(&v, Settings::default());
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].duration_ms, 100);
        assert_eq!(length_ms(&p), 2000);
    }

    #[test]
    fn magnitudes_follow_the_pulse() {
        let p = plan(&Pattern::DoubleTap.pulses(), Settings::default());
        assert_eq!(magnitudes_at(&p, 10).0, (0.6f32 * 65535.0).round() as u16);
        assert_eq!(magnitudes_at(&p, 50), (0, 0));
        assert_ne!(magnitudes_at(&p, 90), (0, 0));
        assert_eq!(length_ms(&[]), 0);
    }
}
