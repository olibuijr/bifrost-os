//! App-scoped credential binding: which apps may read one keychain credential, and whether the person must present
//! user presence (a passcode or biometry) before the credential is released.
//!
//! The keychain already keeps each app's items apart (an app reads only its own). A binding adds two things on top:
//! the owner may list other apps that may read the item (a grant, the way a shared credential is handed on), and the
//! owner may require a presence [`Constraint`] that every read must satisfy. Only the owner grants and revokes.
//!
//! Pure model: the keychain service stores the encoded binding next to the item ([`Binding::encode`]) and asks
//! [`Binding::check_read`] before it returns the secret. The presence itself comes from the local authentication
//! service, which is where the person answers the sheet.

use crate::access_control::{AccessControl, Constraint, Error as AccessError, Expr, Presence};

/// Why a binding refused something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denied {
    /// The requesting app is neither the owner nor listed.
    NotListed,
    /// Only the owner may change the grants.
    NotOwner,
    /// The read needs user presence the person has not given.
    PresenceRequired,
    /// A name holds a line break or a comma, which the stored form cannot keep.
    BadName,
}

/// One credential item's binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    owner: String,
    service: String,
    account: String,
    allowed: Vec<String>,
    presence: Option<Constraint>,
}

fn name_ok(name: &str) -> bool {
    !name.is_empty() && !name.contains(['\n', '\r', ','])
}

impl Binding {
    /// A binding for an item of `owner`, with no other app listed. `presence`, when given, must hold on every read.
    pub fn new(owner: &str, service: &str, account: &str, presence: Option<Constraint>) -> Result<Binding, Denied> {
        if !(name_ok(owner) && name_ok(service) && name_ok(account)) {
            return Err(Denied::BadName);
        }
        Ok(Binding { owner: owner.into(), service: service.into(), account: account.into(), allowed: Vec::new(), presence })
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The apps other than the owner that may read the item.
    pub fn allowed(&self) -> &[String] {
        &self.allowed
    }

    /// The presence the item requires, if any.
    pub fn presence(&self) -> Option<Constraint> {
        self.presence
    }

    /// Lets `app` read the item. Only the owner may grant. Returns false when the app was already listed.
    pub fn grant(&mut self, by: &str, app: &str) -> Result<bool, Denied> {
        if by != self.owner {
            return Err(Denied::NotOwner);
        }
        if !name_ok(app) {
            return Err(Denied::BadName);
        }
        if app == self.owner || self.allowed.iter().any(|a| a == app) {
            return Ok(false);
        }
        self.allowed.push(app.into());
        Ok(true)
    }

    /// Takes `app`'s grant away. Only the owner may revoke. Returns false when the app was not listed.
    pub fn revoke(&mut self, by: &str, app: &str) -> Result<bool, Denied> {
        if by != self.owner {
            return Err(Denied::NotOwner);
        }
        let before = self.allowed.len();
        self.allowed.retain(|a| a != app);
        Ok(self.allowed.len() != before)
    }

    /// Decides whether `requester` may read the secret given the presence the person has shown.
    pub fn check_read(&self, requester: &str, presence: &Presence) -> Result<(), Denied> {
        if requester != self.owner && !self.allowed.iter().any(|a| a == requester) {
            return Err(Denied::NotListed);
        }
        if let Some(constraint) = self.presence {
            // a constraint built from a single name always validates; a failure here would mean the name is invalid
            let control = AccessControl::new(Expr::Constraint(constraint)).map_err(|_: AccessError| Denied::PresenceRequired)?;
            if !control.allows(presence) {
                return Err(Denied::PresenceRequired);
            }
        }
        Ok(())
    }

    /// The binding as the keychain stores it next to the item: one `key=value` line per field.
    pub fn encode(&self) -> String {
        let presence = self.presence.map(constraint_name).unwrap_or("none");
        format!(
            "owner={}\nservice={}\naccount={}\nallowed={}\npresence={}\n",
            self.owner,
            self.service,
            self.account,
            self.allowed.join(","),
            presence
        )
    }

    /// Reads a binding written by [`Binding::encode`]. Unknown or missing fields are `None`.
    pub fn decode(text: &str) -> Option<Binding> {
        let field = |key: &str| -> Option<String> {
            text.lines().find_map(|l| l.strip_prefix(key).and_then(|rest| rest.strip_prefix('=')).map(str::to_string))
        };
        let owner = field("owner")?;
        let service = field("service")?;
        let account = field("account")?;
        let allowed: Vec<String> = field("allowed").unwrap_or_default().split(',').filter(|a| !a.is_empty()).map(str::to_string).collect();
        let presence = match field("presence")?.as_str() {
            "none" => None,
            name => Some(constraint_from_name(name)?),
        };
        let mut binding = Binding::new(&owner, &service, &account, presence).ok()?;
        binding.allowed = allowed;
        Some(binding)
    }
}

/// The stored name of a presence constraint.
pub fn constraint_name(c: Constraint) -> &'static str {
    match c {
        Constraint::DevicePasscode => "device_passcode",
        Constraint::BiometryAny => "biometry_any",
        Constraint::BiometryCurrentSet => "biometry_current_set",
        Constraint::UserPresence => "user_presence",
        Constraint::Watch => "watch",
    }
}

