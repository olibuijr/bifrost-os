//! The tree-walking interpreter. It does no I/O itself: every side effect goes through a [`Host`], so the same script
//! runs under `osascript` (which sends events over the app control sockets) and under the unit tests (a fake host).
//!
//! Commands inside `tell app "Name"` resolve through that app's dictionary (`dictionary::Dictionary`) to an AppEvent:
//! the command's class and id, and a parameter record built by `dictionary::command_params`. A `get`, `set`, `make` or
//! `delete` of an object goes as the core events `getd`, `setd`, `crel` and `delt`, whose `object` parameter is the
//! specifier record `{class, index | name | all, property?}` (or `{class, whose: {property, value}}`). An error reply
//! becomes a script error with the reply's number and message.

use super::ast::{BinOp, Expr, Handler, Program, RepeatKind, Selector, Stmt};
use super::dictionary::{command_params, Command, Dictionary};
use super::parser::parse;
use crate::json::Json;
use std::cmp::Ordering;
use std::fmt;

/// How long an event waits for its reply (`AppEvent` timeout default).
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;
/// `errAECoercionFail`
pub const ERR_COERCION: i64 = -1700;
/// `errAEEventNotHandled`: the app's dictionary has no such command.
pub const ERR_NOT_HANDLED: i64 = -1708;
/// `errAENoSuchObject`: the app's dictionary has no such class or property.
pub const ERR_NO_SUCH_OBJECT: i64 = -1728;
/// `errOSASyntaxError`
pub const ERR_SYNTAX: i64 = -2740;
/// `errOSAGeneric`: any other run-time error.
pub const ERR_GENERIC: i64 = -2700;
/// `errOSAUndefinedVariable` / handler not defined.
pub const ERR_UNDEFINED: i64 = -2753;
/// Handler nesting limit.
const MAX_DEPTH: usize = 200;

/// A run-time error: the number and message a script sees in `on error`.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptError {
    pub number: i64,
    pub message: String,
}

impl ScriptError {
    pub fn new(number: i64, message: impl Into<String>) -> ScriptError {
        ScriptError { number, message: message.into() }
    }

    pub fn general(message: impl Into<String>) -> ScriptError {
        ScriptError::new(ERR_GENERIC, message)
    }
}

impl fmt::Display for ScriptError {
    /// `message (number)`, the form the script tools print.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.number)
    }
}

impl std::error::Error for ScriptError {}

/// A specifier: a reference to an object of an app (`window 1`, `note "x"`, `every note whose ...`).
#[derive(Clone, Debug, PartialEq)]
pub struct Specifier {
    pub class: String,
    pub sel: Sel,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Sel {
    Index(f64),
    Name(String),
    All,
    Whose(String, Box<Value>),
}

/// A script value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Missing,
    Bool(bool),
    Num(f64),
    Str(String),
    List(Vec<Value>),
    Record(Vec<(String, Value)>),
    Spec(Specifier),
}

