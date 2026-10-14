//! Headless host tests for the PDF page, PDF view and page annotation models: `bifrost_ui::pdfpage`,
//! `bifrost_ui::pdfview` and `bifrost_ui::annotation`. Each model is pure data (no display, no device, no network), so
//! every test runs headless and asserts concrete geometry or state.
use bifrost_ui::annotation::{Annotation, PageAnnotations, Subtype};
use bifrost_ui::pdfpage::{normalize_rotation, DisplayBox, PageBoxes, PageError, PdfPage};
use bifrost_ui::pdfview::{DisplayMode, PageRect, PdfView, ViewError, DEFAULT_MAX_SCALE, DEFAULT_MIN_SCALE, DEFAULT_PAGE_GAP};
use bifrost_ui::Rect;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

fn dims(r: Rect) -> (f32, f32, f32, f32) {
    (r.x, r.y, r.w, r.h)
}

fn letter() -> PdfPage {
    PdfPage::new(rect(0.0, 0.0, 612.0, 792.0)).unwrap()
}

// --- pdfpage: boxes, rotation and placement --------------------------------------------------------------------------

#[test]
fn unset_boxes_fall_back_along_the_chain_to_the_media_box() {
    let page = letter();
    let media = dims(rect(0.0, 0.0, 612.0, 792.0));
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Crop)), media);
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Bleed)), media);
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Trim)), media);
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Art)), media);
}

#[test]
fn setting_the_crop_box_moves_the_boxes_that_fall_back_to_it() {
    let mut page = letter();
    let crop = rect(10.0, 20.0, 400.0, 500.0);
    page.boxes_mut().set_bounds(crop, DisplayBox::Crop).unwrap();
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Crop)), dims(crop));
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Trim)), dims(crop));
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Bleed)), dims(crop));
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Art)), dims(crop));
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Media)), (0.0, 0.0, 612.0, 792.0));
}

#[test]
fn a_box_set_on_its_own_does_not_change_the_box_it_falls_back_to() {
    let mut page = letter();
    page.boxes_mut().set_bounds(rect(5.0, 5.0, 100.0, 100.0), DisplayBox::Trim).unwrap();
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Trim)), (5.0, 5.0, 100.0, 100.0));
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Crop)), (0.0, 0.0, 612.0, 792.0));
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Art)), (0.0, 0.0, 612.0, 792.0));
}

#[test]
fn boxes_and_pages_refuse_zero_negative_and_non_finite_sizes() {
    assert!(matches!(PageBoxes::new(rect(0.0, 0.0, 0.0, 10.0)), Err(PageError::BadSize)));
    assert!(matches!(PageBoxes::new(rect(0.0, 0.0, f32::NAN, 10.0)), Err(PageError::BadSize)));
    assert!(matches!(PdfPage::new(rect(0.0, 0.0, 10.0, -1.0)), Err(PageError::BadSize)));
    assert!(matches!(PdfPage::from_image_size(-1.0, 5.0), Err(PageError::BadSize)));

    let mut page = letter();
    let refused = page.boxes_mut().set_bounds(rect(0.0, 0.0, -2.0, 5.0), DisplayBox::Crop);
    assert!(matches!(refused, Err(PageError::BadSize)));
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Crop)), (0.0, 0.0, 612.0, 792.0));
}

#[test]
fn rotation_is_normalised_to_a_quarter_turn_and_odd_angles_are_refused() {
    assert_eq!(normalize_rotation(-90), Ok(270));
    assert_eq!(normalize_rotation(450), Ok(90));
    assert_eq!(normalize_rotation(-180), Ok(180));
    assert_eq!(normalize_rotation(0), Ok(0));
    assert_eq!(normalize_rotation(45), Err(PageError::BadRotation));

    let mut page = letter();
    page.set_rotation(-90).unwrap();
    assert_eq!(page.rotation(), 270);
    assert_eq!(page.set_rotation(30), Err(PageError::BadRotation));
    assert_eq!(page.rotation(), 270);
}

