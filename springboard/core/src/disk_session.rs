//! A disk session: the object an app or the shell holds to hear about block devices. It keeps the last snapshot the
//! caller reported, turns each new snapshot into appeared, disappeared and description-changed events (`disk_events`),
//! and calls the callbacks registered for each kind in registration order. It also owns the approval chain and the
//! claims that gate mount, unmount and eject requests (`disk_commands`).
//!
//! The session reads no hardware and runs no thread or timer. The caller feeds it a snapshot whenever the device list
//! may have changed (for example from a udev monitor), and the callbacks run on the caller's thread, inside `observe`.
//! Differences from the native framework: there is no run loop or dispatch queue to schedule on, and no peek callback.
//! A request returns its plan as commands (`disk_commands::Command`); nothing is run here.

use crate::disk_claims::{ClaimConflict, Claims};
use crate::disk_commands::{self, Command, Refusal};
use crate::disk_events::{self, Approvals, Disk, DiskEvent};
use std::collections::BTreeMap;

type AppearedCallback = Box<dyn FnMut(&Disk)>;
type DisappearedCallback = Box<dyn FnMut(&str)>;
type ChangedCallback = Box<dyn FnMut(&Disk, &Disk)>;

#[derive(Default)]
pub struct Session {
    disks: Vec<Disk>,
    appeared: Vec<AppearedCallback>,
    disappeared: Vec<DisappearedCallback>,
    changed: Vec<ChangedCallback>,
    approvals: Approvals,
    claims: Claims,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called for each disk that arrives in a snapshot it was not in before.
    pub fn on_appeared(&mut self, callback: impl FnMut(&Disk) + 'static) {
        self.appeared.push(Box::new(callback));
    }

    /// Called with the BSD name of each disk that leaves the snapshot.
    pub fn on_disappeared(&mut self, callback: impl FnMut(&str) + 'static) {
        self.disappeared.push(Box::new(callback));
    }

    /// Called with the old and new state when a disk stays but any of its fields change.
    pub fn on_description_changed(&mut self, callback: impl FnMut(&Disk, &Disk) + 'static) {
        self.changed.push(Box::new(callback));
    }

    /// The approval chain for mount, unmount and eject requests. Register gates here.
    pub fn approvals_mut(&mut self) -> &mut Approvals {
        &mut self.approvals
    }

    /// The disks as of the last snapshot, in the order the caller reported them.
    pub fn disks(&self) -> &[Disk] {
        &self.disks
    }

    /// The disk with this BSD name in the last snapshot.
    pub fn disk(&self, bsd_name: &str) -> Option<&Disk> {
        disk_events::find_by_bsd_name(&self.disks, bsd_name)
    }

    /// The disk whose mounted volume sits at `path` in the last snapshot.
    pub fn disk_at_path(&self, path: &str) -> Option<&Disk> {
        disk_events::find_by_volume_path(&self.disks, path)
    }

    /// The description of a disk in the last snapshot, as `disk_events::description` defines it.
    pub fn description(&self, bsd_name: &str) -> Option<BTreeMap<&'static str, String>> {
        self.disk(bsd_name).map(disk_events::description)
    }

    /// Stores `snapshot` as the current state, sends the events between the old and new state to the callbacks, and
    /// returns those events. Callbacks see the new state through `disks()` only after `observe` returns, so they
    /// receive the event values themselves.
    pub fn observe(&mut self, snapshot: Vec<Disk>) -> Vec<DiskEvent> {
        let events = disk_events::diff(&self.disks, &snapshot);
        self.disks = snapshot;
        for event in &events {
            match event {
                DiskEvent::Appeared(disk) => {
                    for callback in self.appeared.iter_mut() {
                        callback(disk);
                    }
                }
                DiskEvent::Disappeared(name) => {
                    for callback in self.disappeared.iter_mut() {
                        callback(name.as_str());
                    }
                }
                DiskEvent::DescriptionChanged { before, after } => {
                    for callback in self.changed.iter_mut() {
                        callback(before, after);
                    }
                }
            }
        }
        events
    }

    /// Claims a disk for `owner`. Other owners are refused until the claim is released.
    pub fn claim(&mut self, bsd_name: &str, owner: &str) -> Result<(), Refusal> {
        self.require(bsd_name)?;
        self.claims.claim(bsd_name, owner).map_err(claim_refusal)
    }

    /// Releases the claim `owner` holds on a disk.
    pub fn unclaim(&mut self, bsd_name: &str, owner: &str) -> Result<(), Refusal> {
        self.claims.unclaim(bsd_name, owner).map_err(claim_refusal)
    }

    /// The owners that hold a claim.
    pub fn claim_owners(&self) -> Vec<String> {
        self.claims.owners()
    }

    /// Drops every claim `owner` holds: the owner quit without releasing them.
    pub fn release_owner(&mut self, owner: &str) -> usize {
        self.claims.release_owner(owner)
    }

    /// Plans an unmount of the named disk. Fails with `NoDisk` when the last snapshot does not have it.
    pub fn unmount(&self, bsd_name: &str, owner: &str) -> Result<Vec<Command>, Refusal> {
        let disk = self.require(bsd_name)?;
        disk_commands::unmount(disk, &self.approvals, &self.claims, owner)
    }

    /// Plans an eject of the named disk. Fails with `NoDisk` when the last snapshot does not have it.
    pub fn eject(&self, bsd_name: &str, owner: &str) -> Result<Vec<Command>, Refusal> {
        let disk = self.require(bsd_name)?;
        disk_commands::eject(disk, &self.approvals, &self.claims, owner)
    }

    fn require(&self, bsd_name: &str) -> Result<&Disk, Refusal> {
        self.disk(bsd_name).ok_or_else(|| Refusal::NoDisk(bsd_name.to_string()))
    }
}

