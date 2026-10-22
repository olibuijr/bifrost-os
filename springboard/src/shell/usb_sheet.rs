//! The accessory approval sheet: when a new USB device appears, the shell runs the arrival flow of
//! `bifrost_core::usb_approval` (HIDRMKit, HIDRMClientKit), holds an unknown device blocked, and asks the person on an
//! alert card with three buttons (Always Allow, Allow Once, Don't Allow, HIDRMUI).
//!
//! * Detection: the shell polls `/sys/bus/usb/devices` once a second (the camera does the same for its devices) and
//!   treats a port it has not seen with this device on it as an arrival. Devices already on the bus at start are
//!   arrivals too, so a device the kernel holds unauthorized is asked about instead of staying silently dead.
//! * Decision: `Approvals::arrive_with` decides from the stored answers and the kernel's `authorized` attribute. A
//!   device the kernel already authorized (hubs, built-in parts) and nobody denied is allowed without a sheet. Only a
//!   device the kernel holds blocked and the person never answered gets the sheet, and it stays blocked meanwhile.
//! * Writing: the shell is not root, so a change to the `authorized` attribute goes to `bifrost-privd`
//!   (`usb.authorize`, which only writes device ports). A write that changes nothing is skipped. A failed write is
//!   tried again on the next scan.
//! * Answers: "Always Allow" and "Don't Allow" are kept in `usb_approvals.txt` in the settings folder, "Allow Once"
//!   lasts until the shell restarts. A device that is unplugged while the sheet is up takes the sheet with it.
//!
//! The sheet waits while the screen is locked (the device stays blocked), then asks about the devices one at a time.
//! It takes every touch while it is up; typing is ignored, so the default stays "blocked".
//!
//! Note: on a stock kernel every new device is authorized at once (`authorized_default` 1), so the sheet only meets
//! devices the system holds back: privd's `usb.policy_set` sets `usbcore.authorized_default` to 2 (authorize internal
//! devices only) and keeps it across reboots (`usb_approval::Policy`).

use super::ui::{AlertCard, Ui, Weight, LINK, MAC_BODY_PT};
use super::Drawn;
use crate::gfx::Surface;
use bifrost_core::usb_approval::{self, Answer, Approvals, Decision, Prompt, Step};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};

/// The file in the settings folder that keeps the "always" answers.
pub const APPROVALS_FILE: &str = "usb_approvals.txt";

/// How often the bus is read, in the shell clock's milliseconds.
pub const SCAN_MS: f64 = 1000.0;

/// The card is wider than a standard alert: three labels sit in one row.
const CARD_WIDTH_PT: f32 = 340.0;

/// Writes a decision to a device port's `authorized` attribute: `(devices root, port, decision)`.
pub type Writer = fn(&Path, &str, Decision) -> io::Result<()>;

/// The real writer: the request goes to privd, which owns the sysfs write.
fn via_privd(_root: &Path, port: &str, decision: Decision) -> io::Result<()> {
    crate::privd_client::connect().usb_authorize(port, decision == Decision::Allow).map_err(|e| io::Error::other(e.message))
}

/// One question waiting for the sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Ask {
    port: String,
    key: String,
    prompt: Prompt,
}

/// What the shell knows about the bus: the approvals, the devices already handled, and the questions waiting.
pub struct Watch {
    /// The sysfs devices directory; empty turns the scan off (host tests that do not use the bus).
    root: PathBuf,
    write: Writer,
    approvals: Approvals,
    /// The device handled on each port, as `vvvv:pppp:serial` text.
    seen: BTreeMap<String, String>,
    /// Ports whose last write failed, so the error is printed once.
    failed: BTreeSet<String>,
    waiting: VecDeque<Ask>,
    next_scan: f64,
    store: Option<PathBuf>,
}

impl Default for Watch {
    fn default() -> Self {
        // unit tests never read the machine's bus or ask privd to write it
        let root = if cfg!(test) { PathBuf::new() } else { PathBuf::from(usb_approval::SYSFS_USB_DEVICES) };
        Watch::new(root, via_privd)
    }
}

