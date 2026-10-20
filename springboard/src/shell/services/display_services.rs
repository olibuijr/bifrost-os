//! The `display` service: the displays an app can read and, with permission, change or capture. It builds
//! `bifrost_ui::display_info::DisplayInfo` records for the output the shell drives (size, scale and bars from the shell,
//! the refresh rate from the output mode, identity and the mode list from the display's sysfs entry and EDID), and it
//! pushes `reconfigured` to every app when the list changes.
//!
//! Methods (JSON parameters):
//! - `list {kind}` answers the displays: `active` (default) or `online`.
//! - `main {}` and `info {id}` answer one display; `modes {id}` answers its mode list.
//! - `set_mode {id, width, height, refresh_hz}` needs the `display-config` permission. The output mode belongs to the
//!   window server (desktop session), so the shell hands the change to it over the output-management client
//!   ([`MODE_REQUEST`], sent by `bare::wayland`). The answer is definite and at once: `{applied: true}` when the mode is
//!   already the current one, else `{applied: false, requested: true}` and the outcome arrives as the `mode_changed`
//!   event. An applied change is kept only when the person calls `keep_mode`; otherwise it reverts after
//!   [`CONFIRM_SECS`] (`mode_reverted` event), and `revert_mode` reverts it at once.
//! - `set_rotation` and `mirror` fail: the output is a single panel the shell does not rotate or mirror.
//! - `capture {id}` answers the screen as `{width, height, png}` (PNG as base64) once the person authorized screen
//!   recording for the app; the first call queues the Allow / Don't Allow sheet and fails.
//!
//! The `reconfigured` event carries `{changes: [{id, flags}], displays: [...]}`; the flag bits are
//! `display_info::ChangeFlags`. The first list seen is the baseline and sends nothing.
use super::privacy_permissions::{self, Kind};
use super::{app_has_permission, screens, Event, Shell, SystemService};
use crate::gfx::rotate::Orientation;
use crate::shell::apphost;
use bifrost_core::json::Json;
use bifrost_ui::capture_access::{Media, Status};
use bifrost_ui::display_info::*;
use bifrost_ui::wire_json::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

pub const SERVICE: &str = "display";
pub const EVENT_RECONFIGURED: &str = "reconfigured";
/// The manifest permission that allows `set_mode`.
pub const PERMISSION_CONFIG: &str = "display-config";
/// The manifest capability required in addition to the person's screen-recording consent.
pub const PERMISSION_SCREEN: &str = "screen";
/// The one app whose manifest may hold `screen` (Color Meter; `app_policy::SCREEN_APP_ID` is this id). Every other app
/// takes pictures through the `screenshot` service, which the shell runs.
pub const SCREEN_CAPTURE_APP_ID: &str = "is.olibuijr.colormeter.desktop";
/// The id of the one display the shell drives.
pub const DISPLAY_ID: DisplayId = 1;

/// The refresh rate of the current output mode in hertz, reported by the compositor connection (0 until it is known).
pub static OUTPUT_REFRESH_HZ: AtomicU32 = AtomicU32::new(0);
/// How long an applied display change waits for the person to keep it before it goes back.
pub const CONFIRM_SECS: f64 = 15.0;

/// An output mode the shell asks the window server for. `refresh_mhz` is in millihertz; 0 takes the fastest refresh
/// the window server has at that size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeRequest {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: i32,
}

/// The request waiting for the Wayland output client to send it (taken once).
pub static MODE_REQUEST: Mutex<Option<ModeRequest>> = Mutex::new(None);
/// The window server's answer to the last request sent: `Some(true)` applied, `Some(false)` refused, `None` while it
/// is out or nothing was sent. Taken once by the service's tick.
pub static MODE_RESULT: Mutex<Option<bool>> = Mutex::new(None);

/// The request for `target`, with the refresh `hz` the person chose (0 keeps the target's own).
pub fn mode_request(target: &DisplayMode, hz: f32) -> ModeRequest {
    let hz = if hz > 0.0 { hz } else { target.refresh_hz };
    ModeRequest { width: target.pixel_width, height: target.pixel_height, refresh_mhz: (hz * 1000.0).round() as i32 }
}

