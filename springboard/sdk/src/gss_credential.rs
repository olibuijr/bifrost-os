//! GSS-API credential management (`gss_acquire_cred`, `gss_inquire_cred`, `GSSCredentialGetLifetime`,
//! `gss_release_cred`, `gss_destroy_cred`): the credential an application holds to start or accept a security
//! context, with its name, its usage and its remaining lifetime in whole seconds.
//!
//! The usage values follow RFC 2744 section 4.5: `GSS_C_BOTH` is 0, `GSS_C_INITIATE` is 1 and `GSS_C_ACCEPT` is 2.
//! `GSS_C_INDEFINITE` is the all-ones 32-bit value, which this model stores as a credential with no expiry time.
//! The reference page names these macros and does not print them, so the values follow the RFC as `gss_status` does.
//!
//! The lifetime is kept as an absolute expiry time in seconds since the epoch, so the remaining lifetime is a
//! subtraction against the caller's clock and never drifts. `Credential::new` records a credential the caller is
//! entitled to hold; it checks no password or certificate, because no mechanism exists here to verify one. Release
//! and destroy calls are replaced by drop.

use crate::gss_name::Name;
use crate::gss_status::RoutineError;
use std::fmt;

/// `GSS_C_INDEFINITE`: a lifetime with no limit.
pub const INDEFINITE: u32 = u32::MAX;

/// `gss_cred_usage_t`: what a credential may be used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Usage {
    /// `GSS_C_BOTH`: initiate and accept.
    Both,
    /// `GSS_C_INITIATE`: start a security context.
    Initiate,
    /// `GSS_C_ACCEPT`: accept a security context.
    Accept,
}

impl Usage {
    /// The numeric value the GSS-API passes for this usage.
    pub fn value(self) -> u32 {
        match self {
            Usage::Both => 0,
            Usage::Initiate => 1,
            Usage::Accept => 2,
        }
    }

    /// The usage for a numeric value, or `None` for a value the RFC does not define.
    pub fn from_value(value: u32) -> Option<Usage> {
        match value {
            0 => Some(Usage::Both),
            1 => Some(Usage::Initiate),
            2 => Some(Usage::Accept),
            _ => None,
        }
    }

    /// True when a credential with this usage may start a security context.
    pub fn can_initiate(self) -> bool {
        self != Usage::Accept
    }

    /// True when a credential with this usage may accept a security context.
    pub fn can_accept(self) -> bool {
        self != Usage::Initiate
    }

    /// True when this usage covers what `wanted` needs.
    pub fn allows(self, wanted: Usage) -> bool {
        match wanted {
            Usage::Both => self == Usage::Both,
            Usage::Initiate => self.can_initiate(),
            Usage::Accept => self.can_accept(),
        }
    }
}

/// Why a credential cannot be used for an operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialError {
    /// The lifetime has run out.
    Expired,
    /// The credential's usage does not cover the operation.
    WrongUsage,
}

impl CredentialError {
    /// The GSS routine error this failure reports.
    pub fn routine_error(self) -> RoutineError {
        match self {
            CredentialError::Expired => RoutineError::CredentialsExpired,
            CredentialError::WrongUsage => RoutineError::NoCred,
        }
    }
}

impl fmt::Display for CredentialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CredentialError::Expired => "credential has expired",
            CredentialError::WrongUsage => "credential usage does not allow this operation",
        })
    }
}

impl std::error::Error for CredentialError {}

/// A credential: the principal it speaks for, its usage, and when it expires.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credential {
    name: Option<Name>,
    usage: Usage,
    /// Expiry in seconds since the epoch. `None` means the lifetime is indefinite.
    expires_at: Option<u64>,
}

impl Credential {
    /// Records a credential that lasts `lifetime` seconds from `now`. `INDEFINITE` never expires. A lifetime of 0
    /// is already expired. The expiry saturates at `u64::MAX` rather than wrapping.
    pub fn new(name: Option<Name>, usage: Usage, lifetime: u32, now: u64) -> Credential {
        let expires_at = if lifetime == INDEFINITE {
            None
        } else {
            Some(now.saturating_add(u64::from(lifetime)))
        };
        Credential { name, usage, expires_at }
    }

    /// The principal this credential speaks for, when one was given.
    pub fn name(&self) -> Option<&Name> {
        self.name.as_ref()
    }

    /// The usage this credential was acquired for.
    pub fn usage(&self) -> Usage {
        self.usage
    }

