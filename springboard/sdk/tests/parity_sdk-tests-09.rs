//! Parity tests for the preview row (sdk-tests-09): a view is rendered headless, offscreen, into an in-memory pixel
//! buffer and inspected without running the shell or an app process. The tests assert buffer sizes, pixel output and
//! accessibility elements, with a pinned clock.

use bifrost_app::testing::{pin, unix_secs, TestWindow};
use bifrost_app::view::{Button, Label, Stack};
use bifrost_core::pin::PINNED_UNIX_SECS;
use bifrost_ui::responder::Responder;
use bifrost_ui::Role;

use bifrost_app::graphics_pdf::{Document, PdfWriter};
use bifrost_app::review::{Policy, ReviewRequests};
use bifrost_app::screen_time::{Configuration, ConfigurationObserver};

#[derive(Default)]
struct Model {
    actions: Vec<String>,
}

impl Responder for Model {
    fn perform(&mut self, action: &str) -> bool {
        self.actions.push(action.into());
        true
    }
}

fn button_window(w: u32, h: u32) -> TestWindow<Model> {
    let root = Stack::vstack(8.0, vec![Box::new(Button::new("Save", "save"))]);
    TestWindow::new(root, Model::default(), w, h)
}

fn empty_window(w: u32, h: u32) -> TestWindow<Model> {
    TestWindow::new(Stack::vstack(8.0, Vec::new()), Model::default(), w, h)
}

#[test]
fn a_headless_preview_renders_the_view_into_a_pixel_buffer_of_the_window_size() {
    let mut w = button_window(320, 200);
    let px = w.draw();
    assert_eq!(px.len(), 320 * 200, "one packed pixel per window point");
}

#[test]
fn the_offscreen_render_of_a_button_differs_from_an_empty_window() {
    let with_button = button_window(320, 200).draw();
    let without = empty_window(320, 200).draw();
    assert_ne!(with_button, without, "the button leaves pixels behind");
}

#[test]
fn the_same_view_renders_the_same_pixels_twice() {
    let mut w = button_window(160, 90);
    let first = w.draw();
    let second = w.draw();
    assert_eq!(first, second, "a headless render of an unchanged view is deterministic");
}

#[test]
fn resizing_the_preview_resizes_the_pixel_buffer() {
    let mut w = button_window(320, 200);
    w.resize(200, 120);
    assert_eq!(w.draw().len(), 200 * 120);
}

#[test]
fn a_preview_exposes_its_accessibility_elements_without_drawing_them() {
    let mut w = button_window(320, 200);
    assert!(w.exists(Role::Button, "Save"));
    assert!(!w.exists(Role::Button, "Cancel"));
    assert_eq!(w.elements().iter().filter(|n| n.role == Role::Button).count(), 1);
}

#[test]
fn a_headless_preview_of_a_label_keeps_its_text_out_of_the_responder() {
    let root = Stack::vstack(8.0, vec![Box::new(Label::new("Hello"))]);
    let mut w = TestWindow::new(root, Model::default(), 120, 40);
    assert_eq!(w.draw().len(), 120 * 40);
    assert!(w.model.actions.is_empty(), "drawing performs no action");
}

#[test]
fn a_pinned_preview_reads_the_pinned_clock() {
    pin();
    assert_eq!(unix_secs(), PINNED_UNIX_SECS);
}

// --- PDF documents and writer (graphics_pdf) -----------------------------------------------------------------------
// Headless: each PDF is written to memory, read back and rendered into a pixel buffer. No file, window or device.

fn centre(img: &bifrost_render::gfx::Image) -> u32 {
    img.px[(img.h / 2) * img.w + img.w / 2]
}

#[test]
fn headless_a_writer_keeps_every_page_size_in_order() {
    let mut w = PdfWriter::new();
    w.add_page(612.0, 792.0);
    w.add_page(300.0, 200.0).stroke_line(0.0, 0.0, 300.0, 200.0, 1.0, [0.0, 0.0, 0.0]);
    w.add_page(100.0, 100.0);
    assert_eq!(w.page_count(), 3);
    let doc = Document::open(w.finish()).expect("reads back");
    assert_eq!(doc.page_count(), 3);
    assert_eq!(doc.page_size(0), Some((612.0, 792.0)));
    assert_eq!(doc.page_size(2), Some((100.0, 100.0)));
    assert_eq!(doc.page_size(3), None);
}

