//! Parity sdk-tests-01a: headless model tests for the preview, remote command and screen saver models. Each test drives
//! a pure state machine through its public API and asserts concrete values. Nothing here needs a display, a device or
//! the network, and no test reads the clock.

use bifrost_ui::preview::{Attachment, Content, ModelError, PreviewProvider, Registry, Reply, Subject};
use bifrost_ui::remote_command::{CommandCenter, CommandError, RatingCommand, RemoteAction};
use bifrost_ui::screensaver::{
    preview_rect, ss_centered_rect_in_rect, ss_random_float_between, ss_random_int_between, ss_random_point_for_size_within_rect,
    BackingStoreType, ConfigureSheet, SaverValue, ScreenSaverView, DEFAULT_FRAME_INTERVAL, MAX_CATCH_UP_FRAMES, MIN_FRAME_INTERVAL,
};
use bifrost_ui::Rect;

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

fn att(id: &str) -> Attachment {
    Attachment { id: id.into(), data: vec![1, 2, 3], content_type: "image/png".into() }
}

#[test]
fn preview_reply_rejects_bitmaps_outside_the_edge_limit() {
    assert_eq!(Reply::new(Content::Bitmap { width: 0, height: 10 }), Err(ModelError::BadSize));
    assert_eq!(Reply::new(Content::Bitmap { width: 10, height: 16385 }), Err(ModelError::BadSize));
    assert!(Reply::new(Content::Bitmap { width: 16384, height: 1 }).is_ok());
}

#[test]
fn preview_reply_rejects_non_finite_pdf_pages() {
    assert_eq!(Reply::new(Content::Pdf { width: f32::NAN, height: 792.0 }), Err(ModelError::BadSize));
    assert_eq!(Reply::new(Content::Pdf { width: 612.0, height: -1.0 }), Err(ModelError::BadSize));
    assert!(Reply::new(Content::Pdf { width: 612.0, height: 792.0 }).is_ok());
}

#[test]
fn html_attachments_need_unique_usable_ids() {
    let dup = Content::Html { body: "<img src=\"cid:a\">".into(), attachments: vec![att("a"), att("a")] };
    assert_eq!(Reply::new(dup), Err(ModelError::DuplicateAttachment));
    let spaced = Content::Html { body: String::new(), attachments: vec![att("a b")] };
    assert_eq!(Reply::new(spaced), Err(ModelError::BadAttachmentId));
    let empty = Content::Html { body: String::new(), attachments: vec![att("")] };
    assert_eq!(Reply::new(empty), Err(ModelError::BadAttachmentId));
}

#[test]
fn html_reply_resolves_only_cid_urls() {
    let content = Content::Html { body: String::new(), attachments: vec![att("logo")] };
    let reply = Reply::new(content).unwrap();
    assert_eq!(reply.resolve_cid("cid:logo").map(|a| a.content_type.as_str()), Some("image/png"));
    assert!(reply.resolve_cid("logo").is_none(), "a bare id is not a cid URL");
    assert!(reply.resolve_cid("cid:other").is_none());
}

#[test]
fn reply_title_falls_back_to_the_file_name() {
    let reply = Reply::new(Content::File { path: "/a/b.txt".into() }).unwrap();
    assert_eq!(reply.display_title("b.txt"), "b.txt");
    let titled = reply.with_title("Notes");
    assert_eq!(titled.title(), "Notes");
    assert_eq!(titled.display_title("b.txt"), "Notes");
    assert_eq!(Reply::new(Content::File { path: "  ".into() }), Err(ModelError::Empty));
}

#[test]
fn registry_refuses_blank_names_and_types() {
    let mut reg = Registry::new();
    assert_eq!(reg.register(" ", &["public.image"]), Err(ModelError::Empty));
    assert_eq!(reg.register("viewer", &["public.image", ""]), Err(ModelError::Empty));
    assert_eq!(reg.provider_for("photo.jpg"), None);
}

#[test]
fn registry_routes_a_file_to_the_provider_of_its_type() {
    let mut reg = Registry::new();
    reg.register("viewer", &["public.image"]).unwrap();
    assert_eq!(reg.provider_for("photo.jpg"), Some("viewer"));
    assert_eq!(reg.provider_for("notes.txt"), None);
}

