//! The tools the Assistant offers the model: the current time (UTC) and a word count. Each is an entry of a `ToolSet`
//! (`bifrost_app::controls::model_tool`), so the arguments the model chooses are checked against the schema before the
//! tool runs, and a failed call reaches the model as text. The same two tools are described to the API by
//! [`tools_json`], which must stay in step with [`tool_set`].

use bifrost_app::controls::generation_schema::{Kind, Property, Value};
use bifrost_app::controls::model_tool::{Tool, ToolSet};
use bifrost_core::json::quote;

const TIME_DESCRIPTION: &str = "The current date and time in UTC, as ISO 8601 text. Use it for anything that depends on today's date or time.";
const WORDS_DESCRIPTION: &str = "Counts the words in a text. Use it when the user asks for a word count.";

/// The tool definitions of a request, as the Messages API takes them (`name`, `description`, `input_schema`).
pub fn tools_json() -> String {
    format!(
        "[{{\"name\":\"current_time\",\"description\":{},\"input_schema\":{{\"type\":\"object\",\"properties\":{{}}}}}},\
         {{\"name\":\"count_words\",\"description\":{},\"input_schema\":{{\"type\":\"object\",\"properties\":{{\"text\":{{\"type\":\"string\",\"description\":\"the text to count\"}}}},\"required\":[\"text\"]}}}}]",
        quote(TIME_DESCRIPTION),
        quote(WORDS_DESCRIPTION)
    )
}

/// The tool set a reply runs its calls with.
pub fn tool_set() -> ToolSet {
    let mut set = ToolSet::new();
    let time = Tool::new("current_time", TIME_DESCRIPTION, Kind::Object(Vec::new()), |_| Ok(iso_utc(now_secs())));
    let words = Tool::new("count_words", WORDS_DESCRIPTION, Kind::Object(vec![Property::new("text", Kind::Text).described("the text to count")]), |args| {
        Ok(format!("{} words", count_words(args)))
    });
    let _ = set.register(time);
    let _ = set.register(words);
    set
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Words in the `text` member of the arguments; 0 when it is missing or not text.
fn count_words(args: &Value) -> usize {
    let Value::Object(members) = args else { return 0 };
    match members.iter().find(|(k, _)| k == "text") {
        Some((_, Value::Text(t))) => t.split_whitespace().count(),
        _ => 0,
    }
}

/// `secs` since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ` (UTC), with the civil date worked out from the day count.
pub fn iso_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Year, month and day of a day count since 1970-01-01 (proleptic Gregorian calendar).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_app::controls::transcript::ToolCall;
    use bifrost_core::json::{self, Json};

    #[test]
    fn the_epoch_and_a_known_second_are_dated_correctly() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        // 1_000_000_000 s: day 11574 (2001-09-09) and 01:46:40.
        assert_eq!(iso_utc(1_000_000_000), "2001-09-09T01:46:40Z");
    }

    #[test]
    fn counting_words_reads_the_text_member() {
        let args = Value::Object(vec![("text".into(), Value::Text("one two  three".into()))]);
        assert_eq!(count_words(&args), 3);
        assert_eq!(count_words(&Value::Object(Vec::new())), 0);
    }

    #[test]
    fn the_tool_set_runs_checked_calls() {
        let set = tool_set();
        assert_eq!(set.names(), vec!["current_time", "count_words"]);
        let words = set.call(&ToolCall { name: "count_words".into(), arguments: "{\"text\":\"a b c d\"}".into() });
        assert_eq!(words.ok().as_deref(), Some("4 words"));
        // `text` is required, so an empty object does not run the tool.
        assert!(set.call(&ToolCall { name: "count_words".into(), arguments: "{}".into() }).is_err());
        // No arguments text means an empty object, which current_time takes.
        let time = set.call(&ToolCall { name: "current_time".into(), arguments: String::new() }).ok().unwrap_or_default();
        assert!(time.starts_with("20") && time.ends_with('Z') && time.len() == 20);
    }

    #[test]
    fn the_definitions_match_the_tool_names() {
        let v = json::parse(&tools_json()).unwrap();
        let tools = v.as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[1].str_field("name").as_deref(), Some("count_words"));
        assert_eq!(tools[1].get("input_schema").and_then(|s| s.get("required")).and_then(Json::as_array).map(|r| r.len()), Some(1));
    }
}
