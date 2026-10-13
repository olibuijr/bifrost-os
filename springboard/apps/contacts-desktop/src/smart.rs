//! Smart groups for contacts: sidebar lists computed from the contact fields, so nothing is stored and they update as
//! contacts change. Each group is a rule over one contact. Pure std.

use springboard::shell::contactstore::Contact;
use std::collections::BTreeSet;

/// The built-in smart groups, in sidebar order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Smart {
    /// Contacts with an email address.
    WithEmail,
    /// Contacts with a phone number.
    WithPhone,
    /// Contacts with a company name.
    WithCompany,
    /// Contacts missing a phone number or an email address.
    Incomplete,
}

impl Smart {
    pub const ALL: [Smart; 4] = [Smart::WithEmail, Smart::WithPhone, Smart::WithCompany, Smart::Incomplete];

    /// The sidebar title.
    pub fn title(self) -> &'static str {
        match self {
            Smart::WithEmail => "With Email",
            Smart::WithPhone => "With Phone",
            Smart::WithCompany => "With Company",
            Smart::Incomplete => "Missing Details",
        }
    }

    /// The sidebar symbol.
    pub fn symbol(self) -> &'static str {
        match self {
            Smart::WithEmail => "envelope",
            Smart::WithPhone => "phone",
            Smart::WithCompany => "building.2",
            Smart::Incomplete => "exclamationmark.circle",
        }
    }

    /// The position in `ALL`, which is the sidebar row offset.
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// The group at a sidebar row offset, if there is one.
    pub fn from_index(index: usize) -> Option<Smart> {
        Self::ALL.get(index).copied()
    }

    /// Whether a contact belongs in the group. A blank field (only spaces counts as blank) does not count.
    pub fn matches(self, c: &Contact) -> bool {
        let filled = |s: &str| !s.trim().is_empty();
        match self {
            Smart::WithEmail => filled(&c.email),
            Smart::WithPhone => filled(&c.phone),
            Smart::WithCompany => filled(&c.company),
            Smart::Incomplete => !filled(&c.email) || !filled(&c.phone),
        }
    }

    /// The identifiers of the contacts in the group.
    pub fn ids(self, contacts: &[Contact]) -> BTreeSet<u64> {
        contacts.iter().filter(|c| self.matches(c)).map(|c| c.id).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(id: u64, email: &str, phone: &str, company: &str) -> Contact {
        Contact {
            id,
            name: format!("Person {id}"),
            email: email.into(),
            phone: phone.into(),
            company: company.into(),
            ..Default::default()
        }
    }

    #[test]
    fn each_rule_reads_its_own_field() {
        let c = contact(1, "a@example.is", "", "Icelandic Co");
        assert!(Smart::WithEmail.matches(&c));
        assert!(!Smart::WithPhone.matches(&c));
        assert!(Smart::WithCompany.matches(&c));
        assert!(Smart::Incomplete.matches(&c), "no phone is incomplete");
    }

    #[test]
    fn blank_fields_made_of_spaces_do_not_count() {
        let c = contact(2, "   ", "\t", "  ");
        assert!(!Smart::WithEmail.matches(&c));
        assert!(!Smart::WithPhone.matches(&c));
        assert!(!Smart::WithCompany.matches(&c));
        assert!(Smart::Incomplete.matches(&c));
    }

    #[test]
    fn a_complete_contact_is_not_incomplete() {
        let c = contact(3, "b@example.is", "5551234", "");
        assert!(!Smart::Incomplete.matches(&c));
    }

    #[test]
    fn ids_list_only_the_matching_contacts_in_order() {
        let list = [
            contact(5, "e@example.is", "555", ""),
            contact(2, "", "555", ""),
            contact(9, "x@example.is", "", ""),
        ];
        assert_eq!(Smart::WithEmail.ids(&list).into_iter().collect::<Vec<_>>(), vec![5, 9]);
        assert_eq!(Smart::WithPhone.ids(&list).into_iter().collect::<Vec<_>>(), vec![2, 5]);
        assert_eq!(Smart::Incomplete.ids(&list).into_iter().collect::<Vec<_>>(), vec![2, 9]);
        assert!(Smart::WithCompany.ids(&list).is_empty());
    }

    #[test]
    fn sidebar_offsets_round_trip() {
        for (i, s) in Smart::ALL.iter().enumerate() {
            assert_eq!(s.index(), i);
            assert_eq!(Smart::from_index(i), Some(*s));
        }
        assert_eq!(Smart::from_index(Smart::ALL.len()), None);
    }

    #[test]
    fn every_group_has_a_title_and_symbol() {
        for s in Smart::ALL {
            assert!(!s.title().is_empty());
            assert!(!s.symbol().is_empty());
        }
    }
}
