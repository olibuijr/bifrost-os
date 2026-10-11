//! What the bifrost Storage, Passcode and Developer pages list, as data (the
//! same way `pages::spec` builds the others), the usage bar those pages
//! draw, and the Developer switches' view of the Android properties.

use super::list::{Body, Group, Item, Spec, Tint};
use super::pages::{Action, Command, PageId};
use super::passcode::Purpose;
use super::storage::{bar_widths, human, Category, Usage, CLEARS};
use super::ui::{Ui, Weight, LABEL, TRACK_C};
use crate::bare::sys;
use crate::gfx::{pack, Surface};
use std::path::Path;

/// The segment colours, in `storage::CATEGORIES` order, then the system share.
const COLORS: [u32; 7] = [pack(255, 159, 10, 255), pack(255, 69, 58, 255), pack(10, 132, 255, 255), pack(48, 209, 88, 255), pack(94, 92, 230, 255), pack(100, 210, 255, 255), pack(142, 142, 147, 255)];
const SYSTEM: u32 = pack(99, 99, 102, 255);

pub fn color(c: Category) -> u32 {
    COLORS[super::storage::CATEGORIES.iter().position(|&k| k == c).unwrap_or(0)]
}

/// The usage bar's content: a caption and coloured segments out of `total`.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub caption: String,
    pub total: u64,
    pub segments: Vec<(u64, u32)>,
}

impl Bar {
    /// The bar for `usage`: categories, then the rest of the used space as
    /// system, then the free track; "Calculating…" before the first result.
    pub fn of(usage: Option<&Usage>) -> Self {
        let Some(u) = usage.filter(|u| u.total > 0) else {
            return Bar { caption: "Calculating…".into(), total: 0, segments: vec![] };
        };
        let mut segments: Vec<(u64, u32)> = u.cats.iter().map(|&(c, b)| (b, color(c))).collect();
        segments.push((u.system(), SYSTEM));
        Bar { caption: format!("{} of {} Used", human(u.used()), human(u.total)), total: u.total, segments }
    }

    /// Draws into `(x, y, w, h)` px: the caption, then the bar.
    pub fn draw(&self, s: &mut Surface, ui: &Ui, (x, y, w, h): (f32, f32, f32, f32)) {
        let caption = ui.text(&self.caption, 15.0, Weight::Regular, LABEL.now());
        s.blit(&caption, x as i32, (y + ui.pt(10.0)) as i32, 255, 255);
        let (bh, by) = (ui.pt(14.0), y + h - ui.pt(14.0 + 14.0));
        let r = ui.pt(3.0).min(3.0);
        s.fill_rrect(x, by, w, bh, r, TRACK_C.now());
        let widths = bar_widths(&self.segments.iter().map(|&(b, _)| b).collect::<Vec<_>>(), self.total, w as u32);
        let mut at = x;
        for (&(_, color), &sw) in self.segments.iter().zip(&widths) {
            if sw > 0 {
                // squared joins between segments; the track keeps the rounded ends
                s.fill_rrect(at, by, sw as f32, bh, 0.0, color);
                at += sw as f32;
            }
        }
    }
}

/// Live state the three pages are built from.
pub struct PageState<'a> {
    pub usage: Option<&'a Usage>,
    pub passcode_on: bool,
    pub dev: &'a DevState,
}