#[test]
fn nearer_registration_beats_an_ancestor_and_ties_go_to_the_first() {
    let mut reg = Registry::new();
    reg.register("broad", &["public.image"]).unwrap();
    reg.register("exact", &["public.jpeg"]).unwrap();
    assert_eq!(reg.provider_for("photo.jpg"), Some("exact"));

    let mut tie = Registry::new();
    tie.register("first", &["public.image"]).unwrap();
    tie.register("second", &["public.image"]).unwrap();
    assert_eq!(tie.provider_for("photo.jpg"), Some("first"));
}

#[test]
fn registering_a_provider_again_replaces_its_types() {
    let mut reg = Registry::new();
    reg.register("viewer", &["public.image"]).unwrap();
    reg.register("viewer", &["com.adobe.pdf"]).unwrap();
    assert_eq!(reg.provider_for("photo.jpg"), None);
    assert_eq!(reg.provider_for("paper.pdf"), Some("viewer"));
}

struct Fixed;

impl PreviewProvider for Fixed {
    fn provide(&self, file_name: &str) -> Result<Reply, ModelError> {
        Reply::new(Content::Bitmap { width: 8, height: 8 }).map(|r| r.with_title(file_name))
    }
}

#[test]
fn preview_calls_the_provider_the_registry_picked() {
    let mut reg = Registry::new();
    reg.register("fixed", &["public.image"]).unwrap();
    let providers: [(&str, &dyn PreviewProvider); 1] = [("fixed", &Fixed)];
    let reply = reg.preview("photo.jpg", &providers).unwrap().unwrap();
    assert_eq!(reply.title(), "photo.jpg");
    assert!(reg.preview("notes.txt", &providers).is_none());
    let none: [(&str, &dyn PreviewProvider); 0] = [];
    assert!(reg.preview("photo.jpg", &none).is_none(), "the chosen provider must be handed in");
}

#[test]
fn file_subjects_must_name_an_absolute_file() {
    assert_eq!(Subject::file("relative/a.txt"), Err(ModelError::BadUrl));
    assert_eq!(Subject::file("/folder/"), Err(ModelError::BadUrl));
    assert!(Subject::file("file:///a/b.txt").is_ok());
}

#[test]
fn searchable_subjects_drop_a_blank_query() {
    assert_eq!(Subject::searchable(" ", None), Err(ModelError::Empty));
    assert_eq!(
        Subject::searchable("com.example.notes/1", Some("  ")),
        Ok(Subject::SearchableItem { identifier: "com.example.notes/1".into(), query: None })
    );
}

#[test]
fn a_narrow_searchable_domain_beats_a_broader_one() {
    let mut reg = Registry::new();
    reg.register_searchable("wide", "com.example").unwrap();
    reg.register_searchable("narrow", "com.example.notes").unwrap();
    let inside = Subject::searchable("com.example.notes/42", None).unwrap();
    assert_eq!(reg.provider_for_subject(&inside), Some("narrow"));
    let other = Subject::searchable("com.example.mail/1", None).unwrap();
    assert_eq!(reg.provider_for_subject(&other), None);
    let prefix_only = Subject::searchable("com.example.notesX/1", None).unwrap();
    assert_eq!(reg.provider_for_subject(&prefix_only), None, "a domain matches on a / boundary only");
}

#[test]
fn remote_actions_are_dispatched_to_the_newest_handler() {
    let mut center = CommandCenter::new();
    let first = center.add_handler(RemoteAction::Play);
    let second = center.add_handler(RemoteAction::Play);
    assert_ne!(first, second);
    assert_eq!(center.handler_count(RemoteAction::Play), 2);
    assert_eq!(center.dispatch(RemoteAction::Play), Ok(second));
    assert!(center.remove_handler(second));
    assert_eq!(center.dispatch(RemoteAction::Play), Ok(first));
    assert!(!center.remove_handler(second), "a removed token is gone");
}

#[test]
fn an_action_without_a_handler_or_switched_off_is_refused() {
    let mut center = CommandCenter::new();
    assert_eq!(center.dispatch(RemoteAction::Pause), Err(CommandError::NoHandler));
    center.add_handler(RemoteAction::Pause);
    center.set_enabled(RemoteAction::Pause, false);
    assert!(!center.is_enabled(RemoteAction::Pause));
    assert_eq!(center.dispatch(RemoteAction::Pause), Err(CommandError::Disabled));
    assert_eq!(center.handler_count(RemoteAction::Pause), 1, "switching off keeps the handler");
    center.set_enabled(RemoteAction::Pause, true);
    assert!(center.dispatch(RemoteAction::Pause).is_ok());
}

