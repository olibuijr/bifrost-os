//! Contract tests: Codable and NotificationCenter parity.
use bifrost_app::coding::*;
use bifrost_app::{DistributedNotificationCenter, NotificationCenter};
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
struct Item {
    name: String,
    count: u32,
    ratio: f64,
    tags: Vec<String>,
    note: Option<String>,
    when: Date,
    extra: BTreeMap<String, i64>,
}
bifrost_app::codable!(Item { name: String, count: u32, ratio: f64, tags: Vec<String>, note: Option<String>, when: Date, extra: BTreeMap<String, i64> });

fn item() -> Item {
    Item {
        name: "a \"q\" <b> & c".into(),
        count: 7,
        ratio: 0.5,
        tags: vec!["x".into(), "y".into()],
        note: None,
        when: Date(86400.0 + 3661.0),
        extra: [("z".to_string(), 1), ("a".to_string(), 2)].into_iter().collect(),
    }
}

#[test]
fn json_round_trips_and_formats() {
    let mut e = JSONEncoder::new();
    e.sorted_keys = true;
    let s = e.encode(&item());
    assert!(s.starts_with("{\"count\":7,\"extra\":{\"a\":2,\"z\":1},\"name\""), "{s}");
    assert!(!s.contains("note"), "None is left out");
    assert_eq!(JSONDecoder::new().decode::<Item>(&s).unwrap(), item());
    e.pretty_printed = true;
    assert!(e.encode(&vec![1u8]).contains("[\n  1\n]"));
    e.date_encoding_strategy = DateEncodingStrategy::Iso8601;
    let iso = e.encode(&item());
    assert!(iso.contains("\"1970-01-02T01:01:01Z\""), "{iso}");
    assert_eq!(JSONDecoder::new().decode::<Item>(&iso).unwrap(), item());
    assert!(JSONDecoder::new().decode::<Item>("{\"name\":1}").is_err());
}

#[test]
fn plist_round_trips() {
    let x = PropertyListEncoder::new().encode(&item());
    assert!(x.contains("<key>count</key>\n\t<integer>7</integer>") && x.contains("&lt;b&gt; &amp; c"), "{x}");
    assert_eq!(PropertyListDecoder::new().decode::<Item>(&x).unwrap(), item());
    let mut m: HashMap<String, Vec<bool>> = HashMap::new();
    m.insert("k".into(), vec![true, false]);
    let x = PropertyListEncoder::new().encode(&m);
    assert_eq!(PropertyListDecoder::new().decode::<HashMap<String, Vec<bool>>>(&x).unwrap(), m);
}

#[test]
fn iso8601_known_values() {
    assert_eq!(format_iso8601(0.0), "1970-01-01T00:00:00Z");
    assert_eq!(parse_iso8601("2000-03-01T00:00:00Z"), Some(951868800.0));
    assert_eq!(parse_iso8601("2000-03-01T02:00:00+02:00"), Some(951868800.0));
}

#[test]
fn notifications_filter_by_name_and_object_and_drop_removes() {
    let nc = NotificationCenter::default();
    let all = Rc::new(Cell::new(0));
    let one = Rc::new(Cell::new(0));
    let (a, o) = (all.clone(), one.clone());
    let t_all = nc.add_observer("n.test", None, move |n| {
        assert_eq!(n.user_info.get("k").map(String::as_str), Some("v"));
        a.set(a.get() + 1)
    });
    let t_one = nc.add_observer("n.test", Some(5), move |_| o.set(o.get() + 1));
    let info: HashMap<String, String> = [("k".to_string(), "v".to_string())].into_iter().collect();
    nc.post("n.test", Some(5), info.clone());
    nc.post("n.test", Some(6), info.clone());
    nc.post("n.other", Some(5), info.clone());
    assert_eq!((all.get(), one.get()), (2, 1));
    drop(t_one);
    nc.post("n.test", Some(5), info.clone());
    assert_eq!((all.get(), one.get()), (3, 1));
    nc.remove_observer(t_all);
    nc.post("n.test", Some(5), info);
    assert_eq!(all.get(), 3);
}

#[test]
fn distributed_center_is_process_local() {
    let hits = Rc::new(Cell::new(0));
    let h = hits.clone();
    let _t = DistributedNotificationCenter.add_observer("d.test", move |_| h.set(h.get() + 1));
    DistributedNotificationCenter.post("d.test", HashMap::new());
    assert_eq!(hits.get(), 1);
}

#[derive(Debug, PartialEq)]
struct Defaulted { id: String, title: String }
bifrost_app::codable!(Defaulted { id: String, title: String = "Untitled".into() });

#[test]
fn codable_field_default_fills_a_missing_key_and_a_plain_field_stays_required() {
    use bifrost_app::coding::JSONDecoder;
    let d: Defaulted = JSONDecoder::new().decode(r#"{"id":"a"}"#).unwrap();
    assert_eq!(d, Defaulted { id: "a".into(), title: "Untitled".into() });
    assert!(JSONDecoder::new().decode::<Defaulted>(r#"{"title":"x"}"#).is_err());
}
