//! USB accessory approval: the decision behind the prompt that asks the user to approve a new USB device before it may
//! control the machine (`HIDRMUI` parity; the reference prompt is the Accessory approval sheet).
//!
//! Pure: no udev, no sysfs writes, no window. The caller reads the kernel's `authorized` attribute of the device
//! (`/sys/bus/usb/devices/<port>/authorized`, parsed with `parse_authorized`) and the device key from udev
//! (`device_key`), asks `Approvals::decide`, shows the prompt on `Decision::Prompt`, records the answer with
//! `Approvals::answer` and writes the result back to the kernel attribute with `sysfs_value`.
//!
//! Rules, in the order they apply:
//! - A device the user denied always stays blocked.
//! - A device the user allowed (always, or once for this session) is allowed.
//! - A device the kernel already authorized (built-in hubs and interfaces) is allowed without a prompt.
//! - Anything else asks the user.
//!
//! The approvals persist as text lines `"<key> allow"` and `"<key> deny"`. "Allow once" lives only in memory and is
//! dropped by `forget_session`.

use std::collections::{BTreeMap, BTreeSet};

/// What the approval step does with a newly seen device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    /// Ask the user, then call `Approvals::answer`.
    Prompt,
}

/// The user's answer in the prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    AllowAlways,
    AllowOnce,
    DenyAlways,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Approvals {
    /// Stored decisions keyed by device key: `true` allowed, `false` denied.
    stored: BTreeMap<String, bool>,
    /// Devices allowed for this session only.
    once: BTreeSet<String>,
}

/// The device key used for approvals: `vvvv:pppp:serial`, lower-case hex ids, `-` when the device has no serial.
/// `None` when the ids are not four hex digits, or the serial would break the text format (whitespace).
pub fn device_key(vendor: &str, product: &str, serial: &str) -> Option<String> {
    let id = |s: &str| -> Option<String> {
        let s = s.trim().to_ascii_lowercase();
        (s.len() == 4 && s.bytes().all(|b| b.is_ascii_hexdigit())).then_some(s)
    };
    let serial = serial.trim();
    if serial.chars().any(char::is_whitespace) {
        return None;
    }
    let serial = if serial.is_empty() { "-" } else { serial };
    Some(format!("{}:{}:{}", id(vendor)?, id(product)?, serial))
}

/// The kernel's `authorized` attribute: `1` allowed, `0` blocked. Anything else is `None`.
pub fn parse_authorized(text: &str) -> Option<bool> {
    match text.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// The text to write to the kernel's `authorized` attribute for a decision. `Prompt` writes nothing.
pub fn sysfs_value(decision: Decision) -> Option<&'static str> {
    match decision {
        Decision::Allow => Some("1"),
        Decision::Deny => Some("0"),
        Decision::Prompt => None,
    }
}

