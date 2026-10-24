//! The appearance file shared by the shell and `bifrost-portal`.
//!
//! The shell writes `$XDG_RUNTIME_DIR/bifrost-appearance` for the desktop user whenever the effective
//! scheme (Auto resolved) or the accent changes; `bifrost-portal` serves it to native apps as
//! `org.freedesktop.portal.Settings` (namespace `org.freedesktop.appearance`). One key per line:
//!
//! ```text
//! color-scheme=1      # 1 prefer dark, 2 prefer light (portal spec; 0 no preference)
//! accent=0.039 0.518 1.000
//! contrast=1          # 1 when Increase Contrast is on (portal spec; 0 normal)
//! ```
//!
//! The portal also mirrors the scheme into GTK 3's `settings.ini` (`gtk-application-prefer-dark-theme`), the one
//! setting GTK 3 reads outside a sandbox; see [`gtk3_settings_ini`].
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
    /// 0 normal, 1 high contrast (Increase Contrast).
    pub contrast: u32,
}

impl Default for State {
    fn default() -> Self {
        State { color_scheme: 0, accent: (0.039, 0.518, 1.0), contrast: 0 }
    }
}

/// A portal value for one `org.freedesktop.appearance` key.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    U32(u32),
    Rgb(f64, f64, f64),
}

impl State {
    /// The effective appearance: `light` is Auto resolved, `accent_rgba` the accent for that scheme
    /// (`gfx::pack` order, R in the low byte), `high_contrast` Increase Contrast.
    pub fn new(light: bool, accent_rgba: u32, high_contrast: bool) -> Self {
        let c = |s: u32| ((accent_rgba >> s) & 0xff) as f64 / 255.0;
        State { color_scheme: if light { 2 } else { 1 }, accent: (c(0), c(8), c(16)), contrast: high_contrast as u32 }
    }

    pub fn serialise(&self) -> String {
        let (r, g, b) = self.accent;
        format!("color-scheme={}\naccent={r:.3} {g:.3} {b:.3}\ncontrast={}\n", self.color_scheme, self.contrast)
    }

    /// The `SettingChanged` signals going from `self` to `new`: one `(key, value)` per key whose value differs,
    /// in [`State::KEYS`] order (all in [`NAMESPACE`]).
    pub fn changes(&self, new: &State) -> Vec<(&'static str, Value)> {
        State::KEYS
            .iter()
            .filter_map(|k| {
                let b = new.read_one(NAMESPACE, k)?;
                (self.read_one(NAMESPACE, k) != Some(b)).then_some((*k, b))
            })
            .collect()
    }

