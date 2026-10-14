//! Parity sdk-complete-11: headless tests of the authentication prompt and biometry sensors, the preview provider
//! registry's unregister, and PDF reply validation. No test shows a sheet or reads a sensor.

use bifrost_ui::auth_view::{AuthView, BiometryKind, ControlSize, Event, Prompt, PromptError, ViewState};
use bifrost_ui::preview::{Content, ModelError, Registry, Reply, Subject};

#[test]
fn a_prompt_needs_a_reason_and_a_cancel_title() {
    assert_eq!(Prompt::new("  ").unwrap_err(), PromptError::EmptyReason);
    let p = Prompt::new("Unlock the saved passwords").unwrap();
    assert_eq!(p.reason(), "Unlock the saved passwords");
    assert_eq!(p.cancel_title(), "Cancel");
    assert_eq!(p.fallback_title(), Some("Enter Password"));
    assert_eq!(p.clone().with_cancel_title(" ").unwrap_err(), PromptError::EmptyCancelTitle);
    assert_eq!(p.with_cancel_title("Not now").unwrap().cancel_title(), "Not now");
}

#[test]
fn an_empty_fallback_title_hides_the_fallback_button() {
    let hidden = Prompt::new("Confirm the purchase").unwrap().with_fallback_title("");
    assert_eq!(hidden.fallback_title(), None);
    let shown = hidden.with_fallback_title("Use Passcode");
    assert_eq!(shown.fallback_title(), Some("Use Passcode"));
}

#[test]
fn a_sensor_name_picks_its_biometry_kind_and_nothing_else() {
    assert_eq!(BiometryKind::from_sensor_name("face"), Some(BiometryKind::Face));
    assert_eq!(BiometryKind::from_sensor_name("touch"), Some(BiometryKind::Touch));
    assert_eq!(BiometryKind::from_sensor_name("optic"), Some(BiometryKind::Optic));
    assert_eq!(BiometryKind::from_sensor_name("none"), Some(BiometryKind::None));
    assert_eq!(BiometryKind::from_sensor_name("Face"), None);
}

#[test]
fn an_auth_view_follows_a_failed_then_successful_attempt() {
    let kind = BiometryKind::from_sensor_name("face").unwrap();
    let mut v = AuthView::new(kind, ControlSize::Regular);
    assert_eq!(v.icon(), "face");
    v.send(Event::Start);
    assert_eq!(v.state(), ViewState::Evaluating);
    v.send(Event::Failed);
    v.send(Event::Start);
    v.send(Event::Succeeded);
    assert_eq!(v.state(), ViewState::Succeeded);
    assert_eq!(v.icon(), "checkmark.circle");
    assert_eq!(v.points(), 24.0);
}

#[test]
fn unregistering_a_provider_removes_its_types_and_domains() {
    let mut reg = Registry::new();
    reg.register("md", &["net.daringfireball.markdown"]).unwrap();
    reg.register_searchable("md", "com.example.notes").unwrap();
    reg.register("text", &["public.plain-text"]).unwrap();
    assert!(reg.unregister("md"));
    assert_eq!(reg.provider_for("README.md"), None);
    assert_eq!(reg.provider_for("todo.txt"), Some("text"));
    let note = Subject::searchable("com.example.notes/1", None).unwrap();
    assert_eq!(reg.provider_for_subject(&note), None);
    assert!(!reg.unregister("md"), "a second removal finds nothing");
}

#[test]
fn a_pdf_reply_keeps_its_page_size_and_refuses_an_empty_page() {
    let reply = Reply::new(Content::Pdf { width: 612.0, height: 792.0 }).unwrap();
    assert_eq!(reply.content(), &Content::Pdf { width: 612.0, height: 792.0 });
    assert_eq!(Reply::new(Content::Pdf { width: 612.0, height: 0.0 }), Err(ModelError::BadSize));
}