/// A number as a script shows it: whole numbers without a decimal point.
pub fn num_text(n: f64) -> String {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn quote_source(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
}

fn spec_text(s: &Specifier) -> String {
    match &s.sel {
        Sel::Index(n) => format!("{} {}", s.class, num_text(*n)),
        Sel::Name(name) => format!("{} {}", s.class, quote_source(name)),
        Sel::All => format!("every {}", s.class),
        Sel::Whose(prop, value) => format!("every {} whose {prop} = {}", s.class, value.source()),
    }
}

/// The specifier record a core event carries as its `object` parameter.
pub fn spec_json(s: &Specifier) -> Json {
    let mut fields = vec![("class".to_string(), Json::Str(s.class.clone()))];
    match &s.sel {
        Sel::Index(n) => fields.push(("index".into(), Json::Num(*n))),
        Sel::Name(name) => fields.push(("name".into(), Json::Str(name.clone()))),
        Sel::All => fields.push(("all".into(), Json::Bool(true))),
        Sel::Whose(prop, value) => fields.push(("whose".into(), Json::Obj(vec![("property".into(), Json::Str(prop.clone())), ("value".into(), value.to_json())]))),
    }
    Json::Obj(fields)
}

impl Value {
    /// The value as `osascript` prints it by default.
    pub fn text(&self) -> String {
        match self {
            Value::Missing => "missing value".into(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => num_text(*n),
            Value::Str(s) => s.clone(),
            Value::List(items) => items.iter().map(Value::text).collect::<Vec<_>>().join(", "),
            Value::Record(fields) => fields.iter().map(|(k, v)| format!("{k}:{}", v.text())).collect::<Vec<_>>().join(", "),
            Value::Spec(s) => spec_text(s),
        }
    }

    /// The value as `osascript -s s` prints it: strings quoted, lists and records in braces.
    pub fn source(&self) -> String {
        match self {
            Value::Missing => "missing value".into(),
            Value::Str(s) => quote_source(s),
            Value::List(items) => format!("{{{}}}", items.iter().map(Value::source).collect::<Vec<_>>().join(", ")),
            Value::Record(fields) => format!("{{{}}}", fields.iter().map(|(k, v)| format!("{k}:{}", v.source())).collect::<Vec<_>>().join(", ")),
            other => other.text(),
        }
    }

    /// The JSON form a value takes as an event parameter or result.
    pub fn to_json(&self) -> Json {
        match self {
            Value::Missing => Json::Null,
            Value::Bool(b) => Json::Bool(*b),
            Value::Num(n) => Json::Num(*n),
            Value::Str(s) => Json::Str(s.clone()),
            Value::List(items) => Json::Arr(items.iter().map(Value::to_json).collect()),
            Value::Record(fields) => Json::Obj(fields.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()),
            Value::Spec(s) => spec_json(s),
        }
    }

    /// Reads a reply's result back as a value (`null` is missing).
    pub fn from_json(j: &Json) -> Value {
        match j {
            Json::Null => Value::Missing,
            Json::Bool(b) => Value::Bool(*b),
            Json::Num(n) => Value::Num(*n),
            Json::Str(s) => Value::Str(s.clone()),
            Json::Arr(items) => Value::List(items.iter().map(Value::from_json).collect()),
            Json::Obj(fields) => Value::Record(fields.iter().map(|(k, v)| (k.clone(), Value::from_json(v))).collect()),
        }
    }

    fn number(&self) -> Result<f64, ScriptError> {
        match self {
            Value::Num(n) => Ok(*n),
            Value::Str(s) => s.trim().parse::<f64>().map_err(|_| coercion(self, "number")),
            _ => Err(coercion(self, "number")),
        }
    }

    fn truth(&self) -> Result<bool, ScriptError> {
        match self {
            Value::Bool(b) => Ok(*b),
            _ => Err(coercion(self, "boolean")),
        }
    }
}

fn coercion(v: &Value, to: &str) -> ScriptError {
    ScriptError::new(ERR_COERCION, format!("Can't make {} into type {to}.", v.source()))
}

/// Everything a script does to the outside world. The `osascript` tool implements it with the app control sockets,
/// the notification and dialog services, `/bin/sh` and `thread::sleep`.
pub trait Host {
    /// Sends one event to `app` and returns the reply's result. An error reply is an `Err` with its number and message.
    fn send_event(&mut self, app: &str, class: &str, id: &str, params: &Json, timeout_secs: u64) -> Result<Json, ScriptError>;
    /// The app's scripting dictionary. An app that is not running is an error.
    fn dictionary(&mut self, app: &str) -> Result<Dictionary, ScriptError>;
    /// Shows a dialog with an OK button; returns the button the person pressed.
    fn display_dialog(&mut self, text: &str, title: Option<&str>) -> Result<String, ScriptError>;
    fn display_notification(&mut self, text: &str, title: Option<&str>) -> Result<(), ScriptError>;
    /// `do shell script`: the command's standard output; a failing command is an error.
    fn shell(&mut self, command: &str) -> Result<String, ScriptError>;
    fn sleep(&mut self, secs: f64);
}

enum Flow {
    Next,
    Exit,
    Return(Value),
}

fn exit_or(f: Flow) -> Flow {
    match f {
        Flow::Exit => Flow::Next,
        other => other,
    }
}

struct Interp<'a> {
    program: &'a Program,
    host: &'a mut dyn Host,
    globals: Vec<(String, Value)>,
    locals: Vec<(String, Value)>,
    /// The result of the last statement that produces one (`it`).
    it: Value,
    /// The app names of the enclosing `tell` blocks, innermost last.
    tells: Vec<String>,
    /// Dictionaries fetched this run, by app name.
    dicts: Vec<(String, Dictionary)>,
    depth: usize,
}

impl<'a> Interp<'a> {
    fn target(&self) -> Result<String, ScriptError> {
        self.tells.last().cloned().ok_or_else(|| ScriptError::general("No application is specified. Put the command in a tell block."))
    }

    fn dictionary(&mut self, app: &str) -> Result<Dictionary, ScriptError> {
        if let Some((_, d)) = self.dicts.iter().find(|(a, _)| a == app) {
            return Ok(d.clone());
        }
        let d = self.host.dictionary(app)?;
        self.dicts.push((app.to_string(), d.clone()));
        Ok(d)
    }

    /// A class the app's dictionary does not declare is an error, when the dictionary declares classes at all.
    fn check_class(&mut self, app: &str, class: &str, verb: &str) -> Result<(), ScriptError> {
        let d = self.dictionary(app)?;
        if !d.classes.is_empty() && d.class(class).is_none() {
            return Err(ScriptError::new(ERR_NO_SUCH_OBJECT, format!("Can't {verb} {class}.")));
        }
        Ok(())
    }

    fn core_event(&mut self, app: &str, id: &str, params: Json) -> Result<Value, ScriptError> {
        let reply = self.host.send_event(app, "core", id, &params, DEFAULT_TIMEOUT_SECS)?;
        Ok(Value::from_json(&reply))
    }

    fn send_command(&mut self, app: &str, cmd: &Command, direct: Option<Json>, with: Vec<(String, Json)>) -> Result<Value, ScriptError> {
        let params = command_params(cmd, direct, with).map_err(ScriptError::general)?;
        let reply = self.host.send_event(app, &cmd.class, &cmd.id, &params, DEFAULT_TIMEOUT_SECS)?;
        Ok(Value::from_json(&reply))
    }

    /// Resolves a specifier's property (or the object itself when `property` is `None`) through a `getd` event.
    fn get_object(&mut self, spec: &Specifier, property: Option<&str>) -> Result<Value, ScriptError> {
        let app = self.target()?;
        self.check_class(&app, &spec.class, "get")?;
        if let Some(p) = property {
            let d = self.dictionary(&app)?;
            if let Some(c) = d.class(&spec.class) {
                if !c.properties.is_empty() && !c.properties.iter().any(|x| x == p) {
                    return Err(ScriptError::new(ERR_NO_SUCH_OBJECT, format!("Can't get {p} of {}.", spec.class)));
                }
            }
        }
        let mut object = spec_json(spec);
        if let (Some(p), Json::Obj(fields)) = (property, &mut object) {
            fields.push(("property".into(), Json::Str(p.to_string())));
        }
        self.core_event(&app, "getd", Json::Obj(vec![("object".into(), object)]))
    }

    /// Sets a variable: the local one when there is one, else the global one, else a new one in the current scope.
    fn assign(&mut self, name: &str, v: Value) {
        if let Some(i) = self.locals.iter().position(|e| e.0 == name) {
            self.locals[i].1 = v;
        } else if let Some(i) = self.globals.iter().position(|e| e.0 == name) {
            self.globals[i].1 = v;
        } else if self.depth > 0 {
            self.locals.push((name.to_string(), v));
        } else {
            self.globals.push((name.to_string(), v));
        }
    }

    fn lookup(&self, name: &str) -> Result<Value, ScriptError> {
        if name == "it" {
            return Ok(self.it.clone());
        }
        self.locals
            .iter()
            .rev()
            .find(|e| e.0 == name)
            .or_else(|| self.globals.iter().find(|e| e.0 == name))
            .map(|e| e.1.clone())
            .ok_or_else(|| ScriptError::new(ERR_UNDEFINED, format!("The variable {name} is not defined.")))
    }

    fn exec_block(&mut self, stmts: &[Stmt]) -> Result<Flow, ScriptError> {
        for st in stmts {
            match self.exec(st)? {
                Flow::Next => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Next)
    }

    /// One pass of a loop body: `None` to go on, or the flow that ends the loop.
    fn body_flow(&mut self, stmts: &[Stmt]) -> Result<Option<Flow>, ScriptError> {
        match self.exec_block(stmts)? {
            Flow::Next => Ok(None),
            other => Ok(Some(other)),
        }
    }

    fn exec(&mut self, st: &Stmt) -> Result<Flow, ScriptError> {
        match st {
            Stmt::Set { name, value } => {
                let v = self.eval(value)?;
                self.assign(name, v);
            }
            Stmt::SetProp { prop, target, value } => {
                let v = self.eval(value)?;
                match self.eval(target)? {
                    Value::Spec(spec) => {
                        let app = self.target()?;
                        self.check_class(&app, &spec.class, "set")?;
                        let mut object = spec_json(&spec);
                        if let Json::Obj(fields) = &mut object {
                            fields.push(("property".into(), Json::Str(prop.clone())));
                        }
                        let params = Json::Obj(vec![("object".into(), object), ("to".into(), v.to_json())]);
                        self.core_event(&app, "setd", params)?;
                    }
                    _ => return Err(ScriptError::general(format!("Can't set {prop} of this value."))),
                }
            }
            Stmt::Get(e) => {
                self.it = match self.eval(e)? {
                    Value::Spec(spec) => self.get_object(&spec, None)?,
                    other => other,
                };
            }
            Stmt::Make { class, props } => {
                let app = self.target()?;
                let dict = self.dictionary(&app)?;
                let props_value = match props {
                    Some(e) => Some(self.eval(e)?),
                    None => None,
                };
                self.it = match dict.command(&format!("make new {class}")) {
                    Some(cmd) => {
                        let cmd = cmd.clone();
                        let with: Vec<(String, Json)> = match props_value {
                            Some(Value::Record(fields)) => fields.iter().map(|(k, v)| (k.clone(), v.to_json())).collect(),
                            None => Vec::new(),
                            Some(_) => return Err(ScriptError::general("make needs a record of properties.")),
                        };
                        self.send_command(&app, &cmd, None, with)?
                    }
                    None => {
                        self.check_class(&app, class, "make")?;
                        let props_json = props_value.map(|v| v.to_json()).unwrap_or(Json::Null);
                        let params = Json::Obj(vec![("class".into(), Json::Str(class.clone())), ("properties".into(), props_json)]);
                        self.core_event(&app, "crel", params)?
                    }
                };
            }
            Stmt::Delete(e) => {
                let Value::Spec(spec) = self.eval(e)? else {
                    return Err(ScriptError::general("delete needs an object."));
                };
                let app = self.target()?;
                self.check_class(&app, &spec.class, "delete")?;
                self.core_event(&app, "delt", Json::Obj(vec![("object".into(), spec_json(&spec))]))?;
            }
            Stmt::Display { dialog, text, title } => {
                let text = self.eval(text)?.text();
                let title = match title {
                    Some(e) => Some(self.eval(e)?.text()),
                    None => None,
                };
                if *dialog {
                    let button = self.host.display_dialog(&text, title.as_deref())?;
                    self.it = Value::Record(vec![("button returned".into(), Value::Str(button))]);
                } else {
                    self.host.display_notification(&text, title.as_deref())?;
                }
            }
            Stmt::Command { phrase, direct, with } => {
                let app = self.target()?;
                let dict = self.dictionary(&app)?;
                let cmd = dict
                    .command(phrase)
                    .cloned()
                    .ok_or_else(|| ScriptError::new(ERR_NOT_HANDLED, format!("{app} doesn't understand the \"{phrase}\" message.")))?;
                let direct_json = match direct {
                    Some(e) => Some(self.eval(e)?.to_json()),
                    None => None,
                };
                let mut with_json = Vec::new();
                for (label, e) in with {
                    with_json.push((label.clone(), self.eval(e)?.to_json()));
                }
                self.it = self.send_command(&app, &cmd, direct_json, with_json)?;
            }
            Stmt::Tell { app, body } => {
                let name = self.eval(app)?.text();
                self.tells.push(name);
                let r = self.exec_block(body);
                self.tells.pop();
                return r;
            }
            Stmt::If { cond, then_body, else_body } => {
                return if self.eval(cond)?.truth()? { self.exec_block(then_body) } else { self.exec_block(else_body) };
            }
            Stmt::Repeat { kind, body } => return self.repeat(kind, body),
            Stmt::Exit => return Ok(Flow::Exit),
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(e)?,
                    None => Value::Missing,
                };
                return Ok(Flow::Return(v));
            }
            Stmt::Try { body, error_var, handler } => {
                return match self.exec_block(body) {
                    Ok(flow) => Ok(flow),
                    Err(err) => match handler {
                        None => Err(err),
                        Some(h) => {
                            if let Some(var) = error_var {
                                let info = vec![("message".to_string(), Value::Str(err.message.clone())), ("number".to_string(), Value::Num(err.number as f64))];
                                self.assign(var, Value::Record(info));
                            }
                            self.exec_block(h)
                        }
                    },
                };
            }
            Stmt::Delay(e) => {
                let secs = self.eval(e)?.number()?;
                self.host.sleep(secs.max(0.0));
            }
            Stmt::Expr(e) => self.it = self.eval(e)?,
        }
        Ok(Flow::Next)
    }

    fn repeat(&mut self, kind: &RepeatKind, body: &[Stmt]) -> Result<Flow, ScriptError> {
        match kind {
            RepeatKind::Forever => loop {
                if let Some(f) = self.body_flow(body)? {
                    return Ok(exit_or(f));
                }
            },
            RepeatKind::Times(e) => {
                let n = self.eval(e)?.number()?;
                let mut i = 0.0;
                while i < n {
                    i += 1.0;
                    if let Some(f) = self.body_flow(body)? {
                        return Ok(exit_or(f));
                    }
                }
                Ok(Flow::Next)
            }
            RepeatKind::Range { var, from, to } => {
                let a = self.eval(from)?.number()?;
                let b = self.eval(to)?.number()?;
                let mut i = a;
                while i <= b {
                    self.assign(var, Value::Num(i));
                    if let Some(f) = self.body_flow(body)? {
                        return Ok(exit_or(f));
                    }
                    i += 1.0;
                }
                Ok(Flow::Next)
            }
            RepeatKind::Each { var, list } => {
                let items = match self.eval(list)? {
                    Value::List(items) => items,
                    other => return Err(ScriptError::general(format!("Can't repeat with {}.", other.source()))),
                };
                for item in items {
                    self.assign(var, item);
                    if let Some(f) = self.body_flow(body)? {
                        return Ok(exit_or(f));
                    }
                }
                Ok(Flow::Next)
            }
        }
    }

    fn eval(&mut self, e: &Expr) -> Result<Value, ScriptError> {
        Ok(match e {
            Expr::Num(n) => Value::Num(*n),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Missing => Value::Missing,
            Expr::Var(name) => self.lookup(name)?,
            Expr::List(items) => {
                let mut out = Vec::new();
                for x in items {
                    out.push(self.eval(x)?);
                }
                Value::List(out)
            }
            Expr::Record(fields) => {
                let mut out = Vec::new();
                for (k, v) in fields {
                    out.push((k.clone(), self.eval(v)?));
                }
                Value::Record(out)
            }
            Expr::Not(a) => Value::Bool(!self.eval(a)?.truth()?),
            Expr::Neg(a) => Value::Num(-self.eval(a)?.number()?),
            Expr::Bin(BinOp::And, a, b) => {
                if !self.eval(a)?.truth()? {
                    Value::Bool(false)
                } else {
                    Value::Bool(self.eval(b)?.truth()?)
                }
            }
            Expr::Bin(BinOp::Or, a, b) => {
                if self.eval(a)?.truth()? {
                    Value::Bool(true)
                } else {
                    Value::Bool(self.eval(b)?.truth()?)
                }
            }
            Expr::Bin(op, a, b) => {
                let x = self.eval(a)?;
                let y = self.eval(b)?;
                binary(*op, x, y)?
            }
            Expr::Call { name, args } => {
                let mut vals = Vec::new();
                for a in args {
                    vals.push(self.eval(a)?);
                }
                self.call_named(name, vals)?
            }
            Expr::Shell(cmd) => {
                let line = self.eval(cmd)?.text();
                let out = self.host.shell(&line)?;
                Value::Str(out.trim_end_matches('\n').to_string())
            }
            Expr::Prop { prop, of } => {
                let target = self.eval(of)?;
                self.property(target, prop)?
            }
            Expr::Item { index, of } => {
                let i = self.eval(index)?.number()?;
                match self.eval(of)? {
                    Value::List(items) => {
                        let k = i as i64;
                        if k < 1 || k as usize > items.len() {
                            return Err(ScriptError::new(ERR_NO_SUCH_OBJECT, format!("Can't get item {}.", num_text(i))));
                        }
                        items[(k - 1) as usize].clone()
                    }
                    other => return Err(ScriptError::general(format!("Can't get item of {}.", other.source()))),
                }
            }
            Expr::Spec { class, sel } => {
                let sel = match sel {
                    Selector::Index(x) => Sel::Index(self.eval(x)?.number()?),
                    Selector::Name(x) => Sel::Name(self.eval(x)?.text()),
                    Selector::All => Sel::All,
                    Selector::Whose { prop, value } => Sel::Whose(prop.clone(), Box::new(self.eval(value)?)),
                };
                Value::Spec(Specifier { class: class.clone(), sel })
            }
        })
    }

    fn property(&mut self, target: Value, prop: &str) -> Result<Value, ScriptError> {
        match target {
            Value::Spec(spec) => self.get_object(&spec, Some(prop)),
            Value::Record(fields) => fields
                .into_iter()
                .find(|(k, _)| k == prop)
                .map(|(_, v)| v)
                .ok_or_else(|| ScriptError::new(ERR_NO_SUCH_OBJECT, format!("Can't get {prop} of this record."))),
            Value::List(items) if prop == "length" || prop == "count" => Ok(Value::Num(items.len() as f64)),
            Value::Str(s) if prop == "length" || prop == "count" => Ok(Value::Num(s.chars().count() as f64)),
            other => Err(ScriptError::general(format!("Can't get {prop} of {}.", other.source()))),
        }
    }

    fn call_named(&mut self, name: &str, args: Vec<Value>) -> Result<Value, ScriptError> {
        let program: &'a Program = self.program;
        match program.handlers.iter().find(|h| h.name == name) {
            Some(h) => self.call(h, args),
            None => Err(ScriptError::new(ERR_UNDEFINED, format!("The handler {name} is not defined."))),
        }
    }

    fn call(&mut self, h: &Handler, args: Vec<Value>) -> Result<Value, ScriptError> {
        if args.len() != h.params.len() {
            return Err(ScriptError::general(format!("Handler {} expects {} arguments but got {}.", h.name, h.params.len(), args.len())));
        }
        if self.depth >= MAX_DEPTH {
            return Err(ScriptError::general("Handlers call each other too deeply."));
        }
        let saved = std::mem::take(&mut self.locals);
        self.locals = h.params.iter().cloned().zip(args).collect();
        self.depth += 1;
        let r = self.exec_block(&h.body);
        self.depth -= 1;
        self.locals = saved;
        match r? {
            Flow::Return(v) => Ok(v),
            _ => Ok(Value::Missing),
        }
    }
}

fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Missing, Value::Missing) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x.eq_ignore_ascii_case(y),
        (Value::List(x), Value::List(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| equal(p, q)),
        (Value::Record(x), Value::Record(y)) => x.len() == y.len() && x.iter().zip(y).all(|((k1, v1), (k2, v2))| k1 == k2 && equal(v1, v2)),
        _ => false,
    }
}

fn compare(a: &Value, b: &Value) -> Result<Ordering, ScriptError> {
    match (a, b) {
        (Value::Num(x), Value::Num(y)) => x.partial_cmp(y).ok_or_else(|| ScriptError::general("Can't compare these numbers.")),
        (Value::Str(x), Value::Str(y)) => Ok(x.to_lowercase().cmp(&y.to_lowercase())),
        _ => Err(ScriptError::general(format!("Can't compare {} and {}.", a.source(), b.source()))),
    }
}

fn binary(op: BinOp, a: Value, b: Value) -> Result<Value, ScriptError> {
    Ok(match op {
        BinOp::Concat => match (a, b) {
            (Value::List(mut x), Value::List(y)) => {
                x.extend(y);
                Value::List(x)
            }
            (a, b) => Value::Str(format!("{}{}", a.text(), b.text())),
        },
        BinOp::Eq => Value::Bool(equal(&a, &b)),
        BinOp::Ne => Value::Bool(!equal(&a, &b)),
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
            let ord = compare(&a, &b)?;
            Value::Bool(match op {
                BinOp::Lt => ord == Ordering::Less,
                BinOp::Le => ord != Ordering::Greater,
                BinOp::Gt => ord == Ordering::Greater,
                _ => ord != Ordering::Less,
            })
        }
        BinOp::And => Value::Bool(a.truth()? && b.truth()?),
        BinOp::Or => Value::Bool(a.truth()? || b.truth()?),
        BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => {
            let x = a.number()?;
            let y = b.number()?;
            if matches!(op, BinOp::Div | BinOp::Mod) && y == 0.0 {
                return Err(ScriptError::general("Can't divide by zero."));
            }
            Value::Num(match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                BinOp::Div => x / y,
                _ => x % y,
            })
        }
    })
}

