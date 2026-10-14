//! Parity sdk-complete-20: headless tests for the display link's time values (`bifrost_app::display_link`), the Compose
//! Post completion of the share picker and the compose draft that shared items start (`bifrost_app::share`). Each test
//! drives the public API with no display, device, clock or shell behind it.

use bifrost_app::display_link::{DisplayLink, Time, TIME_IS_INDEFINITE, VIDEO_TIME_SCALE};
use bifrost_app::proto::{Configure, ToShell, UiRect};
use bifrost_app::share::{compose_draft_from_items, Candidate, Payload, ShareError, SharingService, SharingServiceDelegate, SharingServicePicker, SharingServicePickerDelegate, COMPOSE, COPY};
use bifrost_app::Ctx;
use bifrost_ui::compose::{ComposeDraft, ComposeError, ComposeResult, ComposeSheet, SheetState, MAX_URLS};

// ---- display time values (bifrost_app::display_link::Time) ----

#[test]
fn headless_display_time_counts_nanoseconds_at_the_video_scale() {
    let t = Time::from_nanos(10_000_000);
    assert_eq!(t, Time { value: 10_000_000, time_scale: VIDEO_TIME_SCALE, flags: 0 });
    assert_eq!(t.seconds(), Some(0.01));
    assert!(!t.is_indefinite());
}

#[test]
fn headless_a_time_past_the_integer_range_is_clamped_to_the_largest_value() {
    assert_eq!(Time::from_nanos(u64::MAX).value, i64::MAX);
}

#[test]
fn headless_an_indefinite_time_carries_its_flag_and_has_no_seconds() {
    let t = Time::indefinite();
    assert!(t.is_indefinite());
    assert_eq!(t.flags, TIME_IS_INDEFINITE);
    assert_eq!(t.seconds(), None, "an indefinite time is not zero seconds");
}

#[test]
fn headless_a_time_with_a_zero_scale_has_no_seconds() {
    assert_eq!(Time { value: 5, time_scale: 0, flags: 0 }.seconds(), None);
}

#[test]
fn headless_the_refresh_time_of_a_link_is_its_refresh_period() {
    let link = DisplayLink::new(100).expect("100 Hz is a rate");
    assert_eq!(link.refresh_time(), Time::from_nanos(10_000_000));
    assert_eq!(link.refresh_time().seconds(), Some(0.01));
}

#[test]
fn headless_a_time_stamp_reports_its_output_time_and_refresh_period_as_times() {
    let mut link = DisplayLink::new(100).expect("100 Hz is a rate");
    link.start(0);
    let stamp = link.tick(20_000_000).expect("refresh 2 has begun");
    assert_eq!(stamp.frame_number, 2);
    assert_eq!(stamp.time(), Time::from_nanos(20_000_000));
    assert_eq!(stamp.refresh_time(), Time::from_nanos(10_000_000));
    assert_eq!(stamp.time().seconds(), Some(0.02));
}

// ---- Compose Post completion (bifrost_app::share::SharingServicePicker) ----

/// Installed apps as the shell lists them, the sender left out: id, name, handles, schemes.
fn table() -> Vec<(&'static str, &'static str, Vec<String>, Vec<String>)> {
    vec![("notes", "Notes", vec!["public.plain-text:default".to_string()], vec![])]
}

fn candidates<'a>(table: &'a [(&'a str, &'a str, Vec<String>, Vec<String>)]) -> Vec<Candidate<'a>> {
    table.iter().map(|(id, name, handles, schemes)| Candidate { id, name, handles, schemes }).collect()
}

/// A context with no shell behind it: what the app sends to the shell is collected by `take_sent`.
fn testing_ctx() -> Ctx {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    let cfg = Configure { w: 100, h: 100, scale: 1.0, safe_top: 0.0, safe_bottom: 0.0, dark: true, text_scale: 1.0, reduce_motion: false };
    Ctx::testing(bifrost_render::ui::Ui::new(bifrost_render::font::Font::parse(data).unwrap(), 1.0), cfg)
}

/// Shows the picker's menu and returns its request id, checking that the request reached the shell.
fn show_menu(picker: &mut SharingServicePicker, ctx: &Ctx) -> u32 {
    let request = picker.show(ctx, UiRect { x: 0.0, y: 0.0, w: 10.0, h: 10.0 });
    let sent = ctx.take_sent();
    assert!(sent.iter().any(|m| matches!(m, ToShell::Share { request: r, .. } if *r == request)), "the menu request reaches the shell");
    request
}

#[derive(Default)]
struct PickerLog {
    chose: Vec<Option<String>>,
}

impl SharingServicePickerDelegate for PickerLog {
    fn did_choose(&mut self, service: Option<&SharingService>) {
        self.chose.push(service.map(|s| s.name().to_string()));
    }
}

#[derive(Default)]
struct SharerLog {
    shared: usize,
    failed: Vec<ShareError>,
}

impl SharingServiceDelegate for SharerLog {
    fn did_share_items(&mut self, items: &[Payload]) {
        self.shared += items.len();
    }
    fn did_fail_to_share_items(&mut self, _items: &[Payload], error: ShareError) {
        self.failed.push(error);
    }
}

/// A picker for one text item, with its menu shown, and the request of that menu.
fn shown_for_text(ctx: &Ctx) -> (SharingServicePicker, u32) {
    let table = table();
    let mut picker = SharingServicePicker::new(&[Payload::Text("hi".into())], &candidates(&table));
    let request = show_menu(&mut picker, ctx);
    (picker, request)
}

