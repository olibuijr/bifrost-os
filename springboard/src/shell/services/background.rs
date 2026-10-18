//! Background App Refresh: the shell runs an app's deferred tasks while the app has no window open, as the reference
//! platform's task scheduler does. The app declares its task identifiers in `manifest.toml`
//! (`[background] tasks = ["com.x.refresh"]`), submits requests through the SDK (`bifrost_app::background_tasks`), and
//! this service runs them.
//!
//! Service methods (`background`, called by the app):
//! - `submit` `{identifier, kind: "refresh"|"processing", earliest?, network?, power?}` -> `{"accepted": true}`
//! - `cancel` `{identifier}` -> `{"cancelled": bool}`
//! - `completed` `{identifier}` -> `{"completed": bool}`: the task is done; the shell ends the app when it launched it
//!   for the task, and leaves an app the user had open running.
//!
//! Events the shell sends (`taskStarted` `{identifier, limit}`, `taskExpiring` `{identifier}`), see `bifrost_app::background_tasks`.
//!
//! Rules: a request is due when its time has come and the network and external power it needs are there (the
//! `Scheduler` model in `bifrost_core::background_tasks`). It runs only when Background App Refresh is on, the app is
//! not switched off, the machine is not in Low Power Mode, the app has no task running and the app has runs left in its
//! daily budget. One task runs per app at a time. A refresh may run for 30 s and a processing task for 10 minutes; the
//! app gets a warning 5 s before its time is up. The pending queue and the budget are saved to `background-tasks.txt`
//! under the shell's root so they survive a restart.
//!
//! Game Mode (`shell::gamemode`) pauses new starts while a game is in front full screen; a task already running still
//! ends on time. File indexing is not paused: no indexer runs in the shell (`ui::index_client` is a model only).
//!
//! Known gaps: a hidden launch does not run the notification or extension hooks that a Dock launch runs, and
//! a task started for an app the user opens in the meantime still ends the app when the task completes.
use super::{Event, Shell, SystemService};
use crate::shell::apphost;
use bifrost_core::background_tasks::{Conditions, Kind, Request, Scheduler};
use bifrost_core::json::Json;
use bifrost_core::power_supply::{self, PowerSource, SupplyKind};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Seconds a refresh task may run.
pub const REFRESH_LIMIT_SECS: u64 = 30;
/// Seconds a processing task may run.
pub const PROCESSING_LIMIT_SECS: u64 = 600;
/// The warning arrives this many seconds before the limit.
pub const WARNING_LEAD_SECS: u64 = 5;
/// Task runs an app may start in one UTC day.
pub const DAILY_BUDGET: u32 = 8;
/// A refused launch (the spawn failed) is retried after this many seconds.
pub const RETRY_SECS: u64 = 900;
/// Seconds between two looks at the queue.
pub const TICK_SECS: u64 = 5;
/// Pending requests one app may hold.
pub const MAX_PENDING_PER_APP: usize = 16;
/// Task identifiers one manifest may declare.
pub const MAX_TASKS_PER_MANIFEST: usize = 16;
/// Pref: the app ids whose refresh the user turned off, comma separated. The system-wide switch is the existing
/// `bg_refresh` option (Settings > General > Background App Refresh).
pub const PREF_OFF: &str = "background.refresh.off";