/// The file (in the shell's data directory) that keeps the Refresh Rate choice.
pub const REFRESH_CHOICE_FILE: &str = "display-refresh";
/// The file that keeps the Resolution choice: "native" or "WIDTHxHEIGHT".
pub const RESOLUTION_CHOICE_FILE: &str = "display-resolution";

/// The text stored for a Resolution choice: `None` is the panel's native size.
pub fn resolution_text(choice: Option<(u32, u32)>) -> String {
    choice.map_or_else(|| "native".to_string(), |(w, h)| format!("{w}x{h}"))
}

/// The Resolution choice a stored text names (`None` for native, or when the text is not a size).
pub fn parse_resolution(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.trim().split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// The Resolution choice kept in the data directory: native when the file is missing.
pub fn load_resolution(dir: &std::path::Path) -> Option<(u32, u32)> {
    std::fs::read_to_string(dir.join(RESOLUTION_CHOICE_FILE)).ok().and_then(|t| parse_resolution(&t))
}

/// Keeps the Resolution choice in the data directory.
pub fn save_resolution(dir: &std::path::Path, choice: Option<(u32, u32)>) -> std::io::Result<()> {
    std::fs::write(dir.join(RESOLUTION_CHOICE_FILE), resolution_text(choice))
}

/// Settings > Displays > Resolution row label: "Native" or "2560 × 1440".
pub fn resolution_label(choice: Option<(u32, u32)>) -> String {
    choice.map_or_else(|| "Native".to_string(), |(w, h)| format!("{w} × {h}"))
}

/// Resolution picker row `k`: 0 is the native size, n the (n-1)th size the panel offers (largest first).
pub fn resolution_at(k: usize, sizes: &[(u32, u32)]) -> Option<(u32, u32)> {
    k.checked_sub(1).and_then(|i| sizes.get(i).copied())
}

/// Settings > Displays > Refresh Rate: the fastest refresh the panel offers at its size (the default), the panel's
/// preferred mode, or one rate in hertz.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RefreshChoice {
    Max,
    Native,
    Hz(f32),
}

impl RefreshChoice {
    /// The choice file's text: `max`, `native`, or the rate with three decimals (`164.850`).
    pub fn text(self) -> String {
        match self {
            RefreshChoice::Max => "max".into(),
            RefreshChoice::Native => "native".into(),
            RefreshChoice::Hz(hz) => format!("{hz:.3}"),
        }
    }

    /// Reads the file's text. Anything missing or not a positive rate is `max`.
    pub fn parse(text: &str) -> RefreshChoice {
        match text.trim() {
            "native" => RefreshChoice::Native,
            t => match t.parse::<f32>() {
                Ok(hz) if hz.is_finite() && hz > 0.0 => RefreshChoice::Hz(hz),
                _ => RefreshChoice::Max,
            },
        }
    }

    /// The choice kept in `dir` (`max` when the file is missing).
    pub fn load(dir: &std::path::Path) -> RefreshChoice {
        std::fs::read_to_string(dir.join(REFRESH_CHOICE_FILE)).map_or(RefreshChoice::Max, |t| RefreshChoice::parse(&t))
    }

    /// Writes the choice into `dir`.
    pub fn save(self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(dir.join(REFRESH_CHOICE_FILE), self.text())
    }

    /// Whether two choices are the same (rates match to the millihertz).
    pub fn same(self, other: RefreshChoice) -> bool {
        match (self, other) {
            (RefreshChoice::Hz(a), RefreshChoice::Hz(b)) => (a - b).abs() < 0.0005,
            (a, b) => a == b,
        }
    }

    /// The row's text: `Highest Available`, `Native`, or the rate (`164.85 Hz`, `144 Hz`).
    pub fn label(self) -> String {
        match self {
            RefreshChoice::Max => "Highest Available".into(),
            RefreshChoice::Native => "Native".into(),
            RefreshChoice::Hz(hz) => rate_label(hz),
        }
    }
}

