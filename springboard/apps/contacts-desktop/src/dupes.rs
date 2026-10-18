//! Duplicate detection for contacts. Two contacts are duplicates when they share an email address, or the same phone
//! number (compared by its last seven digits, so "+354 555 1234" and "5551234" match), or the same name with no
//! conflicting phone or email. Names match ignoring case and accents. `merge` folds a group of duplicates into its first
//! card. Pure std.

use crate::details::{CardDetails, Details};
use crate::extras::{Extra, Extras, ValueKind, MAX_EXTRAS};
use crate::names;
use springboard::shell::contactstore::{Contact, LABELS, MAX_NOTES};
use std::collections::{BTreeMap, BTreeSet};

fn email_key(c: &Contact) -> String {
    c.email.trim().to_lowercase()
}

fn phone_key(c: &Contact) -> String {
    let digits: String = c.phone.chars().filter(char::is_ascii_digit).collect();
    let start = digits.len().saturating_sub(7);
    digits[start..].to_string()
}

fn name_key(c: &Contact) -> String {
    names::fold(c.name.trim()).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Two non-empty values that differ.
fn conflict(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && a != b
}

/// Whether two contacts describe the same person.
pub fn is_duplicate(a: &Contact, b: &Contact) -> bool {
    let (ea, eb) = (email_key(a), email_key(b));
    if !ea.is_empty() && ea == eb {
        return true;
    }
    let (pa, pb) = (phone_key(a), phone_key(b));
    if !pa.is_empty() && pa == pb {
        return true;
    }
    let (na, nb) = (name_key(a), name_key(b));
    !na.is_empty() && na == nb && !conflict(&pa, &pb) && !conflict(&ea, &eb)
}

/// The groups of duplicate contacts, as identifiers. Only groups of two or more are returned, each sorted.
pub fn groups(contacts: &[Contact]) -> Vec<Vec<u64>> {
    let n = contacts.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    // Contacts that share an email or a phone key are joined directly; names are compared inside their own bucket.
    let mut by_email: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_phone: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, c) in contacts.iter().enumerate() {
        let (e, p, nm) = (email_key(c), phone_key(c), name_key(c));
        if !e.is_empty() {
            if let Some(&j) = by_email.get(&e) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                parent[a] = b;
            } else {
                by_email.insert(e, i);
            }
        }
        if !p.is_empty() {
            if let Some(&j) = by_phone.get(&p) {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                parent[a] = b;
            } else {
                by_phone.insert(p, i);
            }
        }
        if !nm.is_empty() {
            by_name.entry(nm).or_default().push(i);
        }
    }
    for bucket in by_name.values() {
        for (k, &i) in bucket.iter().enumerate() {
            for &j in &bucket[k + 1..] {
                if is_duplicate(&contacts[i], &contacts[j]) {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a] = b;
                }
            }
        }
    }
    let mut sets: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        sets.entry(root).or_default().push(contacts[i].id);
    }
    sets.into_values()
        .filter(|set| set.len() > 1)
        .map(|mut set| {
            set.sort_unstable();
            set
        })
        .collect()
}

/// Every contact that has a duplicate in the list.
pub fn duplicate_ids(contacts: &[Contact]) -> BTreeSet<u64> {
    groups(contacts).into_iter().flatten().collect()
}

/// Whether a candidate duplicates any contact already in the list.
pub fn has_duplicate(existing: &[Contact], candidate: &Contact) -> bool {
    existing.iter().any(|c| is_duplicate(c, candidate))
}

/// What folding a group of duplicates into one card produced.
#[derive(Debug, PartialEq, Eq)]
pub struct Merged {
    /// The card that stays: the first card with the others' fields folded in.
    pub keep: Contact,
    /// The extra values of the kept card.
    pub extras: Vec<Extra>,
    /// The card fields of the kept card.
    pub details: CardDetails,
    /// The identifiers of the cards that are deleted.
    pub removed: Vec<u64>,
    /// Values that did not fit on the kept card (a card keeps `MAX_EXTRAS` extra values and a note of `MAX_NOTES`).
    pub dropped: usize,
}

