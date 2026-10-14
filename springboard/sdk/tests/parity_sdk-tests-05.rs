//! Parity sdk-tests-05: headless tests for the system materials of `bifrost_app::widgets`. Every drawing test renders into
//! an in-memory `Surface` and asserts pixels. No shell, window server, display or device is involved, and each test sets
//! the process-wide vibrancy switch it depends on before drawing.

use bifrost_app::credential_exchange::{escape, export, import, unescape, ExchangeError, PasswordRecord, HEADER};
use bifrost_app::kit::{pack, Font, Surface, Ui};
use bifrost_app::proto::{MaterialKind, MaterialState};
use bifrost_app::widgets::{self, Blending, Material, State};
use bifrost_ui::visual_effect::MaterialRequest;
use bifrost_ui::Rect;
use std::collections::HashSet;

const W: usize = 64;
const H: usize = 64;
const BACKDROP: u32 = 0xff09_0909;

fn ui() -> Ui {
    let data: &'static [u8] = Box::leak(std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/SFUI.ttf")).unwrap().into_boxed_slice());
    Ui::new(Font::parse(data).unwrap(), 1.0)
}

fn stripes() -> Vec<u32> {
    (0..W * H).map(|i| if (i % W) / 4 % 2 == 0 { pack(255, 255, 255, 255) } else { pack(0, 0, 0, 255) }).collect()
}

fn backdrop() -> Vec<u32> {
    vec![BACKDROP; W * H]
}

fn surface(px: &mut [u32]) -> Surface<'_> {
    Surface { w: W, h: H, stride: W, px, y0: 0 }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn the_catalogue_lists_fourteen_distinct_materials_matching_the_model() {
    let kinds: HashSet<MaterialKind> = Material::ALL.iter().map(|m| m.model_kind()).collect();
    assert_eq!(Material::ALL.len(), 14);
    assert_eq!(kinds.len(), 14, "each material is its own window server kind");
    assert_eq!(kinds, MaterialKind::ALL.into_iter().collect::<HashSet<_>>());
}

#[test]
fn every_material_round_trips_through_its_model_kind() {
    for m in Material::ALL {
        assert_eq!(Material::from_model(m.model_kind()), m, "{m:?}");
    }
}

#[test]
fn materials_map_to_their_window_server_wire_values() {
    assert_eq!(Material::Titlebar.kind().window_server_wire(), 0);
    assert_eq!(Material::Popover.kind().window_server_wire(), 2);
    assert_eq!(Material::Sidebar.kind().window_server_wire(), 3);
    assert_eq!(Material::Hud.kind().window_server_wire(), 7);
    assert_eq!(Material::Selection.kind().window_server_wire(), 15);
}

#[test]
fn an_older_shell_sees_the_sidebar_or_the_title_bar_only() {
    assert_eq!(Material::Sidebar.kind().legacy(), MaterialKind::Sidebar);
    assert_eq!(Material::Hud.kind().legacy(), MaterialKind::Titlebar);
    assert_eq!(Material::Selection.kind().legacy(), MaterialKind::Titlebar);
}

#[test]
fn the_hud_blurs_more_than_the_title_bar_which_blurs_more_than_a_popover() {
    assert_eq!(Material::Hud.blur_pt(), 16.0);
    assert_eq!(Material::Titlebar.blur_pt(), 10.0);
    assert_eq!(Material::Popover.blur_pt(), 6.0);
    assert!(Material::Hud.blur_pt() > Material::Titlebar.blur_pt());
    assert!(Material::Titlebar.blur_pt() > Material::Popover.blur_pt());
}

#[test]
fn every_material_tint_is_translucent_and_every_opaque_fallback_is_opaque() {
    for m in Material::ALL {
        assert!(m.tint() >> 24 < 255, "{m:?} is translucent over the backdrop");
        assert_eq!(m.solid() >> 24, 255, "{m:?} has an opaque fallback");
    }
}

