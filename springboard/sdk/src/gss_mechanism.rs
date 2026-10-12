//! GSS-API security mechanisms (`gss_indicate_mechs`, `gss_indicate_mechs_by_attrs`, `gss_inquire_attrs_for_mech`,
//! `gss_display_mech_attr`, `gss_inquire_mech_for_saslname`, `gss_inquire_saslname_for_mech`): the registry that maps a
//! mechanism identifier to its SASL name and attributes, so that an application can list the mechanisms installed and
//! pick one that meets its needs.
//!
//! The registry starts empty. BifrOSt installs no GSS mechanism, so `indicate_mechs` returns an empty set until a
//! caller registers one, and nothing in this module authenticates anyone. Attributes are names (`GSS_C_MA_*`), not
//! attribute object identifiers, because the reference page does not print those values. `display` returns the name.
//!
//! Not here: loading a mechanism's library, and the release calls, which drop replaces.

use crate::gss_oid::{Oid, OidSet};
use crate::gss_status::RoutineError;

/// A mechanism attribute, named as the GSS-API mechanism attribute registry names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Attr {
    /// `GSS_C_MA_AUTH_INIT`: the mechanism authenticates the initiator.
    AuthInit,
    /// `GSS_C_MA_AUTH_TARG`: the mechanism authenticates the target.
    AuthTarg,
    /// `GSS_C_MA_DELEG_CRED`: the mechanism can delegate the initiator's credentials.
    DelegCred,
    /// `GSS_C_MA_INTEG_PROT`: the mechanism provides integrity protection.
    IntegProt,
    /// `GSS_C_MA_CONF_PROT`: the mechanism provides confidentiality protection.
    ConfProt,
    /// `GSS_C_MA_REPLAY_DET`: the mechanism detects replayed messages.
    ReplayDet,
    /// `GSS_C_MA_OOS_DET`: the mechanism detects out-of-sequence messages.
    OosDet,
    /// `GSS_C_MA_MECH_CONCRETE`: a concrete mechanism rather than a pseudo-mechanism.
    MechConcrete,
    /// `GSS_C_MA_DEPRECATED`: the mechanism is deprecated and should not be chosen.
    Deprecated,
}

impl Attr {
    /// Every attribute in this model.
    pub const ALL: [Attr; 9] = [
        Attr::AuthInit,
        Attr::AuthTarg,
        Attr::DelegCred,
        Attr::IntegProt,
        Attr::ConfProt,
        Attr::ReplayDet,
        Attr::OosDet,
        Attr::MechConcrete,
        Attr::Deprecated,
    ];

    /// The `GSS_C_MA_*` name of the attribute (`gss_display_mech_attr`).
    pub fn name(self) -> &'static str {
        match self {
            Attr::AuthInit => "GSS_C_MA_AUTH_INIT",
            Attr::AuthTarg => "GSS_C_MA_AUTH_TARG",
            Attr::DelegCred => "GSS_C_MA_DELEG_CRED",
            Attr::IntegProt => "GSS_C_MA_INTEG_PROT",
            Attr::ConfProt => "GSS_C_MA_CONF_PROT",
            Attr::ReplayDet => "GSS_C_MA_REPLAY_DET",
            Attr::OosDet => "GSS_C_MA_OOS_DET",
            Attr::MechConcrete => "GSS_C_MA_MECH_CONCRETE",
            Attr::Deprecated => "GSS_C_MA_DEPRECATED",
        }
    }

    /// The attribute with this name. An unknown name is `GSS_S_BAD_MECH_ATTR`.
    pub fn from_name(name: &str) -> Result<Attr, RoutineError> {
        Attr::ALL.iter().copied().find(|a| a.name() == name).ok_or(RoutineError::BadMechAttr)
    }
}

/// One installed mechanism: its identifier, its SASL name if it has one, and its attributes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mechanism {
    oid: Oid,
    sasl_name: Option<String>,
    attrs: Vec<Attr>,
}

