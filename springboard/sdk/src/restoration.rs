//! State restoration (`NativeWindowRestoration` parity): an app saves what it needs to put its window back (the open
//! note, the folder and view) and gets it again after a relaunch, a logout, an update or a crash.
//!
//! The app implements three `App` hooks: `restoration_identifier` (the window's identifier; `None` means the window is
//! not restorable), `encode_restorable_state` (`encodeRestorableState(with:)`) and `restore_state`
//! (`restoreState(with:)`). It calls `ctx.invalidate_restorable_state()` whenever that state changes
//! (`invalidateRestorableState()`); the runtime coalesces the calls and writes at most once per [`DEBOUNCE_MS`], never
//! once per frame. The state is one small JSON file in the app's own data directory ([`FILE`]); the window frame is
//! kept by the shell (`session.rs`), the file only remembers the window size the app last had.
//!
//! The shell starts the app with `BIFROST_RESTORE=1` when the windows should come back ([`restore_requested`]); without
//! it the app starts fresh, as after "Close windows when quitting an application". A missing, corrupt or foreign
//! file (other window, other format version) is ignored and removed, like a stale restoration archive.
use crate::coding::{CodingError, Decodable, Encodable, JSONDecoder, JSONEncoder, Value};
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// The file inside the app's data directory.
pub const FILE: &str = "restorable-state.json";
/// The environment variable the shell sets to `1` when the app should restore its state.
pub const ENV_RESTORE: &str = "BIFROST_RESTORE";
/// Invalidations inside this window (ms) become one write.
pub const DEBOUNCE_MS: f64 = 750.0;
/// The file format version; another version is ignored.
pub const VERSION: i64 = 1;

impl Encodable for Value {
    fn encode(&self) -> Value {
        self.clone()
    }
}
impl Decodable for Value {
    fn decode(v: &Value) -> Result<Self, CodingError> {
        Ok(v.clone())
    }
}

/// The keyed archive an app encodes into and decodes from (`NativeCoder` as `encodeRestorableState(with:)` hands it out).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RestorableState {
    entries: Vec<(String, Value)>,
}

impl RestorableState {
    pub fn new() -> Self {
        Self::default()
    }
    /// `encode(_:forKey:)`: a later value for the same key replaces the earlier one.
    pub fn encode_for<T: Encodable>(&mut self, key: &str, value: &T) {
        if value.is_absent() {
            self.entries.retain(|(k, _)| k != key);
            return;
        }
        let v = value.encode();
        match self.entries.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = v,
            None => self.entries.push((key.to_string(), v)),
        }
    }
    /// `decodeObject(forKey:)`: `None` when the key is missing or has another type.
    pub fn decode_for<T: Decodable>(&self, key: &str) -> Option<T> {
        self.entries.iter().find(|(k, _)| k == key).and_then(|(_, v)| T::decode(v).ok())
    }
    /// `containsValue(forKey:)`.
    pub fn contains(&self, key: &str) -> bool {
        self.entries.iter().any(|(k, _)| k == key)
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    fn to_value(&self) -> Value {
        Value::Dict(self.entries.clone())
    }
    fn from_value(v: &Value) -> Option<Self> {
        if let Value::Dict(d) = v {
            Some(RestorableState { entries: d.clone() })
        } else {
            None
        }
    }
}

/// What the file holds.
#[derive(Clone, Debug, PartialEq)]
struct Record {
    version: i64,
    window: String,
    w: u32,
    h: u32,
    saved_at: i64,
    state: Value,
}
crate::codable!(Record { version: i64, window: String, w: u32, h: u32, saved_at: i64, state: Value });

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

fn read_record(dir: &Path) -> Option<Record> {
    let text = std::fs::read_to_string(path(dir)).ok()?;
    JSONDecoder::new().decode::<Record>(&text).ok()
}

/// Writes the state with an atomic replace. Returns `Ok(false)` without touching the disk when the file already holds
/// the same window, size and state (an invalidation that changed nothing costs no write).
pub fn save(dir: &Path, window: &str, size: (u32, u32), state: &RestorableState, saved_at: i64) -> std::io::Result<bool> {
    let record = Record { version: VERSION, window: window.to_string(), w: size.0, h: size.1, saved_at, state: state.to_value() };
    if read_record(dir).is_some_and(|old| old.version == VERSION && old.window == record.window && old.w == record.w && old.h == record.h && old.state == record.state) {
        return Ok(false);
    }
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{FILE}.tmp"));
    std::fs::write(&tmp, JSONEncoder::new().encode(&record))?;
    std::fs::rename(&tmp, path(dir))?;
    Ok(true)
}

