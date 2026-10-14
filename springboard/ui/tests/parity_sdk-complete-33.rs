//! Parity sdk-complete-33: headless tests for the screen saver descriptor, configuration restore, seeded random values,
//! the render view, the named texture catalog, shader effects, lazy windowing and the shared list text form. No test
//! draws a frame or touches a device.

use bifrost_ui::effect_shader::{EffectKind, ShaderEffect};
use bifrost_ui::lazy_stack::{content_height, visible_items, visible_rows};
use bifrost_ui::render_view::{DrawRequest, PixelSize, RenderView};
use bifrost_ui::saver_random::SaverRandom;
use bifrost_ui::screensaver::{BackingStoreType, SaverValue, DEFAULT_FRAME_INTERVAL};
use bifrost_ui::screensaver_manifest::{ManifestError, SaverManifest};
use bifrost_ui::shader_library::{LibraryError, ShaderLibrary, Stage};
use bifrost_ui::shader_uniform::{UniformError, Value};
use bifrost_ui::shared_list::SharedList;
use bifrost_ui::shared_list_store::{decode, encode};
use bifrost_ui::texture_catalog::TextureCatalog;
use bifrost_ui::Rect;

fn frame() -> Rect {
    Rect { x: 0.0, y: 0.0, w: 1920.0, h: 1080.0 }
}

const CLOCK: &str = "# a saver\nname = Clock\nframe_interval = 0.05\ngamma_fade = true\nbacking_store = retained\n\
option.count = int 1 10 4\noption.stars = bool false\noption.style = choice clock,photos photos\n";

#[test]
fn a_descriptor_sets_the_name_interval_fade_and_backing_store() {
    let text = "name = Clock\nframe_interval = 0.05\ngamma_fade = true\nbacking_store = retained\n";
    let manifest = SaverManifest::parse(text).unwrap();
    assert_eq!(manifest.name(), "Clock");
    let view = manifest.view(frame(), false);
    assert!((view.animation_time_interval() - 0.05).abs() < 1e-12);
    assert!(view.perform_gamma_fade());
    assert_eq!(view.backing_store_type(), BackingStoreType::Retained);
    assert!(!view.has_configure_sheet(), "no options means no configuration sheet");
    assert!(!view.is_preview());
}

#[test]
fn a_descriptor_without_optional_keys_gives_the_defaults() {
    let view = SaverManifest::parse("name = Plain\n").unwrap().view(frame(), true);
    assert_eq!(view.animation_time_interval(), DEFAULT_FRAME_INTERVAL);
    assert!(!view.perform_gamma_fade());
    assert_eq!(view.backing_store_type(), BackingStoreType::Buffered);
    assert!(view.is_preview());
}

#[test]
fn a_descriptor_without_a_name_is_refused() {
    assert_eq!(SaverManifest::parse("gamma_fade = true\n"), Err(ManifestError::MissingName));
    assert_eq!(
        SaverManifest::parse("name =   \n"),
        Err(ManifestError::BadValue { line: 1, key: "name".to_string() })
    );
}

#[test]
fn a_line_without_an_equals_sign_is_a_syntax_error_on_its_line() {
    assert_eq!(SaverManifest::parse("name = A\nno equals here\n"), Err(ManifestError::Syntax { line: 2 }));
}

#[test]
fn an_unknown_key_is_refused_with_its_line() {
    assert_eq!(
        SaverManifest::parse("name = A\nspeed = 3\n"),
        Err(ManifestError::UnknownKey { line: 2, key: "speed".to_string() })
    );
}

#[test]
fn malformed_values_are_refused_with_their_line() {
    assert_eq!(
        SaverManifest::parse("name = A\nframe_interval = -1\n"),
        Err(ManifestError::BadValue { line: 2, key: "frame_interval".to_string() })
    );
    assert_eq!(
        SaverManifest::parse("name = A\ngamma_fade = maybe\n"),
        Err(ManifestError::BadValue { line: 2, key: "gamma_fade".to_string() })
    );
    assert_eq!(
        SaverManifest::parse("name = A\nbacking_store = gpu\n"),
        Err(ManifestError::BadValue { line: 2, key: "backing_store".to_string() })
    );
}

