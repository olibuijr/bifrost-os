//! Parity tests for the SDK surface of the share sheet, the pasteboard, the workspace and notification centers, the
//! widget materials and the extension-like APIs (task sdk-tests-01b). They are headless host tests: pure models, the
//! in-process notification centers and the pasteboard store, with no display, device, network or wall-clock time.
//! Each test names the module it exercises by its full path.

// ---- widgetkit: rendering modes and materials (bifrost_app::widgets) ----

use bifrost_app::proto::{Configure, ToShell, UiRect};
use bifrost_app::widgets::Material;
use bifrost_app::Ctx;
use bifrost_ui::visual_effect::MaterialKind;

#[test]
fn headless_widget_rendering_materials_map_to_visual_effect_kinds_both_ways() {
    assert_eq!(Material::ALL.len(), 14, "every system material is listed once");
    for m in Material::ALL {
        assert_eq!(Material::from_model(m.model_kind()), m, "{m:?} survives a round trip");
        assert_eq!(m.kind(), m.model_kind());
    }
    assert_eq!(Material::Hud.model_kind(), MaterialKind::HudWindow);
    assert_eq!(Material::from_model(MaterialKind::Sidebar), Material::Sidebar);
}

#[test]
fn headless_widget_rendering_blur_radius_follows_the_material_class() {
    assert_eq!(Material::Popover.blur_pt(), 6.0);
    assert_eq!(Material::Selection.blur_pt(), 6.0);
    assert_eq!(Material::Hud.blur_pt(), 16.0);
    assert_eq!(Material::Sidebar.blur_pt(), 10.0);
}

// ---- share sheet: services and the picker (bifrost_app::share, springboard::shell::sharing) ----

use bifrost_app::share::{is_inbound, SharingService, SharingServicePicker, SharingServicePickerDelegate, SharingServiceDelegate, ShareError};
use bifrost_app::share::{text_from_url, Candidate, Payload, COMPOSE, COPY, INBOUND};
use springboard::shell::sharing::{copy_text, text_url};

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

/// Installed apps as the shell lists them, the sender left out: id, name, handles, schemes.
fn table() -> Vec<(&'static str, &'static str, Vec<String>, Vec<String>)> {
    vec![
        ("notes", "Notes", s(&["public.plain-text:default"]), vec![]),
        ("photos", "Photos", s(&["public.png:owner", "public.jpeg"]), vec![]),
        ("browser", "Browser", s(&["public.html:owner"]), s(&["https"])),
    ]
}

fn candidates<'a>(table: &'a [(&'a str, &'a str, Vec<String>, Vec<String>)]) -> Vec<Candidate<'a>> {
    table.iter().map(|(id, name, handles, schemes)| Candidate { id, name, handles, schemes }).collect()
}

fn titles(services: &[SharingService]) -> Vec<&str> {
    services.iter().map(|s| s.title()).collect()
}

fn png() -> Payload {
    Payload::File { grant: [1; 16], name: "a.png".into() }
}

#[test]
fn headless_share_sheet_text_is_offered_to_the_apps_that_open_text_then_compose_and_copy() {
    let table = table();
    let services = SharingService::sharing_services_for_items(&[Payload::Text("hi".into())], &candidates(&table));
    assert_eq!(titles(&services), ["Notes", "Compose Post", "Copy"]);
    assert_eq!(services[0].name(), "notes");
    assert_eq!(services[1].name(), COMPOSE);
    assert_eq!(services[2].name(), COPY);
}

#[test]
fn headless_share_sheet_files_match_by_conformance_and_web_urls_by_scheme() {
    let table = table();
    let files = SharingService::sharing_services_for_items(&[png()], &candidates(&table));
    assert_eq!(titles(&files), ["Photos", "Compose Post", "Copy"]);
    let url = [Payload::Url("https://example.com/x".into())];
    assert_eq!(titles(&SharingService::sharing_services_for_items(&url, &candidates(&table))), ["Browser", "Compose Post", "Copy"]);
}

