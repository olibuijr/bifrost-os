//! Matrix accounts live in the shared passwords group, next to the website logins, so Settings > Passwords lists them
//! and every app reads the same secret. An account is one login item: its service is `matrix:` followed by the
//! homeserver base address (`matrix:https://matrix.org`), its account is the Matrix user id, and its value is the
//! password. The access token is not stored; each start signs in again and makes a device.

use bifrost_app::keychain_groups as keychain;

/// The service prefix of a Matrix login. A website login never starts with it, because a site name has no colon.
pub const SERVICE_PREFIX: &str = "matrix:";

/// A saved Matrix account: the homeserver base address and the user id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub homeserver: String,
    pub user: String,
}

/// The service a login for `homeserver` is stored under.
pub fn service(homeserver: &str) -> String {
    format!("{SERVICE_PREFIX}{homeserver}")
}

/// The account a passwords item is, when it is a Matrix login (`None` for every other item).
pub fn from_item(service_name: &str, account: &str) -> Option<Account> {
    let homeserver = service_name.strip_prefix(SERVICE_PREFIX)?;
    if !homeserver.starts_with("https://") || account.trim().is_empty() {
        return None;
    }
    Some(Account { homeserver: homeserver.to_string(), user: account.to_string() })
}

/// Every Matrix account in a passwords listing, in the listing's order.
pub fn from_rows(rows: &[keychain::Row]) -> Vec<Account> {
    rows.iter().filter_map(|row| from_item(&row.service, &row.account)).collect()
}

/// The account a login is stored under once the server has signed it in: the full user id the login response gives
/// (`@alice:example.org`), whatever was typed (a bare `alice` or a different case). The server's id is always used when
/// it has one; the typed name is kept only when the response gave none.
pub fn stored_user(typed: &str, server_user_id: &str) -> String {
    let server = server_user_id.trim();
    if server.is_empty() {
        typed.trim().to_string()
    } else {
        server.to_string()
    }
}

/// The name a login was saved under before, when it differs from the stored one. Its items (the Passwords login and the
/// crypto items) are moved to the stored name and removed under this one. `None` when the names agree.
pub fn moved_from(typed: &str, stored: &str) -> Option<String> {
    let typed = typed.trim();
    (typed != stored && !typed.is_empty()).then(|| typed.to_string())
}

/// Whether `user` is a full Matrix user id (`@localpart:server`).
pub fn is_full_user_id(user: &str) -> bool {
    user.strip_prefix('@').and_then(|rest| rest.split_once(':')).is_some_and(|(local, server)| !local.is_empty() && !server.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(service: &str, account: &str) -> keychain::Row {
        keychain::Row { service: service.into(), account: account.into(), modified: 0 }
    }

    #[test]
    fn a_login_round_trips_through_its_service_name() {
        let s = service("https://matrix.example.org");
        assert_eq!(s, "matrix:https://matrix.example.org");
        assert_eq!(
            from_item(&s, "@alice:example.org"),
            Some(Account { homeserver: "https://matrix.example.org".into(), user: "@alice:example.org".into() })
        );
    }

    #[test]
    fn website_logins_and_damaged_items_are_not_matrix_accounts() {
        assert_eq!(from_item("example.com", "alice"), None);
        assert_eq!(from_item("matrix:http://plain.example.org", "@a:b"), None, "only https homeservers");
        assert_eq!(from_item("matrix:https://matrix.org", "  "), None, "an item needs a user");
    }

    #[test]
    fn a_listing_keeps_only_the_matrix_accounts_in_order() {
        let rows = vec![
            row("example.com", "bob"),
            row("matrix:https://a.example", "@one:a.example"),
            row("note:Shopping", "secure-note"),
            row("matrix:https://b.example", "@two:b.example"),
        ];
        let accounts = from_rows(&rows);
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].user, "@one:a.example");
        assert_eq!(accounts[1].homeserver, "https://b.example");
    }

    #[test]
    fn the_stored_name_is_the_full_id_the_server_returned() {
        assert_eq!(stored_user("alice", "@alice:example.org"), "@alice:example.org");
        assert_eq!(stored_user("@Alice:example.org", "@alice:example.org"), "@alice:example.org");
        assert_eq!(stored_user("alice", "  "), "alice", "no server id keeps the typed name");
    }

    #[test]
    fn a_bare_localpart_login_is_moved_to_the_full_id() {
        let stored = stored_user("alice", "@alice:example.org");
        assert_eq!(moved_from("alice", &stored).as_deref(), Some("alice"), "the bare item is moved away");
        assert_eq!(moved_from("@alice:example.org", &stored), None, "a full id needs no move");
        assert_eq!(moved_from("  ", &stored), None);
    }

    #[test]
    fn only_full_matrix_ids_count_as_normalised() {
        assert!(is_full_user_id("@alice:example.org"));
        assert!(!is_full_user_id("alice"));
        assert!(!is_full_user_id("@alice"));
        assert!(!is_full_user_id("@:example.org"));
    }
}