#[test]
fn displayed_size_swaps_width_and_height_on_quarter_turns() {
    let mut page = letter();
    assert_eq!(page.displayed_size(DisplayBox::Media), (612.0, 792.0));
    page.set_rotation(90).unwrap();
    assert_eq!(page.displayed_size(DisplayBox::Media), (792.0, 612.0));
    page.set_rotation(180).unwrap();
    assert_eq!(page.displayed_size(DisplayBox::Media), (612.0, 792.0));
    page.set_rotation(270).unwrap();
    assert_eq!(page.displayed_size(DisplayBox::Media), (792.0, 612.0));
}

#[test]
fn transform_maps_the_box_corners_onto_the_displayed_box_for_each_rotation() {
    let mut page = letter();
    let t = page.transform_for(DisplayBox::Media);
    assert_eq!(t.apply(0.0, 0.0), (0.0, 0.0));
    assert_eq!(t.apply(612.0, 792.0), (612.0, 792.0));

    page.set_rotation(90).unwrap();
    let t = page.transform_for(DisplayBox::Media);
    assert_eq!(t.apply(0.0, 0.0), (0.0, 612.0));
    assert_eq!(t.apply(612.0, 0.0), (0.0, 0.0));
    assert_eq!(t.apply(612.0, 792.0), (792.0, 0.0));

    page.set_rotation(180).unwrap();
    let t = page.transform_for(DisplayBox::Media);
    assert_eq!(t.apply(0.0, 0.0), (612.0, 792.0));
    assert_eq!(t.apply(612.0, 792.0), (0.0, 0.0));

    page.set_rotation(270).unwrap();
    let t = page.transform_for(DisplayBox::Media);
    assert_eq!(t.apply(0.0, 0.0), (792.0, 0.0));
    assert_eq!(t.apply(0.0, 792.0), (0.0, 0.0));
    assert_eq!(t.apply(612.0, 0.0), (792.0, 612.0));
}

#[test]
fn transform_starts_at_the_origin_corner_of_an_offset_box() {
    let mut page = letter();
    page.boxes_mut().set_bounds(rect(100.0, 50.0, 300.0, 400.0), DisplayBox::Crop).unwrap();
    let t = page.transform_for(DisplayBox::Crop);
    assert_eq!(t.apply(100.0, 50.0), (0.0, 0.0));
    assert_eq!(t.apply(400.0, 450.0), (300.0, 400.0));
}

#[test]
fn fit_scale_keeps_the_aspect_ratio_inside_the_target() {
    let page = letter();
    assert_eq!(page.fit_scale(DisplayBox::Media, (306.0, 396.0)), Ok(0.5));
    assert_eq!(page.thumbnail_size(DisplayBox::Media, (306.0, 396.0)), Ok((306.0, 396.0)));
    assert_eq!(page.fit_scale(DisplayBox::Media, (0.0, 10.0)), Err(PageError::BadSize));
    assert_eq!(page.fit_scale(DisplayBox::Media, (f32::NAN, 10.0)), Err(PageError::BadSize));
}

#[test]
fn a_rotated_thumbnail_fits_its_landscape_size_inside_a_square() {
    let mut page = letter();
    page.set_rotation(90).unwrap();
    let (w, h) = page.thumbnail_size(DisplayBox::Media, (100.0, 100.0)).unwrap();
    assert!((w - 100.0).abs() < 1e-3, "width {w}");
    assert!((h - 612.0 * 100.0 / 792.0).abs() < 1e-3, "height {h}");
}