pub fn spec(id: PageId, st: &PageState) -> Spec {
    let page = |title: &str, groups: Vec<Group>| Spec { id, title: title.into(), large: false, groups };
    match id {
        PageId::Storage => {
            let u = st.usage;
            let mut groups = vec![Group::new(vec![Item { body: Body::Bar(Bar::of(u)), ..Item::new("") }]).header("THIS COMPUTER")];
            if let Some(u) = u {
                let mut rows: Vec<Item> = u.cats.iter().map(|&(c, b)| Item::new(c.name()).value(human(b))).collect();
                rows.push(Item::new("System").value(human(u.system())));
                rows.push(Item::new("Available").value(human(u.free)));
                groups.push(Group::new(rows));
                for c in CLEARS {
                    let n = u.reclaimable(c);
                    let item = Item::new(c.label()).value(human(n)).action(Action::Run(Command::ClearCache(c)));
                    groups.push(Group::new(vec![item.tint(if n > 0 { Tint::Blue } else { Tint::Dim })]).footer(c.footer()));
                }
            }
            page("Storage", groups)
        }
        PageId::Passcode => {
            let mut groups = vec![];
            if st.passcode_on {
                groups.push(Group::new(vec![Item::new("Turn Passcode Off").tint(Tint::Blue).action(Action::Passcode(Purpose::TurnOff, false)), Item::new("Change Passcode").tint(Tint::Blue).action(Action::Passcode(Purpose::Change, false))]));
                groups.push(Group::new(vec![Item::new("Require Passcode").value("Immediately")]));
                groups.push(Group::new(vec![Item::new("Custom Numeric Code").tint(Tint::Blue).action(Action::Passcode(Purpose::Change, true))]).footer("Change to a code of 4 to 16 digits."));
            } else {
                groups.push(Group::new(vec![Item::new("Turn Passcode On").tint(Tint::Blue).action(Action::Passcode(Purpose::TurnOn, false))]).footer("Asks for a passcode when the screen locks."));
                groups.push(Group::new(vec![Item::new("Custom Numeric Code").tint(Tint::Blue).action(Action::Passcode(Purpose::TurnOn, true))]));
            }
            page("Passcode", groups)
        }
        _ => page(
            "Developer",
            vec![
                Group::new(vec![Item::new("Performance HUD").switch(st.dev.perf_hud, Action::Toggle(super::pages::Toggle::PerfHud))])
                    .footer("Overlays frame time, frames per second, CPU temperature, memory use and battery current on every screen."),
                Group::new(vec![Item::new("Zero-Copy GPU Output").switch(st.dev.gpu_scanout, Action::Toggle(super::pages::Toggle::GpuScanout))])
                    .footer("Experimental: GPU frames go straight to the display. Turn off if the screen misbehaves. Takes effect after the next start."),
            ],
        ),
    }
}

// -- Developer ------------------------------------------------------------------

/// Android's boot-time root switch, written by tools/install-boot.sh.
pub const LOCAL_PROP: &str = "/data/local.prop";
const ROOT_PROP: &str = "service.adb.root";
const TCP_PROP: &str = "persist.adb.tcp.port";
const TCP_PORT: &str = "5555";

/// Whether adbd runs as root: the live property wins, and while it is
/// unset the boot file decides.
pub fn rooted(prop: &str, local_prop: bool) -> bool {
    match prop.trim() {
        "0" => false,
        "1" => true,
        _ => local_prop,
    }
}

/// Whether adb listens on TCP: unset counts as on (this bifrost's default),
/// a port of 0 or less is off.
pub fn wifi_adb(prop: &str) -> bool {
    match prop.trim() {
        "" => true,
        p => p.parse::<i32>().map_or(true, |n| n > 0),
    }
}

/// The boot file's contents: the root switch (only while rooted) and the adb TCP port keys.
/// adbd reads them when it next starts; writing the file never touches the running adbd.
pub fn local_prop(rooted: bool, wifi_adb: bool) -> String {
    let port = if wifi_adb { TCP_PORT } else { "0" };
    let mut s = String::new();
    if rooted {
        s.push_str("service.adb.root=1\n");
    }
    s.push_str(&format!("service.adb.tcp.port={port}\npersist.adb.tcp.port={port}\n"));
    s
}

/// The value of `key` in a `local.prop` body.
fn prop_in<'a>(body: &'a str, key: &str) -> Option<&'a str> {
    body.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(str::trim)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DevState {
    pub rooted: bool,
    pub wifi_adb: bool,
    /// a switch changed the boot file; adbd only picks it up after a restart
    pub restart_pending: bool,
    /// the performance overlay (a pref, filled in by Settings)
    pub perf_hud: bool,
    /// zero-copy GPU output (a pref, read at start)
    pub gpu_scanout: bool,
}