/// The app ids in the [`PREF_OFF`] pref value.
pub fn off_from_pref(value: Option<&str>) -> Vec<String> {
    value.unwrap_or("").split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

/// The [`PREF_OFF`] value for the ids.
pub fn off_to_pref(off: &[String]) -> String {
    off.join(",")
}
/// The file the queue is saved to, under the shell's root.
pub const STORE_FILE: &str = "background-tasks.txt";
/// Network interfaces (`operstate` per interface).
pub const NET_CLASS: &str = "/sys/class/net";
/// Event names, as `bifrost_app::background_tasks` reads them.
pub const EVENT_STARTED: &str = "taskStarted";
pub const EVENT_EXPIRING: &str = "taskExpiring";

/// Seconds a task of this kind may run.
pub fn limit_secs(kind: Kind) -> u64 {
    match kind {
        Kind::Refresh => REFRESH_LIMIT_SECS,
        Kind::Processing => PROCESSING_LIMIT_SECS,
    }
}

/// A task identifier: 1 to 128 ASCII letters, digits, `.`, `-` or `_`. Identifiers are stored one per line, so no
/// other character is allowed.
pub fn valid_identifier(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The task identifiers in a manifest's `[background]` table (`tasks = ["..."]`). Other tables are ignored, invalid
/// and repeated identifiers are dropped, and at most [`MAX_TASKS_PER_MANIFEST`] are kept.
pub fn parse_tasks(text: &str) -> Vec<String> {
    let mut in_table = false;
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && !line.contains('=') {
            in_table = line == "[background]";
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        if key.trim() != "tasks" {
            continue;
        }
        for item in value.trim().trim_start_matches('[').trim_end_matches(']').split(',') {
            let id = apphost::unquote(item.trim());
            if valid_identifier(&id) && !out.contains(&id) && out.len() < MAX_TASKS_PER_MANIFEST {
                out.push(id);
            }
        }
    }
    out
}

/// The runs an app has started today (UTC days of `now / 86400`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    day: u64,
    runs: u32,
}

fn day_of(now: u64) -> u64 {
    now / 86_400
}

impl Budget {
    pub fn from_parts(day: u64, runs: u32) -> Budget {
        Budget { day, runs }
    }

    /// The day and the run count, for saving.
    pub fn parts(&self) -> (u64, u32) {
        (self.day, self.runs)
    }

    /// Runs left today at `now`.
    pub fn remaining(&self, now: u64) -> u32 {
        let used = if self.day == day_of(now) { self.runs } else { 0 };
        DAILY_BUDGET.saturating_sub(used)
    }

    /// Counts one run at `now`, starting a new count on a new day.
    pub fn record(&mut self, now: u64) {
        let day = day_of(now);
        if self.day != day {
            self.day = day;
            self.runs = 0;
        }
        self.runs += 1;
    }
}

/// Whether the machine is on external power for a request that needs it. A machine with no battery is always on
/// external power; with a battery, a mains or USB supply must report `online`.
pub fn on_external_power(sources: &[PowerSource]) -> bool {
    let has_battery = sources.iter().any(|s| s.kind == SupplyKind::Battery);
    !has_battery || sources.iter().any(|s| matches!(s.kind, SupplyKind::Mains | SupplyKind::Usb) && s.online == Some(true))
}

/// Whether any interface other than loopback is up, from `(name, operstate)` pairs.
pub fn network_up(interfaces: &[(String, String)]) -> bool {
    interfaces.iter().any(|(name, state)| name != "lo" && state.trim() == "up")
}

/// [`network_up`] for the interfaces under `class_dir` (`/sys/class/net`). False when the directory cannot be read.
pub fn network_up_in(class_dir: &Path) -> bool {
    let Ok(dir) = std::fs::read_dir(class_dir) else { return false };
    let interfaces: Vec<(String, String)> = dir
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let state = std::fs::read_to_string(e.path().join("operstate")).ok()?;
            Some((name, state))
        })
        .collect();
    network_up(&interfaces)
}

/// Why a task may not start now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Disabled,
    AppOff,
    LowPower,
    Busy,
    BudgetSpent,
}

/// The switches and states a start depends on, besides the request's own conditions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Gate {
    pub enabled: bool,
    pub app_allowed: bool,
    pub low_power: bool,
    pub busy: bool,
}

/// Whether a task may start at `now`, checked in the order the user sees the switches.
pub fn may_start(gate: Gate, budget: &Budget, now: u64) -> Result<(), Refusal> {
    if !gate.enabled {
        return Err(Refusal::Disabled);
    }
    if !gate.app_allowed {
        return Err(Refusal::AppOff);
    }
    if gate.low_power {
        return Err(Refusal::LowPower);
    }
    if gate.busy {
        return Err(Refusal::Busy);
    }
    if budget.remaining(now) == 0 {
        return Err(Refusal::BudgetSpent);
    }
    Ok(())
}

/// A task the shell started and has not ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub identifier: String,
    pub kind: Kind,
    pub started: u64,
    /// True when the shell launched the app for this task (so the shell ends it afterwards).
    pub launched: bool,
    /// The warning has been sent.
    pub warned: bool,
}

/// Where a running task is against its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Running,
    Warn,
    Expired,
}

/// The phase of a task that started at `started`, at time `now`. The warning starts [`WARNING_LEAD_SECS`] before the
/// limit; the task has expired from the limit on.
pub fn phase(kind: Kind, started: u64, now: u64) -> Phase {
    let end = started + limit_secs(kind);
    if now >= end {
        Phase::Expired
    } else if now + WARNING_LEAD_SECS >= end {
        Phase::Warn
    } else {
        Phase::Running
    }
}

/// The queue, budget and run of one app.
#[derive(Debug, Default)]
pub struct AppTasks {
    pub scheduler: Scheduler,
    pub budget: Budget,
    pub run: Option<Run>,
}

/// What the shell does after a planning step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Start the app for the request (a hidden launch, or an event to the running app).
    Start { app: String, request: Request },
    /// Send the expiration warning.
    Warn { app: String, identifier: String },
    /// End the task: the time is up or the app completed it. `launched` says whether the shell started the app.
    Stop { app: String, identifier: String, launched: bool },
}

/// The settings a planning step reads, taken from the shell once per tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub enabled: bool,
    pub low_power: bool,
    /// Game Mode is on: no task starts until it leaves (a running task still ends on time).
    pub paused: bool,
    /// App ids whose refresh the user turned off.
    pub off: Vec<String>,
}