#[test]
fn fit_transform_centres_the_page_in_the_target() {
    let page = PdfPage::from_image_size(200.0, 100.0).unwrap();
    let t = page.fit_transform(DisplayBox::Media, (100.0, 100.0)).unwrap();
    assert_eq!(t.apply(0.0, 0.0), (0.0, 25.0));
    assert_eq!(t.apply(200.0, 100.0), (100.0, 75.0));

    let wide = page.fit_transform(DisplayBox::Media, (300.0, 100.0)).unwrap();
    assert_eq!(wide.apply(0.0, 0.0), (50.0, 0.0));
    assert_eq!(wide.apply(200.0, 100.0), (250.0, 100.0));
}

#[test]
fn an_image_page_is_as_large_as_its_image_at_the_origin() {
    let page = PdfPage::from_image_size(640.0, 480.0).unwrap();
    assert_eq!(dims(page.boxes().bounds(DisplayBox::Media)), (0.0, 0.0, 640.0, 480.0));
    assert_eq!(page.displayed_size(DisplayBox::Media), (640.0, 480.0));
    assert_eq!(page.rotation(), 0);
}

// --- pdfview: layout, scroll, scale and navigation ---------------------------------------------------------------------

fn three_pages() -> PdfView {
    let mut view = PdfView::new(400.0, 300.0);
    view.set_pages(&[(100.0, 200.0); 3]).unwrap();
    view
}

#[test]
fn a_new_view_is_single_page_continuous_at_scale_one_with_default_limits() {
    let view = PdfView::new(400.0, 300.0);
    assert_eq!(view.display_mode(), DisplayMode::SinglePageContinuous);
    assert_eq!(view.scale_factor(), 1.0);
    assert!(!view.auto_scales());
    assert_eq!(view.scale_limits(), (DEFAULT_MIN_SCALE, DEFAULT_MAX_SCALE));
    assert_eq!(view.page_gap(), DEFAULT_PAGE_GAP);
    assert!(DisplayMode::SinglePageContinuous.is_continuous());
    assert!(!DisplayMode::TwoUp.is_continuous());
    assert_eq!(DisplayMode::TwoUp.pages_per_row(), 2);
    assert_eq!(DisplayMode::SinglePage.pages_per_row(), 1);
}

#[test]
fn continuous_pages_stack_one_per_row_with_the_page_gap_between_rows() {
    let view = three_pages();
    assert_eq!(view.content_size(), (400.0, 620.0));
    assert_eq!(view.max_scroll_y(), 320.0);
    let frames = view.page_frames();
    assert_eq!(frames.len(), 3);
    assert_eq!((frames[0].index, frames[0].rect), (0, PageRect { x: 150.0, y: 0.0, w: 100.0, h: 200.0 }));
    assert_eq!((frames[1].index, frames[1].rect), (1, PageRect { x: 150.0, y: 210.0, w: 100.0, h: 200.0 }));
    assert_eq!((frames[2].index, frames[2].rect), (2, PageRect { x: 150.0, y: 420.0, w: 100.0, h: 200.0 }));
}

#[test]
fn two_up_continuous_pairs_pages_and_centres_each_row() {
    let mut view = three_pages();
    view.set_display_mode(DisplayMode::TwoUpContinuous);
    assert_eq!(view.content_size(), (400.0, 410.0));
    let frames = view.page_frames();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].rect, PageRect { x: 95.0, y: 0.0, w: 100.0, h: 200.0 });
    assert_eq!(frames[1].rect, PageRect { x: 205.0, y: 0.0, w: 100.0, h: 200.0 });
    assert_eq!((frames[2].index, frames[2].rect), (2, PageRect { x: 150.0, y: 210.0, w: 100.0, h: 200.0 }));
}

#[test]
fn visible_pages_are_the_frames_that_meet_the_viewport() {
    let mut view = three_pages();
    assert_eq!(view.visible_pages(), vec![0, 1]);
    view.scroll_to(300.0);
    assert_eq!(view.scroll_y(), 300.0);
    assert_eq!(view.visible_pages(), vec![1, 2]);
}

