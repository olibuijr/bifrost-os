//! The local search tool for the on-device assistant: a model [`Tool`] that searches
//! the items apps have indexed and returns the best matches as text the model can read.
//!
//! The caller decides which items the assistant may see and passes them in as [`IndexedItem`]s; this module only
//! searches and formats. Matching and ranking come from [`crate::usersearch::run_user_query`], so the tool finds the
//! same items the Search panel does. Each returned item carries its app and identifier, so the model can name the
//! source. The description of each item is cut to [`SNIPPET_CHARS`] characters to keep the tool output short.
//!
//! Pure model. Gaps: the Foundation pipeline types (search sources, stages, contact resolution, result tables and
//! statistics) are not modelled, and the caller must supply the records rather than the shell index.

use crate::generation_schema::{Kind, Property, Value};
use crate::model_tool::Tool;
use crate::searchquery::{Error, Record};
use crate::usersearch::{run_user_query, UserQueryContext};

/// The name the model calls the tool by.
pub const TOOL_NAME: &str = "search_indexed_content";
/// The most items one call returns.
pub const MAX_RESULTS: usize = 10;
/// The number of items a call returns when it names no limit.
pub const DEFAULT_RESULTS: usize = 5;
/// The longest description shown for an item, in characters.
pub const SNIPPET_CHARS: usize = 200;

const DESCRIPTION: &str = "Searches the content apps have indexed on this device (notes, documents and similar items) \
and returns the best matches with their app and identifier.";

/// An indexed item the tool may return: the app that indexed it, its identifier and its searchable fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedItem {
    pub app: String,
    pub unique_identifier: String,
    pub record: Record,
}

/// The arguments the tool takes: `query` (required words) and `limit` (optional, 1 to [`MAX_RESULTS`]).
pub fn schema() -> Kind {
    Kind::Object(vec![
        Property::new("query", Kind::Text).described("Words to find in the titles, descriptions and keywords."),
        Property::new("limit", Kind::number_in(1.0, MAX_RESULTS as f64))
            .described("How many items to return.")
            .optional(),
    ])
}

/// The tool over `items`. Register it in a [`crate::model_tool::ToolSet`] to offer it to a session.
pub fn tool(items: Vec<IndexedItem>) -> Tool {
    Tool::new(TOOL_NAME, DESCRIPTION, schema(), move |args| search(&items, args))
}

fn search(items: &[IndexedItem], args: &Value) -> Result<String, String> {
    let query = match field(args, "query") {
        Some(Value::Text(text)) => text.as_str(),
        _ => return Err("the query is missing".into()),
    };
    let limit = match field(args, "limit") {
        Some(Value::Number(n)) => (n.round() as usize).clamp(1, MAX_RESULTS),
        _ => DEFAULT_RESULTS,
    };
    let records: Vec<Record> = items.iter().map(|item| item.record.clone()).collect();
    let context = UserQueryContext { max_result_count: limit, ..UserQueryContext::default() };
    let results = run_user_query(query, &context, &records).map_err(describe_error)?;
    if results.items.is_empty() {
        return Ok(format!("No indexed items match \"{query}\"."));
    }
    let mut lines = vec![format!("Found {} item(s) for \"{query}\":", results.items.len())];
    for (rank, hit) in results.items.iter().enumerate() {
        let item = &items[hit.index];
        lines.push(format!(
            "{}. [{}] {}: {} (identifier: {})",
            rank + 1,
            item.app,
            item.record.title,
            snippet(&item.record),
            item.unique_identifier
        ));
    }
    Ok(lines.join("\n"))
}

