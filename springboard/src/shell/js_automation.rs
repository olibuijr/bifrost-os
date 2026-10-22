//! JavaScript automation (`JavaScriptAppleEvents` and `JavaScriptOSA` parity): a script written in JavaScript that
//! drives running apps through the same typed events the BifrOSt script language sends (`bifrost_core::script`).
//!
//! One run = one fresh Boa context (not the browser's: no DOM, no network, no timers). The runner evaluates the
//! script's source at global scope; if the script then defines a global `run` function it is called with the argument
//! list (the `on run argv` convention), and its return value, or else the value of the script's last expression, is
//! the result. The result becomes a [`Value`] (JSON in, JSON out), so `osascript` prints it like any other result.
//!
//! The script sees these globals:
//! - `Application("Name")`: an app. Each property is a command of the app's scripting dictionary (`[[script.command]]`
//!   in its manifest), in camelCase: `Application("Notes").makeNewNote("Title", {body: "text"})` sends the
//!   `make new note` command; the first argument is the direct argument (it goes to the command's first declared
//!   parameter) and the object holds the other parameters by label. A command the dictionary does not declare, or a
//!   label it does not declare, throws.
//! - `delay(seconds)`, `displayDialog(text, {withTitle})`, `displayNotification(text, {withTitle})`, `doShellScript(command)`.
//! - `console.log` and `console.error` collect lines into [`Run::output`].
//!
//! Permission rule ([`Policy`]): the caller decides which apps a script may address and whether it may run shell
//! commands, before the script starts. The default is nothing. A command line run by the person at a terminal uses
//! [`Policy::allow_all`] (the person is the consent); a script run on an app's behalf gets only the grants the person
//! gave that app. An event to an app outside the grants is refused with error -1743 before it is sent, and so is
//! `doShellScript` without the shell grant.
//!
//! - Object specifiers: a property of `Application` that is not a command but the plural of a class the dictionary
//!   declares (`[[script.class]]`) is a collection of that class. `app.notes()` reads every note, `app.notes[0]` is the
//!   first (the index is zero-based, a negative one counts from the end), `app.notes.byName("x")` and
//!   `app.notes.whose({name: "x"})` select by name or by one property. A specifier is read by calling it
//!   (`app.notes[0]()`), a property of it by calling the property (`app.notes[0].name()`), set by assignment
//!   (`app.notes[0].name = "x"`) or `.set(value)`, and the object is removed by `.delete()`. Each one goes as the core
//!   events `getd`, `setd` and `delt`, the same records the BifrOSt script language sends, after the same grant check
//!   and the same check of the dictionary's classes and properties (`-1728` for one it does not declare).
//!
//! Not covered: elements nested in an element (`app.windows[0].tabs[1]`): the event record names one class and one
//! selector, so a specifier reaches the app's own top-level classes only. The event direction is request and reply, as
//! in the reference: an app cannot push a notification into a running script.

use bifrost_core::json::{self, Json};
use bifrost_core::script::dictionary::{command_params, normalize_name, Dictionary};
use bifrost_core::script::interp::{Host, ScriptError, Value, ERR_COERCION, ERR_GENERIC, ERR_NOT_HANDLED, ERR_NO_SUCH_OBJECT};
use boa_engine::{Context, JsResult, JsString, JsValue, NativeFunction, Source};
use std::cell::RefCell;
use std::rc::Rc;

/// `errAEEventNotPermitted`: the script's grants do not cover the request.
pub const ERR_NOT_PERMITTED: i64 = -1743;
/// Seconds an event waits for its reply.
const EVENT_TIMEOUT_SECS: u64 = 120;
/// Loop iterations of any one loop before the run is stopped.
const LOOP_LIMIT: u64 = 5_000_000;

/// Which apps a script may address and whether it may run shell commands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    all: bool,
    /// Lower-case app ids and names.
    apps: Vec<String>,
    shell: bool,
}

impl Policy {
    /// Nothing is allowed.
    pub fn none() -> Policy {
        Policy::default()
    }

    /// Every app and the shell: for a command line the person runs themselves.
    pub fn allow_all() -> Policy {
        Policy { all: true, apps: Vec::new(), shell: true }
    }

    /// Only these apps (ids or names), no shell.
    pub fn for_apps(apps: &[&str]) -> Policy {
        Policy { all: false, apps: apps.iter().map(|a| a.trim().to_lowercase()).collect(), shell: false }
    }

    /// The same policy with the shell grant set.
    pub fn with_shell(mut self, shell: bool) -> Policy {
        self.shell = shell;
        self
    }

