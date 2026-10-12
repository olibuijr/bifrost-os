//! App Intents (`AppIntent`, `AppEntity`, `EntityQuery`, `IntentResult`, `AppShortcutsProvider`).
//!
//! An app declares typed actions with `App::intents()` and runs them in `App::perform_intent`. They are served on
//! top of the control socket (tasks/app-control.md), so `bifrost pc control call APP intent.<id> '{...}'` works:
//!
//! - `intent.list`: the declared intents (id, title, description, parameters, `shortcut`).
//! - `intent.<id>`: validates the parameters against the declaration, then `perform_intent`; the reply is
//!   `{"value": <json>, "dialog": "<text>"}` (`IntentResult`: `ProvidesValue` + `ProvidesDialog`).
//! - `entity.query`: `{"type": "Note", "suggested": true}`, `{"type": "Note", "ids": ["a","b"]}` or
//!   `{"type": "Note", "string": "milk"}` (`suggestedEntities`, `entities(for:)`, `entities(matching:)`);
//!   the reply is a list of `{id, title, subtitle?, type}`.
//!
//! The same intents are declared statically in `manifest.toml` as `[[intent]]` tables so the shell (Search) can
//! list them without launching the app; `Intent::manifest_toml` writes that text. See docs/bifrost-ui.md "App Intents".

use crate::control::{RpcError, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use crate::{App, Ctx};
use bifrost_core::json::Json;

/// `IntentParameter` value types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParamType {
    String,
    Int,
    Bool,
    /// ISO 8601 text: `2026-10-10` or `2026-10-10T14:30:00`
    Date,
    Url,
    /// the id (string) of an entity of this type
    Entity(String),
    /// one of a fixed set of case names, sent as text: `enum(open|closed)`. Case names cannot contain `|`, `)` or `"`.
    Enum(Vec<String>),
    /// a finite floating-point number (`Doubles`)
    Double,
    /// `Currencies`: an object `{"amount": <finite number>, "code": "<ISO 4217 code, three capitals>"}`
    Currency,
}

impl ParamType {
    /// `string`, `int`, `bool`, `date`, `url`, `entity(Type)` or `enum(a|b)`.
    pub fn name(&self) -> String {
        match self {
            ParamType::String => "string".into(),
            ParamType::Int => "int".into(),
            ParamType::Bool => "bool".into(),
            ParamType::Date => "date".into(),
            ParamType::Url => "url".into(),
            ParamType::Entity(t) => format!("entity({t})"),
            ParamType::Enum(cases) => format!("enum({})", cases.join("|")),
            ParamType::Double => "double".into(),
            ParamType::Currency => "currency".into(),
        }
    }
    pub fn parse(s: &str) -> Option<ParamType> {
        Some(match s.trim() {
            "string" => ParamType::String,
            "int" => ParamType::Int,
            "bool" => ParamType::Bool,
            "date" => ParamType::Date,
            "url" => ParamType::Url,
            "double" => ParamType::Double,
            "currency" => ParamType::Currency,
            t if t.starts_with("enum(") => {
                let inner = t.strip_prefix("enum(")?.strip_suffix(')')?;
                let cases: Vec<String> = inner.split('|').map(|c| c.trim().to_string()).collect();
                if cases.iter().any(|c| c.is_empty() || c.contains([')', '"'])) {
                    return None;
                }
                ParamType::Enum(cases)
            }
            t => {
                let inner = t.strip_prefix("entity(")?.strip_suffix(')')?.trim();
                if inner.is_empty() {
                    return None;
                }
                ParamType::Entity(inner.into())
            }
        })
    }
    /// True when `v` is acceptable for this type.
    pub fn accepts(&self, v: &Json) -> bool {
        match (self, v) {
            (ParamType::String, Json::Str(_)) => true,
            (ParamType::Int, Json::Num(n)) => n.is_finite() && n.fract() == 0.0,
            (ParamType::Bool, Json::Bool(_)) => true,
            (ParamType::Date, Json::Str(s)) => is_iso_date(s),
            (ParamType::Url, Json::Str(s)) => s
                .split_once(':')
                .is_some_and(|(scheme, rest)| !scheme.is_empty() && !rest.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))),
            (ParamType::Entity(_), Json::Str(s)) => !s.is_empty(),
            (ParamType::Enum(cases), Json::Str(s)) => cases.iter().any(|c| c == s),
            (ParamType::Double, Json::Num(n)) => n.is_finite(),
            (ParamType::Currency, Json::Obj(_)) => {
                v.get("amount").and_then(|a| a.number()).is_some_and(f64::is_finite) && v.get("code").and_then(|c| c.as_str()).is_some_and(is_currency_code)
            }
            _ => false,
        }
    }
}

/// An ISO 4217 code as written in parameters: exactly three ASCII capital letters.
fn is_currency_code(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|b| b.is_ascii_uppercase())
}

/// `YYYY-MM-DD` optionally followed by `T` and `hh:mm` (then anything, such as seconds or a zone).
fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r).is_some_and(|d| d.len() > 0 && d.iter().all(u8::is_ascii_digit));
    if b.len() < 10 || !digits(0..4) || b[4] != b'-' || !digits(5..7) || b[7] != b'-' || !digits(8..10) {
        return false;
    }
    let (m, d) = (s[5..7].parse::<u32>().unwrap_or(0), s[8..10].parse::<u32>().unwrap_or(0));
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return false;
    }
    if b.len() == 10 {
        return true;
    }
    b.len() >= 16
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && s[11..13].parse::<u32>().unwrap_or(99) < 24
        && s[14..16].parse::<u32>().unwrap_or(99) < 60
}

/// `@Parameter`: a named, typed input of an intent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub title: String,
    pub ty: ParamType,
    pub optional: bool,
}

impl Param {
    pub fn new(name: &str, title: &str, ty: ParamType) -> Param {
        Param { name: name.into(), title: title.into(), ty, optional: false }
    }
    pub fn optional(mut self) -> Param {
        self.optional = true;
        self
    }
}

