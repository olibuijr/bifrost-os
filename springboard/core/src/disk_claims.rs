//! Exclusive claims on block devices. An owner claims a device by its name; while the claim stands, other owners
//! cannot claim it, unclaim it, or run mount, unmount, eject and rename requests on it (see `disk_commands`). The
//! claims are a registry only: they take no lock on the hardware and expire only when their owner releases them.

use std::collections::BTreeMap;

/// Why a claim, release or request was refused: another owner holds the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimConflict {
    pub device: String,
    pub owner: String,
}

/// The claims held on devices, keyed by device name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Claims {
    by_device: BTreeMap<String, String>,
}

impl Claims {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claims the device for `owner`. A repeated claim by the same owner succeeds and changes nothing. A device held
    /// by another owner is refused, and the existing owner is reported.
    pub fn claim(&mut self, device: &str, owner: &str) -> Result<(), ClaimConflict> {
        self.check(device, owner)?;
        self.by_device.insert(device.to_string(), owner.to_string());
        Ok(())
    }

    /// Releases the device. Releasing a device nobody holds succeeds. Only the holder can release a claim.
    pub fn unclaim(&mut self, device: &str, owner: &str) -> Result<(), ClaimConflict> {
        self.check(device, owner)?;
        self.by_device.remove(device);
        Ok(())
    }

    /// Whether any owner holds the device.
    pub fn is_claimed(&self, device: &str) -> bool {
        self.by_device.contains_key(device)
    }

    /// The owner holding the device, if any.
    pub fn owner(&self, device: &str) -> Option<&str> {
        self.by_device.get(device).map(String::as_str)
    }

    /// Allows `requester` to act on the device: an unclaimed device, or one the requester holds. Any other holder is
    /// reported as a conflict.
    pub fn check(&self, device: &str, requester: &str) -> Result<(), ClaimConflict> {
        match self.by_device.get(device) {
            Some(holder) if holder != requester => {
                Err(ClaimConflict { device: device.to_string(), owner: holder.clone() })
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_free_device_can_be_claimed_and_is_then_reported_claimed() {
        let mut claims = Claims::new();
        assert!(!claims.is_claimed("sdb"));
        assert_eq!(claims.claim("sdb", "installer"), Ok(()));
        assert!(claims.is_claimed("sdb"));
        assert_eq!(claims.owner("sdb"), Some("installer"));
    }

    #[test]
    fn a_second_owner_is_refused_and_the_first_keeps_the_device() {
        let mut claims = Claims::new();
        claims.claim("sdb", "installer").unwrap();
        let err = claims.claim("sdb", "files").unwrap_err();
        assert_eq!(err, ClaimConflict { device: "sdb".into(), owner: "installer".into() });
        assert_eq!(claims.owner("sdb"), Some("installer"));
    }

    #[test]
    fn the_same_owner_can_claim_again_without_a_conflict() {
        let mut claims = Claims::new();
        claims.claim("sdb", "installer").unwrap();
        assert_eq!(claims.claim("sdb", "installer"), Ok(()));
        assert_eq!(claims.owner("sdb"), Some("installer"));
    }

    #[test]
    fn only_the_holder_can_unclaim_and_releasing_a_free_device_succeeds() {
        let mut claims = Claims::new();
        claims.claim("sdb", "installer").unwrap();
        assert!(claims.unclaim("sdb", "files").is_err());
        assert!(claims.is_claimed("sdb"));
        assert_eq!(claims.unclaim("sdb", "installer"), Ok(()));
        assert!(!claims.is_claimed("sdb"));
        assert_eq!(claims.unclaim("sdc", "files"), Ok(()));
    }

    #[test]
    fn check_allows_unclaimed_devices_and_the_holder_only() {
        let mut claims = Claims::new();
        assert_eq!(claims.check("sdb", "files"), Ok(()));
        claims.claim("sdb", "installer").unwrap();
        assert_eq!(claims.check("sdb", "installer"), Ok(()));
        assert_eq!(claims.check("sdb", "files").unwrap_err().owner, "installer");
        assert_eq!(claims.check("sdc", "files"), Ok(()));
    }

    #[test]
    fn claims_on_different_devices_do_not_interfere() {
        let mut claims = Claims::new();
        claims.claim("sda", "installer").unwrap();
        assert_eq!(claims.claim("sdb", "files"), Ok(()));
        assert_eq!(claims.owner("sda"), Some("installer"));
        assert_eq!(claims.owner("sdb"), Some("files"));
    }
}
