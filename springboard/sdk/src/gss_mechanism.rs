//! GSS-API security mechanisms (`gss_indicate_mechs`, `gss_indicate_mechs_by_attrs`, `gss_inquire_attrs_for_mech`,
//! `gss_inquire_mech_for_saslname`, `gss_inquire_saslname_for_mech`): the registry that maps a mechanism identifier to
//! its attributes and its SASL protocol name, and the queries an application runs over it.
//!
//! A mechanism is named by an object identifier ([`Oid`]). The registry holds one entry per identifier; an identifier
//! can be registered once, and a SASL name can belong to one mechanism only. The queries follow the attribute rule of
//! the GSS-API mechanism attributes: a mechanism is selected when it has every *critical* attribute, at least one
//! *desired* attribute (when any are given), and none of the *excluded* attributes. The reference lists these
//! queries but not the rule, so the rule here is the one the GSS-API attribute specification (RFC 5587) states.
//!
//! Not here: the mechanism implementations behind the identifiers. The shell registers none, so the registry is
//! empty in a running app and only the model is tested.

use crate::gss_oid::{Oid, OidSet};
use std::collections::BTreeSet;
use std::fmt;

/// One registered mechanism.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mechanism {
    oid: Oid,
    name: String,
    sasl_name: Option<String>,
    attributes: BTreeSet<String>,
}

impl Mechanism {
    /// A mechanism with a display name, an optional SASL protocol name and its attribute names.
    pub fn new(oid: Oid, name: &str, sasl_name: Option<&str>, attributes: &[&str]) -> Mechanism {
        Mechanism {
            oid,
            name: name.to_string(),
            sasl_name: sasl_name.map(|s| s.to_string()),
            attributes: attributes.iter().map(|a| a.to_string()).collect(),
        }
    }

    pub fn oid(&self) -> &Oid {
        &self.oid
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn sasl_name(&self) -> Option<&str> {
        self.sasl_name.as_deref()
    }

    /// True when the mechanism has the named attribute.
    pub fn has(&self, attribute: &str) -> bool {
        self.attributes.contains(attribute)
    }
}

/// Why a registry change or query failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MechanismError {
    /// The identifier is already registered.
    Duplicate,
    /// Another mechanism already uses this SASL protocol name.
    DuplicateSaslName(String),
    /// The name or SASL name is empty.
    EmptyName,
    /// No mechanism is registered under the identifier.
    UnknownMechanism,
}

impl fmt::Display for MechanismError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MechanismError::Duplicate => f.write_str("mechanism identifier is already registered"),
            MechanismError::DuplicateSaslName(name) => write!(f, "SASL name {} is already used", name),
            MechanismError::EmptyName => f.write_str("mechanism name is empty"),
            MechanismError::UnknownMechanism => f.write_str("no mechanism is registered under this identifier"),
        }
    }
}

impl std::error::Error for MechanismError {}

/// The attribute criteria of `gss_indicate_mechs_by_attrs`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Criteria {
    /// Every one must be present.
    pub critical: Vec<String>,
    /// At least one must be present, when this list is not empty.
    pub desired: Vec<String>,
    /// None may be present.
    pub excluded: Vec<String>,
}

/// The mechanisms the application can use, in registration order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MechanismRegistry {
    entries: Vec<Mechanism>,
}

impl MechanismRegistry {
    pub fn new() -> MechanismRegistry {
        MechanismRegistry::default()
    }

