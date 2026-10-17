//! The intent protocol the shell speaks: a catalogue of the actions the person allowed, one non-streamed request that
//! carries it in the system prompt, and a reply parsed as either a plain answer or one intent call checked against the
//! catalogue. Pure: the transport lives in `transport.rs`.

use bifrost_core::json::{self, quote, Json};

/// Output limit of one intent reply.
pub const INTENT_MAX_TOKENS: u32 = 1024;
/// The instructions every intent request carries. The catalogue follows them in the system prompt.
pub const SYSTEM: &str = "You are the assistant of BifrOSt OS. Reply with one JSON object and nothing else. \
To answer the person: {\"answer\":\"<text>\"}. To run one of the actions listed below: \
{\"app\":\"<app id>\",\"intent\":\"<intent id>\",\"params\":{\"<name>\":\"<value>\"},\"summary\":\"<one line>\"}. \
Use only the listed app and intent ids and only the listed parameter names. If no action fits, answer.";

/// An id the shell accepts: letters, digits, `-` and `_`.
pub fn safe_id(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// One parameter of an intent, from a manifest spec `name:type` (a trailing `?` when optional).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub ty: String,
    pub optional: bool,
}

pub fn parse_param(spec: &str) -> Option<Param> {
    let (name, ty) = spec.split_once(':')?;
    let optional = ty.ends_with('?');
    let ty = ty.trim_end_matches('?');
    (safe_id(name) && !ty.is_empty()).then(|| Param { name: name.to_string(), ty: ty.to_string(), optional })
}

/// One intent the assistant may call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub app_id: String,
    pub app_name: String,
    pub intent_id: String,
    pub title: String,
    pub description: String,
    pub params: Vec<Param>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalogue {
    pub entries: Vec<Entry>,
}

impl Catalogue {
    pub fn find(&self, app_id: &str, intent_id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.app_id == app_id && e.intent_id == intent_id)
    }
}

/// The catalogue as a JSON array, the form the backend reads.
pub fn catalogue_json(catalogue: &Catalogue) -> String {
    let items: Vec<String> = catalogue
        .entries
        .iter()
        .map(|e| {
            let params: Vec<String> = e
                .params
                .iter()
                .map(|p| format!("{{\"name\":{},\"type\":{},\"optional\":{}}}", quote(&p.name), quote(&p.ty), p.optional))
                .collect();
            format!(
                "{{\"app\":{},\"app_name\":{},\"intent\":{},\"title\":{},\"description\":{},\"params\":[{}]}}",
                quote(&e.app_id),
                quote(&e.app_name),
                quote(&e.intent_id),
                quote(&e.title),
                quote(&e.description),
                params.join(",")
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// The non-streamed Messages request body: the system prompt with the catalogue, then the messages `(is_user, text)`.
pub fn intent_request_body(model: &str, catalogue: &Catalogue, messages: &[(bool, String)]) -> String {
    let system = format!("{SYSTEM}\n\nActions:\n{}", catalogue_json(catalogue));
    let wire: Vec<String> = messages
        .iter()
        .map(|(user, text)| format!("{{\"role\":\"{}\",\"content\":{}}}", if *user { "user" } else { "assistant" }, quote(text)))
        .collect();
    format!("{{\"model\":{},\"max_tokens\":{INTENT_MAX_TOKENS},\"system\":{},\"messages\":[{}]}}", quote(model), quote(&system), wire.join(","))
}

/// The text of a Messages API reply: its text blocks joined, or the API's error message.
pub fn reply_text(body: &str) -> Result<String, String> {
    let v = json::parse(body).map_err(|e| format!("malformed reply: {e}"))?;
    if let Some(err) = v.get("error") {
        return Err(err.str_field("message").unwrap_or_else(|| "The API reported an error.".into()));
    }
    let blocks = v.get("content").and_then(Json::as_array).ok_or_else(|| "The reply had no content.".to_string())?;
    Ok(blocks
        .iter()
        .filter(|b| b.str_field("type").as_deref() == Some("text"))
        .filter_map(|b| b.str_field("text"))
        .collect::<Vec<_>>()
        .join(""))
}

/// One intent call, checked against the catalogue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub app_id: String,
    pub app_name: String,
    pub intent_id: String,
    pub title: String,
    pub params: Vec<(String, String)>,
    pub summary: String,
}

/// What the assistant answered: text to show, or an intent call to confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Answer(String),
    Intent(Call),
    /// A question the person must answer before the call can be made: a missing parameter, or an intent several apps
    /// offer ([`missing_question`], [`disambiguation_question`]). Shown as text; the answer comes in the next turn.
    Ask(String),
}

/// The question for a required parameter the reply left out: "What text should I use to create note in Notes?"
pub fn missing_question(entry: &Entry, param: &Param) -> String {
    format!("What {} should I use to {} in {}?", param.name, entry.title.to_lowercase(), entry.app_name)
}

/// The question that tells apart several apps' intents with one id: "Which one do you mean: A in X or B in Y?"
pub fn disambiguation_question(candidates: &[&Entry]) -> String {
    let mut options: Vec<String> = candidates.iter().map(|e| format!("{} in {}", e.title, e.app_name)).collect();
    let list = match options.len() {
        0 | 1 => options.concat(),
        n => {
            let last = options.remove(n - 1);
            format!("{} or {last}", options.join(", "))
        }
    };
    format!("Which one do you mean: {list}?")
}

/// The JSON object inside a reply (a fenced block or prose around it is allowed), from its first `{` to its last `}`.
fn json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| &text[start..=end])
}