#[test]
fn headless_share_sheet_a_service_must_take_every_item_and_nothing_shared_offers_nothing() {
    let table = table();
    let mixed = [png(), Payload::Text("x".into())];
    assert_eq!(titles(&SharingService::sharing_services_for_items(&mixed, &candidates(&table))), ["Compose Post", "Copy"]);
    assert!(SharingService::sharing_services_for_items(&[], &candidates(&table)).is_empty());
}

#[test]
fn headless_share_sheet_can_perform_is_false_for_items_a_service_does_not_accept() {
    let table = table();
    let services = SharingService::sharing_services_for_items(&[Payload::Text("hi".into())], &candidates(&table));
    let notes = services.iter().find(|s| s.name() == "notes").expect("Notes takes text");
    let copy = services.iter().find(|s| s.name() == COPY).expect("Copy is always offered");
    assert!(notes.can_perform(&[Payload::Text("again".into())]));
    assert!(!notes.can_perform(&[png()]), "Notes does not open images");
    assert!(!notes.can_perform(&[]), "nothing to share is never performable");
    assert!(copy.can_perform(&[png(), Payload::Url("https://x.test".into())]), "Copy takes every kind");
}

#[test]
fn headless_share_sheet_built_in_services_are_named_and_others_are_not() {
    assert_eq!(SharingService::named(COPY).expect("Copy").title(), "Copy");
    assert_eq!(SharingService::named(COMPOSE).expect("Compose").title(), "Compose Post");
    assert!(SharingService::named("ghost").is_none());
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

#[test]
fn headless_share_sheet_inbound_requests_are_recognised() {
    assert!(is_inbound(INBOUND | 5));
    assert!(!is_inbound(5));
}

#[test]
fn headless_share_sheet_copy_puts_text_urls_and_file_names_on_the_pasteboard_text() {
    let items = [Payload::Text("one".into()), Payload::Url("https://x.test/a".into()), png()];
    assert_eq!(copy_text(&items), "one\nhttps://x.test/a\na.png");
}

#[test]
fn headless_share_sheet_text_travels_as_a_data_url_and_comes_back_unchanged() {
    let text = "héllo, wörld\nline two";
    let url = text_url(text);
    assert!(url.starts_with("data:text/plain;charset=utf-8,"));
    assert_eq!(text_from_url(&url).as_deref(), Some(text));
    assert_eq!(text_from_url("https://example.com"), None, "other URLs carry no text");
}

// ---- share sheet: the picker's answer routing (bifrost_app::share::SharingServicePicker) ----

#[test]
fn headless_share_sheet_picker_offers_the_services_in_menu_order() {
    let table = table();
    let picker = SharingServicePicker::new(&[Payload::Text("hi".into())], &candidates(&table));
    let names: Vec<&str> = picker.services().iter().map(|s| s.name()).collect();
    assert_eq!(names, ["notes", COMPOSE, COPY]);
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

#[test]
fn headless_share_sheet_completion_reports_the_chosen_service_then_the_items() {
    let ctx = testing_ctx();
    let table = table();
    let items = [Payload::Text("hi".into())];
    let mut picker = SharingServicePicker::new(&items, &candidates(&table));
    let request = show_menu(&mut picker, &ctx);
    let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
    assert!(picker.share_done(request, Some("notes"), &mut chooser, &mut sharer));
    assert_eq!(chooser.chose, vec![Some("notes".to_string())]);
    assert_eq!(sharer.shared, 1, "the one item went to the chosen app");
    assert!(!picker.share_done(request, Some("notes"), &mut chooser, &mut sharer), "an answer is taken once");
}

#[test]
fn headless_share_sheet_a_dismissed_menu_shares_nothing_and_a_foreign_target_fails() {
    let ctx = testing_ctx();
    let table = table();
    let items = [Payload::Text("hi".into())];
    let mut picker = SharingServicePicker::new(&items, &candidates(&table));
    let (mut chooser, mut sharer) = (PickerLog::default(), SharerLog::default());
    let dismissed = show_menu(&mut picker, &ctx);
    assert!(picker.share_done(dismissed, None, &mut chooser, &mut sharer));
    assert_eq!(chooser.chose, vec![None]);
    assert_eq!(sharer.shared, 0);

    let foreign = show_menu(&mut picker, &ctx);
    assert!(picker.share_done(foreign, Some("ghost"), &mut chooser, &mut sharer));
    assert_eq!(chooser.chose, vec![None, None]);
    assert_eq!(sharer.failed, vec![ShareError::NotOffered("ghost".into())]);
    assert!(!picker.share_done(9999, Some(COPY), &mut chooser, &mut sharer), "an answer for another request is ignored");
}

#[test]
fn headless_share_sheet_copy_performs_through_the_clipboard_and_others_do_not() {
    let ctx = testing_ctx();
    let copy = SharingService::named(COPY).expect("Copy");
    assert!(copy.perform(&ctx, &[Payload::Text("hi".into())]));
    let sent = ctx.take_sent();
    assert!(sent.iter().any(|m| matches!(m, ToShell::SetClipboard { text } if text == "hi")), "Copy writes the clipboard");
    assert!(!copy.perform(&ctx, &[]), "nothing to copy");
    let table = table();
    let services = SharingService::sharing_services_for_items(&[Payload::Text("hi".into())], &candidates(&table));
    let notes = services.iter().find(|s| s.name() == "notes").expect("Notes takes text");
    assert!(!notes.perform(&ctx, &[Payload::Text("hi".into())]), "apps are reached through the menu, not perform");
}

// ---- clipboard: named pasteboards (bifrost_app::pasteboard) ----

use bifrost_app::pasteboard::{NativePasteboard, HTML, PNG, RTF, STRING, FILE_URL};
use bifrost_app::pasteboard_item::PasteboardItem;

#[test]
fn headless_clipboard_strings_and_types_round_trip_on_a_named_board() {
    let ctx = testing_ctx();
    let pb = NativePasteboard::named("sdk-tests-01b-strings");
    assert_eq!(pb.name(), "sdk-tests-01b-strings");
    pb.clear_contents(&ctx);
    pb.set_string(&ctx, "hello board");
    assert_eq!(pb.string().as_deref(), Some("hello board"));
    assert_eq!(pb.types(), [STRING]);
    assert_eq!(pb.available_type(&[RTF, STRING]), Some(STRING.to_string()));
    assert_eq!(pb.available_type(&[RTF]), None, "no RTF on the board");
    assert!(pb.can_read_item(&[STRING]));
    assert!(!pb.can_read_item(&[PNG]));
}

#[test]
fn headless_clipboard_clearing_steps_the_change_count() {
    let ctx = testing_ctx();
    let pb = NativePasteboard::named("sdk-tests-01b-count");
    let first = pb.clear_contents(&ctx);
    assert_eq!(pb.change_count(), first);
    let second = pb.clear_contents(&ctx);
    assert!(second > first, "every clear is a new change");
    assert_eq!(pb.change_count(), second);
    assert!(pb.types().is_empty(), "a cleared board has no types");
}

#[test]
fn headless_clipboard_items_keep_each_representation_by_type() {
    let ctx = testing_ctx();
    let pb = NativePasteboard::named("sdk-tests-01b-items");
    pb.clear_contents(&ctx);
    let mut plain = PasteboardItem::new();
    plain.set_string("one", STRING);
    let mut rich = PasteboardItem::new();
    rich.set_string("<i>two</i>", HTML);
    assert!(pb.write_items(&ctx, &[plain, rich]));
    assert_eq!(pb.types(), [STRING, HTML]);
    let items = pb.items();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].string(HTML).as_deref(), Some("<i>two</i>"));
    assert_eq!(pb.read_items(&[HTML]).len(), 1, "only the HTML item conforms");
    assert_eq!(pb.available_type(&[RTF, HTML]), Some(HTML.to_string()));
}