/// The kernel attribute to write for a device port: `/sys/bus/usb/devices/<port>/authorized`. A port is a bus
/// (`usb1`) or a bus-port path (`1-2`, `1-2.4`); anything else, including a path separator or `..`, is `None`, so a
/// caller can never be walked out of the sysfs tree.
pub fn authorized_path(port: &str) -> Option<String> {
    let valid = if let Some(bus) = port.strip_prefix("usb") {
        !bus.is_empty() && bus.bytes().all(|b| b.is_ascii_digit())
    } else {
        let (bus, ports) = port.split_once('-')?;
        !bus.is_empty()
            && bus.bytes().all(|b| b.is_ascii_digit())
            && !ports.is_empty()
            && ports.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    valid.then(|| format!("/sys/bus/usb/devices/{port}/authorized"))
}

impl Approvals {
    pub fn new() -> Self {
        Self::default()
    }

    /// The decision for a device, given the kernel's current `authorized` state.
    pub fn decide(&self, key: &str, kernel_authorized: bool) -> Decision {
        match self.stored.get(key) {
            Some(false) => Decision::Deny,
            Some(true) => Decision::Allow,
            None if self.once.contains(key) => Decision::Allow,
            None if kernel_authorized => Decision::Allow,
            None => Decision::Prompt,
        }
    }

    /// Record the user's answer for a device.
    pub fn answer(&mut self, key: &str, answer: Answer) {
        match answer {
            Answer::AllowAlways => {
                self.once.remove(key);
                self.stored.insert(key.to_string(), true);
            }
            Answer::AllowOnce => {
                self.once.insert(key.to_string());
            }
            Answer::DenyAlways => {
                self.once.remove(key);
                self.stored.insert(key.to_string(), false);
            }
        }
    }

    /// Drop the session-only allowances (the user logged out or the machine rebooted).
    pub fn forget_session(&mut self) {
        self.once.clear();
    }

    /// The stored decision for a device, `None` when the user has not answered.
    pub fn stored(&self, key: &str) -> Option<bool> {
        self.stored.get(key).copied()
    }

    /// The persisted form: one `"<key> allow"` or `"<key> deny"` line per device, sorted by key.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (key, allowed) in &self.stored {
            out.push_str(key);
            out.push_str(if *allowed { " allow\n" } else { " deny\n" });
        }
        out
    }

    /// Read the persisted form. Blank lines, `#` comments and malformed lines are skipped.
    pub fn from_text(text: &str) -> Self {
        let mut approvals = Self::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, word)) = line.rsplit_once(' ') else { continue };
            let allowed = match word {
                "allow" => true,
                "deny" => false,
                _ => continue,
            };
            if !key.is_empty() && !key.contains(char::is_whitespace) {
                approvals.stored.insert(key.to_string(), allowed);
            }
        }
        approvals
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_key_normalises_ids_and_marks_missing_serial() {
        assert_eq!(device_key("046D", " C52B ", "ABC123"), Some("046d:c52b:ABC123".to_string()));
        assert_eq!(device_key("046d", "c52b", "  "), Some("046d:c52b:-".to_string()));
        assert_eq!(device_key("46d", "c52b", "x"), None);
        assert_eq!(device_key("046d", "zzzz", "x"), None);
        assert_eq!(device_key("046d", "c52b", "two words"), None);
    }

    #[test]
    fn sysfs_attribute_round_trips_to_the_decision() {
        assert_eq!(parse_authorized("1\n"), Some(true));
        assert_eq!(parse_authorized("0"), Some(false));
        assert_eq!(parse_authorized(""), None);
        assert_eq!(sysfs_value(Decision::Allow), Some("1"));
        assert_eq!(sysfs_value(Decision::Deny), Some("0"));
        assert_eq!(sysfs_value(Decision::Prompt), None);
    }

    #[test]
    fn the_authorized_attribute_path_accepts_only_bus_and_port_names() {
        assert_eq!(authorized_path("1-2").as_deref(), Some("/sys/bus/usb/devices/1-2/authorized"));
        assert_eq!(authorized_path("1-2.4.1").as_deref(), Some("/sys/bus/usb/devices/1-2.4.1/authorized"));
        assert_eq!(authorized_path("usb1").as_deref(), Some("/sys/bus/usb/devices/usb1/authorized"));
        assert_eq!(authorized_path("../../etc"), None);
        assert_eq!(authorized_path("1-2/../../x"), None);
        assert_eq!(authorized_path("1-2:1.0"), None, "interfaces are not written here");
        assert_eq!(authorized_path("1-"), None);
        assert_eq!(authorized_path("usb"), None);
        assert_eq!(authorized_path(""), None);
    }

    #[test]
    fn a_new_device_asks_unless_the_kernel_already_allowed_it() {
        let a = Approvals::new();
        assert_eq!(a.decide("046d:c52b:-", false), Decision::Prompt);
        assert_eq!(a.decide("046d:c52b:-", true), Decision::Allow);
    }

    #[test]
    fn allow_always_and_deny_always_persist_and_win_over_the_kernel() {
        let mut a = Approvals::new();
        a.answer("046d:c52b:-", Answer::AllowAlways);
        a.answer("1234:0001:S1", Answer::DenyAlways);
        assert_eq!(a.decide("046d:c52b:-", false), Decision::Allow);
        assert_eq!(a.decide("1234:0001:S1", true), Decision::Deny);
        assert_eq!(a.stored("1234:0001:S1"), Some(false));
    }

    #[test]
    fn allow_once_lasts_until_the_session_ends() {
        let mut a = Approvals::new();
        a.answer("aaaa:bbbb:-", Answer::AllowOnce);
        assert_eq!(a.decide("aaaa:bbbb:-", false), Decision::Allow);
        assert_eq!(a.stored("aaaa:bbbb:-"), None);
        assert_eq!(a.to_text(), "");
        a.forget_session();
        assert_eq!(a.decide("aaaa:bbbb:-", false), Decision::Prompt);
    }

    #[test]
    fn a_later_always_answer_replaces_a_session_allowance() {
        let mut a = Approvals::new();
        a.answer("aaaa:bbbb:-", Answer::AllowOnce);
        a.answer("aaaa:bbbb:-", Answer::DenyAlways);
        a.forget_session();
        assert_eq!(a.decide("aaaa:bbbb:-", false), Decision::Deny);
    }

    #[test]
    fn the_text_form_round_trips_and_skips_bad_lines() {
        let mut a = Approvals::new();
        a.answer("046d:c52b:-", Answer::AllowAlways);
        a.answer("1234:0001:S1", Answer::DenyAlways);
        let text = a.to_text();
        assert_eq!(text, "046d:c52b:- allow\n1234:0001:S1 deny\n");
        assert_eq!(Approvals::from_text(&text), a);
        let messy = "# comment\n\nnoword\n046d:c52b:- maybe\n046d:c52b:- allow\n";
        let read = Approvals::from_text(messy);
        assert_eq!(read.stored("046d:c52b:-"), Some(true));
        assert_eq!(read.to_text(), "046d:c52b:- allow\n");
    }
}
