//! The system suggestion panel for autofill (task: AutoFillUI). It turns the saved credential identities that cover
//! the field's service into rows a panel can draw, filters them by what the person has typed, and keeps the
//! highlighted row. Pure model: the shell draws the rows and calls [`Panel::next`], [`Panel::previous`] and
//! [`Panel::chosen`]; nothing here reads a store or fills a field.

use crate::credential_identity::{CredentialIdentity, IdentityKind, IdentityStore, IdentityTypes, ServiceIdentifier};

/// One row of the panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    /// The user name, or the host when the identity has no user name.
    pub title: String,
    /// The kind and the host, such as `Password · example.com`.
    pub subtitle: String,
    pub kind: IdentityKind,
    /// The record in the owner's database that filling this row uses.
    pub record_identifier: String,
}

/// The rows the panel offers for `service`, from the identities of the kinds in `types`. When `typed` is not empty,
/// only rows whose user name starts with it (case-insensitive) are kept. The order is the store's: rank, then name.
pub fn suggestions(store: &IdentityStore, service: Option<&ServiceIdentifier>, typed: &str, types: IdentityTypes) -> Vec<Suggestion> {
    store.identities_for(service, types).into_iter().filter(|c| matches_typed(c, typed)).map(row).collect()
}

fn matches_typed(identity: &CredentialIdentity, typed: &str) -> bool {
    let typed = typed.trim().to_lowercase();
    typed.is_empty() || identity.user.to_lowercase().starts_with(&typed)
}

fn row(identity: &CredentialIdentity) -> Suggestion {
    let host = identity.service.host();
    let kind = match identity.kind {
        IdentityKind::Passkey => "Passkey",
        IdentityKind::Password => "Password",
        IdentityKind::OneTimeCode => "One-time code",
    };
    let title = if identity.user.is_empty() { host.clone() } else { identity.user.clone() };
    let subtitle = if host.is_empty() { kind.to_string() } else { format!("{kind} \u{b7} {host}") };
    Suggestion { title, subtitle, kind: identity.kind, record_identifier: identity.record_identifier.clone() }
}

/// The panel's rows and the highlighted one. Nothing is highlighted until the person moves into the list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Panel {
    rows: Vec<Suggestion>,
    selected: Option<usize>,
}

impl Panel {
    pub fn new(rows: Vec<Suggestion>) -> Panel {
        Panel { rows, selected: None }
    }

    pub fn rows(&self) -> &[Suggestion] {
        &self.rows
    }

    /// The highlighted row index, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Moves the highlight down. From no highlight it goes to the first row; past the last row it stops there.
    pub fn next(&mut self) {
        self.selected = match self.selected {
            _ if self.rows.is_empty() => None,
            None => Some(0),
            Some(i) => Some((i + 1).min(self.rows.len() - 1)),
        };
    }

    /// Moves the highlight up. From the first row, or from no highlight, it stays at the first row.
    pub fn previous(&mut self) {
        self.selected = match self.selected {
            _ if self.rows.is_empty() => None,
            None | Some(0) => Some(0),
            Some(i) => Some(i - 1),
        };
    }

    /// The highlighted row, which is what Return fills in.
    pub fn chosen(&self) -> Option<&Suggestion> {
        self.selected.and_then(|i| self.rows.get(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credential_identity::IdentifierType;

    fn save(store: &mut IdentityStore, user: &str, host: &str, record: &str, rank: i64, kind: IdentityKind) {
        let item = CredentialIdentity {
            service: ServiceIdentifier::new(host, IdentifierType::Domain),
            user: user.to_string(),
            record_identifier: record.to_string(),
            rank,
            kind,
        };
        store.save(&[item]).unwrap();
    }

    fn store() -> IdentityStore {
        let mut s = IdentityStore::new();
        save(&mut s, "olibuijr", "example.com", "r1", 5, IdentityKind::Password);
        save(&mut s, "olaf", "example.com", "r2", 9, IdentityKind::Password);
        save(&mut s, "", "example.com", "r3", 1, IdentityKind::OneTimeCode);
        save(&mut s, "other", "elsewhere.org", "r4", 7, IdentityKind::Password);
        s
    }

    #[test]
    fn rows_for_a_service_follow_the_store_order_and_name_their_kind() {
        let s = store();
        let svc = ServiceIdentifier::new("https://login.example.com/session", IdentifierType::Url);
        let rows = suggestions(&s, Some(&svc), "", IdentityTypes::all());
        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["olaf", "olibuijr", "example.com"]);
        assert_eq!(rows[0].subtitle, "Password \u{b7} example.com");
        assert_eq!(rows[2].subtitle, "One-time code \u{b7} example.com");
    }

    #[test]
    fn typed_text_keeps_only_user_names_that_start_with_it() {
        let s = store();
        let svc = ServiceIdentifier::new("example.com", IdentifierType::Domain);
        let rows = suggestions(&s, Some(&svc), "OL", IdentityTypes::all());
        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["olaf", "olibuijr"]);
        assert!(suggestions(&s, Some(&svc), "zz", IdentityTypes::all()).is_empty());
    }

    #[test]
    fn the_kinds_filter_applies() {
        let s = store();
        let svc = ServiceIdentifier::new("example.com", IdentifierType::Domain);
        let codes = IdentityTypes { passkey: false, password: false, one_time_code: true };
        let rows = suggestions(&s, Some(&svc), "", codes);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, IdentityKind::OneTimeCode);
        assert_eq!(rows[0].record_identifier, "r3");
    }

    #[test]
    fn a_disabled_store_offers_nothing() {
        let mut s = store();
        s.set_enabled(false);
        assert!(suggestions(&s, None, "", IdentityTypes::all()).is_empty());
    }

    #[test]
    fn the_highlight_moves_within_the_rows_and_stops_at_both_ends() {
        let mut p = Panel::new(suggestions(&store(), None, "", IdentityTypes::all()));
        assert_eq!(p.selected(), None);
        assert!(p.chosen().is_none());
        p.previous();
        assert_eq!(p.selected(), Some(0));
        p.next();
        p.next();
        p.next();
        p.next();
        assert_eq!(p.selected(), Some(3));
        assert_eq!(p.chosen().unwrap().title, "example.com");
        p.previous();
        assert_eq!(p.selected(), Some(2));
    }

    #[test]
    fn an_empty_panel_never_highlights_a_row() {
        let mut p = Panel::new(Vec::new());
        p.next();
        p.previous();
        assert_eq!(p.selected(), None);
        assert!(p.chosen().is_none());
    }
}
