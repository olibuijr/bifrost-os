//! Parity sdk-complete-07: headless tests for picture sources, picture metadata, picture writing, animation timing,
//! drag and drop picture types and virtual audio descriptions. No test opens a device, a display or a network.

use bifrost_app::audio_virtual::{VirtualDevice, VirtualDeviceSet};
use bifrost_app::dnd::{accepts_image, image_format_of, DragItem, Payload};
use bifrost_app::image_animation::{self, Animation};
use bifrost_app::image_metadata::{self, Format};
use bifrost_app::image_source::{self, ImageSource, SourceError};
use bifrost_app::picture_writer::PictureWriter;
use std::path::Path;

fn jpeg(width: usize, height: usize, orientation: Option<u8>) -> Vec<u8> {
    let mut writer = PictureWriter::new(Format::Jpeg).quality(0.9);
    if let Some(o) = orientation {
        writer = writer.orientation(o);
    }
    assert!(writer.add_frame(width, height, [90u8, 120, 200, 255].repeat(width * height), 0));
    writer.finish().unwrap()
}

fn finish_one(mut writer: PictureWriter, width: usize, height: usize) -> Vec<u8> {
    assert!(writer.add_frame(width, height, [90u8, 120, 200, 255].repeat(width * height), 0));
    writer.finish().unwrap()
}

fn gif(delays: &[u32], loop_count: u32) -> Vec<u8> {
    let mut writer = PictureWriter::new(Format::Gif).loop_count(loop_count);
    for &delay in delays {
        assert!(writer.add_frame(2, 2, [255u8, 0, 0, 255].repeat(4), delay));
    }
    writer.finish().unwrap()
}

#[test]
fn a_file_is_read_as_bytes_and_a_file_over_the_byte_limit_is_refused_before_it_is_read() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let bytes = image_source::read_file(&manifest, 1 << 20).unwrap();
    assert!(bytes.starts_with(b"[package"));
    assert_eq!(ImageSource::new(&bytes).unwrap_err(), SourceError::NotImage);
    assert_eq!(
        image_source::read_file(&manifest, 4).unwrap_err(),
        SourceError::FileTooLarge { bytes: bytes.len() as u64 }
    );
}

#[test]
fn a_missing_file_or_a_directory_is_unreadable() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(image_source::read_file(&dir.join("no-such-picture.png"), 1024).unwrap_err(), SourceError::Unreadable);
    assert_eq!(image_source::read_file(dir, 1024).unwrap_err(), SourceError::Unreadable);
}

#[test]
fn a_picture_over_the_pixel_budget_is_refused_while_a_smaller_one_is_accepted() {
    let bytes = jpeg(60, 60, None);
    assert_eq!(
        ImageSource::with_max_pixels(&bytes, 3000).unwrap_err(),
        SourceError::TooLarge { width: 60, height: 60 }
    );
    assert_eq!(ImageSource::with_max_pixels(&bytes, 3600).unwrap().size(), (60, 60));
    assert!(ImageSource::new(&bytes).is_ok(), "the default limit has no pixel budget");
}

#[test]
fn the_thumbnail_box_follows_the_exif_orientation_of_the_picture() {
    // Stored 6 x 4 with orientation 6 (rotated a quarter turn): it is shown as 4 x 6.
    let bytes = jpeg(6, 4, Some(6));
    let src = ImageSource::new(&bytes).unwrap();
    assert_eq!(src.size(), (6, 4), "size stays the stored size");
    assert_eq!(src.upright_size(), (4, 6));
    assert_eq!(src.thumbnail_size(3, 3), Ok((2, 3)));
}

#[test]
fn a_picture_without_orientation_keeps_its_thumbnail_box_shape() {
    let bytes = jpeg(6, 4, None);
    let src = ImageSource::new(&bytes).unwrap();
    assert_eq!(src.upright_size(), (6, 4));
    assert_eq!(src.thumbnail_size(3, 3), Ok((3, 2)));
}