/// `IntentModes`: a mode an intent may run in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntentMode {
    /// the action runs without bringing the app forward
    Background,
    /// the action needs the app on screen
    Foreground,
}

impl IntentMode {
    pub fn name(self) -> &'static str {
        match self {
            IntentMode::Background => "background",
            IntentMode::Foreground => "foreground",
        }
    }
}

/// `IntentAuthenticationPolicy`: what the system checks before an intent runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Authentication {
    /// runs at any time, including while the device is locked
    #[default]
    AlwaysAllowed,
    /// needs the person to authenticate first
    RequiresAuthentication,
    /// needs the person to unlock the device
    RequiresLocalDeviceAuthentication,
}

impl Authentication {
    pub fn name(self) -> &'static str {
        match self {
            Authentication::AlwaysAllowed => "always-allowed",
            Authentication::RequiresAuthentication => "requires-authentication",
            Authentication::RequiresLocalDeviceAuthentication => "requires-local-device-authentication",
        }
    }
    /// True when `session` satisfies this policy.
    pub fn allows(self, session: Session) -> bool {
        match self {
            Authentication::AlwaysAllowed => true,
            Authentication::RequiresAuthentication => session.authenticated,
            Authentication::RequiresLocalDeviceAuthentication => !session.locked,
        }
    }
}

/// The device state an authentication policy is checked against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session {
    pub locked: bool,
    pub authenticated: bool,
}

/// `AppIntent`: a typed action. `id` is a short token (`[a-z0-9_-]`): the RPC method is `intent.<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub id: String,
    pub title: String,
    pub description: String,
    pub params: Vec<Param>,
    /// `AppShortcut`: Search lists it (only when it has no required parameter) without opening the app
    pub shortcut: bool,
    /// `supportedModes`: where the intent may run. Background only unless `foreground()` is called.
    pub modes: Vec<IntentMode>,
    /// `authenticationPolicy`: what the system must check before the intent runs.
    pub authentication: Authentication,
}