    /// Reads a grant list: one rule per line, `allow <app id or name>`, `allow *` and `shell allow`. Blank lines and
    /// `#` comments are skipped; any other line is ignored, so a typo never widens the grants.
    pub fn from_text(text: &str) -> Policy {
        let mut p = Policy::none();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.eq_ignore_ascii_case("shell allow") {
                p.shell = true;
            } else if let Some(rest) = line.strip_prefix("allow ") {
                let app = rest.trim();
                if app == "*" {
                    p.all = true;
                } else if !app.is_empty() {
                    p.apps.push(app.to_lowercase());
                }
            }
        }
        p
    }

    /// Whether the script may address the app it spells `name`, which the host resolved to `id` when it could.
    pub fn allows(&self, name: &str, id: Option<&str>) -> bool {
        self.all || self.apps.iter().any(|a| a == &name.to_lowercase() || id.is_some_and(|i| a == &i.to_lowercase()))
    }

    /// Whether the script may run `doShellScript`.
    pub fn allows_shell(&self) -> bool {
        self.shell
    }
}

/// What a run produced: the script's result or error, and the lines it logged.
#[derive(Debug, PartialEq)]
pub struct Run {
    pub result: Result<Value, ScriptError>,
    pub output: Vec<String>,
}

/// The state the native functions reach through the context.
struct Env {
    host: RefCell<Box<dyn Host>>,
    policy: Policy,
    source: String,
    argv: Vec<String>,
    dicts: RefCell<Vec<(String, Dictionary)>>,
    log: RefCell<Vec<String>>,
}

impl Env {
    fn authorize(&self, app: &str) -> Result<(), ScriptError> {
        let id = self.host.borrow_mut().app_id(app);
        if self.policy.allows(app, id.as_deref()) {
            Ok(())
        } else {
            Err(ScriptError::new(ERR_NOT_PERMITTED, format!("Not allowed to send events to \"{app}\".")))
        }
    }

    fn dictionary(&self, app: &str) -> Result<Dictionary, ScriptError> {
        if let Some((_, d)) = self.dicts.borrow().iter().find(|(a, _)| a == app) {
            return Ok(d.clone());
        }
        let d = self.host.borrow_mut().dictionary(app)?;
        self.dicts.borrow_mut().push((app.to_string(), d.clone()));
        Ok(d)
    }

    /// One command call: `words` is the command phrase (`make new note`), `direct` the JSON of the direct argument,
    /// `with` the JSON object of the labelled parameters.
    fn send(&self, app: &str, words: &str, direct: Option<&str>, with: &str) -> Result<Json, ScriptError> {
        self.authorize(app)?;
        let dict = self.dictionary(app)?;
        let cmd = dict
            .command(words)
            .ok_or_else(|| ScriptError::new(ERR_NOT_HANDLED, format!("{app} doesn't understand the \"{}\" message.", normalize_name(words))))?
            .clone();
        let direct = match direct {
            Some(text) => Some(json::parse(text).map_err(|e| ScriptError::new(ERR_COERCION, format!("Can't read the argument: {e}")))?),
            None => None,
        };
        let labelled = match json::parse(with) {
            Ok(Json::Obj(fields)) => fields,
            Ok(Json::Null) => Vec::new(),
            Ok(_) => return Err(ScriptError::new(ERR_COERCION, "The parameters must be an object of labels.")),
            Err(e) => return Err(ScriptError::new(ERR_COERCION, format!("Can't read the parameters: {e}"))),
        };
        let params = command_params(&cmd, direct, labelled).map_err(|m| ScriptError::new(ERR_COERCION, m))?;
        self.host.borrow_mut().send_event(app, &cmd.class, &cmd.id, &params, EVENT_TIMEOUT_SECS)
    }
}

/// The class a plural property name (`notes`, `note folders`, `categories`) stands for, when the dictionary declares it.
/// A singular spelling that is itself a class name is taken as well. `None` when the dictionary declares no such class.
fn element_class(dict: &Dictionary, words: &str) -> Option<String> {
    let words = normalize_name(words);
    let mut candidates = vec![words.clone()];
    if let Some(stem) = words.strip_suffix("ies") {
        candidates.push(format!("{stem}y"));
    }
    if let Some(stem) = words.strip_suffix("es") {
        candidates.push(stem.to_string());
    }
    if let Some(stem) = words.strip_suffix('s') {
        candidates.push(stem.to_string());
    }
    candidates.into_iter().find_map(|c| dict.class(&c).map(|k| k.name.clone()))
}

impl Env {
    /// What a property of `Application(app)` means: a dictionary command, or a collection of a declared class (with the
    /// class's properties, empty when the class lists none). Anything else is taken as a command, so the error for an
    /// unknown name is the one a call gives (`-1708`).
    fn resolve(&self, app: &str, words: &str) -> Result<Json, ScriptError> {
        self.authorize(app)?;
        let dict = self.dictionary(app)?;
        let kind = |k: &str| ("kind".to_string(), Json::Str(k.into()));
        if dict.command(words).is_some() {
            return Ok(Json::Obj(vec![kind("command")]));
        }
        Ok(match element_class(&dict, words) {
            Some(class) => {
                let props: Vec<Json> = dict.class(&class).map(|c| c.properties.iter().map(|p| Json::Str(normalize_name(p))).collect()).unwrap_or_default();
                Json::Obj(vec![kind("elements"), ("class".into(), Json::Str(class)), ("properties".into(), Json::Arr(props))])
            }
            None => Json::Obj(vec![kind("command")]),
        })
    }