/// The saved state of `window` and the size the window had. A corrupt file, another format version or another window
/// identifier gives `None`, and the file is removed so it is not tried again.
pub fn load(dir: &Path, window: &str) -> Option<(RestorableState, (u32, u32))> {
    if !path(dir).exists() {
        return None;
    }
    let found = read_record(dir).filter(|r| r.version == VERSION && r.window == window).and_then(|r| RestorableState::from_value(&r.state).map(|s| (s, (r.w, r.h))));
    if found.is_none() {
        discard(dir);
    }
    found
}

/// Removes the saved state (`discardRestorableState`).
pub fn discard(dir: &Path) {
    let _ = std::fs::remove_file(path(dir));
}

/// True when the shell asked this launch to restore (`BIFROST_RESTORE=1`).
pub fn restore_requested() -> bool {
    std::env::var(ENV_RESTORE).is_ok_and(|v| v == "1")
}

/// The debounce behind `invalidateRestorableState()`: the first call arms a write [`DEBOUNCE_MS`] later, further calls
/// before it fires join it.
#[derive(Debug, Default)]
pub struct Invalidation {
    due: Option<f64>,
}

impl Invalidation {
    /// True when this call armed a write (the first of a burst).
    pub fn invalidate(&mut self, now: f64) -> bool {
        if self.due.is_some() {
            return false;
        }
        self.due = Some(now + DEBOUNCE_MS);
        true
    }
    /// When the write is due (ms, the runtime's clock), if one is armed.
    pub fn deadline(&self) -> Option<f64> {
        self.due
    }
    /// True once when the write is due.
    pub fn take_due(&mut self, now: f64) -> bool {
        if self.due.is_some_and(|d| now >= d) {
            self.due = None;
            return true;
        }
        false
    }
    /// True once when a write is armed, due or not (quit, suspend).
    pub fn take_now(&mut self) -> bool {
        self.due.take().is_some()
    }
}

thread_local! {
    static PENDING: RefCell<Invalidation> = RefCell::new(Invalidation::default());
}

pub(crate) fn invalidate(now: f64) -> bool {
    PENDING.with(|p| p.borrow_mut().invalidate(now))
}
pub(crate) fn deadline() -> Option<f64> {
    PENDING.with(|p| p.borrow().deadline())
}
pub(crate) fn take_due(now: f64) -> bool {
    PENDING.with(|p| p.borrow_mut().take_due(now))
}
pub(crate) fn take_now() -> bool {
    PENDING.with(|p| p.borrow_mut().take_now())
}

/// Encodes the app's state and writes it. `None` when the app is not restorable, else whether a write happened.
pub(crate) fn save_app<A: crate::App>(app: &A, ctx: &crate::Ctx) -> Option<bool> {
    let window = app.restoration_identifier()?;
    let mut state = RestorableState::new();
    app.encode_restorable_state(ctx, &mut state);
    Some(save(&ctx.data_dir, window, (ctx.cfg.w, ctx.cfg.h), &state, crate::testing::unix_secs()).unwrap_or(false))
}

