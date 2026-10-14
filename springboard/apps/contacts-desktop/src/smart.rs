//! Smart groups for contacts: sidebar lists computed from the contact fields, so nothing is stored and they update as
//! contacts change. Each built-in group is a rule over one contact. A person's own smart lists (`UserList`) hold several
//! rules (a field, a condition and a value) that must all match, or any of which may; the lists themselves are saved in
//! `smartlists.txt` (see `group_file.rs`). Pure std.

use crate::extras::{Extras, ValueKind};
use crate::names;
use springboard::shell::contactstore::Contact;
use std::collections::BTreeSet;

/// Most smart lists a person can make.
pub const MAX_LISTS: usize = 30;
/// Most rules in one smart list.
pub const MAX_RULES: usize = 6;
/// Longest smart list name and rule value, in characters.
pub const MAX_NAME: usize = 80;
pub const MAX_VALUE: usize = 160;

/// The contact field a rule reads. Phone, email and address also read the extra values of that kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleField {
    Name,
    Company,
    Phone,
    Email,
    Address,
    Notes,
}

impl RuleField {
    pub const ALL: [RuleField; 6] = [RuleField::Name, RuleField::Company, RuleField::Phone, RuleField::Email, RuleField::Address, RuleField::Notes];

    pub fn title(self) -> &'static str {
        match self {
            RuleField::Name => "Name",
            RuleField::Company => "Company",
            RuleField::Phone => "Phone",
            RuleField::Email => "Email",
            RuleField::Address => "Address",
            RuleField::Notes => "Notes",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            RuleField::Name => "name",
            RuleField::Company => "company",
            RuleField::Phone => "phone",
            RuleField::Email => "email",
            RuleField::Address => "address",
            RuleField::Notes => "notes",
        }
    }

    pub fn from_code(code: &str) -> Option<RuleField> {
        RuleField::ALL.into_iter().find(|f| f.code() == code)
    }

    /// The next field in `ALL`, after the last the first.
    pub fn next(self) -> RuleField {
        let at = RuleField::ALL.iter().position(|f| *f == self).unwrap_or(0);
        RuleField::ALL[(at + 1) % RuleField::ALL.len()]
    }
}

/// How a rule compares a field with its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Condition {
    Contains,
    DoesNotContain,
    Is,
    IsNot,
    BeginsWith,
    EndsWith,
    IsSet,
    IsNotSet,
}

impl Condition {
    pub const ALL: [Condition; 8] = [
        Condition::Contains,
        Condition::DoesNotContain,
        Condition::Is,
        Condition::IsNot,
        Condition::BeginsWith,
        Condition::EndsWith,
        Condition::IsSet,
        Condition::IsNotSet,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Condition::Contains => "contains",
            Condition::DoesNotContain => "does not contain",
            Condition::Is => "is",
            Condition::IsNot => "is not",
            Condition::BeginsWith => "begins with",
            Condition::EndsWith => "ends with",
            Condition::IsSet => "is set",
            Condition::IsNotSet => "is not set",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Condition::Contains => "contains",
            Condition::DoesNotContain => "not-contains",
            Condition::Is => "is",
            Condition::IsNot => "is-not",
            Condition::BeginsWith => "begins",
            Condition::EndsWith => "ends",
            Condition::IsSet => "set",
            Condition::IsNotSet => "not-set",
        }
    }

    pub fn from_code(code: &str) -> Option<Condition> {
        Condition::ALL.into_iter().find(|c| c.code() == code)
    }

    /// The next condition in `ALL`, after the last the first.
    pub fn next(self) -> Condition {
        let at = Condition::ALL.iter().position(|c| *c == self).unwrap_or(0);
        Condition::ALL[(at + 1) % Condition::ALL.len()]
    }

    /// Whether the condition compares with a value (`is set` and `is not set` do not).
    pub fn needs_value(self) -> bool {
        !matches!(self, Condition::IsSet | Condition::IsNotSet)
    }
}

/// One rule of a smart list: the field, the condition and the value to compare with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub field: RuleField,
    pub condition: Condition,
    pub value: String,
}