/// Plans one app's step at `now`. An app with a running task only gets its warning or its end; otherwise the first due
/// request starts when the gate allows it. Other due requests stay pending for the next step.
pub fn plan_app(app: &str, tasks: &mut AppTasks, now: u64, conditions: Conditions, settings: &Settings) -> Option<Effect> {
    if let Some(run) = tasks.run.as_mut() {
        match phase(run.kind, run.started, now) {
            Phase::Running => return None,
            Phase::Warn if run.warned => return None,
            Phase::Warn => {
                run.warned = true;
                return Some(Effect::Warn { app: app.to_string(), identifier: run.identifier.clone() });
            }
            Phase::Expired => {}
        }
    }
    if let Some(run) = tasks.run.take() {
        return Some(Effect::Stop { app: app.to_string(), identifier: run.identifier, launched: run.launched });
    }
    let mut due = tasks.scheduler.due(now, conditions);
    if due.is_empty() {
        return None;
    }
    let request = due.remove(0);
    for rest in due {
        let _ = tasks.scheduler.submit(rest);
    }
    let gate = Gate { enabled: settings.enabled && !settings.paused, app_allowed: !settings.off.iter().any(|a| a == app), low_power: settings.low_power, busy: false };
    match may_start(gate, &tasks.budget, now) {
        Ok(()) => {
            tasks.budget.record(now);
            tasks.run = Some(Run { identifier: request.identifier.clone(), kind: request.kind, started: now, launched: false, warned: false });
            Some(Effect::Start { app: app.to_string(), request })
        }
        Err(_) => {
            let _ = tasks.scheduler.submit(request);
            None
        }
    }
}

/// Persisted state: a `budget` line per app (`budget\tapp\tday\truns`) and a `req` line per pending request
/// (`req\tapp\tidentifier\tkind\tearliest|-\tnetwork\tpower`). Lines that do not parse are skipped.
pub fn render_store(apps: &BTreeMap<String, AppTasks>) -> String {
    let mut out = String::new();
    for (app, tasks) in apps {
        let (day, runs) = tasks.budget.parts();
        if day > 0 || runs > 0 {
            out.push_str(&format!("budget\t{app}\t{day}\t{runs}\n"));
        }
        for r in tasks.scheduler.pending() {
            let earliest = r.earliest_begin.map_or("-".to_string(), |v| v.to_string());
            out.push_str(&format!(
                "req\t{app}\t{}\t{}\t{earliest}\t{}\t{}\n",
                r.identifier,
                r.kind.wire(),
                r.requires_network as u8,
                r.requires_external_power as u8
            ));
        }
    }
    out
}

/// The state [`render_store`] wrote.
pub fn parse_store(text: &str) -> BTreeMap<String, AppTasks> {
    let mut apps: BTreeMap<String, AppTasks> = BTreeMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        match fields.as_slice() {
            ["budget", app, day, runs] => {
                if let (Ok(day), Ok(runs)) = (day.parse::<u64>(), runs.parse::<u32>()) {
                    apps.entry(app.to_string()).or_default().budget = Budget::from_parts(day, runs);
                }
            }
            ["req", app, id, kind, earliest, net, power] => {
                let Some(kind) = Kind::from_wire(kind) else { continue };
                if !valid_identifier(id) {
                    continue;
                }
                let earliest_begin = if *earliest == "-" {
                    None
                } else {
                    match earliest.parse::<u64>() {
                        Ok(v) => Some(v),
                        Err(_) => continue,
                    }
                };
                let request = Request { identifier: id.to_string(), kind, earliest_begin, requires_network: *net == "1", requires_external_power: *power == "1" };
                let _ = apps.entry(app.to_string()).or_default().scheduler.submit(request);
            }
            _ => {}
        }
    }
    apps
}

fn now_epoch() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn identifier_param(params: &Json) -> Result<String, String> {
    params.str_field("identifier").filter(|s| !s.is_empty()).ok_or_else(|| "missing identifier".to_string())
}

fn declared_tasks(app: &str) -> Vec<String> {
    apphost::installed().iter().find(|a| a.manifest.id == app).map(|a| a.manifest.background_tasks.clone()).unwrap_or_default()
}