/// A refresh rate as the picker shows it: two decimals at most, no trailing zeros (`164.85 Hz`, `60 Hz`).
pub fn rate_label(hz: f32) -> String {
    let s = format!("{hz:.2}");
    format!("{} Hz", s.trim_end_matches('0').trim_end_matches('.'))
}

/// The choice picked from the Refresh Rate page: row 0 is Highest Available, row n the (n-1)th rate the panel offers.
pub fn choice_at(row: usize, rates: &[f32]) -> RefreshChoice {
    match row.checked_sub(1).and_then(|i| rates.get(i)) {
        Some(&hz) => RefreshChoice::Hz(hz),
        None => RefreshChoice::Max,
    }
}

/// Applies a Refresh Rate choice to the running output at `width` x `height` through the output-management request
/// (`MODE_REQUEST`, the same path as `set_mode`). Highest Available is refresh 0, the fastest the window server has at
/// that size.
pub fn request_refresh(choice: RefreshChoice, width: u32, height: u32) {
    let refresh_mhz = match choice {
        RefreshChoice::Hz(hz) => (hz * 1000.0).round() as i32,
        RefreshChoice::Max | RefreshChoice::Native => 0,
    };
    post(&MODE_REQUEST, Some(ModeRequest { width, height, refresh_mhz }));
}

/// An applied display change waiting to be kept: the mode it replaced, and the time it goes back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Confirm {
    pub previous: DisplayMode,
    pub until: f64,
}

impl Confirm {
    pub fn start(previous: DisplayMode, now: f64) -> Self {
        Confirm { previous, until: now + CONFIRM_SECS }
    }

    pub fn expired(&self, now: f64) -> bool {
        now >= self.until
    }
}

fn post(slot: &Mutex<Option<ModeRequest>>, request: Option<ModeRequest>) {
    *slot.lock().unwrap_or_else(|e| e.into_inner()) = request;
}

/// What the display's sysfs entry says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hardware {
    pub connector: Option<String>,
    pub edid: Option<Edid>,
    pub modes: Vec<(u32, u32)>,
}

impl Hardware {
    /// Reads the first connected connector (an internal panel first) from `/sys/class/drm`.
    pub fn read() -> Hardware {
        let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return Hardware::default() };
        let mut found: Vec<(String, std::path::PathBuf)> = dir
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let connector = name.split_once('-')?.1.to_string();
                let connected = std::fs::read_to_string(e.path().join("status")).ok()?.trim() == "connected";
                connected.then(|| (connector, e.path()))
            })
            .collect();
        found.sort_by_key(|(name, _)| !is_internal(name));
        let Some((connector, path)) = found.into_iter().next() else { return Hardware::default() };
        Hardware {
            edid: std::fs::read(path.join("edid")).ok().and_then(|b| parse_edid(&b)),
            modes: std::fs::read_to_string(path.join("modes")).map(|t| parse_modes(&t)).unwrap_or_default(),
            connector: Some(connector),
        }
    }
}

/// Whether a connector name is a panel built into the machine.
pub fn is_internal(connector: &str) -> bool {
    ["eDP", "LVDS", "DSI"].iter().any(|p| connector.starts_with(p))
}

/// The sizes in a connector's `modes` file ("1920x1080" per line, an interlaced mode ends in "i"), without repeats.
pub fn parse_modes(text: &str) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for line in text.lines() {
        let Some((w, h)) = line.trim().trim_end_matches('i').split_once('x') else { continue };
        let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) else { continue };
        if !out.contains(&(w, h)) {
            out.push((w, h));
        }
    }
    out
}

/// Everything the display list is built from.
#[derive(Clone, Debug)]
pub struct Inputs {
    pub w_px: i32,
    pub h_px: i32,
    pub scale: f32,
    pub top_px: i32,
    pub bottom_px: i32,
    pub refresh_hz: f32,
    pub rotation: Rotation,
    /// Used for the built-in flag when the connector name is not known.
    pub battery_present: bool,
    pub hardware: Hardware,
}

