//! The form factor the system runs on: a desktop or laptop with pointer and hardware keyboard, or a touch tablet.
//! One shell and one SDK serve both; this model is the single switch. It is read once from `BIFROST_FORM` or the file
//! `/etc/bifrost/form` (one word, `desktop` or `tablet`). When neither is set, the machine is inspected once (SMBIOS
//! chassis type, device-tree board, touchscreen input devices) and the form is detected from those facts; a machine
//! that matches none of them is a desktop.

use std::sync::OnceLock;

/// Environment variable that names the form factor. It wins over the file.
pub const ENV: &str = "BIFROST_FORM";
/// System file holding the form factor as a plain word.
pub const FILE: &str = "/etc/bifrost/form";
/// SMBIOS chassis type of the running machine, as a decimal number. Absent on device-tree systems.
pub const SMBIOS_CHASSIS: &str = "/sys/class/dmi/id/chassis_type";
/// Device-tree `compatible` list of the board: NUL-separated strings, most specific first.
pub const DEVICE_TREE_COMPATIBLE: &str = "/proc/device-tree/compatible";
/// The kernel's input device table; each device is a block of `B:` and `H:` lines.
pub const INPUT_DEVICES: &str = "/proc/bus/input/devices";
/// Device-tree boards known to be tablets, matched against the `compatible` list.
pub const TABLET_BOARDS: &[&str] = &["samsung,gta8wifi"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FormFactor {
    /// Pointer and hardware keyboard: no on-screen keyboard unless the user opens it.
    #[default]
    Desktop,
    /// Touch first: text fields raise the on-screen keyboard unless a hardware keyboard is attached.
    Tablet,
}

/// What the machine reports about itself, gathered by `current` and passed to the pure `resolve`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts<'a> {
    /// The SMBIOS chassis type code, when the firmware reports one.
    pub smbios_chassis: Option<u32>,
    /// The device-tree `compatible` list, NUL-separated; empty when there is no device tree.
    pub device_tree_compatible: &'a str,
    /// Whether a direct-touch (touchscreen) input device is present.
    pub touchscreen: bool,
}

impl FormFactor {
    /// Parses the plain word, ignoring case and surrounding whitespace. Unknown words give `None`.
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "desktop" => Some(Self::Desktop),
            "tablet" => Some(Self::Tablet),
            _ => None,
        }
    }

    /// The word `parse` reads back; also what the shell passes to apps in `BIFROST_FORM`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Tablet => "tablet",
        }
    }

    /// Resolves the form from the environment value, the file contents and the machine facts. A value that does not
    /// parse is skipped, so a typo in the variable falls through to the file, and then to detection.
    pub fn resolve(env: Option<&str>, file: Option<&str>, facts: &Facts) -> Self {
        env.and_then(Self::parse).or_else(|| file.and_then(Self::parse)).unwrap_or_else(|| Self::detect(facts))
    }

    /// The fallback when neither the variable nor the file names a form. With SMBIOS, the chassis type decides alone:
    /// a tablet or convertible chassis is a tablet and everything else (laptops included) is a desktop. Without SMBIOS,
    /// a known tablet board or a touchscreen makes a tablet.
    pub fn detect(facts: &Facts) -> Self {
        match facts.smbios_chassis {
            Some(code) if sysinfo_chassis_is_tablet(code) => Self::Tablet,
            Some(_) => Self::Desktop,
            None if device_tree_is_tablet(facts.device_tree_compatible) || facts.touchscreen => Self::Tablet,
            None => Self::Desktop,
        }
    }

    /// The form of this process, read once. Shell and apps call this; tests build a value and pass it instead.
    pub fn current() -> Self {
        static CURRENT: OnceLock<FormFactor> = OnceLock::new();
        *CURRENT.get_or_init(|| {
            let env = std::env::var(ENV).ok();
            let file = std::fs::read_to_string(FILE).ok();
            let smbios_chassis = std::fs::read_to_string(SMBIOS_CHASSIS).ok().and_then(|s| s.trim().parse::<u32>().ok());
            let compatible = std::fs::read(DEVICE_TREE_COMPATIBLE).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
            let input = std::fs::read_to_string(INPUT_DEVICES).unwrap_or_default();
            let facts = Facts { smbios_chassis, device_tree_compatible: &compatible, touchscreen: touchscreen_in_input_devices(&input) };
            Self::resolve(env.as_deref(), file.as_deref(), &facts)
        })
    }

    pub fn is_tablet(self) -> bool {
        self == Self::Tablet
    }

    pub fn is_desktop(self) -> bool {
        self == Self::Desktop
    }

    /// Whether the on-screen keyboard may be raised *by the system* (a focused text field, Terminal opening, the lock
    /// screen, a sheet, Search). A desktop never does, whatever UI is up: it appears there only when the user opens it
    /// on purpose (the Input menu's Keyboard, `Shell::keyboard_open_explicit`). A tablet raises it unless a hardware
    /// keyboard is attached.
    pub fn allows_onscreen_keyboard(self, hardware_keyboard: bool) -> bool {
        match self {
            Self::Desktop => false,
            Self::Tablet => !hardware_keyboard,
        }
    }

    /// Whether the session has no on-screen keyboard at all, so views and the keyboard process stay off.
    pub fn keyboard_disabled_globally(self) -> bool {
        self.is_desktop()
    }
}