#[test]
fn headless_a_filled_page_renders_its_colour_at_the_centre_of_the_pixel_buffer() {
    let mut w = PdfWriter::new();
    w.add_page(40.0, 40.0).fill_rect(0.0, 0.0, 40.0, 40.0, [0.0, 0.0, 1.0]);
    let img = Document::open(w.finish()).expect("reads back").render(0, 40, 40).expect("renders");
    assert_eq!((img.w, img.h), (40, 40));
    let px = centre(&img);
    assert_eq!(px & 0xff, 0, "no red in a blue fill");
    assert_eq!(px >> 16 & 0xff, 255, "full blue");
}

#[test]
fn headless_a_wide_page_renders_into_the_box_at_its_own_shape() {
    let mut w = PdfWriter::new();
    w.add_page(200.0, 100.0).fill_rect(0.0, 0.0, 200.0, 100.0, [1.0, 1.0, 1.0]);
    let img = Document::open(w.finish()).expect("reads back").render(0, 50, 50).expect("renders");
    assert_eq!((img.w, img.h), (50, 25));
}

#[test]
fn headless_a_page_without_drawing_renders_to_its_full_size() {
    let mut w = PdfWriter::new();
    w.add_page(120.0, 60.0);
    let img = Document::open(w.finish()).expect("reads back").render(0, 120, 60).expect("renders");
    assert_eq!((img.w, img.h), (120, 60));
}

#[test]
fn headless_a_stroked_line_darkens_the_pixel_it_crosses() {
    let mut w = PdfWriter::new();
    let page = w.add_page(100.0, 100.0);
    page.fill_rect(0.0, 0.0, 100.0, 100.0, [1.0, 1.0, 1.0]);
    page.stroke_line(0.0, 50.0, 100.0, 50.0, 4.0, [0.0, 0.0, 0.0]);
    let img = Document::open(w.finish()).expect("reads back").render(0, 100, 100).expect("renders");
    assert_eq!(centre(&img) & 0xff, 0, "the black line covers the middle row");
    assert_eq!(img.px[5 * img.w + 5] & 0xff, 255, "the white page shows in the corner");
}

#[test]
fn headless_a_drawn_raster_image_fills_its_rectangle() {
    let mut w = PdfWriter::new();
    w.add_page(100.0, 100.0).draw_rgb_image(vec![255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0], 2, 2, 0.0, 0.0, 100.0, 100.0);
    let img = Document::open(w.finish()).expect("reads back").render(0, 100, 100).expect("renders");
    let px = centre(&img);
    assert!(px & 0xff > 200 && px >> 8 & 0xff < 50, "a red picture paints red: {px:08x}");
}

#[test]
fn headless_out_of_range_colours_are_clamped_before_they_are_drawn() {
    let mut w = PdfWriter::new();
    w.add_page(10.0, 10.0).fill_rect(0.0, 0.0, 10.0, 10.0, [2.0, -1.0, f64::NAN]);
    let img = Document::open(w.finish()).expect("reads back").render(0, 10, 10).expect("renders");
    let px = centre(&img);
    assert_eq!((px & 0xff, px >> 8 & 0xff, px >> 16 & 0xff), (255, 0, 0), "above one is full, below zero is none, NaN is zero");
}

#[test]
fn headless_an_empty_writer_gives_a_document_that_renders_no_page() {
    let doc = Document::open(PdfWriter::new().finish()).expect("reads back");
    assert_eq!(doc.page_count(), 0);
    assert!(doc.render(0, 10, 10).is_err());
}

#[test]
fn headless_bytes_that_are_not_a_pdf_are_refused_by_the_document() {
    assert!(Document::open(b"%PNG not a document".to_vec()).is_err());
}