    /// One core event on an object: `verb` is `getd`, `setd` or `delt`, `object` the specifier record, `property` the
    /// property to read or write (empty for the object itself), `value` the JSON to set. The class and the property must
    /// be declared when the dictionary declares any, as in the BifrOSt script language.
    fn object_event(&self, app: &str, verb: &str, object: &str, property: &str, value: &str) -> Result<Json, ScriptError> {
        self.authorize(app)?;
        let dict = self.dictionary(app)?;
        let mut object = json::parse(object).map_err(|e| ScriptError::new(ERR_COERCION, format!("Can't read the object: {e}")))?;
        let class = object.str_field("class").unwrap_or_default();
        let what = match verb {
            "setd" => "set",
            "delt" => "delete",
            _ => "get",
        };
        if !dict.classes.is_empty() && dict.class(&class).is_none() {
            return Err(ScriptError::new(ERR_NO_SUCH_OBJECT, format!("Can't {what} {class}.")));
        }
        let property = normalize_name(property);
        if !property.is_empty() {
            if let Some(c) = dict.class(&class) {
                if !c.properties.is_empty() && !c.properties.iter().any(|p| normalize_name(p) == property) {
                    return Err(ScriptError::new(ERR_NO_SUCH_OBJECT, format!("Can't {what} {property} of {class}.")));
                }
            }
            if let Json::Obj(fields) = &mut object {
                fields.push(("property".into(), Json::Str(property)));
            }
        }
        let mut params = vec![("object".to_string(), object)];
        if verb == "setd" {
            params.push(("to".into(), json::parse(value).map_err(|e| ScriptError::new(ERR_COERCION, format!("Can't read the value: {e}")))?));
        }
        self.host.borrow_mut().send_event(app, "core", verb, &Json::Obj(params), EVENT_TIMEOUT_SECS)
    }
}

fn native_resolve(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let app = arg(args, 0, ctx)?;
    let words = arg(args, 1, ctx)?;
    reply(env(ctx).resolve(&app, &words))
}

fn native_object(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let (app, verb, object, property, value) = (arg(args, 0, ctx)?, arg(args, 1, ctx)?, arg(args, 2, ctx)?, arg(args, 3, ctx)?, arg(args, 4, ctx)?);
    reply(env(ctx).object_event(&app, &verb, &object, &property, &value))
}

fn js_string(s: &str) -> JsValue {
    JsValue::from(JsString::from(s))
}

fn arg(args: &[JsValue], i: usize, ctx: &mut Context) -> JsResult<String> {
    match args.get(i) {
        Some(v) => Ok(v.to_string(ctx)?.to_std_string_escaped()),
        None => Ok(String::new()),
    }
}

fn env(ctx: &Context) -> Rc<Env> {
    ctx.get_data::<Rc<Env>>().cloned().expect("the automation environment is installed")
}

/// The reply every native gives the prelude: `{"ok":true,"result":...}` or `{"ok":false,"number":..,"message":..}`.
fn reply(r: Result<Json, ScriptError>) -> JsResult<JsValue> {
    let j = match r {
        Ok(v) => Json::Obj(vec![("ok".into(), Json::Bool(true)), ("result".into(), v)]),
        Err(e) => Json::Obj(vec![("ok".into(), Json::Bool(false)), ("number".into(), Json::Num(e.number as f64)), ("message".into(), Json::Str(e.message))]),
    };
    Ok(js_string(&j.encode()))
}

fn native_send(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let app = arg(args, 0, ctx)?;
    let words = arg(args, 1, ctx)?;
    let has_direct = arg(args, 2, ctx)? == "1";
    let direct = arg(args, 3, ctx)?;
    let with = arg(args, 4, ctx)?;
    let e = env(ctx);
    reply(e.send(&app, &words, has_direct.then_some(direct.as_str()), &with))
}

fn native_dialog(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg(args, 0, ctx)?;
    let title = arg(args, 1, ctx)?;
    let e = env(ctx);
    let r = e.host.borrow_mut().display_dialog(&text, (!title.is_empty()).then_some(title.as_str()));
    reply(r.map(Json::Str))
}

fn native_notify(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let text = arg(args, 0, ctx)?;
    let title = arg(args, 1, ctx)?;
    let e = env(ctx);
    let r = e.host.borrow_mut().display_notification(&text, (!title.is_empty()).then_some(title.as_str()));
    reply(r.map(|()| Json::Null))
}

fn native_shell(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let command = arg(args, 0, ctx)?;
    let e = env(ctx);
    if !e.policy.allows_shell() {
        return reply(Err(ScriptError::new(ERR_NOT_PERMITTED, "Not allowed to run shell commands.")));
    }
    let r = e.host.borrow_mut().shell(&command);
    reply(r.map(Json::Str))
}

fn native_sleep(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let secs = arg(args, 0, ctx)?.parse::<f64>().unwrap_or(0.0);
    let e = env(ctx);
    e.host.borrow_mut().sleep(secs);
    reply(Ok(Json::Null))
}