#[test]
fn a_thumbnail_is_turned_upright_and_fits_its_box() {
    let bytes = jpeg(6, 4, Some(6));
    let thumb = ImageSource::new(&bytes).unwrap().thumbnail(3, 3).unwrap();
    assert_eq!((thumb.w, thumb.h), (2, 3));
}

#[test]
fn decode_upright_matches_the_metadata_upright_decode_and_decode_keeps_the_stored_turn() {
    let bytes = jpeg(6, 4, Some(6));
    let src = ImageSource::new(&bytes).unwrap();
    assert_eq!((src.decode().unwrap().w, src.decode().unwrap().h), (6, 4));
    let upright = src.decode_upright().unwrap();
    assert_eq!((upright.w, upright.h), (4, 6));
    let reference = image_metadata::decode_upright(&bytes).unwrap();
    assert_eq!((reference.w, reference.h), (upright.w, upright.h));
}

#[test]
fn software_camera_and_altitude_written_to_a_picture_are_read_back() {
    let writer = PictureWriter::new(Format::Jpeg).software("Field Recorder 2").camera("Acme", "Pocket 3").altitude(-12.5);
    let bytes = finish_one(writer, 2, 2);
    let info = image_metadata::read(&bytes).unwrap();
    assert_eq!(info.software.as_deref(), Some("Field Recorder 2"));
    assert_eq!(info.camera_make.as_deref(), Some("Acme"));
    assert_eq!(info.camera_model.as_deref(), Some("Pocket 3"));
    assert_eq!(info.altitude_m, Some(-12.5));
    assert_eq!(info.lens_model, None, "the writer does not write a lens model");
}

#[test]
fn a_positive_altitude_is_above_sea_level_and_a_picture_without_one_has_none() {
    let above = finish_one(PictureWriter::new(Format::Jpeg).altitude(30.0), 2, 2);
    assert_eq!(image_metadata::read(&above).unwrap().altitude_m, Some(30.0));
    let plain = jpeg(2, 2, None);
    assert_eq!(image_metadata::read(&plain).unwrap().altitude_m, None);
}

#[test]
fn an_empty_camera_name_or_a_non_finite_altitude_is_ignored() {
    let writer = PictureWriter::new(Format::Jpeg).camera("", "").altitude(f64::NAN);
    let bytes = finish_one(writer, 2, 2);
    let info = image_metadata::read(&bytes).unwrap();
    assert_eq!((info.camera_make, info.camera_model, info.altitude_m), (None, None, None));
}

#[test]
fn a_gif_frame_is_on_screen_for_its_delay_and_the_animation_ends_after_its_loops() {
    // Delays 100 ms and 200 ms with two repeats: three passes of 300 ms, so the animation ends at 900 ms.
    let animation = image_animation::read(&gif(&[100, 200], 2)).unwrap();
    assert_eq!(animation.frame_at(0), Some(0));
    assert_eq!(animation.frame_at(99), Some(0));
    assert_eq!(animation.frame_at(100), Some(1));
    assert_eq!(animation.frame_at(299), Some(1));
    assert_eq!(animation.frame_at(300), Some(0), "the second pass starts again at the first frame");
    assert_eq!(animation.frame_at(599), Some(1));
    assert_eq!(animation.frame_at(899), Some(1));
    assert_eq!(animation.frame_at(900), None, "after the last pass the animation has ended");
}

#[test]
fn a_zero_delay_frame_is_shown_for_one_tenth_of_a_second_and_a_forever_loop_never_ends() {
    let animation = image_animation::read(&gif(&[0, 0], 0)).unwrap();
    assert_eq!(animation.duration_ms(), 200);
    assert_eq!(animation.frame_at(0), Some(0));
    assert_eq!(animation.frame_at(100), Some(1));
    assert_eq!(animation.frame_at(199), Some(1));
    assert_eq!(animation.frame_at(200), Some(0));
    assert_eq!(animation.frame_at(10_000_000), Some(0));
}

