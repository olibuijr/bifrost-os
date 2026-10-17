//! Ambient light model. It turns the text of an IIO illuminance channel into lux, smooths a run of readings, and maps
//! lux to a display brightness on a log scale. The model reads no device: a caller reads the channel files under the
//! IIO device directory and passes their text here.

/// Lux at or below which the display uses its minimum brightness.
pub const DARK_LUX: f32 = 1.0;
/// Lux at or above which the display uses its full brightness.
pub const BRIGHT_LUX: f32 = 10_000.0;
/// The dimmest brightness the ambient rule ever chooses, so a dark room never blanks the panel.
pub const MIN_BRIGHTNESS: f32 = 0.1;

/// Failures of a channel reading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadingError {
    /// The raw count, the offset or the scale is not a decimal number.
    Malformed,
    /// The scale is zero or negative, or a value is not finite.
    OutOfRange,
}

/// Converts an IIO illuminance reading to lux with the kernel's rule `(raw + offset) * scale`. An empty offset means
/// zero and an empty scale means one, as the IIO sysfs ABI defines them. A negative result is clamped to zero.
pub fn lux_from_raw(raw: &str, offset: &str, scale: &str) -> Result<f32, ReadingError> {
    let raw = parse_or(raw, None)?;
    let offset = parse_or(offset, Some(0.0))?;
    let scale = parse_or(scale, Some(1.0))?;
    if scale <= 0.0 {
        return Err(ReadingError::OutOfRange);
    }
    let lux = (raw + offset) * scale;
    if !lux.is_finite() || lux > f64::from(f32::MAX) {
        return Err(ReadingError::OutOfRange);
    }
    Ok(lux.max(0.0) as f32)
}

fn parse_or(text: &str, default: Option<f64>) -> Result<f64, ReadingError> {
    let text = text.trim();
    if text.is_empty() {
        return default.ok_or(ReadingError::Malformed);
    }
    let value: f64 = text.parse().map_err(|_| ReadingError::Malformed)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(ReadingError::OutOfRange)
    }
}

/// Exponential smoothing of lux readings. `alpha` is the weight of each new reading, in (0, 1]: a small value reacts
/// slowly and hides flicker from a passing shadow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Smoother {
    alpha: f32,
    value: Option<f32>,
}

impl Smoother {
    /// A smoother with weight `alpha`, or `None` when `alpha` is outside (0, 1] or not finite.
    pub fn new(alpha: f32) -> Option<Smoother> {
        if alpha.is_finite() && alpha > 0.0 && alpha <= 1.0 {
            Some(Smoother { alpha, value: None })
        } else {
            None
        }
    }

    /// Adds a finite, nonnegative reading and returns the smoothed value. Invalid readings are ignored; before a
    /// valid reading, an invalid input returns the dark-room floor without setting the stored value.
    pub fn push(&mut self, lux: f32) -> f32 {
        if !lux.is_finite() || lux < 0.0 {
            return self.value.unwrap_or(DARK_LUX);
        }
        let next = match self.value {
            None => lux,
            Some(previous) => previous + self.alpha * (lux - previous),
        };
        self.value = Some(next);
        next
    }

    /// The current smoothed value, or `None` before the first reading.
    pub fn value(&self) -> Option<f32> {
        self.value
    }
}

/// Display brightness in [`MIN_BRIGHTNESS`, 1] for a lux value. Brightness rises with the logarithm of the light
/// level, because the eye responds to ratios of light rather than to differences. A NaN reading counts as dark.
pub fn brightness_for_lux(lux: f32) -> f32 {
    let lux = if lux.is_nan() { DARK_LUX } else { lux.clamp(DARK_LUX, BRIGHT_LUX) };
    let position = (lux.ln() - DARK_LUX.ln()) / (BRIGHT_LUX.ln() - DARK_LUX.ln());
    MIN_BRIGHTNESS + (1.0 - MIN_BRIGHTNESS) * position
}

/// Decides when the panel brightness is written. The sensor drifts a little from one poll to the next, and every
/// write flickers the panel, so a new target is applied only when it moves at least `step` away from the level last
/// applied. The first target is always applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrightnessGate {
    step: f32,
    applied: Option<f32>,
}

impl BrightnessGate {
    /// A gate that waits for a move of `step`, which must be in (0, 1]. `None` otherwise.
    pub fn new(step: f32) -> Option<BrightnessGate> {
        if step.is_finite() && step > 0.0 && step <= 1.0 {
            Some(BrightnessGate { step, applied: None })
        } else {
            None
        }
    }

    /// Returns the brightness to write for `target`, or `None` when the panel already shows a level within `step`.
    pub fn update(&mut self, target: f32) -> Option<f32> {
        let target = target.clamp(MIN_BRIGHTNESS, 1.0);
        match self.applied {
            Some(level) if (target - level).abs() < self.step => None,
            _ => {
                self.applied = Some(target);
                Some(target)
            }
        }
    }

    /// The brightness last written, or `None` before the first write.
    pub fn applied(&self) -> Option<f32> {
        self.applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn the_first_target_is_always_applied() {
        let mut gate = BrightnessGate::new(0.05).unwrap();
        assert_eq!(gate.applied(), None);
        assert!(near(gate.update(0.6).unwrap(), 0.6));
        assert_eq!(gate.applied(), Some(0.6));
    }

    #[test]
    fn small_drift_is_not_written() {
        let mut gate = BrightnessGate::new(0.1).unwrap();
        gate.update(0.5).unwrap();
        assert_eq!(gate.update(0.52), None);
        assert_eq!(gate.update(0.45), None);
        assert_eq!(gate.applied(), Some(0.5));
    }

    #[test]
    fn a_move_of_a_full_step_is_written() {
        let mut gate = BrightnessGate::new(0.1).unwrap();
        gate.update(0.5).unwrap();
        assert!(near(gate.update(0.6).unwrap(), 0.6));
        assert!(near(gate.update(0.2).unwrap(), 0.2));
    }

    #[test]
    fn targets_are_kept_inside_the_ambient_range() {
        let mut gate = BrightnessGate::new(0.05).unwrap();
        assert!(near(gate.update(0.0).unwrap(), MIN_BRIGHTNESS));
        assert!(near(gate.update(7.0).unwrap(), 1.0));
    }

    #[test]
    fn a_step_outside_zero_to_one_is_refused() {
        assert_eq!(BrightnessGate::new(0.0), None);
        assert_eq!(BrightnessGate::new(-0.1), None);
        assert_eq!(BrightnessGate::new(1.5), None);
        assert_eq!(BrightnessGate::new(f32::NAN), None);
        assert!(BrightnessGate::new(1.0).is_some());
    }
}