/// Two values of one kind that name the same thing: phone numbers by their last seven digits (or, with no digits, the
/// text), emails and addresses ignoring case.
fn same_value(kind: ValueKind, a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if kind != ValueKind::Phone {
        return a.to_lowercase() == b.to_lowercase();
    }
    let tail = |text: &str| {
        let digits: String = text.chars().filter(char::is_ascii_digit).collect();
        if digits.is_empty() {
            return text.to_lowercase();
        }
        digits[digits.len().saturating_sub(7)..].to_string()
    };
    tail(a) == tail(b)
}

/// The card being built by a merge.
struct Folded {
    keep: Contact,
    extras: Vec<Extra>,
    details: CardDetails,
    dropped: usize,
}

impl Folded {
    fn has(&self, kind: ValueKind, value: &str) -> bool {
        let store = match kind {
            ValueKind::Phone => &self.keep.phone,
            ValueKind::Email => &self.keep.email,
            ValueKind::Address => &self.keep.address,
        };
        (!store.trim().is_empty() && same_value(kind, store, value))
            || self.extras.iter().any(|e| e.kind == kind && same_value(kind, &e.value, value))
    }

    /// Adds a phone, email or address of another card: into the store field when the kept card has none, otherwise as an
    /// extra value, unless the card already has it.
    fn add(&mut self, kind: ValueKind, label_index: u8, value: &str) {
        let value = value.trim();
        if value.is_empty() || self.has(kind, value) {
            return;
        }
        let k = kind.index();
        let label_index = label_index % LABELS[k].len() as u8;
        let slot = match kind {
            ValueKind::Phone => &mut self.keep.phone,
            ValueKind::Email => &mut self.keep.email,
            ValueKind::Address => &mut self.keep.address,
        };
        if slot.trim().is_empty() {
            *slot = value.to_string();
            self.keep.labels[k] = label_index;
            return;
        }
        let extra = Extra { kind, label: LABELS[k][label_index as usize].to_string(), value: value.to_string() };
        if self.extras.len() < MAX_EXTRAS && extra.well_formed() {
            self.extras.push(extra);
        } else {
            self.dropped += 1;
        }
    }

    fn fold(&mut self, other: &Contact, other_extras: &[Extra], other_details: &CardDetails) {
        if self.keep.name.trim().is_empty() {
            self.keep.name = other.name.clone();
        }
        if self.keep.company.trim().is_empty() {
            self.keep.company = other.company.clone();
        }
        self.keep.favorite |= other.favorite;
        let note = other.notes.trim();
        if !note.is_empty() && !self.keep.notes.contains(note) {
            let joined = if self.keep.notes.trim().is_empty() {
                note.to_string()
            } else {
                format!("{}\n\n{note}", self.keep.notes.trim_end())
            };
            if joined.chars().count() <= MAX_NOTES {
                self.keep.notes = joined;
            } else {
                self.dropped += 1;
            }
        }
        self.add(ValueKind::Phone, other.labels[0], &other.phone);
        self.add(ValueKind::Email, other.labels[1], &other.email);
        self.add(ValueKind::Address, other.labels[2], &other.address);
        for extra in other_extras {
            let k = extra.kind.index();
            let at = LABELS[k].iter().position(|l| *l == extra.label).unwrap_or(LABELS[k].len() - 1);
            self.add(extra.kind, at as u8, &extra.value);
        }
        self.details.company |= other_details.company;
        for (detail, value) in other_details.filled() {
            if self.details.get(detail).is_empty() {
                self.details.set(detail, value);
            }
        }
    }
}

/// Folds the cards `ids` into the first of them. The first card keeps its name, company and favorite mark; fields it
/// lacks are taken from the others, phones, emails and addresses it does not already have become extra values, the
/// notes are joined, and the card fields it lacks are taken over. `None` when fewer than two of the cards exist, or the
/// folded card would not be valid.
pub fn merge(contacts: &[Contact], values: &Extras, card_details: &Details, ids: &[u64]) -> Option<Merged> {
    let cards: Vec<&Contact> = ids.iter().filter_map(|id| contacts.iter().find(|c| c.id == *id)).collect();
    let (first, rest) = cards.split_first()?;
    if rest.is_empty() {
        return None;
    }
    let mut folded = Folded { keep: (*first).clone(), extras: values.get(first.id).to_vec(), details: card_details.of(first.id), dropped: 0 };
    for other in rest {
        folded.fold(other, values.get(other.id), &card_details.of(other.id));
    }
    if !folded.keep.valid() {
        return None;
    }
    Some(Merged {
        keep: folded.keep,
        extras: folded.extras,
        details: folded.details,
        removed: rest.iter().map(|c| c.id).collect(),
        dropped: folded.dropped,
    })
}