#[test]
fn a_still_picture_stays_on_its_only_frame_and_an_empty_animation_has_no_frame() {
    let png = finish_one(PictureWriter::new(Format::Png), 2, 2);
    let still = image_animation::read(&png).unwrap();
    assert_eq!(still.frame_at(123_456), Some(0));
    let empty = Animation { frames: Vec::new(), loop_count: None };
    assert_eq!(empty.frame_at(0), None);
}

#[test]
fn an_item_carries_the_picture_format_of_its_type() {
    let item = DragItem { types: vec!["public.jpeg".into()], payload: Payload::Url("https://example.org/a".into()) };
    assert_eq!(image_format_of(&item), Some(Format::Jpeg));
}

#[test]
fn a_file_item_without_a_picture_type_falls_back_to_its_extension() {
    let item = DragItem { types: vec!["public.data".into()], payload: Payload::File { grant: [1; 16], name: "Trip.PNG".into() } };
    assert_eq!(image_format_of(&item), Some(Format::Png));
}

#[test]
fn a_text_item_has_no_picture_format() {
    let item = DragItem { types: vec!["public.utf8-plain-text".into()], payload: Payload::Text("hello".into()) };
    assert_eq!(image_format_of(&item), None);
}

#[test]
fn a_window_that_takes_public_image_takes_every_picture_format() {
    assert!(accepts_image(&["public.image"], Format::Gif));
    assert!(!accepts_image(&["public.png"], Format::Jpeg));
    assert!(accepts_image(&["public.png", "public.jpeg"], Format::Jpeg));
    assert!(!accepts_image(&[], Format::Png));
}

fn loopback(uid: &str, rate: f64) -> VirtualDevice {
    VirtualDevice { uid: uid.into(), name: "Loopback".into(), channels: 2, sample_rate: rate }
}

#[test]
fn a_virtual_device_description_reads_back_to_the_same_device() {
    let device = loopback("loop-1", 44_100.0);
    assert_eq!(VirtualDevice::from_descriptor(&device.to_descriptor()), Ok(device));
}

#[test]
fn blank_and_comment_lines_in_a_description_are_skipped() {
    let text = "# loopback\n\nuid=loop-1\nname=Loopback\nchannels=2\nsample_rate=48000\n";
    assert_eq!(VirtualDevice::from_descriptor(text), Ok(loopback("loop-1", 48_000.0)));
}

#[test]
fn a_description_with_a_missing_repeated_or_unknown_key_is_refused() {
    let missing = "uid=loop-1\nname=Loopback\nsample_rate=48000\n";
    assert!(VirtualDevice::from_descriptor(missing).unwrap_err().contains("channels"));
    let repeated = "uid=a\nuid=b\nname=Loopback\nchannels=2\nsample_rate=48000\n";
    assert!(VirtualDevice::from_descriptor(repeated).is_err());
    let unknown = "uid=a\nname=Loopback\nchannels=2\nsample_rate=48000\ncolour=red\n";
    assert!(VirtualDevice::from_descriptor(unknown).is_err());
}

#[test]
fn a_description_with_a_bad_number_or_an_unsupported_rate_is_refused() {
    let words = "uid=a\nname=Loopback\nchannels=two\nsample_rate=48000\n";
    assert!(VirtualDevice::from_descriptor(words).is_err());
    let rate = "uid=a\nname=Loopback\nchannels=2\nsample_rate=47000\n";
    assert!(VirtualDevice::from_descriptor(rate).is_err());
}

#[test]
fn a_device_set_loads_several_descriptions_and_refuses_a_repeated_uid() {
    let a = "uid=a\nname=A\nchannels=1\nsample_rate=16000\n";
    let b = "uid=b\nname=B\nchannels=2\nsample_rate=48000\n";
    let set = VirtualDeviceSet::load(&[a, b]).unwrap();
    assert_eq!(set.devices().len(), 2);
    assert!(VirtualDeviceSet::load(&[a, a]).is_err());
}
