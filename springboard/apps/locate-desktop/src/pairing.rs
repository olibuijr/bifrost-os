//! Device approval. A new device asks to join the account by posting its public key as pending; it polls for its sealed
//! account key and holds nothing until another device approves it. An approving device lists the pending requests in a
//! confirmation sheet and seals the account key only after the person confirms one. Nothing is approved automatically,
//! and a denied request is removed on the server by the caller.

use crate::sealing;
use bifrost_app::cloud_findmy::PendingDevice;

/// The answer to one poll of the sealed-key route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PollAnswer {
    /// The approving device sealed the account key; the value is the base64 blob.
    Sealed(String),
    /// Still waiting for an approval.
    StillWaiting,
    /// The request was denied or deleted.
    Gone,
}

/// The state of a device that asks to join the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Enrollment {
    /// The device has not asked to join.
    Idle,
    /// The device is pending under `pending_id` and polls for its sealed key.
    Waiting { pending_id: String },
    /// The account key is held; the device is on the account.
    Enrolled,
    /// The request was denied. The person can ask again.
    Denied,
}

impl Enrollment {
    /// Starts a join request under the id the cloud gave. Only an idle or denied device can start one.
    pub fn start(&mut self, pending_id: &str) -> bool {
        match self {
            Enrollment::Idle | Enrollment::Denied if !pending_id.trim().is_empty() => {
                *self = Enrollment::Waiting { pending_id: pending_id.trim().to_string() };
                true
            }
            _ => false,
        }
    }

    /// Applies one poll. A sealed answer moves the device to `Enrolled` and returns the blob for `open_account_key`.
    /// A gone answer moves a waiting device to `Denied`. Other states ignore the poll.
    pub fn on_poll(&mut self, answer: PollAnswer) -> Option<String> {
        if !matches!(self, Enrollment::Waiting { .. }) {
            return None;
        }
        match answer {
            PollAnswer::Sealed(blob) => {
                *self = Enrollment::Enrolled;
                Some(blob)
            }
            PollAnswer::StillWaiting => None,
            PollAnswer::Gone => {
                *self = Enrollment::Denied;
                None
            }
        }
    }

    /// The pending id a waiting device polls under.
    pub fn pending_id(&self) -> Option<&str> {
        match self {
            Enrollment::Waiting { pending_id } => Some(pending_id),
            _ => None,
        }
    }

    /// The line the Devices page shows while the device waits.
    pub fn waiting_line(&self) -> Option<&'static str> {
        matches!(self, Enrollment::Waiting { .. }).then_some("Approve this computer on another of your devices")
    }
}

/// The join requests an approving device has been shown. They come from the server list; confirming or denying one
/// removes it here, and the caller seals or deletes on the server.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Approvals {
    pending: Vec<PendingDevice>,
}

impl Approvals {
    /// Replaces the list with the server's, keeping the first entry of each id.
    pub fn sync(&mut self, list: Vec<PendingDevice>) {
        let mut kept: Vec<PendingDevice> = Vec::new();
        for p in list {
            if !kept.iter().any(|k| k.id == p.id) {
                kept.push(p);
            }
        }
        self.pending = kept;
    }

    pub fn pending(&self) -> &[PendingDevice] {
        &self.pending
    }

    /// The confirmation sheet shows while a request waits.
    pub fn sheet_open(&self) -> bool {
        !self.pending.is_empty()
    }

    /// The device names the sheet lists, in the order the server gave them.
    pub fn names(&self) -> Vec<String> {
        self.pending.iter().map(|p| p.name.clone()).collect()
    }

    /// The person confirmed `id`: the request leaves the list and is returned so its key can be sealed to it.
    pub fn confirm(&mut self, id: &str) -> Option<PendingDevice> {
        self.take(id)
    }

    /// The person denied `id`: the request leaves the list and the caller deletes it on the server.
    pub fn deny(&mut self, id: &str) -> Option<PendingDevice> {
        self.take(id)
    }

