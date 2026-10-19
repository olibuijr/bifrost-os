//! Picture metadata for apps: the size, orientation, resolution, capture date, camera and location of a picture, read
//! from its bytes without decoding the pixels, plus a decode that turns the picture upright by its orientation.
//!
//! ```ignore
//! let info = image_metadata::read(&jpeg_bytes).unwrap();
//! let upright = image_metadata::decode_upright(&jpeg_bytes); // pixels rotated and flipped as the camera recorded
//! ```
//!
//! Location values are decimal degrees: north and east are positive, south and west are negative.

use bifrost_render::cg::imageio::exif::{Dict, PropValue};
use bifrost_render::cg::imageio::{orient_image, ImageSource as Source};
use bifrost_render::gfx::Image;

pub use bifrost_render::cg::imageio::Format;

/// A place on the Earth in decimal degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
}

/// What the file says about one picture, read without decoding its pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct PictureInfo {
    pub format: Format,
    pub width: usize,
    pub height: usize,
    /// EXIF orientation 1..=8; 1 means upright.
    pub orientation: u8,
    /// Resolution in dots per inch (horizontal, vertical), when the file records one.
    pub dpi: Option<(f64, f64)>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    /// The capture time as the file stores it, `YYYY:MM:DD HH:MM:SS`.
    pub date_taken: Option<String>,
    /// Exposure time in seconds.
    pub exposure_time: Option<f64>,
    pub f_number: Option<f64>,
    pub iso: Option<i64>,
    pub focal_length_mm: Option<f64>,
    pub location: Option<Location>,
    /// Altitude in metres, negative below sea level, when the GPS block records one.
    pub altitude_m: Option<f64>,
    /// The software that wrote the file, when it records one.
    pub software: Option<String>,
    /// The lens model, when the EXIF block records one.
    pub lens_model: Option<String>,
    /// The lens maker, when the EXIF block records one.
    pub lens_make: Option<String>,
    /// Exposure compensation in EV (negative is darker).
    pub exposure_bias: Option<f64>,
    /// The UTC time the GPS fix was taken, when the GPS block records one.
    pub gps_time: Option<GpsTime>,
    /// Ground speed in kilometres per hour, converted from the unit the file names (km/h, mph or knots).
    pub gps_speed_kmh: Option<f64>,
    /// The direction the camera pointed in degrees clockwise from north, 0 up to 360.
    pub gps_direction_deg: Option<f64>,
}

/// The UTC time of a GPS fix, as the GPS block stores it: a date (when present) and a time of day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpsTime {
    /// `(year, month, day)`, when the block has a date stamp.
    pub date: Option<(u16, u8, u8)>,
    pub hour: u8,
    pub minute: u8,
    pub second: f64,
}

impl GpsTime {
    /// The time as `YYYY:MM:DD HH:MM:SS` (seconds rounded down), or `HH:MM:SS` when there is no date.
    pub fn to_text(&self) -> String {
        let clock = format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second.trunc() as u32);
        match self.date {
            Some((y, m, d)) => format!("{y:04}:{m:02}:{d:02} {clock}"),
            None => clock,
        }
    }
}

/// Reads the metadata of the first image in `bytes`. `None` when the data is not a picture in a supported format.
pub fn read(bytes: &[u8]) -> Option<PictureInfo> {
    let source = Source::new(bytes.to_vec());
    let format = source.format()?;
    let p = source.properties_at(0)?;
    if p.pixel_width == 0 || p.pixel_height == 0 {
        return None;
    }
    Some(PictureInfo {
        format,
        width: p.pixel_width,
        height: p.pixel_height,
        orientation: p.orientation,
        dpi: p.dpi_width.zip(p.dpi_height),
        camera_make: text(&p.tiff, "Make"),
        camera_model: text(&p.tiff, "Model"),
        date_taken: text(&p.exif, "DateTimeOriginal").or_else(|| text(&p.tiff, "DateTime")),
        exposure_time: number(&p.exif, "ExposureTime"),
        f_number: number(&p.exif, "FNumber"),
        iso: p.exif.get("ISOSpeedRatings").and_then(PropValue::as_i64),
        focal_length_mm: number(&p.exif, "FocalLength"),
        location: location(&p.gps),
        altitude_m: altitude(&p.gps),
        software: text(&p.tiff, "Software"),
        lens_model: text(&p.exif, "LensModel"),
        lens_make: text(&p.exif, "LensMake"),
        exposure_bias: number(&p.exif, "ExposureBiasValue"),
        gps_time: gps_time(&p.gps),
        gps_speed_kmh: gps_speed_kmh(&p.gps),
        gps_direction_deg: gps_direction(&p.gps),
    })
}

/// The first image of `bytes` as premultiplied pixels, turned upright by its EXIF orientation.
pub fn decode_upright(bytes: &[u8]) -> Option<Image> {
    let source = Source::new(bytes.to_vec());
    let orientation = source.properties_at(0)?.orientation;
    let picture = source.image_premultiplied(0)?;
    Some(orient_image(&picture, orientation))
}

