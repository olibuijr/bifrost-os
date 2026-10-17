//! The unlock sheet for encrypted volumes that are still locked after login.
//!
//! A worker thread lists the LUKS volumes with no open mapping (`lsblk`, through `bifrost_core::volume_unlock`) and
//! queues them. Once the desktop is unlocked, the shell takes one device at a time and draws the password sheet
//! (the `auth_sheet.rs` pattern: the Wi-Fi card with a secure field). Continue hands the passphrase to
//! `privd` (`disk.unlock_volume`), which opens and mounts the volume. Cancel leaves the volume locked for this
//! session. A failed unlock queues the volume again, so the sheet comes back until the password is right or
//! the person cancels. The passphrase is never stored.

use super::forms::{self, Purpose};
use super::keyboard::Out;
use super::wifi_page::{Sheet, SheetHit};
use super::Drawn;
use crate::gfx::{rgba, Surface};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

/// Devices waiting for the sheet, oldest first.
static QUEUE: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Devices the sheet has already been shown for this session.
static ASKED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// The worker that lists the volumes has been started.
static SCANNED: AtomicBool = AtomicBool::new(false);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The sheet on screen and the volume it unlocks.
pub struct VolumeSheet {
    pub device: String,
    pub sheet: Sheet,
}

/// Lists the locked encrypted volumes and queues them. Runs on a worker thread; a missing `lsblk` queues nothing.
pub fn scan() {
    let Ok(out) = std::process::Command::new("lsblk").args(["-rpno", "NAME,TYPE,FSTYPE,PKNAME"]).output() else { return };
    queue_locked(&String::from_utf8_lossy(&out.stdout));
}

/// Queues the locked volumes in `listing` that have not been asked about yet.
pub fn queue_locked(listing: &str) {
    let locked = bifrost_core::volume_unlock::locked(listing);
    let asked = lock(&ASKED).clone();
    let mut queue = lock(&QUEUE);
    for device in locked {
        if !asked.contains(&device) && !queue.contains(&device) {
            queue.push(device);
        }
    }
}

/// The next volume to ask about. It is marked as asked, so a cancelled sheet does not come back.
pub fn take() -> Option<String> {
    let mut queue = lock(&QUEUE);
    if queue.is_empty() {
        return None;
    }
    let device = queue.remove(0);
    lock(&ASKED).push(device.clone());
    Some(device)
}

/// A failed unlock: the volume goes back to the front of the queue and is asked about again.
pub fn requeue(device: &str) {
    lock(&ASKED).retain(|d| d != device);
    let mut queue = lock(&QUEUE);
    if !queue.iter().any(|d| d == device) {
        queue.insert(0, device.to_string());
    }
}

/// The one-field prompt for a volume: its name as the message, the passphrase in a secure field.
pub fn prompt_for(device: &str) -> forms::Prompt {
    let name = device.rsplit('/').next().unwrap_or(device);
    forms::Prompt {
        purpose: Purpose::VolumeUnlock { device: device.into() },
        title: "Unlock Volume".into(),
        message: format!("The encrypted volume {name} is locked. Enter its password to unlock it."),
        label: "Password",
        ok: "Unlock",
        secure: true,
        max: 512,
        initial: String::new(),
    }
}

impl super::Shell {
    /// Whether the unlock sheet is on screen. Touches and typing belong to it while it is.
    pub(super) fn volume_showing(&self) -> bool {
        self.volume_sheet.is_some()
    }

