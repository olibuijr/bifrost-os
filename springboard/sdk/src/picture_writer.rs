//! Writes pictures with their metadata for apps. A writer takes one or more straight RGBA frames and writes one file in
//! PNG, JPEG, GIF, BMP, TIFF or ICO, with an orientation, a capture date, a location, a JPEG quality, a frame delay
//! and a GIF loop count. Each format keeps the metadata it can carry (JPEG, PNG and TIFF keep the orientation, date
//! and location; GIF, BMP and ICO keep only the pixels and frame timing). WebP is read only.
//!
//! ```ignore
//! let mut writer = PictureWriter::new(Format::Jpeg)
//!     .quality(0.9)
//!     .orientation(6)
//!     .date_taken("2026:10:13 10:00:00")
//!     .location(Location { latitude: 64.25, longitude: -21.5 });
//! writer.add_frame(w, h, rgba, 0);
//! let jpeg = writer.finish().unwrap();
//! ```

use crate::image_metadata::{decimal_to_dms, Format, Location};
use bifrost_render::cg::image::CgImage;
use bifrost_render::cg::imageio::exif::{Dict, PropValue};
use bifrost_render::cg::imageio::{ImageDestination, WriteOptions};

/// Builds one picture file from frames. Settings apply to every frame.
#[derive(Clone, Debug)]
pub struct PictureWriter {
    format: Format,
    quality: Option<f32>,
    orientation: Option<u8>,
    date_taken: Option<String>,
    location: Option<Location>,
    altitude_m: Option<f64>,
    make: Option<String>,
    model: Option<String>,
    software: Option<String>,
    loop_count: u32,
    frames: Vec<(CgImage, u32)>,
}

impl PictureWriter {
    /// A writer for `format`. Formats that cannot be written (WebP) make [`PictureWriter::finish`] return `None`.
    pub fn new(format: Format) -> Self {
        PictureWriter {
            format,
            quality: None,
            orientation: None,
            date_taken: None,
            location: None,
            altitude_m: None,
            make: None,
            model: None,
            software: None,
            loop_count: 0,
            frames: Vec::new(),
        }
    }

    /// JPEG quality from 0.0 (smallest) to 1.0 (best). Values outside the range are clamped; NaN is ignored.
    pub fn quality(mut self, quality: f32) -> Self {
        if !quality.is_nan() {
            self.quality = Some(quality.clamp(0.0, 1.0));
        }
        self
    }

    /// The EXIF orientation 1..=8 (1 is upright). Other values are ignored.
    pub fn orientation(mut self, orientation: u8) -> Self {
        if (1..=8).contains(&orientation) {
            self.orientation = Some(orientation);
        }
        self
    }

    /// The capture time as `YYYY:MM:DD HH:MM:SS`. Any other text is ignored.
    pub fn date_taken(mut self, date: impl Into<String>) -> Self {
        let date = date.into();
        if is_exif_date(&date) {
            self.date_taken = Some(date);
        }
        self
    }

    /// Where the picture was taken.
    pub fn location(mut self, location: Location) -> Self {
        self.location = Some(location);
        self
    }

    /// The camera maker and model, written as TIFF tags. An empty name is ignored.
    pub fn camera(mut self, make: impl Into<String>, model: impl Into<String>) -> Self {
        self.make = non_empty(make.into());
        self.model = non_empty(model.into());
        self
    }

    /// The software that wrote the picture, written as the TIFF Software tag. An empty name is ignored.
    pub fn software(mut self, name: impl Into<String>) -> Self {
        self.software = non_empty(name.into());
        self
    }

    /// The altitude in metres, negative below sea level. A value that is not finite is ignored.
    pub fn altitude(mut self, metres: f64) -> Self {
        if metres.is_finite() {
            self.altitude_m = Some(metres);
        }
        self
    }

    /// How many times an animation repeats; 0 repeats forever.
    pub fn loop_count(mut self, loop_count: u32) -> Self {
        self.loop_count = loop_count;
        self
    }

    /// Adds a frame of `width` x `height` straight RGBA pixels (four bytes each, rows top to bottom) shown for
    /// `delay_ms`. `false` when the pixel count does not match the size or the size is zero.
    pub fn add_frame(&mut self, width: usize, height: usize, rgba: Vec<u8>, delay_ms: u32) -> bool {
        match CgImage::from_rgba8(width, height, rgba) {
            Some(image) => {
                self.frames.push((image, delay_ms));
                true
            }
            None => false,
        }
    }

    /// The number of frames added so far.
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Writes the file. `None` when there are no frames or the format cannot be written.
    pub fn finish(self) -> Option<Vec<u8>> {
        let mut dest = ImageDestination::new(self.format, self.frames.len())?;
        dest.loop_count = self.loop_count;
        for (image, delay_ms) in &self.frames {
            dest.add_image(image, Some(self.options(*delay_ms)));
        }
        dest.finalize()
    }