// --- review prompt policy (review) ----------------------------------------------------------------------------------

#[test]
fn headless_the_default_policy_is_a_120_day_gap_and_three_asks_a_year() {
    assert_eq!(Policy::default(), Policy { min_gap: 120, max_asks: 3, window: 365 });
}

#[test]
fn headless_the_gap_counts_from_the_last_ask() {
    let r = ReviewRequests::from_days(vec![0, 120]);
    let p = Policy::default();
    assert!(!r.allowed(239, p), "119 days after the last ask");
    assert!(r.allowed(240, p), "exactly 120 days after the last ask");
}

#[test]
fn headless_an_ask_leaves_the_year_window_on_the_anniversary_day() {
    let r = ReviewRequests::from_days(vec![0, 120, 240]);
    let p = Policy::default();
    assert!(!r.allowed(364, p), "three asks are still inside the year");
    assert!(r.allowed(365, p), "the ask on day 0 is 365 days old");
}

#[test]
fn headless_checking_an_ask_does_not_record_it() {
    let r = ReviewRequests::new();
    assert!(r.allowed(5, Policy::default()));
    assert!(r.days().is_empty());
}

#[test]
fn headless_days_before_the_epoch_follow_the_same_rules() {
    let mut r = ReviewRequests::new();
    let p = Policy::default();
    assert!(r.request(-400, p));
    assert!(!r.request(-300, p), "100 days is less than the gap");
    assert_eq!(r.days(), &[-400]);
}

// --- screen time configuration (screen_time) -------------------------------------------------------------------------

#[test]
fn headless_a_key_that_only_shares_the_limit_prefix_enforces_nothing() {
    assert!(!Configuration::from_settings_text("limits.x=5\n").enforces_child_restrictions);
}

#[test]
fn headless_one_positive_limit_enforces_even_beside_zero_limits() {
    assert!(Configuration::from_settings_text("limit.a=0\nlimit.b=0\nlimit.c=2\n").enforces_child_restrictions);
}

fn stamp(path: &std::path::Path, secs: u64) {
    let file = std::fs::OpenOptions::new().write(true).open(path).expect("the settings file exists");
    file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)).expect("the time is set");
}

#[test]
fn headless_the_observer_reports_a_real_change_once() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("screentime.ini");
    std::fs::write(&path, "downtime.on=1\n").unwrap();
    stamp(&path, 1_000_000_000);

    let mut obs = ConfigurationObserver::new(&path);
    assert!(obs.start_observing().enforces_child_restrictions, "the file read at start enforces downtime");
    assert!(obs.poll().is_none(), "nothing changed since the start");

    std::fs::write(&path, "downtime.on=0\nlimit.files=0\n").unwrap();
    stamp(&path, 2_000_000_000);
    let changed = obs.poll().expect("the restrictions were lifted");
    assert!(!changed.enforces_child_restrictions);
    assert_eq!(obs.configuration(), changed);
    assert!(obs.poll().is_none(), "the same change is reported once");
}

#[test]
fn headless_an_edit_that_changes_no_restriction_is_not_reported() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("screentime.ini");
    std::fs::write(&path, "limit.files=30\n").unwrap();
    stamp(&path, 1_000_000_000);
    let mut obs = ConfigurationObserver::new(&path);
    obs.start_observing();

    std::fs::write(&path, "limit.files=45\n").unwrap();
    stamp(&path, 2_000_000_000);
    assert!(obs.poll().is_none(), "the limit changed but enforcement did not");
    assert!(obs.configuration().enforces_child_restrictions);
}

#[test]
fn headless_a_stopped_observer_reports_nothing_until_it_starts_again() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("screentime.ini");
    let mut obs = ConfigurationObserver::new(&path);
    assert!(!obs.start_observing().enforces_child_restrictions, "a missing file enforces nothing");
    obs.stop_observing();
    std::fs::write(&path, "downtime.on=1\n").unwrap();
    stamp(&path, 1_000_000_000);
    assert!(obs.poll().is_none());
    assert!(obs.start_observing().enforces_child_restrictions, "starting again reads the file");
}
