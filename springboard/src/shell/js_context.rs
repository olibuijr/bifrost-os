//! A standalone JavaScript context for apps (`JSContext`, `JSValue`, `JSExport` parity): run script text without a
//! web view, read the result as a typed value, hand values in, expose Rust functions to scripts, and collect
//! exceptions and console output.
//!
//! One [`JsContext`] is one fresh Boa realm: no DOM, no network, no timers, no file access. A script sees the
//! language and these globals only: `console` (log, info, warn, error, debug: collected, see
//! [`JsContext::take_console`]) and the functions the host registered with [`JsContext::set_function`].
//! Evaluation runs under a loop-iteration, recursion and stack limit, so a runaway script stops with an exception
//! instead of hanging the app.
//!
//! * Values: [`JsResult`] reads a script value as undefined, null, boolean, number (including NaN and the
//!   infinities), string, bigint (as its decimal text), symbol (description), function (its source text), array or
//!   object (JSON-shaped; a cycle is an exception). [`JsContext::set_value`] writes a JSON value into a global.
//! * Functions: `set_function("name", |args| ...)` makes `name(a, b)` callable from the script. Arguments arrive as
//!   JSON values, the result goes back as JSON; an `Err` becomes a thrown `Error` in the script.
//! * Exceptions: a script error comes back as [`JsException`] (name, message, the stack text when the engine
//!   gave one) and also goes to the handler set with [`JsContext::set_exception_handler`], like
//!   `JSContext.exceptionHandler`.
//! * Promises: a script that evaluates to a promise is settled by running the pending jobs; the result is the
//!   fulfilled value or the rejection as an exception. A promise that cannot settle without a timer or I/O is
//!   reported as an exception saying so.
//!
//! Not covered: object identity across evaluations (values are copied out as JSON), `JSManagedValue` and the
//! garbage collector hooks, `JSVirtualMachine` sharing between contexts (each context owns its realm), and
//! exporting Rust types as classes with methods (functions only).

use bifrost_core::json::{self, Json};
use boa_engine::{Context, JsResult as BoaResult, JsString, JsValue, NativeFunction, Source};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Loop iterations of any one loop before the script is stopped.
const LOOP_LIMIT: u64 = 5_000_000;
/// Call depth before the script is stopped.
const RECURSION_LIMIT: usize = 400;
/// Largest script text accepted, bytes.
const MAX_SOURCE: usize = 4 * 1024 * 1024;
/// Console lines kept between calls to `take_console`.
const CONSOLE_CAP: usize = 1000;

/// One script value read out of the context (`JSValue`).
#[derive(Clone, Debug, PartialEq)]
pub enum JsOut {
    Undefined,
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    /// A bigint, as decimal text.
    BigInt(String),
    /// A symbol, as its description.
    Symbol(String),
    /// A function, as its source text.
    Function(String),
    Array(Vec<JsOut>),
    /// Fields in the order the script wrote them.
    Object(Vec<(String, JsOut)>),
}