#[test]
fn headless_clipboard_raw_data_is_stored_per_type_and_file_lists_are_newline_separated() {
    let ctx = testing_ctx();
    let pb = NativePasteboard::named("sdk-tests-01b-data");
    pb.clear_contents(&ctx);
    pb.set_data(&ctx, &[137, 80, 78, 71], PNG);
    assert_eq!(pb.data(PNG), Some(vec![137, 80, 78, 71]));
    assert_eq!(pb.data(STRING), None);
    pb.write_objects(&ctx, &["file:///a.txt", "file:///b.txt"]);
    assert_eq!(pb.data(FILE_URL).as_deref().map(|d| std::str::from_utf8(d).unwrap()), Some("file:///a.txt\nfile:///b.txt"));
    assert_eq!(pb.data(PNG), None, "writing objects clears the earlier data");
}

// ---- system events: workspace notifications and icons (bifrost_app::workspace) ----

use bifrost_app::proto::WorkspaceEventKind;
use bifrost_app::workspace::{self, icon_for_content_type, icon_for_file};

#[test]
fn headless_system_events_each_workspace_event_has_its_notification_name() {
    use WorkspaceEventKind as K;
    assert_eq!(workspace::notification_name(K::DidLaunchApplication), workspace::DID_LAUNCH_APPLICATION);
    assert_eq!(workspace::notification_name(K::DidTerminateApplication), "workspace.didTerminateApplication");
    assert_eq!(workspace::notification_name(K::DidActivateApplication), "workspace.didActivateApplication");
    assert_eq!(workspace::notification_name(K::WillSleep), "workspace.willSleep");
    assert_eq!(workspace::notification_name(K::DidWake), "workspace.didWake");
    assert_eq!(workspace::notification_name(K::ScreensDidSleep), "workspace.screensDidSleep");
    assert_eq!(workspace::notification_name(K::ScreensDidWake), "workspace.screensDidWake");
}