fn attr(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).map(|s| s.trim().to_string()).unwrap_or_default()
}

impl Watch {
    pub fn new(root: PathBuf, write: Writer) -> Watch {
        Watch { root, write, approvals: Approvals::new(), seen: BTreeMap::new(), failed: BTreeSet::new(), waiting: VecDeque::new(), next_scan: 0.0, store: None }
    }

    /// Reads the saved answers from `file` and keeps writing them there.
    pub fn attach(&mut self, file: PathBuf) {
        if let Ok(text) = std::fs::read_to_string(&file) {
            self.approvals = Approvals::from_text(&text);
        }
        self.store = Some(file);
    }

    fn save(&self) {
        let Some(file) = &self.store else { return };
        if let Err(e) = std::fs::write(file, self.approvals.to_text()) {
            eprintln!("springboard: usb approvals: {e}");
        }
    }

    /// Whether a device is on `port` now.
    fn present(&self, port: &str) -> bool {
        !self.root.as_os_str().is_empty() && self.root.join(port).join("idVendor").is_file()
    }

    /// Writes `decision` for `port` unless the kernel already holds that state.
    fn apply(&self, port: &str, kernel: Option<bool>, decision: Decision) -> io::Result<()> {
        if kernel == Some(decision == Decision::Allow) {
            return Ok(());
        }
        (self.write)(&self.root, port, decision)
    }

    /// Reads the bus: runs the arrival flow for every port with a device not handled yet, and forgets ports that emptied.
    /// Returns whether anything changed (a question was queued or a question went away).
    pub fn scan(&mut self) -> bool {
        if self.root.as_os_str().is_empty() {
            return false;
        }
        let ports = usb_approval::device_ports(&self.root);
        let mut changed = false;
        let gone: Vec<String> = self.seen.keys().filter(|p| !ports.contains(p)).cloned().collect();
        for port in gone {
            self.seen.remove(&port);
            self.failed.remove(&port);
            let before = self.waiting.len();
            self.waiting.retain(|a| a.port != port);
            changed |= self.waiting.len() != before;
        }
        for port in ports {
            let dir = self.root.join(&port);
            let (vendor, product, serial, name) = (attr(&dir, "idVendor"), attr(&dir, "idProduct"), attr(&dir, "serial"), attr(&dir, "product"));
            let id = format!("{vendor}:{product}:{serial}");
            if self.seen.get(&port) == Some(&id) {
                continue;
            }
            // a different device on a port that held one: the old question is about someone else
            self.waiting.retain(|a| a.port != port);
            let kernel = usb_approval::read_kernel(&self.root, &port);
            let result = {
                let me = &*self;
                me.approvals.arrive_with(kernel, &vendor, &product, &serial, &name, &mut |d| me.apply(&port, kernel, d))
            };
            match result {
                Ok(step) => {
                    self.seen.insert(port.clone(), id);
                    self.failed.remove(&port);
                    if let Step::Ask { key, prompt } = step {
                        self.waiting.push_back(Ask { port, key, prompt });
                        changed = true;
                    }
                }
                Err(e) => {
                    if self.failed.insert(port.clone()) {
                        eprintln!("springboard: usb {port}: {e}");
                    }
                }
            }
        }
        changed
    }

    /// The next question for the sheet.
    fn next_ask(&mut self) -> Option<Ask> {
        self.waiting.pop_front()
    }

    /// The person answered: record it and write the result to the kernel. A failed write is printed; the answer stays
    /// recorded, and the next arrival of the device applies it.
    fn finish(&mut self, port: &str, key: &str, answer: Answer) -> Option<Decision> {
        let kernel = usb_approval::read_kernel(&self.root, port);
        let mut approvals = std::mem::take(&mut self.approvals);
        let result = approvals.finish_with(key, answer, &mut |d| self.apply(port, kernel, d));
        self.approvals = approvals;
        self.save();
        match result {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("springboard: usb {port}: {e}");
                // ask again on the next scan, which finds the kernel state unchanged and applies the stored answer
                self.seen.remove(port);
                None
            }
        }
    }

    #[cfg(test)]
    pub fn waiting_len(&self) -> usize {
        self.waiting.len()
    }
}

