//! Tool calls: named actions with a description and a parameter schema that a model may ask the app to run. A registry
//! holds the tools, checks each requested call's arguments against the tool's schema before the tool runs, caps the
//! number of calls, and keeps a record of every call in order. The model itself is not here: a session passes the
//! calls the model asks for to `Registry::call` and gives the returned value back to the model.

use crate::generation_schema::{validate, Issue, Kind, Value};

/// An action the app offers to the model.
pub trait Tool {
    /// A name that is unique within a registry.
    fn name(&self) -> &str;
    /// When and how to use the tool, in plain words. The model reads it to decide whether to call the tool.
    fn description(&self) -> &str;
    /// The arguments the tool accepts. An object schema is the usual form.
    fn parameters(&self) -> Kind;
    /// Whether the schema is written into the session's instructions. Defaults to true.
    fn includes_schema(&self) -> bool {
        true
    }
    /// Runs the action on arguments that already fit the schema. An `Err` is handed back to the model as the failure.
    fn call(&mut self, arguments: &Value) -> Result<Value, String>;
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolError {
    /// A tool with this name is already registered.
    Duplicate(String),
    UnknownTool(String),
    /// The arguments do not fit the schema. Every problem is listed, and the tool did not run.
    InvalidArguments(Vec<Issue>),
    /// The tool ran and reported this failure.
    Failed(String),
    /// The registry has already run its limit of calls.
    LimitReached(usize),
}

/// One call the registry handled, kept in the order it arrived.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub name: String,
    pub arguments: Value,
    pub outcome: Result<Value, ToolError>,
}

pub struct Registry {
    tools: Vec<Box<dyn Tool>>,
    calls: Vec<Call>,
    limit: usize,
}

impl Registry {
    /// A registry that handles at most `limit` calls. A call refused by the limit is not recorded; every other call
    /// is, including unknown tools and invalid arguments.
    pub fn new(limit: usize) -> Self {
        Registry { tools: Vec::new(), calls: Vec::new(), limit }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) -> Result<(), ToolError> {
        let name = tool.name().to_string();
        if self.tools.iter().any(|t| t.name() == name.as_str()) {
            return Err(ToolError::Duplicate(name));
        }
        self.tools.push(tool);
        Ok(())
    }