/// The display list: the one output the shell drives.
pub fn build(i: &Inputs) -> Vec<DisplayInfo> {
    let screens = screens::snapshot(i.w_px, i.h_px, i.scale, i.top_px, i.bottom_px);
    let bounds = screens.all().first().map(|s| s.frame).unwrap_or_default();
    let (pw, ph) = (i.w_px.max(0) as u32, i.h_px.max(0) as u32);
    let mode = DisplayMode { width: bounds.w.round() as u32, height: bounds.h.round() as u32, pixel_width: pw, pixel_height: ph, refresh_hz: i.refresh_hz, usable: true };
    let mut modes = vec![mode];
    for (w, h) in &i.hardware.modes {
        if (*w, *h) != (pw, ph) {
            modes.push(DisplayMode::pixels(*w, *h, 0.0));
        }
    }
    let edid = i.hardware.edid;
    let name = match edid {
        Some(e) if e.vendor != 0 => format!("{} {:04X}", vendor_letters(e.vendor), e.model),
        _ => "Display".to_string(),
    };
    vec![DisplayInfo {
        id: DISPLAY_ID,
        name,
        is_main: true,
        is_builtin: i.hardware.connector.as_deref().map_or(i.battery_present, is_internal),
        is_active: true,
        is_online: true,
        is_asleep: false,
        bounds,
        pixel_width: pw,
        pixel_height: ph,
        mode,
        modes,
        rotation: i.rotation,
        mirror_of: None,
        vendor: edid.map_or(0, |e| e.vendor as u32),
        model: edid.map_or(0, |e| e.model as u32),
        serial: edid.map_or(0, |e| e.serial),
        size_mm: edid.map_or((0.0, 0.0), |e| e.size_mm),
    }]
}

fn rotation_of(o: Orientation) -> Rotation {
    match o {
        Orientation::Portrait => Rotation::Deg0,
        Orientation::LandscapeLeft => Rotation::Deg90,
        Orientation::PortraitUpsideDown => Rotation::Deg180,
        Orientation::LandscapeRight => Rotation::Deg270,
    }
}

#[derive(Default)]
pub struct DisplayServices {
    /// The sysfs reading, kept until the output size changes.
    hardware: Option<((i32, i32), Hardware)>,
    /// The list last reported, to tell a change from a quiet tick.
    last: Option<Vec<DisplayInfo>>,
    /// A change asked for and not answered yet: the mode it replaces.
    asked: Option<DisplayMode>,
    /// An applied change waiting for the person to keep it.
    confirm: Option<Confirm>,
    /// A revert asked for and not answered yet.
    reverting: bool,
}

impl DisplayServices {
    /// Asks the window server for `target`, with the refresh `hz` (0: the target's own).
    fn ask(&mut self, target: DisplayMode, hz: f32) {
        post(&MODE_REQUEST, Some(mode_request(&target, hz)));
    }

    fn current(&mut self, shell: &Shell) -> Vec<DisplayInfo> {
        let key = (shell.w, shell.h);
        if self.hardware.as_ref().map(|h| h.0) != Some(key) {
            self.hardware = Some((key, Hardware::read()));
        }
        let (top, bottom) = if shell.desk_active() {
            let m = shell.desk_metrics();
            (m.menubar, m.dock_bottom)
        } else {
            (shell.layout.status_h, 0)
        };
        let hz = shell.info.display_mode.map(|m| m.2 as f32).filter(|h| *h > 0.0).unwrap_or(OUTPUT_REFRESH_HZ.load(Ordering::Relaxed) as f32);
        build(&Inputs {
            w_px: shell.w,
            h_px: shell.h,
            scale: shell.layout.s,
            top_px: top,
            bottom_px: bottom,
            refresh_hz: hz,
            rotation: rotation_of(shell.orientation()),
            battery_present: shell.info.battery_present,
            hardware: self.hardware.as_ref().map(|h| h.1.clone()).unwrap_or_default(),
        })
    }
}