#[test]
fn options_declare_the_configuration_sheet_with_their_defaults() {
    let manifest = SaverManifest::parse(CLOCK).unwrap();
    assert_eq!(manifest.sheet().keys(), vec!["count", "stars", "style"]);
    assert_eq!(manifest.sheet().get("count"), Some(&SaverValue::Int(4)));
    assert_eq!(manifest.sheet().get("stars"), Some(&SaverValue::Bool(false)));
    assert_eq!(manifest.sheet().get("style"), Some(&SaverValue::Choice("photos".to_string())));
    assert!(manifest.view(frame(), false).has_configure_sheet());
}

#[test]
fn an_option_default_outside_its_range_is_refused() {
    assert_eq!(
        SaverManifest::parse("name = A\noption.count = int 1 10 12\n"),
        Err(ManifestError::BadValue { line: 2, key: "option.count".to_string() })
    );
}

#[test]
fn an_option_without_a_key_is_refused() {
    assert_eq!(
        SaverManifest::parse("name = A\noption. = bool true\n"),
        Err(ManifestError::UnknownKey { line: 2, key: "option.".to_string() })
    );
}

#[test]
fn a_repeated_option_key_is_refused() {
    assert_eq!(
        SaverManifest::parse("name = A\noption.x = bool true\noption.x = bool false\n"),
        Err(ManifestError::BadValue { line: 3, key: "option.x".to_string() })
    );
}

#[test]
fn restore_applies_saved_values_and_skips_the_ones_that_do_not_fit() {
    let mut sheet = SaverManifest::parse(CLOCK).unwrap().sheet().clone();
    let saved = vec![
        ("count".to_string(), SaverValue::Int(9)),
        ("removed".to_string(), SaverValue::Int(1)),
        ("stars".to_string(), SaverValue::Int(1)),
        ("style".to_string(), SaverValue::Choice("clock".to_string())),
    ];
    assert_eq!(sheet.restore(&saved), 2);
    assert_eq!(sheet.get("count"), Some(&SaverValue::Int(9)));
    assert_eq!(sheet.get("stars"), Some(&SaverValue::Bool(false)));
    assert_eq!(sheet.get("style"), Some(&SaverValue::Choice("clock".to_string())));
}

#[test]
fn restore_refuses_a_saved_value_outside_the_range() {
    let mut sheet = SaverManifest::parse(CLOCK).unwrap().sheet().clone();
    assert_eq!(sheet.restore(&[("count".to_string(), SaverValue::Int(11))]), 0);
    assert_eq!(sheet.get("count"), Some(&SaverValue::Int(4)));
}

#[test]
fn the_same_seed_gives_the_same_random_sequence() {
    let mut a = SaverRandom::new(42);
    let mut b = SaverRandom::new(42);
    for _ in 0..16 {
        assert_eq!(a.next_u64(), b.next_u64());
    }
}

#[test]
fn a_different_seed_gives_a_different_first_value() {
    assert_ne!(SaverRandom::new(1).next_u64(), SaverRandom::new(2).next_u64());
}

#[test]
fn unit_values_stay_in_the_half_open_range() {
    let mut r = SaverRandom::new(7);
    for _ in 0..1000 {
        let u = r.next_unit();
        assert!((0.0..1.0).contains(&u), "{u} is outside [0, 1)");
    }
}

#[test]
fn integers_cover_the_closed_range_and_nothing_else() {
    let mut r = SaverRandom::new(99);
    let mut seen = [false; 6];
    for _ in 0..1000 {
        let v = r.next_int_between(1, 6);
        assert!((1..=6).contains(&v));
        seen[(v - 1) as usize] = true;
    }
    assert!(seen.iter().all(|s| *s), "every value from 1 to 6 appears in 1000 draws");
}

#[test]
fn floats_stay_between_their_bounds_in_either_order() {
    let mut r = SaverRandom::new(5);
    for _ in 0..500 {
        let v = r.next_float_between(4.0, 2.0);
        assert!((2.0..=4.0).contains(&v));
    }
}

#[test]
fn points_keep_the_whole_size_inside_the_rectangle() {
    let rect = Rect { x: 10.0, y: 20.0, w: 300.0, h: 200.0 };
    let mut r = SaverRandom::new(11);
    for _ in 0..500 {
        let (x, y) = r.next_point_within((100.0, 50.0), rect);
        assert!((10.0..=210.0).contains(&x), "x {x} leaves the rectangle");
        assert!((20.0..=170.0).contains(&y), "y {y} leaves the rectangle");
    }
}

