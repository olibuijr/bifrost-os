//! `bifrost-portal`: a tiny `org.freedesktop.portal.Desktop` that implements only
//! `org.freedesktop.portal.Settings` (version 2), so GTK, Qt, Firefox and winit/egui apps follow
//! BifrOSt's Light/Dark appearance and accent without the GNOME portal stack.
//!
//! Source of truth: `$XDG_RUNTIME_DIR/bifrost-appearance`, written by the shell (see
//! `springboard::appearance_portal`). Watched with inotify (500 ms poll only if inotify is unavailable); a change emits
//! `SettingChanged` for `color-scheme`, `accent-color` and `contrast`, and mirrors the scheme into GTK 3's settings.ini.
//! D-Bus activated through `/usr/share/dbus-1/services/org.freedesktop.portal.Desktop.service`.
use springboard::appearance_portal::{self as ap, State, Value as V, NAMESPACE};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

extern "C" {
    fn inotify_init1(flags: i32) -> i32;
    fn inotify_add_watch(fd: i32, path: *const std::os::raw::c_char, mask: u32) -> i32;
    fn read(fd: i32, buf: *mut u8, n: usize) -> isize;
}

/// Opens an inotify fd on the runtime dir (close-write, moved-to, create); None when unavailable.
fn watch_runtime_dir() -> Option<i32> {
    let dir = ap::runtime_dir()?;
    let path = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
    unsafe {
        let fd = inotify_init1(0);
        if fd < 0 { return None; }
        // IN_CLOSE_WRITE | IN_MOVED_TO | IN_CREATE
        if inotify_add_watch(fd, path.as_ptr(), 0x8 | 0x80 | 0x100) < 0 { return None; }
        Some(fd)
    }
}

/// True when an inotify event buffer holds an event for the file `target`.
fn events_name(buf: &[u8], target: &str) -> bool {
    let mut i = 0;
    while i + 16 <= buf.len() {
        let len = u32::from_ne_bytes([buf[i + 12], buf[i + 13], buf[i + 14], buf[i + 15]]) as usize;
        let end = (i + 16 + len).min(buf.len());
        let name = &buf[i + 16..end];
        let name = name.split(|b| *b == 0).next().unwrap_or(&[]);
        if name == target.as_bytes() { return true; }
        i += 16 + len;
    }
    false
}

/// Blocks until the appearance file changes; events for other files in the runtime dir are ignored
/// (they used to wake this thread about twice a second). Polls every 500 ms only without inotify.
fn wait_change(fd: Option<i32>) {
    match fd {
        Some(fd) => loop {
            let mut buf = [0u8; 4096];
            let n = unsafe { read(fd, buf.as_mut_ptr(), buf.len()) };
            if n <= 0 { std::thread::sleep(Duration::from_millis(500)); return; }
            if events_name(&buf[..n as usize], ap::FILE_NAME) { return; }
        },
        None => std::thread::sleep(Duration::from_millis(500)),
    }
}

#[cfg(test)]
mod tests {
    use super::events_name;

    fn ev(name: &str) -> Vec<u8> {
        let mut v = vec![0u8; 12];
        let len = (name.len() + 1 + 3) / 4 * 4;
        v.extend((len as u32).to_ne_bytes());
        v.extend(name.as_bytes());
        v.resize(16 + len, 0);
        v
    }

    #[test]
    fn filters_by_file_name() {
        let mut b = ev("other.sock");
        assert!(!events_name(&b, "bifrost-appearance"));
        b.extend(ev("bifrost-appearance"));
        assert!(events_name(&b, "bifrost-appearance"));
    }
}
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedValue, Value};

const PATH: &str = "/org/freedesktop/portal/desktop";

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.portal.Error")]
enum PortalError {
    #[zbus(error)]
    ZBus(zbus::Error),
    NotFound(String),
}

struct Settings(Arc<Mutex<State>>);

fn owned(v: V) -> OwnedValue {
    let v = match v {
        V::U32(n) => Value::from(n),
        V::Rgb(r, g, b) => Value::from((r, g, b)),
    };
    v.try_into().expect("plain values convert")
}

/// `namespaces` patterns: empty matches everything; a trailing `*` matches a prefix.
fn matches(patterns: &[String], ns: &str) -> bool {
    patterns.is_empty()
        || patterns.iter().any(|p| p.is_empty() || p == ns || p.strip_suffix('*').is_some_and(|pre| ns.starts_with(pre)))
}

impl Settings {
    fn state(&self) -> State {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn one(&self, ns: &str, key: &str) -> Result<OwnedValue, PortalError> {
        self.state().read_one(ns, key).map(owned).ok_or_else(|| PortalError::NotFound(format!("{ns} {key}")))
    }
}

#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Settings {
    fn read_all(&self, namespaces: Vec<String>) -> HashMap<String, HashMap<String, OwnedValue>> {
        let mut out = HashMap::new();
        if matches(&namespaces, NAMESPACE) {
            let st = self.state();
            let keys = State::KEYS.iter().filter_map(|k| Some((k.to_string(), owned(st.read_one(NAMESPACE, k)?))));
            out.insert(NAMESPACE.to_string(), keys.collect());
        }
        out
    }

    /// Deprecated: the value wrapped in a second variant.
    fn read(&self, namespace: &str, key: &str) -> Result<OwnedValue, PortalError> {
        let v = self.one(namespace, key)?;
        Ok(Value::new(Value::from(v)).try_into().expect("plain values convert"))
    }

    fn read_one(&self, namespace: &str, key: &str) -> Result<OwnedValue, PortalError> {
        self.one(namespace, key)
    }

    #[zbus(signal)]
    async fn setting_changed(e: &SignalEmitter<'_>, namespace: &str, key: &str, value: Value<'_>) -> zbus::Result<()>;

    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }
}

fn load() -> State {
    ap::runtime_dir()
        .and_then(|d| std::fs::read_to_string(d.join(ap::FILE_NAME)).ok())
        .map(|s| State::parse(&s))
        .unwrap_or_default()
}

/// GTK 3 ignores the portal outside a sandbox: mirror the scheme into its settings.ini (read at app start).
/// Skipped while the shell has published nothing yet (no preference), so a user's own choice is not overwritten.
fn gtk3(state: &State) {
    if state.color_scheme == 0 {
        return;
    }
    if let Err(e) = ap::mirror_gtk3(state) {
        eprintln!("bifrost-portal: gtk-3.0/settings.ini: {e}");
    }
}

fn main() -> zbus::Result<()> {
    let first = load();
    gtk3(&first);
    let shared = Arc::new(Mutex::new(first));
    let conn = zbus::blocking::connection::Builder::session()?
        .name("org.freedesktop.portal.Desktop")?
        .serve_at(PATH, Settings(shared.clone()))?
        .build()?;
    let iface = conn.object_server().interface::<_, Settings>(PATH)?;
    let watch = watch_runtime_dir();
    loop {
        wait_change(watch);
        let new = load();
        let old = std::mem::replace(&mut *shared.lock().unwrap_or_else(|e| e.into_inner()), new);
        let changes = old.changes(&new);
        if changes.iter().any(|(k, _)| *k == "color-scheme") {
            gtk3(&new);
        }
        for (key, value) in changes {
            let v = Value::from(owned(value));
            let _ = zbus::block_on(Settings::setting_changed(iface.signal_emitter(), NAMESPACE, key, v));
        }
    }
}