#[test]
fn scroll_is_clamped_to_the_content_and_ignores_non_finite_values() {
    let mut view = three_pages();
    view.scroll_to(9999.0);
    assert_eq!(view.scroll_y(), 320.0);
    view.scroll_to(-5.0);
    assert_eq!(view.scroll_y(), 0.0);
    view.scroll_to(100.0);
    view.scroll_to(f64::NAN);
    assert_eq!(view.scroll_y(), 100.0);
}

#[test]
fn current_page_is_the_row_that_crosses_the_middle_of_the_viewport() {
    let mut view = three_pages();
    assert_eq!(view.current_page(), Some(0));
    view.scroll_to(190.0);
    assert_eq!(view.current_page(), Some(1));
}

#[test]
fn go_to_page_scrolls_the_row_to_the_top_and_refuses_unknown_pages() {
    let mut view = three_pages();
    view.go_to_page(1).unwrap();
    assert_eq!(view.scroll_y(), 210.0);
    assert_eq!(view.current_page(), Some(1));
    view.go_to_page(2).unwrap();
    assert_eq!(view.scroll_y(), 320.0);
    assert_eq!(view.current_page(), Some(2));
    assert_eq!(view.go_to_page(3), Err(ViewError::NoSuchPage));
    assert_eq!(view.current_page(), Some(2));
}

#[test]
fn next_and_previous_page_step_through_the_document_and_stop_at_the_ends() {
    let mut view = three_pages();
    assert!(!view.previous_page());
    assert!(view.next_page());
    assert_eq!(view.current_page(), Some(1));
    assert!(view.next_page());
    assert_eq!(view.current_page(), Some(2));
    assert!(!view.next_page());
    assert!(view.previous_page());
    assert_eq!(view.current_page(), Some(1));
}

#[test]
fn a_fixed_scale_scales_the_frames_and_the_content() {
    let mut view = three_pages();
    view.set_scale_factor(2.0);
    assert_eq!(view.scale_factor(), 2.0);
    assert!(!view.auto_scales());
    assert_eq!(view.content_size(), (400.0, 1240.0));
    assert_eq!(view.max_scroll_y(), 940.0);
    assert_eq!(view.page_frames()[1].rect, PageRect { x: 100.0, y: 420.0, w: 200.0, h: 400.0 });
}

#[test]
fn a_fixed_scale_is_clamped_into_the_scale_limits() {
    let mut view = three_pages();
    view.set_scale_factor(100.0);
    assert_eq!(view.scale_factor(), DEFAULT_MAX_SCALE);
    view.set_scale_factor(0.01);
    assert_eq!(view.scale_factor(), DEFAULT_MIN_SCALE);
}

#[test]
fn new_scale_limits_clamp_the_current_scale_and_bad_limits_are_refused() {
    let mut view = three_pages();
    view.set_scale_factor(4.0);
    view.set_scale_limits(0.5, 2.0).unwrap();
    assert_eq!(view.scale_factor(), 2.0);
    assert_eq!(view.scale_limits(), (0.5, 2.0));
    assert_eq!(view.set_scale_limits(2.0, 1.0), Err(ViewError::BadScale));
    assert_eq!(view.set_scale_limits(0.0, 1.0), Err(ViewError::BadScale));
    assert_eq!(view.set_scale_limits(f64::NAN, 1.0), Err(ViewError::BadScale));
    assert_eq!(view.scale_limits(), (0.5, 2.0));
}

#[test]
fn autoscale_fits_the_widest_row_to_the_viewport_width() {
    let mut view = three_pages();
    view.set_display_mode(DisplayMode::TwoUpContinuous);
    view.set_auto_scales(true);
    let s = view.effective_scale();
    assert!((s - 400.0 / 210.0).abs() < 1e-9, "scale {s}");
    let frames = view.page_frames();
    assert!((frames[0].rect.x - 0.0).abs() < 1e-9);
    assert!((frames[1].rect.x - 110.0 * 400.0 / 210.0).abs() < 1e-9);
    assert!((frames[0].rect.w - 100.0 * 400.0 / 210.0).abs() < 1e-9);
}