#[test]
fn headless_choosing_compose_post_reports_the_choice_and_waits_for_the_sheet() {
    let ctx = testing_ctx();
    let (mut picker, request) = shown_for_text(&ctx);
    let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
    assert!(picker.share_done(request, Some(COMPOSE), &mut chooser, &mut sharer));
    assert_eq!(chooser.chose, vec![Some(COMPOSE.to_string())]);
    assert_eq!(sharer.shared, 0, "nothing is shared until the sheet reports Done");
    assert!(sharer.failed.is_empty());
    assert!(picker.awaiting_compose());
}

#[test]
fn headless_a_done_compose_post_shares_the_items_once() {
    let ctx = testing_ctx();
    let (mut picker, request) = shown_for_text(&ctx);
    let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
    assert!(picker.share_done(request, Some(COMPOSE), &mut chooser, &mut sharer));
    assert!(picker.compose_finished(ComposeResult::Done, &mut sharer));
    assert_eq!(sharer.shared, 1, "the one shared item went into the post");
    assert!(sharer.failed.is_empty());
    assert!(!picker.awaiting_compose());
    assert!(!picker.compose_finished(ComposeResult::Done, &mut sharer), "a result is taken once");
    assert_eq!(sharer.shared, 1);
}

#[test]
fn headless_a_cancelled_compose_post_fails_with_cancelled() {
    let ctx = testing_ctx();
    let (mut picker, request) = shown_for_text(&ctx);
    let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
    assert!(picker.share_done(request, Some(COMPOSE), &mut chooser, &mut sharer));
    assert!(picker.compose_finished(ComposeResult::Cancelled, &mut sharer));
    assert_eq!(sharer.shared, 0);
    assert_eq!(sharer.failed, vec![ShareError::Cancelled]);
}

#[test]
fn headless_a_compose_result_without_a_compose_share_is_ignored() {
    let ctx = testing_ctx();
    let (mut picker, request) = shown_for_text(&ctx);
    let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
    assert!(picker.share_done(request, Some(COPY), &mut chooser, &mut sharer));
    assert_eq!(sharer.shared, 1, "Copy shares at once");
    assert!(!picker.compose_finished(ComposeResult::Done, &mut sharer));
    assert_eq!(sharer.shared, 1);
    assert!(sharer.failed.is_empty());
}

#[test]
fn headless_a_dismissed_menu_is_not_a_compose_share() {
    let ctx = testing_ctx();
    let (mut picker, request) = shown_for_text(&ctx);
    let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
    assert!(picker.share_done(request, None, &mut chooser, &mut sharer));
    assert!(!picker.awaiting_compose());
    assert!(!picker.compose_finished(ComposeResult::Done, &mut sharer));
}

#[test]
fn headless_a_sheet_started_from_shared_text_reports_done_through_the_picker() {
    let ctx = testing_ctx();
    let items = [Payload::Text("hello".into()), Payload::Url("https://example.test/a".into())];
    let table = table();
    let mut picker = SharingServicePicker::new(&items, &candidates(&table));
    let request = show_menu(&mut picker, &ctx);
    let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
    assert!(picker.share_done(request, Some(COMPOSE), &mut chooser, &mut sharer));

    let mut sheet = ComposeSheet::with_draft(compose_draft_from_items(&items).expect("both items fit a post"));
    assert!(sheet.present());
    let result = sheet.finish(true).expect("the sheet was on screen");
    assert_eq!(result, ComposeResult::Done);
    assert!(picker.compose_finished(result, &mut sharer));
    assert_eq!(sharer.shared, 2, "both items were handed to the post");
}

// ---- the compose draft that shared items start (bifrost_app::share::compose_draft_from_items) ----

#[test]
fn headless_shared_text_urls_and_files_start_the_compose_draft_in_order() {
    let items = [
        Payload::Text("hello".into()),
        Payload::Url("https://example.test/a".into()),
        Payload::File { grant: [3; 16], name: "a.png".into() },
        Payload::Text("again".into()),
    ];
    let draft = compose_draft_from_items(&items).expect("every item fits");
    assert_eq!(draft.text(), "hello\nagain", "text parts are joined by line breaks");
    assert_eq!(draft.urls(), ["https://example.test/a"]);
    assert_eq!(draft.images(), ["a.png"]);
}

#[test]
fn headless_a_shared_link_that_is_not_web_is_refused_with_its_reason() {
    let items = [Payload::Text("hi".into()), Payload::Url("ftp://example.test".into())];
    assert_eq!(compose_draft_from_items(&items), Err(ComposeError::BadUrl("ftp://example.test".into())));
}

#[test]
fn headless_shared_links_past_the_post_limit_are_refused() {
    let items: Vec<Payload> = (0..=MAX_URLS).map(|i| Payload::Url(format!("https://example.test/{i}"))).collect();
    assert_eq!(compose_draft_from_items(&items), Err(ComposeError::Full(MAX_URLS)));
}

#[test]
fn headless_shared_links_alone_leave_the_text_empty_but_the_draft_not_empty() {
    let draft = compose_draft_from_items(&[Payload::Url("https://example.test".into())]).expect("a web link fits");
    assert_eq!(draft.text(), "");
    assert!(!draft.is_empty());
}

#[test]
fn headless_nothing_shared_makes_an_empty_draft() {
    assert_eq!(compose_draft_from_items(&[]), Ok(ComposeDraft::new()));
}

#[test]
fn headless_a_sheet_from_shared_items_stays_editable_until_it_is_presented() {
    let items = [Payload::Text("draft".into())];
    let mut sheet = ComposeSheet::with_draft(compose_draft_from_items(&items).expect("text fits"));
    assert_eq!(sheet.state(), SheetState::Editing);
    sheet.add_image("b.png").expect("an editing sheet takes images");
    assert_eq!(sheet.draft().images(), ["b.png"]);
    assert!(sheet.present());
    assert_eq!(sheet.add_url("https://example.test"), Err(ComposeError::Locked));
    assert_eq!(sheet.draft().text(), "draft");
}