fn obj(pairs: Vec<(&str, Json)>) -> Json {
    Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The `background` system service. Keeps the queues in memory and saves them to `path` when it is set.
pub struct BackgroundService {
    path: Option<PathBuf>,
    apps: BTreeMap<String, AppTasks>,
    loaded: bool,
    next_tick: u64,
    /// background agents this service started (and so stops when their switch goes off)
    agents: std::collections::BTreeSet<String>,
}

impl BackgroundService {
    /// A service that saves its queue to `path`.
    pub fn at(path: PathBuf) -> Self {
        BackgroundService { path: Some(path), apps: BTreeMap::new(), loaded: false, next_tick: 0, agents: Default::default() }
    }

    /// A service that keeps everything in memory (for the tests).
    #[cfg(test)]
    pub fn in_memory() -> Self {
        BackgroundService { path: None, apps: BTreeMap::new(), loaded: true, next_tick: 0, agents: Default::default() }
    }

    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if let Some(path) = &self.path {
            if let Ok(text) = std::fs::read_to_string(path) {
                self.apps = parse_store(&text);
            }
        }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, render_store(&self.apps)).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    /// Queues a request for `app`. `declared` is the app's manifest list: the identifier must be on it.
    pub fn submit(&mut self, app: &str, request: Request, declared: &[String]) -> Result<(), String> {
        if !declared.contains(&request.identifier) {
            return Err(format!("task {} is not declared in the manifest", request.identifier));
        }
        let tasks = self.apps.entry(app.to_string()).or_default();
        let already = tasks.scheduler.pending().iter().any(|r| r.identifier == request.identifier);
        if !already && tasks.scheduler.pending().len() >= MAX_PENDING_PER_APP {
            return Err("too many pending tasks".to_string());
        }
        tasks.scheduler.submit(request).map_err(|_| "empty identifier".to_string())
    }

    /// Removes a pending request. Returns whether it was pending.
    pub fn cancel(&mut self, app: &str, identifier: &str) -> bool {
        self.apps.get_mut(app).is_some_and(|t| t.scheduler.cancel(identifier))
    }

    /// The app reports that its running task `identifier` is done. Returns the stop when it was running.
    pub fn complete(&mut self, app: &str, identifier: &str) -> Option<Effect> {
        let tasks = self.apps.get_mut(app)?;
        if tasks.run.as_ref().is_some_and(|r| r.identifier == identifier) {
            let run = tasks.run.take()?;
            return Some(Effect::Stop { app: app.to_string(), identifier: run.identifier, launched: run.launched });
        }
        None
    }

    /// Plans every app's step at `now`.
    pub fn plan(&mut self, now: u64, conditions: Conditions, settings: &Settings) -> Vec<Effect> {
        self.apps.iter_mut().filter_map(|(app, tasks)| plan_app(app, tasks, now, conditions, settings)).collect()
    }

    /// Pending identifiers of an app, sorted (for the tests).
    #[cfg(test)]
    pub fn pending_ids(&self, app: &str) -> Vec<String> {
        self.apps.get(app).map(|t| t.scheduler.pending().iter().map(|r| r.identifier.clone()).collect()).unwrap_or_default()
    }

    /// The background-agent launcher: starts the agents whose "Allow in the background" switch is on, and stops the
    /// ones the user switched off. A refused launch is logged and not repeated until the switch is cycled.
    fn run_agents(&mut self, shell: &mut Shell) {
        use crate::shell::login_items_page as items;
        let agents = shell.login_ext_cache.get().background;
        let off = items::background_off_from_pref(shell.prefs.get(items::BACKGROUND_OFF_KEY));
        let plan = plan_agents(&agents, &off, &self.agents);
        for app in plan.stop {
            stop_app(shell, &app, true);
            self.agents.remove(&app);
        }
        for app in plan.start {
            // an agent is tried once while it is allowed, so a refused launch does not repeat every look
            if let Err(e) = start_app(shell, &app) {
                eprintln!("springboard: background agent {app}: {e}");
            }
            self.agents.insert(app);
        }
    }

    /// Carries out planned effects on the shell and returns the events for the apps.
    fn execute(&mut self, shell: &mut Shell, effects: Vec<Effect>, now: u64) -> Vec<Event> {
        let mut events = Vec::new();
        for effect in effects {
            match effect {
                Effect::Start { app, request } => match start_app(shell, &app) {
                    Ok(launched) => {
                        if let Some(run) = self.apps.get_mut(&app).and_then(|t| t.run.as_mut()) {
                            run.launched = launched;
                        }
                        let body = obj(vec![("identifier", Json::Str(request.identifier.clone())), ("limit", Json::Num(limit_secs(request.kind) as f64))]);
                        events.push(Event { app: Some(app), event: EVENT_STARTED.to_string(), body });
                    }
                    Err(message) => {
                        bifrost_core::log::shell_logger("background").error(format!("cannot start background task {} of {app}: {message}", request.identifier));
                        if let Some(tasks) = self.apps.get_mut(&app) {
                            tasks.run = None;
                            let mut retry = request.clone();
                            retry.earliest_begin = Some(now + RETRY_SECS);
                            let _ = tasks.scheduler.submit(retry);
                        }
                    }
                },
                Effect::Warn { app, identifier } => {
                    events.push(Event { app: Some(app), event: EVENT_EXPIRING.to_string(), body: obj(vec![("identifier", Json::Str(identifier))]) });
                }
                Effect::Stop { app, launched, .. } => stop_app(shell, &app, launched),
            }
        }
        events
    }
}

/// What the background-agent launcher does on one look.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AgentPlan {
    /// agents to start: allowed by their switch and not started yet
    pub start: Vec<String>,
    /// agents this launcher started that must now stop: their switch went off, or the app is gone
    pub stop: Vec<String>,
}

/// Plans the `[[background-agent]]` launches. `agents` are the apps that declare an agent, `off` the app ids whose
/// "Allow in the background" switch is off (`login.background.off`), `started` the agents already started. An agent
/// is started once while it is allowed; turning its switch off stops it, and turning it on again starts it again.
pub fn plan_agents(agents: &[crate::shell::login_items_page::BackgroundAgent], off: &[String], started: &std::collections::BTreeSet<String>) -> AgentPlan {
    use crate::shell::login_items_page::runnable_background_agents;
    let runnable: Vec<&str> = runnable_background_agents(agents, off).iter().map(|a| a.app_id.as_str()).collect();
    AgentPlan {
        start: runnable.iter().filter(|id| !started.contains(**id)).map(|id| id.to_string()).collect(),
        stop: started.iter().filter(|id| !runnable.contains(&id.as_str())).cloned().collect(),
    }
}

