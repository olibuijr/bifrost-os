//! Backlight, battery and the halt/reboot requests, through sysfs and systemd.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::OnceLock;

const BACKLIGHT_ROOT: &str = "/sys/class/backlight";
const POWER_SUPPLY_ROOT: &str = "/sys/class/power_supply";
const LEDS_ROOT: &str = "/sys/class/leds";

/// Where the panel and the battery live on this machine, and what halting means. Set once at
/// start-up by `configure`; until then (tests, tools) there is no backlight or battery.
struct Config {
    backlight: Option<PathBuf>,
    /// the keyboard backlight LED, on laptops that have one
    kbd_backlight: Option<PathBuf>,
    battery: Option<PathBuf>,
    mains: Option<PathBuf>,
    emulated: bool,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Picks the sysfs nodes: whatever the machine offers.
/// `emulated` hosts (SPRINGBOARD_VIRTUAL) never halt the machine they run on.
pub fn configure(emulated: bool) {
    let cfg = Config {
        backlight: find_backlight(Path::new(BACKLIGHT_ROOT)),
        kbd_backlight: find_keyboard_backlight(Path::new(LEDS_ROOT)),
        battery: find_battery(Path::new(POWER_SUPPLY_ROOT)),
        mains: find_mains(Path::new(POWER_SUPPLY_ROOT)),
        emulated,
    };
    let _ = CONFIG.set(cfg);
}

fn config() -> &'static Config {
    CONFIG.get_or_init(|| Config { backlight: None, kbd_backlight: None, battery: None, mains: None, emulated: false })
}

fn read_num(path: &Path) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn read_word(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default().trim().to_string()
}

/// The entries of `root` by name, so every pick is deterministic.
fn sorted_entries(root: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(root).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    v.sort();
    v
}

/// The backlight to drive: firmware interfaces first (they follow the platform's own policy),
/// then platform, then raw hardware ones; the name breaks ties.
pub fn find_backlight(root: &Path) -> Option<PathBuf> {
    let rank = |p: &PathBuf| match read_word(&p.join("type")).as_str() {
        "firmware" => 0,
        "platform" => 1,
        _ => 2,
    };
    sorted_entries(root).into_iter().min_by_key(rank)
}

/// The first system battery (BAT0, BAT1, ...), excluding mice and keyboards.
pub fn find_battery(root: &Path) -> Option<PathBuf> {
    sorted_entries(root).into_iter().find(|p| read_word(&p.join("type")) == "Battery" && read_word(&p.join("scope")) != "Device")
}

/// The first mains adapter, for machines whose battery reports no status.
pub fn find_mains(root: &Path) -> Option<PathBuf> {
    sorted_entries(root).into_iter().find(|p| read_word(&p.join("type")) == "Mains")
}

pub fn max_brightness() -> i64 {
    config().backlight.as_ref().and_then(|b| read_num(&b.join("max_brightness"))).unwrap_or(255)
}

pub fn brightness() -> i64 {
    config().backlight.as_ref().and_then(|b| read_num(&b.join("brightness"))).unwrap_or(0)
}

/// Sets the panel brightness. A machine without a backlight (a desktop with external monitors) sends the level to
/// the monitors themselves over DDC/CI (`bifrost_core::ddc_ci`), off the caller's thread.
pub fn set_brightness(v: i64) {
    if let Some(b) = &config().backlight {
        let _ = fs::write(b.join("brightness"), v.clamp(0, max_brightness()).to_string());
    } else {
        set_external_brightness(v);
    }
}

/// The keyboard backlight LED under `root` (`/sys/class/leds`): the first one named like `*kbd_backlight`, which is
/// how the laptop platform drivers call it (`tpacpi::kbd_backlight`, `asus::kbd_backlight`, `apple::kbd_backlight`).
pub fn find_keyboard_backlight(root: &Path) -> Option<PathBuf> {
    sorted_entries(root).into_iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with("kbd_backlight")))
}

