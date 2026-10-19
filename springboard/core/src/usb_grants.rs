//! Per-app consent for raw USB and raw HID devices.
//!
//! The system approval in `usb_approval` decides whether a new device may be used at all (the kernel's `authorized`
//! attribute). This module is the second gate: whether one *app* may talk to one *device*. Nothing is open by
//! default. An app that declares the `usb` or `hid` manifest permission may only ask; the person then picks the device
//! on a sheet, and only that device is granted, only to that app.
//!
//! Pure: no sysfs, no sheet, no I/O. The shell builds [`Candidate`]s from the bus, asks [`candidates_for`] which of
//! them an app's request may offer, shows the sheet, and records the answer with [`Grants::grant`] (Allow Once or
//! Always Allow) or not at all (Don't Allow). [`Grants::to_text`] and [`Grants::from_text`] persist the Always grants.
//!
//! Rules:
//! - A grant belongs to one app and one device key. The key is `vvvv:pppp:serial` for USB and `hid:vvvv:pppp:uniq` for
//!   HID, so two identical devices without serial numbers share a grant, and a device with a serial keeps it across
//!   ports.
//! - Allow Once ends when the app quits ([`Grants::end_session`]) or the device is unplugged
//!   ([`Grants::device_gone`]); Always Allow survives both, and is removed by [`Grants::revoke`].
//! - Devices the system already drives (audio, HID, storage, video, hubs, smart cards, wireless controllers) are
//!   never offered for raw access through the USB path: [`protected_class`]. Their own APIs have their own consent.
//! - An app holds at most [`MAX_GRANTS_PER_APP`] grants and may ask about at most [`MAX_CANDIDATES`] devices at once.

use std::collections::BTreeSet;

/// The most grants one app may hold (Once and Always together).
pub const MAX_GRANTS_PER_APP: usize = 32;
/// The most devices one request offers on the sheet queue.
pub const MAX_CANDIDATES: usize = 4;
/// The longest key or app id the store accepts; longer ones are refused, never cut.
pub const MAX_KEY_LEN: usize = 128;

/// How long a grant lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Until the app quits or the device is unplugged.
    Once,
    /// Until the person revokes it.
    Always,
}

/// Whether a USB interface class is one the system drives itself, so a raw claim by an app is refused:
/// audio (0x01), HID (0x03), mass storage (0x08), hub (0x09), smart card (0x0B), video (0x0E), audio/video (0x10) and
/// wireless controller (0xE0). The HID class has its own consent path (the `hid` permission).
pub fn protected_class(class: u8) -> bool {
    matches!(class, 0x01 | 0x03 | 0x08 | 0x09 | 0x0b | 0x0e | 0x10 | 0xe0)
}

/// The device key of a USB device. A serial with whitespace has it replaced by `_` (the persisted form is
/// whitespace-separated) and is cut at 64 characters; no serial gives `-`.
pub fn usb_key(vendor_id: u16, product_id: u16, serial: &str) -> String {
    format!("{vendor_id:04x}:{product_id:04x}:{}", clean_serial(serial))
}

/// The device key of a raw HID device (`hid:vvvv:pppp:uniq`, `uniq` being the HID unique string, often empty).
pub fn hid_key(vendor_id: u16, product_id: u16, unique: &str) -> String {
    format!("hid:{vendor_id:04x}:{product_id:04x}:{}", clean_serial(unique))
}

fn clean_serial(s: &str) -> String {
    let s: String = s.trim().chars().map(|c| if c.is_whitespace() || c.is_control() { '_' } else { c }).take(64).collect();
    if s.is_empty() {
        "-".to_string()
    } else {
        s
    }
}

/// What the shell knows about one connected device, enough to filter and to name it on the sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub key: String,
    /// The product string; empty when the device has none.
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    /// `bDeviceClass` (0 for a device that declares its class per interface).
    pub device_class: u8,
    /// The class of every interface the device carries. Empty when unknown.
    pub interface_classes: Vec<u8>,
    /// A raw HID device (a hidraw node) rather than a USB device. The caller leaves out the ones `hidraw::protected`
    /// refuses, so every HID candidate is eligible.
    pub hid: bool,
    /// The `(usage page, usage)` of the top-level collections of a HID device; empty for USB.
    pub usages: Vec<(u16, u16)>,
}