    fn options(&self, delay_ms: u32) -> WriteOptions {
        let mut exif = Dict::new();
        if let Some(date) = &self.date_taken {
            exif.insert("DateTimeOriginal".into(), PropValue::Text(date.clone()));
        }
        let mut gps = Dict::new();
        if let Some(loc) = self.location {
            let (lat_ref, lon_ref) = (if loc.latitude < 0.0 { "S" } else { "N" }, if loc.longitude < 0.0 { "W" } else { "E" });
            gps.insert("LatitudeRef".into(), PropValue::Text(lat_ref.into()));
            gps.insert("Latitude".into(), PropValue::Floats(decimal_to_dms(loc.latitude).to_vec()));
            gps.insert("LongitudeRef".into(), PropValue::Text(lon_ref.into()));
            gps.insert("Longitude".into(), PropValue::Floats(decimal_to_dms(loc.longitude).to_vec()));
        }
        if let Some(metres) = self.altitude_m {
            let below = i64::from(metres < 0.0);
            gps.insert("AltitudeRef".into(), PropValue::Int(below));
            gps.insert("Altitude".into(), PropValue::Float(metres.abs()));
        }
        let mut tiff = Dict::new();
        for (key, value) in [("Make", &self.make), ("Model", &self.model), ("Software", &self.software)] {
            if let Some(text) = value {
                tiff.insert(key.into(), PropValue::Text(text.clone()));
            }
        }
        WriteOptions { quality: self.quality, orientation: self.orientation, tiff, exif, gps, delay_ms: Some(delay_ms), ..Default::default() }
    }
}

/// `Some` of the text when it is not empty.
fn non_empty(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

/// Whether `text` has the EXIF date form `YYYY:MM:DD HH:MM:SS` (19 characters, digits and the fixed separators).
pub fn is_exif_date(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 19
        && b.iter().enumerate().all(|(i, &c)| match i {
            4 | 7 => c == b':',
            10 => c == b' ',
            13 | 16 => c == b':',
            _ => c.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_animation;
    use crate::image_metadata;

    fn two_pixels() -> Vec<u8> {
        vec![255, 0, 0, 255, 0, 0, 255, 255]
    }

    #[test]
    fn exif_dates_are_checked_by_shape() {
        assert!(is_exif_date("2026:10:13 10:00:00"));
        assert!(!is_exif_date("2026-10-13 10:00:00"));
        assert!(!is_exif_date("2026:10:13 10:00:0x"));
        assert!(!is_exif_date("2026:10:13"));
    }

    #[test]
    fn a_jpeg_keeps_orientation_date_and_location() {
        let mut writer = PictureWriter::new(Format::Jpeg)
            .quality(0.9)
            .orientation(6)
            .date_taken("2026:10:13 10:00:00")
            .location(Location { latitude: 64.25, longitude: -21.5 });
        assert!(writer.add_frame(2, 1, two_pixels(), 0));
        let bytes = writer.finish().unwrap();
        let info = image_metadata::read(&bytes).unwrap();
        assert_eq!((info.format, info.width, info.height), (Format::Jpeg, 2, 1));
        assert_eq!(info.orientation, 6);
        assert_eq!(info.date_taken.as_deref(), Some("2026:10:13 10:00:00"));
        let at = info.location.unwrap();
        assert!((at.latitude - 64.25).abs() < 1e-9);
        assert!((at.longitude - -21.5).abs() < 1e-9);
    }

    #[test]
    fn an_invalid_orientation_or_date_is_ignored() {
        let mut writer = PictureWriter::new(Format::Png).orientation(9).date_taken("yesterday");
        assert!(writer.add_frame(2, 1, two_pixels(), 0));
        let info = image_metadata::read(&writer.finish().unwrap()).unwrap();
        assert_eq!(info.orientation, 1);
        assert_eq!(info.date_taken, None);
    }

    #[test]
    fn a_gif_writes_its_frames_with_delays_and_loop_count() {
        let mut writer = PictureWriter::new(Format::Gif).loop_count(2);
        assert!(writer.add_frame(2, 1, two_pixels(), 50));
        assert!(writer.add_frame(2, 1, two_pixels(), 150));
        assert_eq!(writer.frame_count(), 2);
        let animation = image_animation::read(&writer.finish().unwrap()).unwrap();
        assert_eq!(animation.frames.len(), 2);
        assert_eq!(animation.frames[0].delay_ms, 50);
        assert_eq!(animation.frames[1].delay_ms, 150);
        assert_eq!(animation.loop_count, Some(2));
    }

    #[test]
    fn a_frame_with_the_wrong_pixel_count_is_refused() {
        let mut writer = PictureWriter::new(Format::Png);
        assert!(!writer.add_frame(2, 2, vec![0; 4], 0));
        assert!(!writer.add_frame(0, 1, Vec::new(), 0));
        assert_eq!(writer.frame_count(), 0);
    }

    #[test]
    fn no_frames_or_a_read_only_format_gives_nothing() {
        assert_eq!(PictureWriter::new(Format::Png).finish(), None);
        let mut webp = PictureWriter::new(Format::Webp);
        assert!(webp.add_frame(2, 1, two_pixels(), 0));
        assert_eq!(webp.finish(), None);
    }
}