#[test]
fn rating_command_refuses_values_outside_its_range() {
    let mut rating = RatingCommand::new(1.0, 5.0).unwrap();
    assert_eq!(rating.request(f32::NAN), Err(CommandError::OutOfRange));
    assert_eq!(rating.request(9.0), Err(CommandError::OutOfRange));
    assert_eq!(rating.request(4.0), Ok(4.0));
    assert_eq!(rating.rating(), 4.0);
}

#[test]
fn saver_view_starts_stopped_with_the_default_interval() {
    let view = ScreenSaverView::new(rect(0.0, 0.0, 1920.0, 1080.0), false);
    assert!(!view.is_animating());
    assert!(!view.is_preview());
    assert_eq!(view.animation_time_interval(), DEFAULT_FRAME_INTERVAL);
    assert_eq!(view.backing_store_type(), BackingStoreType::Buffered);
    assert!(!view.perform_gamma_fade());
    assert!(!view.has_configure_sheet());
    assert_eq!(view.frames_animated(), 0);
}

#[test]
fn interval_is_clamped_and_bad_values_keep_the_default() {
    let mut view = ScreenSaverView::new(rect(0.0, 0.0, 10.0, 10.0), true);
    view.set_animation_time_interval(0.0001);
    assert_eq!(view.animation_time_interval(), MIN_FRAME_INTERVAL);
    view.set_animation_time_interval(f64::NAN);
    assert_eq!(view.animation_time_interval(), DEFAULT_FRAME_INTERVAL);
    view.set_animation_time_interval(-1.0);
    assert_eq!(view.animation_time_interval(), DEFAULT_FRAME_INTERVAL);
}

#[test]
fn tick_reports_the_frames_that_are_due() {
    let mut view = ScreenSaverView::new(rect(0.0, 0.0, 10.0, 10.0), false);
    view.set_animation_time_interval(0.1);
    assert_eq!(view.tick(0.0), 0, "nothing is due while stopped");
    view.start_animation(0.0);
    assert!(view.is_animating());
    assert_eq!(view.tick(0.05), 0, "the first frame is one interval after start");
    assert_eq!(view.tick(0.1), 1);
    assert_eq!(view.tick(0.15), 0);
    assert_eq!(view.tick(0.2), 1);
}

#[test]
fn a_long_stall_reports_at_most_the_catch_up_limit() {
    let mut view = ScreenSaverView::new(rect(0.0, 0.0, 10.0, 10.0), false);
    view.set_animation_time_interval(0.1);
    view.start_animation(0.0);
    assert_eq!(view.tick(10.0), MAX_CATCH_UP_FRAMES);
    assert_eq!(view.tick(10.05), 0, "the schedule restarts one interval after the stall");
    assert_eq!(view.tick(10.1), 1);
}

#[test]
fn stop_cancels_the_schedule_and_animate_one_frame_counts() {
    let mut view = ScreenSaverView::new(rect(0.0, 0.0, 10.0, 10.0), false);
    view.set_animation_time_interval(0.1);
    view.start_animation(0.0);
    view.start_animation(5.0);
    view.stop_animation();
    assert_eq!(view.tick(1.0), 0);
    view.animate_one_frame();
    view.animate_one_frame();
    assert_eq!(view.frames_animated(), 2);
}

#[test]
fn saver_view_keeps_its_backing_store_and_fade_requests() {
    let mut view = ScreenSaverView::new(rect(0.0, 0.0, 10.0, 10.0), false);
    view.set_backing_store_type(BackingStoreType::Retained);
    view.set_perform_gamma_fade(true);
    view.set_has_configure_sheet(true);
    assert_eq!(view.backing_store_type(), BackingStoreType::Retained);
    assert!(view.perform_gamma_fade());
    assert!(view.has_configure_sheet());
}