impl Mechanism {
    /// A mechanism record. The attributes are kept in the order given.
    pub fn new(oid: Oid, sasl_name: Option<&str>, attrs: &[Attr]) -> Mechanism {
        Mechanism { oid, sasl_name: sasl_name.map(str::to_string), attrs: attrs.to_vec() }
    }

    /// The mechanism identifier.
    pub fn oid(&self) -> &Oid {
        &self.oid
    }

    /// The SASL protocol name, or `None` when the mechanism has none.
    pub fn sasl_name(&self) -> Option<&str> {
        self.sasl_name.as_deref()
    }

    /// The attributes, in the order they were given.
    pub fn attrs(&self) -> &[Attr] {
        &self.attrs
    }

    /// True when the mechanism has every attribute in `wanted`.
    fn has_all(&self, wanted: &[Attr]) -> bool {
        wanted.iter().all(|a| self.attrs.contains(a))
    }

    /// True when the mechanism has at least one attribute in `unwanted`.
    fn has_any(&self, unwanted: &[Attr]) -> bool {
        unwanted.iter().any(|a| self.attrs.contains(a))
    }
}

/// The mechanisms installed on this system, in the order they were registered.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registry {
    mechs: Vec<Mechanism>,
}

impl Registry {
    /// An empty registry: no mechanism is installed.
    pub fn new() -> Registry {
        Registry::default()
    }

    /// Adds a mechanism. An identifier or SASL name that is already registered is `GSS_S_DUPLICATE_ELEMENT`, and the
    /// registry is left unchanged.
    pub fn register(&mut self, mech: Mechanism) -> Result<(), RoutineError> {
        let clash = self.mechs.iter().any(|m| {
            m.oid == mech.oid || (mech.sasl_name.is_some() && m.sasl_name == mech.sasl_name)
        });
        if clash {
            return Err(RoutineError::DuplicateElement);
        }
        self.mechs.push(mech);
        Ok(())
    }

    /// The record for an identifier. An identifier that is not registered is `GSS_S_BAD_MECH`.
    pub fn find(&self, oid: &Oid) -> Result<&Mechanism, RoutineError> {
        self.mechs.iter().find(|m| &m.oid == oid).ok_or(RoutineError::BadMech)
    }

    /// `gss_indicate_mechs`: every registered mechanism identifier, in registration order.
    pub fn indicate_mechs(&self) -> OidSet {
        let mut set = OidSet::new();
        for m in &self.mechs {
            set.add(m.oid.clone());
        }
        set
    }

    /// `gss_indicate_mechs_by_attrs`: the mechanisms that have every `desired` attribute and none of the `except`
    /// attributes. Empty lists select every mechanism.
    pub fn indicate_by_attrs(&self, desired: &[Attr], except: &[Attr]) -> OidSet {
        let mut set = OidSet::new();
        for m in self.mechs.iter().filter(|m| m.has_all(desired) && !m.has_any(except)) {
            set.add(m.oid.clone());
        }
        set
    }

    /// `gss_inquire_attrs_for_mech`: the attributes of a registered mechanism.
    pub fn attrs_for_mech(&self, oid: &Oid) -> Result<&[Attr], RoutineError> {
        Ok(self.find(oid)?.attrs())
    }

    /// `gss_inquire_mech_for_saslname`: the mechanism whose SASL name equals `name` exactly. No match is
    /// `GSS_S_BAD_MECH`.
    pub fn mech_for_saslname(&self, name: &str) -> Result<Oid, RoutineError> {
        self.mechs
            .iter()
            .find(|m| m.sasl_name.as_deref() == Some(name))
            .map(|m| m.oid.clone())
            .ok_or(RoutineError::BadMech)
    }

