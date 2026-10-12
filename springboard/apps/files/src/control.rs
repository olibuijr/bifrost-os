//! The app's control-socket methods (`files.*`, `bifrost pc control`, the MCP `files` tool).
//!
//! Two layers: `parse` turns a method and its params into a `Req` (pure, unit-tested below), and
//! `Files::ctl_rpc` runs the request against the window. Calls that wait for a listing, a search or a copy, move or
//! trash job return `RpcError::pending(token, ..)` and are completed from `Files::tick` (`ctl_tick`,
//! `ctl_job_done`) with `sdk::complete`, as the Browser does. Undo covers what an agent did through these
//! methods (rename, move, copy, new folder); Edit > Undo in the window runs the same stack.
use crate::*;
use bifrost_app::control::{self as sdk, RpcError, INVALID_PARAMS, METHOD_NOT_FOUND};
use bifrost_core::json::Json;
use std::sync::Mutex;

/// Server error: another job is running.
pub(crate) const BUSY: i64 = -32003;
/// Server error: nothing at that path or nothing to undo.
pub(crate) const NOT_FOUND: i64 = -32004;

const DEFAULT_TIMEOUT_MS: u64 = 10_000;
const LIST_LIMIT: usize = 500;

pub(crate) const METHODS: [&str; 15] = [
    "files.state", "files.list", "files.select", "files.open", "files.navigate", "files.set_view", "files.new_folder", "files.rename",
    "files.move", "files.copy", "files.trash", "files.undo", "files.search", "files.get_info", "files.tags",
];

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Select {
    All,
    None,
    Names(Vec<String>),
    Paths(Vec<PathBuf>),
    Indices(Vec<usize>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Nav {
    Path(PathBuf),
    Back,
    Forward,
    Up,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum View {
    Icon,
    List,
    Columns,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TagEdit {
    pub set: Option<Vec<Tag>>,
    pub add: Vec<Tag>,
    pub remove: Vec<Tag>,
    pub toggle: Vec<Tag>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Req {
    State,
    List { path: Option<PathBuf>, limit: usize, hidden: Option<bool>, sort: Option<Sort> },
    Select { what: Select, add: bool },
    Open { path: Option<PathBuf>, index: Option<usize> },
    Navigate { to: Nav, timeout_ms: u64 },
    SetView { view: Option<View>, sort: Option<Sort>, hidden: Option<bool>, sidebar: Option<bool>, status_bar: Option<bool> },
    NewFolder { name: Option<String>, path: Option<PathBuf> },
    Rename { path: Option<PathBuf>, name: String },
    Transfer { op: Op, paths: Vec<PathBuf>, dest: PathBuf, timeout_ms: u64 },
    Trash { paths: Vec<PathBuf>, timeout_ms: u64 },
    Undo,
    Search { query: String, timeout_ms: u64 },
    Info { path: Option<PathBuf> },
    Tags { paths: Vec<PathBuf>, edit: TagEdit },
}

fn invalid(msg: impl Into<String>) -> RpcError {
    RpcError::new(INVALID_PARAMS, msg)
}

pub(crate) fn obj(fields: Vec<(&str, Json)>) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}
fn text(s: &str) -> Json {
    Json::Str(s.to_string())
}
fn num(n: f64) -> Json {
    Json::Num(n)
}
fn path_json(p: &Path) -> Json {
    Json::Str(p.to_string_lossy().into_owned())
}

fn string_of(params: &Json, key: &str) -> Result<Option<String>, RpcError> {
    match params.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Str(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid(format!("{key} must be a string"))),
    }
}
fn path_of(params: &Json, key: &str) -> Result<Option<PathBuf>, RpcError> {
    Ok(string_of(params, key)?.map(PathBuf::from))
}
fn bool_of(params: &Json, key: &str) -> Result<Option<bool>, RpcError> {
    match params.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(invalid(format!("{key} must be a boolean"))),
    }
}
fn flag(params: &Json, key: &str) -> Result<bool, RpcError> {
    Ok(bool_of(params, key)?.unwrap_or(false))
}
fn strings_of(params: &Json, key: &str) -> Result<Option<Vec<String>>, RpcError> {
    match params.get(key) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Arr(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| invalid(format!("{key} must be an array of strings"))))
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        Some(_) => Err(invalid(format!("{key} must be an array of strings"))),
    }
}
fn paths_of(params: &Json, key: &str) -> Result<Vec<PathBuf>, RpcError> {
    Ok(strings_of(params, key)?.unwrap_or_default().into_iter().map(PathBuf::from).collect())
}
fn timeout_of(params: &Json) -> Result<u64, RpcError> {
    match params.get("timeout_ms") {
        None | Some(Json::Null) => Ok(DEFAULT_TIMEOUT_MS),
        Some(Json::Num(n)) if *n >= 0. => Ok((*n as u64).clamp(100, sdk::MAX_TIMEOUT_MS)),
        Some(_) => Err(invalid("timeout_ms must be a non-negative number")),
    }
}
fn sort_of(params: &Json) -> Result<Option<Sort>, RpcError> {
    let Some(name) = string_of(params, "sort")? else { return Ok(None) };
    Sort::ALL
        .into_iter()
        .find(|s| s.label().eq_ignore_ascii_case(&name))
        .map(Some)
        .ok_or_else(|| invalid(format!("sort must be one of name, date, size, kind (got {name:?})")))
}
fn tag_named(name: &str) -> Result<Tag, RpcError> {
    Tag::ALL
        .into_iter()
        .find(|t| t.label().eq_ignore_ascii_case(name))
        .ok_or_else(|| invalid(format!("unknown tag colour {name:?}; one of red orange yellow green blue purple gray")))
}
fn tags_of(params: &Json, key: &str) -> Result<Option<Vec<Tag>>, RpcError> {
    strings_of(params, key)?.map(|names| names.iter().map(|n| tag_named(n)).collect()).transpose()
}

