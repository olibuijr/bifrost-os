//! Parity sdk-complete-40: headless tests for the flashing light model (frame luminance, the change report) and
//! the input source selection by language, and the page thumbnail grid. No test reads a display or a device.

use bifrost_ui::flashing_lights::{frame_luminance, pixel_luminance, FlashingLights, Sample};
use bifrost_ui::input_source::{InputSource, InputSources, SourceKind};
use bifrost_ui::pdfpage::{thumbnail_grid, PageError, PdfPage, ThumbnailCell};
use bifrost_ui::Rect;

fn s(time_ms: u64, luminance: f64) -> Sample {
    Sample { time_ms, luminance }
}

fn square_wave() -> Vec<Sample> {
    (0..=10).map(|i| s(i * 100, if i % 2 == 0 { 0.0 } else { 1.0 })).collect()
}

#[test]
fn white_and_black_pixels_have_luminance_one_and_zero() {
    assert!((pixel_luminance(0xFFFF_FFFF) - 1.0).abs() < 1e-9);
    assert_eq!(pixel_luminance(0xFF00_0000), 0.0);
}

#[test]
fn the_alpha_byte_does_not_change_the_luminance() {
    assert_eq!(pixel_luminance(0x0012_3456), pixel_luminance(0xFF12_3456));
}

#[test]
fn a_mid_grey_pixel_reads_the_linear_value_of_half_the_channel() {
    // sRGB 0x80 is about 0.2159 linear, and the three channels weigh to the same value.
    assert!((pixel_luminance(0xFF80_8080) - 0.2158605).abs() < 1e-4);
}

#[test]
fn pure_green_is_the_brightest_primary_because_the_eye_weighs_it_most() {
    let red = pixel_luminance(0xFFFF_0000);
    let green = pixel_luminance(0xFF00_FF00);
    let blue = pixel_luminance(0xFF00_00FF);
    assert!((red - 0.2126).abs() < 1e-4);
    assert!((green - 0.7152).abs() < 1e-4);
    assert!((blue - 0.0722).abs() < 1e-4);
}

#[test]
fn a_frame_luminance_is_the_mean_of_its_pixels() {
    let mean = frame_luminance(&[0xFF00_0000, 0xFFFF_FFFF]).expect("a frame with pixels");
    assert!((mean - 0.5).abs() < 1e-9);
}

#[test]
fn an_empty_frame_has_no_luminance() {
    assert_eq!(frame_luminance(&[]), None);
}

#[test]
fn a_flash_sequence_is_reported_once_when_it_becomes_flagged() {
    let mut lights = FlashingLights::new();
    assert_eq!(lights.analyse_change(&square_wave()), Some(true));
    assert_eq!(lights.analyse_change(&square_wave()), None, "the same state is not reported again");
    assert!(lights.is_flagged());
}

#[test]
fn a_calm_sequence_after_a_flagged_one_reports_the_clearing() {
    let mut lights = FlashingLights::new();
    lights.analyse_change(&square_wave());
    let calm = [s(0, 0.2), s(100, 0.25)];
    assert_eq!(lights.analyse_change(&calm), Some(false));
    assert!(!lights.is_flagged());
}

#[test]
fn a_calm_picture_never_reports_a_change() {
    let mut lights = FlashingLights::new();
    assert_eq!(lights.analyse_change(&[s(0, 0.5), s(100, 0.5)]), None);
}

fn two_layouts() -> InputSources {
    let mut sources = InputSources::new();
    sources.install(InputSource::new("keylayout.US", "U.S.", SourceKind::KeyboardLayout, &["en"]));
    sources.install(InputSource::new("keylayout.Icelandic", "Icelandic", SourceKind::KeyboardLayout, &["is"]));
    sources.install(InputSource::new("mode.Japanese.Romaji", "Romaji", SourceKind::InputMode, &["ja"]));
    sources
}