/// The sheet on screen: the device it asks about and the card with the three buttons.
pub struct UsbSheet {
    port: String,
    key: String,
    card: AlertCard,
}

impl UsbSheet {
    fn new(ui: &Ui, ask: Ask) -> UsbSheet {
        // the safe choice, Don't Allow, is the emphasised one: an unanswered question never grants
        let labels = ask.prompt.buttons.iter().enumerate().map(|(i, l)| ui.text(l, MAC_BODY_PT, if i == 2 { Weight::Semibold } else { Weight::Regular }, LINK)).collect();
        let card = AlertCard::new_wide(ui, &ask.prompt.title, &ask.prompt.message, labels, CARD_WIDTH_PT);
        UsbSheet { port: ask.port, key: ask.key, card }
    }

    /// The answer of the button under (x, y), if any.
    pub fn hit(&self, ui: &Ui, w: i32, h: i32, x: f32, y: f32) -> Option<Answer> {
        self.card.hit(ui, w, h, x, y).and_then(Prompt::answer_for)
    }

    /// Centre of button `index`, for tests.
    #[cfg(test)]
    pub fn button(&self, ui: &Ui, w: i32, h: i32, index: usize) -> (f32, f32) {
        self.card.button_centre(ui, w, h, index)
    }

    pub fn draw(&self, s: &mut Surface, ui: &Ui, w: i32, h: i32) {
        self.card.draw(s, ui, w, h);
    }
}

impl super::Shell {
    /// Whether the approval sheet is on screen. It takes every touch while it is.
    pub(super) fn usb_showing(&self) -> bool {
        self.usb_sheet.is_some()
    }

    /// Reads the bus on the scan interval, drops a sheet whose device was unplugged, and shows the next question when
    /// the screen is unlocked and no approval sheet is up.
    pub(super) fn usb_tick(&mut self, t: f64) {
        if t >= self.usb_watch.next_scan {
            self.usb_watch.next_scan = t + SCAN_MS;
            if self.usb_watch.scan() {
                self.dirty = true;
            }
        }
        if self.usb_sheet.as_ref().is_some_and(|s| !self.usb_watch.present(&s.port)) {
            self.usb_sheet = None;
            self.dirty = true;
            self.drawn = Drawn::Nothing;
        }
        if self.usb_sheet.is_some() || self.is_locked() {
            return;
        }
        if let Some(ask) = self.usb_watch.next_ask() {
            self.usb_sheet = Some(UsbSheet::new(&self.ui, ask));
            self.dirty = true;
            self.drawn = Drawn::Nothing;
        }
    }

    /// Whether a release at (x, y) ends a press that went down on the same button of the sheet.
    pub(super) fn usb_press_lands(&self, press: Option<(f32, f32)>, x: f32, y: f32) -> bool {
        super::press_lands(press, (x, y), |px, py| self.usb_sheet.as_ref().and_then(|s| s.hit(&self.ui, self.w, self.h, px, py)))
    }

