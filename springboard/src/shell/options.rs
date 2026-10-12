//! The user's Settings choices, persisted as keys in `settings.ini` (see
//! `prefs`). `Options` is the typed view the pages edit; every setter saves.

use super::prefs::Prefs;
use crate::bare::cpuprofile::Pref;
use super::ui::{TEXT_SCALE_MAX, TEXT_SCALE_MIN};

/// The Auto-Lock choices as (label, ms); 0 is Never.
/// Energy "Put the computer to sleep when the display is off": (label, minutes), 0 = never.
pub const SYSTEM_SLEEP: [(&str, u32); 7] = [("1 minute", 1), ("5 minutes", 5), ("10 minutes", 10), ("15 minutes", 15), ("30 minutes", 30), ("1 hour", 60), ("Never", 0)];
pub const AUTO_LOCK: [(&str, u32); 5] = [("30 seconds", 30_000), ("1 minute", 60_000), ("2 minutes", 120_000), ("5 minutes", 300_000), ("Never", 0)];
/// The Text Size slider's notches, the bifrost Dynamic Type steps.
pub const TEXT_SIZES: [f32; 7] = [TEXT_SCALE_MIN, 0.9, 0.95, 1.0, 1.1, 1.2, TEXT_SCALE_MAX];
/// Battery › Power Mode choices as (label, preference).
pub const POWER_MODES: [(&str, Pref); 4] = [("Automatic", Pref::Auto), ("Performance", Pref::Performance), ("Balanced", Pref::Balanced), ("Battery Saver", Pref::Battery)];
/// Home Screen & Dock › App Icons choices.
pub const ICON_SIZES: [&str; 2] = ["More", "Bigger"];

/// A menu-bar extra the user can show or hide (Settings › Menu Bar, like Golden Gate Control Center settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuExtra { Cpu, Memory, Battery, Network, ControlCenter }
impl MenuExtra {
    pub const ALL: [MenuExtra; 5] = [MenuExtra::Cpu, MenuExtra::Memory, MenuExtra::Battery, MenuExtra::Network, MenuExtra::ControlCenter];
    pub const ALL_BITS: u8 = 0b1_1111;
    pub fn bit(self) -> u8 { 1 << self as u8 }
    pub fn label(self) -> &'static str {
        match self { MenuExtra::Cpu => "CPU Usage", MenuExtra::Memory => "Memory Usage", MenuExtra::Battery => "Battery", MenuExtra::Network => "Wi-Fi and Network", MenuExtra::ControlCenter => "Control Center" }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    /// idle time before the screen turns off, 0 = never
    pub auto_lock_ms: u32,
    /// minutes the display stays off before the system suspends; 0 = never
    pub system_sleep_min: u32,
    pub text_scale: f32,
    pub bold_text: bool,
    pub auto_caps: bool,
    /// Keyboard: double-tap shift locks caps
    pub enable_caps_lock: bool,
    /// Keyboard: "." Shortcut, double space types a period
    pub period_shortcut: bool,
    pub clock_24h: bool,
    pub keyboard_clicks: bool,
    pub lock_sound: bool,
    /// Keyboard Haptics; only offered when the device has a vibrator
    pub keyboard_haptics: bool,
    /// Silence notification banners and screen wakes; alarms still ring.
    pub dnd: bool,
    pub assistive_touch: bool,
    /// index into ICON_SIZES
    pub icon_size: usize,
    /// Settings › Menu Bar: which menu extras show, a `MenuExtra` bit set (all on by default)
    pub menu_extras: u8,
    /// Settings > Screen Saver: index into `screensaver::SAVERS`
    pub screensaver_module: usize,
    /// Settings > Screen Saver > Start After: idle minutes before the saver starts, 0 = never
    pub screensaver_minutes: u32,
    pub auto_join: bool,
    /// ringer and alerts 0..=1
    pub volume: f32,
    /// Battery › Low Power Mode (see `low_battery`)
    pub low_power: bool,
    /// Battery Percentage: the number beside the status-bar battery
    pub battery_percent: bool,
    /// Battery › Power Mode: the CPU profile rule (see `bare::cpuprofile`)
    pub power_profile: Pref,
    /// Home Screen & Dock: recently used apps after the dock separator
    pub dock_recents: bool,
    /// General › Software Update › Automatic Updates: download offered updates in the background
    pub auto_update: bool,
    /// General › Nearby: 0 Receiving Off, 1 Contacts Only (devices already seen nearby), 2 Everyone
    pub nearby: usize,
    pub handoff: bool,
    pub pip_auto: bool,
    pub bg_refresh: bool,
    pub four_finger: bool,
    /// General › Gestures: a swipe from the bottom-left corner takes a screenshot
    pub corner_swipe: bool,
    /// General › Software Update › Back Up Before Updates: a backup (see `backup`) before an update installs
    pub backup_before_update: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { auto_lock_ms: 0, system_sleep_min: 0, text_scale: 1.0, bold_text: false, auto_caps: true, enable_caps_lock: true, period_shortcut: true, clock_24h: true, keyboard_clicks: true, lock_sound: true, keyboard_haptics: true, dnd: false, assistive_touch: false, icon_size: 0, auto_join: true, volume: 0.7, low_power: false, battery_percent: true, power_profile: Pref::Auto, dock_recents: true, auto_update: true, nearby: 1, handoff: true, pip_auto: true, bg_refresh: true, four_finger: true, corner_swipe: false, backup_before_update: true, menu_extras: MenuExtra::ALL_BITS, screensaver_module: 0, screensaver_minutes: 0 }
    }
}