/// Parses a backend reply. Text with no JSON object is a plain answer. A JSON object must be an answer or an intent
/// call on a catalogue entry with known parameter names and every required parameter present.
pub fn parse_reply(text: &str, catalogue: &Catalogue) -> Result<Reply, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("The assistant sent an empty reply.".into());
    }
    let Some(object) = json_object(trimmed) else { return Ok(Reply::Answer(trimmed.to_string())) };
    let Ok(v) = json::parse(object) else { return Ok(Reply::Answer(trimmed.to_string())) };
    if let Some(answer) = v.str_field("answer") {
        return Ok(Reply::Answer(answer));
    }
    let Some(intent_id) = v.str_field("intent") else {
        return Err("The assistant's reply was neither an answer nor an action.".into());
    };
    let not_allowed = || format!("The assistant asked for an action that is not allowed: {intent_id}.");
    let entry = match v.str_field("app") {
        Some(app_id) => catalogue.find(&app_id, &intent_id).ok_or_else(not_allowed)?,
        // no app named: the intent id decides it when one app offers it; several apps make it a question
        None => match catalogue.entries.iter().filter(|e| e.intent_id == intent_id).collect::<Vec<_>>().as_slice() {
            [] => return Err(not_allowed()),
            [only] => *only,
            several => return Ok(Reply::Ask(disambiguation_question(several))),
        },
    };
    let mut params: Vec<(String, String)> = Vec::new();
    if let Some(Json::Obj(pairs)) = v.get("params") {
        for (name, value) in pairs {
            let Some(value) = value.as_str() else { return Err(format!("Parameter {name} is not text.")) };
            if !entry.params.iter().any(|p| &p.name == name) {
                return Err(format!("Unknown parameter {name}."));
            }
            params.push((name.clone(), value.to_string()));
        }
    }
    if let Some(missing) = entry.params.iter().find(|p| !p.optional && !params.iter().any(|(n, _)| n == &p.name)) {
        return Ok(Reply::Ask(missing_question(entry, missing)));
    }
    Ok(Reply::Intent(Call {
        app_id: entry.app_id.clone(),
        app_name: entry.app_name.clone(),
        intent_id: entry.intent_id.clone(),
        title: entry.title.clone(),
        params,
        summary: v.str_field("summary").unwrap_or_default(),
    }))
}

