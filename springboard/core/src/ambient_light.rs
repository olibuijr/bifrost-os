//! Ambient light samples: illuminance in lux with the time it was taken.
//!
//! An ambient light sensor reports raw counts with a scale factor (the industrial I/O channels `in_illuminance_raw`
//! and `in_illuminance_scale`). `from_sysfs` takes the text of those two files and converts it to lux. Reading the
//! device files is the caller's job, so nothing here touches a device.

/// Why a sample could not be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleError {
    /// The lux value is infinite or not a number.
    NotFinite,
    /// The lux value is below zero, which no sensor reports.
    Negative,
    /// The sensor's scale factor must be finite and above zero.
    InvalidScale,
    /// The raw or scale text is not a number.
    Unreadable,
}

/// One illuminance reading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IlluminanceSample {
    /// When the reading was taken, in milliseconds since the epoch.
    pub timestamp_ms: u64,
    /// The illuminance in lux.
    pub lux: f64,
}

impl IlluminanceSample {
    /// A sample of `lux` taken at `timestamp_ms`. Infinite, NaN and negative values are refused.
    pub fn new(timestamp_ms: u64, lux: f64) -> Result<Self, SampleError> {
        if !lux.is_finite() {
            return Err(SampleError::NotFinite);
        }
        if lux < 0.0 {
            return Err(SampleError::Negative);
        }
        Ok(IlluminanceSample { timestamp_ms, lux })
    }

    /// A sample from a raw count and its scale factor: lux is `raw * scale`.
    pub fn from_raw(timestamp_ms: u64, raw: i64, scale: f64) -> Result<Self, SampleError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(SampleError::InvalidScale);
        }
        Self::new(timestamp_ms, raw as f64 * scale)
    }

    /// A sample from the text of the `in_illuminance_raw` and `in_illuminance_scale` files. Surrounding
    /// whitespace, such as the newline a sysfs file ends with, is ignored.
    pub fn from_sysfs(timestamp_ms: u64, raw: &str, scale: &str) -> Result<Self, SampleError> {
        let raw: i64 = raw.trim().parse().map_err(|_| SampleError::Unreadable)?;
        let scale: f64 = scale.trim().parse().map_err(|_| SampleError::Unreadable)?;
        Self::from_raw(timestamp_ms, raw, scale)
    }
}