#[test]
fn random_helpers_map_unit_values_into_their_ranges() {
    assert_eq!(ss_random_int_between(0.0, 1, 6), 1);
    assert_eq!(ss_random_int_between(0.999_999, 1, 6), 6);
    assert_eq!(ss_random_int_between(0.5, 6, 1), 4, "bounds may come in either order");
    assert_eq!(ss_random_float_between(0.5, 0.0, 10.0), 5.0);
    assert_eq!(ss_random_float_between(f64::NAN, 2.0, 4.0), 2.0);
}

#[test]
fn a_point_keeps_the_whole_size_inside_the_rectangle() {
    let area = rect(10.0, 20.0, 300.0, 200.0);
    assert_eq!(ss_random_point_for_size_within_rect(0.5, 0.5, (100.0, 50.0), area), (110.0, 95.0));
    assert_eq!(ss_random_point_for_size_within_rect(0.0, 0.0, (100.0, 50.0), area), (10.0, 20.0));
    assert_eq!(ss_random_point_for_size_within_rect(1.0, 1.0, (100.0, 50.0), area), (210.0, 170.0));
    assert_eq!(ss_random_point_for_size_within_rect(0.9, 0.9, (400.0, 250.0), area), (10.0, 20.0), "oversize sits at the origin");
}

#[test]
fn centred_rect_keeps_the_inner_size() {
    let c = ss_centered_rect_in_rect(rect(0.0, 0.0, 100.0, 50.0), rect(0.0, 0.0, 300.0, 200.0));
    assert_eq!(c, rect(100.0, 75.0, 100.0, 50.0));
}

#[test]
fn preview_rect_fits_the_screen_aspect_into_the_thumbnail() {
    let screen = rect(0.0, 0.0, 1920.0, 1080.0);
    assert_eq!(preview_rect(screen, rect(0.0, 0.0, 192.0, 108.0)), rect(0.0, 0.0, 192.0, 108.0));
    assert_eq!(preview_rect(screen, rect(0.0, 0.0, 384.0, 108.0)), rect(96.0, 0.0, 192.0, 108.0));
}

#[test]
fn configure_sheet_checks_keys_kinds_and_ranges() {
    let mut sheet = ConfigureSheet::new();
    assert!(sheet.add_int("count", 1, 10, 5));
    assert!(!sheet.add_int("bad", 1, 10, 11), "a default outside the range is refused");
    assert!(!sheet.add_int("count", 1, 3, 2), "a used key is refused");
    assert!(sheet.add_bool("spin", true));
    assert!(sheet.add_choice("mode", &["a", "b"], "b"));
    assert!(!sheet.add_choice("other", &["a", "b"], "c"));
    assert_eq!(sheet.keys(), vec!["count", "spin", "mode"]);
    assert_eq!(sheet.get("count"), Some(&SaverValue::Int(5)));
    assert!(sheet.set("count", SaverValue::Int(7)));
    assert!(!sheet.set("count", SaverValue::Int(11)));
    assert!(!sheet.set("spin", SaverValue::Int(1)));
    assert!(!sheet.set("mode", SaverValue::Choice("z".into())));
    assert!(!sheet.set("missing", SaverValue::Bool(true)));
    assert_eq!(sheet.get("count"), Some(&SaverValue::Int(7)));
    assert_eq!(sheet.values().len(), 3);
    assert_eq!(sheet.len(), 3);
}

#[test]
fn a_time_clue_is_open_at_the_end_of_an_interval_and_closed_for_a_range() {
    use bifrost_ui::relevance::{ClueError, DateKind, TimeClue, POINT_CLUE_SECONDS};
    let interval = TimeClue::interval(100.0, 200.0, DateKind::Scheduled).unwrap();
    assert!(interval.is_active(100.0));
    assert!(!interval.is_active(200.0));
    assert_eq!(interval.score_at(1.0, 150.0), Some(2.0));
    assert_eq!(interval.score_at(1.0, 200.0), None);
    let range = TimeClue::range(100.0, 200.0, DateKind::Informational).unwrap();
    assert_eq!(range.score_at(1.0, 200.0), Some(0.5), "the end instant of a range is inside it");
    let point = TimeClue::at(1000.0, DateKind::Default).unwrap();
    assert_eq!(point.window(), (1000.0, 1000.0 + POINT_CLUE_SECONDS));
    assert_eq!(TimeClue::range(9.0, 3.0, DateKind::Default), Err(ClueError::EmptyWindow));
}