impl SystemService for DisplayServices {
    fn name(&self) -> &'static str {
        SERVICE
    }

    fn call(&mut self, shell: &mut Shell, app_id: &str, method: &str, params: &Json, t: f64) -> Result<Json, String> {
        let list = self.current(shell);
        let display = |params: &Json| -> Result<DisplayInfo, String> {
            match u32_of(params, "id") {
                Some(id) => by_id(&list, id).cloned().ok_or_else(|| format!("display: no display {id}")),
                None => main_display(&list).cloned().ok_or_else(|| "display: no display".to_string()),
            }
        };
        match method {
            "list" => match str_of(params, "kind").as_deref().unwrap_or("active") {
                "active" => Ok(Json::Arr(active(&list).iter().map(|d| d.to_json()).collect())),
                "online" => Ok(Json::Arr(online(&list).iter().map(|d| d.to_json()).collect())),
                other => Err(format!("display: unknown list kind {other}")),
            },
            "main" => main_display(&list).map(|d| d.to_json()).ok_or_else(|| "display: no display".to_string()),
            "info" => Ok(display(params)?.to_json()),
            "modes" => Ok(Json::Arr(display(params)?.modes.iter().map(DisplayMode::to_json).collect())),
            "set_mode" => {
                if !app_has_permission(app_id, PERMISSION_CONFIG) {
                    return Err(format!("display: set_mode needs the {PERMISSION_CONFIG} permission"));
                }
                let d = display(params)?;
                let (w, h) = (u32_of(params, "width").ok_or("set_mode needs width")?, u32_of(params, "height").ok_or("set_mode needs height")?);
                let hz = f32_of(params, "refresh_hz").unwrap_or(0.0);
                let want = d.modes.iter().find(|m| m.pixel_width == w && m.pixel_height == h).copied().ok_or_else(|| format!("display: no {w}x{h} mode"))?;
                let current = want.pixel_width == d.mode.pixel_width && want.pixel_height == d.mode.pixel_height && (hz == 0.0 || (hz - d.mode.refresh_hz).abs() < 0.5);
                if current {
                    return Ok(obj(vec![("applied", Json::Bool(true)), ("requested", Json::Bool(false))]));
                }
                if self.asked.is_some() || self.reverting || self.confirm.is_some() {
                    return Err("display: a mode change is already under way; keep or revert it first".into());
                }
                self.asked = Some(d.mode);
                self.ask(want, hz);
                Ok(obj(vec![("applied", Json::Bool(false)), ("requested", Json::Bool(true))]))
            }
            "keep_mode" | "revert_mode" => {
                if !app_has_permission(app_id, PERMISSION_CONFIG) {
                    return Err(format!("display: {method} needs the {PERMISSION_CONFIG} permission"));
                }
                if method == "keep_mode" {
                    return match self.confirm.take() {
                        Some(_) => Ok(obj(vec![("kept", Json::Bool(true))])),
                        None => Err("display: no display change to keep".into()),
                    };
                }
                let Some(c) = self.confirm.take() else { return Err("display: no display change to revert".into()) };
                self.reverting = true;
                self.ask(c.previous, 0.0);
                Ok(obj(vec![("reverting", Json::Bool(true))]))
            }
            "set_rotation" | "mirror" => Err(format!("display: {method} is not available: the shell drives one panel it does not rotate or mirror")),
            "capture" => {
                display(params)?;
                require_screen_recording(app_id)?;
                let img = shell.capture_frame(t);
                let png = crate::pngenc::encode_rgb(img.w, img.h, &img.px);
                Ok(obj(vec![("width", num(img.w as f64)), ("height", num(img.h as f64)), ("png", text(&base64_encode(&png)))]))
            }
            _ => Err(format!("display has no method {method}")),
        }
    }

    fn tick(&mut self, shell: &mut Shell, t: f64) -> Vec<Event> {
        let now = self.current(shell);
        let changes = match &self.last {
            Some(prev) => diff(prev, &now),
            None => Vec::new(),
        };
        let body = obj(vec![
            ("changes", Json::Arr(changes.iter().map(|(id, f)| obj(vec![("id", num(*id)), ("flags", num(f.0))])).collect())),
            ("displays", list_to_json(&now)),
        ]);
        self.last = Some(now);
        let mut events = Vec::new();
        if !changes.is_empty() {
            events.push(Event { app: None, event: EVENT_RECONFIGURED.into(), body });
        }
        // the window server's answer to the last request
        let answer = MODE_RESULT.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(ok) = answer {
            if let Some(previous) = self.asked.take() {
                if ok {
                    self.confirm = Some(Confirm::start(previous, t));
                }
                events.push(mode_event(MODE_CHANGED, ok, ok));
            } else if self.reverting {
                self.reverting = false;
                events.push(mode_event(MODE_REVERTED, ok, false));
            }
        }
        // an applied change the person did not keep goes back
        if self.confirm.is_some_and(|c| c.expired(t)) {
            if let Some(c) = self.confirm.take() {
                self.reverting = true;
                self.ask(c.previous, 0.0);
                events.push(mode_event(MODE_REVERTING, true, false));
            }
        }
        events
    }
}