impl DevState {
    pub fn new(rooted: bool, wifi_adb: bool) -> Self {
        DevState { rooted, wifi_adb, restart_pending: false, perf_hud: false, gpu_scanout: false }
    }

    /// Reads the real state from `/data/local.prop` (what the next boot will use), falling back
    /// to the live properties for what the file does not say.
    pub fn read() -> Self {
        let body = std::fs::read_to_string(LOCAL_PROP).ok();
        let file_root = body.as_deref().map(|b| prop_in(b, ROOT_PROP) == Some("1"));
        let file_port = body.as_deref().and_then(|b| prop_in(b, TCP_PROP).or_else(|| prop_in(b, "service.adb.tcp.port")));
        let live_port = sys::get_prop(TCP_PROP);
        DevState::new(rooted(&sys::get_prop(ROOT_PROP), file_root.unwrap_or(false)), wifi_adb(file_port.unwrap_or(&live_port)))
    }

    /// Turns root adbd on or off for this and the next boot: the live property follows.
    pub fn set_rooted(&mut self, on: bool) {
        self.set_rooted_at(Path::new(LOCAL_PROP), on);
    }

    pub fn set_rooted_at(&mut self, file: &Path, on: bool) {
        self.rooted = on;
        self.write_at(file);
        sys::set_prop(ROOT_PROP, if on { "1" } else { "0" });
    }

    /// Records whether adb listens on TCP (5555 or 0) in the boot file. It does not restart adbd,
    /// which is the shell's parent; the change applies after a restart.
    pub fn set_wifi_adb(&mut self, on: bool) {
        self.set_wifi_adb_at(Path::new(LOCAL_PROP), on);
    }

    pub fn set_wifi_adb_at(&mut self, file: &Path, on: bool) {
        self.wifi_adb = on;
        self.restart_pending = true;
        self.write_at(file);
    }