    /// True when GTK 3 should prefer its dark variant.
    pub fn prefers_dark(&self) -> bool {
        self.color_scheme == 1
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
                "contrast" => {
                    if let Ok(n) = v.trim().parse::<u32>() {
                        if n <= 1 {
                            st.contrast = n;
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
            "contrast" => Some(Value::U32(self.contrast)),
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

/// GTK 3's `settings.ini` with `gtk-application-prefer-dark-theme` set to `dark` in its `[Settings]` group.
/// Every other line is kept as it is; the group is added when missing. GTK 3 reads the file when an app starts.
pub fn gtk3_settings_ini(existing: &str, dark: bool) -> String {
    const KEY: &str = "gtk-application-prefer-dark-theme";
    let line = format!("{KEY}={}", dark as u8);
    let mut out: Vec<String> = Vec::new();
    let (mut in_settings, mut seen_group, mut done) = (false, false, false);
    for l in existing.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            if in_settings && !done {
                out.push(line.clone());
                done = true;
            }
            in_settings = t == "[Settings]";
            seen_group |= in_settings;
        } else if in_settings && t.split_once('=').is_some_and(|(k, _)| k.trim() == KEY) {
            if !done {
                out.push(line.clone());
                done = true;
            }
            continue;
        }
        out.push(l.to_string());
    }
    if !done {
        if !seen_group {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push("[Settings]".to_string());
        }
        out.push(line);
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// The GTK 3 `settings.ini` path: `$XDG_CONFIG_HOME/gtk-3.0/settings.ini`, else `$HOME/.config/gtk-3.0/settings.ini`.
pub fn gtk3_settings_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").filter(|v| !v.is_empty()).map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("gtk-3.0").join("settings.ini"))
}

/// Mirrors the scheme into GTK 3's `settings.ini` (written only when the value changes; atomic rename).
pub fn mirror_gtk3(state: &State) -> std::io::Result<()> {
    let Some(path) = gtk3_settings_path() else { return Ok(()) };
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let body = gtk3_settings_ini(&existing, state.prefers_dark());
    if body == existing {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("ini.bifrost-tmp");
    std::fs::write(&tmp, &body)?;
    std::fs::rename(&tmp, &path)
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
        let s = State::new(false, 10 | 132 << 8 | 255 << 16 | 255 << 24, true);
        assert_eq!(s.serialise(), "color-scheme=1\naccent=0.039 0.518 1.000\ncontrast=1\n");
        let p = State::parse(&s.serialise());
        assert_eq!(p.color_scheme, 1);
        assert_eq!(p.contrast, 1);
        assert!((p.accent.1 - 0.518).abs() < 1e-9);
        assert_eq!(State::new(true, 0, false).color_scheme, 2);
        assert_eq!(State::new(true, 0, false).contrast, 0);
        // a file from an older shell (no contrast line) reads as normal contrast
        assert_eq!(State::parse("color-scheme=2\naccent=0 0 1\n").contrast, 0);
    }

    #[test]
    fn parse_ignores_junk() {
        let p = State::parse("x\ncolor-scheme=9\naccent=2 0 0\ncontrast=7\nfoo=bar\n");
        assert_eq!(p, State::default());
        assert_eq!(State::parse("color-scheme=2").color_scheme, 2);
    }

    #[test]
    fn read_one_maps_keys() {
        let s = State { color_scheme: 2, accent: (0.1, 0.2, 0.3), contrast: 0 };
        assert_eq!(s.read_one(NAMESPACE, "color-scheme"), Some(Value::U32(2)));
        assert_eq!(s.read_one(NAMESPACE, "accent-color"), Some(Value::Rgb(0.1, 0.2, 0.3)));
        assert_eq!(s.read_one(NAMESPACE, "contrast"), Some(Value::U32(0)));
        assert_eq!(State { contrast: 1, ..s }.read_one(NAMESPACE, "contrast"), Some(Value::U32(1)));
        assert_eq!(s.read_one(NAMESPACE, "nope"), None);
        assert_eq!(s.read_one("org.gnome.desktop.interface", "color-scheme"), None);
    }

    #[test]
    fn changes_name_only_the_keys_that_differ() {
        let dark = State::new(false, 0xff_ff_84_0a, false);
        assert!(dark.changes(&dark).is_empty());
        // Light/Dark (Auto flipping at 07:00 included): the scheme and the light accent change
        let light = State::new(true, 0xff_ff_7a_00, false);
        let c = dark.changes(&light);
        assert_eq!(c[0], ("color-scheme", Value::U32(2)));
        assert_eq!(c[1].0, "accent-color");
        assert_eq!(c.len(), 2);
        // Increase Contrast alone
        let hc = State { contrast: 1, ..dark };
        assert_eq!(dark.changes(&hc), vec![("contrast", Value::U32(1))]);
        // an accent change alone
        let purple = State { accent: (0.75, 0.35, 0.95), ..dark };
        assert_eq!(dark.changes(&purple), vec![("accent-color", Value::Rgb(0.75, 0.35, 0.95))]);
    }

    #[test]
    fn gtk3_ini_merges_the_dark_preference() {
        assert_eq!(gtk3_settings_ini("", true), "[Settings]\ngtk-application-prefer-dark-theme=1\n");
        let mine = "# mine\n[Settings]\ngtk-font-name=Inter 11\ngtk-application-prefer-dark-theme = 1\n[Other]\nx=1\n";
        assert_eq!(
            gtk3_settings_ini(mine, false),
            "# mine\n[Settings]\ngtk-font-name=Inter 11\ngtk-application-prefer-dark-theme=0\n[Other]\nx=1\n"
        );
        // the key is added at the end of [Settings] when another group follows it
        assert_eq!(gtk3_settings_ini("[Settings]\na=b\n[Other]\n", true), "[Settings]\na=b\ngtk-application-prefer-dark-theme=1\n[Other]\n");
        // no [Settings] group yet: one is appended
        assert_eq!(gtk3_settings_ini("[Other]\nx=1\n", true), "[Other]\nx=1\n\n[Settings]\ngtk-application-prefer-dark-theme=1\n");
        // idempotent, so the portal rewrites nothing when the scheme did not change
        let once = gtk3_settings_ini(mine, true);
        assert_eq!(gtk3_settings_ini(&once, true), once);
        assert!(State::new(false, 0, false).prefers_dark() && !State::new(true, 0, false).prefers_dark());
    }
}
