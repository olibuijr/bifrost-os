//! Stream configuration: the output size, frame pacing, cursor and audio settings a capture starts with. The
//! configuration is checked before a stream starts, and `destination` says where a source rectangle lands
//! inside the output. Pure: nothing is captured here.

use crate::capture_error::StreamError;
use crate::Rect;

/// The largest queue depth this model accepts. This is a BifrOSt bound, not a value from the reference.
pub const MAX_QUEUE_DEPTH: u32 = 8;

/// The settings a capture stream is started with.
#[derive(Clone, Debug, PartialEq)]
pub struct StreamConfig {
    /// Output width in device pixels.
    pub width: u32,
    /// Output height in device pixels.
    pub height: u32,
    /// Scale the source to fit the output size. When false the source is not scaled.
    pub scales_to_fit: bool,
    /// With `scales_to_fit`, keep the source's proportions and centre it in the output.
    pub preserves_aspect_ratio: bool,
    /// How many frames the queue holds before the oldest is dropped. Must be 1 to `MAX_QUEUE_DEPTH`.
    pub queue_depth: u32,
    /// Shortest time between two frames, in seconds. Zero means no limit.
    pub min_frame_interval: f64,
    pub shows_cursor: bool,
    pub captures_audio: bool,
    /// Audio sample rate in hertz. Needed only when `captures_audio` is set.
    pub sample_rate: u32,
    /// Audio channel count. Needed only when `captures_audio` is set.
    pub channel_count: u32,
    /// A name that identifies the stream in logs.
    pub stream_name: String,
}

impl Default for StreamConfig {
    fn default() -> Self {
        StreamConfig {
            width: 1920,
            height: 1080,
            scales_to_fit: true,
            preserves_aspect_ratio: true,
            queue_depth: 3,
            min_frame_interval: 1.0 / 60.0,
            shows_cursor: true,
            captures_audio: false,
            sample_rate: 48000,
            channel_count: 2,
            stream_name: String::new(),
        }
    }
}

impl StreamConfig {
    /// Checks every value and returns the first problem as `InvalidParameter`.
    pub fn validate(&self) -> Result<(), StreamError> {
        if self.width == 0 || self.height == 0 {
            return Err(StreamError::InvalidParameter);
        }
        if self.queue_depth == 0 || self.queue_depth > MAX_QUEUE_DEPTH {
            return Err(StreamError::InvalidParameter);
        }
        if !self.min_frame_interval.is_finite() || self.min_frame_interval < 0.0 {
            return Err(StreamError::InvalidParameter);
        }
        if self.captures_audio && (self.sample_rate == 0 || self.channel_count == 0) {
            return Err(StreamError::InvalidParameter);
        }
        Ok(())
    }

    /// Frames per second that the minimum interval allows, or `None` when there is no limit.
    pub fn max_frame_rate(&self) -> Option<f64> {
        if self.min_frame_interval > 0.0 {
            Some(1.0 / self.min_frame_interval)
        } else {
            None
        }
    }

    /// Where a source of `src_w` by `src_h` points lands in the output, in output pixels.
    ///
    /// Scaling to fit with aspect kept centres the source at the largest size that fits. Scaling to fit
    /// without keeping aspect fills the whole output. Without scaling the source sits at the top left,
    /// cut to the output size. An empty source gives an empty rectangle when it has to be scaled.
    pub fn destination(&self, src_w: f32, src_h: f32) -> Rect {
        let out_w = self.width as f32;
        let out_h = self.height as f32;
        if !self.scales_to_fit {
            return Rect { x: 0.0, y: 0.0, w: src_w.min(out_w), h: src_h.min(out_h) };
        }
        if !self.preserves_aspect_ratio {
            return Rect { x: 0.0, y: 0.0, w: out_w, h: out_h };
        }
        if src_w <= 0.0 || src_h <= 0.0 {
            return Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        }
        let scale = (out_w / src_w).min(out_h / src_h);
        let w = src_w * scale;
        let h = src_h * scale;
        Rect { x: (out_w - w) / 2.0, y: (out_h - h) / 2.0, w, h }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> StreamConfig {
        StreamConfig { width: 1920, height: 1080, ..StreamConfig::default() }
    }

    #[test]
    fn default_configuration_is_valid() {
        assert_eq!(StreamConfig::default().validate(), Ok(()));
    }

    #[test]
    fn zero_size_and_out_of_range_queue_depth_are_rejected() {
        let mut c = config();
        c.width = 0;
        assert_eq!(c.validate(), Err(StreamError::InvalidParameter));
        let mut c = config();
        c.queue_depth = 9;
        assert_eq!(c.validate(), Err(StreamError::InvalidParameter));
        c.queue_depth = 8;
        assert_eq!(c.validate(), Ok(()));
        c.queue_depth = 0;
        assert_eq!(c.validate(), Err(StreamError::InvalidParameter));
    }

    #[test]
    fn audio_needs_a_sample_rate_and_channels_only_when_enabled() {
        let mut c = config();
        c.sample_rate = 0;
        assert_eq!(c.validate(), Ok(()));
        c.captures_audio = true;
        assert_eq!(c.validate(), Err(StreamError::InvalidParameter));
        c.sample_rate = 48000;
        c.channel_count = 0;
        assert_eq!(c.validate(), Err(StreamError::InvalidParameter));
    }

    #[test]
    fn non_finite_or_negative_interval_is_rejected() {
        let mut c = config();
        c.min_frame_interval = -0.5;
        assert_eq!(c.validate(), Err(StreamError::InvalidParameter));
        c.min_frame_interval = f64::NAN;
        assert_eq!(c.validate(), Err(StreamError::InvalidParameter));
    }

    #[test]
    fn frame_rate_follows_the_interval_and_zero_means_unlimited() {
        let mut c = config();
        c.min_frame_interval = 0.0;
        assert_eq!(c.max_frame_rate(), None);
        c.min_frame_interval = 0.25;
        assert_eq!(c.max_frame_rate(), Some(4.0));
    }

    #[test]
    fn fit_with_aspect_centres_the_source_at_the_largest_size() {
        // A 960 by 960 source in a 1920 by 1080 output scales by min(2, 1.125) = 1.125 to 1080 by 1080.
        let r = config().destination(960.0, 960.0);
        assert_eq!(r, Rect { x: 420.0, y: 0.0, w: 1080.0, h: 1080.0 });
    }

    #[test]
    fn fit_without_aspect_fills_the_output() {
        let mut c = config();
        c.preserves_aspect_ratio = false;
        assert_eq!(c.destination(3840.0, 2160.0), Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 });
    }

    #[test]
    fn unscaled_source_sits_top_left_and_is_cut_to_the_output() {
        let mut c = config();
        c.scales_to_fit = false;
        assert_eq!(c.destination(3000.0, 500.0), Rect { x: 0.0, y: 0.0, w: 1920.0, h: 500.0 });
        assert_eq!(c.destination(0.0, 0.0), Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
    }

    #[test]
    fn empty_source_with_scaling_gives_an_empty_rectangle() {
        assert_eq!(config().destination(0.0, 400.0), Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
    }
}