    /// A touch on the sheet: a button answers it, anything else is ignored.
    pub(super) fn usb_touch(&mut self, x: f32, y: f32, up: bool) {
        if up {
            let hit = self.usb_sheet.as_ref().and_then(|s| s.hit(&self.ui, self.w, self.h, x, y));
            if let Some(answer) = hit {
                if let Some(sheet) = self.usb_sheet.take() {
                    self.usb_watch.finish(&sheet.port, &sheet.key, answer);
                }
            }
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Draws the card over a dimmed screen.
    pub(super) fn usb_draw(&self, s: &mut Surface) {
        if let Some(sheet) = &self.usb_sheet {
            sheet.draw(s, &self.ui, self.w, self.h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::Screen;

    fn direct(root: &Path, port: &str, decision: Decision) -> io::Result<()> {
        usb_approval::enforce(root, port, decision).map(|_| ())
    }

    fn bus(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("usb-sheet-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn plug(root: &Path, port: &str, authorized: &str, vendor: &str, product: &str, name: &str) {
        let dir = root.join(port);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("authorized"), authorized).unwrap();
        std::fs::write(dir.join("idVendor"), format!("{vendor}\n")).unwrap();
        std::fs::write(dir.join("idProduct"), format!("{product}\n")).unwrap();
        std::fs::write(dir.join("product"), format!("{name}\n")).unwrap();
    }

    fn kernel(root: &Path, port: &str) -> Option<bool> {
        usb_approval::read_kernel(root, port)
    }

    fn shell_on(root: &Path) -> crate::shell::Shell {
        let mut sh = crate::shell::tests::shell();
        // a new shell starts on the lock screen, and no sheet goes over it: these tests are about an unlocked person
        sh.screen = Screen::Home;
        sh.usb_watch = Watch::new(root.to_path_buf(), direct);
        sh
    }

    fn tap_button(sh: &mut crate::shell::Shell, index: usize) {
        let (x, y) = sh.usb_sheet.as_ref().expect("the sheet is up").button(&sh.ui, sh.w, sh.h, index);
        sh.usb_touch(x, y, true);
    }

    #[test]
    fn a_held_device_gets_the_sheet_and_always_allow_writes_the_kernel_and_the_file() {
        let root = bus("allow");
        plug(&root, "1-4", "0", "046d", "c52b", "Wireless Keyboard");
        let store = root.join("approvals.txt");
        let mut sh = shell_on(&root);
        sh.usb_watch.attach(store.clone());
        sh.usb_tick(0.0);
        assert!(sh.usb_showing());
        assert_eq!(kernel(&root, "1-4"), Some(false), "held blocked while the sheet is up");
        // a touch outside the buttons answers nothing
        sh.usb_touch(2.0, 2.0, true);
        assert!(sh.usb_showing() && kernel(&root, "1-4") == Some(false));
        // a press without the release does nothing either
        let (x, y) = sh.usb_sheet.as_ref().unwrap().button(&sh.ui, sh.w, sh.h, 0);
        sh.usb_touch(x, y, false);
        assert!(sh.usb_showing());
        tap_button(&mut sh, 0);
        assert!(!sh.usb_showing());
        assert_eq!(kernel(&root, "1-4"), Some(true));
        assert_eq!(std::fs::read_to_string(&store).unwrap(), "046d:c52b:- allow\n");
        // unplugged and plugged in again: allowed at once, no sheet
        std::fs::remove_dir_all(root.join("1-4")).unwrap();
        sh.usb_tick(1000.0);
        plug(&root, "1-4", "0", "046d", "c52b", "Wireless Keyboard");
        sh.usb_tick(2000.0);
        assert!(!sh.usb_showing());
        assert_eq!(kernel(&root, "1-4"), Some(true));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn dont_allow_keeps_the_device_blocked_and_is_remembered() {
        let root = bus("deny");
        plug(&root, "1-2", "0", "1234", "0001", "Stick");
        let mut sh = shell_on(&root);
        sh.usb_tick(0.0);
        tap_button(&mut sh, 2);
        assert_eq!(kernel(&root, "1-2"), Some(false));
        // the kernel authorizes it again later (a replug): the stored denial blocks it without a question
        std::fs::write(root.join("1-2").join("authorized"), "1").unwrap();
        std::fs::remove_dir_all(root.join("1-2")).unwrap();
        sh.usb_tick(1000.0);
        plug(&root, "1-2", "1", "1234", "0001", "Stick");
        sh.usb_tick(2000.0);
        assert!(!sh.usb_showing());
        assert_eq!(kernel(&root, "1-2"), Some(false));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn allow_once_lets_the_device_in_but_is_not_saved() {
        let root = bus("once");
        plug(&root, "2-1", "0", "aaaa", "bbbb", "");
        let store = root.join("approvals.txt");
        let mut sh = shell_on(&root);
        sh.usb_watch.attach(store.clone());
        sh.usb_tick(0.0);
        assert!(sh.usb_sheet.is_some());
        tap_button(&mut sh, 1);
        assert_eq!(kernel(&root, "2-1"), Some(true));
        assert_eq!(std::fs::read_to_string(&store).unwrap(), "");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_device_the_kernel_already_authorized_needs_no_sheet_and_no_write() {
        fn refuse(_: &Path, _: &str, _: Decision) -> io::Result<()> {
            Err(io::Error::other("nothing should be written"))
        }
        let root = bus("hub");
        plug(&root, "1-1", "1", "05e3", "0610", "Hub");
        let mut sh = crate::shell::tests::shell();
        sh.usb_watch = Watch::new(root.clone(), refuse);
        sh.usb_tick(0.0);
        assert!(!sh.usb_showing() && sh.usb_watch.waiting_len() == 0);
        assert_eq!(kernel(&root, "1-1"), Some(true));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_sheet_waits_for_the_unlock_and_goes_when_the_device_is_unplugged() {
        let root = bus("lock");
        plug(&root, "1-3", "0", "0001", "0002", "Gadget");
        let mut sh = shell_on(&root);
        let screen = sh.screen;
        sh.screen = Screen::Lock;
        sh.usb_tick(0.0);
        assert!(!sh.usb_showing(), "no sheet over the lock screen");
        assert_eq!(kernel(&root, "1-3"), Some(false), "still held");
        sh.screen = screen;
        sh.usb_tick(10.0);
        assert!(sh.usb_showing());
        std::fs::remove_dir_all(root.join("1-3")).unwrap();
        sh.usb_tick(1500.0);
        assert!(!sh.usb_showing(), "the sheet goes with the device");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn devices_are_asked_about_one_at_a_time() {
        let root = bus("queue");
        plug(&root, "1-5", "0", "0001", "0001", "One");
        plug(&root, "1-6", "0", "0002", "0002", "Two");
        let mut sh = shell_on(&root);
        sh.usb_tick(0.0);
        assert!(sh.usb_showing());
        assert_eq!(sh.usb_watch.waiting_len(), 1);
        tap_button(&mut sh, 0);
        assert!(!sh.usb_showing());
        sh.usb_tick(10.0);
        assert!(sh.usb_showing(), "the second device is asked after the first is answered");
        tap_button(&mut sh, 2);
        assert_eq!(kernel(&root, "1-5"), Some(true));
        assert_eq!(kernel(&root, "1-6"), Some(false));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_saved_answers_are_read_back_at_start() {
        let root = bus("saved");
        plug(&root, "1-7", "0", "046d", "c52b", "Keyboard");
        let store = root.join("approvals.txt");
        std::fs::write(&store, "046d:c52b:- allow\n").unwrap();
        let mut sh = shell_on(&root);
        sh.usb_watch.attach(store);
        sh.usb_tick(0.0);
        assert!(!sh.usb_showing());
        assert_eq!(kernel(&root, "1-7"), Some(true), "the saved answer is applied to the held device");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_scan_is_off_without_a_bus_and_runs_once_a_second() {
        let mut sh = crate::shell::tests::shell();
        sh.usb_tick(0.0);
        assert!(!sh.usb_showing(), "tests do not read the machine's bus");
        let root = bus("rate");
        let mut sh = shell_on(&root);
        sh.usb_tick(0.0);
        plug(&root, "1-8", "0", "0003", "0003", "Late");
        sh.usb_tick(500.0);
        assert!(!sh.usb_showing(), "not read again before the interval");
        sh.usb_tick(1000.0);
        assert!(sh.usb_showing());
        let _ = std::fs::remove_dir_all(&root);
    }
}