    /// `GSSCredentialGetLifetime`: whole seconds left at `now`, 0 once expired, `INDEFINITE` when unlimited.
    pub fn lifetime(&self, now: u64) -> u32 {
        match self.expires_at {
            None => INDEFINITE,
            Some(expiry) => {
                let remaining = expiry.saturating_sub(now);
                // Clamp to the 32-bit field; INDEFINITE is reserved, so the largest finite answer is one less.
                remaining.min(u64::from(INDEFINITE - 1)) as u32
            }
        }
    }

    /// True when the lifetime has run out at `now`. An indefinite credential never expires.
    pub fn is_expired(&self, now: u64) -> bool {
        matches!(self.expires_at, Some(expiry) if now >= expiry)
    }

    /// Checks that the credential may be used for `wanted` at `now`. An expired credential is refused first.
    pub fn usable_for(&self, wanted: Usage, now: u64) -> Result<(), CredentialError> {
        if self.is_expired(now) {
            return Err(CredentialError::Expired);
        }
        if !self.usage.allows(wanted) {
            return Err(CredentialError::WrongUsage);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gss_name::NameType;

    fn ftp_name() -> Name {
        Name::import(b"ftp@files.example.org", NameType::HostBasedService).unwrap()
    }

    #[test]
    fn usage_values_follow_the_rfc() {
        assert_eq!(Usage::Both.value(), 0);
        assert_eq!(Usage::Initiate.value(), 1);
        assert_eq!(Usage::Accept.value(), 2);
        assert_eq!(Usage::from_value(3), None);
        assert_eq!(Usage::from_value(1), Some(Usage::Initiate));
    }

    #[test]
    fn a_finite_lifetime_counts_down_from_acquisition() {
        // Acquired at t = 1000 with 3600 seconds: expires at 4600.
        let c = Credential::new(None, Usage::Both, 3600, 1000);
        assert_eq!(c.lifetime(1000), 3600);
        assert_eq!(c.lifetime(1600), 3000);
        assert!(!c.is_expired(4599));
        assert_eq!(c.lifetime(4600), 0);
        assert!(c.is_expired(4600));
        assert_eq!(c.lifetime(5000), 0);
    }

    #[test]
    fn an_indefinite_credential_never_expires() {
        let c = Credential::new(None, Usage::Accept, INDEFINITE, 500);
        assert_eq!(c.lifetime(500), INDEFINITE);
        assert_eq!(c.lifetime(u64::MAX), INDEFINITE);
        assert!(!c.is_expired(u64::MAX));
    }

    #[test]
    fn a_zero_lifetime_is_expired_at_once() {
        let c = Credential::new(None, Usage::Both, 0, 77);
        assert!(c.is_expired(77));
        assert_eq!(c.usable_for(Usage::Initiate, 77), Err(CredentialError::Expired));
        assert_eq!(CredentialError::Expired.routine_error(), RoutineError::CredentialsExpired);
    }

    #[test]
    fn an_initiate_credential_cannot_accept_and_the_error_maps_to_no_cred() {
        let c = Credential::new(Some(ftp_name()), Usage::Initiate, 60, 0);
        assert!(c.usable_for(Usage::Initiate, 0).is_ok());
        assert_eq!(c.usable_for(Usage::Accept, 0), Err(CredentialError::WrongUsage));
        assert_eq!(c.usable_for(Usage::Both, 0), Err(CredentialError::WrongUsage));
        assert_eq!(CredentialError::WrongUsage.routine_error(), RoutineError::NoCred);
    }

    #[test]
    fn a_both_credential_covers_every_use() {
        let c = Credential::new(None, Usage::Both, 60, 0);
        assert!(c.usable_for(Usage::Initiate, 0).is_ok());
        assert!(c.usable_for(Usage::Accept, 0).is_ok());
        assert!(c.usable_for(Usage::Both, 0).is_ok());
        assert!(Usage::Initiate.allows(Usage::Initiate));
        assert!(!Usage::Initiate.allows(Usage::Both));
    }

    #[test]
    fn the_name_is_kept_with_the_credential() {
        let c = Credential::new(Some(ftp_name()), Usage::Initiate, 60, 0);
        assert_eq!(c.name().unwrap().display(), "ftp@files.example.org");
        assert_eq!(c.usage(), Usage::Initiate);
        assert!(Credential::new(None, Usage::Both, 1, 0).name().is_none());
    }
}