#[test]
fn stack_order_puts_eligible_items_first_by_score() {
    use bifrost_ui::relevance::{rank, Item, Relevance};
    let rel = |score: f32| Some(Relevance { score, duration: 100.0 });
    let items = [
        Item { date: 0.0, relevance: rel(1.0) },
        Item { date: 0.0, relevance: None },
        Item { date: 0.0, relevance: rel(3.0) },
        Item { date: 500.0, relevance: rel(9.0) },
        Item { date: 0.0, relevance: rel(3.0) },
    ];
    assert_eq!(rank(&items, 50.0), vec![2, 4, 0, 1, 3], "equal scores keep their given order; ineligible items follow");
    assert_eq!(rank(&items, 550.0), vec![3, 0, 1, 2, 4], "the item whose date has come goes to the front");
}

#[test]
fn a_short_drag_does_not_scroll_and_a_tap_is_reported_as_a_tap() {
    use bifrost_ui::scroll::Scroller;
    let mut s = Scroller::new();
    s.set_max(100.0);
    s.down(200.0, 0.0);
    s.moved(190.0, 10.0);
    assert!(!s.dragging(), "a move inside the 12 px slop is not a drag");
    assert_eq!(s.offset(), 0.0);
    assert!(s.up(190.0, 20.0), "a release without a drag is a tap");
    let mut tap = Scroller::new();
    tap.down(50.0, 0.0);
    assert!(tap.up(50.0, 5.0), "a tap without a drag reports true");
}

#[test]
fn a_drag_follows_the_finger_past_the_slop() {
    use bifrost_ui::scroll::Scroller;
    let mut s = Scroller::new();
    s.set_max(100.0);
    s.down(200.0, 0.0);
    s.moved(150.0, 10.0);
    assert!(s.dragging());
    assert_eq!(s.offset(), 38.0, "the drag starts at the slop edge, 188 - 150");
}

#[test]
fn pulling_past_the_end_rubber_bands_beyond_the_maximum() {
    use bifrost_ui::scroll::Scroller;
    let mut s = Scroller::new();
    s.set_max(100.0);
    s.down(300.0, 0.0);
    s.moved(200.0, 10.0);
    assert_eq!(s.offset(), 88.0, "inside the range the content follows the finger");
    s.moved(100.0, 20.0);
    assert!(s.offset() > 100.0 && s.offset() < 188.0, "the overshoot is rubber-banded, not free");
}

#[test]
fn jumps_and_range_changes_clamp_the_offset() {
    use bifrost_ui::scroll::Scroller;
    let mut s = Scroller::new();
    s.set_max(100.0);
    s.jump_to(250.0);
    assert_eq!(s.offset(), 100.0);
    s.jump_top();
    assert_eq!(s.offset(), 0.0);
    s.jump_to(80.0);
    s.set_max(50.0);
    assert_eq!(s.offset(), 50.0, "a shorter range pulls the offset back inside it");
}

#[test]
fn a_released_bounce_springs_back_to_the_edge() {
    use bifrost_ui::scroll::Scroller;
    let mut s = Scroller::new();
    s.set_max(100.0);
    s.down(100.0, 0.0);
    s.moved(180.0, 10.0);
    assert!(s.offset() < 0.0, "pulled above the top");
    assert!(!s.up(180.0, 20.0), "a drag release is not a tap");
    assert!(s.animating(), "the spring is running");
    s.tick(1020.0);
    assert_eq!(s.offset(), 0.0, "the spring settles on the edge");
    assert!(!s.animating());
}

#[test]
fn a_fling_decelerates_and_stops_below_the_threshold() {
    use bifrost_ui::scroll::Scroller;
    let mut s = Scroller::new();
    s.set_max(10_000.0);
    s.down(500.0, 0.0);
    s.moved(400.0, 10.0);
    s.moved(300.0, 20.0);
    assert!(!s.up(300.0, 25.0));
    assert!(s.animating(), "a fast release flings");
    s.tick(525.0);
    assert!(s.offset() > 1000.0 && s.offset() < 4000.0, "the fling carries the content on");
    s.tick(20_000.0);
    assert!(!s.animating(), "the fling stops once its speed is below the threshold");
    assert!(s.offset() > 3400.0 && s.offset() < 3600.0, "about 188 + 6.67 / 0.002002 px in total");
}
