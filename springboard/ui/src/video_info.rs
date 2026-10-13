//! Video metadata for a library item, as a pure model (the video information a media library reports for a movie or
//! show: frame size, duration, frame rate and codec). The library shows the resolution label and the duration text;
//! the media layer supplies the values.

#[derive(Clone, Debug, PartialEq)]
pub enum VideoInfoError {
    /// Width or height is zero.
    BadFrameSize,
    /// Duration is negative or not finite.
    BadDuration,
    /// Frame rate is zero, negative or not finite.
    BadFrameRate,
    /// The codec name is empty.
    EmptyCodec,
}

/// The video track of a library item.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoInfo {
    width: u32,
    height: u32,
    duration_secs: f64,
    frame_rate: f64,
    codec: String,
}

impl VideoInfo {
    /// Validates the track values. The codec name is trimmed.
    pub fn new(
        width: u32,
        height: u32,
        duration_secs: f64,
        frame_rate: f64,
        codec: &str,
    ) -> Result<Self, VideoInfoError> {
        if width == 0 || height == 0 {
            return Err(VideoInfoError::BadFrameSize);
        }
        if !duration_secs.is_finite() || duration_secs < 0.0 {
            return Err(VideoInfoError::BadDuration);
        }
        if !frame_rate.is_finite() || frame_rate <= 0.0 {
            return Err(VideoInfoError::BadFrameRate);
        }
        let codec = codec.trim();
        if codec.is_empty() {
            return Err(VideoInfoError::EmptyCodec);
        }
        Ok(Self { width, height, duration_secs, frame_rate, codec: codec.to_string() })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn duration_secs(&self) -> f64 {
        self.duration_secs
    }

    pub fn frame_rate(&self) -> f64 {
        self.frame_rate
    }

    pub fn codec(&self) -> &str {
        &self.codec
    }

    /// Width divided by height.
    pub fn aspect_ratio(&self) -> f64 {
        f64::from(self.width) / f64::from(self.height)
    }

    /// True for frames at least 720 lines high.
    pub fn is_hd(&self) -> bool {
        self.height >= 720
    }

    /// The short resolution label a library row shows, such as "1080p". Frames of 2160 lines and up show "4K";
    /// every other height shows the line count with a `p` suffix.
    pub fn resolution_label(&self) -> String {
        if self.height >= 2160 {
            "4K".to_string()
        } else {
            format!("{}p", self.height)
        }
    }

    /// The duration as `m:ss`, or `h:mm:ss` from one hour on. Fractions of a second are dropped.
    pub fn duration_text(&self) -> String {
        let total = self.duration_secs.floor() as u64;
        let (hours, minutes, seconds) = (total / 3600, (total % 3600) / 60, total % 60);
        if hours > 0 {
            format!("{hours}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes}:{seconds:02}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_labels() {
        let hd = VideoInfo::new(1920, 1080, 5400.0, 24.0, "h264").unwrap();
        assert_eq!(hd.resolution_label(), "1080p");
        assert!(hd.is_hd());
        let uhd = VideoInfo::new(3840, 2160, 60.0, 30.0, "hevc").unwrap();
        assert_eq!(uhd.resolution_label(), "4K");
        let sd = VideoInfo::new(640, 480, 60.0, 30.0, "mpeg4").unwrap();
        assert_eq!(sd.resolution_label(), "480p");
        assert!(!sd.is_hd());
    }

    #[test]
    fn duration_text_uses_hours_from_one_hour_on() {
        let short = VideoInfo::new(1, 1, 125.9, 24.0, "h264").unwrap();
        assert_eq!(short.duration_text(), "2:05");
        let long = VideoInfo::new(1, 1, 5400.0, 24.0, "h264").unwrap();
        assert_eq!(long.duration_text(), "1:30:00");
        let zero = VideoInfo::new(1, 1, 0.0, 24.0, "h264").unwrap();
        assert_eq!(zero.duration_text(), "0:00");
    }

    #[test]
    fn label_boundaries_and_duration_rollover() {
        let just_below = VideoInfo::new(3840, 2159, 1.0, 24.0, "hevc").unwrap();
        assert_eq!(just_below.resolution_label(), "2159p");
        let qhd = VideoInfo::new(2560, 1440, 3599.9, 24.0, "hevc").unwrap();
        assert_eq!(qhd.resolution_label(), "1440p");
        assert!(qhd.is_hd());
        assert_eq!(qhd.duration_text(), "59:59");
        let edge = VideoInfo::new(1280, 719, 3600.0, 24.0, "h264").unwrap();
        assert!(!edge.is_hd());
        assert_eq!(edge.duration_text(), "1:00:00");
        let hd = VideoInfo::new(1280, 720, 1.0, 24.0, "h264").unwrap();
        assert!(hd.is_hd());
    }

    #[test]
    fn aspect_ratio_is_width_over_height() {
        let wide = VideoInfo::new(1920, 1080, 1.0, 24.0, "h264").unwrap();
        assert!((wide.aspect_ratio() - 16.0 / 9.0).abs() < 1e-9);
    }

    #[test]
    fn invalid_track_values_are_refused() {
        assert_eq!(VideoInfo::new(0, 1080, 1.0, 24.0, "h264"), Err(VideoInfoError::BadFrameSize));
        assert_eq!(VideoInfo::new(1920, 1080, -1.0, 24.0, "h264"), Err(VideoInfoError::BadDuration));
        assert_eq!(VideoInfo::new(1920, 1080, f64::INFINITY, 24.0, "h264"), Err(VideoInfoError::BadDuration));
        assert_eq!(VideoInfo::new(1920, 1080, 1.0, 0.0, "h264"), Err(VideoInfoError::BadFrameRate));
        assert_eq!(VideoInfo::new(1920, 0, 1.0, 24.0, "h264"), Err(VideoInfoError::BadFrameSize));
        assert_eq!(VideoInfo::new(1920, 1080, f64::NAN, 24.0, "h264"), Err(VideoInfoError::BadDuration));
        assert_eq!(VideoInfo::new(1920, 1080, 1.0, f64::NAN, "h264"), Err(VideoInfoError::BadFrameRate));
        assert_eq!(VideoInfo::new(1920, 1080, 1.0, -24.0, "h264"), Err(VideoInfoError::BadFrameRate));
        assert_eq!(VideoInfo::new(1920, 1080, 1.0, 24.0, "  "), Err(VideoInfoError::EmptyCodec));
    }

    #[test]
    fn codec_is_trimmed() {
        let v = VideoInfo::new(1920, 1080, 1.0, 24.0, " hevc ").unwrap();
        assert_eq!(v.codec(), "hevc");
    }
}