/// Validates one `files.*` call. Pure: no window state is touched.
pub(crate) fn parse(method: &str, params: &Json) -> Result<Req, RpcError> {
    let name = method.strip_prefix("files.").unwrap_or(method);
    Ok(match name {
        "state" => Req::State,
        "list" => Req::List {
            path: path_of(params, "path")?,
            limit: match params.get("limit") {
                None | Some(Json::Null) => LIST_LIMIT,
                Some(Json::Num(n)) if *n >= 1. => *n as usize,
                Some(_) => return Err(invalid("limit must be a positive number")),
            },
            hidden: bool_of(params, "hidden")?,
            sort: sort_of(params)?,
        },
        "select" => {
            let what = if flag(params, "all")? {
                Select::All
            } else if flag(params, "none")? {
                Select::None
            } else if let Some(n) = strings_of(params, "names")? {
                Select::Names(n)
            } else if params.get("paths").is_some() {
                Select::Paths(paths_of(params, "paths")?)
            } else if let Some(Json::Arr(items)) = params.get("indices") {
                Select::Indices(
                    items.iter().map(|v| v.number().filter(|n| *n >= 0.).map(|n| n as usize).ok_or_else(|| invalid("indices must be non-negative integers"))).collect::<Result<_, _>>()?,
                )
            } else {
                return Err(invalid("select needs one of names, paths, indices, all or none"));
            };
            Req::Select { what, add: flag(params, "add")? }
        }
        "open" => {
            let index = match params.get("index") {
                None | Some(Json::Null) => None,
                Some(Json::Num(n)) if *n >= 0. => Some(*n as usize),
                Some(_) => return Err(invalid("index must be a non-negative integer")),
            };
            let path = path_of(params, "path")?;
            if path.is_none() && index.is_none() {
                return Err(invalid("open needs path or index"));
            }
            Req::Open { path, index }
        }
        "navigate" => {
            let to = if flag(params, "back")? {
                Nav::Back
            } else if flag(params, "forward")? {
                Nav::Forward
            } else if flag(params, "up")? {
                Nav::Up
            } else {
                Nav::Path(path_of(params, "path")?.ok_or_else(|| invalid("navigate needs path, back, forward or up"))?)
            };
            Req::Navigate { to, timeout_ms: timeout_of(params)? }
        }
        "set_view" => {
            let view = match string_of(params, "view")?.as_deref() {
                None => None,
                Some("icon") | Some("icons") | Some("grid") => Some(View::Icon),
                Some("list") => Some(View::List),
                Some("columns") | Some("column") => Some(View::Columns),
                Some(v) => return Err(invalid(format!("view must be icon, list or columns (got {v:?})"))),
            };
            let req = Req::SetView {
                view,
                sort: sort_of(params)?,
                hidden: bool_of(params, "hidden")?,
                sidebar: bool_of(params, "sidebar")?,
                status_bar: bool_of(params, "status_bar")?,
            };
            if matches!(req, Req::SetView { view: None, sort: None, hidden: None, sidebar: None, status_bar: None }) {
                return Err(invalid("set_view needs one of view, sort, hidden, sidebar, status_bar"));
            }
            req
        }
        "new_folder" => Req::NewFolder { name: string_of(params, "name")?, path: path_of(params, "path")? },
        "rename" => Req::Rename {
            path: path_of(params, "path")?,
            name: string_of(params, "name")?.ok_or_else(|| invalid("rename needs name"))?,
        },
        "move" | "copy" => Req::Transfer {
            op: if name == "move" { Op::Move } else { Op::Copy },
            paths: paths_of(params, "paths")?,
            dest: path_of(params, "dest")?.ok_or_else(|| invalid(format!("{name} needs dest")))?,
            timeout_ms: timeout_of(params)?,
        },
        "trash" => Req::Trash { paths: paths_of(params, "paths")?, timeout_ms: timeout_of(params)? },
        "undo" => Req::Undo,
        "search" => Req::Search { query: string_of(params, "query")?.ok_or_else(|| invalid("search needs query"))?, timeout_ms: timeout_of(params)? },
        "get_info" => Req::Info { path: path_of(params, "path")? },
        "tags" => {
            let mut paths = paths_of(params, "paths")?;
            if let Some(p) = path_of(params, "path")? {
                paths.insert(0, p);
            }
            Req::Tags {
                paths,
                edit: TagEdit {
                    set: tags_of(params, "set")?,
                    add: match params.get("add") {
                        Some(Json::Arr(_)) => tags_of(params, "add")?.unwrap_or_default(),
                        _ => vec![],
                    },
                    remove: tags_of(params, "remove")?.unwrap_or_default(),
                    toggle: tags_of(params, "toggle")?.unwrap_or_default(),
                },
            }
        }
        _ => return Err(RpcError::new(METHOD_NOT_FOUND, format!("no method {method}"))),
    })
}