#[test]
fn selecting_for_a_language_picks_the_enabled_source_that_types_it() {
    let mut sources = two_layouts();
    sources.enable("keylayout.Icelandic").expect("installed");
    assert_eq!(sources.select_for_language("is-IS"), Some("keylayout.Icelandic".to_string()));
    assert_eq!(sources.current().map(|s| s.id.as_str()), Some("keylayout.Icelandic"));
}

#[test]
fn a_language_with_no_enabled_source_leaves_the_selection_alone() {
    let mut sources = two_layouts();
    assert_eq!(sources.select_for_language("de"), None);
    assert_eq!(sources.current().map(|s| s.id.as_str()), Some("keylayout.US"));
}

#[test]
fn a_source_that_is_installed_but_not_enabled_is_not_chosen() {
    let mut sources = two_layouts();
    assert_eq!(sources.select_for_language("ja"), None, "the Japanese mode is installed but not enabled");
    sources.enable("mode.Japanese.Romaji").expect("installed");
    assert_eq!(sources.select_for_language("ja"), Some("mode.Japanese.Romaji".to_string()));
}

fn letter_page() -> PdfPage {
    PdfPage::new(Rect { x: 0.0, y: 0.0, w: 612.0, h: 792.0 }).expect("a letter page")
}

fn landscape_page() -> PdfPage {
    PdfPage::from_image_size(200.0, 100.0).expect("a landscape page")
}

#[test]
fn a_thumbnail_grid_fits_as_many_columns_as_the_width_allows() {
    // Cells of 100 points with a 10 point gap: 400 points hold three columns, since 3 * 110 - 10 = 320 fits and 4 * 110 - 10 = 430 does not.
    let pages = vec![letter_page(), letter_page(), letter_page(), letter_page()];
    let cells = thumbnail_grid(&pages, 400.0, (100.0, 100.0), 10.0).expect("a valid grid");
    assert_eq!(cells.len(), 4);
    assert_eq!(cells[3].page, 3);
    assert!((cells[3].rect.y - 110.0 - 0.0).abs() < 1e-3, "the fourth page starts the second row");
}

#[test]
fn a_portrait_thumbnail_is_centred_horizontally_in_its_cell() {
    let cells = thumbnail_grid(&[letter_page()], 400.0, (100.0, 100.0), 10.0).expect("a valid grid");
    let r = cells[0].rect;
    // A letter page fits 100 by 100 at 77.27 by 100 (the height limits it).
    assert!((r.h - 100.0).abs() < 1e-3);
    assert!((r.w - 77.27).abs() < 0.01);
    assert!((r.x - (100.0 - r.w) / 2.0).abs() < 1e-3);
    assert!(r.y.abs() < 1e-3);
}

#[test]
fn a_landscape_thumbnail_is_centred_vertically_in_its_cell() {
    let cells = thumbnail_grid(&[landscape_page()], 400.0, (100.0, 100.0), 10.0).expect("a valid grid");
    let r = cells[0].rect;
    assert!((r.w - 100.0).abs() < 1e-3);
    assert!((r.h - 50.0).abs() < 1e-3);
    assert!((r.y - 25.0).abs() < 1e-3);
    assert!(r.x.abs() < 1e-3);
}

#[test]
fn an_empty_document_has_no_thumbnails() {
    assert_eq!(thumbnail_grid(&[], 400.0, (100.0, 100.0), 10.0), Ok(Vec::<ThumbnailCell>::new()));
}

#[test]
fn a_grid_refuses_an_area_too_narrow_for_one_cell_and_a_zero_size() {
    assert_eq!(thumbnail_grid(&[letter_page()], 50.0, (100.0, 100.0), 10.0), Err(PageError::BadSize));
    assert_eq!(thumbnail_grid(&[letter_page()], 400.0, (0.0, 100.0), 10.0), Err(PageError::BadSize));
    assert_eq!(thumbnail_grid(&[letter_page()], 400.0, (100.0, 100.0), -1.0), Err(PageError::BadSize));
}