impl Intent {
    pub fn new(id: &str, title: &str, description: &str) -> Intent {
        Intent {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            params: Vec::new(),
            shortcut: false,
            modes: vec![IntentMode::Background],
            authentication: Authentication::AlwaysAllowed,
        }
    }
    /// Adds the foreground mode: the intent may bring the app forward (see [`IntentResult::continue_in_foreground`]).
    pub fn foreground(mut self) -> Intent {
        if !self.supports(IntentMode::Foreground) {
            self.modes.push(IntentMode::Foreground);
        }
        self
    }
    pub fn authentication(mut self, policy: Authentication) -> Intent {
        self.authentication = policy;
        self
    }
    /// True when the intent declares `mode`.
    pub fn supports(&self, mode: IntentMode) -> bool {
        self.modes.contains(&mode)
    }
    /// Whether the intent may run in `session`, given its authentication policy. The shell or app supplies the state.
    pub fn may_run(&self, session: Session) -> bool {
        self.authentication.allows(session)
    }
    /// Checks a result against the declaration: a continue-in-foreground request needs the foreground mode, and an
    /// `opens` follow-on must name a declared intent whose parameters are valid. `declared` is every intent of the app.
    pub fn check_result(&self, result: &IntentResult, declared: &[Intent]) -> Result<(), RpcError> {
        if result.continue_foreground && !self.supports(IntentMode::Foreground) {
            return Err(failed(format!("intent {} cannot continue in the foreground", self.id)));
        }
        if let Some((target, params)) = &result.opens {
            let next = declared
                .iter()
                .find(|i| &i.id == target)
                .ok_or_else(|| failed(format!("intent {} opens an undeclared intent {target}", self.id)))?;
            next.validate(params)?;
        }
        Ok(())
    }
    pub fn param(mut self, p: Param) -> Intent {
        self.params.push(p);
        self
    }
    pub fn shortcut(mut self) -> Intent {
        self.shortcut = true;
        self
    }
    pub fn method(&self) -> String {
        format!("intent.{}", self.id)
    }
    pub fn to_json(&self) -> Json {
        Json::Obj(vec![
            ("id".into(), Json::Str(self.id.clone())),
            ("title".into(), Json::Str(self.title.clone())),
            ("description".into(), Json::Str(self.description.clone())),
            ("shortcut".into(), Json::Bool(self.shortcut)),
            ("modes".into(), Json::Arr(self.modes.iter().map(|m| Json::Str(m.name().into())).collect())),
            ("authentication".into(), Json::Str(self.authentication.name().into())),
            (
                "parameters".into(),
                Json::Arr(
                    self.params
                        .iter()
                        .map(|p| {
                            Json::Obj(vec![
                                ("name".into(), Json::Str(p.name.clone())),
                                ("title".into(), Json::Str(p.title.clone())),
                                ("type".into(), Json::Str(p.ty.name())),
                                ("optional".into(), Json::Bool(p.optional)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }
    /// The `[[intent]]` table for `manifest.toml` (the shell reads `id`, `title`, `description`, `shortcut`, `params`).
    pub fn manifest_toml(&self) -> String {
        let params: Vec<String> = self.params.iter().map(|p| format!("\"{}:{}{}\"", p.name, p.ty.name(), if p.optional { "?" } else { "" })).collect();
        format!(
            "[[intent]]\nid = \"{}\"\ntitle = \"{}\"\ndescription = \"{}\"\nshortcut = {}\nparams = [{}]\n",
            self.id,
            self.title.replace('"', "'"),
            self.description.replace('"', "'"),
            self.shortcut,
            params.join(", ")
        )
    }

    /// Checks `params` (a JSON object) against the declaration: required ones present, types right, no unknown names.
    pub fn validate(&self, params: &Json) -> Result<(), RpcError> {
        let fields: &[(String, Json)] = match params {
            Json::Obj(f) => f,
            Json::Null => &[],
            _ => return Err(RpcError::new(INVALID_PARAMS, "params must be an object")),
        };
        for (k, _) in fields {
            if !self.params.iter().any(|p| &p.name == k) {
                return Err(RpcError::new(INVALID_PARAMS, format!("unknown parameter {k}")));
            }
        }
        for p in &self.params {
            match fields.iter().find(|(k, _)| k == &p.name).map(|(_, v)| v) {
                None | Some(Json::Null) if !p.optional => return Err(RpcError::new(INVALID_PARAMS, format!("missing parameter {} ({})", p.name, p.ty.name()))),
                None | Some(Json::Null) => {}
                Some(v) if !p.ty.accepts(v) => return Err(RpcError::new(INVALID_PARAMS, format!("parameter {} must be {}", p.name, p.ty.name()))),
                Some(_) => {}
            }
        }
        Ok(())
    }
}

/// `AppEntity`: something an intent parameter can refer to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entity {
    pub id: String,
    pub ty: String,
    pub title: String,
    pub subtitle: Option<String>,
}

impl Entity {
    pub fn new(ty: &str, id: &str, title: &str) -> Entity {
        Entity { id: id.into(), ty: ty.into(), title: title.into(), subtitle: None }
    }
    pub fn subtitle(mut self, s: &str) -> Entity {
        self.subtitle = Some(s.into());
        self
    }
    pub fn to_json(&self) -> Json {
        let mut f = vec![
            ("id".to_string(), Json::Str(self.id.clone())),
            ("type".to_string(), Json::Str(self.ty.clone())),
            ("title".to_string(), Json::Str(self.title.clone())),
        ];
        if let Some(s) = &self.subtitle {
            f.push(("subtitle".into(), Json::Str(s.clone())));
        }
        Json::Obj(f)
    }
}

/// `EntityQuery` (`EntityStringQuery`, `suggestedEntities`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityQuery {
    Suggested,
    Ids(Vec<String>),
    String(String),
}

/// `IntentResult`: a value (`ProvidesValue`) and the dialog to show or speak (`ProvidesDialog`).
#[derive(Clone, Debug, PartialEq)]
pub struct IntentResult {
    pub value: Json,
    pub dialog: Option<String>,
    /// `continueInForeground`: ask the system to bring the app forward. The intent must declare the foreground mode.
    pub continue_foreground: bool,
    /// `result(opensIntent:)`: the next intent to run, with its parameters (checked by `Intent::check_result`).
    pub opens: Option<(String, Json)>,
    /// `result(view:)`: a result view shown next to the dialog.
    pub snippet: Option<Snippet>,
}

impl IntentResult {
    pub fn value(value: Json) -> IntentResult {
        IntentResult { value, dialog: None, continue_foreground: false, opens: None, snippet: None }
    }
    pub fn dialog(text: &str) -> IntentResult {
        IntentResult { value: Json::Null, dialog: Some(text.into()), continue_foreground: false, opens: None, snippet: None }
    }
    pub fn with_snippet(mut self, snippet: Snippet) -> IntentResult {
        self.snippet = Some(snippet);
        self
    }
    pub fn with_dialog(mut self, text: &str) -> IntentResult {
        self.dialog = Some(text.into());
        self
    }
    pub fn continue_in_foreground(mut self) -> IntentResult {
        self.continue_foreground = true;
        self
    }
    pub fn opens(mut self, intent_id: &str, params: Json) -> IntentResult {
        self.opens = Some((intent_id.into(), params));
        self
    }
    /// `{"value", "dialog"}`, plus `continueInForeground` and `opens` only when they are set.
    pub fn to_json(&self) -> Json {
        let mut f = vec![("value".into(), self.value.clone()), ("dialog".into(), self.dialog.clone().map_or(Json::Null, Json::Str))];
        if self.continue_foreground {
            f.push(("continueInForeground".into(), Json::Bool(true)));
        }
        if let Some((id, params)) = &self.opens {
            f.push(("opens".into(), Json::Obj(vec![("intent".into(), Json::Str(id.clone())), ("parameters".into(), params.clone())])));
        }
        if let Some(snippet) = &self.snippet {
            f.push(("snippet".into(), snippet.to_json()));
        }
        Json::Obj(f)
    }
    /// Reads back a reply produced by `to_json` (`{"value": .., "dialog": text or null}`); `None` for any other shape.
    pub fn from_json(j: &Json) -> Option<IntentResult> {
        let value = j.get("value")?.clone();
        let dialog = match j.get("dialog")? {
            Json::Null => None,
            Json::Str(s) => Some(s.clone()),
            _ => return None,
        };
        let continue_foreground = matches!(j.get("continueInForeground"), Some(Json::Bool(true)));
        let opens = match j.get("opens") {
            None => None,
            Some(o) => Some((o.get("intent")?.as_str()?.to_string(), o.get("parameters")?.clone())),
        };
        let snippet = match j.get("snippet") {
            None | Some(Json::Null) => None,
            Some(s) => Some(Snippet::from_json(s)?),
        };
        Some(IntentResult { value, dialog, continue_foreground, opens, snippet })
    }
    /// The value at `path` (see [`PropertyPath`]), or `None` when the path does not exist in `value`.
    pub fn at(&self, path: &str) -> Option<&Json> {
        PropertyPath::parse(path)?.resolve(&self.value)
    }
}

/// `IntentResult.Snippet`: a result view that goes with the dialog. It is a title and an ordered list of label and
/// text rows; the shell draws the rows, the app only builds the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub title: String,
    pub rows: Vec<SnippetRow>,
}

/// One row of a [`Snippet`]: a caption and the text beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnippetRow {
    pub label: String,
    pub text: String,
}

impl Snippet {
    pub fn new(title: &str) -> Snippet {
        Snippet { title: title.into(), rows: Vec::new() }
    }
    pub fn row(mut self, label: &str, text: &str) -> Snippet {
        self.rows.push(SnippetRow { label: label.into(), text: text.into() });
        self
    }
    /// `{"title": text, "rows": [{"label": text, "text": text}, ...]}`
    pub fn to_json(&self) -> Json {
        Json::Obj(vec![
            ("title".into(), Json::Str(self.title.clone())),
            (
                "rows".into(),
                Json::Arr(
                    self.rows
                        .iter()
                        .map(|r| Json::Obj(vec![("label".into(), Json::Str(r.label.clone())), ("text".into(), Json::Str(r.text.clone()))]))
                        .collect(),
                ),
            ),
        ])
    }
    /// Reads back `to_json`; `None` when the title is missing or any row lacks a label or text.
    pub fn from_json(j: &Json) -> Option<Snippet> {
        let title = j.get("title")?.as_str()?.to_string();
        let rows = j
            .get("rows")?
            .as_array()?
            .iter()
            .map(|r| Some(SnippetRow { label: r.get("label")?.as_str()?.to_string(), text: r.get("text")?.as_str()?.to_string() }))
            .collect::<Option<Vec<SnippetRow>>>()?;
        Some(Snippet { title, rows })
    }
}

/// One step of a [`PropertyPath`]: an object member by name, or an array element by index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathStep {
    Key(String),
    Index(usize),
}

/// A path into a nested intent value, such as `items[2].title`, `items.2.title` or `value.0`. Tests use it to read
/// one field of a result, the way a key path reads a property of a typed result; the path is checked when parsed,
/// not against a type, so a wrong name only shows as `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyPath {
    pub steps: Vec<PathStep>,
}

impl PropertyPath {
    /// Parses `.`-separated segments. A segment is a name with zero or more `[n]` indexes after it, or a bare number
    /// (an array index). Empty segments, a leading index and non-numeric indexes are rejected.
    pub fn parse(path: &str) -> Option<PropertyPath> {
        let mut steps = Vec::new();
        for seg in path.split('.') {
            if !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit()) {
                steps.push(PathStep::Index(seg.parse().ok()?));
                continue;
            }
            let (name, mut rest) = seg.split_at(seg.find('[').unwrap_or(seg.len()));
            if name.is_empty() {
                return None;
            }
            steps.push(PathStep::Key(name.into()));
            while let Some(inner) = rest.strip_prefix('[') {
                let (digits, after) = inner.split_once(']')?;
                steps.push(PathStep::Index(digits.parse().ok()?));
                rest = after;
            }
            if !rest.is_empty() {
                return None;
            }
        }
        if steps.is_empty() {
            None
        } else {
            Some(PropertyPath { steps })
        }
    }
    /// The value this path reaches under `root`, or `None` if a name is missing, an index is out of range, or a step
    /// meets a value of the wrong kind.
    pub fn resolve<'a>(&self, root: &'a Json) -> Option<&'a Json> {
        self.steps.iter().try_fold(root, |v, step| match step {
            PathStep::Key(k) => v.get(k),
            PathStep::Index(i) => v.as_array()?.get(*i),
        })
    }
}