/// The texts a rule reads from a contact: the store's value and, for a phone, email or address, the extra values.
fn texts(c: &Contact, extras: &Extras, field: RuleField) -> Vec<String> {
    let (store, kind) = match field {
        RuleField::Name => (&c.name, None),
        RuleField::Company => (&c.company, None),
        RuleField::Phone => (&c.phone, Some(ValueKind::Phone)),
        RuleField::Email => (&c.email, Some(ValueKind::Email)),
        RuleField::Address => (&c.address, Some(ValueKind::Address)),
        RuleField::Notes => (&c.notes, None),
    };
    let mut out = vec![store.clone()];
    if let Some(kind) = kind {
        out.extend(extras.get(c.id).iter().filter(|e| e.kind == kind).map(|e| e.value.clone()));
    }
    out
}

impl Rule {
    /// Whether the contact satisfies the rule. Texts are compared ignoring case and accents, and a blank text counts as
    /// not set. `does not contain` and `is not` hold when none of the field's texts match.
    pub fn matches(&self, c: &Contact, extras: &Extras) -> bool {
        let found: Vec<String> = texts(c, extras, self.field).iter().map(|t| names::fold(t)).filter(|t| !t.is_empty()).collect();
        let want = names::fold(&self.value);
        match self.condition {
            Condition::IsSet => !found.is_empty(),
            Condition::IsNotSet => found.is_empty(),
            Condition::Contains => found.iter().any(|t| t.contains(&want)),
            Condition::DoesNotContain => !found.iter().any(|t| t.contains(&want)),
            Condition::Is => found.iter().any(|t| *t == want),
            Condition::IsNot => !found.iter().any(|t| *t == want),
            Condition::BeginsWith => found.iter().any(|t| t.starts_with(&want)),
            Condition::EndsWith => found.iter().any(|t| t.ends_with(&want)),
        }
    }

    /// True when the rule can be saved: a condition that needs a value has one (within `MAX_VALUE` characters).
    pub fn valid(&self) -> bool {
        let value = self.value.trim();
        value.chars().count() <= MAX_VALUE && (!self.condition.needs_value() || !value.is_empty())
    }
}

/// A smart list a person made: a name, whether any or all of its rules must match, and the rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserList {
    pub name: String,
    /// Any rule may match (otherwise all must).
    pub any: bool,
    pub rules: Vec<Rule>,
}

impl UserList {
    /// A name of one to `MAX_NAME` characters without control characters, and one to `MAX_RULES` valid rules.
    pub fn valid(&self) -> bool {
        let name = self.name.trim();
        !name.is_empty()
            && name.chars().count() <= MAX_NAME
            && !name.chars().any(char::is_control)
            && (1..=MAX_RULES).contains(&self.rules.len())
            && self.rules.iter().all(Rule::valid)
    }

    /// Whether a contact belongs to the list. A list without rules holds nobody.
    pub fn matches(&self, c: &Contact, extras: &Extras) -> bool {
        if self.rules.is_empty() {
            return false;
        }
        if self.any {
            self.rules.iter().any(|r| r.matches(c, extras))
        } else {
            self.rules.iter().all(|r| r.matches(c, extras))
        }
    }