    fn take(&mut self, id: &str) -> Option<PendingDevice> {
        let at = self.pending.iter().position(|p| p.id == id)?;
        Some(self.pending.remove(at))
    }
}

/// The approving device's seal for a confirmed request: the account private key sealed to the pending device.
pub fn seal_for(pending: &PendingDevice, account_private: &[u8], ephemeral: &[u8]) -> Result<sealing::Sealed, sealing::SealError> {
    let device_public = bifrost_core::plist::base64_decode(&pending.device_public_key).ok_or(sealing::SealError::Payload)?;
    sealing::seal_account_key(&device_public, &pending.id, account_private, ephemeral)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(id: &str, name: &str) -> PendingDevice {
        PendingDevice { id: id.into(), name: name.into(), device_public_key: String::new() }
    }

    #[test]
    fn a_device_waits_until_its_key_is_sealed_then_is_enrolled() {
        let mut e = Enrollment::Idle;
        assert!(e.start("p1"));
        assert_eq!(e.waiting_line(), Some("Approve this computer on another of your devices"));
        assert_eq!(e.on_poll(PollAnswer::StillWaiting), None);
        assert_eq!(e.pending_id(), Some("p1"));
        assert_eq!(e.on_poll(PollAnswer::Sealed("blob".into())), Some("blob".into()));
        assert_eq!(e, Enrollment::Enrolled);
        assert_eq!(e.waiting_line(), None);
    }

    #[test]
    fn a_deleted_request_is_denied_and_can_be_started_again() {
        let mut e = Enrollment::Idle;
        e.start("p1");
        assert_eq!(e.on_poll(PollAnswer::Gone), None);
        assert_eq!(e, Enrollment::Denied);
        assert!(e.start("p2"), "a denied device may ask again");
        assert_eq!(e.pending_id(), Some("p2"));
    }

    #[test]
    fn an_enrolled_device_does_not_restart_or_wait_again() {
        let mut e = Enrollment::Enrolled;
        assert!(!e.start("p3"));
        assert_eq!(e.on_poll(PollAnswer::Gone), None);
        assert_eq!(e, Enrollment::Enrolled);
        assert!(!Enrollment::Idle.start("  "), "an empty id is refused");
    }

    #[test]
    fn approvals_list_pending_devices_and_nothing_is_confirmed_by_itself() {
        let mut a = Approvals::default();
        assert!(!a.sheet_open());
        a.sync(vec![pending("p1", "Tab"), pending("p2", "Midget"), pending("p1", "Tab again")]);
        assert!(a.sheet_open());
        assert_eq!(a.names(), vec!["Tab".to_string(), "Midget".to_string()], "duplicate ids are dropped");
        assert_eq!(a.pending().len(), 2, "syncing alone approves nothing");
    }

    #[test]
    fn confirming_or_denying_removes_only_that_request() {
        let mut a = Approvals::default();
        a.sync(vec![pending("p1", "Tab"), pending("p2", "Midget")]);
        assert_eq!(a.confirm("p1").map(|p| p.name), Some("Tab".to_string()));
        assert_eq!(a.deny("p2").map(|p| p.name), Some("Midget".to_string()));
        assert_eq!(a.deny("p2"), None, "a request is taken once");
        assert!(!a.sheet_open());
    }

    #[test]
    fn a_sealed_key_reaches_only_the_pending_device() {
        let device_private = [0x44u8; 32];
        let device_public = bifrost_app::controls::x25519::PrivateKey::from_raw(&device_private).unwrap().public_key().raw();
        let p = PendingDevice {
            id: "p1".into(),
            name: "Tab".into(),
            device_public_key: bifrost_core::plist::base64_encode(&device_public),
        };
        let account_private = [0x11u8; 32];
        let blob = seal_for(&p, &account_private, &[0x55; 32]).unwrap();
        let opened = sealing::open_account_key(&device_private, "p1", &blob).unwrap();
        assert_eq!(opened.raw(), account_private);
        let broken = PendingDevice { device_public_key: "!!".into(), ..p };
        assert!(seal_for(&broken, &account_private, &[0x55; 32]).is_err());
    }
}