#[test]
fn a_selection_tint_depends_on_emphasis_and_window_activity() {
    let normal = Material::Selection.tint_for(false, true);
    assert_ne!(Material::Selection.tint_for(true, true), normal, "an emphasized selection is the full accent");
    assert_ne!(Material::Selection.tint_for(false, false), normal, "an inactive selection is neutral grey");
}

#[test]
fn materials_other_than_the_selection_ignore_emphasis_and_activity() {
    for m in Material::ALL.into_iter().filter(|m| *m != Material::Selection) {
        assert_eq!(m.tint_for(true, false), m.tint_for(false, true), "{m:?}");
    }
}

#[test]
fn the_default_material_state_follows_the_window() {
    assert_eq!(State::default(), State::FollowsWindowActiveState);
    assert!(State::FollowsWindowActiveState.looks_active(true));
    assert!(!State::FollowsWindowActiveState.looks_active(false));
    assert!(State::Active.looks_active(false), "an active state stays active in a background window");
    assert!(!State::Inactive.looks_active(true), "an inactive state stays opaque in the key window");
}

#[test]
fn the_default_blending_is_behind_the_window() {
    assert_eq!(Blending::default(), Blending::BehindWindow);
    assert_ne!(Blending::WithinWindow, Blending::BehindWindow);
}

#[test]
fn headless_a_within_window_material_blurs_only_inside_its_bounds() {
    let ui = ui();
    let mut px = stripes();
    let before = px.clone();
    let b = rect(8.0, 8.0, 48.0, 48.0);
    widgets::visual_effect(&mut surface(&mut px), &ui, b, 6.0, Material::Titlebar, Blending::WithinWindow, State::Active);
    assert_eq!(px[0], before[0], "outside the material nothing changes");
    assert_eq!(px[63 * W + 63], before[63 * W + 63], "the far corner is outside the material");
    let mid = px[32 * W + 32];
    assert!(mid != pack(255, 255, 255, 255) && mid != pack(0, 0, 0, 255), "the stripes are blurred away: {mid:08x}");
    let (r, g, b) = ((mid >> 16) & 255, (mid >> 8) & 255, mid & 255);
    assert!(r < 128 && g < 128 && b < 128, "the dark tint pulls the blurred grey down");
}

#[test]
fn headless_a_behind_window_material_with_vibrancy_off_paints_its_opaque_colour() {
    let ui = ui();
    widgets::set_vibrancy(false, false);
    let _ = widgets::take_frame_materials();
    let mut px = backdrop();
    let b = rect(10.0, 10.0, 40.0, 40.0);
    widgets::visual_effect(&mut surface(&mut px), &ui, b, 12.0, Material::Popover, Blending::BehindWindow, State::Active);
    assert_eq!(px[30 * W + 30], Material::Popover.solid(), "the centre is the opaque colour");
    assert_eq!(px[4 * W + 4], BACKDROP, "the rounded corner stays outside the material");
    assert_eq!(px[0], BACKDROP, "the area outside the bounds stays untouched");
    assert!(widgets::take_frame_materials().is_empty(), "nothing is declared to the shell without vibrancy");
}

#[test]
fn headless_a_behind_window_material_with_vibrancy_on_writes_its_tint_and_declares_its_frame() {
    let ui = ui();
    widgets::set_vibrancy(true, true);
    let _ = widgets::take_frame_materials();
    let mut px = backdrop();
    let b = rect(10.0, 10.0, 40.0, 40.0);
    widgets::visual_effect(&mut surface(&mut px), &ui, b, 12.0, Material::Sidebar, Blending::BehindWindow, State::Inactive);
    assert_eq!(px[30 * W + 30], Material::Sidebar.tint(), "the inside is replaced by the translucent tint");
    assert!(px[30 * W + 30] >> 24 < 255, "the tint lets the blurred desktop show through");
    let frames = widgets::take_frame_materials();
    assert_eq!(frames.len(), 1, "one region is declared");
    let f = &frames[0];
    assert_eq!(f.kind, MaterialKind::Sidebar);
    assert_eq!(f.state, MaterialState::Inactive);
    assert_eq!(f.blending, Blending::BehindWindow);
    assert_eq!(f.rect, b);
    assert!(!f.emphasized);
    widgets::set_vibrancy(false, false);
}