/// The event names of a mode change: `mode_changed` (applied or refused), `mode_reverting` (the revert was asked for,
/// after the confirmation timed out) and `mode_reverted` (the window server answered the revert).
pub const MODE_CHANGED: &str = "mode_changed";
pub const MODE_REVERTING: &str = "mode_reverting";
pub const MODE_REVERTED: &str = "mode_reverted";

fn mode_event(name: &str, applied: bool, with_revert_in: bool) -> Event {
    let mut fields = vec![("applied", Json::Bool(applied))];
    if with_revert_in {
        fields.push(("revert_in", num(CONFIRM_SECS)));
    }
    Event { app: None, event: name.into(), body: obj(fields) }
}

/// Lets a call through only when the app declares `screen` and the person authorized screen recording. The first call
/// of an app that has not been asked queues the sheet and fails; a refusal fails with its reason.
pub fn require_screen_recording(app_id: &str) -> Result<(), String> {
    // Capture the app does itself (frames, window images, streams) is for the one first-party app the launch policy
    // grants it to; a package that declares `screen` anyway is launched without the grant and gets nothing here.
    if !screen_capture_declared(app_id, super::app_has_permission(app_id, PERMISSION_SCREEN)) {
        return Err("the screen permission is not declared by this app".into());
    }
    require_screenshot_consent(app_id)
}

/// Whether `app_id` may capture the screen itself: it is the app the policy grants `screen` to and its manifest declares it.
pub fn screen_capture_declared(app_id: &str, declared: bool) -> bool {
    declared && app_id == SCREEN_CAPTURE_APP_ID
}