const KEYS: [&str; 31] = ["auto_lock_ms", "system_sleep_minutes", "text_scale", "bold_text", "auto_caps", "enable_caps_lock", "period_shortcut", "clock_24h", "keyboard_clicks", "lock_sound", "keyboard_haptics", "dnd", "assistive_touch", "icon_size", "auto_join", "volume", "low_power", "battery_percent", "power_profile", "dock_recents", "auto_update", "nearby", "handoff", "pip_auto", "bg_refresh", "four_finger", "corner_swipe", "backup_before_update", "menu_extras", "screensaver_module", "screensaver_minutes"];

impl Options {
    /// Reads every key, keeping the default for a missing or malformed one.
    pub fn load(p: &Prefs) -> Self {
        let d = Options::default();
        let flag = |k: &str, default: bool| p.get_bool(k, default);
        Options {
            auto_lock_ms: p.get_parsed("auto_lock_ms").unwrap_or(d.auto_lock_ms),
            system_sleep_min: p.get_parsed("system_sleep_minutes").unwrap_or(d.system_sleep_min),
            text_scale: p.get_parsed::<f32>("text_scale").map_or(d.text_scale, |v| v.clamp(TEXT_SCALE_MIN, TEXT_SCALE_MAX)),
            bold_text: flag("bold_text", d.bold_text),
            auto_caps: flag("auto_caps", d.auto_caps),
            enable_caps_lock: flag("enable_caps_lock", d.enable_caps_lock),
            period_shortcut: flag("period_shortcut", d.period_shortcut),
            clock_24h: flag("clock_24h", d.clock_24h),
            keyboard_clicks: flag("keyboard_clicks", d.keyboard_clicks),
            lock_sound: flag("lock_sound", d.lock_sound),
            keyboard_haptics: flag("keyboard_haptics", d.keyboard_haptics),
            dnd: flag("dnd", d.dnd),
            assistive_touch: flag("assistive_touch", d.assistive_touch),
            menu_extras: p.get_parsed("menu_extras").unwrap_or(d.menu_extras),
            screensaver_module: p.get_parsed("screensaver_module").filter(|&i: &usize| i < super::screensaver::SAVERS.len()).unwrap_or(d.screensaver_module),
            screensaver_minutes: p.get_parsed("screensaver_minutes").unwrap_or(d.screensaver_minutes),
            icon_size: p.get_parsed("icon_size").filter(|&i| i < ICON_SIZES.len()).unwrap_or(d.icon_size),
            auto_join: flag("auto_join", d.auto_join),
            volume: p.get_parsed::<f32>("volume").map_or(d.volume, |v| v.clamp(0.0, 1.0)),
            low_power: flag("low_power", d.low_power),
            battery_percent: flag("battery_percent", d.battery_percent),
            power_profile: p.get("power_profile").and_then(Pref::parse).unwrap_or(d.power_profile),
            dock_recents: flag("dock_recents", d.dock_recents),
            auto_update: flag("auto_update", d.auto_update),
            nearby: p.get_parsed("nearby").or_else(|| p.get_parsed(concat!("air", "drop"))).filter(|&i: &usize| i < 3).unwrap_or(d.nearby),
            handoff: flag("handoff", d.handoff),
            pip_auto: flag("pip_auto", d.pip_auto),
            bg_refresh: flag("bg_refresh", d.bg_refresh),
            four_finger: flag("four_finger", d.four_finger),
            corner_swipe: flag("corner_swipe", d.corner_swipe),
            backup_before_update: flag("backup_before_update", d.backup_before_update),
        }
    }