#[test]
fn headless_an_older_shell_receives_legacy_kinds_and_the_default_state() {
    let ui = ui();
    widgets::set_vibrancy(true, false);
    let _ = widgets::take_frame_materials();
    let mut px = backdrop();
    let b = rect(0.0, 0.0, 32.0, 32.0);
    widgets::visual_effect(&mut surface(&mut px), &ui, b, 4.0, Material::Hud, Blending::BehindWindow, State::Active);
    let frames = widgets::take_frame_materials();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].kind, MaterialKind::Titlebar, "a HUD is shown to an old shell as a title bar");
    assert_eq!(frames[0].state, MaterialState::FollowsWindowActiveState);
    widgets::set_vibrancy(false, false);
}

#[test]
fn headless_an_emphasized_selection_declares_its_emphasis_to_the_shell() {
    let ui = ui();
    widgets::set_vibrancy(true, true);
    let _ = widgets::take_frame_materials();
    let req = MaterialRequest::new(MaterialKind::Selection, rect(4.0, 4.0, 40.0, 20.0), 4.0).emphasized(true);
    let mut px = backdrop();
    widgets::visual_effect_request(&mut surface(&mut px), &ui, &req, true);
    assert_eq!(px[14 * W + 20], Material::Selection.tint_for(true, true), "the full accent tints the inside");
    let frames = widgets::take_frame_materials();
    assert_eq!(frames.len(), 1);
    assert!(frames[0].emphasized);
    widgets::set_vibrancy(false, false);
}

#[test]
fn headless_a_selection_tint_follows_the_window_activity_of_its_request() {
    let ui = ui();
    widgets::set_vibrancy(true, true);
    let req = MaterialRequest::new(MaterialKind::Selection, rect(4.0, 4.0, 40.0, 20.0), 4.0);
    let draw = |window_active: bool| {
        let _ = widgets::take_frame_materials();
        let mut px = backdrop();
        widgets::visual_effect_request(&mut surface(&mut px), &ui, &req, window_active);
        px[14 * W + 20]
    };
    let (active, inactive) = (draw(true), draw(false));
    assert_eq!(active, Material::Selection.tint_for(false, true));
    assert_eq!(inactive, Material::Selection.tint_for(false, false));
    assert_ne!(active, inactive, "an inactive window draws the neutral selection");
    widgets::set_vibrancy(false, false);
}

#[test]
fn headless_a_request_that_blends_within_the_window_draws_no_shell_region() {
    let ui = ui();
    widgets::set_vibrancy(true, true);
    let _ = widgets::take_frame_materials();
    let req = MaterialRequest::new(MaterialKind::HudWindow, rect(8.0, 8.0, 48.0, 48.0), 4.0).blending(Blending::WithinWindow);
    let mut px = stripes();
    let before = px.clone();
    widgets::visual_effect_request(&mut surface(&mut px), &ui, &req, true);
    assert!(widgets::take_frame_materials().is_empty(), "within-window blending is not sent to the shell");
    assert_eq!(px[0], before[0], "outside the material nothing changes");
    assert_ne!(px[32 * W + 32], before[32 * W + 32], "the content under the material is blurred");
    widgets::set_vibrancy(false, false);
}

// Credential exchange: the document format for password records (`bifrost_app::credential_exchange`).

fn record(service: &str, user: &str, password: &str) -> PasswordRecord {
    PasswordRecord { service: service.to_string(), user: user.to_string(), password: password.to_string() }
}

#[test]
fn headless_an_export_writes_the_header_and_one_line_per_record_in_order() {
    let text = export(&[record("example.com", "ann", "pw1"), record("example.org", "bob", "pw2")]);
    assert_eq!(text, "bifrost-credential-exchange 1\nexample.com\tann\tpw1\nexample.org\tbob\tpw2\n");
    assert_eq!(export(&[]), format!("{HEADER}\n"), "no records leaves the header alone");
}

