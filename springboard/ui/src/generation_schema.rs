//! Structured output: a schema for the shape of a generated value, and a check of a value against it.
//! Objects have described, typed members, string choices and numeric ranges, as a generation schema does. Nothing here
//! constrains a model while it writes. An app uses `validate` to learn whether a reply fits, and `describe` to put the
//! shape in its prompt.

/// A value as a model returns it: the content of a structured response.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    Text(String),
    Array(Vec<Value>),
    /// Members in the order they arrived.
    Object(Vec<(String, Value)>),
}

/// The type of a schema node.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Text,
    /// A number, optionally limited to the inclusive range `low..=high`.
    Number(Option<(f64, f64)>),
    Bool,
    /// A string that must be one of the choices.
    Choice(Vec<String>),
    Array(Box<Kind>),
    Object(Vec<Property>),
}

/// A named member of an object schema.
#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub name: String,
    pub kind: Kind,
    /// What the member means. It is shown to the model with the schema.
    pub description: String,
    /// An optional member may be missing or null.
    pub optional: bool,
}

/// One problem found by `validate`: where it is and what is wrong with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub path: String,
    pub message: String,
}

impl Kind {
    pub fn number() -> Kind {
        Kind::Number(None)
    }

    pub fn number_in(low: f64, high: f64) -> Kind {
        Kind::Number(Some((low, high)))
    }

    pub fn array_of(item: Kind) -> Kind {
        Kind::Array(Box::new(item))
    }

    pub fn choice(options: &[&str]) -> Kind {
        Kind::Choice(options.iter().map(|s| s.to_string()).collect())
    }

    /// A compact text form of the schema, for a prompt. It lists each member with its type, choices, range and
    /// description. Optional members have a `?` after the name.
    pub fn describe(&self) -> String {
        match self {
            Kind::Text => "text".into(),
            Kind::Bool => "boolean".into(),
            Kind::Number(None) => "number".into(),
            Kind::Number(Some((low, high))) => format!("number from {low} to {high}"),
            Kind::Choice(options) => format!("one of {}", quoted(options)),
            Kind::Array(item) => format!("array of {}", item.describe()),
            Kind::Object(props) => {
                let members: Vec<String> = props
                    .iter()
                    .map(|p| {
                        let mut line = format!("{}{}: {}", p.name, if p.optional { "?" } else { "" }, p.kind.describe());
                        if !p.description.is_empty() {
                            line.push_str(&format!(" ({})", p.description));
                        }
                        line
                    })
                    .collect();
                format!("object {{ {} }}", members.join(", "))
            }
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Bool => "boolean",
            Kind::Number(_) => "number",
            Kind::Choice(_) => "choice",
            Kind::Array(_) => "array",
            Kind::Object(_) => "object",
        }
    }
}

impl Value {
    fn name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::Text(_) => "text",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }
}

impl Property {
    pub fn new(name: &str, kind: Kind) -> Self {
        Property { name: name.into(), kind, description: String::new(), optional: false }
    }