#[test]
fn autoscale_respects_the_upper_scale_limit() {
    let mut view = three_pages();
    view.set_scale_limits(DEFAULT_MIN_SCALE, 1.0).unwrap();
    view.set_auto_scales(true);
    assert_eq!(view.effective_scale(), 1.0);
    view.set_scale_factor(0.5);
    assert!(!view.auto_scales());
    assert_eq!(view.effective_scale(), 0.5);
}

#[test]
fn single_page_modes_lay_out_only_the_current_row() {
    let mut view = three_pages();
    view.set_display_mode(DisplayMode::SinglePage);
    view.go_to_page(2).unwrap();
    let frames = view.page_frames();
    assert_eq!(frames.len(), 1);
    assert_eq!((frames[0].index, frames[0].rect), (2, PageRect { x: 150.0, y: 0.0, w: 100.0, h: 200.0 }));
    assert_eq!(view.visible_pages(), vec![2]);
    assert_eq!(view.content_size(), (400.0, 200.0));
    view.scroll_to(50.0);
    assert_eq!(view.scroll_y(), 0.0);
}

#[test]
fn two_up_single_row_mode_lays_out_the_row_of_the_current_page() {
    let mut view = three_pages();
    view.set_display_mode(DisplayMode::TwoUp);
    view.go_to_page(2).unwrap();
    let frames = view.page_frames();
    assert_eq!(frames.len(), 1);
    assert_eq!((frames[0].index, frames[0].rect), (2, PageRect { x: 150.0, y: 0.0, w: 100.0, h: 200.0 }));
}

#[test]
fn changing_the_display_mode_keeps_the_current_page() {
    let mut view = three_pages();
    view.go_to_page(1).unwrap();
    view.set_display_mode(DisplayMode::SinglePage);
    assert_eq!(view.current_page(), Some(1));
    view.set_display_mode(DisplayMode::TwoUpContinuous);
    assert_eq!(view.scroll_y(), 0.0);
    assert_eq!(view.current_page(), Some(0));
}

#[test]
fn set_pages_resets_the_position_and_refuses_bad_sizes() {
    let mut view = three_pages();
    view.go_to_page(2).unwrap();
    view.set_pages(&[(50.0, 50.0)]).unwrap();
    assert_eq!(view.page_count(), 1);
    assert_eq!(view.current_page(), Some(0));
    assert_eq!(view.scroll_y(), 0.0);
    assert_eq!(view.set_pages(&[(0.0, 50.0)]), Err(ViewError::BadSize));
    assert_eq!(view.page_count(), 1);
}

#[test]
fn an_empty_view_has_no_pages_and_no_current_page() {
    let mut view = PdfView::new(400.0, 300.0);
    assert_eq!(view.page_count(), 0);
    assert_eq!(view.current_page(), None);
    assert!(view.page_frames().is_empty());
    assert!(view.visible_pages().is_empty());
    assert_eq!(view.content_size(), (400.0, 300.0));
    assert!(!view.next_page());
    assert!(!view.previous_page());
    assert_eq!(view.go_to_page(0), Err(ViewError::NoSuchPage));
}

#[test]
fn page_gap_is_clamped_to_zero_for_negative_or_non_finite_values() {
    let mut view = three_pages();
    view.set_page_gap(-5.0);
    assert_eq!(view.page_gap(), 0.0);
    view.set_page_gap(f64::NAN);
    assert_eq!(view.page_gap(), 0.0);
    view.set_page_gap(12.0);
    assert_eq!(view.page_gap(), 12.0);
    assert_eq!(view.content_size(), (400.0, 624.0));
}