#[test]
fn headless_escape_keeps_a_field_on_one_line_and_unescape_restores_it() {
    let raw = "a\\b\tc\nd\re";
    let escaped = escape(raw);
    assert_eq!(escaped, "a\\\\b\\tc\\nd\\re");
    assert!(!escaped.contains('\t') && !escaped.contains('\n') && !escaped.contains('\r'));
    assert_eq!(unescape(&escaped).as_deref(), Some(raw));
}

#[test]
fn headless_unescape_refuses_an_unknown_escape_and_a_trailing_backslash() {
    assert_eq!(unescape("bad\\x"), None);
    assert_eq!(unescape("end\\"), None);
    assert_eq!(unescape("plain"), Some("plain".to_string()));
}

#[test]
fn headless_an_exported_document_imports_back_to_the_same_records() {
    let records = vec![record("mail.example", "ann", "tab\there\nnew line"), record("bank.example", "bob", "")];
    let report = import(&export(&records)).unwrap();
    assert_eq!(report.records, records, "an empty password is a record too");
    assert_eq!(report.duplicates, 0);
}

#[test]
fn headless_import_skips_blank_lines() {
    let doc = format!("{HEADER}\n\nx.example\tu\tp\n\n");
    let report = import(&doc).unwrap();
    assert_eq!(report.records, vec![record("x.example", "u", "p")]);
}

#[test]
fn headless_a_repeated_service_and_user_replaces_the_password_in_its_first_place() {
    let doc = format!("{HEADER}\nx\tu\told\ny\tu\tother\nx\tu\tnew\n");
    let report = import(&doc).unwrap();
    assert_eq!(report.records, vec![record("x", "u", "new"), record("y", "u", "other")], "the last copy wins and keeps its place");
    assert_eq!(report.duplicates, 1);
}

#[test]
fn headless_the_same_service_with_another_user_is_a_separate_record() {
    let doc = format!("{HEADER}\nx\tann\tp1\nx\tbob\tp2\n");
    let report = import(&doc).unwrap();
    assert_eq!(report.records.len(), 2);
    assert_eq!(report.duplicates, 0);
}

#[test]
fn headless_a_document_without_the_header_is_refused() {
    assert_eq!(import(""), Err(ExchangeError::MissingHeader));
    assert_eq!(import("x\tu\tp\n"), Err(ExchangeError::MissingHeader));
    assert_eq!(import("bifrost-credential-exchange\n"), Err(ExchangeError::MissingHeader), "the version number is required");
}

#[test]
fn headless_a_newer_exchange_version_is_refused_rather_than_guessed_at() {
    assert_eq!(import("bifrost-credential-exchange 2\nx\tu\tp\n"), Err(ExchangeError::UnsupportedVersion));
}

#[test]
fn headless_a_record_must_have_three_fields_and_valid_escapes() {
    assert_eq!(import(&format!("{HEADER}\nonly\ttwo\n")), Err(ExchangeError::MalformedRecord(2)));
    assert_eq!(import(&format!("{HEADER}\nx\tu\tp\nx\tu\tp\textra\n")), Err(ExchangeError::MalformedRecord(3)));
    assert_eq!(import(&format!("{HEADER}\nx\tu\tbad\\q\n")), Err(ExchangeError::MalformedRecord(2)));
}

#[test]
fn headless_a_record_needs_a_service_and_a_user_name() {
    assert_eq!(import(&format!("{HEADER}\n\tu\tp\n")), Err(ExchangeError::EmptyField(2)));
    assert_eq!(import(&format!("{HEADER}\nx\t\tp\n")), Err(ExchangeError::EmptyField(2)));
}

#[test]
fn headless_one_bad_record_refuses_the_whole_document() {
    let doc = format!("{HEADER}\nx\tu\tp\nbad line\n");
    assert!(import(&doc).is_err(), "a partial import never happens");
}

#[test]
fn the_import_errors_name_their_line() {
    assert_eq!(ExchangeError::MalformedRecord(4).to_string(), "line 4 is not a valid credential record");
    assert_eq!(ExchangeError::EmptyField(3).to_string(), "line 3 has an empty service or user name");
}
