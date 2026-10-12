//! Tools a model may call while it answers. A [`Tool`] has a name, a description and a parameter schema, which is an
//! object [`Kind`] as structured output uses. A [`ToolSet`] holds the tools a session offers, describes them for the
//! instructions of a request, and runs one [`ToolCall`] the model asked for: the arguments are read as JSON, checked
//! against the schema, and only then is the handler run.
//!
//! A failed call does not end the turn. [`ToolSet::output_for`] turns the error into the text of the tool output, so the
//! model sees what went wrong and can try again. This is a pure model: the handlers run in the calling thread, and
//! nothing here calls a model or reads any data.

use crate::generation_schema::{self, Issue, Kind, ParseError, Value};
use crate::transcript::{ToolCall, ToolOutput};
use std::fmt;

/// The longest tool name accepted. This is a BifrOSt limit, not a model limit.
pub const MAX_NAME_LEN: usize = 64;

type Handler = Box<dyn Fn(&Value) -> Result<String, String>>;

/// A tool: its name, when the model should use it, the arguments it takes and the action it performs.
pub struct Tool {
    name: String,
    description: String,
    parameters: Kind,
    in_instructions: bool,
    handler: Handler,
}

impl fmt::Debug for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tool").field("name", &self.name).field("parameters", &self.parameters).finish()
    }
}

impl Tool {
    /// A tool called `name` that takes `parameters` (an object schema) and runs `handler` on the parsed arguments. The
    /// handler returns the output the model reasons about, or an error message the model is shown.
    pub fn new(
        name: &str,
        description: &str,
        parameters: Kind,
        handler: impl Fn(&Value) -> Result<String, String> + 'static,
    ) -> Self {
        Tool {
            name: name.into(),
            description: description.into(),
            parameters,
            in_instructions: true,
            handler: Box::new(handler),
        }
    }

    /// Leaves the parameter schema out of the instructions. The model then sees only the name and the description.
    pub fn without_schema_in_instructions(mut self) -> Self {
        self.in_instructions = false;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }
}

/// Why a tool could not be registered, or a call to it did not run.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolError {
    /// The name is empty, longer than `MAX_NAME_LEN`, or has a character other than ASCII letters, digits, `_` and `-`.
    InvalidName { name: String },
    /// A tool with this name is already in the set.
    Duplicate { name: String },
    /// The parameter schema is not an object, so the arguments have no members to name.
    NotAnObject { name: String },
    /// The description is empty. The model reads it to decide when to call the tool.
    NoDescription { name: String },
    /// The model asked for a tool that is not in the set.
    Unknown { name: String },
    /// The arguments are not JSON.
    Malformed { name: String, error: ParseError },
    /// The arguments do not fit the parameter schema.
    Invalid { name: String, issues: Vec<Issue> },
    /// The handler returned an error.
    Failed { name: String, message: String },
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::InvalidName { name } => {
                write!(f, "tool name `{name}` is not 1 to {MAX_NAME_LEN} letters, digits, `_` or `-`")
            }
            ToolError::Duplicate { name } => write!(f, "tool `{name}` is already registered"),
            ToolError::NotAnObject { name } => write!(f, "tool `{name}` needs an object schema for its arguments"),
            ToolError::NoDescription { name } => write!(f, "tool `{name}` has no description"),
            ToolError::Unknown { name } => write!(f, "there is no tool named `{name}`"),
            ToolError::Malformed { name, error } => write!(f, "arguments for `{name}` are not JSON: {error}"),
            ToolError::Invalid { name, issues } => {
                let problems: Vec<String> = issues.iter().map(|i| format!("{}: {}", i.path, i.message)).collect();
                write!(f, "arguments for `{name}` do not fit the schema: {}", problems.join("; "))
            }
            ToolError::Failed { name, message } => write!(f, "tool `{name}` failed: {message}"),
        }
    }
}

/// The tools a session offers, in the order they were registered.
#[derive(Debug, Default)]
pub struct ToolSet {
    tools: Vec<Tool>,
}