#[test]
fn shrinking_the_viewport_clamps_the_scroll_position() {
    let mut view = three_pages();
    view.scroll_to(300.0);
    view.set_view_size(400.0, 1000.0);
    assert_eq!(view.max_scroll_y(), 0.0);
    assert_eq!(view.scroll_y(), 0.0);
}

// --- annotation: stacking, hit testing and visibility ------------------------------------------------------------------

fn highlight_at(x: f32, y: f32) -> Annotation {
    Annotation::new(Subtype::Highlight, rect(x, y, 40.0, 20.0))
}

#[test]
fn a_new_annotation_has_no_contents_an_opaque_black_colour_and_no_drawing() {
    let a = Annotation::new(Subtype::Stamp, rect(1.0, 2.0, 3.0, 4.0));
    assert_eq!(a.subtype, Subtype::Stamp);
    assert_eq!(a.bounds, rect(1.0, 2.0, 3.0, 4.0));
    assert_eq!(a.contents, "");
    assert_eq!(a.color, 0xff00_0000);
    assert!(a.drawing.is_none());
}

#[test]
fn a_hit_returns_the_topmost_annotation_whose_bounds_contain_the_point() {
    let mut page = PageAnnotations::new();
    let bottom = page.add(highlight_at(0.0, 0.0));
    let top = page.add(highlight_at(10.0, 5.0));
    assert_eq!(page.ids(), vec![bottom, top]);
    assert_eq!(page.annotation_at(15.0, 10.0), Some(top));
    assert_eq!(page.annotation_at(2.0, 2.0), Some(bottom));
    assert_eq!(page.annotation_at(200.0, 200.0), None);
}

#[test]
fn removing_an_annotation_exposes_the_one_below_and_ids_are_never_reused() {
    let mut page = PageAnnotations::new();
    let first = page.add(highlight_at(0.0, 0.0));
    let second = page.add(highlight_at(0.0, 0.0));
    assert_eq!(first, 1);
    assert_eq!(second, 2);
    assert_eq!(page.annotation_at(5.0, 5.0), Some(second));
    assert_eq!(page.remove(second).map(|a| a.bounds), Some(rect(0.0, 0.0, 40.0, 20.0)));
    assert_eq!(page.annotation_at(5.0, 5.0), Some(first));
    assert_eq!(page.remove(second), None);
    let third = page.add(highlight_at(100.0, 100.0));
    assert_eq!(third, 3);
    assert_eq!(page.len(), 2);
}

#[test]
fn hidden_annotations_stay_on_the_page_but_cannot_be_hit() {
    let mut page = PageAnnotations::new();
    let id = page.add(highlight_at(0.0, 0.0));
    assert!(page.displays_annotations());
    page.set_displays_annotations(false);
    assert!(!page.displays_annotations());
    assert_eq!(page.annotation_at(5.0, 5.0), None);
    assert_eq!(page.ids(), vec![id]);
    page.set_displays_annotations(true);
    assert_eq!(page.annotation_at(5.0, 5.0), Some(id));
}

#[test]
fn an_empty_page_has_no_annotations_and_hits_nothing() {
    let page = PageAnnotations::default();
    assert!(page.is_empty());
    assert_eq!(page.len(), 0);
    assert!(page.ids().is_empty());
    assert_eq!(page.annotation_at(0.0, 0.0), None);
    assert!(page.get(1).is_none());
}

#[test]
fn get_returns_the_annotation_by_id() {
    let mut page = PageAnnotations::new();
    let id = page.add(Annotation::new(Subtype::Link, rect(0.0, 0.0, 10.0, 10.0)));
    assert_eq!(page.get(id).map(|a| a.subtype), Some(Subtype::Link));
    assert!(page.get(id + 1).is_none());
}

// --- print: page info, text layout and the job document ---------------------------------------------------------------

use bifrost_ui::ipp::Value;
use bifrost_ui::print::{document_format, text_pages, Destination, Orientation, PaperSize, PrintError, PrintInfo, PrintOperation, TEXT_SIZE};
use std::path::{Path, PathBuf};