/// Starts the installed app `app` without a window, unless it runs already. Ok(true) when this call launched it.
fn start_app(shell: &mut Shell, app: &str) -> Result<bool, String> {
    let slot = apphost::slot_of(app).ok_or_else(|| "not installed".to_string())?;
    if shell.ext.len() <= slot {
        shell.ext.resize_with(slot + 1, || None);
    }
    if shell.ext[slot].as_ref().is_some_and(|h| h.running()) {
        return Ok(false);
    }
    let registered = apphost::installed();
    let installed = registered.get(slot).ok_or_else(|| "not installed".to_string())?;
    if apphost::is_known_incompatible(&installed.manifest.id, &installed.exec_path()) {
        return Err("the app speaks another SDK protocol".to_string());
    }
    let data = apphost::root().join("appdata").join(app);
    let cfg = shell.ext_cfg();
    let host = apphost::AppHost::spawn_with(app, &installed.exec_path(), &data, &apphost::root(), cfg, false).map_err(|e| e.to_string())?;
    shell.ext[slot] = Some(host);
    Ok(true)
}

/// Ends an app the shell launched for a task. An app the user had open keeps running.
fn stop_app(shell: &mut Shell, app: &str, launched: bool) {
    if !launched {
        return;
    }
    let Some(slot) = apphost::slot_of(app) else { return };
    if let Some(mut host) = shell.ext.get_mut(slot).and_then(Option::take) {
        host.kill();
    }
}