impl Candidate {
    /// Whether the device may be offered for raw USB access: at least one interface the system does not drive. A
    /// device whose interfaces are unknown follows its device class.
    pub fn eligible(&self) -> bool {
        if self.hid {
            true
        } else if self.interface_classes.is_empty() {
            !protected_class(self.device_class)
        } else {
            self.interface_classes.iter().any(|c| !protected_class(*c))
        }
    }

    /// The device's name on the sheet: the product string, or its ids.
    pub fn label(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            format!("USB device {:04x}:{:04x}", self.vendor_id, self.product_id)
        } else {
            name.to_string()
        }
    }
}

/// One entry of the filter list an app passes to `request`. Every field that is set must match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    /// Matches the device class or the class of any interface.
    pub class: Option<u8>,
    /// HID only: matches a top-level collection's usage page.
    pub usage_page: Option<u16>,
    /// HID only: with `usage_page`, matches a top-level collection's usage.
    pub usage: Option<u16>,
}

impl Filter {
    pub fn matches(&self, c: &Candidate) -> bool {
        self.vendor_id.is_none_or(|v| v == c.vendor_id)
            && self.product_id.is_none_or(|p| p == c.product_id)
            && self.class.is_none_or(|k| c.device_class == k || c.interface_classes.contains(&k))
            && (self.usage_page.is_none() && self.usage.is_none()
                || c.usages.iter().any(|&(page, usage)| self.usage_page.is_none_or(|p| p == page) && self.usage.is_none_or(|u| u == usage)))
    }
}

/// The devices a request may offer: connected, eligible, matching at least one filter (all of them when `filters` is
/// empty), not granted to the app yet, and at most [`MAX_CANDIDATES`], in the order given. `already` says whether the
/// app holds a grant for a key.
pub fn candidates_for(filters: &[Filter], connected: &[Candidate], already: &dyn Fn(&str) -> bool) -> Vec<Candidate> {
    connected
        .iter()
        .filter(|c| c.eligible())
        .filter(|c| filters.is_empty() || filters.iter().any(|f| f.matches(c)))
        .filter(|c| !already(&c.key))
        .take(MAX_CANDIDATES)
        .cloned()
        .collect()
}

/// The title of the sheet. The device is named in the text under it ([`sheet_detail`]).
pub fn sheet_title(app: &str) -> String {
    format!("\u{201c}{app}\u{201d} would like to use a device connected to this computer.")
}

/// The text under the title: the device by name, what the grant allows and where to change it.
pub fn sheet_detail(device_label: &str) -> String {
    format!(
        "\u{201c}{device_label}\u{201d}. Allowing lets the app talk to this device directly, only this one, while it is \
         open. Always Allow remembers the choice. You can change this at any time in Settings > Privacy & Security > App Permissions."
    )
}

/// The grants of every app. Cheap to clone; one copy lives in the shell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grants {
    always: BTreeSet<(String, String)>,
    once: BTreeSet<(String, String)>,
}

fn valid(app: &str, key: &str) -> bool {
    let ok = |s: &str| !s.is_empty() && s.len() <= MAX_KEY_LEN && !s.chars().any(|c| c.is_whitespace() || c.is_control());
    ok(app) && ok(key)
}

impl Grants {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `app` holds a grant for `key`.
    pub fn is_granted(&self, app: &str, key: &str) -> bool {
        let k = (app.to_string(), key.to_string());
        self.always.contains(&k) || self.once.contains(&k)
    }

    /// How many grants `app` holds.
    pub fn count(&self, app: &str) -> usize {
        self.always.iter().chain(self.once.iter()).filter(|(a, _)| a == app).count()
    }