fn field<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
    match args {
        Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn describe_error(error: Error) -> String {
    match error {
        Error::EmptyQueryString => "the query has no words to search for".into(),
        Error::NoFields => "the query searches no field".into(),
    }
}

/// The description of an item, or its keywords when it has none, cut to [`SNIPPET_CHARS`] characters.
pub fn snippet(record: &Record) -> String {
    let text = if record.description.trim().is_empty() {
        record.keywords.join(", ")
    } else {
        record.description.trim().to_string()
    };
    if text.chars().count() <= SNIPPET_CHARS {
        return text;
    }
    let mut cut: String = text.chars().take(SNIPPET_CHARS).collect();
    cut.push_str("...");
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_tool::{ToolError, ToolSet};
    use crate::transcript::ToolCall;

    fn item(app: &str, id: &str, title: &str, description: &str, keywords: &[&str]) -> IndexedItem {
        IndexedItem {
            app: app.into(),
            unique_identifier: id.into(),
            record: Record {
                title: title.into(),
                description: description.into(),
                keywords: keywords.iter().map(|k| k.to_string()).collect(),
            },
        }
    }

    fn set_with(items: Vec<IndexedItem>) -> ToolSet {
        let mut set = ToolSet::new();
        set.register(tool(items)).unwrap();
        set
    }

    fn call(arguments: &str) -> ToolCall {
        ToolCall { name: TOOL_NAME.into(), arguments: arguments.into() }
    }

    #[test]
    fn returns_matches_with_app_and_identifier() {
        let set = set_with(vec![
            item("notes", "note-1", "Oat milk recipe", "Warm it slowly", &[]),
            item("docs", "doc-9", "Quarterly report", "Revenue figures", &["finance"]),
        ]);
        let text = set.call(&call("{\"query\":\"milk\"}")).unwrap();
        assert_eq!(
            text,
            "Found 1 item(s) for \"milk\":\n1. [notes] Oat milk recipe: Warm it slowly (identifier: note-1)"
        );
    }

    #[test]
    fn no_match_is_a_plain_answer_not_an_error() {
        let set = set_with(vec![item("notes", "n", "Groceries", "", &[])]);
        assert_eq!(set.call(&call("{\"query\":\"tax\"}")).unwrap(), "No indexed items match \"tax\".");
    }

    #[test]
    fn limit_caps_the_number_of_items() {
        let items: Vec<IndexedItem> =
            (0..6).map(|i| item("notes", &format!("n{i}"), &format!("Trip {i}"), "", &[])).collect();
        let set = set_with(items);
        let two = set.call(&call("{\"query\":\"trip\",\"limit\":2}")).unwrap();
        assert!(two.starts_with("Found 2 item(s)"), "{two}");
        let default = set.call(&call("{\"query\":\"trip\"}")).unwrap();
        assert!(default.starts_with("Found 5 item(s)"), "{default}");
    }

    #[test]
    fn schema_rejects_a_missing_query_and_an_out_of_range_limit() {
        let set = set_with(vec![item("notes", "n", "Milk", "", &[])]);
        assert!(matches!(set.call(&call("{\"limit\":3}")), Err(ToolError::Invalid { .. })));
        assert!(matches!(set.call(&call("{\"query\":\"milk\",\"limit\":11}")), Err(ToolError::Invalid { .. })));
    }

    #[test]
    fn an_empty_query_is_reported_to_the_model() {
        let set = set_with(vec![item("notes", "n", "Milk", "", &[])]);
        let out = set.output_for(&call("{\"query\":\"   \"}"));
        assert_eq!(out.text, "error: tool `search_indexed_content` failed: the query has no words to search for");
    }

    #[test]
    fn keywords_stand_in_for_a_missing_description() {
        let rec = Record { title: "t".into(), description: " ".into(), keywords: vec!["a".into(), "b".into()] };
        assert_eq!(snippet(&rec), "a, b");
    }

    #[test]
    fn long_descriptions_are_cut_on_a_character_boundary() {
        let long = "é".repeat(SNIPPET_CHARS + 50);
        let rec = Record { title: "t".into(), description: long, keywords: vec![] };
        let cut = snippet(&rec);
        assert_eq!(cut.chars().count(), SNIPPET_CHARS + 3);
        assert!(cut.ends_with("..."));
        let short = Record { title: "t".into(), description: "é".repeat(SNIPPET_CHARS), keywords: vec![] };
        assert_eq!(snippet(&short).chars().count(), SNIPPET_CHARS);
    }

    #[test]
    fn the_tool_is_described_with_its_schema() {
        let set = set_with(vec![]);
        let text = set.instructions();
        assert!(text.contains("search_indexed_content"));
        assert!(text.contains("query"));
        assert!(set.get(TOOL_NAME).is_some());
    }
}
