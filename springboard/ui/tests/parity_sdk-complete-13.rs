//! Parity sdk-complete-13: headless tests for the Bluetooth MIDI picker and advertiser (the scan response that completes
//! a shortened name, the BLE MIDI packet round trip) and for background removal in subject lifting. Pure models: no
//! radio, no camera and no segmentation model runs here.

use bifrost_ui::advertising;
use bifrost_ui::midi_ble::{self, BleMessage};
use bifrost_ui::midi_ble_picker::{MidiAdvertiser, MidiPicker};
use bifrost_ui::subject_lift::{segment_background, Interaction, PixelRect, Rgba, Subject};

// ---- coreaudiokit: Bluetooth MIDI peripheral picker and advertising (bifrost_ui::midi_ble_picker) ----

#[test]
fn a_shortened_advertised_name_is_completed_by_the_scan_response() {
    let mut adv = MidiAdvertiser::new("Keyboard Pro");
    adv.set_advertising(true);
    let mut picker = MidiPicker::new();
    let advertisement = adv.payload().expect("the advertisement fits");
    assert!(picker.observe(4, advertisement.as_bytes(), -48, 0));
    assert_eq!(picker.get(4).expect("listed").label(), "Keyboard", "the advertisement holds only the shortened name");
    let response = adv.scan_response().expect("the scan response fits");
    assert!(picker.observe_scan_response(4, response.as_bytes()));
    assert_eq!(picker.get(4).expect("listed").label(), "Keyboard Pro");
}

#[test]
fn the_scan_response_holds_a_name_of_up_to_twenty_nine_bytes_and_no_longer_one() {
    let fits = "abcdefghijklmnopqrstuvwxyz123";
    let too_long = "abcdefghijklmnopqrstuvwxyz1234";
    let response = MidiAdvertiser::new(fits).scan_response().expect("fits");
    assert_eq!(response.as_bytes().len(), 31);
    assert_eq!(advertising::local_name(response.as_bytes()), Some(fits.to_string()));
    let empty = MidiAdvertiser::new(too_long).scan_response().expect("empty, not an error");
    assert!(empty.as_bytes().is_empty(), "a name longer than the scan response is left to the advertisement");
}

#[test]
fn an_unnamed_advertiser_sends_an_empty_scan_response() {
    let response = MidiAdvertiser::new("").scan_response().expect("empty");
    assert!(response.as_bytes().is_empty());
}

#[test]
fn a_scan_response_from_an_unknown_peripheral_lists_nothing() {
    let mut picker = MidiPicker::new();
    let response = MidiAdvertiser::new("Pad").scan_response().expect("fits");
    assert!(!picker.observe_scan_response(9, response.as_bytes()));
    assert!(picker.get(9).is_none());
}

#[test]
fn a_scan_response_without_a_name_keeps_the_known_name() {
    let mut picker = MidiPicker::new();
    let mut adv = MidiAdvertiser::new("Pad");
    adv.set_advertising(true);
    let advertisement = adv.payload().expect("fits");
    assert!(picker.observe(2, advertisement.as_bytes(), -60, 0));
    let empty = MidiAdvertiser::new("").scan_response().expect("empty");
    assert!(!picker.observe_scan_response(2, empty.as_bytes()));
    assert_eq!(picker.get(2).expect("listed").label(), "Pad");
}

#[test]
fn a_midi_message_survives_the_ble_packet_round_trip_with_its_timestamp() {
    let packet = midi_ble::encode(300, &[&[0x90, 0x3C, 0x64]], 185).expect("encodes");
    assert_eq!(packet, vec![0x82, 0xAC, 0x90, 0x3C, 0x64]);
    let messages = midi_ble::decode(&packet).expect("decodes");
    assert_eq!(messages, vec![BleMessage { timestamp: 300, bytes: vec![0x90, 0x3C, 0x64] }]);
}

// ---- visionkit: subject lifting and background removal (bifrost_ui::subject_lift) ----

const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// A `w` x `h` image whose pixel at `(x, y)` is `colour(x, y)`.
fn image(w: usize, h: usize, colour: impl Fn(usize, usize) -> [u8; 4]) -> Rgba {
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            pixels.extend_from_slice(&colour(x, y));
        }
    }
    Rgba::new(w, h, pixels).expect("valid image")
}

#[test]
fn background_removal_keeps_the_dark_block_and_drops_the_border_colour() {
    let img = image(7, 7, |x, y| if (2..=4).contains(&x) && (2..=4).contains(&y) { BLACK } else { WHITE });
    let mask = segment_background(&img, 30).expect("a subject");
    assert_eq!(mask.bounds(), Some(PixelRect { x: 2, y: 2, w: 3, h: 3 }));
    assert_eq!(mask.alpha_at(3, 3), Some(255));
    assert_eq!(mask.alpha_at(1, 1), Some(0));
}

#[test]
fn background_removal_drops_a_speck_that_touches_no_other_subject_pixel() {
    let img = image(7, 7, |x, y| {
        if ((2..=4).contains(&x) && (2..=4).contains(&y)) || (x, y) == (1, 1) {
            BLACK
        } else {
            WHITE
        }
    });
    let mask = segment_background(&img, 30).expect("a subject");
    assert_eq!(mask.alpha_at(1, 1), Some(0), "a diagonal speck is not 4-connected to the block");
    assert_eq!(mask.bounds(), Some(PixelRect { x: 2, y: 2, w: 3, h: 3 }));
}

#[test]
fn background_removal_treats_colours_within_the_tolerance_as_background() {
    // Grey 235 is 20 below the white border, inside a tolerance of 30. Only the black pixel is subject.
    let grey = [235, 235, 235, 255];
    let img = image(5, 5, |x, y| match (x, y) {
        (2, 2) => BLACK,
        (1, 2) => grey,
        _ => WHITE,
    });
    let mask = segment_background(&img, 30).expect("a subject");
    assert_eq!(mask.bounds(), Some(PixelRect { x: 2, y: 2, w: 1, h: 1 }));
    assert_eq!(mask.alpha_at(1, 2), Some(0));
}

#[test]
fn a_uniform_image_has_no_subject() {
    let img = image(4, 4, |_, _| WHITE);
    assert!(segment_background(&img, 30).is_none());
}

#[test]
fn a_segmented_subject_lifts_from_the_image_it_came_from() {
    let img = image(7, 7, |x, y| if (2..=4).contains(&x) && (2..=4).contains(&y) { BLACK } else { WHITE });
    let subject = Subject::new(3, segment_background(&img, 30).expect("a subject")).expect("has pixels");
    let it = Interaction::new(vec![subject]);
    let cut = it.lift(3, &img).expect("lifted");
    assert_eq!((cut.width(), cut.height()), (3, 3));
    assert_eq!(cut.pixels()[3], 255, "the centre of the block keeps full alpha");
}
