//! Animated pictures for apps: the frames of an animated GIF (or another multi-image file) with their display times and
//! loop count, as renderer pixels. A still picture reads as a one-frame animation with no delay.
//!
//! ```ignore
//! let animation = image_animation::read(&gif_bytes).unwrap();
//! for frame in &animation.frames {
//!     draw(&frame.image, frame.delay_ms); // delay is in milliseconds
//! }
//! ```

use bifrost_render::cg::imageio::ImageSource as Source;
use bifrost_render::gfx::Image;

/// One image of an animation: its pixels (premultiplied) and how long it stays on screen.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimationFrame {
    pub image: Image,
    /// Display time in milliseconds. Still pictures report 0.
    pub delay_ms: u32,
}

/// Frames whose delay is this many milliseconds or less are shown for `FALLBACK_DELAY_MS` instead, the rule browsers
/// apply to animated GIF.
const SHORT_DELAY_MS: u32 = 10;
const FALLBACK_DELAY_MS: u32 = 100;

impl AnimationFrame {
    /// How long the frame is on screen, in milliseconds.
    pub fn shown_ms(&self) -> u32 {
        if self.delay_ms <= SHORT_DELAY_MS {
            FALLBACK_DELAY_MS
        } else {
            self.delay_ms
        }
    }
}

/// The frames of a picture file in display order.
#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub frames: Vec<AnimationFrame>,
    /// How many times the animation repeats; `Some(0)` repeats forever. `None` when the file does not say.
    pub loop_count: Option<u32>,
}

impl Animation {
    /// Whether there is more than one frame to show.
    pub fn is_animated(&self) -> bool {
        self.frames.len() > 1
    }

    /// The time for one pass through all the frames, in milliseconds, using each frame's `shown_ms`.
    pub fn duration_ms(&self) -> u64 {
        self.frames.iter().map(|f| u64::from(f.shown_ms())).sum()
    }

    /// The index of the frame on screen `elapsed_ms` after the animation starts. The frames play once, then repeat
    /// `loop_count` more times (`Some(0)` or `None` repeats forever). `None` once the last pass has ended, when the
    /// last frame stays on screen, or when there are no frames.
    pub fn frame_at(&self, elapsed_ms: u64) -> Option<usize> {
        let pass = self.duration_ms();
        if pass == 0 {
            return None;
        }
        let offset = match self.loop_count {
            Some(repeats) if repeats > 0 => {
                if elapsed_ms >= pass.saturating_mul(u64::from(repeats) + 1) {
                    return None;
                }
                elapsed_ms % pass
            }
            _ => elapsed_ms % pass,
        };
        let mut end = 0u64;
        for (index, frame) in self.frames.iter().enumerate() {
            end += u64::from(frame.shown_ms());
            if offset < end {
                return Some(index);
            }
        }
        None
    }
}

/// Reads every image of `bytes` as an animation. `None` when the data is not a picture in a supported format or no
/// frame decodes.
pub fn read(bytes: &[u8]) -> Option<Animation> {
    let source = Source::new(bytes.to_vec());
    source.format()?;
    let mut frames = Vec::new();
    while let Some(image) = source.image_premultiplied(frames.len()) {
        let delay_ms = source.delay_ms_at(frames.len()).unwrap_or(0);
        frames.push(AnimationFrame { image, delay_ms });
    }
    if frames.is_empty() {
        return None;
    }
    Some(Animation { frames, loop_count: source.loop_count() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics_imageio;
    use crate::image_metadata::Format;
    use bifrost_render::cg::image::CgImage;
    use bifrost_render::cg::imageio::{ImageDestination, WriteOptions};

    /// Writes a GIF with one frame per `(delay_ms)` entry, each frame a 2 x 2 solid picture.
    fn gif_with_delays(delays: &[u32], loop_count: u32) -> Vec<u8> {
        let mut dest = ImageDestination::new(Format::Gif, delays.len()).unwrap();
        dest.loop_count = loop_count;
        for &delay in delays {
            let img = CgImage::from_rgba8(2, 2, vec![255u8, 0, 0, 255].repeat(4)).unwrap();
            dest.add_image(&img, Some(WriteOptions { delay_ms: Some(delay), ..Default::default() }));
        }
        dest.finalize().unwrap()
    }

    #[test]
    fn a_gif_reads_back_its_frames_delays_and_loop_count() {
        // GIF stores delays in hundredths of a second, so these values survive the round trip exactly.
        let bytes = gif_with_delays(&[100, 200], 3);
        let animation = read(&bytes).unwrap();
        assert_eq!(animation.frames.len(), 2);
        assert_eq!(animation.frames[0].delay_ms, 100);
        assert_eq!(animation.frames[1].delay_ms, 200);
        assert_eq!(animation.loop_count, Some(3));
        assert!(animation.is_animated());
        assert_eq!(animation.duration_ms(), 300);
    }

    #[test]
    fn a_still_png_is_a_one_frame_animation_without_delay() {
        let png = graphics_imageio::encode(Format::Png, 2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        let animation = read(&png).unwrap();
        assert_eq!(animation.frames.len(), 1);
        assert_eq!(animation.frames[0].delay_ms, 0);
        assert!(!animation.is_animated());
        assert_eq!((animation.frames[0].image.w, animation.frames[0].image.h), (2, 1));
    }

    #[test]
    fn data_that_is_not_a_picture_has_no_animation() {
        assert_eq!(read(b"plain text, not a picture"), None);
    }
}
