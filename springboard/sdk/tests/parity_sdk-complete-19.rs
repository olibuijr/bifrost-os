//! Parity sdk-complete-19: the Codable `Data` form. JSON and XML carry bytes as base64, a binary property list keeps
//! them as data objects, and a struct with a `Data` field round-trips through every format. The local saving tests
//! use a temporary data directory: no account, no network, no device.

use bifrost_app::binary_plist::{self, BValue};
use bifrost_app::coding::{Data, JSONDecoder, JSONEncoder, PropertyListDecoder, PropertyListEncoder, Value};
use bifrost_app::defaults::{self, UserDefaults};
use bifrost_app::restoration::{self, RestorableState};

#[derive(Clone, Debug, PartialEq)]
struct Blob {
    name: String,
    bytes: Data,
}
bifrost_app::codable!(Blob { name: String, bytes: Data });

fn sample() -> Blob {
    Blob { name: "icon".into(), bytes: Data(vec![0, 1, 2, 254, 255]) }
}

#[test]
fn json_writes_data_as_standard_base64_text() {
    assert_eq!(JSONEncoder::new().encode(&Data(vec![0, 1, 2, 254, 255])), "\"AAEC/v8=\"");
    assert_eq!(JSONEncoder::new().encode(&Data(vec![1, 2, 3])), "\"AQID\"");
    assert_eq!(JSONEncoder::new().encode(&Data(Vec::new())), "\"\"", "empty data is an empty string");
}

#[test]
fn json_reads_base64_text_back_into_data() {
    let data: Data = JSONDecoder::new().decode("\"AAEC/v8=\"").unwrap();
    assert_eq!(data, Data(vec![0, 1, 2, 254, 255]));
}

#[test]
fn json_rejects_text_that_is_not_base64() {
    assert!(JSONDecoder::new().decode::<Data>("\"!!\"").is_err(), "an illegal character is an error");
    assert!(JSONDecoder::new().decode::<Data>("[1]").is_err(), "an array is not data");
}

#[test]
fn a_struct_with_a_data_field_round_trips_through_json() {
    let text = JSONEncoder::new().encode(&sample());
    assert_eq!(text, "{\"name\":\"icon\",\"bytes\":\"AAEC/v8=\"}");
    let back: Blob = JSONDecoder::new().decode(&text).unwrap();
    assert_eq!(back, sample());
}

#[test]
fn xml_writes_data_as_a_data_element() {
    let text = PropertyListEncoder::new().encode(&Data(vec![1, 2, 3]));
    assert!(text.contains("<data>AQID</data>"), "got: {text}");
}

#[test]
fn xml_reads_a_data_element_with_line_breaks_inside_the_base64() {
    let text = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n\t<data>\n\tAAEC\n\t/v8=\n\t</data>\n</plist>\n";
    let data: Data = PropertyListDecoder::new().decode(text).unwrap();
    assert_eq!(data, Data(vec![0, 1, 2, 254, 255]), "white space inside base64 is ignored");
}

#[test]
fn xml_reads_an_empty_data_element_as_no_bytes() {
    let text = "<plist version=\"1.0\"><data/></plist>";
    assert_eq!(PropertyListDecoder::new().decode::<Data>(text).unwrap(), Data(Vec::new()));
}

#[test]
fn a_struct_with_a_data_field_round_trips_through_xml() {
    let text = PropertyListEncoder::new().encode(&sample());
    assert!(text.contains("<key>bytes</key>"));
    let back: Blob = PropertyListDecoder::new().decode(&text).unwrap();
    assert_eq!(back, sample());
}

#[test]
fn a_binary_property_list_keeps_data_as_data_objects() {
    let bytes = PropertyListEncoder::new().encode_binary(&sample());
    let back: Blob = PropertyListDecoder::new().decode_binary(&bytes).unwrap();
    assert_eq!(back, sample());
    let generic: Blob = PropertyListDecoder::new().decode_bytes(&bytes).unwrap();
    assert_eq!(generic, sample(), "decode_bytes reads binary data fields too");
}

#[test]
fn the_value_tree_maps_data_to_and_from_the_binary_form() {
    assert_eq!(binary_plist::from_value(&Value::Data(vec![7, 8])), BValue::Data(vec![7, 8]));
    assert_eq!(binary_plist::to_value(&BValue::Data(vec![7, 8])).unwrap(), Value::Data(vec![7, 8]));
}

#[test]
fn a_uid_still_has_no_codable_form() {
    assert!(binary_plist::to_value(&BValue::Uid(3)).is_err());
}

#[test]
fn local_preferences_survive_a_relaunch_without_an_account() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut store = UserDefaults::open(dir.path());
        store.set_int("level", 7);
        store.set_string("player", "Ada");
        store.set_data("slot", &[1, 2, 255]);
    }
    let relaunched = UserDefaults::open(dir.path());
    assert_eq!(relaunched.int("level"), 7);
    assert_eq!(relaunched.string("player").as_deref(), Some("Ada"));
    assert_eq!(relaunched.data("slot"), Some(vec![1, 2, 255]));
}

#[test]
fn a_removed_local_value_stays_removed_after_a_relaunch() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = UserDefaults::open(dir.path());
    store.set_bool("music", true);
    store.remove("music");
    assert!(!UserDefaults::open(dir.path()).contains("music"));
}

#[test]
fn a_registered_default_is_not_written_to_the_save_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = UserDefaults::open(dir.path());
    store.register([("difficulty", defaults::Value::Int(2))]);
    assert_eq!(store.int("difficulty"), 2, "the fallback is read");
    assert!(!UserDefaults::open(dir.path()).contains("difficulty"), "and nothing is saved for it");
}

#[test]
fn progress_and_window_state_are_kept_in_separate_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut progress = RestorableState::new();
    progress.encode_for("level", &"forest".to_string());
    assert!(restoration::save(dir.path(), "game", (640, 480), &progress, 0).unwrap());
    UserDefaults::open(dir.path()).set_int("best", 3);

    assert_ne!(restoration::FILE, defaults::FILE, "the two files never share a name");
    assert!(dir.path().join(restoration::FILE).exists());
    assert!(dir.path().join(defaults::FILE).exists());
    let (state, size) = restoration::load(dir.path(), "game").expect("the saved window comes back");
    assert_eq!(state.decode_for::<String>("level").as_deref(), Some("forest"));
    assert_eq!(size, (640, 480));
}

#[test]
fn restored_state_keeps_data_fields() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = RestorableState::new();
    state.encode_for("thumbnail", &bifrost_app::coding::Data(vec![9, 8, 0]));
    restoration::save(dir.path(), "viewer", (320, 200), &state, 0).unwrap();
    let (back, _) = restoration::load(dir.path(), "viewer").expect("state comes back");
    assert_eq!(back.decode_for::<bifrost_app::coding::Data>("thumbnail"), Some(bifrost_app::coding::Data(vec![9, 8, 0])));
}

#[test]
fn an_unreadable_window_file_does_not_cost_the_preferences() {
    let dir = tempfile::tempdir().unwrap();
    UserDefaults::open(dir.path()).set_string("theme", "dark");
    std::fs::write(restoration::path(dir.path()), "{ not json").unwrap();
    assert!(restoration::load(dir.path(), "main").is_none(), "the broken window state is ignored");
    assert_eq!(UserDefaults::open(dir.path()).string("theme").as_deref(), Some("dark"));
}