/// Whether an SMBIOS chassis type code names a tablet or convertible. Uses the chassis naming of `bifrost_core`, so the
/// codes 30 to 32 are defined in one place.
fn sysinfo_chassis_is_tablet(code: u32) -> bool {
    bifrost_core::sysinfo::chassis_name(code) == "Tablet"
}

/// Whether a device-tree `compatible` list (NUL-separated) names a known tablet board.
pub fn device_tree_is_tablet(compatible: &str) -> bool {
    compatible.split('\0').any(|name| TABLET_BOARDS.contains(&name))
}

/// Whether the text of `/proc/bus/input/devices` lists a direct-touch device. The kernel sets the `INPUT_PROP_DIRECT`
/// property (bit 1 of the `B: PROP=` mask) on touchscreens and not on touchpads, which carry the pointer and buttonpad
/// bits instead, so a touchpad does not count.
pub fn touchscreen_in_input_devices(text: &str) -> bool {
    const PROP_DIRECT: u64 = 1 << 1;
    text.split("\n\n").any(|block| {
        block.lines().any(|line| {
            line.strip_prefix("B: PROP=")
                .and_then(|mask| u64::from_str_radix(mask.trim(), 16).ok())
                .is_some_and(|mask| mask & PROP_DIRECT != 0)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO_FACTS: Facts = Facts { smbios_chassis: None, device_tree_compatible: "", touchscreen: false };

    fn smbios(code: u32) -> Facts<'static> {
        Facts { smbios_chassis: Some(code), ..NO_FACTS }
    }

    #[test]
    fn words_parse_ignoring_case_and_whitespace() {
        assert_eq!(FormFactor::parse("tablet\n"), Some(FormFactor::Tablet));
        assert_eq!(FormFactor::parse("  Desktop "), Some(FormFactor::Desktop));
        assert_eq!(FormFactor::parse("phone"), None);
        assert_eq!(FormFactor::parse(""), None);
    }

    #[test]
    fn as_str_round_trips() {
        for f in [FormFactor::Desktop, FormFactor::Tablet] {
            assert_eq!(FormFactor::parse(f.as_str()), Some(f));
        }
    }

    #[test]
    fn the_variable_wins_over_the_file_and_the_default_is_desktop() {
        assert_eq!(FormFactor::resolve(Some("tablet"), Some("desktop"), &NO_FACTS), FormFactor::Tablet);
        assert_eq!(FormFactor::resolve(None, Some("tablet\n"), &NO_FACTS), FormFactor::Tablet);
        assert_eq!(FormFactor::resolve(None, None, &NO_FACTS), FormFactor::Desktop);
        assert_eq!(FormFactor::resolve(Some("garbage"), Some("tablet"), &NO_FACTS), FormFactor::Tablet);
        assert_eq!(FormFactor::resolve(Some("garbage"), Some("also garbage"), &NO_FACTS), FormFactor::Desktop);
    }

    #[test]
    fn an_explicit_setting_beats_the_detected_form() {
        let tablet_machine = Facts { smbios_chassis: Some(31), ..NO_FACTS };
        assert_eq!(FormFactor::resolve(Some("desktop"), None, &tablet_machine), FormFactor::Desktop);
        assert_eq!(FormFactor::resolve(None, Some("desktop"), &tablet_machine), FormFactor::Desktop);
        let desktop_machine = Facts { touchscreen: true, ..smbios(3) };
        assert_eq!(FormFactor::resolve(Some("tablet"), None, &desktop_machine), FormFactor::Tablet);
    }

    #[test]
    fn smbios_tablet_and_convertible_chassis_are_tablets() {
        for code in [30, 31, 32] {
            assert_eq!(FormFactor::detect(&smbios(code)), FormFactor::Tablet, "chassis {code}");
        }
    }

    #[test]
    fn smbios_desktop_and_laptop_chassis_stay_desktop_even_with_a_touchscreen() {
        for code in [3, 4, 6, 7, 8, 9, 10, 11, 13, 14, 15, 16, 17, 99] {
            let facts = Facts { smbios_chassis: Some(code), device_tree_compatible: "samsung,gta8wifi", touchscreen: true };
            assert_eq!(FormFactor::detect(&facts), FormFactor::Desktop, "chassis {code}");
        }
    }

    #[test]
    fn a_known_tablet_board_is_a_tablet_without_smbios() {
        let board = Facts { device_tree_compatible: "samsung,gta8wifi\0samsung,exynos7870\0", ..NO_FACTS };
        assert_eq!(FormFactor::detect(&board), FormFactor::Tablet);
        let bare = Facts { device_tree_compatible: "samsung,gta8wifi", ..NO_FACTS };
        assert_eq!(FormFactor::detect(&bare), FormFactor::Tablet);
    }

    #[test]
    fn an_unknown_board_is_not_a_tablet_by_name() {
        let other = Facts { device_tree_compatible: "raspberrypi,4-model-b\0brcm,bcm2711\0", ..NO_FACTS };
        assert_eq!(FormFactor::detect(&other), FormFactor::Desktop);
        assert!(!device_tree_is_tablet(""));
        assert!(!device_tree_is_tablet("samsung,gta8wifi-rev2"));
    }

    #[test]
    fn a_touchscreen_is_a_tablet_without_smbios() {
        let touch = Facts { touchscreen: true, ..NO_FACTS };
        assert_eq!(FormFactor::detect(&touch), FormFactor::Tablet);
    }

    #[test]
    fn no_facts_at_all_is_a_desktop() {
        assert_eq!(FormFactor::detect(&NO_FACTS), FormFactor::Desktop);
        assert_eq!(FormFactor::detect(&Facts::default()), FormFactor::Desktop);
    }

    #[test]
    fn a_touchpad_alone_is_not_a_touchscreen() {
        let proc = "I: Bus=0019 Vendor=0000 Product=0001\nN: Name=\"Power Button\"\nH: Handlers=kbd event0\nB: PROP=0\n\n\
                    I: Bus=0011 Vendor=0002 Product=0007\nN: Name=\"SynPS/2 Synaptics TouchPad\"\nH: Handlers=mouse0 event4\nB: PROP=5\n";
        assert!(!touchscreen_in_input_devices(proc));
    }

    #[test]
    fn a_direct_touch_device_is_a_touchscreen_in_any_block() {
        let proc = "N: Name=\"AT Keyboard\"\nB: PROP=0\n\n\
                    N: Name=\"Goodix Capacitive TouchScreen\"\nH: Handlers=event7\nB: PROP=2\n";
        assert!(touchscreen_in_input_devices(proc));
        assert!(touchscreen_in_input_devices("B: PROP=3\n"));
        assert!(!touchscreen_in_input_devices(""));
        assert!(!touchscreen_in_input_devices("N: Name=\"x\"\nB: PROP=zz\n"));
    }

    #[test]
    fn a_desktop_never_raises_the_keyboard_by_itself() {
        for hw in [false, true] {
            assert!(!FormFactor::Desktop.allows_onscreen_keyboard(hw));
        }
    }

    #[test]
    fn a_tablet_raises_it_without_a_hardware_keyboard() {
        assert!(FormFactor::Tablet.allows_onscreen_keyboard(false));
    }

    #[test]
    fn a_hardware_keyboard_on_a_tablet_hides_it() {
        assert!(!FormFactor::Tablet.allows_onscreen_keyboard(true));
    }

    #[test]
    fn only_a_desktop_turns_the_keyboard_off_for_the_session() {
        assert!(FormFactor::Desktop.keyboard_disabled_globally());
        assert!(!FormFactor::Tablet.keyboard_disabled_globally());
    }
}