/// The person's screen-recording consent alone, for the shell-mediated `screenshot` service: the shell takes the
/// picture and saves it, so the app needs no `screen` entry in its manifest. The first call of an app that has not
/// been asked queues the sheet and fails; a refusal fails with its reason.
pub fn require_screenshot_consent(app_id: &str) -> Result<(), String> {
    match privacy_permissions::access(app_id).status(Media::Screen) {
        Status::Authorized => Ok(()),
        Status::NotDetermined => {
            let name = apphost::installed().iter().find(|a| a.manifest.id == app_id).map(|a| a.manifest.name.clone()).unwrap_or_else(|| app_id.to_string());
            privacy_permissions::queue_prompt(app_id, &name, Kind::Media(Media::Screen), "");
            Err("screen recording is not authorized yet: the person has been asked".into())
        }
        Status::Restricted => Err("screen recording is restricted on this system".into()),
        Status::Denied => Err("the person did not allow screen recording for this app".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_color_meter_captures_the_screen_itself() {
        assert!(screen_capture_declared(SCREEN_CAPTURE_APP_ID, true));
        assert!(!screen_capture_declared(SCREEN_CAPTURE_APP_ID, false), "the manifest must declare it");
        assert!(!screen_capture_declared("is.olibuijr.preview.desktop", true), "a legacy `screen` entry grants nothing");
        assert!(!screen_capture_declared("is.example.recorder", true));
    }

    #[test]
    fn the_resolution_choice_is_native_by_default_and_round_trips_a_size() {
        let dir = std::env::temp_dir().join(format!("display-resolution-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_resolution(&dir), None, "a missing file is the native size");
        save_resolution(&dir, Some((1920, 1200))).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(RESOLUTION_CHOICE_FILE)).unwrap(), "1920x1200");
        assert_eq!(load_resolution(&dir), Some((1920, 1200)));
        save_resolution(&dir, None).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(RESOLUTION_CHOICE_FILE)).unwrap(), "native");
        assert_eq!(load_resolution(&dir), None);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(parse_resolution("garbage"), None);
        assert_eq!(resolution_at(0, &[(2560, 1600)]), None, "row 0 is native");
        assert_eq!(resolution_at(1, &[(2560, 1600), (1920, 1200)]), Some((2560, 1600)));
        assert_eq!(resolution_at(3, &[(2560, 1600)]), None, "a row past the list is refused");
        assert_eq!(resolution_label(Some((2560, 1440))), "2560 × 1440");
    }

    #[test]
    fn the_refresh_choice_file_defaults_to_highest_and_round_trips_a_picked_rate() {
        let dir = std::env::temp_dir().join(format!("display-refresh-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(RefreshChoice::load(&dir), RefreshChoice::Max, "a missing file is the highest rate");
        let rates = [164.85, 144.0, 120.0, 100.0, 60.0];
        let picked = choice_at(1, &rates);
        assert_eq!(picked, RefreshChoice::Hz(164.85));
        picked.save(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(REFRESH_CHOICE_FILE)).unwrap(), "164.850", "the rate with three decimals");
        assert_eq!(RefreshChoice::load(&dir), RefreshChoice::Hz(164.85));
        choice_at(0, &rates).save(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(REFRESH_CHOICE_FILE)).unwrap(), "max");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refresh_text_and_labels_read_back_as_the_picker_shows_them() {
        assert_eq!(RefreshChoice::parse("native\n"), RefreshChoice::Native);
        assert_eq!(RefreshChoice::parse("bogus"), RefreshChoice::Max, "anything unreadable is the highest rate");
        assert_eq!(RefreshChoice::parse("-5"), RefreshChoice::Max);
        assert_eq!(RefreshChoice::Hz(60.0).label(), "60 Hz");
        assert_eq!(RefreshChoice::Hz(164.85).label(), "164.85 Hz");
        assert_eq!(RefreshChoice::Max.label(), "Highest Available");
        assert!(RefreshChoice::Hz(144.0).same(RefreshChoice::Hz(144.0004)));
        assert!(!RefreshChoice::Hz(144.0).same(RefreshChoice::Hz(120.0)));
        assert!(!RefreshChoice::Max.same(RefreshChoice::Native));
    }

    fn inputs() -> Inputs {
        Inputs {
            w_px: 2880,
            h_px: 1800,
            scale: 2.0,
            top_px: 48,
            bottom_px: 120,
            refresh_hz: 120.0,
            rotation: Rotation::Deg0,
            battery_present: false,
            hardware: Hardware::default(),
        }
    }

    #[test]
    fn the_one_display_has_the_shells_size_scale_and_refresh() {
        let list = build(&inputs());
        assert_eq!(list.len(), 1);
        let d = &list[0];
        assert!(d.is_main && d.is_active && d.is_online);
        assert_eq!((d.pixel_width, d.pixel_height), (2880, 1800));
        assert_eq!((d.bounds.w, d.bounds.h), (1440.0, 900.0));
        assert_eq!(d.refresh_hz(), 120.0);
        assert_eq!(d.mode.scale(), 2.0);
        assert_eq!(d.modes.len(), 1);
        assert!(!d.is_builtin);
    }

    #[test]
    fn hardware_adds_modes_identity_and_the_built_in_flag() {
        let mut i = inputs();
        i.hardware = Hardware {
            connector: Some("eDP-1".into()),
            edid: Some(Edid { vendor: (4 << 10) | (5 << 5) | 12, model: 0xa0b1, serial: 9, size_mm: (300.0, 190.0) }),
            modes: vec![(2880, 1800), (1920, 1200), (1280, 800)],
        };
        let d = &build(&i)[0];
        assert!(d.is_builtin);
        assert_eq!(d.modes.len(), 3);
        assert_eq!(d.vendor_code(), "DEL");
        assert_eq!((d.model, d.serial, d.size_mm), (0xa0b1, 9, (300.0, 190.0)));
        assert_eq!(d.name, "DEL A0B1");
    }

    #[test]
    fn the_built_in_flag_falls_back_to_the_battery() {
        let mut i = inputs();
        i.battery_present = true;
        assert!(build(&i)[0].is_builtin);
        i.hardware.connector = Some("HDMI-A-1".into());
        assert!(!build(&i)[0].is_builtin);
    }

    #[test]
    fn mode_files_parse_without_repeats() {
        assert_eq!(parse_modes("1920x1080\n1920x1080\n1280x720\n720x480i\nnonsense\n"), vec![(1920, 1080), (1280, 720), (720, 480)]);
        assert!(parse_modes("").is_empty());
    }

    #[test]
    fn internal_connectors_are_recognised() {
        assert!(is_internal("eDP-1") && is_internal("LVDS-1") && is_internal("DSI-1"));
        assert!(!is_internal("DP-2") && !is_internal("HDMI-A-1"));
    }

    #[test]
    fn a_resize_is_reported_as_a_reconfiguration() {
        let before = build(&inputs());
        let mut i = inputs();
        i.w_px = 1920;
        i.h_px = 1080;
        i.scale = 1.0;
        let changes = diff(&before, &build(&i));
        assert_eq!(changes.len(), 1);
        assert!(changes[0].1.contains(ChangeFlags::SET_MODE));
        assert!(diff(&before, &before).is_empty());
    }

    #[test]
    fn a_request_names_the_size_and_the_chosen_or_fastest_refresh() {
        let target = DisplayMode { width: 1280, height: 800, pixel_width: 2560, pixel_height: 1600, refresh_hz: 60.0, usable: true };
        assert_eq!(mode_request(&target, 0.0), ModeRequest { width: 2560, height: 1600, refresh_mhz: 60_000 });
        assert_eq!(mode_request(&target, 59.94).refresh_mhz, 59_940);
        let fastest = DisplayMode { refresh_hz: 0.0, ..target };
        assert_eq!(mode_request(&fastest, 0.0).refresh_mhz, 0, "0 lets the window server pick its fastest refresh");
    }

    #[test]
    fn an_applied_change_is_kept_for_fifteen_seconds_and_then_goes_back() {
        let previous = DisplayMode { width: 1280, height: 800, pixel_width: 1280, pixel_height: 800, refresh_hz: 60.0, usable: true };
        let c = Confirm::start(previous, 100.0);
        assert_eq!(CONFIRM_SECS, 15.0);
        assert!(!c.expired(114.9));
        assert!(c.expired(115.0));
        assert_eq!(c.previous, previous);
    }

    #[test]
    fn mode_events_say_whether_a_revert_is_offered() {
        let e = mode_event(MODE_CHANGED, true, true);
        assert_eq!(e.event, "mode_changed");
        assert_eq!(e.body.get("revert_in").and_then(|v| v.number()), Some(15.0));
        let refused = mode_event(MODE_CHANGED, false, false);
        assert!(refused.body.get("revert_in").is_none());
    }

    #[test]
    fn orientations_map_to_quarter_turns() {
        assert_eq!(rotation_of(Orientation::Portrait), Rotation::Deg0);
        assert_eq!(rotation_of(Orientation::LandscapeLeft).degrees(), 90);
        assert_eq!(rotation_of(Orientation::PortraitUpsideDown).degrees(), 180);
        assert_eq!(rotation_of(Orientation::LandscapeRight).degrees(), 270);
    }
}