/// Runs the intent `id` in-process, the way a test does: the parameters are checked against the declaration, then
/// `perform_intent` returns the typed result. `dispatch` gives the JSON reply the control socket sends; this gives the
/// `IntentResult` itself, so a test can read `value`, `dialog` or `at(path)` directly.
pub fn run<A: App>(app: &mut A, ctx: Option<&Ctx>, data_dir: &std::path::Path, id: &str, params: &Json) -> Result<IntentResult, RpcError> {
    let intent = app.intents().into_iter().find(|i| i.id == id).ok_or_else(|| RpcError::new(METHOD_NOT_FOUND, format!("no intent {id}")))?;
    intent.validate(params)?;
    let result = app.perform_intent(ctx, data_dir, id, params)?;
    intent.check_result(&result, &app.intents())?;
    Ok(result)
}

/// The `entity.query` request in `params`.
pub fn parse_entity_query(params: &Json) -> Result<(String, EntityQuery), RpcError> {
    let ty = params.get("type").and_then(Json::as_str).filter(|t| !t.is_empty()).ok_or_else(|| RpcError::new(INVALID_PARAMS, "entity.query needs a \"type\""))?;
    let q = if let Some(ids) = params.get("ids") {
        let ids = ids.as_array().ok_or_else(|| RpcError::new(INVALID_PARAMS, "\"ids\" must be an array of strings"))?;
        EntityQuery::Ids(ids.iter().filter_map(|i| i.as_str().map(str::to_string)).collect())
    } else if let Some(s) = params.get("string") {
        EntityQuery::String(s.as_str().ok_or_else(|| RpcError::new(INVALID_PARAMS, "\"string\" must be text"))?.to_string())
    } else {
        EntityQuery::Suggested
    };
    Ok((ty.to_string(), q))
}

/// Intent methods the app answers, for `app.methods`: `intent.list`, `entity.query` and one per intent.
pub fn methods<A: App>(app: &A) -> Vec<String> {
    let intents = app.intents();
    if intents.is_empty() {
        return Vec::new();
    }
    let mut v = vec!["intent.list".to_string(), "entity.query".to_string()];
    v.extend(intents.iter().map(Intent::method));
    v
}

/// Serves `intent.*` and `entity.query`; `None` for any other method (the caller falls through to `App::rpc`).
pub fn dispatch<A: App>(app: &mut A, ctx: Option<&Ctx>, data_dir: &std::path::Path, method: &str, params: &Json) -> Option<Result<Json, RpcError>> {
    if method != "entity.query" && !method.starts_with("intent.") {
        return None;
    }
    let intents = app.intents();
    if intents.is_empty() {
        return None; // an app without intents may still own these names
    }
    Some(match method {
        "intent.list" => Ok(Json::Arr(intents.iter().map(Intent::to_json).collect())),
        "entity.query" => parse_entity_query(params).and_then(|(ty, q)| {
            let found = app.entities(data_dir, &ty, &q)?;
            Ok(Json::Arr(found.iter().map(Entity::to_json).collect()))
        }),
        m => {
            let id = &m["intent.".len()..];
            match intents.iter().find(|i| i.id == id) {
                None => Err(RpcError::new(METHOD_NOT_FOUND, format!("no intent {id}"))),
                Some(intent) => intent
                    .validate(params)
                    .and_then(|_| app.perform_intent(ctx, data_dir, id, params))
                    .and_then(|r| intent.check_result(&r, &intents).map(|_| r))
                    .map(|r| r.to_json()),
            }
        }
    })
}