#[test]
fn headless_system_events_icons_follow_the_content_type_hierarchy() {
    assert_eq!(icon_for_content_type("public.folder"), "folder");
    assert_eq!(icon_for_content_type("com.example.unknown"), "doc", "an unknown type gets the plain document glyph");
    assert_eq!(icon_for_file("notes.unknownext"), "doc", "a name without a known extension gets the document glyph");
}

// ---- notification centers: in-process and distributed (bifrost_app::notification) ----

use bifrost_app::notification::{DistributedNotificationCenter, NotificationCenter};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

#[test]
fn headless_notification_center_delivers_named_posts_to_matching_observers() {
    let center = NotificationCenter::default();
    let hits = Rc::new(Cell::new(0));
    let seen = hits.clone();
    let token = center.add_observer("sdk-tests-01b.note", None, move |_| seen.set(seen.get() + 1));
    center.post("sdk-tests-01b.note", None, HashMap::new());
    center.post("sdk-tests-01b.other", None, HashMap::new());
    assert_eq!(hits.get(), 1, "only the observed name is delivered");
    center.remove_observer(token);
    center.post("sdk-tests-01b.note", None, HashMap::new());
    assert_eq!(hits.get(), 1, "a removed observer hears nothing");
}

#[test]
fn headless_notification_center_filters_by_object_and_passes_user_info() {
    let center = NotificationCenter::default();
    let got = Rc::new(Cell::new(0u64));
    let sink = got.clone();
    let _token = center.add_observer("sdk-tests-01b.object", Some(7), move |n| {
        sink.set(n.object.unwrap_or(0) + n.user_info.get("k").map_or(0, |v| v.len() as u64));
    });
    let mut info = HashMap::new();
    info.insert("k".to_string(), "abc".to_string());
    center.post("sdk-tests-01b.object", Some(8), info.clone());
    assert_eq!(got.get(), 0, "another object is not delivered");
    center.post("sdk-tests-01b.object", Some(7), info);
    assert_eq!(got.get(), 10, "the object 7 and the three-letter value arrive");
}