/// The highest keyboard backlight level (a few steps on most laptops), or 0 without one.
pub fn keyboard_backlight_max() -> i64 {
    config().kbd_backlight.as_ref().and_then(|b| read_num(&b.join("max_brightness"))).unwrap_or(0)
}

/// The current keyboard backlight level, 0 when off or without one.
pub fn keyboard_backlight() -> i64 {
    config().kbd_backlight.as_ref().and_then(|b| read_num(&b.join("brightness"))).unwrap_or(0)
}

/// Sets the keyboard backlight level (0 to `keyboard_backlight_max()`); nothing without one.
pub fn set_keyboard_backlight(v: i64) {
    if let Some(b) = &config().kbd_backlight {
        let _ = fs::write(b.join("brightness"), v.clamp(0, keyboard_backlight_max()).to_string());
    }
}

static EXTERNAL_TARGET: AtomicI32 = AtomicI32::new(-1);
static EXTERNAL_BUSY: AtomicBool = AtomicBool::new(false);

/// Hands a level (0 to `max_brightness()`) to the external monitors as a percent. A monitor takes about a tenth of
/// a second per change, so one worker thread applies only the newest level while the slider keeps moving.
fn set_external_brightness(v: i64) {
    if config().emulated {
        return;
    }
    let max = max_brightness().max(1);
    EXTERNAL_TARGET.store((v.clamp(0, max) * 100 / max) as i32, Ordering::SeqCst);
    if EXTERNAL_BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut done = -1;
        loop {
            let want = EXTERNAL_TARGET.load(Ordering::SeqCst);
            if want != done {
                bifrost_core::ddc_ci::set_external_brightness(want as u8);
                done = want;
                continue;
            }
            EXTERNAL_BUSY.store(false, Ordering::SeqCst);
            if EXTERNAL_TARGET.load(Ordering::SeqCst) == done || EXTERNAL_BUSY.swap(true, Ordering::SeqCst) {
                break;
            }
        }
    });
}

/// Makes the power key (gpio-keys) a wakeup source, so pressing it resumes a suspended device.
/// The flag lives on the input device and on its parent platform device depending on the kernel.
pub fn enable_power_key_wake() {
    let Ok(dir) = fs::read_dir("/sys/class/input") else { return };
    for e in dir.flatten() {
        let p = e.path();
        if !fs::read_to_string(p.join("name")).is_ok_and(|n| n.trim() == "gpio-keys") {
            continue;
        }
        for f in [p.join("device/power/wakeup"), p.join("power/wakeup")] {
            let _ = fs::write(f, "enabled");
        }
    }
}

/// (percent, charging) from the battery directory `bat`; None when the capacity cannot be
/// read. Without a status file the mains adapter `mains` says whether the charger is in.
pub fn read_battery(bat: &Path, mains: Option<&Path>) -> Option<(u8, bool)> {
    let pct = read_num(&bat.join("capacity"))?.clamp(0, 100) as u8;
    let charging = match fs::read_to_string(bat.join("status")) {
        Ok(status) => matches!(status.trim(), "Charging" | "Full"),
        Err(_) => mains.and_then(|m| read_num(&m.join("online"))) == Some(1),
    };
    Some((pct, charging))
}

/// (percent, charging) from sysfs; None when the battery cannot be read.
pub fn battery_reading() -> Option<(u8, bool)> {
    let c = config();
    read_battery(c.battery.as_ref()?, c.mains.as_deref())
}

