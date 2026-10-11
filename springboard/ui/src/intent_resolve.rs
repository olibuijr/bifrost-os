//! Parameter resolution for typed actions: which input is missing, which entity text is ambiguous, which value
//! does not fit its type, and whether the action should confirm before it runs. It also picks the execution mode.
//!
//! A caller describes the declared parameters with `ParamSpec`, passes the values the user gave as text, and gets
//! one `Resolution`. Parameters are checked in declaration order and the first problem is reported, so the caller
//! can ask one question at a time. Entity text matches a candidate by id or title (case-insensitive); when several
//! candidates match and none is exact, the caller is asked to disambiguate.

/// One of the candidates an entity parameter can resolve to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: String,
    pub title: String,
}

impl Candidate {
    pub fn new(id: &str, title: &str) -> Candidate {
        Candidate { id: id.into(), title: title.into() }
    }
}

/// The value type of a parameter, as far as resolution is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Int,
    Bool,
    /// one of a fixed set of case names
    Enum(Vec<String>),
    /// an entity; with no candidates, any non-empty text is accepted as an id
    Entity(Vec<Candidate>),
}

/// A declared parameter: its name, the title shown to the user, its type and whether it may be left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamSpec {
    pub name: String,
    pub title: String,
    pub kind: Kind,
    pub optional: bool,
}

impl ParamSpec {
    pub fn new(name: &str, title: &str, kind: Kind) -> ParamSpec {
        ParamSpec { name: name.into(), title: title.into(), kind, optional: false }
    }
    pub fn optional(mut self) -> ParamSpec {
        self.optional = true;
        self
    }
}

/// The values the user supplied, by parameter name. A missing name or an empty string means "not given".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Supplied {
    pairs: Vec<(String, String)>,
}

impl Supplied {
    pub fn new() -> Supplied {
        Supplied::default()
    }
    pub fn with(mut self, name: &str, value: &str) -> Supplied {
        self.pairs.push((name.into(), value.into()));
        self
    }
    fn get(&self, name: &str) -> Option<&str> {
        self.pairs.iter().find(|(n, _)| n == name).map(|(_, v)| v.trim()).filter(|v| !v.is_empty())
    }
}

/// What the caller must do next, or the resolved values when nothing is left to ask.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// every parameter is resolved; `values` holds `(name, resolved text)` in declaration order.
    /// Optional parameters that were not given are left out.
    Ready { values: Vec<(String, String)> },
    /// a required parameter was not given
    Needs { param: String, title: String },
    /// the entity text matched several candidates; `choices` are their ids
    Disambiguate { param: String, choices: Vec<String> },
    /// the text does not fit the parameter; `reason` is short and plain
    Invalid { param: String, reason: String },
    /// all values are resolved, and the action asks before it runs; `summary` lists the values
    Confirm { summary: String },
}

/// Resolves `supplied` against `specs`. `confirm` is the action's own flag for asking before it runs.
pub fn resolve(specs: &[ParamSpec], supplied: &Supplied, confirm: bool) -> Resolution {
    let mut values = Vec::new();
    for spec in specs {
        let Some(raw) = supplied.get(&spec.name) else {
            if spec.optional {
                continue;
            }
            return Resolution::Needs { param: spec.name.clone(), title: spec.title.clone() };
        };
        match resolve_one(spec, raw) {
            Ok(text) => values.push((spec.name.clone(), text)),
            Err(res) => return res,
        }
    }
    if confirm {
        let summary = values.iter().map(|(n, v)| format!("{n}: {v}")).collect::<Vec<_>>().join(", ");
        return Resolution::Confirm { summary };
    }
    Resolution::Ready { values }
}

fn resolve_one(spec: &ParamSpec, raw: &str) -> Result<String, Resolution> {
    let invalid = |reason: &str| Resolution::Invalid { param: spec.name.clone(), reason: reason.into() };
    match &spec.kind {
        Kind::Text => Ok(raw.to_string()),
        Kind::Int => raw.parse::<i64>().map(|n| n.to_string()).map_err(|_| invalid("expected a whole number")),
        Kind::Bool => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" => Ok("true".into()),
            "false" | "no" => Ok("false".into()),
            _ => Err(invalid("expected yes or no")),
        },
        Kind::Enum(cases) => cases
            .iter()
            .find(|c| c.eq_ignore_ascii_case(raw))
            .cloned()
            .ok_or_else(|| invalid("not one of the listed cases")),
        Kind::Entity(cands) if cands.is_empty() => Ok(raw.to_string()),
        Kind::Entity(cands) => {
            // an exact id, then an exact title, wins outright
            if let Some(c) = cands.iter().find(|c| c.id == raw) {
                return Ok(c.id.clone());
            }
            let exact: Vec<&Candidate> = cands.iter().filter(|c| c.title.eq_ignore_ascii_case(raw)).collect();
            match exact.as_slice() {
                [only] => return Ok(only.id.clone()),
                [_, _, ..] => {
                    return Err(Resolution::Disambiguate {
                        param: spec.name.clone(),
                        choices: exact.iter().map(|c| c.id.clone()).collect(),
                    })
                }
                [] => {}
            }
            let lower = raw.to_lowercase();
            let partial: Vec<&Candidate> = cands.iter().filter(|c| c.title.to_lowercase().contains(&lower)).collect();
            match partial.as_slice() {
                [] => Err(invalid("no matching item")),
                [only] => Ok(only.id.clone()),
                many => Err(Resolution::Disambiguate {
                    param: spec.name.clone(),
                    choices: many.iter().map(|c| c.id.clone()).collect(),
                }),
            }
        }
    }
}