impl SystemService for BackgroundService {
    fn name(&self) -> &'static str {
        "background"
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, _t: f64) -> Result<Json, String> {
        self.ensure_loaded();
        match method {
            "submit" => {
                let request = Request::from_json(params)?;
                self.submit(app_id, request, &declared_tasks(app_id))?;
                self.save();
                Ok(obj(vec![("accepted", Json::Bool(true))]))
            }
            "cancel" => {
                let identifier = identifier_param(params)?;
                let cancelled = self.cancel(app_id, &identifier);
                self.save();
                Ok(obj(vec![("cancelled", Json::Bool(cancelled))]))
            }
            "completed" => {
                let identifier = identifier_param(params)?;
                let stop = self.complete(app_id, &identifier);
                let completed = stop.is_some();
                if let Some(effect) = stop {
                    let _ = self.execute(shell, vec![effect], now_epoch());
                    self.save();
                }
                Ok(obj(vec![("completed", Json::Bool(completed))]))
            }
            _ => Err(format!("no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
        self.ensure_loaded();
        let now = now_epoch();
        if now < self.next_tick {
            return Vec::new();
        }
        self.next_tick = now + TICK_SECS;
        self.run_agents(shell);
        if self.apps.is_empty() {
            return Vec::new();
        }
        let conditions = Conditions {
            network: network_up_in(Path::new(NET_CLASS)),
            external_power: on_external_power(&power_supply::sources_in(Path::new(super::power_service::SUPPLY_CLASS))),
        };
        let settings = Settings {
            enabled: shell.options().bg_refresh,
            low_power: shell.low_power(),
            paused: shell.game_mode_on(),
            off: off_from_pref(shell.prefs.get(PREF_OFF)),
        };
        let effects = self.plan(now, conditions, &settings);
        if effects.is_empty() {
            return Vec::new();
        }
        let events = self.execute(shell, effects, now);
        self.save();
        events
    }
}

#[cfg(test)]
mod agent_tests {
    use super::*;
    use crate::shell::login_items_page::BackgroundAgent;

    fn agents(ids: &[&str]) -> Vec<BackgroundAgent> {
        ids.iter().map(|id| BackgroundAgent { app_id: id.to_string(), app_name: id.to_string() }).collect()
    }

    fn set(ids: &[&str]) -> std::collections::BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn allowed_agents_start_once_and_switched_off_ones_do_not() {
        let off = vec!["is.b".to_string()];
        let plan = plan_agents(&agents(&["is.a", "is.b"]), &off, &set(&[]));
        assert_eq!(plan, AgentPlan { start: vec!["is.a".into()], stop: vec![] });
        let again = plan_agents(&agents(&["is.a", "is.b"]), &off, &set(&["is.a"]));
        assert_eq!(again, AgentPlan::default());
    }

    #[test]
    fn turning_the_switch_off_stops_the_agent_and_turning_it_on_starts_it_again() {
        let off = vec!["is.a".to_string()];
        let plan = plan_agents(&agents(&["is.a"]), &off, &set(&["is.a"]));
        assert_eq!(plan, AgentPlan { start: vec![], stop: vec!["is.a".into()] });
        let back = plan_agents(&agents(&["is.a"]), &[], &set(&[]));
        assert_eq!(back.start, ["is.a"]);
    }

    #[test]
    fn an_uninstalled_agent_is_stopped() {
        let plan = plan_agents(&agents(&[]), &[], &set(&["is.gone"]));
        assert_eq!(plan.stop, ["is.gone"]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONLINE: Conditions = Conditions { network: true, external_power: false };
    const PLUGGED: Conditions = Conditions { network: false, external_power: true };
    const BOTH: Conditions = Conditions { network: true, external_power: true };
    const NONE: Conditions = Conditions { network: false, external_power: false };

    fn settings() -> Settings {
        Settings { enabled: true, low_power: false, paused: false, off: Vec::new() }
    }

    fn declared(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    fn started_app(effect: Option<Effect>) -> String {
        match effect {
            Some(Effect::Start { app, .. }) => app,
            other => panic!("expected a start, got {other:?}"),
        }
    }

    #[test]
    fn a_manifest_declares_its_task_identifiers() {
        let text = "id = \"com.x.app\"\nname = \"X\"\nkind = \"app\"\nexec = \"bin/x\"\n[background]\ntasks = [\"com.x.app.refresh\", \"com.x.app.sync\"]\n";
        let m = apphost::Manifest::parse(text).unwrap();
        assert_eq!(m.background_tasks, vec!["com.x.app.refresh", "com.x.app.sync"]);
    }

    #[test]
    fn only_the_background_table_declares_tasks() {
        let text = "[background]\ntasks = [\"a.one\"]\n[[intent]]\ntasks = [\"a.two\"]\n[quick]\ntasks = [\"a.three\"]\n";
        assert_eq!(parse_tasks(text), vec!["a.one"]);
        assert!(parse_tasks("id = \"x\"\ntasks = [\"a.one\"]\n").is_empty());
    }

    #[test]
    fn bad_and_repeated_identifiers_are_dropped() {
        let text = "[background]\ntasks = [\"ok.one\", \"bad id\", \"../x\", \"\", \"ok.one\", \"ok-2\"]\n";
        assert_eq!(parse_tasks(text), vec!["ok.one", "ok-2"]);
        let long = format!("[background]\ntasks = [\"{}\"]\n", "a".repeat(129));
        assert!(parse_tasks(&long).is_empty());
    }

    #[test]
    fn the_task_limits_are_30_seconds_and_ten_minutes() {
        assert_eq!(limit_secs(Kind::Refresh), 30);
        assert_eq!(limit_secs(Kind::Processing), 600);
    }

    #[test]
    fn external_power_needs_mains_when_there_is_a_battery() {
        let battery = PowerSource {
            name: "BAT0".into(),
            kind: SupplyKind::Battery,
            percent: Some(50),
            state: bifrost_core::power_supply::PowerState::Discharging,
            online: None,
            minutes_to_empty: None,
            minutes_to_full: None,
        };
        let mains = |online| PowerSource { name: "AC".into(), kind: SupplyKind::Mains, percent: None, state: bifrost_core::power_supply::PowerState::Unknown, online, minutes_to_empty: None, minutes_to_full: None };
        assert!(on_external_power(&[]), "a machine with no battery is on external power");
        assert!(!on_external_power(&[battery.clone()]));
        assert!(!on_external_power(&[battery.clone(), mains(Some(false))]));
        assert!(on_external_power(&[battery, mains(Some(true))]));
    }

    #[test]
    fn the_network_is_up_when_a_real_interface_is_up() {
        let pairs = |v: &[(&str, &str)]| v.iter().map(|(n, s)| (n.to_string(), s.to_string())).collect::<Vec<_>>();
        assert!(network_up(&pairs(&[("wlan0", "up\n")])));
        assert!(!network_up(&pairs(&[("lo", "unknown\n"), ("wlan0", "down\n")])));
        assert!(!network_up(&[]));
    }

    #[test]
    fn a_gate_refuses_in_the_order_the_switches_are_shown() {
        let budget = Budget::default();
        let open = Gate { enabled: true, app_allowed: true, low_power: false, busy: false };
        assert_eq!(may_start(open, &budget, 0), Ok(()));
        assert_eq!(may_start(Gate { enabled: false, ..open }, &budget, 0), Err(Refusal::Disabled));
        assert_eq!(may_start(Gate { app_allowed: false, ..open }, &budget, 0), Err(Refusal::AppOff));
        assert_eq!(may_start(Gate { low_power: true, ..open }, &budget, 0), Err(Refusal::LowPower));
        assert_eq!(may_start(Gate { busy: true, ..open }, &budget, 0), Err(Refusal::Busy));
    }

    #[test]
    fn the_budget_allows_eight_runs_a_day_and_resets_on_the_next_day() {
        let mut b = Budget::default();
        let day = 10 * 86_400 + 100;
        for _ in 0..DAILY_BUDGET {
            assert!(b.remaining(day) > 0);
            b.record(day);
        }
        assert_eq!(b.remaining(day), 0);
        let gate = Gate { enabled: true, app_allowed: true, low_power: false, busy: false };
        assert_eq!(may_start(gate, &b, day + 10), Err(Refusal::BudgetSpent));
        assert_eq!(b.remaining(day + 86_400), DAILY_BUDGET);
        b.record(day + 86_400);
        assert_eq!(b.remaining(day + 86_400), DAILY_BUDGET - 1);
    }

    #[test]
    fn the_warning_and_the_end_follow_the_limit_with_a_fake_clock() {
        // a refresh started at 1000 is 30 s long: running until 1024, warned from 1025, expired at 1030.
        assert_eq!(phase(Kind::Refresh, 1000, 1024), Phase::Running);
        assert_eq!(phase(Kind::Refresh, 1000, 1025), Phase::Warn);
        assert_eq!(phase(Kind::Refresh, 1000, 1029), Phase::Warn);
        assert_eq!(phase(Kind::Refresh, 1000, 1030), Phase::Expired);
        // a processing task: warned from 1595, expired at 1600
        assert_eq!(phase(Kind::Processing, 1000, 1594), Phase::Running);
        assert_eq!(phase(Kind::Processing, 1000, 1595), Phase::Warn);
        assert_eq!(phase(Kind::Processing, 1000, 1600), Phase::Expired);
    }

    #[test]
    fn a_due_request_starts_once_warns_once_then_stops_at_the_limit() {
        let mut tasks = AppTasks::default();
        tasks.scheduler.submit(Request::refresh("com.x.refresh").earliest(2000)).unwrap();
        assert_eq!(plan_app("com.x", &mut tasks, 1999, ONLINE, &settings()), None, "not before its time");
        assert_eq!(started_app(plan_app("com.x", &mut tasks, 2000, ONLINE, &settings())), "com.x");
        assert_eq!(plan_app("com.x", &mut tasks, 2010, ONLINE, &settings()), None, "running");
        assert_eq!(plan_app("com.x", &mut tasks, 2025, ONLINE, &settings()), Some(Effect::Warn { app: "com.x".into(), identifier: "com.x.refresh".into() }));
        assert_eq!(plan_app("com.x", &mut tasks, 2026, ONLINE, &settings()), None, "warned once");
        assert_eq!(plan_app("com.x", &mut tasks, 2030, ONLINE, &settings()), Some(Effect::Stop { app: "com.x".into(), identifier: "com.x.refresh".into(), launched: false }));
        assert!(tasks.run.is_none());
    }

    #[test]
    fn a_network_request_waits_for_the_network_and_a_power_request_for_power() {
        let mut tasks = AppTasks::default();
        tasks.scheduler.submit(Request::processing("sync").network()).unwrap();
        tasks.scheduler.submit(Request::processing("index").external_power()).unwrap();
        assert_eq!(plan_app("a", &mut tasks, 10, PLUGGED, &settings()).map(|e| match e { Effect::Start { request, .. } => request.identifier, _ => String::new() }), Some("index".to_string()));
        // the run for "index" must end before "sync" can start
        let mut t2 = AppTasks::default();
        t2.scheduler.submit(Request::processing("sync").network()).unwrap();
        assert_eq!(plan_app("a", &mut t2, 10, PLUGGED, &settings()), None);
        assert_eq!(plan_app("a", &mut t2, 10, BOTH, &settings()).is_some(), true);
        assert_eq!(plan_app("a", &mut t2, 10, NONE, &settings()), None);
    }

    #[test]
    fn one_task_per_app_at_a_time() {
        let mut tasks = AppTasks::default();
        tasks.scheduler.submit(Request::refresh("first")).unwrap();
        tasks.scheduler.submit(Request::refresh("second")).unwrap();
        let effect = plan_app("a", &mut tasks, 100, NONE, &settings());
        assert!(matches!(effect, Some(Effect::Start { ref request, .. }) if request.identifier == "first"));
        assert_eq!(tasks.scheduler.pending().len(), 1, "the second stays pending");
        assert_eq!(plan_app("a", &mut tasks, 101, NONE, &settings()), None);
        assert_eq!(plan_app("a", &mut tasks, 200, NONE, &settings()), Some(Effect::Stop { app: "a".into(), identifier: "first".into(), launched: false }));
        let next = plan_app("a", &mut tasks, 200, NONE, &settings());
        assert!(matches!(next, Some(Effect::Start { ref request, .. }) if request.identifier == "second"));
    }

    #[test]
    fn low_power_and_the_switches_keep_the_request_pending() {
        let mut tasks = AppTasks::default();
        tasks.scheduler.submit(Request::refresh("r")).unwrap();
        let low = Settings { low_power: true, ..settings() };
        assert_eq!(plan_app("a", &mut tasks, 100, NONE, &low), None);
        assert_eq!(tasks.scheduler.pending().len(), 1);
        let off = Settings { off: vec!["a".into()], ..settings() };
        assert_eq!(plan_app("a", &mut tasks, 100, NONE, &off), None);
        let disabled = Settings { enabled: false, ..settings() };
        assert_eq!(plan_app("a", &mut tasks, 100, NONE, &disabled), None);
        assert_eq!(tasks.scheduler.pending().len(), 1);
        assert_eq!(started_app(plan_app("a", &mut tasks, 100, NONE, &settings())), "a");
    }

    #[test]
    fn game_mode_pauses_new_starts_only() {
        let mut tasks = AppTasks::default();
        tasks.scheduler.submit(Request::refresh("r")).unwrap();
        let paused = Settings { paused: true, ..settings() };
        assert_eq!(plan_app("a", &mut tasks, 100, NONE, &paused), None);
        assert_eq!(tasks.scheduler.pending().len(), 1, "the request waits for Game Mode to end");
        assert_eq!(started_app(plan_app("a", &mut tasks, 100, NONE, &settings())), "a");
    }

    #[test]
    fn an_exhausted_budget_keeps_the_request_for_the_next_day() {
        let mut tasks = AppTasks::default();
        let day = 5 * 86_400;
        tasks.budget = Budget::from_parts(day_of(day), DAILY_BUDGET);
        tasks.scheduler.submit(Request::refresh("r")).unwrap();
        assert_eq!(plan_app("a", &mut tasks, day + 1, NONE, &settings()), None);
        assert_eq!(tasks.scheduler.pending().len(), 1);
        assert_eq!(started_app(plan_app("a", &mut tasks, day + 86_400, NONE, &settings())), "a");
    }

    #[test]
    fn submit_refuses_an_undeclared_identifier_and_too_many_requests() {
        let mut svc = BackgroundService::in_memory();
        let list = declared(&["com.x.refresh"]);
        assert!(svc.submit("com.x", Request::refresh("com.x.other"), &list).is_err());
        assert!(svc.submit("com.x", Request::refresh("com.x.refresh"), &list).is_ok());
        // a replacement of a pending identifier does not count against the limit
        assert!(svc.submit("com.x", Request::processing("com.x.refresh"), &list).is_ok());
        assert_eq!(svc.pending_ids("com.x"), vec!["com.x.refresh"]);
        let many = declared(&(0..MAX_PENDING_PER_APP + 1).map(|i| format!("t{i}")).collect::<Vec<_>>().iter().map(String::as_str).collect::<Vec<_>>());
        let mut full = BackgroundService::in_memory();
        let mut last = Ok(());
        for id in &many {
            last = full.submit("b", Request::refresh(id), &many);
        }
        assert_eq!(last, Err("too many pending tasks".to_string()));
    }

    #[test]
    fn completing_the_running_task_stops_it_once() {
        let mut svc = BackgroundService::in_memory();
        svc.submit("a", Request::refresh("r"), &declared(&["r"])).unwrap();
        let tasks = svc.apps.get_mut("a").unwrap();
        assert!(matches!(plan_app("a", tasks, 100, NONE, &settings()), Some(Effect::Start { .. })));
        assert_eq!(svc.complete("a", "other"), None);
        assert_eq!(svc.complete("a", "r"), Some(Effect::Stop { app: "a".into(), identifier: "r".into(), launched: false }));
        assert_eq!(svc.complete("a", "r"), None);
    }

    #[test]
    fn the_queue_and_budget_round_trip_through_the_store_text() {
        let mut apps: BTreeMap<String, AppTasks> = BTreeMap::new();
        let t = apps.entry("com.x".to_string()).or_default();
        t.scheduler.submit(Request::processing("com.x.sync").earliest(1_700_000_000).network()).unwrap();
        t.scheduler.submit(Request::refresh("com.x.refresh").external_power()).unwrap();
        t.budget = Budget::from_parts(19_000, 3);
        let text = render_store(&apps);
        let back = parse_store(&text);
        let bt = &back["com.x"];
        assert_eq!(bt.budget.parts(), (19_000, 3));
        let pending = bt.scheduler.pending();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].identifier, "com.x.refresh");
        assert!(pending[0].requires_external_power);
        assert_eq!(pending[1].earliest_begin, Some(1_700_000_000));
        assert!(pending[1].requires_network);
    }

    #[test]
    fn broken_store_lines_are_skipped() {
        let text = "req\ta\tgood\trefresh\t-\t0\t0\nreq\ta\tbad id\trefresh\t-\t0\t0\nreq\ta\tx\thourly\t-\t0\t0\nreq\ta\ty\trefresh\tsoon\t0\t0\nbudget\ta\tnope\t1\njunk\n";
        let back = parse_store(text);
        assert_eq!(back["a"].scheduler.pending().len(), 1);
        assert_eq!(back["a"].budget.parts(), (0, 0));
    }

    #[test]
    fn the_queue_is_saved_to_disk_and_read_back() {
        let dir = std::env::temp_dir().join(format!("bifrost-bg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(STORE_FILE);
        {
            let mut svc = BackgroundService::at(path.clone());
            svc.ensure_loaded();
            svc.submit("com.x", Request::refresh("com.x.refresh"), &declared(&["com.x.refresh"])).unwrap();
            svc.save();
        }
        let mut again = BackgroundService::at(path.clone());
        again.ensure_loaded();
        assert_eq!(again.pending_ids("com.x"), vec!["com.x.refresh"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