#[test]
fn a_new_view_draws_at_its_bounds_times_the_scale() {
    assert_eq!(RenderView::new(320.0, 200.0, 2.0).drawable_size(), PixelSize { width: 640, height: 400 });
}

#[test]
fn a_fractional_scale_rounds_to_whole_pixels() {
    let size = RenderView::new(100.0, 50.0, 1.5625).drawable_size();
    assert_eq!(size, PixelSize { width: 156, height: 78 });
}

#[test]
fn set_bounds_reports_when_the_drawable_changes() {
    let mut view = RenderView::new(100.0, 100.0, 1.0);
    assert!(view.set_bounds(200.0, 100.0));
    assert_eq!(view.drawable_size(), PixelSize { width: 200, height: 100 });
    assert!(!view.set_bounds(200.0, 100.0), "the same bounds change nothing");
}

#[test]
fn a_content_scale_change_resizes_the_drawable() {
    let mut view = RenderView::new(100.0, 100.0, 1.0);
    assert!(view.set_content_scale(2.0));
    assert_eq!(view.drawable_size(), PixelSize { width: 200, height: 200 });
}

#[test]
fn a_manual_drawable_size_applies_only_with_automatic_resizing_off() {
    let mut view = RenderView::new(100.0, 100.0, 1.0);
    assert!(!view.set_drawable_size(50, 50), "automatic resizing is on by default");
    assert!(!view.set_auto_resize(false));
    assert!(view.set_drawable_size(50, 50));
    assert_eq!(view.drawable_size(), PixelSize { width: 50, height: 50 });
    assert!(!view.set_bounds(300.0, 300.0), "the bounds do not move the drawable while it is manual");
    assert_eq!(view.drawable_size(), PixelSize { width: 50, height: 50 });
}

#[test]
fn turning_automatic_resizing_back_on_takes_the_bounds_size() {
    let mut view = RenderView::new(100.0, 100.0, 1.0);
    view.set_auto_resize(false);
    view.set_bounds(300.0, 300.0);
    assert!(view.set_auto_resize(true));
    assert_eq!(view.drawable_size(), PixelSize { width: 300, height: 300 });
}

#[test]
fn invalid_bounds_count_as_zero_and_an_invalid_scale_as_one() {
    let mut view = RenderView::new(f64::NAN, -5.0, f64::INFINITY);
    assert!(view.drawable_size().is_empty());
    assert!(view.set_bounds(40.0, 30.0));
    assert_eq!(view.drawable_size(), PixelSize { width: 40, height: 30 }, "the scale stayed at one");
}

#[test]
fn an_empty_drawable_draws_nothing_and_keeps_the_loop_schedule() {
    let mut view = RenderView::new(0.0, 10.0, 1.0);
    assert_eq!(view.frame(0.0), None);
    assert!(view.set_bounds(10.0, 10.0));
    assert_eq!(view.frame(0.0), Some(DrawRequest { size: PixelSize { width: 10, height: 10 } }));
}

#[test]
fn a_timed_view_draws_at_its_frame_rate() {
    let mut view = RenderView::new(10.0, 10.0, 1.0);
    assert_eq!(view.preferred_fps(), 60);
    assert!(view.frame(0.0).is_some());
    assert!(view.frame(0.01).is_none(), "the next frame is due one sixtieth of a second later");
    assert!(view.frame(1.0 / 60.0).is_some());
}

#[test]
fn the_preferred_frame_rate_is_clamped() {
    let mut view = RenderView::new(10.0, 10.0, 1.0);
    view.set_preferred_fps(0);
    assert_eq!(view.preferred_fps(), 1);
    view.set_preferred_fps(500);
    assert_eq!(view.preferred_fps(), 120);
}

#[test]
fn a_paused_view_draws_nothing_until_resumed() {
    let mut view = RenderView::new(10.0, 10.0, 1.0);
    view.set_paused(true);
    assert!(view.is_paused());
    assert_eq!(view.frame(0.0), None);
    view.set_paused(false);
    assert!(view.frame(0.0).is_some());
}

#[test]
fn an_on_demand_view_draws_once_per_batch_of_requests() {
    let mut view = RenderView::new(10.0, 10.0, 1.0);
    view.set_on_demand(true);
    assert_eq!(view.frame(0.0), None);
    view.request_redraw();
    view.request_redraw();
    assert!(view.frame(0.0).is_some());
    assert_eq!(view.frame(0.1), None);
}