/// The constraint a stored name stands for.
pub fn constraint_from_name(name: &str) -> Option<Constraint> {
    [Constraint::DevicePasscode, Constraint::BiometryAny, Constraint::BiometryCurrentSet, Constraint::UserPresence, Constraint::Watch]
        .into_iter()
        .find(|c| constraint_name(*c) == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passcode() -> Presence {
        Presence { passcode: true, biometry: false, enrolled_set_unchanged: true, watch: false }
    }

    fn nothing() -> Presence {
        Presence { passcode: false, biometry: false, enrolled_set_unchanged: true, watch: false }
    }

    #[test]
    fn the_owner_reads_and_others_need_a_grant() {
        let b = Binding::new("app.owner", "svc", "acct", None).unwrap();
        assert_eq!(b.check_read("app.owner", &nothing()), Ok(()));
        assert_eq!(b.check_read("app.other", &nothing()), Err(Denied::NotListed));
    }

    #[test]
    fn only_the_owner_grants_and_revokes() {
        let mut b = Binding::new("app.owner", "svc", "acct", None).unwrap();
        assert_eq!(b.grant("app.other", "app.third"), Err(Denied::NotOwner));
        assert_eq!(b.grant("app.owner", "app.other"), Ok(true));
        assert_eq!(b.grant("app.owner", "app.other"), Ok(false), "a second grant changes nothing");
        assert_eq!(b.check_read("app.other", &nothing()), Ok(()));
        assert_eq!(b.revoke("app.other", "app.other"), Err(Denied::NotOwner));
        assert_eq!(b.revoke("app.owner", "app.other"), Ok(true));
        assert_eq!(b.check_read("app.other", &nothing()), Err(Denied::NotListed));
    }

    #[test]
    fn a_presence_requirement_must_be_shown_on_every_read() {
        let b = Binding::new("app.owner", "svc", "acct", Some(Constraint::DevicePasscode)).unwrap();
        assert_eq!(b.check_read("app.owner", &nothing()), Err(Denied::PresenceRequired));
        assert_eq!(b.check_read("app.owner", &passcode()), Ok(()));
    }

    #[test]
    fn names_with_separators_are_refused() {
        assert_eq!(Binding::new("app,owner", "svc", "acct", None), Err(Denied::BadName));
        assert_eq!(Binding::new("app", "svc\n", "acct", None), Err(Denied::BadName));
        assert_eq!(Binding::new("", "svc", "acct", None), Err(Denied::BadName));
    }

    #[test]
    fn a_binding_round_trips_through_its_stored_form() {
        let mut b = Binding::new("app.owner", "svc", "acct", Some(Constraint::UserPresence)).unwrap();
        b.grant("app.owner", "app.a").unwrap();
        b.grant("app.owner", "app.b").unwrap();
        let back = Binding::decode(&b.encode()).unwrap();
        assert_eq!(back, b);
        assert_eq!(back.presence(), Some(Constraint::UserPresence));
    }

    #[test]
    fn a_stored_form_without_its_fields_is_refused() {
        assert_eq!(Binding::decode("owner=app.owner\nservice=svc\n"), None);
        assert_eq!(Binding::decode("owner=a\nservice=s\naccount=c\nallowed=\npresence=bogus\n"), None);
    }

    #[test]
    fn every_constraint_has_a_stored_name_that_reads_back() {
        for c in [Constraint::DevicePasscode, Constraint::BiometryAny, Constraint::BiometryCurrentSet, Constraint::UserPresence, Constraint::Watch] {
            assert_eq!(constraint_from_name(constraint_name(c)), Some(c));
        }
    }
}