/// Runs a parsed program. The top-level statements run first; then, when the script has `on run`, that handler runs
/// with `argv` as its one list argument (none when it declares no parameter). The result is the value of the last
/// `return` or of the last statement that produces one.
pub fn run(program: &Program, host: &mut dyn Host, argv: &[String]) -> Result<Value, ScriptError> {
    let mut interp = Interp { program, host, globals: Vec::new(), locals: Vec::new(), it: Value::Missing, tells: Vec::new(), dicts: Vec::new(), depth: 0 };
    let top = interp.exec_block(&program.body)?;
    let mut result = match top {
        Flow::Return(v) => v,
        _ => interp.it.clone(),
    };
    if let Some(h) = program.handlers.iter().find(|h| h.name == "run") {
        let args = if h.params.is_empty() { Vec::new() } else { vec![Value::List(argv.iter().map(|a| Value::Str(a.clone())).collect())] };
        result = interp.call(h, args)?;
    }
    Ok(result)
}

/// Parses and runs a script's source text. A syntax error is an `ERR_SYNTAX` script error.
pub fn run_source(src: &str, host: &mut dyn Host, argv: &[String]) -> Result<Value, ScriptError> {
    let program = parse(src).map_err(|e| ScriptError::new(ERR_SYNTAX, e.to_string()))?;
    run(&program, host, argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::script::dictionary::Class;

    #[derive(Default)]
    struct Fake {
        sent: Vec<(String, String, String, Json)>,
        replies: Vec<Result<Json, ScriptError>>,
        dialogs: Vec<String>,
        notes: Vec<String>,
        shell_lines: Vec<String>,
        slept: f64,
    }

    impl Host for Fake {
        fn send_event(&mut self, app: &str, class: &str, id: &str, params: &Json, timeout_secs: u64) -> Result<Json, ScriptError> {
            assert_eq!(timeout_secs, DEFAULT_TIMEOUT_SECS);
            self.sent.push((app.into(), class.into(), id.into(), params.clone()));
            if self.replies.is_empty() {
                Ok(Json::Null)
            } else {
                self.replies.remove(0)
            }
        }
        fn dictionary(&mut self, app: &str) -> Result<Dictionary, ScriptError> {
            if app == "Notes" {
                Ok(notes_dictionary())
            } else {
                Err(ScriptError::new(-600, format!("{app} isn't running.")))
            }
        }
        fn display_dialog(&mut self, text: &str, _title: Option<&str>) -> Result<String, ScriptError> {
            self.dialogs.push(text.into());
            Ok("OK".into())
        }
        fn display_notification(&mut self, text: &str, _title: Option<&str>) -> Result<(), ScriptError> {
            self.notes.push(text.into());
            Ok(())
        }
        fn shell(&mut self, command: &str) -> Result<String, ScriptError> {
            self.shell_lines.push(command.into());
            Ok(format!("ran {command}\n"))
        }
        fn sleep(&mut self, secs: f64) {
            self.slept += secs;
        }
    }

    fn notes_dictionary() -> Dictionary {
        Dictionary {
            commands: vec![
                Command { name: "make new note".into(), class: "notes".into(), id: "make".into(), params: vec!["title".into(), "body".into()], result: "note".into() },
                Command { name: "count notes".into(), class: "notes".into(), id: "count".into(), params: vec![], result: String::new() },
            ],
            classes: vec![Class { name: "note".into(), properties: vec!["name".into(), "body".into()], elements: vec![] }],
        }
    }

    fn exec(src: &str, host: &mut Fake) -> Result<Value, ScriptError> {
        run_source(src, host, &[])
    }

    #[test]
    fn arithmetic_strings_and_precedence() {
        let mut h = Fake::default();
        assert_eq!(exec("return 2 + 3 * 4", &mut h), Ok(Value::Num(14.0)));
        assert_eq!(exec("return \"a\" & 14 & \"b\"", &mut h), Ok(Value::Str("a14b".into())));
        assert_eq!(exec("return 7 mod 4 = 3 and not false", &mut h), Ok(Value::Bool(true)));
        assert_eq!(exec("return \"ABC\" = \"abc\"", &mut h), Ok(Value::Bool(true)), "string comparison ignores case");
        assert!(exec("return 1 / 0", &mut h).is_err());
    }

    #[test]
    fn variables_and_loops() {
        let mut h = Fake::default();
        let total = exec("set total to 0\nrepeat with i from 1 to 4\nset total to total + i\nend repeat\nreturn total", &mut h);
        assert_eq!(total, Ok(Value::Num(10.0)));
        let times = exec("set n to 0\nrepeat 3 times\nset n to n + 1\nend repeat\nreturn n", &mut h);
        assert_eq!(times, Ok(Value::Num(3.0)));
        let each = exec("set out to \"\"\nrepeat with x in {\"a\", \"b\"}\nset out to out & x\nend repeat\nreturn out", &mut h);
        assert_eq!(each, Ok(Value::Str("ab".into())));
        let stopped = exec("set n to 0\nrepeat\nset n to n + 1\nif n = 5 then exit repeat\nend repeat\nreturn n", &mut h);
        assert_eq!(stopped, Ok(Value::Num(5.0)));
    }

    #[test]
    fn if_else_branches() {
        let mut h = Fake::default();
        let src = "set x to 2\nif x > 5 then\nreturn \"big\"\nelse if x > 1 then\nreturn \"mid\"\nelse\nreturn \"small\"\nend if";
        assert_eq!(exec(src, &mut h), Ok(Value::Str("mid".into())));
        assert_eq!(exec("if 1 > 2 then return 1 else return 2", &mut h), Ok(Value::Num(2.0)));
    }

    #[test]
    fn handlers_and_argv() {
        let mut h = Fake::default();
        assert_eq!(exec("on add(a, b)\nreturn a + b\nend add\nreturn add(2, 5)", &mut h), Ok(Value::Num(7.0)));
        let argv = vec!["first".to_string(), "second".to_string()];
        let r = run_source("on run argv\nreturn item 2 of argv\nend run", &mut h, &argv);
        assert_eq!(r, Ok(Value::Str("second".into())));
        assert!(exec("return nothing(1)", &mut h).is_err());
        assert!(exec("on f(a)\nreturn a\nend f\nreturn f()", &mut h).is_err(), "wrong argument count is an error");
    }

    #[test]
    fn try_catches_errors_with_number_and_message() {
        let mut h = Fake::default();
        assert_eq!(exec("try\nset z to 1 / 0\non error e\nreturn number of e\nend try", &mut h), Ok(Value::Num(ERR_GENERIC as f64)));
        let caught = exec("try\nset z to 1 / 0\non error e\nreturn message of e\nend try", &mut h);
        assert_eq!(caught, Ok(Value::Str("Can't divide by zero.".into())));
        assert!(exec("try\nset z to 1 / 0\nend try", &mut h).is_err(), "without a handler the error goes on");
    }

    #[test]
    fn tell_sends_a_dictionary_command_as_an_event() {
        let mut h = Fake::default();
        let src = "tell app \"Notes\"\nmake new note with properties {title:\"Hi\", body:\"x\"}\nend tell";
        exec(src, &mut h).unwrap();
        let (app, class, id, params) = h.sent[0].clone();
        assert_eq!((app.as_str(), class.as_str(), id.as_str()), ("Notes", "notes", "make"));
        assert_eq!(params.str_field("title").as_deref(), Some("Hi"));
        assert_eq!(params.str_field("body").as_deref(), Some("x"));
    }

    #[test]
    fn a_direct_argument_and_a_labelled_one_both_reach_the_params() {
        let mut h = Fake::default();
        exec("tell app \"Notes\" to count notes", &mut h).unwrap();
        assert_eq!(h.sent[0].2, "count");
        assert_eq!(h.sent[0].3, Json::Obj(vec![]));
    }

    #[test]
    fn an_unknown_command_is_not_handled() {
        let mut h = Fake::default();
        let e = exec("tell app \"Notes\"\nexplode now\nend tell", &mut h).unwrap_err();
        assert_eq!(e.number, ERR_NOT_HANDLED);
        assert!(e.message.contains("explode now"));
    }

    #[test]
    fn a_bad_parameter_label_is_an_error_before_any_event() {
        let mut h = Fake::default();
        let r = exec("tell app \"Notes\"\nmake new note with properties {colour:\"red\"}\nend tell", &mut h);
        assert!(r.is_err());
        assert!(h.sent.is_empty());
    }

    #[test]
    fn get_and_set_send_core_events_with_a_specifier_record() {
        let mut h = Fake::default();
        h.replies = vec![Ok(Json::Str("Shopping".into())), Ok(Json::Null)];
        let got = exec("tell app \"Notes\"\nget name of note 1\nend tell", &mut h).unwrap();
        assert_eq!(got, Value::Str("Shopping".into()));
        let (_, class, id, params) = h.sent[0].clone();
        assert_eq!((class.as_str(), id.as_str()), ("core", "getd"));
        let object = params.get("object").unwrap();
        assert_eq!(object.str_field("class").as_deref(), Some("note"));
        assert_eq!(object.str_field("property").as_deref(), Some("name"));
        exec("tell app \"Notes\"\nset name of note \"Todo\" to \"Groceries\"\nend tell", &mut h).unwrap();
        assert_eq!(h.sent[1].2, "setd");
        assert_eq!(h.sent[1].3.get("to").and_then(Json::as_str), Some("Groceries"));
    }

    #[test]
    fn an_undeclared_class_or_property_is_no_such_object() {
        let mut h = Fake::default();
        let e = exec("tell app \"Notes\"\nget colour of note 1\nend tell", &mut h).unwrap_err();
        assert_eq!(e.number, ERR_NO_SUCH_OBJECT);
        let c = exec("tell app \"Notes\"\nget name of window 1\nend tell", &mut h).unwrap_err();
        assert_eq!(c.number, ERR_NO_SUCH_OBJECT);
    }

    #[test]
    fn every_with_whose_is_a_specifier_record() {
        let mut h = Fake::default();
        exec("tell app \"Notes\"\nget every note whose name is \"x\"\nend tell", &mut h).unwrap();
        let object = h.sent[0].3.get("object").unwrap().clone();
        let whose = object.get("whose").unwrap();
        assert_eq!(whose.str_field("property").as_deref(), Some("name"));
        assert_eq!(whose.str_field("value").as_deref(), Some("x"));
    }

    #[test]
    fn an_app_that_is_not_running_fails_the_tell() {
        let mut h = Fake::default();
        let e = exec("tell app \"Mail\" to count notes", &mut h).unwrap_err();
        assert_eq!(e.number, -600);
    }

    #[test]
    fn a_reply_error_surfaces_with_its_number_and_message() {
        let mut h = Fake::default();
        h.replies = vec![Err(ScriptError::new(-1753, "no such note"))];
        let e = exec("tell app \"Notes\" to count notes", &mut h).unwrap_err();
        assert_eq!(e.to_string(), "no such note (-1753)");
    }

    #[test]
    fn display_shell_and_delay_go_to_the_host() {
        let mut h = Fake::default();
        exec("display dialog \"Hello\" with title \"T\"", &mut h).unwrap();
        exec("display notification \"Done\"", &mut h).unwrap();
        let out = exec("set o to do shell script \"echo hi\"\nreturn o", &mut h);
        assert_eq!(out, Ok(Value::Str("ran echo hi".into())));
        exec("delay 0.5", &mut h).unwrap();
        assert_eq!(h.dialogs, vec!["Hello"]);
        assert_eq!(h.notes, vec!["Done"]);
        assert_eq!(h.shell_lines, vec!["echo hi"]);
        assert_eq!(h.slept, 0.5);
    }

    #[test]
    fn a_dialog_returns_its_button() {
        let mut h = Fake::default();
        let r = exec("display dialog \"x\"\nreturn it", &mut h);
        assert_eq!(r, Ok(Value::Record(vec![("button returned".into(), Value::Str("OK".into()))])));
    }

    #[test]
    fn syntax_errors_are_reported_as_such() {
        let mut h = Fake::default();
        let e = exec("set a to )", &mut h).unwrap_err();
        assert_eq!(e.number, ERR_SYNTAX);
    }

    #[test]
    fn list_and_record_values_print_in_source_form() {
        let v = Value::Record(vec![("a".into(), Value::List(vec![Value::Num(1.0), Value::Str("x".into())]))]);
        assert_eq!(v.source(), "{a:{1, \"x\"}}");
        assert_eq!(Value::from_json(&v.to_json()), v);
    }
}