fn native_log(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let line = arg(args, 0, ctx)?;
    env(ctx).log.borrow_mut().push(line);
    reply(Ok(Json::Null))
}

fn native_source(_: &JsValue, _: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let e = env(ctx);
    let argv = Json::Arr(e.argv.iter().cloned().map(Json::Str).collect());
    reply(Ok(Json::Obj(vec![("source".into(), Json::Str(e.source.clone())), ("argv".into(), argv)])))
}

/// The JavaScript half: the natives are gathered out of the global object, then the globals a script sees are made.
const PRELUDE: &str = r#"(function(){'use strict';
const H={};
for(const k of Object.getOwnPropertyNames(globalThis)){if(k.startsWith('__bf_')){H[k.slice(5)]=globalThis[k];delete globalThis[k];}}
class AutomationError extends Error{constructor(message,number){super(message);this.name='AutomationError';this.number=number;}}
function call(name,...args){
  const r=JSON.parse(H[name](...args));
  if(!r.ok){throw new AutomationError(r.message,r.number);}
  return r.result;
}
function words(prop){return prop.replace(/([A-Z])/g,' $1').toLowerCase().trim();}
function reference(app,cls,props,sel,collection){
  const object=()=>JSON.stringify(Object.assign({class:cls},sel));
  const get=(property)=>call('object',app,'getd',object(),property,'null');
  const target=()=>get('');
  return new Proxy(target,{
    get(t,prop){
      if(typeof prop!=='string'||prop==='then'||prop==='toJSON'){return undefined;}
      if(prop==='toString'){return function(){return 'Application('+JSON.stringify(app)+').'+cls+'['+JSON.stringify(sel)+']';};}
      if(prop==='delete'){return function(){return call('object',app,'delt',object(),'','null');};}
      if(collection){
        if(prop==='byName'){return function(n){return reference(app,cls,props,{name:String(n)},false);};}
        if(prop==='whose'){
          return function(c){
            const keys=c===null||typeof c!=='object'?[]:Object.keys(c);
            if(keys.length!==1){throw new AutomationError('whose takes an object with one property.',-1700);}
            return reference(app,cls,props,{whose:{property:words(keys[0]),value:c[keys[0]]}},true);
          };
        }
        if(/^-?\d+$/.test(prop)){
          const n=Number(prop);
          return reference(app,cls,props,{index:n>=0?n+1:n},false);
        }
      }
      const property=words(prop);
      if(props.length>0&&props.indexOf(property)<0){throw new AutomationError("Can't get "+property+" of "+cls+'.',-1728);}
      const read=function(){return get(property);};
      read.set=function(v){return call('object',app,'setd',object(),property,JSON.stringify(v===undefined?null:v));};
      return read;
    },
    set(t,prop,value){
      if(typeof prop!=='string'){return false;}
      call('object',app,'setd',object(),words(prop),JSON.stringify(value===undefined?null:value));
      return true;
    }
  });
}
globalThis.Application=function(name){
  if(typeof name!=='string'||name===''){throw new AutomationError('Application needs the name of an app.',-1700);}
  return new Proxy({},{
    get(target,prop){
      if(typeof prop!=='string'||prop==='then'||prop==='toJSON'){return undefined;}
      if(prop==='toString'){return function(){return 'Application('+JSON.stringify(name)+')';};}
      const r=call('resolve',name,words(prop));
      if(r.kind==='elements'){return reference(name,r.class,r.properties,{all:true},true);}
      return function(direct,params){
        const hasDirect=arguments.length>0&&direct!==undefined;
        return call('send',name,words(prop),hasDirect?'1':'0',hasDirect?JSON.stringify(direct):'null',JSON.stringify(params===undefined?{}:params));
      };
    }
  });
};
globalThis.delay=function(seconds){call('sleep',String(Number(seconds)));};
globalThis.displayDialog=function(text,options){
  const o=options||{};
  return {buttonReturned:call('dialog',String(text),o.withTitle===undefined?'':String(o.withTitle))};
};
globalThis.displayNotification=function(text,options){
  const o=options||{};
  call('notify',String(text),o.withTitle===undefined?'':String(o.withTitle));
};
globalThis.doShellScript=function(command){return call('shell',String(command));};
function logger(){return function(...a){call('log',a.map(x=>typeof x==='string'?x:JSON.stringify(x)).join(' '));};}
globalThis.console={log:logger(),error:logger(),info:logger(),warn:logger()};
globalThis.__main=function(){
  const s=call('source');
  try{
    let out=(0,eval)(s.source);
    if(typeof globalThis.run==='function'){out=globalThis.run(s.argv);}
    const j=JSON.stringify(out===undefined?null:out);
    return '{"ok":true,"result":'+(j===undefined?'null':j)+'}';
  }catch(e){
    const isObj=e!==null&&typeof e==='object';
    const n=(isObj&&typeof e.number==='number')?e.number:((e instanceof SyntaxError)?-2740:-2700);
    const m=(isObj&&e.message!==undefined)?String(e.message):String(e);
    return JSON.stringify({ok:false,number:n,message:m});
  }
};
})();"#;

