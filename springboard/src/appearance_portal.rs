//! The appearance file shared by the shell and `bifrost-portal`.
//!
//! The shell writes `$XDG_RUNTIME_DIR/bifrost-appearance` for the desktop user whenever the effective
//! scheme (Auto resolved) or the accent changes; `bifrost-portal` serves it to native apps as
//! `org.freedesktop.portal.Settings` (namespace `org.freedesktop.appearance`). One key per line:
//!
//! ```text
//! color-scheme=1      # 1 prefer dark, 2 prefer light (portal spec; 0 no preference)
//! accent=0.039 0.518 1.000
//! ```
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FILE_NAME: &str = "bifrost-appearance";
pub const NAMESPACE: &str = "org.freedesktop.appearance";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    /// 0 no preference, 1 prefer dark, 2 prefer light.
    pub color_scheme: u32,
    /// sRGB, each 0..1.
    pub accent: (f64, f64, f64),
}

impl Default for State {
    fn default() -> Self {
        State { color_scheme: 0, accent: (0.039, 0.518, 1.0) }
    }
}

/// A portal value for one `org.freedesktop.appearance` key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    U32(u32),
    Rgb(f64, f64, f64),
}

impl State {
    pub fn new(light: bool, accent_rgba: u32) -> Self {
        let c = |s: u32| ((accent_rgba >> s) & 0xff) as f64 / 255.0;
        State { color_scheme: if light { 2 } else { 1 }, accent: (c(0), c(8), c(16)) }
    }

    pub fn serialise(&self) -> String {
        let (r, g, b) = self.accent;
        format!("color-scheme={}\naccent={r:.3} {g:.3} {b:.3}\n", self.color_scheme)
    }

    /// Unknown keys and bad lines are ignored; missing keys keep their defaults.
    pub fn parse(s: &str) -> Self {
        let mut st = State::default();
        for line in s.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            match k.trim() {
                "color-scheme" => {
                    if let Ok(n) = v.trim().parse::<u32>() {
                        if n <= 2 {
                            st.color_scheme = n;
                        }
                    }
                }
                "accent" => {
                    let f: Vec<f64> = v.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    if f.len() == 3 && f.iter().all(|x| (0.0..=1.0).contains(x)) {
                        st.accent = (f[0], f[1], f[2]);
                    }
                }
                _ => {}
            }
        }
        st
    }

    /// The value `ReadOne(namespace, key)` returns; `None` is NotFound.
    pub fn read_one(&self, namespace: &str, key: &str) -> Option<Value> {
        if namespace != NAMESPACE {
            return None;
        }
        match key {
            "color-scheme" => Some(Value::U32(self.color_scheme)),
            "accent-color" => Some(Value::Rgb(self.accent.0, self.accent.1, self.accent.2)),
            "contrast" => Some(Value::U32(0)),
            _ => None,
        }
    }

    pub const KEYS: [&'static str; 3] = ["color-scheme", "accent-color", "contrast"];
}

/// The desktop user's runtime dir, from the session file the shell writes, else `$XDG_RUNTIME_DIR`.
pub fn runtime_dir() -> Option<PathBuf> {
    std::fs::read_to_string("/run/bifrost/user/session.json")
        .ok()
        .and_then(|s| {
            let k = s.find("\"XDG_RUNTIME_DIR\"")?;
            let rest = &s[k + 17..];
            let rest = &rest[rest.find('"')? + 1..];
            Some(PathBuf::from(&rest[..rest.find('"')?]))
        })
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
}

extern "C" {
    fn geteuid() -> u32;
}

/// True when this process runs as root.
pub fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { geteuid() == 0 }
}

static LAST: Mutex<Option<String>> = Mutex::new(None);

/// Writes the file atomically when `state` differs from the last write (cheap to call every frame).
/// `owner` is asked only when a write happens (the shell chowns to the account when it runs as root).
pub fn publish(state: State, owner: impl FnOnce() -> Option<(u32, u32)>) {
    let body = state.serialise();
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_deref() == Some(body.as_str()) {
        return;
    }
    let Some(dir) = runtime_dir() else { return };
    if write(&dir, &body, owner()).is_ok() {
        *last = Some(body);
    }
}

fn write(dir: &Path, body: &str, owner: Option<(u32, u32)>) -> std::io::Result<()> {
    let tmp = dir.join(format!(".{FILE_NAME}.tmp"));
    std::fs::write(&tmp, body)?;
    if let Some((uid, gid)) = owner {
        std::os::unix::fs::chown(&tmp, Some(uid), Some(gid))?;
    }
    std::fs::rename(&tmp, dir.join(FILE_NAME))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let s = State::new(false, 10 | 132 << 8 | 255 << 16 | 255 << 24);
        assert_eq!(s.serialise(), "color-scheme=1\naccent=0.039 0.518 1.000\n");
        let p = State::parse(&s.serialise());
        assert_eq!(p.color_scheme, 1);
        assert!((p.accent.1 - 0.518).abs() < 1e-9);
        assert_eq!(State::new(true, 0).color_scheme, 2);
    }

    #[test]
    fn parse_ignores_junk() {
        let p = State::parse("x\ncolor-scheme=9\naccent=2 0 0\nfoo=bar\n");
        assert_eq!(p, State::default());
        assert_eq!(State::parse("color-scheme=2").color_scheme, 2);
    }

    #[test]
    fn read_one_maps_keys() {
        let s = State { color_scheme: 2, accent: (0.1, 0.2, 0.3) };
        assert_eq!(s.read_one(NAMESPACE, "color-scheme"), Some(Value::U32(2)));
        assert_eq!(s.read_one(NAMESPACE, "accent-color"), Some(Value::Rgb(0.1, 0.2, 0.3)));
        assert_eq!(s.read_one(NAMESPACE, "contrast"), Some(Value::U32(0)));
        assert_eq!(s.read_one(NAMESPACE, "nope"), None);
        assert_eq!(s.read_one("org.gnome.desktop.interface", "color-scheme"), None);
    }
}