    /// Writes every key (only the ones that differ from `p` touch the disk).
    pub fn save(&self, p: &mut Prefs) {
        let flag = |b: bool| if b { "1" } else { "0" };
        let values = [
            self.auto_lock_ms.to_string(),
            self.system_sleep_min.to_string(),
            format!("{:.2}", self.text_scale),
            flag(self.bold_text).into(),
            flag(self.auto_caps).into(),
            flag(self.enable_caps_lock).into(),
            flag(self.period_shortcut).into(),
            flag(self.clock_24h).into(),
            flag(self.keyboard_clicks).into(),
            flag(self.lock_sound).into(),
            flag(self.keyboard_haptics).into(),
            flag(self.dnd).into(),
            flag(self.assistive_touch).into(),
            self.icon_size.to_string(),
            flag(self.auto_join).into(),
            format!("{:.2}", self.volume),
            flag(self.low_power).into(),
            flag(self.battery_percent).into(),
            self.power_profile.key().into(),
            flag(self.dock_recents).into(),
            flag(self.auto_update).into(),
            self.nearby.to_string(),
            flag(self.handoff).into(),
            flag(self.pip_auto).into(),
            flag(self.bg_refresh).into(),
            flag(self.four_finger).into(),
            flag(self.corner_swipe).into(),
            flag(self.backup_before_update).into(),
            self.menu_extras.to_string(),
            self.screensaver_module.to_string(),
            self.screensaver_minutes.to_string(),
        ];
        let pairs: Vec<(&str, &str)> = KEYS.iter().copied().zip(values.iter().map(String::as_str)).collect();
        if let Err(e) = p.set_many(&pairs) {
            eprintln!("springboard: settings.ini: {e}");
        }
    }

    /// The Auto-Lock label for the current choice.
    pub fn system_sleep_label(&self) -> &'static str {
        SYSTEM_SLEEP.iter().find(|(_, m)| *m == self.system_sleep_min).map_or("Never", |(l, _)| l)
    }

    pub fn auto_lock_label(&self) -> &'static str {
        AUTO_LOCK.iter().find(|(_, ms)| *ms == self.auto_lock_ms).map_or("Never", |(l, _)| l)
    }

    /// The Text Size notch (0..TEXT_SIZES.len()) nearest the current scale.
    pub fn text_size_step(&self) -> usize {
        TEXT_SIZES.iter().enumerate().min_by(|a, b| (a.1 - self.text_scale).abs().total_cmp(&(b.1 - self.text_scale).abs())).map_or(3, |(i, _)| i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_round_trip() {
        let mut p = Prefs::memory();
        let d = Options::load(&p);
        assert_eq!(d, Options::default());
        assert_eq!(d.auto_lock_label(), "Never");
        assert_eq!(d.text_size_step(), 3);
        let o = Options { auto_lock_ms: 0, system_sleep_min: 15, text_scale: 1.2, bold_text: true, auto_caps: false, enable_caps_lock: false, period_shortcut: false, clock_24h: false, keyboard_clicks: false, lock_sound: false, keyboard_haptics: false, dnd: true, assistive_touch: true, icon_size: 1, auto_join: false, volume: 0.25, low_power: true, battery_percent: false, power_profile: Pref::Battery, dock_recents: false, auto_update: false, nearby: 2, handoff: false, pip_auto: false, bg_refresh: false, four_finger: false, corner_swipe: true, backup_before_update: false, menu_extras: 0b101, screensaver_module: 2, screensaver_minutes: 5 };
        o.save(&mut p);
        assert_eq!(p.get("auto_lock_ms"), Some("0"));
        assert_eq!(p.get("system_sleep_minutes"), Some("15"));
        assert_eq!(p.get("text_scale"), Some("1.20"));
        assert_eq!(p.get("bold_text"), Some("1"));
        assert_eq!(p.get("keyboard_clicks"), Some("0"));
        assert_eq!(p.get("keyboard_haptics"), Some("0"));
        assert_eq!(p.get("volume"), Some("0.25"));
        assert_eq!(p.get("low_power"), Some("1"));
        assert_eq!(p.get("power_profile"), Some("battery"));
        let back = Options::load(&p);
        assert_eq!(back, o);
        assert_eq!(back.auto_lock_label(), "Never");
        assert_eq!(back.text_size_step(), 5);
    }

    #[test]
    fn malformed_values_fall_back_and_ranges_clamp() {
        let mut p = Prefs::memory();
        p.set("auto_lock_ms", "soon").unwrap();
        p.set("text_scale", "9").unwrap();
        p.set("icon_size", "7").unwrap();
        p.set("volume", "-1").unwrap();
        p.set("bold_text", "yes").unwrap();
        p.set("power_profile", "turbo").unwrap();
        let o = Options::load(&p);
        assert_eq!(o.auto_lock_ms, 0);
        assert_eq!(o.text_scale, TEXT_SCALE_MAX);
        assert_eq!(o.icon_size, 0);
        assert_eq!(o.volume, 0.0);
        assert!(!o.bold_text);
        assert_eq!(o.power_profile, Pref::Auto);
    }
}
