//! Rows for a route picker. Local outputs come first in the order given. Discovered receivers follow, sorted by name
//! ignoring case. Each row says whether its route is active, using the app's `RouteAuth`.

use crate::route_auth::RouteAuth;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Destination {
    pub id: String,
    pub name: String,
    /// True for a built-in output, false for a discovered receiver.
    pub local: bool,
    pub active: bool,
}

/// Builds the picker rows. A row with an empty identifier, or an identifier already listed, is skipped; the first
/// occurrence wins, so a local output is never repeated as a receiver.
pub fn destinations(local: &[(&str, &str)], found: &[(&str, &str)], auth: &RouteAuth) -> Vec<Destination> {
    let mut rows = Vec::new();
    for &(id, name) in local {
        add(&mut rows, id, name, true, auth);
    }
    let local_count = rows.len();
    for &(id, name) in found {
        add(&mut rows, id, name, false, auth);
    }
    rows[local_count..].sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    rows
}

fn add(rows: &mut Vec<Destination>, id: &str, name: &str, local: bool, auth: &RouteAuth) {
    let id = id.trim();
    if id.is_empty() || rows.iter().any(|r| r.id == id) {
        return;
    }
    rows.push(Destination {
        id: id.to_string(),
        name: name.trim().to_string(),
        local,
        active: auth.is_active(id),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(rows: &[Destination]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    #[test]
    fn local_outputs_come_first_in_their_given_order() {
        let auth = RouteAuth::new();
        let rows = destinations(&[("speaker", "Speaker"), ("headphones", "Headphones")], &[], &auth);
        assert_eq!(names(&rows), vec!["Speaker", "Headphones"]);
        assert!(rows.iter().all(|r| r.local));
    }

    #[test]
    fn found_receivers_are_sorted_ignoring_case_after_the_locals() {
        let auth = RouteAuth::new();
        let found = [("r2", "bedroom"), ("r1", "Attic"), ("r3", "Kitchen")];
        let rows = destinations(&[("speaker", "Speaker")], &found, &auth);
        assert_eq!(names(&rows), vec!["Speaker", "Attic", "bedroom", "Kitchen"]);
        assert!(!rows[1].local);
    }

    #[test]
    fn a_receiver_with_a_local_id_or_a_repeated_id_is_skipped() {
        let auth = RouteAuth::new();
        let found = [("speaker", "Speaker copy"), ("r1", "Attic"), ("r1", "Attic again"), ("  ", "Blank")];
        let rows = destinations(&[("speaker", "Speaker")], &found, &auth);
        assert_eq!(names(&rows), vec!["Speaker", "Attic"]);
    }

    #[test]
    fn active_state_comes_from_the_authorization_model() {
        let mut auth = RouteAuth::new();
        auth.authorize("r1");
        auth.authorize("r2");
        auth.set_active("r2", true).unwrap();
        let rows = destinations(&[], &[("r1", "Attic"), ("r2", "Kitchen")], &auth);
        assert_eq!(rows[0].id, "r1");
        assert!(!rows[0].active);
        assert_eq!(rows[1].id, "r2");
        assert!(rows[1].active);
    }

    #[test]
    fn names_are_trimmed_and_a_blank_list_gives_no_rows() {
        let auth = RouteAuth::new();
        assert!(destinations(&[], &[], &auth).is_empty());
        let rows = destinations(&[(" tv ", "  Living room  ")], &[], &auth);
        assert_eq!(rows[0].id, "tv");
        assert_eq!(rows[0].name, "Living room");
    }
}