impl JsOut {
    /// The `typeof`-style name (`null` for null, `array` for arrays).
    pub fn type_name(&self) -> &'static str {
        match self {
            JsOut::Undefined => "undefined",
            JsOut::Null => "null",
            JsOut::Bool(_) => "boolean",
            JsOut::Number(_) => "number",
            JsOut::String(_) => "string",
            JsOut::BigInt(_) => "bigint",
            JsOut::Symbol(_) => "symbol",
            JsOut::Function(_) => "function",
            JsOut::Array(_) => "array",
            JsOut::Object(_) => "object",
        }
    }

    /// `toBool`: the script's truthiness.
    pub fn to_bool(&self) -> bool {
        match self {
            JsOut::Undefined | JsOut::Null => false,
            JsOut::Bool(b) => *b,
            JsOut::Number(n) => *n != 0.0 && !n.is_nan(),
            JsOut::String(s) => !s.is_empty(),
            JsOut::BigInt(s) => s != "0",
            _ => true,
        }
    }

    /// The number, when this is one.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JsOut::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsOut::String(s) => Some(s),
            _ => None,
        }
    }

    /// The field of an object called `key`.
    pub fn get(&self, key: &str) -> Option<&JsOut> {
        match self {
            JsOut::Object(f) => f.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The element of an array.
    pub fn at(&self, i: usize) -> Option<&JsOut> {
        match self {
            JsOut::Array(a) => a.get(i),
            _ => None,
        }
    }

    /// The value as JSON where it has a JSON form (functions, symbols and undefined do not, NaN and infinities
    /// become null as `JSON.stringify` does).
    pub fn to_json(&self) -> Option<Json> {
        Some(match self {
            JsOut::Null => Json::Null,
            JsOut::Bool(b) => Json::Bool(*b),
            JsOut::Number(n) => {
                if n.is_finite() {
                    Json::Num(*n)
                } else {
                    Json::Null
                }
            }
            JsOut::String(s) => Json::Str(s.clone()),
            JsOut::Array(a) => Json::Arr(a.iter().map(|v| v.to_json().unwrap_or(Json::Null)).collect()),
            JsOut::Object(f) => Json::Obj(f.iter().filter_map(|(k, v)| v.to_json().map(|j| (k.clone(), j))).collect()),
            JsOut::Undefined | JsOut::BigInt(_) | JsOut::Symbol(_) | JsOut::Function(_) => return None,
        })
    }
}

/// An uncaught exception (`JSValue` of an `Error`, as `JSContext.exception` gives it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsException {
    /// The error's `name` (`TypeError`, `SyntaxError`, ...), or `Error`.
    pub name: String,
    pub message: String,
    /// The engine's stack text, empty when it gave none.
    pub stack: String,
}

impl std::fmt::Display for JsException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.name, self.message)
    }
}

/// A Rust function a script can call: its JSON arguments in, a JSON value or an error message out.
pub type HostFunction = Box<dyn FnMut(&[Json]) -> Result<Json, String>>;

struct Env {
    functions: RefCell<HashMap<String, HostFunction>>,
    console: RefCell<Vec<(String, String)>>,
    source: RefCell<String>,
}

fn js_string(s: &str) -> JsValue {
    JsValue::from(JsString::from(s))
}

fn arg(args: &[JsValue], i: usize, ctx: &mut Context) -> BoaResult<String> {
    match args.get(i) {
        Some(v) => Ok(v.to_string(ctx)?.to_std_string_escaped()),
        None => Ok(String::new()),
    }
}

fn env(ctx: &Context) -> Rc<Env> {
    ctx.get_data::<Rc<Env>>().cloned().expect("the context environment is installed")
}