fn content_text(page: &bifrost_ui::print::PdfPage) -> String {
    String::from_utf8(page.content.clone()).unwrap()
}

#[test]
fn paper_sizes_report_their_points_and_pwg_keywords() {
    assert_eq!(PaperSize::A4.points(), (595.0, 842.0));
    assert_eq!(PaperSize::Letter.points(), (612.0, 792.0));
    assert_eq!(PaperSize::Legal.points(), (612.0, 1008.0));
    assert_eq!(PaperSize::A4.media(), "A4");
    assert_eq!(PaperSize::A4.pwg(), "iso_a4_210x297mm");
    assert_eq!(PaperSize::Letter.pwg(), "na_letter_8.5x11in");
}

#[test]
fn print_info_defaults_to_portrait_a4_with_one_copy_and_72_point_margins() {
    let info = PrintInfo::default();
    assert_eq!(info.paper, PaperSize::A4);
    assert_eq!(info.orientation, Orientation::Portrait);
    assert_eq!(info.margins, [72.0; 4]);
    assert_eq!(info.copies, 1);
    assert_eq!(info.pages, None);
    assert_eq!(info.paper_size(), (595.0, 842.0));
    assert_eq!(info.imageable(), (72.0, 72.0, 451.0, 698.0));
}

#[test]
fn landscape_swaps_the_paper_and_the_printable_area() {
    let info = PrintInfo { paper: PaperSize::A4, orientation: Orientation::Landscape, ..Default::default() };
    assert_eq!(info.paper_size(), (842.0, 595.0));
    assert_eq!(info.imageable(), (72.0, 72.0, 698.0, 451.0));
}

#[test]
fn margins_set_the_printable_rect_and_never_go_negative() {
    let none = PrintInfo { paper: PaperSize::Letter, margins: [0.0; 4], ..Default::default() };
    assert_eq!(none.imageable(), (0.0, 0.0, 612.0, 792.0));
    let wide = PrintInfo { margins: [400.0, 400.0, 400.0, 400.0], ..Default::default() };
    assert_eq!(wide.imageable().2, 0.0);
    assert_eq!(wide.imageable().3, 0.0);
}

#[test]
fn job_attributes_carry_copies_media_orientation_and_the_page_range() {
    let info = PrintInfo { paper: PaperSize::Letter, orientation: Orientation::Landscape, copies: 3, pages: None, ..Default::default() };
    let attrs = info.attributes();
    let names: Vec<&str> = attrs.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["copies", "media", "orientation-requested"]);
    assert_eq!(attrs[0].1, Value::Integer(3));
    assert_eq!(attrs[1].1, Value::keyword("na_letter_8.5x11in"));
    assert_eq!(attrs[2].1, Value::Enum(4));

    let ranged = PrintInfo { pages: Some((2, 7)), ..Default::default() };
    let names: Vec<String> = ranged.attributes().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names.last().map(String::as_str), Some("page-ranges"));
}

#[test]
fn text_is_laid_out_in_helvetica_from_the_top_left_margin() {
    let pages = text_pages("one\ntwo", TEXT_SIZE, &PrintInfo::default());
    assert_eq!(pages.len(), 1);
    assert_eq!((pages[0].width, pages[0].height), (595.0, 842.0));
    let content = content_text(&pages[0]);
    assert!(content.contains("BT /F0 11 Tf 72.00 756.25 Td (one) Tj ET"), "{content}");
    assert!(content.contains("BT /F0 11 Tf 72.00 742.50 Td (two) Tj ET"), "{content}");
}

#[test]
fn text_pages_break_after_as_many_lines_as_the_printable_height_holds() {
    let text = (0..100).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
    let pages = text_pages(&text, TEXT_SIZE, &PrintInfo::default());
    assert_eq!(pages.len(), 2);
    assert_eq!(content_text(&pages[0]).matches("Tj").count(), 50);
    assert_eq!(content_text(&pages[1]).matches("Tj").count(), 50);
}