/// What `files.undo` reverts.
enum Undo {
    Rename { from: PathBuf, to: PathBuf },
    Move(Vec<(PathBuf, PathBuf)>),
    Copy(Vec<PathBuf>),
    NewFolder(PathBuf),
}

/// An async copy, move or trash waiting for its job.
struct JobCtl {
    token: u64,
    kind: JobKind,
    sources: Vec<PathBuf>,
    dest: PathBuf,
}

static UNDO: Mutex<Vec<Undo>> = Mutex::new(Vec::new());
static NAV: Mutex<Option<u64>> = Mutex::new(None);
static SEARCH: Mutex<Option<u64>> = Mutex::new(None);
static JOB: Mutex<Option<JobCtl>> = Mutex::new(None);
const UNDO_DEPTH: usize = 32;

fn remember(op: Undo) {
    if let Ok(mut u) = UNDO.lock() {
        if u.len() >= UNDO_DEPTH {
            u.remove(0);
        }
        u.push(op);
    }
}

/// Stores `token` in `slot`; a call it replaces is answered with a "superseded" error.
fn arm(slot: &Mutex<Option<u64>>, token: u64) {
    if let Ok(mut s) = slot.lock() {
        if let Some(old) = s.replace(token) {
            sdk::complete(old, Err(RpcError::new(NOT_FOUND, "superseded by a newer call")));
        }
    }
}
fn disarm(slot: &Mutex<Option<u64>>) -> Option<u64> {
    slot.lock().ok().and_then(|mut s| s.take())
}

fn kind_name(sort: Sort) -> String {
    sort.label().to_lowercase()
}

