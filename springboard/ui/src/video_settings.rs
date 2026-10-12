//! Encoder settings for a video output: codec, frame size, average bit rate, key frame interval, and how a source
//! of another size lands in the frame. This is the pure model behind a video writer's settings. No frames are
//! encoded here.

/// The compressed format of the output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

/// How a source of another size fills the output frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// The source is stretched to the frame, changing its aspect ratio.
    Stretch,
    /// The source is scaled to fit inside the frame, leaving bars where the aspect ratios differ.
    AspectFit,
    /// The source is scaled to cover the frame. The parts that overflow the frame are cropped.
    AspectFill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsError {
    /// A width or height is zero.
    ZeroSize,
    /// A width or height is odd. Both codecs code 4:2:0 chroma, so frame sizes must be even.
    OddSize,
    /// The average bit rate is zero.
    ZeroBitRate,
    /// The key frame interval is zero.
    ZeroKeyFrameInterval,
}

/// A rectangle in frame pixels. `x` and `y` are negative when a fill crops the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

/// The settings of one video output. Bit rate and key frame interval are optional and unset by default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoSettings {
    codec: Codec,
    width: u32,
    height: u32,
    average_bit_rate: Option<u64>,
    max_key_frame_interval: Option<u32>,
}

impl VideoSettings {
    /// A frame of `width` by `height` pixels. Both must be non-zero and even.
    pub fn new(codec: Codec, width: u32, height: u32) -> Result<Self, SettingsError> {
        if width == 0 || height == 0 {
            return Err(SettingsError::ZeroSize);
        }
        if width % 2 == 1 || height % 2 == 1 {
            return Err(SettingsError::OddSize);
        }
        Ok(Self { codec, width, height, average_bit_rate: None, max_key_frame_interval: None })
    }
    /// Sets the average bit rate in bits per second.
    pub fn with_average_bit_rate(mut self, bits_per_second: u64) -> Result<Self, SettingsError> {
        if bits_per_second == 0 {
            return Err(SettingsError::ZeroBitRate);
        }
        self.average_bit_rate = Some(bits_per_second);
        Ok(self)
    }
    /// Sets the largest number of frames between two key frames.
    pub fn with_max_key_frame_interval(mut self, frames: u32) -> Result<Self, SettingsError> {
        if frames == 0 {
            return Err(SettingsError::ZeroKeyFrameInterval);
        }
        self.max_key_frame_interval = Some(frames);
        Ok(self)
    }
    pub fn codec(&self) -> Codec {
        self.codec
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    pub fn average_bit_rate(&self) -> Option<u64> {
        self.average_bit_rate
    }
    pub fn max_key_frame_interval(&self) -> Option<u32> {
        self.max_key_frame_interval
    }
    /// The size in bytes of `seconds` of video at the average bit rate, rounded up. None when no bit rate is set,
    /// or when the duration is negative or not finite.
    pub fn estimated_bytes(&self, seconds: f64) -> Option<u64> {
        let bits_per_second = self.average_bit_rate?;
        if !seconds.is_finite() || seconds < 0.0 {
            return None;
        }
        Some((bits_per_second as f64 * seconds / 8.0).ceil() as u64)
    }
    /// Where a source of `src_width` by `src_height` pixels lands in the frame. Fit keeps the aspect ratio and
    /// centres the result. Fill returns the scaled size that covers the frame, which is larger than the frame
    /// in one direction, and the overflow is cropped. None when the source is empty.
    pub fn content_rect(&self, src_width: u32, src_height: u32, fit: Fit) -> Option<Rect> {
        if src_width == 0 || src_height == 0 {
            return None;
        }
        let frame_w = self.width as i64;
        let frame_h = self.height as i64;
        if fit == Fit::Stretch {
            return Some(Rect { x: 0, y: 0, width: frame_w, height: frame_h });
        }
        let scale_x = frame_w as f64 / src_width as f64;
        let scale_y = frame_h as f64 / src_height as f64;
        let scale = if fit == Fit::AspectFit { scale_x.min(scale_y) } else { scale_x.max(scale_y) };
        let width = (src_width as f64 * scale).round() as i64;
        let height = (src_height as f64 * scale).round() as i64;
        Some(Rect { x: (frame_w - width) / 2, y: (frame_h - height) / 2, width, height })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_size_must_be_non_zero_and_even() {
        assert!(VideoSettings::new(Codec::H264, 1920, 1080).is_ok());
        assert_eq!(VideoSettings::new(Codec::H264, 1921, 1080), Err(SettingsError::OddSize));
        assert_eq!(VideoSettings::new(Codec::Hevc, 1920, 1079), Err(SettingsError::OddSize));
        assert_eq!(VideoSettings::new(Codec::Hevc, 0, 1080), Err(SettingsError::ZeroSize));
    }

    #[test]
    fn bit_rate_and_key_frame_interval_refuse_zero() {
        let base = VideoSettings::new(Codec::H264, 640, 480).unwrap();
        assert_eq!(base.clone().with_average_bit_rate(0), Err(SettingsError::ZeroBitRate));
        assert_eq!(base.clone().with_max_key_frame_interval(0), Err(SettingsError::ZeroKeyFrameInterval));
        let set = base.with_average_bit_rate(4_000_000).unwrap().with_max_key_frame_interval(60).unwrap();
        assert_eq!(set.average_bit_rate(), Some(4_000_000));
        assert_eq!(set.max_key_frame_interval(), Some(60));
    }

    #[test]
    fn estimated_bytes_is_bit_rate_times_duration_rounded_up() {
        let s = VideoSettings::new(Codec::H264, 1920, 1080).unwrap().with_average_bit_rate(8_000_000).unwrap();
        // 8 000 000 bit/s for 10 s is 80 000 000 bits, which is 10 000 000 bytes.
        assert_eq!(s.estimated_bytes(10.0), Some(10_000_000));
        // 8 000 000 bit/s for 0.1 us is 0.1 byte, which rounds up to 1 byte.
        assert_eq!(s.estimated_bytes(0.0000001), Some(1));
        assert_eq!(s.estimated_bytes(-1.0), None);
        let unset = VideoSettings::new(Codec::H264, 1920, 1080).unwrap();
        assert_eq!(unset.estimated_bytes(10.0), None);
    }

    #[test]
    fn aspect_fit_letterboxes_a_wide_source() {
        // 1000 x 500 scaled by min(1.92, 2.16) = 1.92 gives 1920 x 960, centred vertically: 60 px bars.
        let s = VideoSettings::new(Codec::H264, 1920, 1080).unwrap();
        assert_eq!(
            s.content_rect(1000, 500, Fit::AspectFit),
            Some(Rect { x: 0, y: 60, width: 1920, height: 960 })
        );
    }

    #[test]
    fn aspect_fill_covers_the_frame_and_crops_the_overflow() {
        // 1000 x 500 scaled by max(1.92, 2.16) = 2.16 gives 2160 x 1080, which overflows by 240 px across.
        let s = VideoSettings::new(Codec::H264, 1920, 1080).unwrap();
        assert_eq!(
            s.content_rect(1000, 500, Fit::AspectFill),
            Some(Rect { x: -120, y: 0, width: 2160, height: 1080 })
        );
    }

    #[test]
    fn stretch_fills_the_frame_and_an_empty_source_has_no_rect() {
        let s = VideoSettings::new(Codec::Hevc, 1920, 1080).unwrap();
        assert_eq!(
            s.content_rect(640, 480, Fit::Stretch),
            Some(Rect { x: 0, y: 0, width: 1920, height: 1080 })
        );
        assert_eq!(s.content_rect(0, 480, Fit::AspectFit), None);
        assert_eq!(s.content_rect(640, 0, Fit::AspectFill), None);
    }
}