/// A name field of a card that the merge review sheet lets the person choose from.
#[allow(dead_code)] // the review sheet is not drawn yet; the model is tested
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeField {
    Name,
    Company,
    Phone,
    Email,
    Address,
    Notes,
}

/// One field where the duplicates disagree: each distinct value with the card it comes from, in card order.
#[allow(dead_code)] // the review sheet is not drawn yet; the model is tested
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeChoice {
    pub field: MergeField,
    pub options: Vec<(u64, String)>,
}

fn field_value(c: &Contact, field: MergeField) -> &str {
    match field {
        MergeField::Name => &c.name,
        MergeField::Company => &c.company,
        MergeField::Phone => &c.phone,
        MergeField::Email => &c.email,
        MergeField::Address => &c.address,
        MergeField::Notes => &c.notes,
    }
}

/// The fields of the cards `ids` that differ, for the review sheet. A field where every card is blank, or where all
/// the cards agree, is not offered. The first option is the first card's value (the card that stays).
#[allow(dead_code)] // the review sheet is not drawn yet; the model is tested
pub fn review(contacts: &[Contact], ids: &[u64]) -> Vec<MergeChoice> {
    let cards: Vec<&Contact> = ids.iter().filter_map(|id| contacts.iter().find(|c| c.id == *id)).collect();
    [MergeField::Name, MergeField::Company, MergeField::Phone, MergeField::Email, MergeField::Address, MergeField::Notes]
        .into_iter()
        .filter_map(|field| {
            let mut options: Vec<(u64, String)> = Vec::new();
            for c in &cards {
                let value = field_value(c, field).trim();
                if !value.is_empty() && !options.iter().any(|(_, v)| v == value) {
                    options.push((c.id, value.to_string()));
                }
            }
            (options.len() > 1).then_some(MergeChoice { field, options })
        })
        .collect()
}