impl ToolSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a tool. Its name must be valid and unused, its schema an object, and its description not empty.
    pub fn register(&mut self, tool: Tool) -> Result<(), ToolError> {
        if !valid_name(&tool.name) {
            return Err(ToolError::InvalidName { name: tool.name });
        }
        if self.get(&tool.name).is_some() {
            return Err(ToolError::Duplicate { name: tool.name });
        }
        if !matches!(tool.parameters, Kind::Object(_)) {
            return Err(ToolError::NotAnObject { name: tool.name });
        }
        if tool.description.trim().is_empty() {
            return Err(ToolError::NoDescription { name: tool.name });
        }
        self.tools.push(tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.name == name)
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name.as_str()).collect()
    }

    /// The text that describes the tools for a request: a header line, then one line per tool with its description and,
    /// unless it was left out, its arguments. Empty when the set has no tools.
    pub fn instructions(&self) -> String {
        if self.tools.is_empty() {
            return String::new();
        }
        let mut lines = vec!["You may call these tools.".to_string()];
        for tool in &self.tools {
            let mut line = format!("- {}: {}", tool.name, tool.description);
            if tool.in_instructions {
                line.push_str(&format!(" Arguments: {}", tool.parameters.describe()));
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    /// Runs one call the model asked for. The arguments are read as JSON (an empty text means no arguments), checked
    /// against the schema, and only then passed to the handler. A call that fails any check does not run the handler.
    pub fn call(&self, call: &ToolCall) -> Result<String, ToolError> {
        let name = call.name.as_str();
        let tool = self.get(name).ok_or_else(|| ToolError::Unknown { name: name.into() })?;
        let text = call.arguments.trim();
        let args = if text.is_empty() {
            Value::Object(Vec::new())
        } else {
            generation_schema::parse(text).map_err(|error| ToolError::Malformed { name: name.into(), error })?
        };
        let issues = generation_schema::validate(&tool.parameters, &args);
        if !issues.is_empty() {
            return Err(ToolError::Invalid { name: name.into(), issues });
        }
        (tool.handler)(&args).map_err(|message| ToolError::Failed { name: name.into(), message })
    }

    /// The output that goes back to the model for a call. A call that fails becomes the text `error: ...`, so the turn
    /// goes on and the model can correct the call.
    pub fn output_for(&self, call: &ToolCall) -> ToolOutput {
        let text = match self.call(call) {
            Ok(text) => text,
            Err(error) => format!("error: {error}"),
        };
        ToolOutput { name: call.name.clone(), text }
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation_schema::Property;
    use std::cell::Cell;
    use std::rc::Rc;

    fn weather_schema() -> Kind {
        Kind::Object(vec![
            Property::new("city", Kind::Text).described("city name"),
            Property::new("units", Kind::choice(&["c", "f"])).optional(),
        ])
    }

    fn field<'a>(args: &'a Value, key: &str) -> Option<&'a Value> {
        match args {
            Value::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The weather tool: it reports the city it was given, or fails when no city text arrived.
    fn weather(runs: Rc<Cell<u32>>) -> Tool {
        Tool::new("weather", "Current weather for a city.", weather_schema(), move |args| {
            runs.set(runs.get() + 1);
            match field(args, "city") {
                Some(Value::Text(city)) => Ok(format!("4 C, rain in {city}")),
                _ => Err("no city".into()),
            }
        })
    }

    fn call(name: &str, arguments: &str) -> ToolCall {
        ToolCall { name: name.into(), arguments: arguments.into() }
    }

    #[test]
    fn a_registered_tool_runs_on_the_parsed_arguments() {
        let mut set = ToolSet::new();
        set.register(weather(Rc::new(Cell::new(0)))).unwrap();
        assert_eq!(set.call(&call("weather", "{\"city\":\"Reykjavik\"}")), Ok("4 C, rain in Reykjavik".to_string()));
        assert_eq!(set.names(), vec!["weather"]);
    }

    #[test]
    fn arguments_are_checked_against_the_schema_before_the_handler_runs() {
        let runs = Rc::new(Cell::new(0));
        let mut set = ToolSet::new();
        set.register(weather(runs.clone())).unwrap();
        let err = set.call(&call("weather", "{\"city\":3,\"units\":\"k\"}")).unwrap_err();
        assert_eq!(
            err,
            ToolError::Invalid {
                name: "weather".into(),
                issues: vec![
                    Issue { path: "$.city".into(), message: "expected text, found number".into() },
                    Issue { path: "$.units".into(), message: "\"k\" is not one of \"c\", \"f\"".into() },
                ],
            }
        );
        assert_eq!(runs.get(), 0, "a call with bad arguments never reaches the handler");
    }

    #[test]
    fn malformed_and_unknown_calls_are_named_in_the_error_and_the_output() {
        let mut set = ToolSet::new();
        set.register(weather(Rc::new(Cell::new(0)))).unwrap();
        assert!(matches!(set.call(&call("weather", "{\"city\":")), Err(ToolError::Malformed { .. })));
        assert_eq!(set.call(&call("stocks", "{}")), Err(ToolError::Unknown { name: "stocks".into() }));
        let output = set.output_for(&call("stocks", "{}"));
        assert_eq!(output.name, "stocks");
        assert_eq!(output.text, "error: there is no tool named `stocks`");
    }

    #[test]
    fn an_empty_argument_text_means_no_arguments_and_extra_members_are_refused() {
        let mut set = ToolSet::new();
        set.register(Tool::new("time", "The local time.", Kind::Object(vec![]), |_| Ok("noon".into()))).unwrap();
        assert_eq!(set.call(&call("time", "  ")), Ok("noon".to_string()));
        assert_eq!(
            set.call(&call("time", "{\"zone\":1}")),
            Err(ToolError::Invalid {
                name: "time".into(),
                issues: vec![Issue { path: "$.zone".into(), message: "not in the schema".into() }],
            })
        );
    }

    #[test]
    fn a_handler_error_reaches_the_model_as_the_tool_output() {
        let mut set = ToolSet::new();
        set.register(Tool::new("weather", "Current weather.", weather_schema(), |_| Err("station offline".into())))
            .unwrap();
        let output = set.output_for(&call("weather", "{\"city\":\"Akureyri\"}"));
        assert_eq!(output.text, "error: tool `weather` failed: station offline");
    }

    #[test]
    fn registration_refuses_bad_names_duplicates_non_object_schemas_and_empty_descriptions() {
        let mut set = ToolSet::new();
        let named = |n: &str| Tool::new(n, "d", Kind::Object(vec![]), |_| Ok(String::new()));
        assert_eq!(set.register(named("")), Err(ToolError::InvalidName { name: "".into() }));
        assert!(matches!(set.register(named("get weather")), Err(ToolError::InvalidName { .. })));
        assert!(matches!(set.register(named(&"a".repeat(65))), Err(ToolError::InvalidName { .. })));
        assert_eq!(set.register(named("ok_name-1")), Ok(()));
        assert_eq!(set.register(named("ok_name-1")), Err(ToolError::Duplicate { name: "ok_name-1".into() }));
        let text_schema = Tool::new("plain", "d", Kind::Text, |_| Ok(String::new()));
        assert_eq!(set.register(text_schema), Err(ToolError::NotAnObject { name: "plain".into() }));
        let blank = Tool::new("quiet", "   ", Kind::Object(vec![]), |_| Ok(String::new()));
        assert_eq!(set.register(blank), Err(ToolError::NoDescription { name: "quiet".into() }));
        assert_eq!(set.names(), vec!["ok_name-1"], "a refused tool is not kept");
    }

    #[test]
    fn instructions_describe_each_tool_on_one_line_and_leave_out_schemas_on_request() {
        let mut set = ToolSet::new();
        set.register(weather(Rc::new(Cell::new(0)))).unwrap();
        set.register(
            Tool::new("time", "The local time.", Kind::Object(vec![]), |_| Ok(String::new()))
                .without_schema_in_instructions(),
        )
        .unwrap();
        assert_eq!(
            set.instructions(),
            "You may call these tools.\n\
             - weather: Current weather for a city. Arguments: object { city: text (city name), units?: one of \"c\", \"f\" }\n\
             - time: The local time."
        );
        assert_eq!(ToolSet::new().instructions(), "");
    }
}