    /// Starts the volume listing once, and shows the next locked volume when the desktop is unlocked and no other
    /// sheet is up.
    pub(super) fn volume_tick(&mut self, t: f64) {
        if !SCANNED.swap(true, Ordering::Relaxed) {
            std::thread::spawn(scan);
        }
        if self.volume_sheet.is_some() || self.is_locked() {
            return;
        }
        let Some(device) = take() else { return };
        let sheet = Sheet::form(&self.ui, self.w, t, prompt_for(&device));
        self.volume_sheet = Some(VolumeSheet { device, sheet });
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Typing into the sheet: Return unlocks with a passphrase that is not empty, Escape cancels, other keys edit the field.
    pub(super) fn volume_type(&mut self, out: Out, t: f64) {
        if out == Out::Named("Escape") {
            self.volume_close(None);
            return;
        }
        let typed = match self.volume_sheet.as_mut() {
            Some(v) if out == Out::Enter => v.sheet.form_result().map(|(_, text)| text),
            Some(v) => {
                v.sheet.edit(&self.ui, out, t);
                None
            }
            None => return,
        };
        if let Some(password) = typed {
            self.volume_close(Some(password));
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// A touch on the sheet: Cancel or Unlock close it. The sheet takes every touch while it is up.
    pub(super) fn volume_touch(&mut self, x: f32, y: f32, up: bool) {
        if up {
            let hit = self.volume_sheet.as_ref().and_then(|v| v.sheet.hit(&self.ui, self.w, self.h, x, y));
            match hit {
                Some(SheetHit::Cancel) => self.volume_close(None),
                Some(SheetHit::Join) => {
                    let typed = self.volume_sheet.as_ref().and_then(|v| v.sheet.form_result()).map(|(_, text)| text);
                    if let Some(password) = typed {
                        self.volume_close(Some(password));
                    }
                }
                _ => {}
            }
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Takes the sheet down. A passphrase goes to privd; a failure queues the volume again. `password` is `None` for a cancel.
    fn volume_close(&mut self, password: Option<String>) {
        let Some(v) = self.volume_sheet.take() else { return };
        if let Some(password) = password {
            if let Err(e) = crate::privd_client::connect().unlock_volume(&v.device, &password) {
                eprintln!("springboard: unlock {}: {}", v.device, e.message);
                requeue(&v.device);
            }
        }
        self.dirty = true;
        self.drawn = Drawn::Nothing;
    }

    /// Draws the sheet over a dimmed screen.
    pub(super) fn volume_draw(&mut self, s: &mut Surface, t: f64) {
        let (w, h) = (self.w, self.h);
        if let Some(v) = self.volume_sheet.as_mut() {
            s.fill_rrect(0.0, 0.0, w as f32, h as f32, 0.0, rgba(0, 0, 0, 120));
            v.sheet.draw(s, &self.ui, w, h, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The statics are shared by the test threads, so each test uses its own device names and only looks at those.
    fn queued(prefix: &str) -> usize {
        lock(&QUEUE).iter().filter(|d| d.starts_with(prefix)).count()
    }

    #[test]
    fn locked_volumes_are_queued_once_and_asked_ones_do_not_come_back() {
        let listing = "/dev/vsheettest1 part crypto_LUKS\n/dev/vsheettest2 part crypto_LUKS\n";
        queue_locked(listing);
        queue_locked(listing);
        assert_eq!(queued("/dev/vsheettest1"), 1);
        assert_eq!(queued("/dev/vsheettest2"), 1);
        // the sheet asked about vsheettest1 already
        lock(&QUEUE).retain(|d| d != "/dev/vsheettest1");
        lock(&ASKED).push("/dev/vsheettest1".to_string());
        queue_locked(listing);
        assert_eq!(queued("/dev/vsheettest1"), 0, "an asked volume does not come back");
        lock(&QUEUE).retain(|d| !d.starts_with("/dev/vsheettest"));
        lock(&ASKED).retain(|d| !d.starts_with("/dev/vsheettest"));
    }

    #[test]
    fn a_failed_unlock_is_asked_again_once() {
        lock(&ASKED).push("/dev/vsheettest3".to_string());
        requeue("/dev/vsheettest3");
        requeue("/dev/vsheettest3");
        assert_eq!(queued("/dev/vsheettest3"), 1);
        assert!(!lock(&ASKED).iter().any(|d| d == "/dev/vsheettest3"));
        lock(&QUEUE).retain(|d| !d.starts_with("/dev/vsheettest"));
    }

    #[test]
    fn the_prompt_names_the_volume_and_hides_the_passphrase() {
        let p = prompt_for("/dev/sda2");
        assert!(p.secure);
        assert!(p.message.contains("sda2"));
        assert_eq!(p.purpose, Purpose::VolumeUnlock { device: "/dev/sda2".into() });
    }
}