/// How an action runs: in the foreground (the app comes forward) or in the background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Foreground,
    Background,
}

/// Picks the mode to run in. The requested side is used when the action supports it; otherwise the first mode the
/// action supports. `None` only when the action declares no modes at all.
pub fn pick_mode(supported: &[Mode], want_foreground: bool) -> Option<Mode> {
    let wanted = if want_foreground { Mode::Foreground } else { Mode::Background };
    if supported.contains(&wanted) {
        return Some(wanted);
    }
    supported.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note_candidates() -> Kind {
        Kind::Entity(vec![
            Candidate::new("n1", "Shopping list"),
            Candidate::new("n2", "Shopping ideas"),
            Candidate::new("n3", "Travel"),
        ])
    }

    fn specs() -> Vec<ParamSpec> {
        vec![
            ParamSpec::new("note", "Note", note_candidates()),
            ParamSpec::new("count", "Count", Kind::Int).optional(),
        ]
    }

    #[test]
    fn a_missing_required_parameter_is_asked_first() {
        let r = resolve(&specs(), &Supplied::new().with("count", "3"), false);
        assert_eq!(r, Resolution::Needs { param: "note".into(), title: "Note".into() });
        // an empty string counts as not given
        let r = resolve(&specs(), &Supplied::new().with("note", "  "), false);
        assert!(matches!(r, Resolution::Needs { .. }));
    }

    #[test]
    fn an_exact_id_or_title_resolves_without_asking() {
        let r = resolve(&specs(), &Supplied::new().with("note", "n3"), false);
        assert_eq!(r, Resolution::Ready { values: vec![("note".into(), "n3".into())] });
        let r = resolve(&specs(), &Supplied::new().with("note", "travel"), false);
        assert_eq!(r, Resolution::Ready { values: vec![("note".into(), "n3".into())] });
    }

    #[test]
    fn several_partial_matches_ask_to_disambiguate() {
        let r = resolve(&specs(), &Supplied::new().with("note", "shopping"), false);
        assert_eq!(r, Resolution::Disambiguate { param: "note".into(), choices: vec!["n1".into(), "n2".into()] });
        // an exact title beats partial matches
        let r = resolve(&specs(), &Supplied::new().with("note", "Shopping list"), false);
        assert_eq!(r, Resolution::Ready { values: vec![("note".into(), "n1".into())] });
    }

    #[test]
    fn values_are_checked_against_their_type() {
        let r = resolve(&specs(), &Supplied::new().with("note", "n1").with("count", "three"), false);
        assert_eq!(r, Resolution::Invalid { param: "count".into(), reason: "expected a whole number".into() });
        let r = resolve(&specs(), &Supplied::new().with("note", "zzz"), false);
        assert!(matches!(r, Resolution::Invalid { ref param, .. } if param == "note"));
        let bools = vec![ParamSpec::new("on", "On", Kind::Bool)];
        let r = resolve(&bools, &Supplied::new().with("on", "YES"), false);
        assert_eq!(r, Resolution::Ready { values: vec![("on".into(), "true".into())] });
    }

    #[test]
    fn enum_cases_match_without_regard_to_case() {
        let e = vec![ParamSpec::new("mode", "Mode", Kind::Enum(vec!["Open".into(), "Closed".into()]))];
        let r = resolve(&e, &Supplied::new().with("mode", "closed"), false);
        assert_eq!(r, Resolution::Ready { values: vec![("mode".into(), "Closed".into())] });
        let r = resolve(&e, &Supplied::new().with("mode", "half"), false);
        assert!(matches!(r, Resolution::Invalid { .. }));
    }

    #[test]
    fn optional_absent_values_are_left_out_and_confirmation_comes_last() {
        let r = resolve(&specs(), &Supplied::new().with("note", "n3"), false);
        assert_eq!(r, Resolution::Ready { values: vec![("note".into(), "n3".into())] });
        let r = resolve(&specs(), &Supplied::new().with("note", "n3").with("count", "2"), true);
        assert_eq!(r, Resolution::Confirm { summary: "note: n3, count: 2".into() });
        // a missing required value is still asked before confirmation
        let r = resolve(&specs(), &Supplied::new(), true);
        assert!(matches!(r, Resolution::Needs { .. }));
    }

    #[test]
    fn entity_without_candidates_accepts_any_text() {
        let s = vec![ParamSpec::new("id", "Id", Kind::Entity(vec![]))];
        let r = resolve(&s, &Supplied::new().with("id", "anything"), false);
        assert_eq!(r, Resolution::Ready { values: vec![("id".into(), "anything".into())] });
    }

    #[test]
    fn foreground_is_used_only_when_supported() {
        assert_eq!(pick_mode(&[Mode::Foreground, Mode::Background], true), Some(Mode::Foreground));
        assert_eq!(pick_mode(&[Mode::Foreground, Mode::Background], false), Some(Mode::Background));
        // a background-only action cannot come forward: it runs in the background
        assert_eq!(pick_mode(&[Mode::Background], true), Some(Mode::Background));
        assert_eq!(pick_mode(&[Mode::Foreground], false), Some(Mode::Foreground));
        assert_eq!(pick_mode(&[], true), None);
    }
}