    fn write_at(&self, file: &Path) {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::write(file, local_prop(self.rooted, self.wifi_adb)).is_ok() {
            let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
        } else {
            eprintln!("springboard: cannot write {}", file.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::storage::{Clear, CATEGORIES};

    fn usage() -> Usage {
        Usage { total: 32_000_000_000, free: 10_000_000_000, cats: CATEGORIES.iter().map(|&c| (c, 1_000_000_000)).collect(), reclaim: vec![(Clear::DeilduCache, 5_000_000), (Clear::Thumbnails, 0)] }
    }

    fn labels(s: &Spec) -> Vec<String> {
        s.groups.iter().flat_map(|g| g.items.iter().map(|i| i.label.clone())).collect()
    }

    #[test]
    fn storage_page_shows_a_bar_the_categories_and_the_clear_actions() {
        let dev = DevState::new(true, true);
        let u = usage();
        let s = spec(PageId::Storage, &PageState { usage: Some(&u), passcode_on: false, dev: &dev });
        assert_eq!(s.title, "Storage");
        assert!(matches!(&s.groups[0].items[0].body, Body::Bar(b) if b.caption == "22.0 GB of 32.0 GB Used" && b.segments.len() == 8));
        let l = labels(&s);
        assert!(l.contains(&"Photos".to_string()) && l.contains(&"Logs".to_string()) && l.contains(&"System".to_string()) && l.contains(&"Available".to_string()));
        let clear = s.groups.iter().flat_map(|g| &g.items).find(|i| i.label == "Clear Deildu Cache").unwrap();
        assert_eq!(clear.value, "5.0 MB");
        assert_eq!(clear.action, Action::Run(Command::ClearCache(Clear::DeilduCache)));
        assert!(clear.tappable());
        let empty = s.groups.iter().flat_map(|g| &g.items).find(|i| i.label == "Clear Thumbnails & Artwork").unwrap();
        assert!(!empty.tappable(), "nothing to clear greys the row out");
    }

    #[test]
    fn storage_page_before_the_scan_finishes_only_has_the_bar() {
        let dev = DevState::new(true, true);
        let s = spec(PageId::Storage, &PageState { usage: None, passcode_on: false, dev: &dev });
        assert_eq!(s.groups.len(), 1);
        assert!(matches!(&s.groups[0].items[0].body, Body::Bar(b) if b.caption == "Calculating…" && b.segments.is_empty()));
    }

    #[test]
    fn bar_segments_follow_the_categories_then_system() {
        let u = usage();
        let b = Bar::of(Some(&u));
        assert_eq!(b.segments[0], (1_000_000_000, color(Category::Photos)));
        assert_eq!(b.segments[7], (u.system(), SYSTEM));
        assert_eq!(b.segments.iter().map(|s| s.0).sum::<u64>(), u.used());
        assert_eq!(Bar::of(Some(&Usage::default())).caption, "Calculating…");
    }

    #[test]
    fn passcode_page_offers_what_fits_the_state() {
        let dev = DevState::new(true, true);
        let off = spec(PageId::Passcode, &PageState { usage: None, passcode_on: false, dev: &dev });
        assert_eq!(labels(&off), ["Turn Passcode On", "Custom Numeric Code"]);
        assert_eq!(off.groups[0].items[0].action, Action::Passcode(Purpose::TurnOn, false));
        let on = spec(PageId::Passcode, &PageState { usage: None, passcode_on: true, dev: &dev });
        assert_eq!(labels(&on), ["Turn Passcode Off", "Change Passcode", "Require Passcode", "Custom Numeric Code"]);
        assert_eq!(on.groups[1].items[0].value, "Immediately");
        assert_eq!(on.groups[0].items[1].action, Action::Passcode(Purpose::Change, false));
    }

    #[test]
    fn developer_page_shows_the_real_switch_states() {
        let dev = DevState::new(false, true);
        let s = spec(PageId::Developer, &PageState { usage: None, passcode_on: false, dev: &dev });
        assert_eq!(labels(&s), ["Performance HUD", "Zero-Copy GPU Output"]);
        assert_eq!(s.groups[0].items[0].accessory, super::super::list::Accessory::Switch(false));
        assert_eq!(s.groups[1].items[0].accessory, super::super::list::Accessory::Switch(false));
    }

    #[test]
    fn properties_decide_the_switch_states() {
        assert!(rooted("1", false));
        assert!(!rooted("0", true), "the live property wins over the boot file");
        assert!(rooted("", true));
        assert!(!rooted("", false));
        assert!(wifi_adb("") && wifi_adb("5555") && !wifi_adb("0") && !wifi_adb("-1"));
    }

    #[test]
    fn boot_file_carries_root_and_the_adb_port() {
        assert_eq!(local_prop(true, true), "service.adb.root=1\nservice.adb.tcp.port=5555\npersist.adb.tcp.port=5555\n");
        assert_eq!(local_prop(false, false), "service.adb.tcp.port=0\npersist.adb.tcp.port=0\n");
        assert_eq!(prop_in(&local_prop(true, false), TCP_PROP), Some("0"));
    }

    #[test]
    fn wifi_adb_switch_only_rewrites_the_boot_file_and_asks_for_a_restart() {
        let dir = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-storage-dev")).join(std::process::id().to_string());
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("local.prop");
        let mut d = DevState::new(true, true);
        d.set_wifi_adb_at(&file, false);
        assert!(!d.wifi_adb && d.restart_pending);
        let body = std::fs::read_to_string(&file).unwrap();
        assert!(body.contains("service.adb.root=1") && body.contains("persist.adb.tcp.port=0"));
        d.set_rooted_at(&file, false);
        let body = std::fs::read_to_string(&file).unwrap();
        assert!(!body.contains("service.adb.root") && body.contains("service.adb.tcp.port=0"));
        d.set_rooted_at(&file, true);
        d.set_wifi_adb_at(&file, true);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), local_prop(true, true));
    }
}