/// `__bf_call(name, argsJson)`: runs the host function and answers `{"ok":true,"result":...}` or `{"ok":false,"message":...}`.
fn native_call(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> BoaResult<JsValue> {
    let name = arg(args, 0, ctx)?;
    let raw = arg(args, 1, ctx)?;
    let e = env(ctx);
    let reply = match json::parse(&raw) {
        Err(m) => Err(format!("unreadable arguments: {m}")),
        Ok(Json::Arr(list)) => {
            // the function is taken out while it runs, so it cannot be re-entered through the script
            let taken = e.functions.borrow_mut().remove(&name);
            match taken {
                None => Err(format!("{name} is not available")),
                Some(mut f) => {
                    let r = f(&list);
                    e.functions.borrow_mut().insert(name, f);
                    r
                }
            }
        }
        Ok(_) => Err("arguments must be an array".into()),
    };
    let doc = match reply {
        Ok(v) => Json::Obj(vec![("ok".into(), Json::Bool(true)), ("result".into(), v)]),
        Err(m) => Json::Obj(vec![("ok".into(), Json::Bool(false)), ("message".into(), Json::Str(m))]),
    };
    Ok(js_string(&doc.encode()))
}

fn native_log(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> BoaResult<JsValue> {
    let level = arg(args, 0, ctx)?;
    let text = arg(args, 1, ctx)?;
    let e = env(ctx);
    let mut c = e.console.borrow_mut();
    if c.len() < CONSOLE_CAP {
        c.push((level, text));
    }
    Ok(JsValue::undefined())
}

fn native_source(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> BoaResult<JsValue> {
    Ok(js_string(&env(ctx).source.borrow()))
}

/// The JavaScript half. `__bfRun` evaluates the pending source at global scope and answers one JSON document:
/// `{"ok":true,"t":TYPE,"v":VALUE}` (VALUE as `encode` builds it), `{"ok":true,"pending":true}` for an unsettled
/// promise, or `{"ok":false,"name":..,"message":..,"stack":..}`. `__bfSettled` reads a promise's outcome after
/// the jobs ran. `__bfDefine(name)` makes the global function that calls the host.
const PRELUDE: &str = r#"(function(){'use strict';
const H={};
for(const k of Object.getOwnPropertyNames(globalThis)){if(k.startsWith('__bf_')){H[k.slice(5)]=globalThis[k];delete globalThis[k];}}
function fmt(a){
  if(typeof a==='string')return a;
  try{if(a instanceof Error)return String(a.name)+': '+String(a.message);}catch(e){}
  try{const j=JSON.stringify(a);if(j!==undefined)return j;}catch(e){}
  try{return String(a);}catch(e){return '[unprintable]';}
}
function logger(level){return function(...a){H.log(level,a.map(fmt).join(' '));};}
globalThis.console={log:logger('log'),info:logger('info'),warn:logger('warn'),error:logger('error'),debug:logger('debug')};
const seen=new WeakSet();
function enc(v,depth){
  if(depth>64)throw new TypeError('the value is nested too deeply');
  const t=typeof v;
  if(v===null)return {t:'null'};
  if(t==='undefined')return {t:'undefined'};
  if(t==='boolean')return {t:'boolean',v:v};
  if(t==='number')return {t:'number',v:Number.isFinite(v)?v:null,s:String(v)};
  if(t==='string')return {t:'string',v:v};
  if(t==='bigint')return {t:'bigint',v:String(v)};
  if(t==='symbol')return {t:'symbol',v:String(v.description===undefined?'':v.description)};
  if(t==='function')return {t:'function',v:Function.prototype.toString.call(v)};
  if(seen.has(v))throw new TypeError('the value contains a cycle');
  seen.add(v);
  try{
    if(Array.isArray(v)){const out=[];for(let i=0;i<v.length;i++)out.push(enc(v[i],depth+1));return {t:'array',v:out};}
    if(v instanceof Error)return {t:'object',v:[['name',{t:'string',v:String(v.name)}],['message',{t:'string',v:String(v.message)}]]};
    if(typeof v.toJSON==='function'){const j=v.toJSON();if(j!==v)return enc(j,depth+1);}
    const out=[];for(const k of Object.keys(v)){out.push([k,enc(v[k],depth+1)]);}
    return {t:'object',v:out};
  }finally{seen.delete(v);}
}
function fail(e){
  const isObj=e!==null&&(typeof e==='object'||typeof e==='function');
  return JSON.stringify({ok:false,name:isObj&&e.name!==undefined?String(e.name):'Error',message:isObj&&e.message!==undefined?String(e.message):String(e),stack:isObj&&typeof e.stack==='string'?e.stack:''});
}
let settled=null;
globalThis.__bfRun=function(){
  settled=null;
  try{
    const v=(0,eval)(H.source());
    if(v!==null&&(typeof v==='object'||typeof v==='function')&&typeof v.then==='function'){
      v.then(function(r){try{settled=JSON.stringify({ok:true,r:enc(r,0)});}catch(e){settled=fail(e);}},function(e){settled=fail(e);});
      return '{"ok":true,"pending":true}';
    }
    return JSON.stringify({ok:true,r:enc(v,0)});
  }catch(e){return fail(e);}
};
globalThis.__bfSettled=function(){const s=settled;settled=null;return s===null?'{"ok":true,"pending":true}':s;};
globalThis.__bfDefine=function(name){
  Object.defineProperty(globalThis,name,{configurable:true,writable:true,enumerable:false,value:function(...args){
    const r=JSON.parse(H.call(name,JSON.stringify(args)));
    if(!r.ok)throw new Error(r.message);
    return r.result;
  }});
};
for(const n of ['__bfRun','__bfSettled','__bfDefine'])Object.defineProperty(globalThis,n,{enumerable:false});
})();"#;

/// A JavaScript context (`JSContext`).
pub struct JsContext {
    ctx: Context,
    env: Rc<Env>,
    handler: Option<Box<dyn FnMut(&JsException)>>,
    last_exception: Option<JsException>,
}

impl Default for JsContext {
    fn default() -> Self {
        Self::new()
    }
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        && !name.starts_with("__bf")
        && !RESERVED.contains(&name)
}

const RESERVED: [&str; 24] = [
    "undefined", "NaN", "Infinity", "globalThis", "eval", "Object", "Function", "Array", "String", "Number", "Boolean", "Symbol", "Error", "Promise", "JSON",
    "Math", "console", "Proxy", "Reflect", "Date", "RegExp", "Map", "Set", "WeakSet",
];

fn read(t: &Json) -> Result<JsOut, String> {
    let ty = t.str_field("t").ok_or("a value has no type")?;
    let v = t.get("v");
    Ok(match ty.as_str() {
        "undefined" => JsOut::Undefined,
        "null" => JsOut::Null,
        "boolean" => JsOut::Bool(matches!(v, Some(Json::Bool(true)))),
        "number" => match v.and_then(Json::number) {
            Some(n) => JsOut::Number(n),
            None => match t.str_field("s").as_deref() {
                Some("Infinity") => JsOut::Number(f64::INFINITY),
                Some("-Infinity") => JsOut::Number(f64::NEG_INFINITY),
                _ => JsOut::Number(f64::NAN),
            },
        },
        "string" => JsOut::String(v.and_then(Json::as_str).unwrap_or("").to_string()),
        "bigint" => JsOut::BigInt(v.and_then(Json::as_str).unwrap_or("0").to_string()),
        "symbol" => JsOut::Symbol(v.and_then(Json::as_str).unwrap_or("").to_string()),
        "function" => JsOut::Function(v.and_then(Json::as_str).unwrap_or("").to_string()),
        "array" => JsOut::Array(v.and_then(Json::as_array).unwrap_or(&[]).iter().map(read).collect::<Result<_, _>>()?),
        "object" => {
            let mut fields = Vec::new();
            for pair in v.and_then(Json::as_array).unwrap_or(&[]) {
                let kv = pair.as_array().ok_or("a field is not a pair")?;
                let key = kv.first().and_then(Json::as_str).ok_or("a field has no name")?;
                fields.push((key.to_string(), read(kv.get(1).ok_or("a field has no value")?)?));
            }
            JsOut::Object(fields)
        }
        other => return Err(format!("unknown value type {other}")),
    })
}

fn exception_of(doc: &Json) -> JsException {
    JsException { name: doc.str_field("name").unwrap_or_else(|| "Error".into()), message: doc.str_field("message").unwrap_or_default(), stack: doc.str_field("stack").unwrap_or_default() }
}

impl JsContext {
    /// A fresh context with the limits and the prelude installed.
    pub fn new() -> JsContext {
        let env = Rc::new(Env { functions: RefCell::new(HashMap::new()), console: RefCell::new(Vec::new()), source: RefCell::new(String::new()) });
        let mut ctx = Context::default();
        ctx.insert_data(env.clone());
        let natives: [(&str, fn(&JsValue, &[JsValue], &mut Context) -> BoaResult<JsValue>); 3] = [("call", native_call), ("log", native_log), ("source", native_source)];
        for (name, f) in natives {
            let key = JsString::from(format!("__bf_{name}").as_str());
            ctx.register_global_callable(key, 0, NativeFunction::from_fn_ptr(f)).expect("register native");
        }
        let _ = ctx.eval(Source::from_bytes(PRELUDE));
        let rl = ctx.runtime_limits_mut();
        rl.set_loop_iteration_limit(LOOP_LIMIT);
        rl.set_recursion_limit(RECURSION_LIMIT);
        rl.set_stack_size_limit(10 * 1024);
        rl.set_backtrace_limit(10);
        JsContext { ctx, env, handler: None, last_exception: None }
    }

    /// `exceptionHandler`: called with every exception a script raises, before `evaluate_script` returns it.
    pub fn set_exception_handler(&mut self, handler: impl FnMut(&JsException) + 'static) {
        self.handler = Some(Box::new(handler));
    }

    /// `exception`: the last exception raised, until the next evaluation succeeds.
    pub fn exception(&self) -> Option<&JsException> {
        self.last_exception.as_ref()
    }

    fn raise(&mut self, e: JsException) -> JsException {
        if let Some(h) = self.handler.as_mut() {
            h(&e);
        }
        self.last_exception = Some(e.clone());
        e
    }

    /// `evaluateScript`: runs `source` at global scope and reads the value of its last expression. Definitions
    /// made with `var`, `function` and assignment to globals stay for the next call (`let` and `const` do not, as in
    /// any indirect evaluation). A promise result is settled first.
    pub fn evaluate_script(&mut self, source: &str) -> Result<JsOut, JsException> {
        if source.len() > MAX_SOURCE {
            return Err(self.raise(JsException { name: "RangeError".into(), message: "the script is larger than 4 MB".into(), stack: String::new() }));
        }
        *self.env.source.borrow_mut() = source.to_string();
        let first = self.call_text("__bfRun()");
        let doc = match first {
            Ok(d) if matches!(d.get("pending"), Some(Json::Bool(true))) => {
                let _ = self.ctx.run_jobs();
                match self.call_text("__bfSettled()") {
                    Ok(d) if matches!(d.get("pending"), Some(Json::Bool(true))) => {
                        return Err(self.raise(JsException {
                            name: "Error".into(),
                            message: "the promise did not settle: it waits for a timer or an outside event this context does not have".into(),
                            stack: String::new(),
                        }))
                    }
                    other => other,
                }
            }
            other => other,
        };
        match doc {
            Err(e) => Err(self.raise(e)),
            Ok(d) if matches!(d.get("ok"), Some(Json::Bool(true))) => {
                let out = d.get("r").ok_or_else(|| JsException { name: "Error".into(), message: "the script produced no result".into(), stack: String::new() });
                match out.and_then(|t| read(t).map_err(|m| JsException { name: "Error".into(), message: m, stack: String::new() })) {
                    Ok(v) => {
                        self.last_exception = None;
                        Ok(v)
                    }
                    Err(e) => Err(self.raise(e)),
                }
            }
            Ok(d) => Err(self.raise(exception_of(&d))),
        }
    }

    /// Runs a short fixed call and parses its JSON answer. An engine-level stop (loop or stack limit) is an exception.
    fn call_text(&mut self, call: &str) -> Result<Json, JsException> {
        let value = self.ctx.eval(Source::from_bytes(call)).map_err(|e| JsException { name: "RuntimeLimit".into(), message: e.to_string(), stack: String::new() })?;
        let text = value
            .to_string(&mut self.ctx)
            .map_err(|e| JsException { name: "Error".into(), message: e.to_string(), stack: String::new() })?
            .to_std_string_escaped();
        json::parse(&text).map_err(|m| JsException { name: "Error".into(), message: format!("unreadable script result: {m}"), stack: String::new() })
    }

    /// Writes a JSON value into the global `name` (`setObject:forKeyedSubscript:` for plain values).
    pub fn set_value(&mut self, name: &str, value: &Json) -> Result<(), String> {
        if !valid_name(name) {
            return Err(format!("{name:?} is not a usable global name"));
        }
        let code = format!("globalThis.{name}={};0", value.encode());
        self.ctx.eval(Source::from_bytes(code.as_str())).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Exposes `function` to scripts as the global `name(...)`. Arguments arrive as JSON (functions and symbols
    /// arrive as null); the return value goes back to the script as a value. An `Err` is thrown in the script as an
    /// `Error` with that message. Registering a name again replaces the function.
    pub fn set_function(&mut self, name: &str, function: impl FnMut(&[Json]) -> Result<Json, String> + 'static) -> Result<(), String> {
        if !valid_name(name) {
            return Err(format!("{name:?} is not a usable function name"));
        }
        self.env.functions.borrow_mut().insert(name.to_string(), Box::new(function));
        let code = format!("__bfDefine({})", json::quote(name));
        self.ctx.eval(Source::from_bytes(code.as_str())).map(|_| ()).map_err(|e| e.to_string())
    }

    /// The console lines scripts wrote since the last call, as `(level, text)`.
    pub fn take_console(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.env.console.borrow_mut())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(src: &str) -> Result<JsOut, JsException> {
        JsContext::new().evaluate_script(src)
    }

    #[test]
    fn scripts_return_typed_values() {
        assert_eq!(eval("1 + 2").unwrap(), JsOut::Number(3.0));
        assert_eq!(eval("'a' + 'b'").unwrap(), JsOut::String("ab".into()));
        assert_eq!(eval("1 < 2").unwrap(), JsOut::Bool(true));
        assert_eq!(eval("null").unwrap(), JsOut::Null);
        assert_eq!(eval("undefined").unwrap(), JsOut::Undefined);
        assert_eq!(eval("123456789012345678901234567890n").unwrap(), JsOut::BigInt("123456789012345678901234567890".into()));
        assert_eq!(eval("Symbol('tag')").unwrap(), JsOut::Symbol("tag".into()));
        match eval("0 / 0").unwrap() {
            JsOut::Number(n) => assert!(n.is_nan()),
            other => panic!("{other:?}"),
        }
        assert_eq!(eval("1 / 0").unwrap(), JsOut::Number(f64::INFINITY));
        assert_eq!(eval("-1 / 0").unwrap(), JsOut::Number(f64::NEG_INFINITY));
        let f = eval("(function add(a, b) { return a + b; })").unwrap();
        assert!(matches!(&f, JsOut::Function(src) if src.contains("return a + b")), "{f:?}");
    }

    #[test]
    fn arrays_and_objects_keep_their_shape_and_order() {
        let v = eval("({z: 1, a: [1, 'two', null, {k: true}], m: {n: 2.5}})").unwrap();
        assert_eq!(v.type_name(), "object");
        assert_eq!(v.get("z"), Some(&JsOut::Number(1.0)));
        assert_eq!(v.get("a").and_then(|a| a.at(1)), Some(&JsOut::String("two".into())));
        assert_eq!(v.get("a").and_then(|a| a.at(3)).and_then(|o| o.get("k")), Some(&JsOut::Bool(true)));
        assert_eq!(v.get("m").and_then(|m| m.get("n")).and_then(JsOut::as_f64), Some(2.5));
        match &v {
            JsOut::Object(fields) => assert_eq!(fields.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), vec!["z", "a", "m"]),
            _ => unreachable!(),
        }
        let json = v.to_json().unwrap().encode();
        assert!(json.starts_with("{\"z\":1,\"a\":[1,\"two\",null,{\"k\":true}]"), "{json}");
        assert_eq!(eval("[1, 2, 3].map(x => x * 2)").unwrap().to_json().unwrap().encode(), "[2,4,6]");
    }

    #[test]
    fn a_cycle_is_an_exception_not_a_hang() {
        let e = eval("var a = {}; a.self = a; a").unwrap_err();
        assert!(e.message.contains("cycle"), "{e}");
    }

    #[test]
    fn globals_persist_between_evaluations() {
        let mut c = JsContext::new();
        c.evaluate_script("var counter = 40; function bump(n) { counter += n; return counter; }").unwrap();
        assert_eq!(c.evaluate_script("bump(2)").unwrap(), JsOut::Number(42.0));
        assert_eq!(c.evaluate_script("counter").unwrap(), JsOut::Number(42.0));
    }

    #[test]
    fn exceptions_carry_name_and_message_and_reach_the_handler() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut c = JsContext::new();
        let sink = seen.clone();
        c.set_exception_handler(move |e| sink.borrow_mut().push(e.to_string()));
        let e = c.evaluate_script("null.x").unwrap_err();
        assert_eq!(e.name, "TypeError");
        let s = c.evaluate_script("var = ;").unwrap_err();
        assert_eq!(s.name, "SyntaxError");
        let t = c.evaluate_script("throw new RangeError('too far')").unwrap_err();
        assert_eq!((t.name.as_str(), t.message.as_str()), ("RangeError", "too far"));
        let plain = c.evaluate_script("throw 'just text'").unwrap_err();
        assert_eq!((plain.name.as_str(), plain.message.as_str()), ("Error", "just text"));
        assert_eq!(seen.borrow().len(), 4);
        assert_eq!(c.exception().map(|e| e.message.as_str()), Some("just text"));
        c.evaluate_script("1").unwrap();
        assert!(c.exception().is_none(), "a good evaluation clears it");
    }

    #[test]
    fn a_runaway_script_is_stopped() {
        let e = eval("while (true) {}").unwrap_err();
        assert!(!e.message.is_empty());
        let r = eval("function f() { return f(); } f()").unwrap_err();
        assert!(!r.message.is_empty());
    }

    #[test]
    fn host_functions_take_and_return_json_and_can_fail() {
        let mut c = JsContext::new();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let log = calls.clone();
        c.set_function("hostAdd", move |args| {
            log.borrow_mut().push(args.len());
            let sum: f64 = args.iter().filter_map(Json::number).sum();
            Ok(Json::Num(sum))
        })
        .unwrap();
        c.set_function("hostFail", |_| Err("no access".into())).unwrap();
        c.set_function("hostEcho", |args| Ok(args.first().cloned().unwrap_or(Json::Null))).unwrap();
        assert_eq!(c.evaluate_script("hostAdd(1, 2, 3.5)").unwrap(), JsOut::Number(6.5));
        assert_eq!(*calls.borrow(), vec![3]);
        assert_eq!(c.evaluate_script("hostEcho({a: [1, {b: 'x'}]}).a[1].b").unwrap(), JsOut::String("x".into()));
        let e = c.evaluate_script("hostFail()").unwrap_err();
        assert_eq!(e.message, "no access");
        assert_eq!(c.evaluate_script("try { hostFail(); 'no' } catch (e) { 'caught ' + e.message }").unwrap(), JsOut::String("caught no access".into()));
        assert!(c.set_function("1bad", |_| Ok(Json::Null)).is_err());
        assert!(c.set_function("__bfRun", |_| Ok(Json::Null)).is_err());
        assert!(c.set_function("console", |_| Ok(Json::Null)).is_err());
        // a later registration replaces the first
        c.set_function("hostAdd", |_| Ok(Json::Num(-1.0))).unwrap();
        assert_eq!(c.evaluate_script("hostAdd(1)").unwrap(), JsOut::Number(-1.0));
    }

    #[test]
    fn values_go_in_as_globals() {
        let mut c = JsContext::new();
        let cfg = json::parse(r#"{"name":"demo","sizes":[1,2,3],"on":true}"#).unwrap();
        c.set_value("config", &cfg).unwrap();
        assert_eq!(c.evaluate_script("config.sizes.length + (config.on ? 10 : 0)").unwrap(), JsOut::Number(13.0));
        assert_eq!(c.evaluate_script("config.name").unwrap(), JsOut::String("demo".into()));
        assert!(c.set_value("not valid", &Json::Null).is_err());
        assert!(c.set_value("__bfRun", &Json::Null).is_err());
    }

    #[test]
    fn console_output_is_collected_with_its_level() {
        let mut c = JsContext::new();
        c.evaluate_script("console.log('a', 1, {x: 2}); console.error(new Error('bad')); console.warn(undefined)").unwrap();
        let lines = c.take_console();
        assert_eq!(lines[0], ("log".to_string(), "a 1 {\"x\":2}".to_string()));
        assert_eq!(lines[1], ("error".to_string(), "Error: bad".to_string()));
        assert_eq!(lines[2].0, "warn");
        assert!(c.take_console().is_empty());
    }

    #[test]
    fn promises_settle_or_report_that_they_cannot() {
        let mut c = JsContext::new();
        assert_eq!(c.evaluate_script("Promise.resolve(41).then(v => v + 1)").unwrap(), JsOut::Number(42.0));
        let rejected = c.evaluate_script("Promise.reject(new TypeError('nope'))").unwrap_err();
        assert_eq!((rejected.name.as_str(), rejected.message.as_str()), ("TypeError", "nope"));
        let never = c.evaluate_script("new Promise(function () {})").unwrap_err();
        assert!(never.message.contains("did not settle"), "{never}");
    }

    #[test]
    fn there_is_no_dom_network_or_timer_in_a_context() {
        let mut c = JsContext::new();
        for name in ["window", "document", "fetch", "XMLHttpRequest", "setTimeout", "require", "process"] {
            assert_eq!(c.evaluate_script(&format!("typeof {name}")).unwrap(), JsOut::String("undefined".into()), "{name} must not exist");
        }
        // the bridge cannot be reached by name either
        assert_eq!(c.evaluate_script("typeof __bf_call").unwrap(), JsOut::String("undefined".into()));
    }

    #[test]
    fn truthiness_follows_the_script_rules() {
        assert!(!JsOut::Undefined.to_bool() && !JsOut::Null.to_bool() && !JsOut::Number(0.0).to_bool() && !JsOut::Number(f64::NAN).to_bool());
        assert!(!JsOut::String(String::new()).to_bool());
        assert!(JsOut::String("0".into()).to_bool(), "a non-empty string is true, even \"0\"");
        assert!(!JsOut::BigInt("0".into()).to_bool() && JsOut::BigInt("5".into()).to_bool());
        assert!(JsOut::Array(vec![]).to_bool() && JsOut::Number(-1.0).to_bool());
    }
}