/// Converts degrees, minutes and seconds (as EXIF stores a coordinate) to decimal degrees. `negative` is true for the
/// south and west hemispheres. `None` for an empty or more than three-part value, or a value out of range (latitude
/// beyond 90, longitude beyond 180).
pub fn dms_to_decimal(parts: &[f64], negative: bool, max_degrees: f64) -> Option<f64> {
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| !p.is_finite() || *p < 0.0) {
        return None;
    }
    let minutes = parts.get(1).copied().unwrap_or(0.0);
    let seconds = parts.get(2).copied().unwrap_or(0.0);
    let value = parts[0] + minutes / 60.0 + seconds / 3600.0;
    if value > max_degrees {
        return None;
    }
    Some(if negative { -value } else { value })
}

/// Splits decimal degrees (sign ignored) into whole degrees, whole minutes and seconds, the form EXIF stores.
pub fn decimal_to_dms(value: f64) -> [f64; 3] {
    let magnitude = value.abs();
    let degrees = magnitude.trunc();
    let minutes_exact = (magnitude - degrees) * 60.0;
    let minutes = minutes_exact.trunc();
    let seconds = (minutes_exact - minutes) * 60.0;
    [degrees, minutes, seconds]
}

fn text(d: &Dict, key: &str) -> Option<String> {
    d.get(key)?.as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn number(d: &Dict, key: &str) -> Option<f64> {
    d.get(key)?.as_f64()
}

fn location(gps: &Dict) -> Option<Location> {
    let latitude = coordinate(gps, "Latitude", "LatitudeRef", 'S', 90.0)?;
    let longitude = coordinate(gps, "Longitude", "LongitudeRef", 'W', 180.0)?;
    Some(Location { latitude, longitude })
}

/// The altitude in metres. AltitudeRef 1 means below sea level; any other value (or none) means above.
fn altitude(gps: &Dict) -> Option<f64> {
    let metres = number(gps, "Altitude")?;
    let below = gps.get("AltitudeRef").and_then(PropValue::as_i64) == Some(1);
    Some(if below { -metres } else { metres })
}

/// The GPS time stamp (hours, minutes, seconds in UTC) with the date stamp when there is one. `None` for a time
/// that is not a valid clock reading.
fn gps_time(gps: &Dict) -> Option<GpsTime> {
    let parts = match gps.get("TimeStamp")? {
        PropValue::Floats(v) => v.clone(),
        PropValue::Ints(v) => v.iter().map(|&i| i as f64).collect(),
        _ => return None,
    };
    let [h, m, s] = parts[..] else { return None };
    if !(0.0..24.0).contains(&h) || !(0.0..60.0).contains(&m) || !(0.0..61.0).contains(&s) {
        return None;
    }
    let date = text(gps, "DateStamp").and_then(|t| {
        let mut it = t.split(':').map(|x| x.parse::<u16>().ok());
        let (y, mo, d) = (it.next()??, it.next()??, it.next()??);
        ((1..=12).contains(&mo) && (1..=31).contains(&d)).then_some((y, mo as u8, d as u8))
    });
    Some(GpsTime { date, hour: h as u8, minute: m as u8, second: s })
}

/// The ground speed in km/h. SpeedRef is `K` (km/h, the default), `M` (miles per hour) or `N` (knots).
fn gps_speed_kmh(gps: &Dict) -> Option<f64> {
    let speed = number(gps, "Speed").filter(|v| v.is_finite() && *v >= 0.0)?;
    let factor = match text(gps, "SpeedRef").map(|r| r.to_ascii_uppercase()).as_deref() {
        Some("M") => 1.609344,
        Some("N") => 1.852,
        _ => 1.0,
    };
    Some(speed * factor)
}

/// The image direction in degrees from true or magnetic north, when it is a compass reading (0 up to 360).
fn gps_direction(gps: &Dict) -> Option<f64> {
    number(gps, "ImgDirection").filter(|v| (0.0..=360.0).contains(v))
}

fn coordinate(gps: &Dict, value_key: &str, ref_key: &str, negative_letter: char, max: f64) -> Option<f64> {
    let parts = match gps.get(value_key)? {
        PropValue::Floats(v) => v.clone(),
        PropValue::Ints(v) => v.iter().map(|&i| i as f64).collect(),
        PropValue::Float(f) => vec![*f],
        _ => return None,
    };
    let hemisphere = text(gps, ref_key)?.to_ascii_uppercase();
    dms_to_decimal(&parts, hemisphere.starts_with(negative_letter), max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics_imageio;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn degrees_minutes_seconds_become_decimal_degrees() {
        // 64 deg 8 min 30.5 s = 64 + 8/60 + 30.5/3600
        let v = dms_to_decimal(&[64.0, 8.0, 30.5], false, 90.0).unwrap();
        assert!(close(v, 64.0 + 8.0 / 60.0 + 30.5 / 3600.0));
    }

    #[test]
    fn south_and_west_are_negative() {
        assert_eq!(dms_to_decimal(&[21.0, 30.0, 0.0], true, 180.0), Some(-21.5));
    }

    #[test]
    fn a_missing_minute_or_second_counts_as_zero() {
        assert_eq!(dms_to_decimal(&[64.0], false, 90.0), Some(64.0));
        assert_eq!(dms_to_decimal(&[64.0, 30.0], false, 90.0), Some(64.5));
    }

    #[test]
    fn out_of_range_or_malformed_coordinates_are_refused() {
        assert_eq!(dms_to_decimal(&[95.0], false, 90.0), None);
        assert_eq!(dms_to_decimal(&[], false, 90.0), None);
        assert_eq!(dms_to_decimal(&[1.0, 2.0, 3.0, 4.0], false, 90.0), None);
        assert_eq!(dms_to_decimal(&[f64::NAN], false, 90.0), None);
        assert_eq!(dms_to_decimal(&[-1.0], false, 90.0), None);
    }

    #[test]
    fn decimal_to_dms_and_back_is_the_same_value() {
        for value in [64.14180555, -21.9406, 0.0, 179.999] {
            let [d, m, s] = decimal_to_dms(value);
            let back = dms_to_decimal(&[d, m, s], value < 0.0, 180.0).unwrap();
            assert!(close(back, value), "{value} came back as {back}");
        }
    }

    #[test]
    fn a_plain_png_has_size_and_no_camera_data() {
        let png = graphics_imageio::encode(Format::Png, 2, 1, &[255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        let info = read(&png).unwrap();
        assert_eq!((info.format, info.width, info.height), (Format::Png, 2, 1));
        assert_eq!(info.orientation, 1);
        assert_eq!(info.camera_make, None);
        assert_eq!(info.location, None);
    }

    #[test]
    fn data_that_is_not_a_picture_has_no_info() {
        assert_eq!(read(b"not a picture at all"), None);
        assert!(decode_upright(b"not a picture at all").is_none());
    }

    #[test]
    fn decode_upright_turns_the_pixels_by_the_orientation() {
        // A 2 x 1 picture with orientation 6 (rotate 90 degrees clockwise) comes out 1 x 2 when upright.
        let rgba = [255, 0, 0, 255, 0, 0, 255, 255];
        let mut dest = bifrost_render::cg::imageio::ImageDestination::new(Format::Png, 1).unwrap();
        let img = bifrost_render::cg::image::CgImage::from_rgba8(2, 1, rgba.to_vec()).unwrap();
        dest.add_image(&img, Some(bifrost_render::cg::imageio::WriteOptions { orientation: Some(6), ..Default::default() }));
        let bytes = dest.finalize().unwrap();
        let upright = decode_upright(&bytes).unwrap();
        assert_eq!((upright.w, upright.h), (1, 2));
    }

    #[test]
    fn a_gps_time_with_a_date_reads_as_a_utc_clock_reading() {
        let mut gps = Dict::new();
        gps.insert("TimeStamp".into(), PropValue::Floats(vec![13.0, 5.0, 9.5]));
        gps.insert("DateStamp".into(), PropValue::Text("2026:10:19".into()));
        let t = gps_time(&gps).unwrap();
        assert_eq!((t.hour, t.minute, t.date), (13, 5, Some((2026, 10, 19))));
        assert_eq!(t.to_text(), "2026:10:19 13:05:09");
        gps.remove("DateStamp");
        assert_eq!(gps_time(&gps).unwrap().to_text(), "13:05:09");
        gps.insert("TimeStamp".into(), PropValue::Floats(vec![25.0, 0.0, 0.0]));
        assert_eq!(gps_time(&gps), None, "hour 25 is not a time");
        gps.insert("TimeStamp".into(), PropValue::Floats(vec![1.0, 2.0]));
        assert_eq!(gps_time(&gps), None, "two parts are not a time");
    }

    #[test]
    fn gps_speed_is_converted_to_kilometres_per_hour() {
        let mut gps = Dict::new();
        gps.insert("Speed".into(), PropValue::Float(100.0));
        assert_eq!(gps_speed_kmh(&gps), Some(100.0), "no unit means km/h");
        gps.insert("SpeedRef".into(), PropValue::Text("M".into()));
        assert!(close(gps_speed_kmh(&gps).unwrap(), 160.9344));
        gps.insert("SpeedRef".into(), PropValue::Text("N".into()));
        assert!(close(gps_speed_kmh(&gps).unwrap(), 185.2));
        gps.insert("Speed".into(), PropValue::Float(-1.0));
        assert_eq!(gps_speed_kmh(&gps), None);
    }

    #[test]
    fn a_direction_outside_a_compass_turn_is_refused() {
        let mut gps = Dict::new();
        gps.insert("ImgDirection".into(), PropValue::Float(271.5));
        assert_eq!(gps_direction(&gps), Some(271.5));
        gps.insert("ImgDirection".into(), PropValue::Float(400.0));
        assert_eq!(gps_direction(&gps), None);
    }
}