#[test]
fn the_draw_request_carries_the_drawable_size() {
    let mut view = RenderView::new(40.0, 30.0, 2.0);
    assert_eq!(view.frame(0.0), Some(DrawRequest { size: PixelSize { width: 80, height: 60 } }));
}

#[test]
fn a_name_resolves_to_the_smallest_scale_at_least_the_request() {
    let mut catalog = TextureCatalog::new();
    assert!(catalog.add("logo", 1.0, "logo.png"));
    assert!(catalog.add("logo", 2.0, "logo@2x.png"));
    assert!(catalog.add("logo", 3.0, "logo@3x.png"));
    assert_eq!(catalog.resolve("logo", 2.0), Some("logo@2x.png"));
    assert_eq!(catalog.resolve("logo", 1.5), Some("logo@2x.png"), "a larger file is scaled down, not up");
    assert_eq!(catalog.resolve("logo", 1.0), Some("logo.png"));
    assert_eq!(catalog.resolve("logo", 3.5), Some("logo@3x.png"), "the largest file when all are smaller");
}

#[test]
fn variants_are_kept_in_scale_order_whatever_the_insert_order() {
    let mut catalog = TextureCatalog::new();
    catalog.add("icon", 3.0, "a");
    catalog.add("icon", 1.0, "b");
    catalog.add("icon", 2.0, "c");
    assert_eq!(catalog.scales("icon"), vec![1.0, 2.0, 3.0]);
}

#[test]
fn a_duplicate_scale_an_empty_name_or_path_and_a_bad_scale_are_refused() {
    let mut catalog = TextureCatalog::new();
    assert!(catalog.add("logo", 2.0, "first"));
    assert!(!catalog.add("logo", 2.0, "second"));
    assert!(!catalog.add("", 1.0, "x"));
    assert!(!catalog.add("x", 1.0, ""));
    assert!(!catalog.add("x", 0.0, "x"));
    assert!(!catalog.add("x", f64::NAN, "x"));
    assert_eq!(catalog.resolve("logo", 2.0), Some("first"));
    assert_eq!(catalog.names(), vec!["logo"]);
}

#[test]
fn an_unknown_name_resolves_to_nothing_and_names_are_sorted() {
    let mut catalog = TextureCatalog::new();
    catalog.add("b", 1.0, "b.png");
    catalog.add("a", 1.0, "a.png");
    assert_eq!(catalog.resolve("missing", 1.0), None);
    assert_eq!(catalog.names(), vec!["a", "b"]);
}

#[test]
fn the_effect_kinds_use_the_reference_modifier_names() {
    assert_eq!(EffectKind::Color.name(), "colorEffect");
    assert_eq!(EffectKind::Distortion.name(), "distortionEffect");
    assert_eq!(EffectKind::Layer.name(), "layerEffect");
    assert_eq!(EffectKind::Visual.name(), "visualEffect");
    assert!(EffectKind::Distortion.returns_offset());
    assert!(!EffectKind::Color.returns_offset());
}

#[test]
fn a_fragment_function_validates_for_every_effect_kind() {
    let mut lib = ShaderLibrary::new();
    lib.add("tint", Stage::Fragment).unwrap();
    for kind in [EffectKind::Color, EffectKind::Distortion, EffectKind::Layer, EffectKind::Visual] {
        assert_eq!(ShaderEffect::new(kind, "tint").validate(&lib), Ok(()));
    }
}

#[test]
fn a_vertex_function_is_refused_as_an_effect() {
    let mut lib = ShaderLibrary::new();
    lib.add("warp_vertex", Stage::Vertex).unwrap();
    assert_eq!(
        ShaderEffect::new(EffectKind::Color, "warp_vertex").validate(&lib),
        Err(LibraryError::WrongStage { name: "warp_vertex".to_string(), found: Stage::Vertex })
    );
}

#[test]
fn a_missing_function_is_not_found() {
    let lib = ShaderLibrary::new();
    assert_eq!(
        ShaderEffect::new(EffectKind::Layer, "nope").validate(&lib),
        Err(LibraryError::NotFound("nope".to_string()))
    );
}