    /// Registers a mechanism. The identifier, the display name and any SASL name must each be new (the name must not be
    /// empty).
    pub fn register(&mut self, mechanism: Mechanism) -> Result<(), MechanismError> {
        if mechanism.name.is_empty() {
            return Err(MechanismError::EmptyName);
        }
        if self.entries.iter().any(|m| m.oid == mechanism.oid) {
            return Err(MechanismError::Duplicate);
        }
        if let Some(sasl) = &mechanism.sasl_name {
            if sasl.is_empty() {
                return Err(MechanismError::EmptyName);
            }
            if self.entries.iter().any(|m| m.sasl_name.as_deref() == Some(sasl.as_str())) {
                return Err(MechanismError::DuplicateSaslName(sasl.clone()));
            }
        }
        self.entries.push(mechanism);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `gss_indicate_mechs`: every registered mechanism identifier, in registration order.
    pub fn indicate(&self) -> OidSet {
        let mut set = OidSet::new();
        for m in &self.entries {
            set.add(m.oid.clone());
        }
        set
    }

    /// `gss_indicate_mechs_by_attrs`: the mechanisms that meet the criteria, in registration order.
    pub fn indicate_by_attrs(&self, criteria: &Criteria) -> OidSet {
        let mut set = OidSet::new();
        for m in self.entries.iter().filter(|m| meets(m, criteria)) {
            set.add(m.oid.clone());
        }
        set
    }

    /// `gss_inquire_attrs_for_mech`: the attributes of one mechanism, or with `None` the union over all of them, sorted.
    pub fn attributes_for(&self, oid: Option<&Oid>) -> Result<Vec<String>, MechanismError> {
        match oid {
            Some(oid) => {
                let m = self.find(oid)?;
                Ok(m.attributes.iter().cloned().collect())
            }
            None => {
                let all: BTreeSet<String> = self.entries.iter().flat_map(|m| m.attributes.iter().cloned()).collect();
                Ok(all.into_iter().collect())
            }
        }
    }

    /// `gss_inquire_mech_for_saslname`: the mechanism that uses this SASL protocol name, if any.
    pub fn for_sasl_name(&self, sasl: &str) -> Option<&Oid> {
        self.entries.iter().find(|m| m.sasl_name.as_deref() == Some(sasl)).map(|m| &m.oid)
    }

    /// `gss_inquire_saslname_for_mech`: the SASL name of one mechanism. `Ok(None)` when it has none.
    pub fn sasl_name_for(&self, oid: &Oid) -> Result<Option<&str>, MechanismError> {
        Ok(self.find(oid)?.sasl_name.as_deref())
    }

    fn find(&self, oid: &Oid) -> Result<&Mechanism, MechanismError> {
        self.entries.iter().find(|m| &m.oid == oid).ok_or(MechanismError::UnknownMechanism)
    }
}

/// The attribute rule: critical all present, desired any present (or none asked), excluded none present.
fn meets(m: &Mechanism, c: &Criteria) -> bool {
    c.critical.iter().all(|a| m.has(a))
        && (c.desired.is_empty() || c.desired.iter().any(|a| m.has(a)))
        && !c.excluded.iter().any(|a| m.has(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(last: u32) -> Oid {
        Oid::from_arcs(&[1, 2, 840, 113554, 1, 2, last]).unwrap()
    }

    fn registry() -> MechanismRegistry {
        let mut r = MechanismRegistry::new();
        r.register(Mechanism::new(oid(2), "Kerberos", Some("GSSAPI"), &["pseudo-random", "delegation"])).unwrap();
        r.register(Mechanism::new(oid(3), "Negotiate", None, &["delegation", "negotiable"])).unwrap();
        r.register(Mechanism::new(oid(4), "Local", Some("LOCAL"), &["pseudo-random"])).unwrap();
        r
    }

    #[test]
    fn indicate_lists_every_mechanism_in_registration_order() {
        let r = registry();
        let set = r.indicate();
        assert_eq!(set.len(), 3);
        let order: Vec<Oid> = set.iter().cloned().collect();
        assert_eq!(order, vec![oid(2), oid(3), oid(4)]);
        assert!(MechanismRegistry::new().indicate().is_empty());
    }

    #[test]
    fn attribute_criteria_select_by_critical_desired_and_excluded() {
        let r = registry();
        let delegating = Criteria { critical: vec!["delegation".into()], ..Criteria::default() };
        assert_eq!(r.indicate_by_attrs(&delegating).iter().cloned().collect::<Vec<_>>(), vec![oid(2), oid(3)]);

        let either = Criteria { desired: vec!["negotiable".into(), "pseudo-random".into()], ..Criteria::default() };
        assert_eq!(r.indicate_by_attrs(&either).len(), 3);

        let not_negotiable = Criteria { critical: vec!["pseudo-random".into()], excluded: vec!["negotiable".into()], ..Criteria::default() };
        assert_eq!(r.indicate_by_attrs(&not_negotiable).iter().cloned().collect::<Vec<_>>(), vec![oid(2), oid(4)]);
    }

    #[test]
    fn attributes_for_one_mechanism_or_the_sorted_union() {
        let r = registry();
        assert_eq!(r.attributes_for(Some(&oid(3))).unwrap(), vec!["delegation".to_string(), "negotiable".to_string()]);
        assert_eq!(
            r.attributes_for(None).unwrap(),
            vec!["delegation".to_string(), "negotiable".to_string(), "pseudo-random".to_string()]
        );
        assert_eq!(r.attributes_for(Some(&oid(9))), Err(MechanismError::UnknownMechanism));
    }

    #[test]
    fn sasl_names_map_both_ways_and_missing_names_are_none() {
        let r = registry();
        assert_eq!(r.for_sasl_name("GSSAPI"), Some(&oid(2)));
        assert_eq!(r.for_sasl_name("gssapi"), None);
        assert_eq!(r.sasl_name_for(&oid(2)), Ok(Some("GSSAPI")));
        assert_eq!(r.sasl_name_for(&oid(3)), Ok(None));
        assert_eq!(r.sasl_name_for(&oid(9)), Err(MechanismError::UnknownMechanism));
    }

    #[test]
    fn duplicates_and_empty_names_are_refused_and_leave_the_registry_unchanged() {
        let mut r = registry();
        assert_eq!(r.register(Mechanism::new(oid(2), "Again", None, &[])), Err(MechanismError::Duplicate));
        assert_eq!(
            r.register(Mechanism::new(oid(7), "Other", Some("GSSAPI"), &[])),
            Err(MechanismError::DuplicateSaslName("GSSAPI".to_string()))
        );
        assert_eq!(r.register(Mechanism::new(oid(8), "", None, &[])), Err(MechanismError::EmptyName));
        assert_eq!(r.register(Mechanism::new(oid(8), "Blank SASL", Some(""), &[])), Err(MechanismError::EmptyName));
        assert_eq!(r.len(), 3);
    }
}