/// The confirmation card: "Create note 'Groceries' in Notes?"
pub fn card_text(call: &Call) -> String {
    let mut text = call.title.clone();
    for (_, value) in &call.params {
        text.push_str(&format!(" '{value}'"));
    }
    format!("{text} in {}?", call.app_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Catalogue {
        let entry = |app_id: &str, app_name: &str, intent_id: &str, title: &str, params: Vec<Param>| Entry {
            app_id: app_id.into(),
            app_name: app_name.into(),
            intent_id: intent_id.into(),
            title: title.into(),
            description: "Makes a note".into(),
            params,
        };
        Catalogue {
            entries: vec![
                entry(
                    "is.olibuijr.notes",
                    "Notes",
                    "create-note",
                    "Create note",
                    vec![parse_param("text:string").unwrap(), parse_param("folder:string?").unwrap()],
                ),
                entry("is.olibuijr.calendar", "Calendar", "add-event", "Add event", vec![parse_param("title:string").unwrap()]),
            ],
        }
    }

    #[test]
    fn parse_param_reads_optional_and_rejects_malformed_specs() {
        assert_eq!(parse_param("folder:string?"), Some(Param { name: "folder".into(), ty: "string".into(), optional: true }));
        assert_eq!(parse_param("text"), None);
        assert_eq!(parse_param("bad name:string"), None);
    }

    #[test]
    fn the_intent_request_snapshot_is_exact() {
        let catalogue = Catalogue { entries: vec![sample().entries.remove(0)] };
        let body = intent_request_body("claude-haiku-5-5", &catalogue, &[(true, "Note groceries".to_string())]);
        let system = format!("{SYSTEM}\n\nActions:\n{}", catalogue_json(&catalogue));
        assert_eq!(
            body,
            format!(
                "{{\"model\":\"claude-haiku-5-5\",\"max_tokens\":1024,\"system\":{},\"messages\":[{{\"role\":\"user\",\"content\":\"Note groceries\"}}]}}",
                quote(&system)
            )
        );
        assert!(body.contains("\\\"intent\\\":\\\"create-note\\\""), "the catalogue rides in the system prompt");
        assert!(!body.contains("\"tools\""), "the intent request offers no tools");
    }

    #[test]
    fn the_catalogue_json_lists_names_types_and_optional_flags() {
        let json_text = catalogue_json(&sample());
        assert!(json_text.contains("\"name\":\"folder\",\"type\":\"string\",\"optional\":true"));
        assert!(json_text.starts_with("[{\"app\":\"is.olibuijr.notes\""));
    }

    #[test]
    fn a_plain_answer_is_text_and_an_answer_object_is_its_text() {
        let c = sample();
        assert_eq!(parse_reply("It is sunny.", &c), Ok(Reply::Answer("It is sunny.".into())));
        assert_eq!(parse_reply("{\"answer\":\"Four.\"}", &c), Ok(Reply::Answer("Four.".into())));
        assert_eq!(parse_reply("```json\n{\"answer\":\"Four.\"}\n```", &c), Ok(Reply::Answer("Four.".into())));
        assert!(parse_reply("   ", &c).is_err());
    }

    #[test]
    fn an_intent_reply_becomes_a_call_with_checked_parameters() {
        let reply = parse_reply(
            r#"{"app":"is.olibuijr.notes","intent":"create-note","params":{"text":"Groceries"},"summary":"Make a grocery note"}"#,
            &sample(),
        )
        .unwrap();
        let Reply::Intent(call) = reply else { panic!("expected an intent call") };
        assert_eq!(call.intent_id, "create-note");
        assert_eq!(call.params, vec![("text".to_string(), "Groceries".to_string())]);
        assert_eq!(call.summary, "Make a grocery note");
    }

    #[test]
    fn unknown_intents_parameters_and_missing_required_ones_are_refused() {
        let c = sample();
        assert!(parse_reply(r#"{"app":"is.olibuijr.notes","intent":"delete-all","params":{}}"#, &c).unwrap_err().contains("not allowed"));
        assert!(parse_reply(r#"{"app":"is.olibuijr.notes","intent":"create-note","params":{"text":"a","colour":"red"}}"#, &c).unwrap_err().contains("Unknown parameter colour"));
        assert!(parse_reply(r#"{"app":"is.olibuijr.notes","intent":"create-note","params":{"text":3}}"#, &c).unwrap_err().contains("not text"));
        assert!(matches!(parse_reply(r#"{"app":"is.olibuijr.notes","intent":"create-note","params":{"folder":"Work"}}"#, &c), Ok(Reply::Ask(q)) if q == "What text should I use to create note in Notes?"));
        assert!(matches!(parse_reply(r#"{"app":"is.olibuijr.notes","intent":"create-note","params":{"text":"a"}}"#, &c), Ok(Reply::Intent(_))));
    }

    #[test]
    fn an_intent_named_without_an_app_is_resolved_or_asked_about() {
        let mut c = sample();
        let mut copy = c.entries[0].clone();
        copy.app_id = "is.example.journal".into();
        copy.app_name = "Journal".into();
        c.entries.push(copy);
        // two apps offer create-note: the person is asked which one
        assert_eq!(
            parse_reply(r#"{"intent":"create-note","params":{"text":"a"}}"#, &c),
            Ok(Reply::Ask("Which one do you mean: Create note in Notes or Create note in Journal?".into()))
        );
        // one app offers add-event: it is the call
        let Ok(Reply::Intent(call)) = parse_reply(r#"{"intent":"add-event","params":{"title":"Lunch"}}"#, &c) else { panic!("expected a call") };
        assert_eq!((call.app_id.as_str(), call.intent_id.as_str()), ("is.olibuijr.calendar", "add-event"));
        assert!(parse_reply(r#"{"intent":"delete-all","params":{}}"#, &c).unwrap_err().contains("not allowed"));
    }

    #[test]
    fn disambiguation_lists_every_option() {
        let c = sample();
        let refs: Vec<&Entry> = c.entries.iter().collect();
        assert_eq!(disambiguation_question(&refs[..1]), "Which one do you mean: Create note in Notes?");
        assert_eq!(disambiguation_question(&refs), "Which one do you mean: Create note in Notes or Add event in Calendar?");
        assert_eq!(disambiguation_question(&[refs[0], refs[1], refs[0]]), "Which one do you mean: Create note in Notes, Add event in Calendar or Create note in Notes?");
    }

    #[test]
    fn reply_text_joins_the_text_blocks_and_reports_errors() {
        let ok = r#"{"content":[{"type":"text","text":"{\"answer\":"},{"type":"tool_use","id":"x"},{"type":"text","text":"\"Hi\"}"}]}"#;
        assert_eq!(reply_text(ok).unwrap(), "{\"answer\":\"Hi\"}");
        assert_eq!(reply_text(r#"{"type":"error","error":{"message":"bad key"}}"#), Err("bad key".into()));
        assert!(reply_text("nope").is_err());
    }

    #[test]
    fn the_card_names_the_call_and_its_app() {
        let call = Call {
            app_id: "is.olibuijr.notes".into(),
            app_name: "Notes".into(),
            intent_id: "create-note".into(),
            title: "Create note".into(),
            params: vec![("text".into(), "Groceries".into())],
            summary: String::new(),
        };
        assert_eq!(card_text(&call), "Create note 'Groceries' in Notes?");
    }
}