#[test]
fn an_effect_checks_its_uniforms_when_they_are_set() {
    let mut effect = ShaderEffect::new(EffectKind::Color, "tint");
    assert_eq!(effect.set_uniform("strength", Value::Float(0.5)), Ok(()));
    assert_eq!(effect.uniforms().get("strength"), Some(Value::Float(0.5)));
    assert_eq!(effect.set_uniform("strength", Value::Int(2)), Err(UniformError::TypeChanged));
    assert_eq!(effect.set_uniform("bad name", Value::Float(1.0)), Err(UniformError::BadName));
    assert_eq!(effect.set_uniform("gain", Value::Float(f32::NAN)), Err(UniformError::NotFinite));
    assert_eq!(effect.uniforms().len(), 1);
}

#[test]
fn content_height_counts_the_spacing_between_rows_only() {
    assert_eq!(content_height(0, 10.0, 2.0), 0.0);
    assert_eq!(content_height(3, 10.0, 2.0), 34.0);
}

#[test]
fn a_viewport_at_the_top_builds_only_the_rows_it_touches() {
    assert_eq!(visible_rows(100, 10.0, 0.0, 35.0, 0.0, 0), 0..4);
}

#[test]
fn a_scrolled_viewport_builds_the_rows_under_it() {
    assert_eq!(visible_rows(100, 10.0, 0.0, 20.0, 95.0, 0), 9..12);
}

#[test]
fn overscan_widens_the_range_on_both_sides() {
    assert_eq!(visible_rows(100, 10.0, 0.0, 20.0, 95.0, 1), 8..13);
}

#[test]
fn a_scroll_past_the_end_builds_no_rows() {
    assert_eq!(visible_rows(5, 10.0, 0.0, 20.0, 1000.0, 0), 5..5);
}

#[test]
fn spacing_moves_the_row_pitch() {
    assert_eq!(visible_rows(100, 10.0, 10.0, 45.0, 0.0, 0), 0..3);
}

#[test]
fn no_rows_or_no_row_height_builds_nothing() {
    assert_eq!(visible_rows(0, 10.0, 0.0, 100.0, 0.0, 0), 0..0);
    assert_eq!(visible_rows(10, 0.0, 0.0, 100.0, 0.0, 0), 0..0);
}

#[test]
fn a_grid_builds_the_items_of_the_rows_under_the_viewport() {
    assert_eq!(visible_items(10, 3, 10.0, 0.0, 10.0, 10.0, 0), 3..9);
}

#[test]
fn a_grid_with_no_columns_is_one_column() {
    assert_eq!(visible_items(10, 0, 10.0, 0.0, 10.0, 0.0, 0), 0..2);
}

#[test]
fn a_shared_list_round_trips_with_its_favourites_in_place() {
    let mut list = SharedList::new(5);
    list.add("a", "Alpha");
    list.add("b", "Beta");
    list.set_favourite("a", true);
    list.add("c", "Gamma");
    let text = encode(&list);
    assert_eq!(text, "R\tc\tGamma\nR\tb\tBeta\nF\ta\tAlpha\n");
    assert_eq!(decode(&text, 5), list);
}

#[test]
fn keys_and_titles_with_tabs_backslashes_and_line_breaks_survive() {
    let mut list = SharedList::new(5);
    list.add("dir\\file", "Line\tone\nTwo\r");
    let text = encode(&list);
    assert_eq!(text.lines().count(), 1, "an entry is one line");
    assert_eq!(decode(&text, 5), list);
}

#[test]
fn lines_that_do_not_parse_are_skipped() {
    let list = decode("X\ta\tb\nR\tkeep\tKeep\nR only-one-field\n", 5);
    assert_eq!(list.len(), 1);
    assert!(list.contains("keep"));
}

#[test]
fn decoding_keeps_only_the_newest_recents_within_the_capacity() {
    let list = decode("R\t1\tOne\nR\t2\tTwo\nR\t3\tThree\n", 2);
    assert_eq!(list.len(), 2);
    assert!(list.contains("1") && list.contains("2"));
    assert!(!list.contains("3"));
}

#[test]
fn a_key_repeated_in_the_text_is_kept_once() {
    let list = decode("R\ta\tA\nR\ta\tA again\n", 5);
    assert_eq!(list.len(), 1);
    assert_eq!(list.entries()[0].title, "A");
}