fn claim_refusal(conflict: ClaimConflict) -> Refusal {
    Refusal::Claimed { device: conflict.device, owner: conflict.owner }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn disk(name: &str, mount: Option<&str>) -> Disk {
        Disk {
            bsd_name: name.to_string(),
            size: 1024,
            removable: true,
            read_only: false,
            model: "Stick".to_string(),
            mount_point: mount.map(str::to_string),
        }
    }

    fn recording() -> (Session, Rc<RefCell<Vec<String>>>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut session = Session::new();
        let a = log.clone();
        session.on_appeared(move |d| a.borrow_mut().push(format!("+{}", d.bsd_name)));
        let r = log.clone();
        session.on_disappeared(move |n| r.borrow_mut().push(format!("-{n}")));
        let c = log.clone();
        session.on_description_changed(move |b, a| {
            c.borrow_mut().push(format!("~{}:{}", b.bsd_name, a.mount_point.clone().unwrap_or_default()))
        });
        (session, log)
    }

    #[test]
    fn observe_dispatches_each_kind_in_event_order() {
        let (mut s, log) = recording();
        s.observe(vec![disk("disk2", None), disk("disk1", None)]);
        assert_eq!(*log.borrow(), vec!["+disk1", "+disk2"]);
        log.borrow_mut().clear();
        let events = s.observe(vec![disk("disk1", Some("/run/a")), disk("disk3", None)]);
        assert_eq!(events.len(), 3);
        assert_eq!(*log.borrow(), vec!["-disk2", "+disk3", "~disk1:/run/a"]);
    }

    #[test]
    fn unchanged_snapshot_sends_nothing() {
        let (mut s, log) = recording();
        s.observe(vec![disk("disk1", None)]);
        log.borrow_mut().clear();
        assert!(s.observe(vec![disk("disk1", None)]).is_empty());
        assert!(log.borrow().is_empty());
        assert_eq!(s.disks().len(), 1);
    }

    #[test]
    fn lookups_use_the_last_snapshot() {
        let mut s = Session::new();
        s.observe(vec![disk("disk1", Some("/run/usb/"))]);
        assert!(s.disk("disk1").is_some());
        assert!(s.disk("disk9").is_none());
        assert!(s.disk_at_path("/run/usb").is_some());
        let d = s.description("disk1").expect("description");
        assert_eq!(d.get("volume_path").map(String::as_str), Some("/run/usb/"));
        assert_eq!(d.get("device_model").map(String::as_str), Some("Stick"));
    }

    #[test]
    fn unknown_disk_is_refused() {
        let mut s = Session::new();
        assert_eq!(s.unmount("disk9", "files"), Err(Refusal::NoDisk("disk9".to_string())));
        assert_eq!(s.eject("disk9", "files"), Err(Refusal::NoDisk("disk9".to_string())));
        assert_eq!(s.claim("disk9", "files"), Err(Refusal::NoDisk("disk9".to_string())));
    }

    #[test]
    fn claim_blocks_other_owners_until_released() {
        let mut s = Session::new();
        s.observe(vec![disk("disk1", Some("/run/a"))]);
        assert_eq!(s.claim("disk1", "installer"), Ok(()));
        assert_eq!(
            s.unmount("disk1", "files"),
            Err(Refusal::Claimed { device: "disk1".to_string(), owner: "installer".to_string() })
        );
        assert_eq!(
            s.unclaim("disk1", "files"),
            Err(Refusal::Claimed { device: "disk1".to_string(), owner: "installer".to_string() })
        );
        assert!(s.unmount("disk1", "installer").is_ok());
        assert_eq!(s.unclaim("disk1", "installer"), Ok(()));
    }

    #[test]
    fn approval_gate_vetoes_unmount() {
        let mut s = Session::new();
        s.observe(vec![disk("disk1", Some("/run/a"))]);
        s.approvals_mut().register(|op, _| match op {
            disk_events::Operation::Unmount => Err("busy".to_string()),
            _ => Ok(()),
        });
        assert_eq!(s.unmount("disk1", "files"), Err(Refusal::Vetoed("busy".to_string())));
        assert!(s.eject("disk1", "files").is_err());
    }
}
