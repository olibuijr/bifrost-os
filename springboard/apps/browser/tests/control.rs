//! The Browser's `browser.*` control methods, pure layer: parameter validation and error codes, result shaping,
//! the console and network logs (apps/browser/README.md "Web Inspector"). The modules are compiled straight from
//! src/ because the app is a binary crate; they depend only on the SDK and core.
#![allow(dead_code)]

#[path = "../src/control.rs"]
mod control;
#[path = "../src/network.rs"]
mod network;

use std::collections::VecDeque;

use bifrost_app::control::{INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_core::json::{self, Json};
use control::{parse, DomMode, FindDir, Input, Panel, Req, Wait, WaitFor};

fn p(text: &str) -> Json {
    json::parse(text).expect("test json")
}

fn code(method: &str, params: &str) -> i64 {
    parse(method, &p(params)).expect_err("should be rejected").code
}

fn ok(method: &str, params: &str) -> Req {
    parse(method, &p(params)).expect("should parse")
}

#[test]
fn unknown_methods_are_not_found() {
    assert_eq!(code("browser.nope", "{}"), METHOD_NOT_FOUND);
    assert_eq!(code("nope", "{}"), METHOD_NOT_FOUND);
    for m in control::METHODS {
        // every listed method is known (it may still need parameters)
        let e = parse(m, &p("{}")).err();
        assert!(e.is_none_or(|e| e.code != METHOD_NOT_FOUND), "{m} is listed but not handled");
    }
}

#[test]
fn console_is_an_alias_of_browser_console() {
    assert_eq!(ok("console", "{}"), ok("browser.console", "{}"));
    match ok("console", r#"{"level":"error","limit":5,"since_seq":9,"clear":true,"id":4}"#) {
        Req::Console { id, level, limit, since_seq, clear } => {
            assert_eq!((id, level.as_deref(), limit, since_seq, clear), (Some(4), Some("error"), 5, Some(9), true));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn defaults_follow_the_contract() {
    assert_eq!(ok("browser.console", "{}"), Req::Console { id: None, level: None, limit: 200, since_seq: None, clear: false });
    match ok("browser.eval", r#"{"js":"1+1"}"#) {
        Req::Eval { timeout_ms, .. } => assert_eq!(timeout_ms, 10_000),
        other => panic!("{other:?}"),
    }
    match ok("browser.dom", "{}") {
        Req::Dom { selector, mode, limit, max_bytes, .. } => {
            assert_eq!((selector.as_str(), mode, limit, max_bytes), ("html", DomMode::OuterHtml, 200, 200_000));
        }
        other => panic!("{other:?}"),
    }
    match ok("browser.navigate", r#"{"url":"example.com"}"#) {
        Req::Navigate { wait, .. } => assert_eq!(wait, Wait::Load),
        other => panic!("{other:?}"),
    }
    match ok("browser.network", "{}") {
        Req::Network { limit, since_seq, clear, .. } => assert_eq!((limit, since_seq, clear), (200, None, false)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn tab_ids_must_be_unsigned_integers() {
    for bad in [r#"{"id":-1}"#, r#"{"id":1.5}"#, r#"{"id":"3"}"#, r#"{"id":4294967296}"#, r#"{"id":true}"#] {
        assert_eq!(code("browser.back", bad), INVALID_PARAMS, "{bad}");
    }
    assert_eq!(ok("browser.back", r#"{"id":7}"#), Req::Back { id: Some(7) });
    assert_eq!(ok("browser.back", r#"{"id":null}"#), Req::Back { id: None });
    assert_eq!(code("browser.tab.select", "{}"), INVALID_PARAMS);
    assert_eq!(code("browser.tab.close", "{}"), INVALID_PARAMS);
    assert_eq!(ok("browser.tab.select", r#"{"id":2}"#), Req::TabSelect { id: 2 });
}

#[test]
fn params_must_be_an_object() {
    assert_eq!(code("browser.state", "[]"), INVALID_PARAMS);
    assert_eq!(code("browser.state", "3"), INVALID_PARAMS);
    assert_eq!(ok("browser.state", "null"), Req::State);
}

#[test]
fn timeouts_are_bounded() {
    for bad in ["0", "60001", "-5", "1.5", r#""x""#] {
        assert_eq!(code("browser.eval", &format!(r#"{{"js":"1","timeout_ms":{bad}}}"#)), INVALID_PARAMS, "{bad}");
    }
    match ok("browser.eval", r#"{"js":"1","timeout_ms":60000}"#) {
        Req::Eval { timeout_ms, .. } => assert_eq!(timeout_ms, 60_000),
        other => panic!("{other:?}"),
    }
}

#[test]
fn navigate_and_eval_need_their_text() {
    assert_eq!(code("browser.navigate", "{}"), INVALID_PARAMS);
    assert_eq!(code("browser.navigate", r#"{"url":""}"#), INVALID_PARAMS);
    assert_eq!(code("browser.navigate", r#"{"url":"a.com","wait":"later"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.eval", "{}"), INVALID_PARAMS);
    assert_eq!(code("browser.eval", r#"{"js":3}"#), INVALID_PARAMS);
    match ok("browser.navigate", r#"{"url":"a.com","wait":"commit","timeout_ms":500}"#) {
        Req::Navigate { wait, timeout_ms, .. } => assert_eq!((wait, timeout_ms), (Wait::Commit, 500)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn console_levels_are_checked() {
    assert_eq!(code("browser.console", r#"{"level":"fatal"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.console", r#"{"limit":-1}"#), INVALID_PARAMS);
    assert_eq!(code("browser.console", r#"{"clear":"yes"}"#), INVALID_PARAMS);
    for l in ["error", "warn", "info", "log", "debug"] {
        ok("browser.console", &format!(r#"{{"level":"{l}"}}"#));
    }
}

#[test]
fn dom_modes_and_screenshot_options() {
    assert_eq!(code("browser.dom", r#"{"mode":"xml"}"#), INVALID_PARAMS);
    assert!(matches!(ok("browser.dom", r#"{"mode":"query","selector":"a.x","limit":3}"#), Req::Dom { mode: DomMode::Query, limit: 3, .. }));
    assert_eq!(code("browser.screenshot", r#"{"full_page":true,"selector":"p"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.screenshot", r#"{"selector":"p","rect":{"x":0,"y":0,"w":1,"h":1}}"#), INVALID_PARAMS);
    assert_eq!(code("browser.screenshot", r#"{"rect":{"x":0,"y":0,"w":0,"h":10}}"#), INVALID_PARAMS);
    assert_eq!(code("browser.screenshot", r#"{"rect":{"x":0,"y":0}}"#), INVALID_PARAMS);
    assert_eq!(code("browser.screenshot", r#"{"format":"jpeg"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.screenshot", r#"{"scale":9}"#), INVALID_PARAMS);
    match ok("browser.screenshot", r#"{"full_page":true,"max_pixels":1000000}"#) {
        Req::Screenshot { full_page, max_pixels, .. } => assert_eq!((full_page, max_pixels), (true, 1_000_000)),
        other => panic!("{other:?}"),
    }
    match ok("browser.screenshot", "{}") {
        Req::Screenshot { max_pixels, scale, .. } => assert_eq!((max_pixels, scale), (control::MAX_SHOT_PIXELS, 1.0)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn wait_conditions_need_their_arguments() {
    assert_eq!(code("browser.wait", "{}"), INVALID_PARAMS);
    assert_eq!(code("browser.wait", r#"{"for":"selector"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.wait", r#"{"for":"js"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.wait", r#"{"for":"text"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.wait", r#"{"for":"never"}"#), INVALID_PARAMS);
    assert!(matches!(ok("browser.wait", r##"{"for":"selector","selector":"#a"}"##), Req::Wait { what: WaitFor::Selector(_), .. }));
    assert!(matches!(ok("browser.wait", r#"{"for":"idle","timeout_ms":2000}"#), Req::Wait { what: WaitFor::Idle, timeout_ms: 2000, .. }));
    assert!(matches!(ok("browser.wait", r#"{"for":"console","text":"ready"}"#), Req::Wait { what: WaitFor::Console(Some(_)), .. }));
}

#[test]
fn viewport_zoom_find_and_user_agent() {
    assert_eq!(code("browser.viewport", r#"{"width":400}"#), INVALID_PARAMS);
    assert_eq!(code("browser.viewport", r#"{"width":0,"height":10}"#), INVALID_PARAMS);
    assert_eq!(code("browser.viewport", r#"{"width":9000,"height":10}"#), INVALID_PARAMS);
    assert_eq!(code("browser.viewport", r#"{"preset":"fridge"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.viewport", r#"{"reset":true,"width":10,"height":10}"#), INVALID_PARAMS);
    assert_eq!(code("browser.viewport", r#"{"dpr":9}"#), INVALID_PARAMS);
    assert_eq!(ok("browser.viewport", r#"{"preset":"mobile"}"#), Req::Viewport { size: Some((393, 852)), dpr: Some(2.0), reset: false });
    assert_eq!(ok("browser.viewport", r#"{"width":500,"height":300,"dpr":1}"#), Req::Viewport { size: Some((500, 300)), dpr: Some(1.0), reset: false });
    assert_eq!(ok("browser.viewport", r#"{"reset":true}"#), Req::Viewport { size: None, dpr: None, reset: true });

    assert_eq!(code("browser.zoom", r#"{"factor":0.1}"#), INVALID_PARAMS);
    assert_eq!(code("browser.zoom", r#"{"factor":6}"#), INVALID_PARAMS);
    assert_eq!(ok("browser.zoom", "{}"), Req::Zoom { factor: None });

    assert_eq!(code("browser.find", "{}"), INVALID_PARAMS);
    assert_eq!(code("browser.find", r#"{"query":"x","dir":"z"}"#), INVALID_PARAMS);
    assert_eq!(ok("browser.find", r#"{"query":"x","dir":"b"}"#), Req::Find { id: None, query: "x".into(), dir: FindDir::Back });
    assert_eq!(ok("browser.find", r#"{"dir":"c"}"#), Req::Find { id: None, query: String::new(), dir: FindDir::Clear });

    assert_eq!(ok("browser.user_agent", "{}"), Req::UserAgent { ua: None });
    assert_eq!(code("browser.user_agent", r#"{"ua":5}"#), INVALID_PARAMS);
}

#[test]
fn input_kinds_are_validated() {
    assert_eq!(code("browser.input", "{}"), INVALID_PARAMS);
    assert_eq!(code("browser.input", r#"{"kind":"drag"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.input", r#"{"kind":"click"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.input", r#"{"kind":"click","x":10}"#), INVALID_PARAMS);
    assert_eq!(code("browser.input", r#"{"kind":"click","x":-1,"y":3}"#), INVALID_PARAMS);
    assert_eq!(code("browser.input", r#"{"kind":"click","x":1,"y":3,"button":"right"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.input", r#"{"kind":"key"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.input", r#"{"kind":"key","key":"NotAKey"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.input", r#"{"kind":"key","key":"a","mods":["hyper"]}"#), INVALID_PARAMS);
    assert_eq!(code("browser.input", r#"{"kind":"text"}"#), INVALID_PARAMS);
    assert_eq!(code("browser.input", r#"{"kind":"scroll"}"#), INVALID_PARAMS);
    assert_eq!(ok("browser.input", r#"{"kind":"click","x":10,"y":20}"#), Req::Input { id: None, input: Input::Click { x: 10.0, y: 20.0 } });
    assert_eq!(ok("browser.input", r#"{"kind":"key","key":"Enter"}"#), Req::Input { id: None, input: Input::Key { key: "Enter".into(), mods: 0 } });
    assert_eq!(ok("browser.input", r#"{"kind":"key","key":"a","mods":["ctrl","shift"]}"#), Req::Input { id: None, input: Input::Key { key: "a".into(), mods: 3 } });
    assert_eq!(ok("browser.input", r#"{"kind":"scroll","dy":120}"#), Req::Input { id: None, input: Input::Scroll { dx: 0.0, dy: 120.0, x: -1.0, y: -1.0 } });
}

#[test]
fn devtools_panels() {
    assert_eq!(code("browser.devtools", r#"{"panel":"timeline"}"#), INVALID_PARAMS);
    assert_eq!(ok("browser.devtools", r#"{"panel":"sources"}"#), Req::Devtools { open: None, panel: Some(Panel::Sources) });
    assert_eq!(code("browser.devtools", r#"{"open":"yes"}"#), INVALID_PARAMS);
    assert_eq!(ok("browser.devtools", r#"{"open":true,"panel":"network"}"#), Req::Devtools { open: Some(true), panel: Some(Panel::Network) });
    assert_eq!(ok("browser.devtools", "{}"), Req::Devtools { open: None, panel: None });
}

#[test]
fn tab_methods() {
    assert_eq!(ok("browser.tab.new", "{}"), Req::TabNew { url: None, background: false });
    assert_eq!(ok("browser.tab.new", r#"{"url":"a.com","background":true}"#), Req::TabNew { url: Some("a.com".into()), background: true });
    assert_eq!(code("browser.tab.new", r#"{"background":1}"#), INVALID_PARAMS);
    assert_eq!(ok("browser.tab.reopen", "{}"), Req::TabReopen);
    assert_eq!(ok("browser.reload", r#"{"hard":true}"#), Req::Reload { id: None, hard: true });
}

#[test]
fn error_helpers_use_the_contract_codes() {
    assert_eq!(control::no_tab(3).code, -32001);
    let e = control::engine_down("exited");
    assert_eq!(e.code, -32002);
    assert_eq!(e.reason.as_deref(), Some("exited"));
    assert_eq!(control::invalid("x").code, INVALID_PARAMS);
}

fn rec(seq: u64, level: &str, text: &str) -> control::Rec {
    control::Rec { seq, ts_ms: 1000 + seq, level: level.into(), text: text.into(), source: String::new(), line: 0 }
}

#[test]
fn console_query_pages_by_seq_and_filters_by_level() {
    let recs: VecDeque<_> = (1..=6).map(|n| rec(n, if n % 2 == 0 { "error" } else { "log" }, &format!("t{n}"))).collect();
    let all = control::console_query(&recs, 6, None, 200, None);
    assert_eq!(all.get("next_seq").and_then(Json::as_u64), Some(6));
    assert_eq!(all.get("entries").and_then(Json::as_array).map(|e| e.len()), Some(6));
    let errors = control::console_query(&recs, 6, Some("error"), 200, None);
    assert_eq!(errors.get("entries").and_then(Json::as_array).map(|e| e.len()), Some(3));
    // the newest two, oldest first
    let last = control::console_query(&recs, 6, None, 2, None);
    let seqs: Vec<u64> = last.get("entries").and_then(Json::as_array).unwrap().iter().filter_map(|e| e.get("seq").and_then(Json::as_u64)).collect();
    assert_eq!(seqs, vec![5, 6]);
    let since = control::console_query(&recs, 6, None, 200, Some(4));
    assert_eq!(since.get("entries").and_then(Json::as_array).map(|e| e.len()), Some(2));
    let one = control::console_query(&recs, 6, None, 1, Some(5));
    let first = &one.get("entries").and_then(Json::as_array).unwrap()[0];
    for key in ["seq", "ts_ms", "level", "text", "source", "line"] {
        assert!(first.get(key).is_some(), "missing {key}");
    }
}

#[test]
fn js_snippets_are_single_line_and_quote_their_input() {
    let hostile = "a\"b'c\nd\\e</script>";
    let snippets = [
        control::js_dom(hostile, DomMode::Query, 10, 100),
        control::js_element_rect(hostile),
        control::js_tree(hostile, 100),
        control::js_wait_poll(&WaitFor::Text(hostile.into())).unwrap(),
        control::js_wait_poll(&WaitFor::Selector(hostile.into())).unwrap(),
        control::js_wait_poll(&WaitFor::Js(hostile.into())).unwrap(),
        control::js_wait_poll(&WaitFor::Idle).unwrap(),
        control::JS_SOURCE.to_string(),
        control::JS_VIEW_INFO.to_string(),
        network::COLLECTOR_JS.to_string(),
        network::drain_js(),
    ];
    for s in &snippets {
        assert!(!s.contains('\n'), "snippet has a raw newline: {s}");
        assert!(!s.contains("//"), "snippet has a line comment: {s}");
    }
    assert!(snippets[0].contains(r#""a\"b'c\nd\\e</script>""#));
    assert!(control::js_wait_poll(&WaitFor::Load).is_none());
    assert!(control::js_wait_poll(&WaitFor::Console(None)).is_none());
}

#[test]
fn eval_results_are_typed() {
    let ok = control::eval_ok("number", "42");
    assert_eq!(ok.get("ok"), Some(&Json::Bool(true)));
    assert_eq!(ok.get("result"), Some(&Json::Num(42.0)));
    assert_eq!(ok.get("type").and_then(Json::as_str), Some("number"));
    let obj = control::eval_ok("object", r#"{"a":[1,"x"]}"#);
    assert_eq!(obj.get("result").and_then(|r| r.get("a")).and_then(Json::as_array).map(|a| a.len()), Some(2));
    let err = control::eval_err("ReferenceError: x is not defined");
    assert_eq!(err.get("ok"), Some(&Json::Bool(false)));
    assert_eq!(err.get("error").and_then(Json::as_str), Some("ReferenceError: x is not defined"));
    assert_eq!(control::inner_string("string", r#""{\"a\":1}""#).as_deref(), Some(r#"{"a":1}"#));
    assert_eq!(control::inner_string("number", "1"), None);
}

#[test]
fn dom_replies_surface_selector_errors() {
    let e = control::dom_reply(r#"{"error":"SyntaxError: bad selector"}"#).expect_err("a bad selector");
    assert_eq!(e.code, INVALID_PARAMS);
    let ok = control::dom_reply(r#"{"matches":2,"items":[]}"#).unwrap();
    assert_eq!(ok.get("matches").and_then(Json::as_u64), Some(2));
}

#[test]
fn base64_and_png_helpers() {
    assert_eq!(control::base64(b""), "");
    assert_eq!(control::base64(b"f"), "Zg==");
    assert_eq!(control::base64(b"fo"), "Zm8=");
    assert_eq!(control::base64(b"foo"), "Zm9v");
    assert_eq!(control::base64(b"foob"), "Zm9vYg==");
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&640u32.to_be_bytes());
    png.extend_from_slice(&480u32.to_be_bytes());
    assert_eq!(control::png_size(&png), Some((640, 480)));
    assert_eq!(control::png_size(b"not a png at all, not at all!"), None);
    let r = control::screenshot_reply(&png, 1.5, (0.0, 120.0)).unwrap();
    assert_eq!(r.get("width").and_then(Json::as_u64), Some(640));
    assert_eq!(r.get("height").and_then(Json::as_u64), Some(480));
    assert_eq!(r.get("format").and_then(Json::as_str), Some("png"));
    assert_eq!(r.get("encoding").and_then(Json::as_str), Some("base64"));
    assert_eq!(r.get("scroll").and_then(|s| s.get("y")).and_then(Json::number), Some(120.0));
    assert!(control::screenshot_reply(b"junk", 1.0, (0.0, 0.0)).is_err());
    let big = vec![0u8; control::MAX_SHOT_BYTES + 1];
    assert!(control::screenshot_reply(&big, 1.0, (0.0, 0.0)).is_err());
}

#[test]
fn network_log_keeps_two_thousand_entries_and_pages_by_seq() {
    let mut log = network::NetLog::default();
    let items: Vec<String> = (0..2005).map(|n| format!(r#"{{"method":"GET","url":"https://x.test/{n}","status":200,"type":"fetch","ts":5,"dur":1.5,"size":10}}"#)).collect();
    let raw = format!("[{}]", items.join(","));
    assert_eq!(log.ingest(7, &raw, 99), 2005);
    assert_eq!(log.entries(7).map(|q| q.len()), Some(network::MAX_ENTRIES));
    assert_eq!(log.last_seq(7), 2005);
    assert_eq!(log.entries(7).unwrap().front().map(|r| r.seq), Some(6));
    let q = log.query(7, Some(2003), 200, false);
    assert_eq!(q.get("entries").and_then(Json::as_array).map(|e| e.len()), Some(2));
    assert_eq!(q.get("next_seq").and_then(Json::as_u64), Some(2005));
    log.query(7, None, 10, true);
    assert_eq!(log.entries(7).map(|q| q.len()), Some(0));
    assert_eq!(log.last_seq(7), 2005);
    assert_eq!(log.ingest(7, "not json", 1), 0);
    assert_eq!(log.entries(8).map(|q| q.len()), None);
}

#[test]
fn network_entries_carry_status_errors_and_failures() {
    let mut log = network::NetLog::default();
    let raw = r#"[
        {"method":"POST","url":"https://a.test/api","status":500,"type":"xhr","ts":10,"dur":20,"size":3},
        {"method":"GET","url":"https://a.test/gone.png","status":0,"type":"img","ts":11,"dur":0,"size":0,"error":"failed to load"},
        {"method":"GET","url":"https://a.test/ok.css","status":0,"type":"link","dur":4,"size":2048,"cache":true}
    ]"#;
    assert_eq!(log.ingest(1, raw, 777), 3);
    let q = log.query(1, None, 200, false);
    let entries = q.get("entries").and_then(Json::as_array).unwrap();
    assert_eq!(entries[0].get("status").and_then(Json::as_u64), Some(500));
    assert_eq!(entries[0].get("type").and_then(Json::as_str), Some("xhr"));
    assert_eq!(entries[1].get("error").and_then(Json::as_str), Some("failed to load"));
    assert_eq!(entries[2].get("from_cache"), Some(&Json::Bool(true)));
    // an entry without a page timestamp gets the app's
    assert_eq!(entries[2].get("ts_ms").and_then(Json::as_u64), Some(777));
    let recs = log.entries(1).unwrap();
    assert!(recs[0].failed() && recs[1].failed() && !recs[2].failed());
    assert_eq!(network::rows(recs, 1, "", 800.0).len(), 2);
    assert_eq!(network::rows(recs, 2, "", 800.0).len(), 1);
    assert_eq!(network::rows(recs, 0, "gone", 800.0).len(), 1);
    assert_eq!(network::human_size(0), "-");
    assert_eq!(network::human_size(2048), "2.0 kB");
    assert_eq!(network::short_name("https://a.test/x/y?z=1"), "/x/y?z=1");
    assert_eq!(network::short_name("https://a.test/"), "a.test");
}

#[test]
fn downloads_parses_cancel_and_clear() {
    assert_eq!(ok("browser.downloads", "{}"), Req::Downloads { cancel: None, clear: false });
    assert_eq!(ok("browser.downloads", r#"{"cancel":3,"clear":true}"#), Req::Downloads { cancel: Some(3), clear: true });
    assert_eq!(code("browser.downloads", r#"{"cancel":"x"}"#), INVALID_PARAMS);
}