    /// The identifiers of the contacts in the list.
    pub fn ids(&self, contacts: &[Contact], extras: &Extras) -> BTreeSet<u64> {
        contacts.iter().filter(|c| self.matches(c, extras)).map(|c| c.id).collect()
    }
}

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

    fn rule(field: RuleField, condition: Condition, value: &str) -> Rule {
        Rule { field, condition, value: value.into() }
    }

    fn person(id: u64, name: &str, company: &str, email: &str) -> Contact {
        Contact { id, name: name.into(), company: company.into(), email: email.into(), ..Default::default() }
    }

    #[test]
    fn text_conditions_ignore_case_and_accents() {
        let c = person(1, "Ólafur Búi", "Acme Ltd", "olafur@acme.is");
        let none = Extras::default();
        assert!(rule(RuleField::Name, Condition::Contains, "bui").matches(&c, &none));
        assert!(rule(RuleField::Name, Condition::Is, "OLAFUR  bui").matches(&c, &none), "spaces are folded too");
        assert!(rule(RuleField::Name, Condition::BeginsWith, "ola").matches(&c, &none));
        assert!(rule(RuleField::Email, Condition::EndsWith, ".IS").matches(&c, &none));
        assert!(!rule(RuleField::Name, Condition::Is, "olafur").matches(&c, &none));
        assert!(rule(RuleField::Company, Condition::DoesNotContain, "corp").matches(&c, &none));
        assert!(!rule(RuleField::Company, Condition::DoesNotContain, "acme").matches(&c, &none));
        assert!(rule(RuleField::Company, Condition::IsNot, "acme").matches(&c, &none), "acme ltd is not acme");
    }

    #[test]
    fn set_conditions_treat_blank_text_as_not_set() {
        let none = Extras::default();
        let c = person(1, "Anna", "   ", "a@x.is");
        assert!(rule(RuleField::Email, Condition::IsSet, "").matches(&c, &none));
        assert!(rule(RuleField::Company, Condition::IsNotSet, "").matches(&c, &none));
        assert!(!rule(RuleField::Phone, Condition::IsSet, "").matches(&c, &none));
        assert!(!Condition::IsSet.needs_value() && Condition::Contains.needs_value());
    }

    #[test]
    fn phone_email_and_address_rules_also_read_the_extra_values() {
        let c = person(7, "Anna", "", "");
        let mut extras = Extras::default();
        assert!(extras.add(7, crate::extras::Extra { kind: ValueKind::Email, label: "work".into(), value: "anna@work.is".into() }));
        assert!(rule(RuleField::Email, Condition::EndsWith, "work.is").matches(&c, &extras));
        assert!(rule(RuleField::Email, Condition::IsSet, "").matches(&c, &extras));
        assert!(!rule(RuleField::Phone, Condition::IsSet, "").matches(&c, &extras), "an email is not a phone number");
        assert!(!rule(RuleField::Email, Condition::DoesNotContain, "work").matches(&c, &extras));
    }

    #[test]
    fn a_list_needs_all_its_rules_or_any_of_them() {
        let none = Extras::default();
        let rules = vec![rule(RuleField::Company, Condition::Contains, "acme"), rule(RuleField::Email, Condition::IsSet, "")];
        let mut list = UserList { name: "Acme people".into(), any: false, rules };
        let both = person(1, "Anna", "Acme", "a@acme.is");
        let one = person(2, "Bo", "Acme", "");
        let neither = person(3, "Cara", "", "");
        assert_eq!(list.ids(&[both.clone(), one.clone(), neither.clone()], &none), BTreeSet::from([1]));
        list.any = true;
        assert_eq!(list.ids(&[both, one, neither], &none), BTreeSet::from([1, 2]));
        list.rules.clear();
        assert!(!list.matches(&person(1, "Anna", "Acme", "a@acme.is"), &none), "a list without rules holds nobody");
    }

    #[test]
    fn lists_and_rules_are_checked_before_they_are_kept() {
        let ok = UserList { name: "Friends".into(), any: true, rules: vec![rule(RuleField::Name, Condition::Contains, "an")] };
        assert!(ok.valid());
        assert!(!UserList { name: "  ".into(), ..ok.clone() }.valid(), "a list needs a name");
        assert!(!UserList { rules: vec![], ..ok.clone() }.valid(), "and a rule");
        assert!(!UserList { rules: vec![rule(RuleField::Name, Condition::Is, "  ")], ..ok.clone() }.valid(), "a rule that compares needs a value");
        assert!(UserList { rules: vec![rule(RuleField::Phone, Condition::IsNotSet, "")], ..ok.clone() }.valid());
        assert!(!UserList { rules: vec![rule(RuleField::Name, Condition::Is, "a"); MAX_RULES + 1], ..ok.clone() }.valid());
        assert!(!UserList { name: "x".repeat(MAX_NAME + 1), ..ok }.valid());
    }

    #[test]
    fn fields_and_conditions_cycle_and_read_back_from_their_codes() {
        let mut field = RuleField::Name;
        for _ in 0..RuleField::ALL.len() {
            assert_eq!(RuleField::from_code(field.code()), Some(field));
            field = field.next();
        }
        assert_eq!(field, RuleField::Name, "a full turn comes back to the start");
        let mut condition = Condition::Contains;
        for _ in 0..Condition::ALL.len() {
            assert_eq!(Condition::from_code(condition.code()), Some(condition));
            assert!(!condition.title().is_empty());
            condition = condition.next();
        }
        assert_eq!(condition, Condition::Contains);
        assert_eq!(RuleField::from_code("shoe"), None);
        assert_eq!(Condition::from_code("sorta"), None);
    }
}