#[test]
fn tabs_print_as_four_spaces_and_carriage_returns_are_dropped() {
    let pages = text_pages("a\tb\r", TEXT_SIZE, &PrintInfo::default());
    assert!(content_text(&pages[0]).contains("(a    b) Tj"));
}

#[test]
fn a_long_paragraph_wraps_onto_several_lines_inside_the_margins() {
    let text = "word ".repeat(300);
    let pages = text_pages(&text, TEXT_SIZE, &PrintInfo::default());
    let lines: usize = pages.iter().map(|p| content_text(p).matches("Tj").count()).sum();
    assert!(lines >= 2, "{lines} lines");
}

#[test]
fn a_text_job_builds_a_pdf_document() {
    let op = PrintOperation::text("Notes", "hello");
    let (format, bytes) = op.document().unwrap();
    assert_eq!(format, "application/pdf");
    assert!(bytes.starts_with(b"%PDF-"));
    assert_eq!(op.job_name, "Notes");
    assert_eq!(op.info, PrintInfo::default());
    assert_eq!(op.destination, Destination::Printer(None));
}

#[test]
fn a_pages_job_writes_the_pages_it_was_given() {
    let page = bifrost_ui::print::PdfPage { width: 100.0, height: 50.0, content: b"0 0 m 10 10 l S".to_vec(), images: Vec::new() };
    let (format, bytes) = PrintOperation::pages("Drawing", vec![page], vec!["Helvetica"]).document().unwrap();
    assert_eq!(format, "application/pdf");
    assert!(bytes.starts_with(b"%PDF-"));
}

#[test]
fn job_options_and_destination_are_kept_on_the_operation() {
    let op = PrintOperation::text("Notes", "x")
        .with_options(vec![("sides".to_string(), "two-sided-long-edge".to_string())])
        .to(Destination::Pdf(PathBuf::from("out.pdf")));
    assert_eq!(op.options, vec![("sides".to_string(), "two-sided-long-edge".to_string())]);
    assert_eq!(op.destination, Destination::Pdf(PathBuf::from("out.pdf")));
}

#[test]
fn a_file_job_is_named_after_its_file_and_a_missing_file_is_refused() {
    let op = PrintOperation::new("reports/Q3.pdf");
    assert_eq!(op.job_name, "Q3.pdf");
    let err = PrintOperation::new("/nonexistent-parity-sdk-tests-03/missing.pdf").document();
    assert!(matches!(err, Err(PrintError::Failed(_))));
}

#[test]
fn document_formats_follow_the_file_extension_case_insensitively() {
    assert_eq!(document_format(Path::new("a.PDF")), "application/pdf");
    assert_eq!(document_format(Path::new("scan.png")), "image/png");
    assert_eq!(document_format(Path::new("photo.JPEG")), "image/jpeg");
    assert_eq!(document_format(Path::new("notes.txt")), "text/plain");
    assert_eq!(document_format(Path::new("archive.bin")), "application/octet-stream");
    assert_eq!(document_format(Path::new("no-extension")), "application/octet-stream");
}

#[test]
fn a_pdf_file_job_sends_the_file_bytes_unchanged() {
    // cargo's per-target scratch directory keeps the fixture out of the source tree
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("parity_sdk-tests-03-print.pdf");
    let bytes = b"%PDF-1.4\n% print fixture\n%%EOF\n".to_vec();
    std::fs::write(&path, &bytes).unwrap();
    let (format, data) = PrintOperation::new(path.clone()).document().unwrap();
    assert_eq!(format, "application/pdf");
    assert_eq!(data, bytes);
    assert_eq!(PrintOperation::new(path).job_name, "parity_sdk-tests-03-print.pdf");
}
