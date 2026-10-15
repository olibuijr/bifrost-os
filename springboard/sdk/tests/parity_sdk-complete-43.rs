//! Parity sdk-complete-43: headless tests of the device event notices and the event-model notices. No device is opened;
//! the tests build events by hand and check the text and the authorization gate.

use bifrost_app::bluetooth_events::{notices_for_registration, notice_for, DeviceEvent, DeviceEventKind, DeviceNotice};
use bifrost_app::usernotif_model::AuthorizationStatus;
use bifrost_core::bt_address::BdAddr;

fn event(kind: DeviceEventKind, last: u8) -> DeviceEvent {
    DeviceEvent { kind, address: BdAddr([0x00, 0x1A, 0x7D, 0xDA, 0x71, last]) }
}

#[test]
fn a_connect_notice_names_the_device_by_its_address_when_no_name_is_known() {
    let ev = event(DeviceEventKind::Connected, 0x13);
    let notice = notice_for(&ev, None);
    assert_eq!(notice.title, "Bluetooth device connected");
    assert_eq!(notice.body, ev.address.to_string());
}

#[test]
fn a_disconnect_notice_says_the_device_disconnected() {
    let notice = notice_for(&event(DeviceEventKind::Disconnected, 1), None);
    assert_eq!(notice.title, "Bluetooth device disconnected");
}

#[test]
fn a_known_device_name_replaces_the_address_in_the_body() {
    let notice = notice_for(&event(DeviceEventKind::Connected, 2), Some("  Keyboard  "));
    assert_eq!(notice.body, "Keyboard", "surrounding spaces are trimmed");
}

#[test]
fn a_blank_device_name_falls_back_to_the_address() {
    let ev = event(DeviceEventKind::Connected, 3);
    assert_eq!(notice_for(&ev, Some("   ")).body, ev.address.to_string());
}

#[test]
fn an_authorized_app_gets_one_notice_per_event_in_order() {
    let events = vec![event(DeviceEventKind::Connected, 4), event(DeviceEventKind::Disconnected, 5)];
    let notices = notices_for_registration(events.clone(), AuthorizationStatus::Authorized, |_| None);
    let expected: Vec<DeviceNotice> = events.iter().map(|e| notice_for(e, None)).collect();
    assert_eq!(notices, expected);
}

#[test]
fn a_provisional_app_is_also_allowed_to_post() {
    let events = vec![event(DeviceEventKind::Connected, 6)];
    let notices = notices_for_registration(events, AuthorizationStatus::Provisional, |_| None);
    assert_eq!(notices.len(), 1);
}

#[test]
fn an_app_that_was_not_authorized_gets_no_notices() {
    for status in [AuthorizationStatus::NotDetermined, AuthorizationStatus::Denied] {
        let events = vec![event(DeviceEventKind::Connected, 7)];
        assert!(notices_for_registration(events, status, |_| None).is_empty(), "{status:?} posts nothing");
    }
}

#[test]
fn the_name_lookup_is_asked_for_each_device_address() {
    let events = vec![event(DeviceEventKind::Connected, 8), event(DeviceEventKind::Connected, 9)];
    let notices = notices_for_registration(events, AuthorizationStatus::Authorized, |addr| {
        (addr.0[5] == 8).then(|| "Mouse".to_string())
    });
    assert_eq!(notices[0].body, "Mouse");
    assert_eq!(notices[1].body, BdAddr([0x00, 0x1A, 0x7D, 0xDA, 0x71, 9]).to_string());
}

mod js {
    use bifrost_app::js_value::JsValue;
    use bifrost_app::webview_dev::EvalValue;

    fn eval(ty: &str, json: &str) -> EvalValue {
        EvalValue { ty: ty.into(), json: json.into() }
    }

    #[test]
    fn a_number_result_reads_as_a_number() {
        assert_eq!(JsValue::from_eval(&eval("number", "42")), Ok(JsValue::Number(42.0)));
    }

    #[test]
    fn a_string_result_decodes_its_escapes() {
        assert_eq!(JsValue::from_eval(&eval("string", r#""a\nb \"q\"""#)), Ok(JsValue::String("a\nb \"q\"".into())));
    }

    #[test]
    fn an_undefined_result_reads_as_undefined_whatever_its_text() {
        assert_eq!(JsValue::from_eval(&eval("undefined", "null")), Ok(JsValue::Undefined));
    }

    #[test]
    fn a_nested_array_and_object_keep_their_order_and_types() {
        let value = eval("array", r#"[1,"x",{"k":true,"n":null}]"#);
        let expected = JsValue::Array(vec![
            JsValue::Number(1.0),
            JsValue::String("x".into()),
            JsValue::Object(vec![("k".into(), JsValue::Bool(true)), ("n".into(), JsValue::Null)]),
        ]);
        assert_eq!(JsValue::from_eval(&value), Ok(expected));
    }

    #[test]
    fn a_result_whose_text_does_not_match_its_type_is_refused() {
        assert!(JsValue::from_eval(&eval("number", r#""7""#)).is_err());
        assert!(JsValue::from_eval(&eval("object", "[1]")).is_err());
    }

    #[test]
    fn a_dom_element_and_an_unknown_type_have_no_value_form() {
        assert!(JsValue::from_eval(&eval("element", "{}")).unwrap_err().contains("element"));
        assert!(JsValue::from_eval(&eval("symbol", "1")).unwrap_err().contains("symbol"));
    }

    #[test]
    fn a_field_of_an_object_is_found_by_name() {
        let obj = JsValue::Object(vec![("title".into(), JsValue::String("Hi".into()))]);
        assert_eq!(obj.get("title"), Some(&JsValue::String("Hi".into())));
        assert_eq!(obj.get("missing"), None);
        assert_eq!(JsValue::Number(1.0).get("title"), None);
    }

    #[test]
    fn values_become_script_literals_that_quote_their_strings() {
        let value = JsValue::Object(vec![
            ("a".into(), JsValue::Number(1.5)),
            ("b".into(), JsValue::String("q\"x".into())),
        ]);
        assert_eq!(value.to_literal().unwrap(), r#"{"a":1.5,"b":"q\"x"}"#);
        let list = JsValue::Array(vec![JsValue::Null, JsValue::Bool(true), JsValue::Undefined]);
        assert_eq!(list.to_literal().unwrap(), "[null,true,undefined]");
    }

    #[test]
    fn a_non_finite_number_has_no_script_literal() {
        assert!(JsValue::Number(f64::NAN).to_literal().is_err());
        assert!(JsValue::Array(vec![JsValue::Number(f64::INFINITY)]).to_literal().is_err());
    }
}