impl Files {
    pub(crate) fn ctl_methods(&self) -> Vec<&'static str> {
        METHODS.to_vec()
    }

    fn ctl_abs(&self, p: &Path) -> PathBuf {
        if p.is_absolute() { p.to_path_buf() } else { self.path.join(p) }
    }

    fn ctl_view(&self) -> &'static str {
        if self.columns { "columns" } else if self.grid { "icon" } else { "list" }
    }

    fn entry_json(&self, e: &Entry) -> Json {
        obj(vec![
            ("name", text(&e.name)),
            ("path", path_json(&e.path)),
            ("is_dir", Json::Bool(e.is_dir)),
            ("kind", text(fs::kind_description(&e.name, e.is_dir, e.kind))),
            ("size", num(e.size as f64)),
            ("mtime", num(e.mtime as f64)),
            ("items", e.items.map_or(Json::Null, |n| num(n as f64))),
            ("tags", Json::Arr(self.tag_index.tags(&e.path).into_iter().map(|t| text(t.label())).collect())),
        ])
    }

    fn selection_json(&self) -> Json {
        Json::Arr(
            self.selection
                .iter()
                .filter_map(|&i| self.rows.get(i).map(|e| obj(vec![("index", num(i as f64)), ("name", text(&e.name)), ("path", path_json(&e.path))])))
                .collect(),
        )
    }

    fn state_json(&self) -> Json {
        obj(vec![
            ("version", text(env!("CARGO_PKG_VERSION"))),
            ("path", path_json(&self.path)),
            ("view", text(self.ctl_view())),
            ("sort", text(&kind_name(self.sort))),
            ("hidden", Json::Bool(self.show_hidden)),
            ("sidebar", Json::Bool(!self.sidebar_hidden)),
            ("status_bar", Json::Bool(self.status_bar)),
            ("loading", Json::Bool(self.loading)),
            ("error", self.error.as_deref().map_or(Json::Null, text)),
            ("notice", self.notice.as_deref().map_or(Json::Null, text)),
            ("rows", num(self.visible().len() as f64)),
            ("selection", self.selection_json()),
            ("search", if self.query.trim().is_empty() { Json::Null } else { text(self.query.trim()) }),
            ("searching", Json::Bool(self.search.is_some())),
            ("busy", self.work.as_ref().map_or(Json::Null, |w| text(&w.title))),
            ("can_back", Json::Bool(self.at > 0)),
            ("can_forward", Json::Bool(self.at + 1 < self.history.len())),
            ("clipboard", self.clip.as_ref().map_or(Json::Null, |(p, cut)| obj(vec![("items", num(p.len() as f64)), ("cut", Json::Bool(*cut))]))),
        ])
    }

    fn rows_json(&self, limit: usize) -> Json {
        Json::Arr(self.visible().into_iter().take(limit).filter_map(|i| self.rows.get(i)).map(|e| self.entry_json(e)).collect())
    }

    /// The paths a call names, else the current selection.
    fn ctl_targets(&self, given: Vec<PathBuf>) -> Result<Vec<PathBuf>, RpcError> {
        let paths: Vec<PathBuf> = if given.is_empty() { self.selected_paths() } else { given.iter().map(|p| self.ctl_abs(p)).collect() };
        if paths.is_empty() {
            return Err(invalid("no paths given and nothing is selected"));
        }
        if let Some(missing) = paths.iter().find(|p| std::fs::symlink_metadata(p).is_err()) {
            return Err(RpcError::new(NOT_FOUND, format!("no such file: {}", missing.display())));
        }
        Ok(paths)
    }

    fn ctl_row_of(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|e| e.path == path)
    }

    fn ctl_busy(&self) -> Result<(), RpcError> {
        match &self.work {
            Some(w) => Err(RpcError::new(BUSY, format!("busy: {}", w.title))),
            None => Ok(()),
        }
    }

    /// Answers one `files.*` call. `Err` with `RpcError::pending` means "answer later".
    pub(crate) fn ctl_rpc(&mut self, method: &str, params: &Json) -> Result<Json, RpcError> {
        match parse(method, params)? {
            Req::State => Ok(self.state_json()),
            Req::List { path, limit, hidden, sort } => {
                let Some(path) = path else {
                    return Ok(obj(vec![("path", path_json(&self.path)), ("entries", self.rows_json(limit))]));
                };
                let dir = self.ctl_abs(&path);
                let mut list = fs::list(&dir).map_err(|e| RpcError::new(NOT_FOUND, format!("{}: {e}", dir.display())))?;
                if !hidden.unwrap_or(self.show_hidden) {
                    list.retain(|e| !e.name.starts_with('.'));
                }
                fs::sort(&mut list, sort.unwrap_or(self.sort));
                list.sort_by_key(|e| !e.is_dir);
                let total = list.len();
                Ok(obj(vec![
                    ("path", path_json(&dir)),
                    ("total", num(total as f64)),
                    ("entries", Json::Arr(list.iter().take(limit).map(|e| self.entry_json(e)).collect())),
                ]))
            }
            Req::Select { what, add } => {
                let visible = self.visible();
                let picked: Vec<usize> = match what {
                    Select::All => visible,
                    Select::None => vec![],
                    Select::Indices(ix) => {
                        if let Some(bad) = ix.iter().find(|i| **i >= self.rows.len()) {
                            return Err(invalid(format!("index {bad} is out of range ({} rows)", self.rows.len())));
                        }
                        ix
                    }
                    Select::Names(names) => {
                        let mut out = vec![];
                        for n in &names {
                            let at = visible.iter().copied().find(|&i| self.rows[i].name == *n).ok_or_else(|| RpcError::new(NOT_FOUND, format!("no row named {n:?} in {}", self.path.display())))?;
                            out.push(at);
                        }
                        out
                    }
                    Select::Paths(paths) => {
                        let mut out = vec![];
                        for p in &paths {
                            let p = self.ctl_abs(p);
                            out.push(self.ctl_row_of(&p).ok_or_else(|| RpcError::new(NOT_FOUND, format!("{} is not a row of the open folder", p.display())))?);
                        }
                        out
                    }
                };
                if !add {
                    self.selection.clear();
                }
                for i in picked {
                    if !self.selection.contains(&i) {
                        self.selection.push(i);
                    }
                }
                self.revision += 1;
                Ok(obj(vec![("selection", self.selection_json())]))
            }
            Req::Open { path, index } => {
                let i = match (index, path.as_ref().map(|p| self.ctl_abs(p))) {
                    (Some(i), _) if i < self.rows.len() => Some(i),
                    (Some(i), _) => return Err(invalid(format!("index {i} is out of range ({} rows)", self.rows.len()))),
                    (None, Some(p)) => self.ctl_row_of(&p),
                    (None, None) => None,
                };
                if let Some(i) = i {
                    let is_dir = self.rows[i].is_dir;
                    let path = self.rows[i].path.clone();
                    self.open(i);
                    return Ok(obj(vec![("opened", path_json(&path)), ("is_dir", Json::Bool(is_dir))]));
                }
                let p = self.ctl_abs(&path.unwrap_or_default());
                if p.is_dir() {
                    self.navigate(p.clone(), true);
                    Ok(obj(vec![("opened", path_json(&p)), ("is_dir", Json::Bool(true))]))
                } else {
                    Err(RpcError::new(NOT_FOUND, format!("{} is not in the open folder", p.display())))
                }
            }
            Req::Navigate { to, timeout_ms } => {
                match to {
                    Nav::Back if self.at == 0 => return Err(RpcError::new(NOT_FOUND, "no earlier folder in the history")),
                    Nav::Forward if self.at + 1 >= self.history.len() => return Err(RpcError::new(NOT_FOUND, "no later folder in the history")),
                    Nav::Up if self.path.parent().is_none() => return Err(RpcError::new(NOT_FOUND, "already at the top")),
                    Nav::Back => self.go_back(),
                    Nav::Forward => self.go_forward(),
                    Nav::Up => self.go_up(),
                    Nav::Path(p) => {
                        let p = self.ctl_abs(&p);
                        let virtual_folder = p == Path::new(fs::RECENTS) || p.starts_with(TAGGED) || smart::folder_index(&p).is_some();
                        if !virtual_folder && !p.is_dir() {
                            return Err(RpcError::new(NOT_FOUND, format!("not a folder: {}", p.display())));
                        }
                        self.navigate(p, true);
                    }
                }
                let token = sdk::new_token();
                arm(&NAV, token);
                Err(RpcError::pending(token, timeout_ms + 1500))
            }
            Req::SetView { view, sort, hidden, sidebar, status_bar } => {
                if let Some(v) = view {
                    match v {
                        View::Icon => self.set_view(true, false),
                        View::List => self.set_view(false, false),
                        View::Columns => self.set_view(false, true),
                    }
                }
                if let Some(s) = sort {
                    self.sort = s;
                    self.sort_rows();
                }
                if let Some(h) = hidden.filter(|h| *h != self.show_hidden) {
                    let _ = h;
                    self.toggle_hidden();
                }
                if let Some(s) = sidebar.filter(|s| *s == self.sidebar_hidden) {
                    let _ = s;
                    self.toggle_sidebar();
                }
                if let Some(b) = status_bar.filter(|b| *b != self.status_bar) {
                    let _ = b;
                    self.toggle_status_bar();
                }
                self.revision += 1;
                Ok(self.state_json())
            }
            Req::NewFolder { name, path } => {
                let dir = path.map(|p| self.ctl_abs(&p)).unwrap_or_else(|| self.path.clone());
                if !dir.is_dir() || self.in_bin() && dir == self.path {
                    return Err(invalid(format!("cannot create a folder in {}", dir.display())));
                }
                let name = name.unwrap_or_else(|| "New Folder".into());
                if !fs::valid_name(&name) {
                    return Err(invalid(format!("not a valid folder name: {name:?}")));
                }
                let unique = fs::unique_name(&dir, &name);
                let unique = unique.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(name);
                let made = fs::new_folder(&dir, &unique).map_err(|e| RpcError::new(INVALID_PARAMS, format!("could not create the folder: {e}")))?;
                remember(Undo::NewFolder(made.clone()));
                if dir == self.path {
                    self.navigate(dir, false);
                }
                Ok(obj(vec![("created", path_json(&made))]))
            }
            Req::Rename { path, name } => {
                let from = match path {
                    Some(p) => self.ctl_abs(&p),
                    None if self.selection.len() == 1 => self.selected_paths().remove(0),
                    None => return Err(invalid("rename needs path, or exactly one selected item")),
                };
                if !fs::valid_name(&name) {
                    return Err(invalid(format!("not a valid name: {name:?}")));
                }
                let to = fs::rename(&from, &name).map_err(|e| RpcError::new(INVALID_PARAMS, format!("could not rename: {e}")))?;
                self.with_tags(|index| index.moved(&from, &to));
                remember(Undo::Rename { from: from.clone(), to: to.clone() });
                let dir = self.path.clone();
                self.navigate(dir, false);
                Ok(obj(vec![("from", path_json(&from)), ("to", path_json(&to))]))
            }
            Req::Transfer { op, paths, dest, timeout_ms } => {
                self.ctl_busy()?;
                let paths = self.ctl_targets(paths)?;
                let dest = self.ctl_abs(&dest);
                if !dest.is_dir() {
                    return Err(RpcError::new(NOT_FOUND, format!("not a folder: {}", dest.display())));
                }
                let (kind, title) = match op {
                    Op::Copy => (JobKind::Transfer(Op::Copy), "Copying"),
                    Op::Move => (JobKind::Transfer(Op::Move), "Moving"),
                };
                self.ctl_start(kind, title, paths, dest, timeout_ms)
            }
            Req::Trash { paths, timeout_ms } => {
                self.ctl_busy()?;
                if self.in_bin() {
                    return Err(invalid("the open folder is Recently Deleted; items there are already trashed"));
                }
                let paths = self.ctl_targets(paths)?;
                let dest = self.path.clone();
                self.ctl_start(JobKind::Trash, "Moving to the Trash", paths, dest, timeout_ms)
            }
            Req::Undo => self.ctl_undo(),
            Req::Search { query, timeout_ms } => {
                self.query.clear();
                self.query.push_str(&query);
                self.searching = !query.trim().is_empty();
                self.query_changed();
                self.update_controls();
                if self.search.is_none() {
                    return Ok(self.search_json());
                }
                let token = sdk::new_token();
                arm(&SEARCH, token);
                Err(RpcError::pending(token, timeout_ms + 1500))
            }
            Req::Info { path } => {
                let entry = match path {
                    Some(p) => {
                        let p = self.ctl_abs(&p);
                        let found = self.rows.iter().find(|e| e.path == p).cloned();
                        let parent = p.parent().map(Path::to_path_buf).unwrap_or_default();
                        found
                            .or_else(|| fs::list(&parent).ok().and_then(|l| l.into_iter().find(|e| e.path == p)))
                            .ok_or_else(|| RpcError::new(NOT_FOUND, format!("no such file: {}", p.display())))?
                    }
                    None => self.focus_entry().cloned().ok_or_else(|| invalid("get_info needs path or a focused item"))?,
                };
                let wall = Wall::sample(bifrost_app::proto::sys::now_ms());
                let summary = entry.is_dir.then(|| {
                    let deadline = Instant::now() + INFO_BUDGET;
                    springboard::files::info::summarize(&entry.path, &mut || Instant::now() < deadline)
                });
                let tags = TagIndex::load(&sidecar_path(&self.home));
                let created = springboard::files::info::created_at(&entry.path);
                let rows = springboard::files::info::rows(&entry, summary, created, &tags, wall.unix, wall.offset);
                Ok(obj(vec![
                    ("entry", self.entry_json(&entry)),
                    ("rows", Json::Arr(rows.into_iter().map(|(k, v)| obj(vec![("label", text(k)), ("value", text(&v))])).collect())),
                ]))
            }
            Req::Tags { paths, edit } => {
                let paths = self.ctl_targets(paths)?;
                let changes = edit.set.is_some() || !edit.add.is_empty() || !edit.remove.is_empty() || !edit.toggle.is_empty();
                if changes {
                    self.with_tags(|index| {
                        for p in &paths {
                            if let Some(set) = &edit.set {
                                index.clear(p);
                                for t in set {
                                    if !index.has(p, *t) {
                                        index.toggle(p, *t);
                                    }
                                }
                            }
                            for t in &edit.add {
                                if !index.has(p, *t) {
                                    index.toggle(p, *t);
                                }
                            }
                            for t in &edit.remove {
                                if index.has(p, *t) {
                                    index.toggle(p, *t);
                                }
                            }
                            for t in &edit.toggle {
                                index.toggle(p, *t);
                            }
                        }
                    });
                }
                let items = paths
                    .iter()
                    .map(|p| obj(vec![("path", path_json(p)), ("tags", Json::Arr(self.tag_index.tags(p).into_iter().map(|t| text(t.label())).collect()))]))
                    .collect();
                Ok(obj(vec![("changed", Json::Bool(changes)), ("items", Json::Arr(items))]))
            }
        }
    }

    fn search_json(&self) -> Json {
        obj(vec![
            ("query", text(self.query.trim())),
            ("path", path_json(&self.path)),
            ("count", num(self.visible().len() as f64)),
            ("entries", self.rows_json(LIST_LIMIT)),
        ])
    }

    fn ctl_start(&mut self, kind: JobKind, title: &str, sources: Vec<PathBuf>, dest: PathBuf, timeout_ms: u64) -> Result<Json, RpcError> {
        self.start_job(title, kind, sources.clone(), dest.clone());
        if self.work.is_none() {
            return Err(RpcError::new(BUSY, "busy: another job is running"));
        }
        let token = sdk::new_token();
        if let Ok(mut j) = JOB.lock() {
            if let Some(old) = j.replace(JobCtl { token, kind, sources, dest }) {
                sdk::complete(old.token, Err(RpcError::new(NOT_FOUND, "superseded by a newer call")));
            }
        }
        Err(RpcError::pending(token, timeout_ms + 1500))
    }

    /// Reverts the newest agent rename, move, copy or new folder.
    pub(crate) fn ctl_undo(&mut self) -> Result<Json, RpcError> {
        let Some(op) = UNDO.lock().ok().and_then(|mut u| u.pop()) else {
            return Err(RpcError::new(NOT_FOUND, "nothing to undo"));
        };
        let fail = |what: &str, e: std::io::Error| RpcError::new(INVALID_PARAMS, format!("could not undo {what}: {e}"));
        let reply = match op {
            Undo::Rename { from, to } => {
                let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let back = fs::rename(&to, &name).map_err(|e| fail("the rename", e))?;
                self.with_tags(|index| index.moved(&to, &back));
                obj(vec![("undone", text("rename")), ("path", path_json(&back))])
            }
            Undo::Move(pairs) => {
                let mut restored = vec![];
                for (from, to) in &pairs {
                    let Some(dir) = from.parent() else { continue };
                    let back = fs::move_to(to, dir).map_err(|e| fail("the move", e))?;
                    self.with_tags(|index| index.moved(to, &back));
                    restored.push(path_json(&back));
                }
                obj(vec![("undone", text("move")), ("paths", Json::Arr(restored))])
            }
            Undo::Copy(copies) => {
                for c in &copies {
                    fs::delete(c).map_err(|e| fail("the copy", e))?;
                    self.with_tags(|index| index.removed(c));
                }
                obj(vec![("undone", text("copy")), ("removed", Json::Arr(copies.iter().map(|p| path_json(p)).collect()))])
            }
            Undo::NewFolder(path) => {
                std::fs::remove_dir(&path).map_err(|e| fail("the new folder (it must be empty)", e))?;
                obj(vec![("undone", text("new_folder")), ("path", path_json(&path))])
            }
        };
        let dir = self.path.clone();
        self.navigate(dir, false);
        Ok(reply)
    }

    /// Completes the navigate and search calls that were waiting. Called at the end of `tick`.
    pub(crate) fn ctl_tick(&mut self) {
        if !self.loading {
            if let Some(token) = disarm(&NAV) {
                let reply = match &self.error {
                    Some(e) => Err(RpcError::new(NOT_FOUND, format!("could not open {}: {e}", self.path.display()))),
                    None => Ok(self.state_json()),
                };
                sdk::complete(token, reply);
            }
        }
        if self.search.is_none() {
            if let Some(token) = disarm(&SEARCH) {
                sdk::complete(token, Ok(self.search_json()));
            }
        }
    }

    /// Completes the waiting copy, move or trash call with the job's report. Called before `finish_job`.
    pub(crate) fn ctl_job_done(&mut self, report: &Report) {
        let Some(job) = JOB.lock().ok().and_then(|mut j| j.take()) else { return };
        let ok = report.failed.is_empty() && !report.cancelled;
        match job.kind {
            JobKind::Transfer(Op::Copy) if !report.done.is_empty() => remember(Undo::Copy(report.done.clone())),
            JobKind::Transfer(Op::Move) if !report.done.is_empty() => {
                let pairs: Vec<(PathBuf, PathBuf)> = if report.done.len() == job.sources.len() {
                    job.sources.iter().cloned().zip(report.done.iter().cloned()).collect()
                } else {
                    job.sources.iter().filter_map(|s| s.file_name().map(|n| (s.clone(), job.dest.join(n)))).filter(|(_, to)| report.done.contains(to)).collect()
                };
                remember(Undo::Move(pairs));
            }
            _ => {}
        }
        let paths = |v: &[PathBuf]| Json::Arr(v.iter().map(|p| path_json(p)).collect());
        let reply = obj(vec![
            ("ok", Json::Bool(ok)),
            ("done", paths(&report.done)),
            ("skipped", paths(&report.skipped)),
            ("failed", Json::Arr(report.failed.iter().map(|(p, why)| obj(vec![("path", path_json(p)), ("error", text(why))])).collect())),
            ("cancelled", Json::Bool(report.cancelled)),
        ]);
        sdk::complete(job.token, Ok(reply));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bifrost_core::json;

    fn p(s: &str) -> Json {
        json::parse(s).unwrap()
    }

    #[test]
    fn every_method_parses_with_minimal_params() {
        for (m, params) in [
            ("files.state", "{}"),
            ("files.list", "{}"),
            ("files.select", r#"{"all":true}"#),
            ("files.open", r#"{"index":0}"#),
            ("files.navigate", r#"{"up":true}"#),
            ("files.set_view", r#"{"view":"list"}"#),
            ("files.new_folder", "{}"),
            ("files.rename", r#"{"name":"b"}"#),
            ("files.move", r#"{"dest":"/b"}"#),
            ("files.copy", r#"{"dest":"/b"}"#),
            ("files.trash", "{}"),
            ("files.undo", "{}"),
            ("files.search", r#"{"query":""}"#),
            ("files.get_info", "{}"),
            ("files.tags", "{}"),
        ] {
            parse(m, &p(params)).unwrap_or_else(|e| panic!("{m}: {}", e.message));
        }
        assert_eq!(METHODS.len(), 15);
    }

    #[test]
    fn unknown_method_is_not_found() {
        assert_eq!(parse("files.nope", &p("{}")).unwrap_err().code, METHOD_NOT_FOUND);
    }

    #[test]
    fn select_forms() {
        assert_eq!(parse("files.select", &p(r#"{"names":["a","b"],"add":true}"#)).unwrap(), Req::Select { what: Select::Names(vec!["a".into(), "b".into()]), add: true });
        assert_eq!(parse("files.select", &p(r#"{"indices":[2,0]}"#)).unwrap(), Req::Select { what: Select::Indices(vec![2, 0]), add: false });
        assert_eq!(parse("files.select", &p(r#"{"none":true}"#)).unwrap(), Req::Select { what: Select::None, add: false });
        assert_eq!(parse("files.select", &p("{}")).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(parse("files.select", &p(r#"{"indices":[-1]}"#)).unwrap_err().code, INVALID_PARAMS);
    }

    #[test]
    fn navigate_forms_and_timeout() {
        assert_eq!(parse("files.navigate", &p(r#"{"path":"/tmp","timeout_ms":500}"#)).unwrap(), Req::Navigate { to: Nav::Path("/tmp".into()), timeout_ms: 500 });
        assert_eq!(parse("files.navigate", &p(r#"{"back":true}"#)).unwrap(), Req::Navigate { to: Nav::Back, timeout_ms: DEFAULT_TIMEOUT_MS });
        assert_eq!(parse("files.navigate", &p(r#"{"path":"/","timeout_ms":900000}"#)).unwrap(), Req::Navigate { to: Nav::Path("/".into()), timeout_ms: sdk::MAX_TIMEOUT_MS });
        assert!(parse("files.navigate", &p("{}")).is_err());
        assert!(parse("files.navigate", &p(r#"{"path":3}"#)).is_err());
    }

    #[test]
    fn set_view_validates() {
        assert_eq!(
            parse("files.set_view", &p(r#"{"view":"columns","sort":"Size","hidden":true}"#)).unwrap(),
            Req::SetView { view: Some(View::Columns), sort: Some(Sort::Size), hidden: Some(true), sidebar: None, status_bar: None }
        );
        assert!(parse("files.set_view", &p(r#"{"view":"gallery"}"#)).is_err());
        assert!(parse("files.set_view", &p(r#"{"sort":"colour"}"#)).is_err());
        assert!(parse("files.set_view", &p("{}")).is_err());
    }

    #[test]
    fn transfer_needs_dest_and_keeps_paths() {
        assert_eq!(
            parse("files.move", &p(r#"{"paths":["/a/x","y"],"dest":"/b"}"#)).unwrap(),
            Req::Transfer { op: Op::Move, paths: vec!["/a/x".into(), "y".into()], dest: "/b".into(), timeout_ms: DEFAULT_TIMEOUT_MS }
        );
        assert!(matches!(parse("files.copy", &p(r#"{"dest":"/b"}"#)).unwrap(), Req::Transfer { op: Op::Copy, .. }));
        assert!(parse("files.copy", &p("{}")).is_err());
        assert!(parse("files.move", &p(r#"{"paths":"x","dest":"/b"}"#)).is_err());
    }

    #[test]
    fn rename_search_and_info() {
        assert!(parse("files.rename", &p(r#"{"path":"/a"}"#)).is_err());
        assert_eq!(parse("files.search", &p(r#"{"query":"report"}"#)).unwrap(), Req::Search { query: "report".into(), timeout_ms: DEFAULT_TIMEOUT_MS });
        assert!(parse("files.search", &p("{}")).is_err());
        assert_eq!(parse("files.get_info", &p(r#"{"path":"/a"}"#)).unwrap(), Req::Info { path: Some("/a".into()) });
    }

    #[test]
    fn tags_parse_colours() {
        let Req::Tags { paths, edit } = parse("files.tags", &p(r#"{"path":"/a","paths":["/b"],"add":["Red","blue"],"remove":["gray"]}"#)).unwrap() else { panic!() };
        assert_eq!(paths, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(edit.add, vec![Tag::Red, Tag::Blue]);
        assert_eq!(edit.remove, vec![Tag::Gray]);
        assert_eq!(edit.set, None);
        assert!(parse("files.tags", &p(r#"{"set":["pink"]}"#)).is_err());
    }

    #[test]
    fn pending_marker_round_trips() {
        let e = RpcError::pending(9, 5000);
        assert_eq!(e.pending_token(), Some((9, 5000)));
    }
}