/// Checks that `source` parses as a script, without running it: what `osacompile -l JavaScript` does before it writes the
/// compiled file. A syntax error is `-2740` with the engine's message.
pub fn check_syntax(source: &str) -> Result<(), ScriptError> {
    let mut ctx = Context::default();
    boa_engine::Script::parse(Source::from_bytes(source), None, &mut ctx).map(|_| ()).map_err(|e| ScriptError::new(bifrost_core::script::interp::ERR_SYNTAX, e.to_string()))
}

/// Runs `source` as a JavaScript automation script with `argv`, against `host`, under `policy`.
pub fn run(source: &str, argv: &[String], host: Box<dyn Host>, policy: Policy) -> Run {
    let e = Rc::new(Env { host: RefCell::new(host), policy, source: source.to_string(), argv: argv.to_vec(), dicts: RefCell::new(Vec::new()), log: RefCell::new(Vec::new()) });
    let mut ctx = Context::default();
    ctx.insert_data(e.clone());
    let natives: [(&str, fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>); 9] = [
        ("resolve", native_resolve),
        ("object", native_object),
        ("send", native_send),
        ("dialog", native_dialog),
        ("notify", native_notify),
        ("shell", native_shell),
        ("sleep", native_sleep),
        ("log", native_log),
        ("source", native_source),
    ];
    for (name, f) in natives {
        let key = JsString::from(format!("__bf_{name}").as_str());
        ctx.register_global_callable(key, 0, NativeFunction::from_fn_ptr(f)).expect("register native");
    }
    let result = match ctx.eval(Source::from_bytes(PRELUDE)) {
        Err(err) => Err(ScriptError::general(format!("the automation prelude failed: {err}"))),
        Ok(_) => {
            let rl = ctx.runtime_limits_mut();
            rl.set_loop_iteration_limit(LOOP_LIMIT);
            rl.set_recursion_limit(400);
            rl.set_stack_size_limit(10 * 1024);
            rl.set_backtrace_limit(10);
            finish(&mut ctx)
        }
    };
    let _ = ctx.run_jobs();
    let output = e.log.borrow().clone();
    Run { result, output }
}