    pub fn described(mut self, text: &str) -> Self {
        self.description = text.into();
        self
    }

    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

fn quoted(items: &[String]) -> String {
    items.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(", ")
}

/// Checks `value` against `schema`. An empty result means the value fits. Every problem is reported, members in schema
/// order first and then unknown members, so a caller can show them all at once.
pub fn validate(schema: &Kind, value: &Value) -> Vec<Issue> {
    let mut issues = Vec::new();
    check(schema, value, "$", &mut issues);
    issues
}

fn problem(issues: &mut Vec<Issue>, path: &str, message: impl Into<String>) {
    issues.push(Issue { path: path.into(), message: message.into() });
}

fn check(kind: &Kind, value: &Value, path: &str, issues: &mut Vec<Issue>) {
    match (kind, value) {
        (Kind::Text, Value::Text(_)) | (Kind::Bool, Value::Bool(_)) => {}
        (Kind::Number(range), Value::Number(n)) => {
            if !n.is_finite() {
                problem(issues, path, "number is not finite");
            } else if let Some((low, high)) = range {
                if n < low || n > high {
                    problem(issues, path, format!("{n} is outside {low}..={high}"));
                }
            }
        }
        (Kind::Choice(options), Value::Text(t)) => {
            if !options.iter().any(|o| o == t) {
                problem(issues, path, format!("\"{t}\" is not one of {}", quoted(options)));
            }
        }
        (Kind::Array(item), Value::Array(values)) => {
            for (i, v) in values.iter().enumerate() {
                check(item, v, &format!("{path}[{i}]"), issues);
            }
        }
        (Kind::Object(props), Value::Object(fields)) => check_object(props, fields, path, issues),
        (expected, found) => {
            problem(issues, path, format!("expected {}, found {}", expected.name(), found.name()));
        }
    }
}

fn check_object(props: &[Property], fields: &[(String, Value)], path: &str, issues: &mut Vec<Issue>) {
    for prop in props {
        let here = format!("{path}.{}", prop.name);
        let found = fields.iter().find(|(k, _)| *k == prop.name).map(|(_, v)| v);
        match found {
            None | Some(Value::Null) if prop.optional => {}
            None => problem(issues, &here, "missing"),
            Some(Value::Null) => problem(issues, &here, "is null"),
            Some(v) => check(&prop.kind, v, &here, issues),
        }
    }
    for (key, _) in fields {
        if !props.iter().any(|p| p.name == *key) {
            problem(issues, &format!("{path}.{key}"), "not in the schema");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> Kind {
        Kind::Object(vec![
            Property::new("title", Kind::Text).described("the dish name"),
            Property::new("minutes", Kind::number_in(1.0, 240.0)),
            Property::new("difficulty", Kind::choice(&["easy", "medium", "hard"])),
            Property::new("tags", Kind::array_of(Kind::Text)).optional(),
        ])
    }

    fn text(s: &str) -> Value {
        Value::Text(s.into())
    }

    fn obj(fields: Vec<(&str, Value)>) -> Value {
        Value::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn a_value_that_matches_the_schema_has_no_issues() {
        let v = obj(vec![("title", text("Soup")), ("minutes", Value::Number(25.0)), ("difficulty", text("easy"))]);
        assert!(validate(&recipe(), &v).is_empty());
        let with_tags = obj(vec![
            ("title", text("Soup")),
            ("minutes", Value::Number(25.0)),
            ("difficulty", text("easy")),
            ("tags", Value::Array(vec![text("warm")])),
        ]);
        assert!(validate(&recipe(), &with_tags).is_empty());
    }

    #[test]
    fn a_missing_required_member_is_reported_and_an_optional_one_may_be_null() {
        let v = obj(vec![("minutes", Value::Number(5.0)), ("difficulty", text("hard"))]);
        assert_eq!(validate(&recipe(), &v), vec![Issue { path: "$.title".into(), message: "missing".into() }]);
        let null_tags = obj(vec![
            ("title", text("x")),
            ("minutes", Value::Number(5.0)),
            ("difficulty", text("hard")),
            ("tags", Value::Null),
        ]);
        assert!(validate(&recipe(), &null_tags).is_empty());
        let null_title = obj(vec![("title", Value::Null), ("minutes", Value::Number(5.0)), ("difficulty", text("hard"))]);
        assert_eq!(validate(&recipe(), &null_title), vec![Issue { path: "$.title".into(), message: "is null".into() }]);
    }

    #[test]
    fn wrong_types_are_reported_with_the_path_to_the_member() {
        let v = obj(vec![
            ("title", Value::Number(3.0)),
            ("minutes", Value::Number(5.0)),
            ("difficulty", text("easy")),
            ("tags", Value::Array(vec![text("ok"), Value::Bool(true)])),
        ]);
        let issues = validate(&recipe(), &v);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0], Issue { path: "$.title".into(), message: "expected text, found number".into() });
        assert_eq!(issues[1], Issue { path: "$.tags[1]".into(), message: "expected text, found boolean".into() });
    }

    #[test]
    fn a_choice_and_a_numeric_range_are_enforced() {
        let v = obj(vec![("title", text("x")), ("minutes", Value::Number(300.0)), ("difficulty", text("extreme"))]);
        let issues = validate(&recipe(), &v);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].message, "300 is outside 1..=240");
        assert_eq!(issues[1].message, "\"extreme\" is not one of \"easy\", \"medium\", \"hard\"");
    }

    #[test]
    fn unknown_members_and_non_finite_numbers_are_refused() {
        let v = obj(vec![
            ("title", text("x")),
            ("minutes", Value::Number(f64::NAN)),
            ("difficulty", text("easy")),
            ("author", text("me")),
        ]);
        let issues = validate(&recipe(), &v);
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].message, "number is not finite");
        assert_eq!(issues[1], Issue { path: "$.author".into(), message: "not in the schema".into() });
    }

    #[test]
    fn describe_lists_members_types_choices_ranges_and_descriptions() {
        assert_eq!(
            recipe().describe(),
            "object { title: text (the dish name), minutes: number from 1 to 240, \
             difficulty: one of \"easy\", \"medium\", \"hard\", tags?: array of text }"
        );
    }
}
