//! Merging the hits several providers give for one Search query. A provider is the shell's own index of an app's items, or
//! an app that answers queries itself. Each provider keeps the order it gave its hits. The merged list takes them
//! round-robin, so one provider with many matches cannot push every other provider's results off the page.
//!
//! Pure: the caller collects the batches and shows the list.

use std::collections::HashSet;

/// One result: the app it belongs to, the id that app gave it, and its title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub app: String,
    pub id: String,
    pub title: String,
}

/// Joins the batches round-robin. Each batch contributes at most `per_provider` hits, and the list stops at `limit`.
/// A hit that two batches both give (the same app and id) is listed once, where it first comes up.
pub fn merge(batches: &[Vec<Hit>], per_provider: usize, limit: usize) -> Vec<Hit> {
    let queues: Vec<&[Hit]> = batches.iter().map(|b| &b[..b.len().min(per_provider)]).collect();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::new();
    let mut round = 0;
    loop {
        let mut any = false;
        for queue in &queues {
            let Some(hit) = queue.get(round) else { continue };
            any = true;
            if out.len() >= limit {
                return out;
            }
            if seen.insert((hit.app.clone(), hit.id.clone())) {
                out.push(hit.clone());
            }
        }
        if !any {
            return out;
        }
        round += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(app: &str, id: &str) -> Hit {
        Hit { app: app.into(), id: id.into(), title: format!("{app} {id}") }
    }

    fn ids(list: &[Hit]) -> Vec<String> {
        list.iter().map(|h| format!("{}/{}", h.app, h.id)).collect()
    }

    #[test]
    fn the_providers_take_turns_so_one_with_many_hits_cannot_fill_the_list() {
        let many = vec![hit("notes", "1"), hit("notes", "2"), hit("notes", "3")];
        let one = vec![hit("mail", "9")];
        assert_eq!(ids(&merge(&[many, one], 10, 10)), vec!["notes/1", "mail/9", "notes/2", "notes/3"]);
    }

    #[test]
    fn each_provider_contributes_at_most_its_cap() {
        let many = vec![hit("notes", "1"), hit("notes", "2"), hit("notes", "3"), hit("notes", "4")];
        assert_eq!(ids(&merge(&[many], 2, 10)), vec!["notes/1", "notes/2"]);
    }

    #[test]
    fn the_list_stops_at_the_limit() {
        let many = vec![hit("notes", "1"), hit("notes", "2")];
        let one = vec![hit("mail", "9")];
        assert_eq!(ids(&merge(&[many, one], 10, 2)), vec!["notes/1", "mail/9"]);
        assert!(merge(&[vec![hit("notes", "1")]], 10, 0).is_empty());
    }

    #[test]
    fn a_hit_given_twice_is_listed_once_and_the_same_id_in_two_apps_is_two_hits() {
        let first = vec![hit("notes", "1")];
        let second = vec![hit("notes", "1"), hit("mail", "1")];
        assert_eq!(ids(&merge(&[first, second], 10, 10)), vec!["notes/1", "mail/1"]);
    }

    #[test]
    fn no_batches_or_empty_batches_give_no_hits() {
        assert!(merge(&[], 10, 10).is_empty());
        assert!(merge(&[Vec::new(), Vec::new()], 10, 10).is_empty());
    }
}