fn finish(ctx: &mut Context) -> Result<Value, ScriptError> {
    let text = match ctx.eval(Source::from_bytes("__main()")) {
        Ok(v) => match v.to_string(ctx) {
            Ok(s) => s.to_std_string_escaped(),
            Err(err) => return Err(ScriptError::new(ERR_GENERIC, err.to_string())),
        },
        // a stop the script cannot catch (the loop limit, the stack limit)
        Err(err) => return Err(ScriptError::new(ERR_GENERIC, err.to_string())),
    };
    let doc = json::parse(&text).map_err(|m| ScriptError::new(ERR_GENERIC, format!("unreadable script result: {m}")))?;
    if matches!(doc.get("ok"), Some(Json::Bool(true))) {
        Ok(Value::from_json(doc.get("result").unwrap_or(&Json::Null)))
    } else {
        let number = doc.get("number").and_then(Json::number).map_or(ERR_GENERIC, |n| n as i64);
        Err(ScriptError::new(number, doc.str_field("message").unwrap_or_default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::script::dictionary::Command;

    type Sent = Rc<RefCell<Vec<(String, String, String, Json)>>>;

    struct Mock {
        sent: Sent,
        shell_out: Rc<RefCell<Vec<String>>>,
        slept: Rc<RefCell<f64>>,
    }

    impl Host for Mock {
        fn send_event(&mut self, app: &str, class: &str, id: &str, params: &Json, _timeout: u64) -> Result<Json, ScriptError> {
            self.sent.borrow_mut().push((app.into(), class.into(), id.into(), params.clone()));
            match id {
                "make" => Ok(Json::Str("note-1".into())),
                "fail" => Err(ScriptError::new(-128, "The note could not be made.")),
                "getd" if class == "core" && params.get("object").and_then(|o| o.get("all")).is_some() => Ok(Json::Arr(vec![Json::Str("n1".into()), Json::Str("n2".into())])),
                "getd" => Ok(Json::Str("a value".into())),
                _ => Ok(Json::Null),
            }
        }
        fn dictionary(&mut self, app: &str) -> Result<Dictionary, ScriptError> {
            if app != "Notes" {
                return Err(ScriptError::new(-600, format!("{app} isn't running.")));
            }
            let cmd = |name: &str, id: &str, params: &[&str]| Command { name: name.into(), class: "notes".into(), id: id.into(), params: params.iter().map(|p| p.to_string()).collect(), result: String::new() };
            let class = |name: &str, props: &[&str]| bifrost_core::script::dictionary::Class { name: name.into(), properties: props.iter().map(|p| p.to_string()).collect(), elements: vec![] };
            Ok(Dictionary {
                commands: vec![cmd("make new note", "make", &["title", "body"]), cmd("count notes", "count", &[]), cmd("fail now", "fail", &[])],
                classes: vec![class("note", &["name", "body", "creation date"]), class("category", &[])],
            })
        }
        fn display_dialog(&mut self, text: &str, title: Option<&str>) -> Result<String, ScriptError> {
            self.shell_out.borrow_mut().push(format!("dialog:{}:{text}", title.unwrap_or("")));
            Ok("OK".into())
        }
        fn display_notification(&mut self, text: &str, _title: Option<&str>) -> Result<(), ScriptError> {
            self.shell_out.borrow_mut().push(format!("note:{text}"));
            Ok(())
        }
        fn shell(&mut self, command: &str) -> Result<String, ScriptError> {
            self.shell_out.borrow_mut().push(format!("sh:{command}"));
            Ok("shell output".into())
        }
        fn sleep(&mut self, secs: f64) {
            *self.slept.borrow_mut() += secs;
        }
        fn app_id(&mut self, app: &str) -> Option<String> {
            (app == "Notes").then(|| "is.example.notes".to_string())
        }
    }

    fn mock() -> (Box<dyn Host>, Sent, Rc<RefCell<Vec<String>>>, Rc<RefCell<f64>>) {
        let sent: Sent = Rc::default();
        let out: Rc<RefCell<Vec<String>>> = Rc::default();
        let slept: Rc<RefCell<f64>> = Rc::default();
        (Box::new(Mock { sent: sent.clone(), shell_out: out.clone(), slept: slept.clone() }), sent, out, slept)
    }

    fn go(source: &str, argv: &[&str], policy: Policy) -> (Run, Sent, Rc<RefCell<Vec<String>>>, Rc<RefCell<f64>>) {
        let (host, sent, out, slept) = mock();
        let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
        (run(source, &argv, host, policy), sent, out, slept)
    }

    #[test]
    fn a_dictionary_command_is_sent_as_a_typed_event() {
        let (r, sent, _, _) = go(r#"Application("Notes").makeNewNote("Hi", {body: "text"})"#, &[], Policy::allow_all());
        assert_eq!(r.result, Ok(Value::Str("note-1".into())));
        let sent = sent.borrow();
        assert_eq!(sent.len(), 1);
        assert_eq!((sent[0].0.as_str(), sent[0].1.as_str(), sent[0].2.as_str()), ("Notes", "notes", "make"));
        assert_eq!(sent[0].3.str_field("title").as_deref(), Some("Hi"));
        assert_eq!(sent[0].3.str_field("body").as_deref(), Some("text"));
    }

    #[test]
    fn a_run_function_gets_argv_and_console_lines_are_collected() {
        let (r, _, _, _) = go(r#"function run(argv){ console.log("hello", argv[0]); console.error({n: 1}); return argv.length + 1; }"#, &["a", "b"], Policy::none());
        assert_eq!(r.result, Ok(Value::Num(3.0)));
        assert_eq!(r.output, vec!["hello a".to_string(), r#"{"n":1}"#.to_string()]);
    }

    #[test]
    fn the_last_expression_is_the_result_and_objects_become_records() {
        let (r, _, _, _) = go(r#"({a: 1, b: [true, null, "s"]})"#, &[], Policy::none());
        assert_eq!(r.result, Ok(Value::Record(vec![("a".into(), Value::Num(1.0)), ("b".into(), Value::List(vec![Value::Bool(true), Value::Missing, Value::Str("s".into())]))])));
        let (r, _, _, _) = go("undefined", &[], Policy::none());
        assert_eq!(r.result, Ok(Value::Missing));
    }

    #[test]
    fn undeclared_commands_and_labels_throw_with_event_error_numbers() {
        let (r, sent, _, _) = go(r#"Application("Notes").explode()"#, &[], Policy::allow_all());
        assert_eq!(r.result.unwrap_err().number, ERR_NOT_HANDLED);
        let (r, _, _, _) = go(r#"Application("Notes").makeNewNote("x", {colour: "red"})"#, &[], Policy::allow_all());
        assert_eq!(r.result.unwrap_err().number, ERR_COERCION);
        let (r, _, _, _) = go(r#"Application("Mail").anything()"#, &[], Policy::allow_all());
        assert_eq!(r.result.unwrap_err().number, -600, "the host's error number reaches the script");
        assert!(sent.borrow().is_empty(), "nothing was sent for a refused command");
    }

    #[test]
    fn the_script_can_catch_an_app_error_and_see_its_number() {
        let (r, _, _, _) = go(r#"try { Application("Notes").failNow(); "no" } catch (e) { e.number + ":" + e.message }"#, &[], Policy::allow_all());
        assert_eq!(r.result, Ok(Value::Str("-128:The note could not be made.".into())));
    }

    #[test]
    fn events_outside_the_grants_are_refused_before_they_are_sent() {
        let (r, sent, _, _) = go(r#"Application("Notes").countNotes()"#, &[], Policy::none());
        let e = r.result.unwrap_err();
        assert_eq!(e.number, ERR_NOT_PERMITTED);
        assert!(e.message.contains("Notes"));
        assert!(sent.borrow().is_empty());
        // a grant by app id covers the name the script spells (the host resolves it)
        let (r, sent, _, _) = go(r#"Application("Notes").countNotes()"#, &[], Policy::for_apps(&["is.example.notes"]));
        assert_eq!(r.result, Ok(Value::Missing));
        assert_eq!(sent.borrow().len(), 1);
        // a grant for another app does not
        let (r, _, _, _) = go(r#"Application("Notes").countNotes()"#, &[], Policy::for_apps(&["is.example.mail"]));
        assert_eq!(r.result.unwrap_err().number, ERR_NOT_PERMITTED);
    }

    #[test]
    fn the_shell_needs_its_own_grant() {
        let (r, _, out, _) = go(r#"doShellScript("echo hi")"#, &[], Policy::for_apps(&["notes"]));
        assert_eq!(r.result.unwrap_err().number, ERR_NOT_PERMITTED);
        assert!(out.borrow().is_empty());
        let (r, _, out, _) = go(r#"doShellScript("echo hi")"#, &[], Policy::for_apps(&[]).with_shell(true));
        assert_eq!(r.result, Ok(Value::Str("shell output".into())));
        assert_eq!(out.borrow().as_slice(), ["sh:echo hi".to_string()]);
    }

    #[test]
    fn dialogs_notifications_and_delays_go_to_the_host() {
        let (r, _, out, slept) = go(r#"delay(1.5); displayNotification("done", {withTitle: "T"}); displayDialog("sure?", {withTitle: "Q"}).buttonReturned"#, &[], Policy::none());
        assert_eq!(r.result, Ok(Value::Str("OK".into())));
        assert_eq!(out.borrow().as_slice(), ["note:done".to_string(), "dialog:Q:sure?".to_string()]);
        assert_eq!(*slept.borrow(), 1.5);
    }

    #[test]
    fn syntax_errors_thrown_errors_and_runaway_loops_end_the_run() {
        let (r, _, _, _) = go("var = ;", &[], Policy::none());
        assert_eq!(r.result.unwrap_err().number, bifrost_core::script::interp::ERR_SYNTAX);
        let (r, _, _, _) = go(r#"throw new Error("boom")"#, &[], Policy::none());
        let e = r.result.unwrap_err();
        assert_eq!((e.number, e.message.as_str()), (ERR_GENERIC, "boom"));
        let (r, _, _, _) = go("while (true) {}", &[], Policy::none());
        assert!(r.result.is_err(), "the loop limit stops a script that never ends");
    }

    #[test]
    fn check_syntax_accepts_a_script_without_running_it_and_refuses_a_bad_one() {
        assert!(check_syntax("function run(argv){ return argv.length; }\nthrow new Error('never runs');").is_ok());
        let e = check_syntax("var = ;").unwrap_err();
        assert_eq!(e.number, bifrost_core::script::interp::ERR_SYNTAX);
        assert!(!e.message.is_empty());
        assert!(check_syntax("").is_ok());
    }

    #[test]
    fn a_grant_list_is_read_line_by_line_and_typos_grant_nothing() {
        let p = Policy::from_text("# my script\nallow Notes\nallow is.example.mail\nshell allow\n\nalow Reminders\nallow \n");
        assert!(p.allows("notes", None) && p.allows("Whatever", Some("is.example.mail")));
        assert!(!p.allows("Reminders", None));
        assert!(p.allows_shell());
        let all = Policy::from_text("allow *");
        assert!(all.allows("Anything", None) && !all.allows_shell());
        assert_eq!(Policy::from_text(""), Policy::none());
        assert!(!Policy::none().allows("Notes", Some("is.example.notes")));
    }

    fn object_of(sent: &Sent, i: usize) -> Json {
        sent.borrow()[i].3.get("object").cloned().expect("a core event carries an object")
    }

    #[test]
    fn a_collection_reads_every_object_with_a_getd_event() {
        let (r, sent, _, _) = go(r#"Application("Notes").notes()"#, &[], Policy::allow_all());
        assert_eq!(r.result, Ok(Value::List(vec![Value::Str("n1".into()), Value::Str("n2".into())])));
        let sent = sent.borrow();
        assert_eq!((sent[0].0.as_str(), sent[0].1.as_str(), sent[0].2.as_str()), ("Notes", "core", "getd"));
        let object = sent[0].3.get("object").unwrap();
        assert_eq!(object.str_field("class").as_deref(), Some("note"));
        assert_eq!(object.get("all"), Some(&Json::Bool(true)));
    }

    #[test]
    fn an_index_is_zero_based_and_a_negative_one_counts_from_the_end() {
        let (r, sent, _, _) = go(r#"[Application("Notes").notes[0](), Application("Notes").notes[2].name(), Application("Notes").notes[-1]()]"#, &[], Policy::allow_all());
        assert_eq!(r.result, Ok(Value::List(vec![Value::Str("a value".into()); 3])));
        assert_eq!(object_of(&sent, 0).get("index"), Some(&Json::Num(1.0)));
        let second = object_of(&sent, 1);
        assert_eq!((second.get("index"), second.str_field("property").as_deref()), (Some(&Json::Num(3.0)), Some("name")));
        assert_eq!(object_of(&sent, 2).get("index"), Some(&Json::Num(-1.0)));
    }

    #[test]
    fn selection_by_name_and_by_property_builds_the_specifier_record() {
        let (r, sent, _, _) = go(r#"Application("Notes").notes.byName("Shopping").body(); Application("Notes").notes.whose({creationDate: "today"})()"#, &[], Policy::allow_all());
        assert!(r.result.is_ok());
        let by_name = object_of(&sent, 0);
        assert_eq!((by_name.str_field("name").as_deref(), by_name.str_field("property").as_deref()), (Some("Shopping"), Some("body")));
        let whose = object_of(&sent, 1);
        let w = whose.get("whose").unwrap();
        assert_eq!((w.str_field("property").as_deref(), w.str_field("value").as_deref()), (Some("creation date"), Some("today")));
        let (r, _, _, _) = go(r#"Application("Notes").notes.whose({a: 1, b: 2})"#, &[], Policy::allow_all());
        assert_eq!(r.result.unwrap_err().number, ERR_COERCION);
    }

    #[test]
    fn assigning_a_property_sends_setd_and_delete_sends_delt() {
        let (r, sent, _, _) = go(r#"const n = Application("Notes").notes.byName("x"); n.body = "hi"; n.name.set({a: [1]}); n.delete()"#, &[], Policy::allow_all());
        assert!(r.result.is_ok(), "{:?}", r.result);
        let sent = sent.borrow();
        assert_eq!(sent.iter().map(|s| s.2.as_str()).collect::<Vec<_>>(), ["setd", "setd", "delt"]);
        assert_eq!(sent[0].3.str_field("to").as_deref(), Some("hi"));
        assert_eq!(sent[0].3.get("object").unwrap().str_field("property").as_deref(), Some("body"));
        assert_eq!(sent[1].3.get("to").unwrap().encode(), r#"{"a":[1]}"#);
        assert!(sent[2].3.get("object").unwrap().get("property").is_none() && sent[2].3.get("to").is_none());
    }

    #[test]
    fn a_property_the_class_does_not_declare_is_no_such_object_and_nothing_is_sent() {
        let (r, sent, _, _) = go(r#"Application("Notes").notes[0].colour()"#, &[], Policy::allow_all());
        let e = r.result.unwrap_err();
        assert_eq!(e.number, ERR_NO_SUCH_OBJECT);
        assert!(e.message.contains("colour") && e.message.contains("note"));
        let (r, _, _, _) = go(r#"Application("Notes").notes[0].colour = "red""#, &[], Policy::allow_all());
        assert_eq!(r.result.unwrap_err().number, ERR_NO_SUCH_OBJECT);
        // a class that lists no properties accepts any, as the BifrOSt script language does
        let (r, sent2, _, _) = go(r#"Application("Notes").categories[0].anything()"#, &[], Policy::allow_all());
        assert!(r.result.is_ok());
        assert_eq!(sent2.borrow().len(), 1);
        assert!(sent.borrow().is_empty());
    }

    #[test]
    fn specifiers_obey_the_grants_and_a_name_that_is_no_class_stays_a_command() {
        let (r, sent, _, _) = go(r#"Application("Notes").notes()"#, &[], Policy::none());
        assert_eq!(r.result.unwrap_err().number, ERR_NOT_PERMITTED);
        assert!(sent.borrow().is_empty());
        let (r, _, _, _) = go(r#"Application("Notes").folders()"#, &[], Policy::allow_all());
        assert_eq!(r.result.unwrap_err().number, ERR_NOT_HANDLED, "folders is neither a command nor a class");
        let (r, _, _, _) = go(r#"Application("Notes").notes[0].name()"#, &[], Policy::for_apps(&["is.example.notes"]));
        assert_eq!(r.result, Ok(Value::Str("a value".into())));
    }

    #[test]
    fn plural_names_find_their_declared_class() {
        let class = |n: &str| bifrost_core::script::dictionary::Class { name: n.into(), properties: vec![], elements: vec![] };
        let dict = Dictionary { commands: vec![], classes: vec![class("category"), class("note folder"), class("box"), class("note"), class("news")] };
        assert_eq!(element_class(&dict, "notes").as_deref(), Some("note"));
        assert_eq!(element_class(&dict, "categories").as_deref(), Some("category"));
        assert_eq!(element_class(&dict, "note folders").as_deref(), Some("note folder"));
        assert_eq!(element_class(&dict, "boxes").as_deref(), Some("box"));
        assert_eq!(element_class(&dict, "news").as_deref(), Some("news"), "an exact class name wins over a stripped plural");
        assert_eq!(element_class(&dict, "Note  Folders").as_deref(), Some("note folder"));
        assert_eq!(element_class(&dict, "windows"), None);
    }
}
