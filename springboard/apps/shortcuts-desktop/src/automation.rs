//! Automation for Shortcuts: the intents the app declares, a dry run of a shortcut and the scripting model of its list.
//! Everything here is pure over the list, so the window, the control socket and the unit tests share one path.
//!
//! - `run-shortcut` and `plan-shortcut` take a `shortcut` parameter. Its text is resolved with the shared resolution
//!   model (`intent_resolve`): an id or a unique name wins, a name that several shortcuts share asks for the id, and
//!   an unknown name stops with a reason.
//! - `plan-shortcut` runs the shortcut's steps through a `Workflow` as a dry run. Each step adds its line and the
//!   output is the steps in order. Nothing is performed.
//! - `run-shortcut` resolves the same way and then reports that this device cannot run shortcut actions yet. The
//!   shell has no action runner, so the reply never claims success.
//! - `script.dictionary`, `script.get` and `script.set` expose the list through the scripting model: a `shortcut`
//!   object with `name` (read-write), `color` (read-write, a palette index) and `actions` (read-only count).
//!   Shortcuts are found by name; the first one with a name wins.

use crate::model::{self, Shortcut, MAX_NAME};
use crate::widget::PALETTE;
use bifrost_app::actor::Actor;
use bifrost_app::control::{RpcError, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_app::controls::intent_resolve as ir;
use bifrost_app::controls::onscreen_entity::{EntityRef, OnscreenContext};
use bifrost_app::controls::scripting as sc;
use bifrost_app::controls::value_query::{QueryError, ValueItem, ValueQueryResult};
use bifrost_app::controls::workflow::{Step, Workflow, WorkflowController, WorkflowError};
use bifrost_app::intents::{self, Entity, EntityQuery, Intent, IntentResult, Param, ParamType};
use bifrost_core::json::Json;
use std::path::Path;

/// The file the window saves the list to, inside the app's data directory.
pub const FILE: &str = "shortcuts.tsv";
/// The entity type a shortcut parameter refers to.
pub const SHORTCUT: &str = "Shortcut";
pub const RUN: &str = "run-shortcut";
pub const PLAN: &str = "plan-shortcut";

/// The intents this app declares. `manifest.toml` lists the same ids (checked by a unit test).
pub fn declared() -> Vec<Intent> {
    // Optional at the protocol level: when the parameter is missing, the shortcut in front of the window is used.
    let shortcut = || Param::new("shortcut", "Shortcut", ParamType::Entity(SHORTCUT.into())).optional();
    vec![
        Intent::new(RUN, "Run Shortcut", "Runs a shortcut's actions. Pass its id, or its name when only one shortcut has it.")
            .param(shortcut()),
        Intent::new(PLAN, "Plan Shortcut", "Lists the steps a shortcut would run, in order, without running them.")
            .param(shortcut()),
    ]
}

/// Runs the plans on the planner's own thread, so a plan never runs on the thread that handles the window. The caller
/// waits for the answer; the window does not.
pub struct Planner {
    actor: Actor<()>,
}

impl Planner {
    pub fn new() -> Planner {
        Planner { actor: Actor::new(()) }
    }

    /// The dry-run text of `s` (see `plan_text`), computed on the planner's thread.
    pub fn plan(&self, s: Shortcut) -> Result<String, String> {
        self.actor.ask(move |_| plan_text(&s)).unwrap_or_else(|| Err("the planner has stopped".to_string()))
    }
}

impl Default for Planner {
    fn default() -> Self {
        Planner::new()
    }
}

/// The shortcuts as a parameter-picker result: the id is the list index, the title the name, and the properties
/// `color` (palette index) and `actions` (count of steps).
pub fn picker(list: &[Shortcut]) -> Result<ValueQueryResult, QueryError> {
    let items = list
        .iter()
        .enumerate()
        .map(|(i, s)| {
            ValueItem::new(&i.to_string(), &s.name)
                .map(|item| item.with_property("color", &s.color.to_string()).with_property("actions", &s.actions.len().to_string()))
        })
        .collect::<Result<Vec<ValueItem>, QueryError>>()?;
    ValueQueryResult::new(items)
}

/// The annotations for the shortcuts the window shows: each shown one is a view, and the one in front is selected.
pub fn onscreen_context(shown: &[(usize, bool)]) -> OnscreenContext {
    let mut ctx = OnscreenContext::new();
    for &(i, front) in shown {
        // the type and the index are never empty, so the annotation is always accepted
        let _ = ctx.annotate(i as u64, EntityRef { type_name: SHORTCUT.into(), id: i.to_string() }, front);
    }
    ctx
}

/// The index of the shortcut in front, from the onscreen annotations (an open editor or picker).
pub fn frontmost(ctx: &OnscreenContext) -> Option<usize> {
    let annotations = ctx.view_annotations();
    let found = annotations
        .iter()
        .find(|a| a.is_selected && a.entity.type_name == SHORTCUT)
        .and_then(|a| a.entity.id.parse::<usize>().ok());
    found
}

/// The list as the app should see it when no window is open: the saved file, or the first-launch shortcuts.
pub fn stored(data_dir: &Path) -> Vec<Shortcut> {
    let path = data_dir.join(FILE);
    if path.exists() {
        model::load(&path)
    } else {
        model::defaults()
    }
}

/// Performs one of the declared intents on `list`.
///
/// Without a `shortcut` parameter the shortcut in front (`frontmost`, an index into `list`) is used.
pub fn perform(
    list: &[Shortcut],
    id: &str,
    params: &Json,
    front: Option<usize>,
    planner: &Planner,
) -> Result<IntentResult, RpcError> {
    if id != RUN && id != PLAN {
        return Err(RpcError::new(METHOD_NOT_FOUND, format!("no intent {id}")));
    }
    let index = match (params.get("shortcut"), front) {
        (None | Some(Json::Null), Some(i)) if i < list.len() => i,
        (None | Some(Json::Null), _) => return Err(RpcError::new(INVALID_PARAMS, "Shortcut is required")),
        _ => resolve(list, params)?,
    };
    let s = &list[index];
    if id == RUN {
        return Err(intents::failed(format!("{} cannot run on this device yet", s.name)));
    }
    let steps = planner.plan(s.clone()).map_err(|why| intents::failed(format!("{} {why}", s.name)))?;
    let n = s.actions.len();
    let plural = if n == 1 { "" } else { "s" };
    Ok(IntentResult::value(Json::Str(steps)).with_dialog(&format!("{} runs {n} step{plural}.", s.name)))
}

/// Resolves the `shortcut` parameter to an index into `list`.
fn resolve(list: &[Shortcut], params: &Json) -> Result<usize, RpcError> {
    let candidates: Vec<ir::Candidate> =
        list.iter().enumerate().map(|(i, s)| ir::Candidate::new(&i.to_string(), &s.name)).collect();
    let specs = [ir::ParamSpec::new("shortcut", "Shortcut", ir::Kind::Entity(candidates))];
    let text = params.get("shortcut").and_then(Json::as_str).unwrap_or("");
    match ir::resolve(&specs, &ir::Supplied::new().with("shortcut", text), false) {
        ir::Resolution::Ready { values } => values
            .first()
            .and_then(|(_, id)| id.parse::<usize>().ok())
            .filter(|i| *i < list.len())
            .ok_or_else(|| RpcError::new(INVALID_PARAMS, "no shortcut has that id")),
        ir::Resolution::Disambiguate { choices, .. } => {
            let options: Vec<String> = choices.iter().map(|id| format!("{} (id {id})", name_at(list, id))).collect();
            Err(RpcError::new(
                INVALID_PARAMS,
                format!("more than one shortcut matches {text}; pass the id of one: {}", options.join(", ")),
            ))
        }
        ir::Resolution::Invalid { reason, .. } => Err(RpcError::new(INVALID_PARAMS, reason)),
        ir::Resolution::Needs { title, .. } => Err(RpcError::new(INVALID_PARAMS, format!("{title} is required"))),
        ir::Resolution::Confirm { .. } => Err(RpcError::new(INVALID_PARAMS, "the shortcut needs confirmation")),
    }
}

fn name_at(list: &[Shortcut], id: &str) -> String {
    id.parse::<usize>().ok().and_then(|i| list.get(i)).map(|s| s.name.clone()).unwrap_or_default()
}

/// One line per action, in order, produced by a workflow whose steps each add their line to the text so far.
pub fn plan_text(s: &Shortcut) -> Result<String, String> {
    let mut wf = Workflow::new();
    for a in &s.actions {
        let line = match a.value() {
            v if v.is_empty() => a.title().to_string(),
            v => format!("{}: {v}", a.title()),
        };
        wf.add(Step::new(a.title(), move |input: &str| {
            Ok(if input.is_empty() { line.clone() } else { format!("{input}\n{line}") })
        }));
    }
    let mut controller = WorkflowController::new(wf);
    controller.run("").map_err(|e| match e {
        WorkflowError::NoEnabledSteps => "has no actions".to_string(),
        other => format!("could not be planned: {other:?}"),
    })?;
    Ok(controller.workflow().output().unwrap_or_default().to_string())
}

/// `entity.query` for the shortcut type: all of them, the ones with the given ids, or the ones whose name contains
/// the text (ignoring case).
pub fn entities(list: &[Shortcut], ty: &str, query: &EntityQuery) -> Result<Vec<Entity>, RpcError> {
    if ty != SHORTCUT {
        return Err(RpcError::new(INVALID_PARAMS, format!("unknown entity type {ty}")));
    }
    let result = picker(list).map_err(|e| RpcError::new(INVALID_PARAMS, format!("{e:?}")))?;
    let all = result.items().iter().map(|item| {
        let n: usize = item.property("actions").and_then(|v| v.parse().ok()).unwrap_or(0);
        let steps = if n == 1 { "1 action".to_string() } else { format!("{n} actions") };
        Entity::new(SHORTCUT, item.id(), item.title()).subtitle(&steps)
    });
    Ok(match query {
        EntityQuery::Suggested => all.collect(),
        EntityQuery::Ids(ids) => all.filter(|e| ids.contains(&e.id)).collect(),
        EntityQuery::String(text) => {
            let text = text.to_lowercase();
            all.filter(|e| e.title.to_lowercase().contains(&text)).collect()
        }
    })
}

// ---- scripting ----

fn dictionary() -> sc::Dictionary {
    let prop = |code: &str, kind: sc::Kind, writable: bool| sc::Property { code: code.into(), kind, writable };
    sc::Dictionary {
        suite: "Shortcuts".into(),
        classes: vec![
            sc::Class {
                name: "application".into(),
                properties: vec![prop("name", sc::Kind::Text, false)],
                elements: vec![sc::ElementArray { code: "shortcuts".into(), class: "shortcut".into() }],
            },
            sc::Class {
                name: "shortcut".into(),
                properties: vec![
                    prop("name", sc::Kind::Text, true),
                    prop("color", sc::Kind::Int, true),
                    prop("actions", sc::Kind::Int, false),
                ],
                elements: Vec::new(),
            },
        ],
    }
}

/// The scripting view of `list`: a fresh target each call, so a failed set changes nothing.
fn target(list: &[Shortcut]) -> sc::Target {
    let mut root = sc::Object::new("application", "", "Shortcuts");
    root.put("name", sc::Value::Text("Shortcuts".into()));
    for (i, s) in list.iter().enumerate() {
        let mut o = sc::Object::new("shortcut", &i.to_string(), &s.name);
        o.put("name", sc::Value::Text(s.name.clone()));
        o.put("color", sc::Value::Int(s.color as i64));
        o.put("actions", sc::Value::Int(s.actions.len() as i64));
        root.add("shortcuts", o);
    }
    sc::Target::new(dictionary(), root)
}

fn path(i: usize) -> [sc::Step<'static>; 1] {
    [sc::Step { element: "shortcuts", by: sc::Locator::Index(i) }]
}

fn index_of(list: &[Shortcut], name: &str) -> Result<usize, RpcError> {
    list.iter().position(|s| s.name == name).ok_or_else(|| RpcError::new(INVALID_PARAMS, format!("no shortcut named {name}")))
}

fn script_err(e: sc::ScriptError) -> RpcError {
    RpcError::new(INVALID_PARAMS, e.message)
}

fn to_json(v: sc::Value) -> Json {
    match v {
        sc::Value::Null => Json::Null,
        sc::Value::Bool(b) => Json::Bool(b),
        sc::Value::Int(n) => Json::Num(n as f64),
        sc::Value::Text(t) => Json::Str(t),
    }
}

fn from_json(j: &Json) -> Result<sc::Value, RpcError> {
    match j {
        Json::Null => Ok(sc::Value::Null),
        Json::Bool(b) => Ok(sc::Value::Bool(*b)),
        Json::Num(n) if n.is_finite() && n.fract() == 0.0 => Ok(sc::Value::Int(*n as i64)),
        Json::Str(t) => Ok(sc::Value::Text(t.clone())),
        _ => Err(RpcError::new(INVALID_PARAMS, "value must be text, a whole number, yes or no, or null")),
    }
}

/// `script.dictionary`: the classes and properties, as the XML text the scripting model writes.
pub fn dictionary_json() -> Json {
    Json::Str(dictionary().describe_xml())
}

/// `script.get` with `shortcut` (a name) and `property` (`name`, `color` or `actions`).
pub fn script_get(list: &[Shortcut], params: &Json) -> Result<Json, RpcError> {
    let i = index_of(list, intents::str_param(params, "shortcut")?)?;
    let property = intents::str_param(params, "property")?;
    let v = target(list).get(&path(i), property).map_err(script_err)?;
    Ok(to_json(v))
}

/// `script.set` with `shortcut`, `property` (`name` or `color`) and `value`. The new name and colour are checked
/// the way the window checks them; the list changes only when every check passes.
pub fn script_set(list: &mut Vec<Shortcut>, params: &Json) -> Result<Json, RpcError> {
    let i = index_of(list.as_slice(), intents::str_param(params, "shortcut")?)?;
    let property = intents::str_param(params, "property")?;
    let value = from_json(params.get("value").ok_or_else(|| RpcError::new(INVALID_PARAMS, "value is missing"))?)?;
    let mut t = target(list.as_slice());
    let p = path(i);
    t.set(&p, property, value).map_err(script_err)?;
    let name = match t.get(&p, "name").map_err(script_err)? {
        sc::Value::Text(n) => n,
        _ => String::new(),
    };
    let color = match t.get(&p, "color").map_err(script_err)? {
        sc::Value::Int(c) => c,
        _ => -1,
    };
    if name.trim().is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(RpcError::new(
            INVALID_PARAMS,
            format!("a name is 1 to {MAX_NAME} characters with no control characters"),
        ));
    }
    if color < 0 || color as usize >= PALETTE.len() {
        return Err(RpcError::new(INVALID_PARAMS, format!("color is an index from 0 to {}", PALETTE.len() - 1)));
    }
    list[i].name = name;
    list[i].color = color as usize;
    Ok(to_json(t.get(&p, property).map_err(script_err)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Action;
    use crate::ShortcutsApp;
    use bifrost_app::App;
    use std::path::PathBuf;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("shortcuts-automation-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("create the test directory");
        d
    }

    fn save(d: &Path, list: &[Shortcut]) {
        model::save(&d.join(FILE), list).expect("save the fixture list");
    }

    fn app() -> ShortcutsApp {
        ShortcutsApp { w: None, reminders: None, asked_login: false, planner: Planner::new() }
    }

    fn obj(pairs: &[(&str, &str)]) -> Json {
        Json::Obj(pairs.iter().map(|(k, v)| (k.to_string(), Json::Str(v.to_string()))).collect())
    }

    fn two_saves() -> Vec<Shortcut> {
        vec![
            Shortcut { name: "Save Battery".into(), color: 3, actions: vec![Action::LowPower(true)], schedule: None },
            Shortcut { name: "Save Power".into(), color: 2, actions: Vec::new(), schedule: None },
        ]
    }

    #[test]
    fn declared_intents_are_listed_in_the_manifest() {
        let manifest = include_str!("../manifest.toml");
        let ids: Vec<String> = declared().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![RUN.to_string(), PLAN.to_string()]);
        for id in ids {
            assert!(manifest.contains(&format!("id = \"{id}\"")), "{id} is missing from manifest.toml");
        }
    }

    #[test]
    fn run_resolves_by_id_or_unique_name_then_says_it_cannot_run() {
        let d = dir("run");
        save(&d, &model::defaults());
        let mut a = app();
        for text in ["0", "Bedtime", "bedtime"] {
            let err = intents::run(&mut a, None, &d, RUN, &obj(&[("shortcut", text)])).unwrap_err();
            assert!(err.message.contains("Bedtime cannot run"), "{text}: {}", err.message);
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_name_several_shortcuts_share_asks_for_the_id() {
        let d = dir("ambiguous");
        save(&d, &two_saves());
        let mut a = app();
        let err = intents::run(&mut a, None, &d, RUN, &obj(&[("shortcut", "Save")])).unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
        assert!(err.message.contains("Save Battery (id 0)"), "{}", err.message);
        assert!(err.message.contains("Save Power (id 1)"), "{}", err.message);
        // an exact name wins over the partial matches
        let err = intents::run(&mut a, None, &d, RUN, &obj(&[("shortcut", "Save Power")])).unwrap_err();
        assert_eq!(err.message, "Save Power cannot run on this device yet");
        let err = intents::run(&mut a, None, &d, RUN, &obj(&[("shortcut", "Nope")])).unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_shortcut_parameter_is_asked_for() {
        let d = dir("missing");
        save(&d, &model::defaults());
        let err = intents::run(&mut app(), None, &d, PLAN, &Json::Obj(Vec::new())).unwrap_err();
        assert_eq!(err.code, INVALID_PARAMS);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn plan_lists_the_steps_in_order_without_running_them() {
        let d = dir("plan");
        save(&d, &model::defaults());
        let r = intents::run(&mut app(), None, &d, PLAN, &obj(&[("shortcut", "Bedtime")])).unwrap();
        assert_eq!(r.value, Json::Str("Set Do Not Disturb: On\nSet Brightness: 10%\nSet Wi-Fi: Off".into()));
        assert_eq!(r.dialog.as_deref(), Some("Bedtime runs 3 steps."));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_catalog_gives_each_intent_its_title_and_description() {
        let intents = declared();
        assert_eq!(intents[0].title, "Run Shortcut");
        assert_eq!(intents[0].description, "Runs a shortcut's actions. Pass its id, or its name when only one shortcut has it.");
        assert_eq!(intents[1].title, "Plan Shortcut");
        assert_eq!(intents[1].description, "Lists the steps a shortcut would run, in order, without running them.");
    }

    #[test]
    fn an_intent_from_the_action_library_runs_on_the_shortcut_in_front() {
        let list = model::defaults();
        let front = list.iter().position(|s| s.name == "Bedtime");
        let planner = Planner::new();
        // the window runs an intent with no parameters, so the shortcut in front is the one used
        let r = perform(&list, PLAN, &Json::Obj(Vec::new()), front, &planner).unwrap();
        assert_eq!(r.dialog.as_deref(), Some("Bedtime runs 3 steps."));
        let err = perform(&list, RUN, &Json::Obj(Vec::new()), front, &planner).unwrap_err();
        assert_eq!(err.message, "Bedtime cannot run on this device yet");
    }

    #[test]
    fn plan_of_a_shortcut_without_actions_says_so() {
        let d = dir("empty");
        save(&d, &two_saves());
        let err = intents::run(&mut app(), None, &d, PLAN, &obj(&[("shortcut", "Save Power")])).unwrap_err();
        assert_eq!(err.message, "Save Power has no actions");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_control_socket_reaches_the_same_intents() {
        let d = dir("dispatch");
        save(&d, &model::defaults());
        let reply = intents::dispatch(&mut app(), None, &d, "intent.plan-shortcut", &obj(&[("shortcut", "Quick Timer")]));
        let json = reply.expect("the intent is handled").expect("the plan succeeds");
        assert_eq!(json.get("value").and_then(Json::as_str), Some("Start Timer: 10 min"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn entity_query_suggests_and_matches_shortcuts_by_name() {
        let d = dir("entities");
        save(&d, &two_saves());
        let mut a = app();
        let all = intents::dispatch(&mut a, None, &d, "entity.query", &Json::Obj(vec![("type".into(), Json::Str(SHORTCUT.into()))]));
        let Some(Ok(Json::Arr(all))) = all else { panic!("entity.query failed") };
        assert_eq!(all.len(), 2);
        let found = intents::dispatch(
            &mut a,
            None,
            &d,
            "entity.query",
            &Json::Obj(vec![("type".into(), Json::Str(SHORTCUT.into())), ("string".into(), Json::Str("BATTERY".into()))]),
        );
        let Some(Ok(Json::Arr(found))) = found else { panic!("entity.query failed") };
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].get("title").and_then(Json::as_str), Some("Save Battery"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn scripts_read_and_change_name_and_colour_and_refuse_the_rest() {
        let d = dir("script");
        save(&d, &model::defaults());
        let mut a = app();
        let get = |a: &mut ShortcutsApp, shortcut: &str, property: &str| {
            a.rpc(None, &d, "script.get", &Json::Obj(vec![
                ("shortcut".into(), Json::Str(shortcut.into())),
                ("property".into(), Json::Str(property.into())),
            ]))
        };
        let set = |a: &mut ShortcutsApp, shortcut: &str, property: &str, value: Json| {
            a.rpc(None, &d, "script.set", &Json::Obj(vec![
                ("shortcut".into(), Json::Str(shortcut.into())),
                ("property".into(), Json::Str(property.into())),
                ("value".into(), value),
            ]))
        };
        assert_eq!(get(&mut a, "Bedtime", "name"), Ok(Json::Str("Bedtime".into())));
        assert_eq!(get(&mut a, "Bedtime", "actions"), Ok(Json::Num(3.0)));
        assert!(get(&mut a, "Nope", "name").is_err());
        assert_eq!(set(&mut a, "Bedtime", "name", Json::Str("Night".into())), Ok(Json::Str("Night".into())));
        assert_eq!(get(&mut a, "Night", "name"), Ok(Json::Str("Night".into())));
        assert_eq!(set(&mut a, "Night", "color", Json::Num(6.0)), Ok(Json::Num(6.0)));
        // the change reached the saved file, so the next launch shows it
        let saved = model::load(&d.join(FILE));
        assert_eq!((saved[0].name.as_str(), saved[0].color), ("Night", 6));
        // read-only, out of range and empty values are refused and change nothing
        assert_eq!(set(&mut a, "Night", "actions", Json::Num(1.0)).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(set(&mut a, "Night", "color", Json::Num(9.0)).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(set(&mut a, "Night", "name", Json::Str("  ".into())).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(set(&mut a, "Night", "color", Json::Str("red".into())).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(model::load(&d.join(FILE))[0].name, "Night");
        let xml = a.rpc(None, &d, "script.dictionary", &Json::Obj(Vec::new()));
        assert!(matches!(xml, Ok(Json::Str(ref x)) if x.contains("shortcut")));
        let unknown = a.rpc(None, &d, "script.run", &Json::Obj(Vec::new()));
        assert_eq!(unknown.unwrap_err().code, METHOD_NOT_FOUND);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn script_set_renames_in_the_list_it_is_given() {
        let mut list = two_saves();
        let params = Json::Obj(vec![
            ("shortcut".into(), Json::Str("Save Power".into())),
            ("property".into(), Json::Str("name".into())),
            ("value".into(), Json::Str("Power Saver".into())),
        ]);
        script_set(&mut list, &params).expect("the rename is allowed");
        assert_eq!(list[1].name, "Power Saver");
        assert_eq!(list[0].actions, vec![Action::LowPower(true)]);
    }

    #[test]
    fn picker_gives_each_shortcut_its_index_name_colour_and_action_count() {
        let list = two_saves();
        let r = picker(&list).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r.get("0").map(|i| i.title()), Some("Save Battery"));
        assert_eq!(r.get("0").and_then(|i| i.property("color")), Some("3"));
        assert_eq!(r.get("1").and_then(|i| i.property("actions")), Some("0"));
        let by_colour: Vec<&str> = r.with_property_value("color", "2").iter().map(|i| i.id()).collect();
        assert_eq!(by_colour, vec!["1"]);
    }

    #[test]
    fn the_entity_query_reads_its_subtitles_from_the_picker() {
        let q = EntityQuery::Suggested;
        let got = entities(&two_saves(), SHORTCUT, &q).unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].title, "Save Battery");
        assert_eq!(got[1].title, "Save Power");
    }

    #[test]
    fn the_shortcut_in_front_is_the_selected_annotation() {
        let mut ctx = OnscreenContext::new();
        assert_eq!(frontmost(&ctx), None);
        ctx.annotate(0, EntityRef { type_name: SHORTCUT.into(), id: "0".into() }, false).unwrap();
        assert_eq!(frontmost(&ctx), None, "shown but not in front");
        ctx.annotate(1, EntityRef { type_name: SHORTCUT.into(), id: "1".into() }, true).unwrap();
        assert_eq!(frontmost(&ctx), Some(1));
        assert!(ctx.set_visible(1, false));
        assert_eq!(frontmost(&ctx), None, "a hidden view is not on screen");
    }

    #[test]
    fn a_grid_shows_every_tile_and_an_editor_shows_one_in_front() {
        let grid = onscreen_context(&[(0, false), (1, false)]);
        assert_eq!(frontmost(&grid), None);
        assert_eq!(grid.view_annotations().len(), 2);
        let editor = onscreen_context(&[(1, true)]);
        assert_eq!(frontmost(&editor), Some(1));
    }

    #[test]
    fn with_no_parameter_the_shortcut_in_front_is_used() {
        let list = two_saves();
        let planner = Planner::new();
        let r = perform(&list, RUN, &Json::Obj(Vec::new()), Some(0), &planner).err().map(|e| e.message);
        assert_eq!(r, Some("Save Battery cannot run on this device yet".to_string()));
        let plan = perform(&list, PLAN, &Json::Obj(Vec::new()), Some(0), &planner).unwrap();
        assert_eq!(plan.dialog.as_deref(), Some("Save Battery runs 1 step."));
    }

    #[test]
    fn with_no_parameter_and_nothing_in_front_the_parameter_is_asked_for() {
        let list = two_saves();
        let err = perform(&list, PLAN, &Json::Obj(Vec::new()), None, &Planner::new()).err();
        assert_eq!(err.map(|e| e.message), Some("Shortcut is required".to_string()));
    }

    #[test]
    fn an_explicit_parameter_wins_over_the_shortcut_in_front() {
        let list = two_saves();
        let plan = perform(&list, PLAN, &obj(&[("shortcut", "Save Power")]), Some(0), &Planner::new());
        assert_eq!(plan.err().map(|e| e.message), Some("Save Power has no actions".to_string()));
    }

    #[test]
    fn the_planner_gives_the_same_text_as_planning_on_this_thread() {
        let s = two_saves()[0].clone();
        assert_eq!(Planner::new().plan(s.clone()), plan_text(&s));
        assert_eq!(Planner::default().plan(two_saves()[1].clone()), Err("has no actions".to_string()));
    }
}