    /// Records a grant. Returns `false` and records nothing when the app or key is malformed or the app already holds
    /// [`MAX_GRANTS_PER_APP`] other grants. Always Allow replaces an Allow Once for the same pair.
    pub fn grant(&mut self, app: &str, key: &str, scope: Scope) -> bool {
        if !valid(app, key) {
            return false;
        }
        if !self.is_granted(app, key) && self.count(app) >= MAX_GRANTS_PER_APP {
            return false;
        }
        let k = (app.to_string(), key.to_string());
        match scope {
            Scope::Always => {
                self.once.remove(&k);
                self.always.insert(k);
            }
            Scope::Once => {
                if !self.always.contains(&k) {
                    self.once.insert(k);
                }
            }
        }
        true
    }

    /// Takes one grant back. Returns whether there was one.
    pub fn revoke(&mut self, app: &str, key: &str) -> bool {
        let k = (app.to_string(), key.to_string());
        let a = self.always.remove(&k);
        let o = self.once.remove(&k);
        a || o
    }

    /// Takes back every grant of `app` (the person removed the app's access, or uninstalled it).
    pub fn revoke_app(&mut self, app: &str) {
        self.always.retain(|(a, _)| a != app);
        self.once.retain(|(a, _)| a != app);
    }

    /// The app quit: its Allow Once grants end.
    pub fn end_session(&mut self, app: &str) {
        self.once.retain(|(a, _)| a != app);
    }

    /// The device was unplugged: every Allow Once grant for it ends, for every app.
    pub fn device_gone(&mut self, key: &str) {
        self.once.retain(|(_, k)| k != key);
    }

