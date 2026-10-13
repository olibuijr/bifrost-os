//! The `display` service: the displays an app can read and, with permission, change or capture. It builds
//! `bifrost_ui::display_info::DisplayInfo` records for the output the shell drives (size, scale and bars from the shell,
//! the refresh rate from the output mode, identity and the mode list from the display's sysfs entry and EDID), and it
//! pushes `reconfigured` to every app when the list changes.
//!
//! Methods (JSON parameters):
//! - `list {kind}` answers the displays: `active` (default) or `online`.
//! - `main {}` and `info {id}` answer one display; `modes {id}` answers its mode list.
//! - `set_mode {id, width, height, refresh_hz}` needs the `display-config` permission. The shell does not own the
//!   output mode in a desktop session (the compositor does), so a mode other than the current one is recorded as the
//!   requested mode and answered `{applied: false, pending: true}`; asking for the current mode answers `applied: true`.
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

pub const SERVICE: &str = "display";
pub const EVENT_RECONFIGURED: &str = "reconfigured";
/// The manifest permission that allows `set_mode`.
pub const PERMISSION_CONFIG: &str = "display-config";
/// The id of the one display the shell drives.
pub const DISPLAY_ID: DisplayId = 1;

/// The refresh rate of the current output mode in hertz, reported by the compositor connection (0 until it is known).
pub static OUTPUT_REFRESH_HZ: AtomicU32 = AtomicU32::new(0);

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
    /// The mode `set_mode` asked for last.
    requested: Option<DisplayMode>,
}

impl DisplayServices {
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
                let applied = want.pixel_width == d.mode.pixel_width && want.pixel_height == d.mode.pixel_height && (hz == 0.0 || (hz - d.mode.refresh_hz).abs() < 0.5);
                if !applied {
                    self.requested = Some(DisplayMode { refresh_hz: if hz > 0.0 { hz } else { want.refresh_hz }, ..want });
                }
                Ok(obj(vec![("applied", Json::Bool(applied)), ("pending", Json::Bool(!applied))]))
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

    fn tick(&mut self, shell: &mut Shell, _t: f64) -> Vec<Event> {
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
        if changes.is_empty() {
            Vec::new()
        } else {
            vec![Event { app: None, event: EVENT_RECONFIGURED.into(), body }]
        }
    }
}

/// Lets a call through only when the person authorized screen recording for the app. The first call of an app that
/// has not been asked queues the sheet and fails; a refusal fails with its reason.
pub fn require_screen_recording(app_id: &str) -> Result<(), String> {
    match privacy_permissions::access(app_id).status(Media::Screen) {
        Status::Authorized => Ok(()),
        Status::NotDetermined => {
            let name = apphost::installed().iter().find(|a| a.manifest.id == app_id).map(|a| a.manifest.name.clone()).unwrap_or_else(|| app_id.to_string());
            privacy_permissions::queue_prompt(app_id, &name, Kind::Media(Media::Screen));
            Err("screen recording is not authorized yet: the person has been asked".into())
        }
        Status::Restricted => Err("screen recording is restricted on this system".into()),
        Status::Denied => Err("the person did not allow screen recording for this app".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn orientations_map_to_quarter_turns() {
        assert_eq!(rotation_of(Orientation::Portrait), Rotation::Deg0);
        assert_eq!(rotation_of(Orientation::LandscapeLeft).degrees(), 90);
        assert_eq!(rotation_of(Orientation::PortraitUpsideDown).degrees(), 180);
        assert_eq!(rotation_of(Orientation::LandscapeRight).degrees(), 270);
    }
}
