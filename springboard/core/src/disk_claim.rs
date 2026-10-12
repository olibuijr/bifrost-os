//! Exclusive-use claims on a disk, one holder at a time. A claim names its owner and may carry a release gate. When
//! another owner asks for a disk that is already claimed, the holder's gate decides whether the disk changes hands:
//! `Ok` hands it over, `Err(reason)` refuses and the holder keeps it. A holder with no gate cannot be displaced. The
//! registry records claims only. It touches no device, runs no timer and holds no callbacks into the system; the
//! caller drops a claim with `disappeared` when its disk leaves the snapshot.

use std::collections::BTreeMap;

/// Decides whether the holder gives up its claim. `Ok(())` releases it, `Err(reason)` keeps it.
pub type ReleaseGate = Box<dyn Fn() -> Result<(), String>>;

struct Holder {
    owner: String,
    release: Option<ReleaseGate>,
}

/// Why a claim or an unclaim was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaimError {
    /// Another owner holds the disk and gave it no release gate, so it cannot be taken.
    Busy { owner: String },
    /// The holder's release gate refused the handover. The holder keeps the disk.
    Refused { owner: String, reason: String },
    /// The disk has no claim to release.
    NotClaimed,
    /// The disk is claimed by a different owner, who alone may release it.
    NotOwner { owner: String },
}

/// The claims on every disk the caller knows about, keyed by BSD device name.
#[derive(Default)]
pub struct Claims {
    held: BTreeMap<String, Holder>,
}

impl Claims {
    pub fn new() -> Self {
        Self::default()
    }

    /// Claims `bsd_name` for `owner`. A free disk is taken at once. A disk held by the same owner is claimed again
    /// with the new gate. A disk held by another owner is taken only if that holder's gate allows it.
    pub fn claim(&mut self, bsd_name: &str, owner: &str, release: Option<ReleaseGate>) -> Result<(), ClaimError> {
        match self.held.get(bsd_name) {
            None => {}
            Some(h) if h.owner == owner => {}
            Some(h) => match &h.release {
                None => return Err(ClaimError::Busy { owner: h.owner.clone() }),
                Some(gate) => gate().map_err(|reason| ClaimError::Refused { owner: h.owner.clone(), reason })?,
            },
        }
        self.held.insert(bsd_name.to_string(), Holder { owner: owner.to_string(), release });
        Ok(())
    }

    /// Gives up the claim on `bsd_name`. Only its owner may do so.
    pub fn unclaim(&mut self, bsd_name: &str, owner: &str) -> Result<(), ClaimError> {
        let holder = match self.held.get(bsd_name) {
            None => return Err(ClaimError::NotClaimed),
            Some(h) => h.owner.clone(),
        };
        if holder != owner {
            return Err(ClaimError::NotOwner { owner: holder });
        }
        self.held.remove(bsd_name);
        Ok(())
    }

    /// Whether any owner holds `bsd_name`.
    pub fn is_claimed(&self, bsd_name: &str) -> bool {
        self.held.contains_key(bsd_name)
    }

    /// The owner holding `bsd_name`, if any.
    pub fn owner_of(&self, bsd_name: &str) -> Option<&str> {
        self.held.get(bsd_name).map(|h| h.owner.as_str())
    }

    /// Drops the claim on a disk that has left. Returns whether there was one.
    pub fn disappeared(&mut self, bsd_name: &str) -> bool {
        self.held.remove(bsd_name).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(result: Result<(), &'static str>) -> Option<ReleaseGate> {
        let msg = result.map_err(String::from);
        Some(Box::new(move || msg.clone()))
    }

    #[test]
    fn a_free_disk_is_taken_by_the_first_claim() {
        let mut claims = Claims::new();
        assert_eq!(claims.claim("sdb", "backup", None), Ok(()));
        assert!(claims.is_claimed("sdb"));
        assert_eq!(claims.owner_of("sdb"), Some("backup"));
        assert!(!claims.is_claimed("sdc"));
    }

    #[test]
    fn a_holder_without_a_gate_cannot_be_displaced() {
        let mut claims = Claims::new();
        claims.claim("sdb", "backup", None).unwrap();
        assert_eq!(claims.claim("sdb", "indexer", None), Err(ClaimError::Busy { owner: "backup".into() }));
        assert_eq!(claims.owner_of("sdb"), Some("backup"));
    }

    #[test]
    fn a_gate_that_allows_the_handover_moves_the_claim() {
        let mut claims = Claims::new();
        claims.claim("sdb", "backup", gate(Ok(()))).unwrap();
        assert_eq!(claims.claim("sdb", "indexer", None), Ok(()));
        assert_eq!(claims.owner_of("sdb"), Some("indexer"));
    }

    #[test]
    fn a_refusing_gate_keeps_the_holder() {
        let mut claims = Claims::new();
        claims.claim("sdb", "backup", gate(Err("still writing"))).unwrap();
        assert_eq!(
            claims.claim("sdb", "indexer", None),
            Err(ClaimError::Refused { owner: "backup".into(), reason: "still writing".into() })
        );
        assert_eq!(claims.owner_of("sdb"), Some("backup"));
    }

    #[test]
    fn only_the_owner_can_unclaim() {
        let mut claims = Claims::new();
        claims.claim("sdb", "backup", None).unwrap();
        assert_eq!(claims.unclaim("sdb", "indexer"), Err(ClaimError::NotOwner { owner: "backup".into() }));
        assert!(claims.is_claimed("sdb"));
        assert_eq!(claims.unclaim("sdb", "backup"), Ok(()));
        assert!(!claims.is_claimed("sdb"));
    }

    #[test]
    fn unclaiming_a_free_disk_reports_not_claimed() {
        let mut claims = Claims::new();
        assert_eq!(claims.unclaim("sdb", "backup"), Err(ClaimError::NotClaimed));
    }

    #[test]
    fn a_disappeared_disk_loses_its_claim() {
        let mut claims = Claims::new();
        claims.claim("sdb", "backup", None).unwrap();
        assert!(claims.disappeared("sdb"));
        assert!(!claims.is_claimed("sdb"));
        assert!(!claims.disappeared("sdb"));
        assert_eq!(claims.claim("sdb", "indexer", None), Ok(()));
    }
}
