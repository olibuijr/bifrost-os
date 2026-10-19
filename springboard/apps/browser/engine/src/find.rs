//! Find in page using standard DOM APIs; Servo 0.7 has neither an embedder find API nor window.find.
use super::*;

const SCRIPT: &str = include_str!("find.js");

impl Engine {
    pub(super) fn find_in_page(&self, id: u32, mode: &str, query: &str) {
        let Some(view) = self.views.get(&id) else { return };
        let script = format!("({SCRIPT})({}, {})", serde_json::to_string(mode).unwrap(), serde_json::to_string(query).unwrap());
        view.evaluate_javascript(script, move |result| {
            let count = match result { Ok(servo::JSValue::Number(number)) => number as u64, _ => 0 };
            say(&format!("found {id} {count}"));
        });
    }
}