/// For power policy only (profiles, sleep): an unreadable battery never triggers low-battery
/// handling, and a machine with no system battery counts as mains power. Do not show or log this
/// value as a battery reading; use `battery_reading` for that.
pub fn battery() -> (u8, bool) {
    let c = config();
    battery_reading().unwrap_or((100, c.battery.is_none()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halt {
    Shutdown,
    Reboot,
}

/// What `halt` does on this machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HaltPlan {
    /// `systemctl <verb>`
    Systemctl(&'static str),
    /// reboot(2) straight away: no init to ask
    Kernel,
    /// an emulated host: nothing may happen
    Nothing,
}

/// The halt mechanism for a machine: pure, so it is tested without halting anything.
pub fn halt_plan(emulated: bool, systemctl: bool, how: Halt) -> HaltPlan {
    match (emulated, systemctl) {
        (true, _) => HaltPlan::Nothing,
        (false, true) => HaltPlan::Systemctl(if how == Halt::Reboot { "reboot" } else { "poweroff" }),
        (false, false) => HaltPlan::Kernel,
    }
}

/// True when `name` is an executable file in one of the directories of `path` (PATH syntax).
fn on_path(name: &str, path: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.split(':').filter(|d| !d.is_empty()).any(|d| fs::metadata(Path::new(d).join(name)).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

#[cfg(target_os = "linux")]
extern "C" {
    fn sync();
    fn reboot(cmd: std::os::raw::c_int) -> std::os::raw::c_int;
}

/// Syncs, then asks systemd to shut down or reboot (the kernel when there is no systemctl);
/// an emulated host does nothing at all. The caller flushes its own app data first.
pub fn halt(how: Halt) {
    let c = config();
    let systemctl = on_path("systemctl", &std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()));
    match halt_plan(c.emulated, systemctl, how) {
        HaltPlan::Nothing => eprintln!("springboard: halt {how:?} ignored (emulated host)"),
        HaltPlan::Systemctl(verb) => {
            desktop_sync();
            if let Err(e) = crate::privd_client::connect().power(verb) {
                eprintln!("springboard: systemctl {verb}: {e}");
            }
        }
        HaltPlan::Kernel => {
            desktop_sync();
            kernel_reboot(how);
        }
    }
}

#[cfg(target_os = "linux")]
fn desktop_sync() {
    // SAFETY: sync takes no arguments and has no preconditions.
    unsafe { sync() };
}

#[cfg(not(target_os = "linux"))]
fn desktop_sync() {}

#[cfg(target_os = "linux")]
fn kernel_reboot(how: Halt) {
    const RB_AUTOBOOT: i32 = 0x0123_4567;
    const RB_POWER_OFF: i32 = 0x4321_fedc_u32 as i32;
    // SAFETY: reboot(2) with a valid command; it only returns on failure.
    unsafe { reboot(if how == Halt::Reboot { RB_AUTOBOOT } else { RB_POWER_OFF }) };
}

#[cfg(not(target_os = "linux"))]
fn kernel_reboot(_how: Halt) {}

/// Keeps the Wi-Fi chip in its power-save mode (best effort, nl80211).
pub fn wifi_power_save(on: bool) {
    let _ = super::nl80211::set_power_save(crate::net::wifi::IFACE, on);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake sysfs tree under the scratch dir; `files` are (relative path, content).
    fn fake_tree(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = PathBuf::from(std::env::var("HOME").unwrap()).join(".local/share/springboard-emu/test-scratch").join(format!("power-{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for (rel, content) in files {
            let p = root.join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
        root
    }

    #[test]
    fn review_peripheral_battery_is_not_system_power() {
        let r=fake_tree("device-battery",&[("00-mouse/type","Battery"),("00-mouse/scope","Device"),("BAT0/type","Battery"),("BAT0/scope","System")]);
        assert_eq!(find_battery(&r),Some(r.join("BAT0")));
        fs::remove_dir_all(r.join("BAT0")).unwrap();assert_eq!(find_battery(&r),None);
    }

    #[test]
    fn the_keyboard_backlight_is_the_led_named_for_it() {
        let r = fake_tree("kbd", &[("input3::capslock/brightness", "0\n"), ("tpacpi::kbd_backlight/max_brightness", "2\n")]);
        assert_eq!(find_keyboard_backlight(&r), Some(r.join("tpacpi::kbd_backlight")));
        assert_eq!(find_keyboard_backlight(&fake_tree("kbd2", &[("input3::capslock/brightness", "0\n")])), None);
        assert_eq!(find_keyboard_backlight(Path::new("/nonexistent-springboard-test")), None);
        assert_eq!(keyboard_backlight_max(), 0, "no configured LED in tests");
    }

    #[test]
    fn backlight_prefers_firmware_then_platform_then_name() {
        let r = fake_tree("bl", &[("intel_backlight/type", "raw\n"), ("acpi_video0/type", "firmware\n"), ("amdgpu_bl0/type", "platform\n")]);
        assert_eq!(find_backlight(&r), Some(r.join("acpi_video0")));
        let r = fake_tree("bl2", &[("b/type", "raw"), ("a/type", "raw"), ("c/type", "platform")]);
        assert_eq!(find_backlight(&r), Some(r.join("c")));
        let r = fake_tree("bl3", &[("b/type", "raw"), ("a/type", "raw")]);
        assert_eq!(find_backlight(&r), Some(r.join("a")), "ties go by name");
        assert_eq!(find_backlight(&fake_tree("bl4", &[])), None);
        assert_eq!(find_backlight(Path::new("/nonexistent-springboard-test")), None);
    }

    #[test]
    fn battery_is_the_first_battery_type_supply() {
        let r = fake_tree("bat", &[("AC/type", "Mains"), ("AC/online", "1"), ("BAT1/type", "Battery"), ("BAT0/type", "Battery"), ("hidpp/type", "Battery")]);
        assert_eq!(find_battery(&r), Some(r.join("BAT0")));
        assert_eq!(find_mains(&r), Some(r.join("AC")));
        assert_eq!(find_battery(&fake_tree("bat2", &[("AC/type", "Mains")])), None);
    }

    #[test]
    fn battery_reads_status_else_the_adapter() {
        let r = fake_tree("rd", &[("BAT0/capacity", "42\n"), ("BAT0/status", "Charging\n"), ("AC/online", "0\n")]);
        assert_eq!(read_battery(&r.join("BAT0"), Some(&r.join("AC"))), Some((42, true)));
        let r = fake_tree("rd2", &[("BAT0/capacity", "150"), ("AC/online", "1")]);
        assert_eq!(read_battery(&r.join("BAT0"), Some(&r.join("AC"))), Some((100, true)), "no status: the adapter decides");
        assert_eq!(read_battery(&r.join("BAT0"), None), Some((100, false)));
        let r = fake_tree("rd3", &[("BAT0/capacity", "7"), ("BAT0/status", "Discharging"), ("AC/online", "1")]);
        assert_eq!(read_battery(&r.join("BAT0"), Some(&r.join("AC"))), Some((7, false)), "a status file wins");
        assert_eq!(read_battery(&r.join("missing"), None), None);
    }

    #[test]
    fn halt_plan_picks_the_mechanism() {
        use HaltPlan::*;
        assert_eq!(halt_plan(false, true, Halt::Shutdown), Systemctl("poweroff"));
        assert_eq!(halt_plan(false, true, Halt::Reboot), Systemctl("reboot"));
        assert_eq!(halt_plan(false, false, Halt::Reboot), Kernel);
        assert_eq!(halt_plan(true, true, Halt::Reboot), Nothing, "an emulated host never halts the machine");
    }

    #[test]
    fn on_path_finds_executables_only() {
        use std::os::unix::fs::PermissionsExt;
        let r = fake_tree("path", &[("bin/systemctl", ""), ("bin2/data", "")]);
        fs::set_permissions(r.join("bin/systemctl"), fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("/nonexistent:{}:{}", r.join("bin2").display(), r.join("bin").display());
        assert!(on_path("systemctl", &path));
        assert!(!on_path("data", &path), "not executable");
        assert!(!on_path("systemctl", ""));
    }
}