    /// The keys `app` holds, sorted.
    pub fn granted_keys(&self, app: &str) -> Vec<String> {
        let mut v: Vec<String> = self.always.iter().chain(self.once.iter()).filter(|(a, _)| a == app).map(|(_, k)| k.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// Every app that holds a grant, sorted.
    pub fn apps(&self) -> Vec<String> {
        let mut v: Vec<String> = self.always.iter().chain(self.once.iter()).map(|(a, _)| a.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// The persisted form: one `"<app> <key>"` line per Always grant, sorted. Allow Once is never saved.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for (app, key) in &self.always {
            out.push_str(app);
            out.push(' ');
            out.push_str(key);
            out.push('\n');
        }
        out
    }

    /// Reads the persisted form. Blank lines, `#` comments and malformed lines are skipped, and an app past its cap
    /// keeps the first [`MAX_GRANTS_PER_APP`] lines.
    pub fn from_text(text: &str) -> Self {
        let mut grants = Self::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split_whitespace();
            let (Some(app), Some(key), None) = (parts.next(), parts.next(), parts.next()) else { continue };
            grants.grant(app, key, Scope::Always);
        }
        grants
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(vendor: u16, product: u16, class: u8, interfaces: &[u8]) -> Candidate {
        Candidate { key: usb_key(vendor, product, ""), name: String::new(), vendor_id: vendor, product_id: product, device_class: class, interface_classes: interfaces.to_vec(), hid: false, usages: Vec::new() }
    }

    #[test]
    fn keys_are_stable_and_safe_for_the_text_form() {
        assert_eq!(usb_key(0x046d, 0xc52b, ""), "046d:c52b:-");
        assert_eq!(usb_key(0x046d, 0xc52b, "AB 12\n"), "046d:c52b:AB_12");
        assert_eq!(hid_key(1, 2, "  "), "hid:0001:0002:-");
        assert_eq!(usb_key(1, 2, &"x".repeat(200)).len(), "0001:0002:".len() + 64);
    }

    #[test]
    fn the_system_driven_classes_are_protected_and_vendor_specific_is_not() {
        for c in [0x01, 0x03, 0x08, 0x09, 0x0b, 0x0e, 0x10, 0xe0] {
            assert!(protected_class(c), "{c:#x}");
        }
        for c in [0x00, 0x02, 0x07, 0x0a, 0xff] {
            assert!(!protected_class(c), "{c:#x}");
        }
    }

    #[test]
    fn a_device_is_eligible_when_one_interface_is_not_system_driven() {
        assert!(dev(1, 1, 0, &[0xff]).eligible());
        assert!(dev(1, 1, 0, &[0x03, 0xff]).eligible(), "a composite device with one raw interface");
        assert!(!dev(1, 1, 0, &[0x03]).eligible(), "a keyboard");
        assert!(!dev(1, 1, 0, &[0x08]).eligible(), "a storage stick");
        assert!(!dev(1, 1, 0x09, &[]).eligible(), "a hub, interfaces unknown");
        assert!(dev(1, 1, 0xff, &[]).eligible());
    }

    #[test]
    fn a_filter_needs_every_set_field_and_class_looks_at_interfaces() {
        let c = dev(0x1234, 0x5678, 0, &[0xff, 0x02]);
        assert!(Filter::default().matches(&c));
        assert!(Filter { vendor_id: Some(0x1234), ..Default::default() }.matches(&c));
        assert!(!Filter { vendor_id: Some(0x1234), product_id: Some(1), ..Default::default() }.matches(&c));
        assert!(Filter { class: Some(0x02), ..Default::default() }.matches(&c));
        assert!(!Filter { class: Some(0x07), ..Default::default() }.matches(&c));
    }

    #[test]
    fn a_request_offers_only_eligible_matching_ungranted_devices_up_to_the_limit() {
        let keyboard = dev(1, 1, 0, &[0x03]);
        let a = dev(0x1234, 1, 0, &[0xff]);
        let b = dev(0x1234, 2, 0, &[0xff]);
        let other = dev(0x9999, 3, 0, &[0xff]);
        let connected = vec![keyboard.clone(), a.clone(), b.clone(), other.clone()];
        let none = |_: &str| false;
        let any = candidates_for(&[], &connected, &none);
        assert_eq!(any, vec![a.clone(), b.clone(), other.clone()], "the keyboard is never offered");
        let f = [Filter { vendor_id: Some(0x1234), ..Default::default() }];
        assert_eq!(candidates_for(&f, &connected, &none), vec![a.clone(), b.clone()]);
        let granted_a = |k: &str| k == a.key;
        assert_eq!(candidates_for(&f, &connected, &granted_a), vec![b.clone()]);
        let many: Vec<Candidate> = (0..10u16).map(|i| dev(0x1234, 100 + i, 0, &[0xff])).collect();
        assert_eq!(candidates_for(&[], &many, &none).len(), MAX_CANDIDATES);
    }

    #[test]
    fn once_ends_with_the_app_or_the_device_and_always_does_not() {
        let mut g = Grants::new();
        assert!(g.grant("is.a.app", "1234:0001:-", Scope::Once));
        assert!(g.grant("is.a.app", "1234:0002:-", Scope::Always));
        assert!(g.grant("is.b.app", "1234:0001:-", Scope::Once));
        assert!(g.is_granted("is.a.app", "1234:0001:-") && !g.is_granted("is.c.app", "1234:0001:-"));
        g.device_gone("1234:0001:-");
        assert!(!g.is_granted("is.a.app", "1234:0001:-") && !g.is_granted("is.b.app", "1234:0001:-"));
        assert!(g.is_granted("is.a.app", "1234:0002:-"), "an Always grant survives an unplug");
        g.grant("is.a.app", "1234:0003:-", Scope::Once);
        g.end_session("is.a.app");
        assert_eq!(g.granted_keys("is.a.app"), vec!["1234:0002:-".to_string()]);
    }

    #[test]
    fn always_replaces_once_and_once_never_downgrades_always() {
        let mut g = Grants::new();
        g.grant("a", "k", Scope::Once);
        g.grant("a", "k", Scope::Always);
        g.end_session("a");
        assert!(g.is_granted("a", "k"));
        g.grant("a", "k", Scope::Once);
        g.end_session("a");
        assert!(g.is_granted("a", "k"), "Allow Once after Always keeps the Always");
        assert!(g.revoke("a", "k"));
        assert!(!g.revoke("a", "k"));
    }

    #[test]
    fn an_app_cannot_hold_more_than_the_cap_and_malformed_input_is_refused() {
        let mut g = Grants::new();
        for i in 0..MAX_GRANTS_PER_APP {
            assert!(g.grant("a", &format!("k{i}"), Scope::Once));
        }
        assert!(!g.grant("a", "one-more", Scope::Always));
        assert!(g.grant("a", "k0", Scope::Always), "upgrading an existing grant is not a new grant");
        assert!(g.grant("b", "one-more", Scope::Always), "the cap is per app");
        assert!(!g.grant("", "k", Scope::Once));
        assert!(!g.grant("a", "has space", Scope::Once));
        assert!(!g.grant("a", &"x".repeat(MAX_KEY_LEN + 1), Scope::Once));
        assert!(!g.grant("a\nb", "k", Scope::Once));
    }

    #[test]
    fn the_text_form_round_trips_always_grants_only() {
        let mut g = Grants::new();
        g.grant("is.a.app", "046d:c52b:-", Scope::Always);
        g.grant("is.a.app", "hid:046d:c52b:-", Scope::Always);
        g.grant("is.b.app", "1234:0001:S_1", Scope::Always);
        g.grant("is.b.app", "1234:0002:-", Scope::Once);
        let text = g.to_text();
        assert_eq!(text, "is.a.app 046d:c52b:-\nis.a.app hid:046d:c52b:-\nis.b.app 1234:0001:S_1\n");
        let back = Grants::from_text(&text);
        assert!(back.is_granted("is.b.app", "1234:0001:S_1") && !back.is_granted("is.b.app", "1234:0002:-"));
        assert_eq!(back.apps(), vec!["is.a.app".to_string(), "is.b.app".to_string()]);
    }

    #[test]
    fn hostile_text_is_skipped_line_by_line() {
        let text = "# comment\n\nonlyone\na b c\n../../etc/passwd k\ngood 1234:0001:-\n\u{0}\u{1} x\n";
        let g = Grants::from_text(text);
        assert!(g.is_granted("good", "1234:0001:-"));
        assert!(g.is_granted("../../etc/passwd", "k"), "ids are opaque text, never paths");
        assert_eq!(g.apps().len(), 2);
        assert!(!g.is_granted("a", "b"), "a line with three fields is malformed");
        let big = (0..100).map(|i| format!("app k{i}\n")).collect::<String>();
        assert_eq!(Grants::from_text(&big).count("app"), MAX_GRANTS_PER_APP);
    }

    #[test]
    fn revoking_an_app_removes_every_grant_it_held() {
        let mut g = Grants::new();
        g.grant("a", "k1", Scope::Always);
        g.grant("a", "k2", Scope::Once);
        g.grant("b", "k1", Scope::Always);
        g.revoke_app("a");
        assert_eq!(g.count("a"), 0);
        assert!(g.is_granted("b", "k1"));
    }

    #[test]
    fn the_sheet_names_the_app_and_the_device() {
        let c = Candidate { name: " Logic Probe ".into(), ..dev(1, 2, 0, &[0xff]) };
        assert_eq!(c.label(), "Logic Probe");
        assert_eq!(dev(0x0403, 0x6001, 0, &[0xff]).label(), "USB device 0403:6001");
        assert!(sheet_title("Scope").starts_with("\u{201c}Scope\u{201d} would like to use"));
        let detail = sheet_detail(&c.label());
        assert!(detail.starts_with("\u{201c}Logic Probe\u{201d}.") && detail.contains("App Permissions"));
    }

    #[test]
    fn hid_candidates_are_always_eligible_and_filter_by_usage() {
        let pad = Candidate { key: hid_key(1, 2, ""), name: "Pad".into(), vendor_id: 1, product_id: 2, device_class: 0, interface_classes: vec![], hid: true, usages: vec![(0xff00, 1), (0x0c, 1)] };
        assert!(pad.eligible());
        assert!(Filter { usage_page: Some(0xff00), ..Default::default() }.matches(&pad));
        assert!(Filter { usage_page: Some(0x0c), usage: Some(1), ..Default::default() }.matches(&pad));
        assert!(!Filter { usage_page: Some(0xff00), usage: Some(2), ..Default::default() }.matches(&pad));
        assert!(!Filter { usage_page: Some(0x01), ..Default::default() }.matches(&pad));
        assert!(Filter { usage: Some(1), ..Default::default() }.matches(&pad), "a usage alone matches any page");
        assert!(Filter { vendor_id: Some(1), ..Default::default() }.matches(&pad));
    }
}