    /// The names of the registered tools, in registration order.
    pub fn names(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    /// Text for the session's instructions: one entry per tool with its description, and its schema when the tool
    /// includes it.
    pub fn instructions(&self) -> String {
        let mut out = String::new();
        for tool in &self.tools {
            out.push_str(&format!("- {}: {}\n", tool.name(), tool.description()));
            if tool.includes_schema() {
                out.push_str(&format!("  arguments: {}\n", tool.parameters().describe()));
            }
        }
        out
    }

    /// Every call handled so far, oldest first.
    pub fn calls(&self) -> &[Call] {
        &self.calls
    }

    /// Handles one call the model asked for. The arguments are checked against the tool's schema first, and a call
    /// that fails the check does not run the tool.
    pub fn call(&mut self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        if self.calls.len() >= self.limit {
            return Err(ToolError::LimitReached(self.limit));
        }
        let outcome = self.run(name, &arguments);
        let result = outcome.clone();
        self.calls.push(Call { name: name.to_string(), arguments, outcome });
        result
    }

    fn run(&mut self, name: &str, arguments: &Value) -> Result<Value, ToolError> {
        let index = match self.tools.iter().position(|t| t.name() == name) {
            Some(i) => i,
            None => return Err(ToolError::UnknownTool(name.to_string())),
        };
        let schema = self.tools[index].parameters();
        let issues = validate(&schema, arguments);
        if !issues.is_empty() {
            return Err(ToolError::InvalidArguments(issues));
        }
        self.tools[index].call(arguments).map_err(ToolError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation_schema::Property;
    use std::cell::Cell;
    use std::rc::Rc;

    /// Forecast tool: needs a city, takes an optional number of days from 1 to 7. Counts its runs.
    struct Weather {
        runs: Rc<Cell<u32>>,
    }

    impl Tool for Weather {
        fn name(&self) -> &str {
            "weather"
        }
        fn description(&self) -> &str {
            "Forecast for a city."
        }
        fn parameters(&self) -> Kind {
            Kind::Object(vec![
                Property::new("city", Kind::Text).described("city name"),
                Property::new("days", Kind::number_in(1.0, 7.0)).optional(),
            ])
        }
        fn call(&mut self, arguments: &Value) -> Result<Value, String> {
            self.runs.set(self.runs.get() + 1);
            match arguments {
                Value::Object(fields) => match fields.iter().find(|(k, _)| k.as_str() == "city") {
                    Some((_, Value::Text(city))) => Ok(Value::Text(format!("sunny in {city}"))),
                    _ => Err("no city".to_string()),
                },
                _ => Err("not an object".to_string()),
            }
        }
    }

    /// Tool with no arguments whose schema is left out of the instructions and which always fails.
    struct Silent;

    impl Tool for Silent {
        fn name(&self) -> &str {
            "sync"
        }
        fn description(&self) -> &str {
            "Syncs the library."
        }
        fn parameters(&self) -> Kind {
            Kind::Object(Vec::new())
        }
        fn includes_schema(&self) -> bool {
            false
        }
        fn call(&mut self, _arguments: &Value) -> Result<Value, String> {
            Err("offline".to_string())
        }
    }

    fn obj(pairs: Vec<(&str, Value)>) -> Value {
        Value::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn registry_with_weather(limit: usize) -> (Registry, Rc<Cell<u32>>) {
        let runs = Rc::new(Cell::new(0));
        let mut reg = Registry::new(limit);
        reg.register(Box::new(Weather { runs: runs.clone() })).unwrap();
        (reg, runs)
    }

    #[test]
    fn instructions_list_each_tool_with_its_schema() {
        let (reg, _) = registry_with_weather(4);
        let text = reg.instructions();
        assert!(text.starts_with("- weather: Forecast for a city.\n"));
        assert!(text.contains("  arguments: object { city: text (city name), days?: number from 1 to 7 }\n"));
    }

    #[test]
    fn duplicate_names_are_refused() {
        let (mut reg, _) = registry_with_weather(4);
        let again = reg.register(Box::new(Weather { runs: Rc::new(Cell::new(0)) }));
        assert_eq!(again, Err(ToolError::Duplicate("weather".to_string())));
        assert_eq!(reg.names(), vec!["weather"]);
    }

    #[test]
    fn a_valid_call_runs_the_tool_and_is_recorded() {
        let (mut reg, runs) = registry_with_weather(4);
        let args = obj(vec![("city", Value::Text("Reykjavik".to_string())), ("days", Value::Number(3.0))]);
        assert_eq!(reg.call("weather", args.clone()), Ok(Value::Text("sunny in Reykjavik".to_string())));
        assert_eq!(runs.get(), 1);
        assert_eq!(reg.calls().len(), 1);
        assert_eq!(reg.calls()[0].name, "weather");
        assert_eq!(reg.calls()[0].arguments, args);
    }

    #[test]
    fn arguments_are_checked_before_the_tool_runs() {
        let (mut reg, runs) = registry_with_weather(4);
        let missing = obj(vec![("days", Value::Number(2.0))]);
        match reg.call("weather", missing) {
            Err(ToolError::InvalidArguments(issues)) => {
                assert_eq!(issues.len(), 1);
                assert_eq!(issues[0].path, "$.city");
            }
            other => panic!("expected invalid arguments, got {other:?}"),
        }
        let out_of_range = obj(vec![("city", Value::Text("Akureyri".to_string())), ("days", Value::Number(9.0))]);
        assert!(matches!(reg.call("weather", out_of_range), Err(ToolError::InvalidArguments(_))));
        assert_eq!(runs.get(), 0);
    }

    #[test]
    fn unknown_tools_are_recorded_as_refused() {
        let (mut reg, _) = registry_with_weather(4);
        assert_eq!(reg.call("stocks", Value::Null), Err(ToolError::UnknownTool("stocks".to_string())));
        assert_eq!(reg.calls()[0].outcome, Err(ToolError::UnknownTool("stocks".to_string())));
    }

    #[test]
    fn the_call_limit_stops_further_calls() {
        let (mut reg, runs) = registry_with_weather(2);
        let args = obj(vec![("city", Value::Text("Oslo".to_string()))]);
        assert!(reg.call("weather", args.clone()).is_ok());
        assert!(reg.call("weather", args.clone()).is_ok());
        assert_eq!(reg.call("weather", args), Err(ToolError::LimitReached(2)));
        assert_eq!(reg.calls().len(), 2);
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn failures_are_reported_and_schemas_can_be_left_out() {
        let mut reg = Registry::new(4);
        reg.register(Box::new(Silent)).unwrap();
        assert!(reg.instructions().contains("- sync: Syncs the library.\n"));
        assert!(!reg.instructions().contains("arguments:"));
        assert_eq!(reg.call("sync", Value::Object(Vec::new())), Err(ToolError::Failed("offline".to_string())));
        assert_eq!(reg.calls()[0].outcome, Err(ToolError::Failed("offline".to_string())));
    }
}