/// Folds the cards `ids` like `merge`, but first takes each picked field from the card the person chose: `picks` pairs
/// a field with the id of its card. A phone takes its label from the same card. `None` when a pick names a card that is
/// not in `ids`, or the merge itself is refused.
#[allow(dead_code)] // the review sheet is not drawn yet; the model is tested
pub fn merge_with(contacts: &[Contact], values: &Extras, card_details: &Details, ids: &[u64], picks: &[(MergeField, u64)]) -> Option<Merged> {
    if picks.iter().any(|(_, from)| !ids.contains(from)) {
        return None;
    }
    let mut list = contacts.to_vec();
    let keep_at = list.iter().position(|c| Some(&c.id) == ids.first())?;
    for (field, from) in picks {
        let src = list.iter().find(|c| c.id == *from)?.clone();
        let keep = &mut list[keep_at];
        match field {
            MergeField::Name => keep.name = src.name,
            MergeField::Company => keep.company = src.company,
            MergeField::Phone => {
                keep.phone = src.phone;
                keep.labels[0] = src.labels[0];
            }
            MergeField::Email => {
                keep.email = src.email;
                keep.labels[1] = src.labels[1];
            }
            MergeField::Address => keep.address = src.address,
            MergeField::Notes => keep.notes = src.notes,
        }
    }
    merge(&list, values, card_details, ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::details::Detail;

    #[test]
    fn review_offers_only_the_fields_where_the_duplicates_disagree() {
        let mut a = c(1, "Anna As", "555 0100", "");
        a.company = "Acme".into();
        let mut b = c(2, "Anna A.", "555 0100", "");
        b.company = "Acme".into();
        b.address = "Gata 1".into();
        let choices = review(&[a, b], &[1, 2]);
        let fields: Vec<MergeField> = choices.iter().map(|ch| ch.field).collect();
        assert_eq!(fields, vec![MergeField::Name], "the phone and company agree; the address has one value");
        assert_eq!(choices[0].options, vec![(1, "Anna As".to_string()), (2, "Anna A.".to_string())]);
    }

    #[test]
    fn a_pick_takes_the_field_and_its_label_from_the_chosen_card() {
        let mut a = c(1, "Anna As", "555 0100", "");
        a.labels = [0, 1, 2];
        let mut b = c(2, "Anna As", "555 0200", "");
        b.labels = [2, 1, 2];
        let picks = [(MergeField::Phone, 2)];
        let merged = merge_with(&[a, b], &Extras::default(), &Details::default(), &[1, 2], &picks).unwrap();
        assert_eq!(merged.keep.phone, "555 0200");
        assert_eq!(merged.keep.labels[0], 2, "the chosen card's phone label comes with it");
        assert_eq!(merged.removed, vec![2]);
    }

    #[test]
    fn a_pick_from_a_card_outside_the_merge_is_refused() {
        let a = c(1, "Anna As", "", "");
        let b = c(2, "Anna As", "", "");
        let outside = c(3, "Bo Berg", "", "");
        assert!(merge_with(&[a, b, outside], &Extras::default(), &Details::default(), &[1, 2], &[(MergeField::Name, 3)]).is_none());
    }

    #[test]
    fn without_picks_the_merge_is_the_plain_fold() {
        let a = c(1, "Anna As", "555 0100", "");
        let b = c(2, "A. As", "555 0100", "");
        let list = [a, b];
        let plain = merge(&list, &Extras::default(), &Details::default(), &[1, 2]);
        let picked = merge_with(&list, &Extras::default(), &Details::default(), &[1, 2], &[]);
        assert_eq!(plain, picked);
    }

    fn c(id: u64, name: &str, phone: &str, email: &str) -> Contact {
        Contact { id, name: name.into(), phone: phone.into(), email: email.into(), ..Default::default() }
    }

    #[test]
    fn the_same_email_is_a_duplicate_whatever_the_name() {
        let a = c(1, "Anna Lindqvist", "", "Anna@Example.org");
        let b = c(2, "A. Lindqvist", "", "anna@example.org");
        assert!(is_duplicate(&a, &b));
    }

    #[test]
    fn phone_numbers_match_on_their_last_seven_digits() {
        let a = c(1, "Anna", "+354 555 1234", "");
        let b = c(2, "Bo", "5551234", "");
        assert!(is_duplicate(&a, &b));
        assert!(!is_duplicate(&a, &c(3, "Bo", "555 9999", "")));
    }

    #[test]
    fn the_same_name_is_a_duplicate_unless_phone_or_email_conflict() {
        assert!(is_duplicate(&c(1, "Ólafur Ólafsson", "", ""), &c(2, "olafur  ólafsson", "", "")));
        assert!(!is_duplicate(&c(1, "Anna As", "111 1111", ""), &c(2, "Anna As", "222 2222", "")));
        assert!(!is_duplicate(&c(1, "Anna As", "", "a@x.is"), &c(2, "Anna As", "", "b@x.is")));
        assert!(is_duplicate(&c(1, "Anna As", "111 1111", ""), &c(2, "Anna As", "", "")));
    }

    #[test]
    fn different_people_are_not_duplicates() {
        assert!(!is_duplicate(&c(1, "Anna As", "", ""), &c(2, "Bo Berg", "", "")));
        assert!(!is_duplicate(&c(1, "", "", ""), &c(2, "", "", "")), "empty contacts share nothing");
    }

    #[test]
    fn groups_join_chains_and_leave_single_contacts_out() {
        let list = [
            c(1, "Anna As", "111 1111", ""),
            c(2, "Bo Berg", "111 1111", ""),
            c(3, "Cara Cole", "", "cara@x.is"),
            c(4, "C. Cole", "", "CARA@x.is"),
            c(5, "Dan Dahl", "", ""),
        ];
        assert_eq!(groups(&list), vec![vec![1, 2], vec![3, 4]]);
        assert_eq!(duplicate_ids(&list), BTreeSet::from([1, 2, 3, 4]));
        assert!(has_duplicate(&list, &c(9, "Dan Dahl", "", "")));
        assert!(!has_duplicate(&list, &c(9, "Eva Eld", "", "")));
    }

    #[test]
    fn a_merge_keeps_the_first_card_and_fills_what_it_lacks() {
        let mut first = c(1, "Anna As", "", "anna@x.is");
        first.labels = [0, 1, 2];
        let mut second = c(2, "A. As", "555 0100", "");
        second.company = "Acme".into();
        second.favorite = true;
        second.address = "Gata 1".into();
        second.labels = [2, 0, 1];
        let merged = merge(&[first, second], &Extras::default(), &Details::default(), &[1, 2]).unwrap();
        assert_eq!(merged.removed, vec![2]);
        let keep = &merged.keep;
        assert_eq!((keep.id, keep.name.as_str()), (1, "Anna As"), "the first card keeps its name");
        assert_eq!((keep.phone.as_str(), keep.email.as_str()), ("555 0100", "anna@x.is"));
        assert_eq!((keep.company.as_str(), keep.address.as_str()), ("Acme", "Gata 1"));
        assert!(keep.favorite);
        assert_eq!(keep.labels, [2, 1, 1], "the phone and address bring their labels; the email stays home");
        assert!(merged.extras.is_empty());
    }

    #[test]
    fn different_values_become_extra_values_and_the_same_ones_do_not() {
        let first = c(1, "Anna As", "+354 555 0100", "anna@x.is");
        let mut second = c(2, "Anna As", "5550100", "ANNA@x.is");
        second.address = "Gata 1".into();
        let mut third = c(3, "Anna As", "555 0200", "anna@work.is");
        third.labels = [2, 1, 2];
        let mut values = Extras::default();
        assert!(values.add(3, Extra { kind: ValueKind::Phone, label: "home".into(), value: "555 0300".into() }));
        let merged = merge(&[first, second, third], &values, &Details::default(), &[1, 2, 3]).unwrap();
        assert_eq!(merged.removed, vec![2, 3]);
        let found: Vec<(ValueKind, &str, &str)> = merged.extras.iter().map(|e| (e.kind, e.label.as_str(), e.value.as_str())).collect();
        assert_eq!(
            found,
            vec![
                (ValueKind::Phone, "work", "555 0200"),
                (ValueKind::Email, "work", "anna@work.is"),
                (ValueKind::Phone, "home", "555 0300"),
            ],
            "the same phone and email are not added again"
        );
        assert_eq!(merged.keep.address, "Gata 1", "the empty address is filled from the second card");
    }

    #[test]
    fn notes_are_joined_once_and_card_fields_are_taken_over() {
        let mut first = c(1, "Anna As", "", "");
        first.notes = "Met at a fair".into();
        let mut second = c(2, "Anna As", "", "");
        second.notes = "Likes tea".into();
        let mut third = c(3, "Anna As", "", "");
        third.notes = "Met at a fair".into();
        let mut one = CardDetails::default();
        one.set(Detail::Nickname, "Annie");
        let mut two = CardDetails::default();
        two.set(Detail::Nickname, "An");
        two.set(Detail::JobTitle, "Engineer");
        two.company = true;
        let mut details = Details::default();
        details.set(1, one);
        details.set(2, two);
        let merged = merge(&[first, second, third], &Extras::default(), &details, &[1, 2, 3]).unwrap();
        assert_eq!(merged.keep.notes, "Met at a fair\n\nLikes tea", "a note the card already has is not repeated");
        assert_eq!(merged.details.get(Detail::Nickname), "Annie", "the first card's field wins");
        assert_eq!(merged.details.get(Detail::JobTitle), "Engineer");
        assert!(merged.details.company);
    }

    #[test]
    fn values_beyond_the_limits_are_counted_not_lost_silently() {
        let mut all = vec![c(1, "Anna As", "100", "")];
        for i in 0..8u64 {
            all.push(c(10 + i, "Anna As", &format!("20{i}"), ""));
        }
        let ids: Vec<u64> = all.iter().map(|card| card.id).collect();
        let merged = merge(&all, &Extras::default(), &Details::default(), &ids).unwrap();
        assert_eq!(merged.extras.len(), MAX_EXTRAS);
        assert_eq!(merged.dropped, 2);
        assert_eq!(merged.removed.len(), 8);
    }

    #[test]
    fn a_merge_needs_two_cards_that_exist() {
        let list = [c(1, "Anna As", "", ""), c(2, "Bo Berg", "", "")];
        assert!(merge(&list, &Extras::default(), &Details::default(), &[1]).is_none());
        assert!(merge(&list, &Extras::default(), &Details::default(), &[1, 9]).is_none());
        assert!(merge(&list, &Extras::default(), &Details::default(), &[]).is_none());
        let merged = merge(&list, &Extras::default(), &Details::default(), &[2, 1]).unwrap();
        assert_eq!(merged.keep.id, 2, "the first id given is the card that stays");
    }
}