/// Hands the saved state to the app. True when the app restored something (and wants a frame).
pub(crate) fn restore_app<A: crate::App>(app: &mut A, ctx: &crate::Ctx) -> bool {
    let Some(window) = app.restoration_identifier().map(str::to_string) else { return false };
    match load(&ctx.data_dir, &window) {
        Some((state, _size)) => app.restore_state(ctx, &state),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, Ctx, Damage, Surface};

    /// A tiny notes app: an open note and a view.
    struct Notes {
        open: String,
        view: String,
        restorable: bool,
    }
    impl App for Notes {
        fn id(&self) -> &str {
            "test.notes"
        }
        fn draw(&mut self, _ctx: &Ctx, _s: &mut Surface, _t: f64) -> Damage {
            Damage::new(1, 1)
        }
        fn restoration_identifier(&self) -> Option<&str> {
            self.restorable.then_some("main")
        }
        fn encode_restorable_state(&self, _ctx: &Ctx, state: &mut RestorableState) {
            state.encode_for("open", &self.open);
            state.encode_for("view", &self.view);
        }
        fn restore_state(&mut self, _ctx: &Ctx, state: &RestorableState) -> bool {
            self.open = state.decode_for("open").unwrap_or_default();
            self.view = state.decode_for("view").unwrap_or_default();
            true
        }
    }

    fn notes(open: &str) -> Notes {
        Notes { open: open.into(), view: "list".into(), restorable: true }
    }

    fn ctx_in(dir: &Path) -> Ctx {
        let mut ctx = crate::runtime::test_ctx();
        ctx.data_dir = dir.to_path_buf();
        ctx
    }

    #[test]
    fn restorable_state_round_trips_through_a_relaunch() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_in(dir.path());
        assert_eq!(save_app(&notes("groceries"), &ctx), Some(true));
        // a new process: a fresh app object reads the same directory
        let mut relaunched = Notes { open: String::new(), view: String::new(), restorable: true };
        assert!(restore_app(&mut relaunched, &ctx));
        assert_eq!((relaunched.open.as_str(), relaunched.view.as_str()), ("groceries", "list"));
    }

    #[test]
    fn is_restorable_follows_the_identifier() {
        let mut app = notes("a");
        assert!(app.is_restorable());
        app.restorable = false;
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_in(dir.path());
        assert!(!app.is_restorable());
        assert_eq!(save_app(&app, &ctx), None, "a window without an identifier writes nothing");
        assert!(!path(dir.path()).exists());
        assert!(!restore_app(&mut app, &ctx));
    }

    #[test]
    fn invalidate_coalesces_writes() {
        let mut inv = Invalidation::default();
        assert!(inv.invalidate(1000.0), "the first call arms the write");
        for i in 1..100 {
            assert!(!inv.invalidate(1000.0 + i as f64), "the rest join it");
        }
        assert_eq!(inv.deadline(), Some(1000.0 + DEBOUNCE_MS));
        assert!(!inv.take_due(1000.0 + DEBOUNCE_MS - 1.0), "nothing before the deadline");
        assert!(inv.take_due(1000.0 + DEBOUNCE_MS), "one write at the deadline");
        assert!(!inv.take_due(5000.0), "and no second one");
        assert!(inv.invalidate(6000.0), "a later change arms the next write");
        assert!(inv.take_now(), "quit writes at once");
        assert!(!inv.take_now());
    }

    #[test]
    fn an_unchanged_state_costs_no_write() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_in(dir.path());
        assert_eq!(save_app(&notes("a"), &ctx), Some(true));
        assert_eq!(save_app(&notes("a"), &ctx), Some(false), "same state, same size");
        assert_eq!(save_app(&notes("b"), &ctx), Some(true), "a change is written");
        assert!(!dir.path().join(format!("{FILE}.tmp")).exists(), "the temporary file is renamed away");
    }

    #[test]
    fn a_corrupt_or_foreign_file_is_ignored_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_in(dir.path());
        std::fs::write(path(dir.path()), "{ not json").unwrap();
        let mut app = notes("keep");
        assert!(!restore_app(&mut app, &ctx));
        assert_eq!(app.open, "keep", "the app keeps its fresh state");
        assert!(!path(dir.path()).exists(), "the corrupt file is gone");
        // another window identifier
        assert_eq!(save_app(&notes("a"), &ctx), Some(true));
        assert!(load(dir.path(), "other").is_none());
        assert!(!path(dir.path()).exists());
        // another format version
        std::fs::write(path(dir.path()), r#"{"version":99,"window":"main","w":1,"h":1,"saved_at":0,"state":{}}"#).unwrap();
        assert!(load(dir.path(), "main").is_none());
    }

    #[test]
    fn the_window_size_is_kept_automatically() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_in(dir.path());
        save_app(&notes("a"), &ctx);
        let (_, size) = load(dir.path(), "main").unwrap();
        assert_eq!(size, (100, 100), "the context's window size, with no help from the app");
    }

    #[test]
    fn invalidating_holds_sudden_termination_until_the_write() {
        let ctx = crate::runtime::test_ctx();
        ctx.enable_sudden_termination();
        assert_eq!(ctx.take_sent(), vec![crate::proto::ToShell::SuddenTermination { enabled: true }]);
        ctx.invalidate_restorable_state();
        ctx.invalidate_restorable_state();
        assert_eq!(ctx.take_sent(), vec![crate::proto::ToShell::SuddenTermination { enabled: false }], "one hold for the whole burst");
        assert!(deadline().is_some());
        ctx.finish_restorable_write();
        assert_eq!(ctx.take_sent(), vec![crate::proto::ToShell::SuddenTermination { enabled: true }]);
    }

    #[test]
    fn restore_is_only_requested_by_the_shell() {
        // the variable is not set in the test environment
        std::env::remove_var(ENV_RESTORE);
        assert!(!restore_requested());
    }
}