/// Convenience for `App::perform_intent` bodies: a required string parameter.
pub fn str_param<'a>(params: &'a Json, name: &str) -> Result<&'a str, RpcError> {
    params.get(name).and_then(Json::as_str).ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("parameter {name} must be text")))
}

/// Convenience: an internal failure while performing an intent.
pub fn failed(message: impl Into<String>) -> RpcError {
    RpcError::new(INTERNAL_ERROR, message)
}

#[cfg(test)]
mod tests {
    #[test]
    fn snippet_rows_keep_their_order_and_round_trip_through_a_result() {
        let snippet = Snippet::new("Totals").row("Net", "100").row("VAT", "24");
        assert_eq!(snippet.rows.len(), 2);
        assert_eq!(snippet.rows[1], SnippetRow { label: "VAT".into(), text: "24".into() });
        let r = IntentResult::value(Json::Null).with_snippet(snippet.clone());
        let j = r.to_json();
        assert_eq!(j.get("snippet").and_then(|s| s.get("title")).and_then(Json::as_str), Some("Totals"));
        let back = IntentResult::from_json(&j).unwrap();
        assert_eq!(back.snippet, Some(snippet));
        assert_eq!(back, r);
    }

    #[test]
    fn result_without_a_snippet_has_no_snippet_key() {
        let plain = IntentResult::value(Json::Num(1.0)).with_dialog("Done");
        assert!(plain.to_json().get("snippet").is_none());
        let back = IntentResult::from_json(&json::parse(r#"{"value":1,"dialog":"Done","snippet":null}"#).unwrap()).unwrap();
        assert_eq!(back.snippet, None);
        assert_eq!(back, plain);
    }

    #[test]
    fn snippet_from_json_rejects_a_row_without_text() {
        let bad_row = json::parse(r#"{"title":"x","rows":[{"label":"a"}]}"#).unwrap();
        assert_eq!(Snippet::from_json(&bad_row), None);
        assert_eq!(Snippet::from_json(&json::parse(r#"{"rows":[]}"#).unwrap()), None, "a title is required");
        let empty = Snippet::from_json(&json::parse(r#"{"title":"Empty","rows":[]}"#).unwrap()).unwrap();
        assert!(empty.rows.is_empty());
        assert_eq!(IntentResult::from_json(&json::parse(r#"{"value":1,"dialog":null,"snippet":{"rows":[]}}"#).unwrap()), None);
    }

    #[test]
    fn double_parameter_takes_finite_numbers_only() {
        assert_eq!(ParamType::parse("double"), Some(ParamType::Double));
        assert_eq!(ParamType::Double.name(), "double");
        assert!(ParamType::Double.accepts(&Json::Num(2.5)));
        assert!(ParamType::Double.accepts(&Json::Num(-3.0)));
        assert!(!ParamType::Double.accepts(&Json::Num(f64::INFINITY)));
        assert!(!ParamType::Double.accepts(&Json::Num(f64::NAN)));
        assert!(!ParamType::Double.accepts(&Json::Str("2.5".into())));
    }

    #[test]
    fn currency_parameter_needs_an_amount_and_a_three_letter_code() {
        let money = |amount: Json, code: &str| Json::Obj(vec![("amount".into(), amount), ("code".into(), Json::Str(code.into()))]);
        assert_eq!(ParamType::parse("currency"), Some(ParamType::Currency));
        assert!(ParamType::Currency.accepts(&money(Json::Num(12.5), "ISK")));
        assert!(ParamType::Currency.accepts(&money(Json::Num(-4.0), "EUR")), "refunds are negative");
        assert!(!ParamType::Currency.accepts(&money(Json::Num(1.0), "isk")), "codes are capitals");
        assert!(!ParamType::Currency.accepts(&money(Json::Num(1.0), "ISKK")), "codes are three letters");
        assert!(!ParamType::Currency.accepts(&money(Json::Num(f64::INFINITY), "USD")), "amount is finite");
        assert!(!ParamType::Currency.accepts(&Json::Obj(vec![("code".into(), Json::Str("USD".into()))])), "amount is required");
        assert!(!ParamType::Currency.accepts(&Json::Str("ISK 12".into())), "a bare string is not a currency");
    }

    #[test]
    fn validate_checks_a_currency_parameter_by_its_declaration() {
        let pay = Intent::new("pay", "Pay", "").param(Param::new("price", "Price", ParamType::Currency));
        let ok = json::parse(r#"{"price":{"amount":9.99,"code":"USD"}}"#).unwrap();
        assert!(pay.validate(&ok).is_ok());
        let bad = json::parse(r#"{"price":{"amount":9.99,"code":"usd"}}"#).unwrap();
        assert!(pay.validate(&bad).is_err());
        assert!(pay.manifest_toml().contains(r#""price:currency""#));
    }

    use super::*;
    use bifrost_core::json;

    fn add() -> Intent {
        Intent::new("add-note", "Add Note", "Creates a note.")
            .param(Param::new("text", "Text", ParamType::String))
            .param(Param::new("due", "Due", ParamType::Date).optional())
            .param(Param::new("folder", "Folder", ParamType::Entity("Folder".into())).optional())
            .shortcut()
    }

    #[test]
    fn param_types_round_trip_by_name() {
        for t in ["string", "int", "bool", "date", "url", "entity(Note)"] {
            assert_eq!(ParamType::parse(t).unwrap().name(), t);
        }
        assert!(ParamType::parse("entity()").is_none());
        assert!(ParamType::parse("float").is_none());
    }

    #[test]
    fn types_accept_only_their_values() {
        assert!(ParamType::Int.accepts(&Json::Num(3.0)));
        assert!(!ParamType::Int.accepts(&Json::Num(3.5)));
        assert!(!ParamType::Bool.accepts(&Json::Str("true".into())));
        assert!(ParamType::Date.accepts(&Json::Str("2026-10-10".into())));
        assert!(ParamType::Date.accepts(&Json::Str("2026-10-10T14:30:00".into())));
        assert!(!ParamType::Date.accepts(&Json::Str("2026-13-10".into())));
        assert!(!ParamType::Date.accepts(&Json::Str("tomorrow".into())));
        assert!(ParamType::Url.accepts(&Json::Str("https://example.com".into())));
        assert!(!ParamType::Url.accepts(&Json::Str("example.com".into())));
        assert!(!ParamType::Entity("Note".into()).accepts(&Json::Str(String::new())));
    }

    #[test]
    fn validate_checks_required_unknown_and_types() {
        let i = add();
        assert!(i.validate(&json::parse(r#"{"text":"milk"}"#).unwrap()).is_ok());
        assert!(i.validate(&json::parse(r#"{"text":"milk","due":"2026-10-11","folder":"f1"}"#).unwrap()).is_ok());
        assert_eq!(i.validate(&json::parse("{}").unwrap()).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(i.validate(&json::parse(r#"{"text":1}"#).unwrap()).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(i.validate(&json::parse(r#"{"text":"a","nope":1}"#).unwrap()).unwrap_err().code, INVALID_PARAMS);
        assert!(i.validate(&json::parse(r#"{"text":"a","due":null}"#).unwrap()).is_ok());
    }

    #[test]
    fn manifest_text_lists_params_with_optional_marker() {
        let t = add().manifest_toml();
        assert!(t.starts_with("[[intent]]\nid = \"add-note\"\n"));
        assert!(t.contains("shortcut = true"));
        assert!(t.contains("params = [\"text:string\", \"due:date?\", \"folder:entity(Folder)?\"]"));
    }

    #[test]
    fn entity_query_forms() {
        let q = |s: &str| parse_entity_query(&json::parse(s).unwrap());
        assert_eq!(q(r#"{"type":"Note"}"#).unwrap(), ("Note".into(), EntityQuery::Suggested));
        assert_eq!(q(r#"{"type":"Note","ids":["a","b"]}"#).unwrap().1, EntityQuery::Ids(vec!["a".into(), "b".into()]));
        assert_eq!(q(r#"{"type":"Note","string":"milk"}"#).unwrap().1, EntityQuery::String("milk".into()));
        assert!(q(r#"{"ids":[]}"#).is_err());
    }

    #[test]
    fn result_and_entity_json() {
        let r = IntentResult::value(Json::Num(2.0)).with_dialog("Done");
        assert_eq!(crate::control::encode(&r.to_json()), r#"{"value":2,"dialog":"Done"}"#);
        let e = Entity::new("Note", "n1", "Milk").subtitle("Home");
        assert_eq!(crate::control::encode(&e.to_json()), r#"{"id":"n1","type":"Note","title":"Milk","subtitle":"Home"}"#);
    }

    #[test]
    fn param_type_parse_trims_and_rejects_non_values() {
        assert_eq!(ParamType::parse("  int "), Some(ParamType::Int));
        assert_eq!(ParamType::parse("entity( Note )"), Some(ParamType::Entity("Note".into())));
        assert!(ParamType::parse("entity(Note").is_none());
        assert!(ParamType::parse("").is_none());
    }

    #[test]
    fn date_and_url_validation_edges() {
        let date = |s: &str| ParamType::Date.accepts(&Json::Str(s.into()));
        assert!(!date("2026-10-10T25:00"), "hour out of range");
        assert!(!date("2026-10-10T14:61"), "minute out of range");
        assert!(!date("2026-10-10X14:30"));
        assert!(!date("2026-00-10") && !date("2026-10-32"));
        let url = |s: &str| ParamType::Url.accepts(&Json::Str(s.into()));
        assert!(url("mailto:a@b.c") && !url("://x") && !url("http:"));
        assert!(!ParamType::Int.accepts(&Json::Num(f64::NAN)));
        assert!(!ParamType::String.accepts(&Json::Null));
    }

    #[test]
    fn validate_accepts_null_params_only_without_required_ones() {
        assert_eq!(add().validate(&Json::Null).unwrap_err().code, INVALID_PARAMS);
        assert!(Intent::new("ping", "Ping", "").validate(&Json::Null).is_ok());
        assert_eq!(add().validate(&Json::Str("x".into())).unwrap_err().code, INVALID_PARAMS, "params must be an object");
    }

    #[test]
    fn intent_json_and_method_name() {
        let i = add();
        assert_eq!(i.method(), "intent.add-note");
        let j = i.to_json();
        assert_eq!(j.get("shortcut"), Some(&Json::Bool(true)));
        let ps = j.get("parameters").and_then(Json::as_array).unwrap();
        assert_eq!(ps.len(), 3);
        assert_eq!(ps[2].get("type").and_then(Json::as_str), Some("entity(Folder)"));
        assert_eq!(ps[0].get("optional"), Some(&Json::Bool(false)));
    }

    #[test]
    fn manifest_text_replaces_double_quotes() {
        let t = Intent::new("q", "Say \"hi\"", "A \"quote\"").manifest_toml();
        assert!(t.contains("title = \"Say 'hi'\"") && t.contains("description = \"A 'quote'\""));
        assert!(t.ends_with("params = []\n"));
    }

    #[test]
    fn entity_query_rejects_malformed_fields() {
        let q = |s: &str| parse_entity_query(&json::parse(s).unwrap());
        assert!(q(r#"{"type":""}"#).is_err());
        assert!(q(r#"{"type":"Note","ids":"a"}"#).is_err());
        assert!(q(r#"{"type":"Note","string":3}"#).is_err());
        assert_eq!(q(r#"{"type":"Note","ids":["a",1]}"#).unwrap().1, EntityQuery::Ids(vec!["a".into()]), "non-text ids are skipped");
    }

    #[test]
    fn helpers_and_result_constructors() {
        let p = json::parse(r#"{"a":"x","n":1}"#).unwrap();
        assert_eq!(str_param(&p, "a").unwrap(), "x");
        assert_eq!(str_param(&p, "n").unwrap_err().code, INVALID_PARAMS);
        assert_eq!(str_param(&p, "missing").unwrap_err().code, INVALID_PARAMS);
        assert_eq!(failed("boom").code, INTERNAL_ERROR);
        let d = IntentResult::dialog("Hi");
        assert_eq!((d.value.clone(), d.dialog.as_deref()), (Json::Null, Some("Hi")));
        assert_eq!(IntentResult::value(Json::Bool(true)).dialog, None);
    }

    struct Bare;
    impl App for Bare {
        fn id(&self) -> &str {
            "bare"
        }
        fn draw(&mut self, _ctx: &Ctx, _s: &mut crate::Surface, _t: f64) -> crate::Damage {
            crate::Damage::new(1, 1)
        }
    }

    #[test]
    fn an_app_without_intents_answers_nothing() {
        let mut app = Bare;
        assert!(methods(&app).is_empty());
        assert!(dispatch(&mut app, None, std::path::Path::new("/tmp"), "intent.list", &Json::Null).is_none());
    }

    struct Demo;
    impl App for Demo {
        fn id(&self) -> &str {
            "demo"
        }
        fn draw(&mut self, _ctx: &Ctx, _s: &mut crate::Surface, _t: f64) -> crate::Damage {
            crate::Damage::new(1, 1)
        }
        fn intents(&self) -> Vec<Intent> {
            vec![add()]
        }
        fn perform_intent(&mut self, _c: Option<&Ctx>, _d: &std::path::Path, id: &str, p: &Json) -> Result<IntentResult, RpcError> {
            assert_eq!(id, "add-note");
            Ok(IntentResult::value(Json::Str(str_param(p, "text")?.into())).with_dialog("Added"))
        }
        fn entities(&mut self, _d: &std::path::Path, ty: &str, q: &EntityQuery) -> Result<Vec<Entity>, RpcError> {
            match (ty, q) {
                ("Folder", EntityQuery::String(s)) => Ok(vec![Entity::new("Folder", "f1", s)]),
                _ => Ok(vec![]),
            }
        }
    }

    #[test]
    fn dispatch_lists_performs_and_queries() {
        let d = std::path::Path::new("/tmp");
        let mut app = Demo;
        let list = dispatch(&mut app, None, d, "intent.list", &Json::Null).unwrap().unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        let ok = dispatch(&mut app, None, d, "intent.add-note", &json::parse(r#"{"text":"milk"}"#).unwrap()).unwrap().unwrap();
        assert_eq!(ok.get("dialog").and_then(Json::as_str), Some("Added"));
        assert_eq!(ok.get("value").and_then(Json::as_str), Some("milk"));
        let bad = dispatch(&mut app, None, d, "intent.add-note", &json::parse("{}").unwrap()).unwrap().unwrap_err();
        assert_eq!(bad.code, INVALID_PARAMS);
        let nf = dispatch(&mut app, None, d, "intent.missing", &Json::Null).unwrap().unwrap_err();
        assert_eq!(nf.code, METHOD_NOT_FOUND);
        let ents = dispatch(&mut app, None, d, "entity.query", &json::parse(r#"{"type":"Folder","string":"Home"}"#).unwrap()).unwrap().unwrap();
        assert_eq!(ents.as_array().unwrap()[0].get("title").and_then(Json::as_str), Some("Home"));
        assert!(dispatch(&mut app, None, d, "notes.list", &Json::Null).is_none());
        assert_eq!(methods(&app), vec!["intent.list", "entity.query", "intent.add-note"]);
    }

    #[test]
    fn enum_parameter_parses_names_and_accepts_only_its_cases() {
        let t = ParamType::parse("enum(open|closed)").unwrap();
        assert_eq!(t, ParamType::Enum(vec!["open".into(), "closed".into()]));
        assert_eq!(t.name(), "enum(open|closed)");
        assert_eq!(ParamType::parse(" enum( open | closed ) ").unwrap().name(), "enum(open|closed)");
        assert!(t.accepts(&Json::Str("closed".into())));
        assert!(!t.accepts(&Json::Str("maybe".into())), "a case that is not declared");
        assert!(!t.accepts(&Json::Num(1.0)), "enum values are text");
        assert!(ParamType::parse("enum()").is_none());
        assert!(ParamType::parse("enum(a||b)").is_none(), "empty case");
        assert!(ParamType::parse("enum(a)b)").is_none(), "a case cannot contain ')'");
    }

    #[test]
    fn enum_parameter_validates_and_lands_in_manifest_text() {
        let i = Intent::new("set-mode", "Set Mode", "").param(Param::new("mode", "Mode", ParamType::Enum(vec!["open".into(), "closed".into()])));
        assert!(i.validate(&json::parse(r#"{"mode":"open"}"#).unwrap()).is_ok());
        let bad = i.validate(&json::parse(r#"{"mode":"maybe"}"#).unwrap()).unwrap_err();
        assert_eq!(bad.code, INVALID_PARAMS);
        assert_eq!(bad.message, "parameter mode must be enum(open|closed)");
        assert!(i.manifest_toml().contains("params = [\"mode:enum(open|closed)\"]"));
    }

    #[test]
    fn property_path_parses_names_indexes_and_rejects_malformed() {
        let p = PropertyPath::parse("items[2].title").unwrap();
        assert_eq!(p.steps, vec![PathStep::Key("items".into()), PathStep::Index(2), PathStep::Key("title".into())]);
        assert_eq!(PropertyPath::parse("value.0").unwrap().steps, vec![PathStep::Key("value".into()), PathStep::Index(0)]);
        assert_eq!(PropertyPath::parse("grid[1][0]").unwrap().steps.len(), 3);
        assert!(PropertyPath::parse("a..b").is_none(), "empty segment");
        assert!(PropertyPath::parse("[0]").is_none(), "no name before the index");
        assert!(PropertyPath::parse("a[x]").is_none(), "index is not a number");
        assert!(PropertyPath::parse("a[1").is_none(), "unclosed index");
        assert!(PropertyPath::parse("").is_none());
    }

    #[test]
    fn property_path_resolves_into_nested_results() {
        let v = json::parse(r#"{"items":[{"title":"milk"},{"title":"eggs"}],"count":2}"#).unwrap();
        let at = |p: &str| PropertyPath::parse(p).unwrap().resolve(&v).cloned();
        assert_eq!(at("items[1].title"), Some(Json::Str("eggs".into())));
        assert_eq!(at("items.0.title"), Some(Json::Str("milk".into())), "numeric segment form");
        assert_eq!(at("count"), Some(Json::Num(2.0)));
        assert_eq!(at("items[5].title"), None, "index out of range");
        assert_eq!(at("items.title"), None, "name on an array");
        assert_eq!(at("count.x"), None, "member of a number");
    }

    #[test]
    fn result_reads_back_from_its_json_and_reads_paths() {
        let r = IntentResult::value(json::parse(r#"{"items":[{"title":"milk"}]}"#).unwrap()).with_dialog("Found 1");
        assert_eq!(r.at("items[0].title"), Some(&Json::Str("milk".into())));
        let back = IntentResult::from_json(&r.to_json()).unwrap();
        assert_eq!(back, r);
        assert_eq!(IntentResult::from_json(&json::parse(r#"{"value":1}"#).unwrap()), None, "dialog key is required");
        assert_eq!(IntentResult::from_json(&json::parse(r#"{"value":1,"dialog":3}"#).unwrap()), None, "dialog is text or null");
    }

    #[test]
    fn run_performs_by_id_and_checks_parameters_first() {
        let d = std::path::Path::new("/tmp");
        let mut app = Demo;
        let ok = run(&mut app, None, d, "add-note", &json::parse(r#"{"text":"milk","due":"2026-10-11"}"#).unwrap()).unwrap();
        assert_eq!(ok.value, Json::Str("milk".into()));
        assert_eq!(ok.dialog.as_deref(), Some("Added"));
        assert_eq!(run(&mut app, None, d, "add-note", &json::parse(r#"{"text":"a","due":"2026-13-01"}"#).unwrap()).unwrap_err().code, INVALID_PARAMS, "bad date is refused before perform");
        assert_eq!(run(&mut app, None, d, "add-note", &Json::Null).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(run(&mut app, None, d, "missing", &Json::Null).unwrap_err().code, METHOD_NOT_FOUND);
    }

    #[test]
    fn intents_run_in_background_unless_they_declare_foreground() {
        let bg = Intent::new("ping", "Ping", "");
        assert_eq!(bg.modes, vec![IntentMode::Background], "background is the default");
        let fg = bg.clone().foreground().foreground();
        assert_eq!(fg.modes, vec![IntentMode::Background, IntentMode::Foreground], "foreground is added once");
        assert!(fg.supports(IntentMode::Foreground) && !bg.supports(IntentMode::Foreground));
        let j = fg.to_json();
        assert_eq!(j.get("modes").and_then(Json::as_array).map(|m| m.len()), Some(2));
        assert_eq!(j.get("authentication").and_then(Json::as_str), Some("always-allowed"));
    }

    #[test]
    fn continue_in_foreground_needs_the_foreground_mode() {
        let declared = vec![Intent::new("open", "Open", ""), Intent::new("ping", "Ping", "")];
        let result = IntentResult::value(Json::Null).continue_in_foreground();
        let err = declared[0].check_result(&result, &declared).unwrap_err();
        assert_eq!(err.code, INTERNAL_ERROR);
        assert_eq!(err.message, "intent open cannot continue in the foreground");
        let open = declared[0].clone().foreground();
        assert!(open.check_result(&result, &declared).is_ok());
        assert!(declared[0].check_result(&IntentResult::value(Json::Null), &declared).is_ok(), "a plain result needs no mode");
    }

    #[test]
    fn opens_follow_on_names_a_declared_intent_with_valid_parameters() {
        let declared = vec![Intent::new("start", "Start", "").shortcut(), add()];
        let start = &declared[0];
        let good = IntentResult::dialog("Next").opens("add-note", json::parse(r#"{"text":"milk"}"#).unwrap());
        assert!(start.check_result(&good, &declared).is_ok());
        let undeclared = IntentResult::value(Json::Null).opens("nope", Json::Null);
        assert!(start.check_result(&undeclared, &declared).unwrap_err().message.contains("undeclared intent nope"));
        let bad_params = IntentResult::value(Json::Null).opens("add-note", json::parse("{}").unwrap());
        assert_eq!(start.check_result(&bad_params, &declared).unwrap_err().code, INVALID_PARAMS, "the follow-on's parameters are checked");
    }

    #[test]
    fn result_json_carries_follow_ons_only_when_set() {
        let plain = IntentResult::value(Json::Num(1.0)).with_dialog("Done");
        assert_eq!(crate::control::encode(&plain.to_json()), r#"{"value":1,"dialog":"Done"}"#, "existing shape is unchanged");
        let full = IntentResult::value(Json::Null).continue_in_foreground().opens("add-note", json::parse(r#"{"text":"a"}"#).unwrap());
        let text = crate::control::encode(&full.to_json());
        assert!(text.contains(r#""continueInForeground":true"#) && text.contains(r#""opens":{"intent":"add-note","parameters":{"text":"a"}}"#));
        assert_eq!(IntentResult::from_json(&full.to_json()).unwrap(), full);
        assert!(!IntentResult::from_json(&plain.to_json()).unwrap().continue_foreground);
    }

    #[test]
    fn authentication_policies_check_the_session() {
        let locked = Session { locked: true, authenticated: false };
        let unlocked = Session { locked: false, authenticated: false };
        let signed_in = Session { locked: false, authenticated: true };
        assert!(Authentication::AlwaysAllowed.allows(locked));
        assert!(!Authentication::RequiresLocalDeviceAuthentication.allows(locked));
        assert!(Authentication::RequiresLocalDeviceAuthentication.allows(unlocked));
        assert!(!Authentication::RequiresAuthentication.allows(unlocked));
        assert!(Authentication::RequiresAuthentication.allows(signed_in));
        let guarded = Intent::new("pay", "Pay", "").authentication(Authentication::RequiresAuthentication);
        assert!(!guarded.may_run(unlocked) && guarded.may_run(signed_in));
        assert_eq!(guarded.to_json().get("authentication").and_then(Json::as_str), Some("requires-authentication"));
        assert_eq!(Authentication::default(), Authentication::AlwaysAllowed);
    }
}
