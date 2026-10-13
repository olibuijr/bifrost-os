//! Duplicate detection for contacts. Two contacts are duplicates when they share an email address, or the same phone
//! number (compared by its last seven digits, so "+354 555 1234" and "5551234" match), or the same name with no
//! conflicting phone or email. Names match ignoring case and accents. Pure std.

use crate::extras::{Extra, ValueKind};
use crate::names;
use springboard::shell::contactstore::Contact;
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

/// Folds `others` into `keep`. An empty field of the kept card takes the other card's value; a value that differs
/// is returned as an extra value under the label the other card gave it, so nothing is lost. Notes are joined and a
/// favourite stays a favourite. Values that repeat what the kept card already holds are dropped.
pub fn merge(keep: &Contact, others: &[Contact]) -> (Contact, Vec<Extra>) {
    let mut card = keep.clone();
    let mut spare: Vec<Extra> = Vec::new();
    for other in others {
        if card.name.trim().is_empty() {
            card.name = other.name.clone();
        }
        if card.company.trim().is_empty() {
            card.company = other.company.clone();
        }
        card.favorite |= other.favorite;
        let notes = other.notes.trim();
        if !notes.is_empty() && !card.notes.contains(notes) {
            card.notes = if card.notes.trim().is_empty() {
                notes.to_string()
            } else {
                format!("{}\n{notes}", card.notes.trim_end())
            };
        }
        let kinds = [ValueKind::Phone, ValueKind::Email, ValueKind::Address];
        for (k, kind) in kinds.into_iter().enumerate() {
            let value = [&other.phone, &other.email, &other.address][k].trim().to_string();
            if value.is_empty() {
                continue;
            }
            let own = match k {
                0 => &mut card.phone,
                1 => &mut card.email,
                _ => &mut card.address,
            };
            if own.trim().is_empty() {
                *own = value;
                card.labels[k] = other.labels[k];
            } else if own.trim().to_lowercase() != value.to_lowercase()
                && !spare.iter().any(|e| e.kind == kind && e.value.to_lowercase() == value.to_lowercase())
            {
                let labels = springboard::shell::contactstore::LABELS[k];
                let label = labels[other.labels[k] as usize % labels.len()].to_string();
                spare.push(Extra { kind, label, value });
            }
        }
    }
    (card, spare)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_fills_gaps_and_keeps_differing_values_as_extras() {
        let keep = c(1, "Anna As", "111 1111", "");
        let mut other = c(2, "Anna As", "222 2222", "anna@x.is");
        other.notes = "met at fair".into();
        other.favorite = true;
        let (card, spare) = merge(&keep, &[other]);
        assert_eq!(card.id, 1);
        assert_eq!(card.phone, "111 1111");
        assert_eq!(card.email, "anna@x.is");
        assert_eq!(card.notes, "met at fair");
        assert!(card.favorite);
        assert_eq!(spare.len(), 1);
        assert_eq!(spare[0].value, "222 2222");
        assert!(spare[0].well_formed());
    }

    #[test]
    fn merge_drops_repeated_values() {
        let keep = c(1, "Anna", "111", "A@x.is");
        let (_, spare) = merge(&keep, &[c(2, "Anna", "111", "a@x.is"), c(3, "Anna", "111", "")]);
        assert!(spare.is_empty());
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
}