#[test]
fn headless_notification_center_distributed_posts_stay_in_their_own_namespace() {
    let local = NotificationCenter::default();
    let distributed = DistributedNotificationCenter;
    let plain_hits = Rc::new(Cell::new(0));
    let dist_hits = Rc::new(Cell::new(0));
    let p = plain_hits.clone();
    let _plain = local.add_observer("ping", None, move |_| p.set(p.get() + 1));
    let d = dist_hits.clone();
    let _dist = distributed.add_observer("ping", move |_| d.set(d.get() + 1));
    distributed.post("ping", HashMap::new());
    assert_eq!(dist_hits.get(), 1, "the distributed observer hears its post");
    assert_eq!(plain_hits.get(), 0, "an in-process observer of the same name does not");
}

#[test]
fn headless_notification_center_distributed_post_with_reaches_the_shell_and_this_app() {
    let ctx = testing_ctx();
    let distributed = DistributedNotificationCenter;
    let heard = Rc::new(Cell::new(false));
    let h = heard.clone();
    let _token = distributed.add_observer("sdk-tests-01b.broadcast", move |_| h.set(true));
    let mut info = HashMap::new();
    info.insert("k".to_string(), "v".to_string());
    distributed.post_with(&ctx, "sdk-tests-01b.broadcast", info);
    assert!(heard.get(), "the sender's own observers hear it synchronously");
    let sent = ctx.take_sent();
    assert!(
        sent.iter().any(|m| matches!(m, ToShell::DistPost { name, user_info } if name == "sdk-tests-01b.broadcast" && user_info == &vec![("k".to_string(), "v".to_string())])),
        "the shell is asked to relay the post to other apps"
    );
}

// ---- app extensions: preview items in the preview panel (bifrost_app::peek) ----

use bifrost_app::peek::{ItemState, PreviewPanel};
use bifrost_app::proto::peek::PreviewItem;

fn preview(title: &str) -> PreviewItem {
    PreviewItem { url: format!("file:///{title}.txt"), title: title.to_string() }
}

#[test]
fn headless_app_extensions_preview_panel_steps_and_clamps_its_items() {
    let mut panel = PreviewPanel::new();
    assert!(panel.current_item().is_none());
    panel.set_items(vec![preview("one"), preview("two"), preview("three")]);
    assert_eq!(panel.current_item().map(|i| i.title.as_str()), Some("one"));
    panel.step(1);
    panel.step(1);
    panel.step(5);
    assert_eq!(panel.current_preview_item_index(), 2, "stepping stops at the last item");
    panel.step(-9);
    assert_eq!(panel.current_preview_item_index(), 0, "and at the first");
    panel.set_current_preview_item_index(10);
    assert_eq!(panel.current_preview_item_index(), 2);
    panel.set_items(vec![preview("only")]);
    assert_eq!(panel.current_preview_item_index(), 0, "the index is clamped into a shorter list");
}

#[test]
fn headless_app_extensions_preview_items_start_ready_and_track_their_state() {
    let mut panel = PreviewPanel::new();
    panel.set_items(vec![preview("a"), preview("b")]);
    assert_eq!(panel.display_state(0), Some(ItemState::Ready));
    assert!(panel.set_display_state(1, ItemState::Loading));
    assert_eq!(panel.display_state(1), Some(ItemState::Loading));
    assert!(!panel.set_display_state(5, ItemState::Failed), "no item past the end");
    assert_eq!(panel.display_state(5), None);
    assert_eq!(panel.items().len(), 2);
}