    /// `gss_inquire_saslname_for_mech`: the SASL name of a registered mechanism. A mechanism with no SASL name is
    /// `GSS_S_UNAVAILABLE`.
    pub fn saslname_for_mech(&self, oid: &Oid) -> Result<&str, RoutineError> {
        self.find(oid)?.sasl_name().ok_or(RoutineError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kerberos() -> Oid {
        Oid::parse("1.2.840.113554.1.2.2").unwrap()
    }

    fn spnego() -> Oid {
        Oid::parse("1.3.6.1.5.5.2").unwrap()
    }

    // Kerberos is concrete with a SASL name; SPNEGO has no SASL name and is marked deprecated in this model.
    fn sample() -> Registry {
        let mut r = Registry::new();
        r.register(Mechanism::new(
            kerberos(),
            Some("GSSAPI"),
            &[Attr::AuthInit, Attr::AuthTarg, Attr::IntegProt, Attr::ConfProt, Attr::MechConcrete],
        ))
        .unwrap();
        r.register(Mechanism::new(spnego(), None, &[Attr::AuthInit, Attr::Deprecated])).unwrap();
        r
    }

    #[test]
    fn an_empty_registry_indicates_no_mechanisms() {
        let r = Registry::new();
        assert!(r.indicate_mechs().is_empty());
        assert_eq!(r.indicate_mechs().len(), 0);
    }

    #[test]
    fn indicate_mechs_lists_the_identifiers_in_registration_order() {
        let set = sample().indicate_mechs();
        assert_eq!(set.len(), 2);
        assert_eq!(set.iter().next(), Some(&kerberos()));
        assert!(set.contains(&spnego()));
    }

    #[test]
    fn a_duplicate_identifier_or_sasl_name_is_refused_and_leaves_the_registry_unchanged() {
        let mut r = sample();
        assert_eq!(
            r.register(Mechanism::new(kerberos(), None, &[])),
            Err(RoutineError::DuplicateElement)
        );
        assert_eq!(
            r.register(Mechanism::new(Oid::parse("1.2.3").unwrap(), Some("GSSAPI"), &[])),
            Err(RoutineError::DuplicateElement)
        );
        assert_eq!(r.indicate_mechs().len(), 2);
    }

    #[test]
    fn indicate_by_attrs_needs_every_desired_attribute() {
        // SPNEGO lacks integrity protection, so only Kerberos has both attributes.
        let set = sample().indicate_by_attrs(&[Attr::AuthInit, Attr::IntegProt], &[]);
        assert_eq!(set.len(), 1);
        assert!(set.contains(&kerberos()));
    }

    #[test]
    fn indicate_by_attrs_drops_a_mechanism_with_an_excluded_attribute() {
        // Only the deprecated mechanism is excluded; the other one has AuthInit and survives.
        let set = sample().indicate_by_attrs(&[Attr::AuthInit], &[Attr::Deprecated]);
        assert_eq!(set.len(), 1);
        assert!(set.contains(&kerberos()));
        assert!(!set.contains(&spnego()));
    }

    #[test]
    fn sasl_names_map_both_ways_and_refuse_what_is_not_there() {
        let r = sample();
        assert_eq!(r.mech_for_saslname("GSSAPI"), Ok(kerberos()));
        assert_eq!(r.saslname_for_mech(&kerberos()), Ok("GSSAPI"));
        assert_eq!(r.saslname_for_mech(&spnego()), Err(RoutineError::Unavailable));
        assert_eq!(r.mech_for_saslname("NTLM"), Err(RoutineError::BadMech));
    }

    #[test]
    fn an_unknown_mechanism_has_no_attributes_and_attribute_names_round_trip() {
        let r = sample();
        assert_eq!(r.attrs_for_mech(&Oid::parse("1.2.3").unwrap()), Err(RoutineError::BadMech));
        assert_eq!(r.attrs_for_mech(&kerberos()).unwrap().len(), 5);
        assert_eq!(Attr::IntegProt.name(), "GSS_C_MA_INTEG_PROT");
        assert_eq!(Attr::from_name("GSS_C_MA_CONF_PROT"), Ok(Attr::ConfProt));
        assert_eq!(Attr::from_name("GSS_C_MA_NOPE"), Err(RoutineError::BadMechAttr));
    }
}
