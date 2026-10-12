//! Wi-Fi secrets for apps, kept as generic keychain items (see `keychain`): the shared password of a personal
//! network, the user name and password of an enterprise (802.1X) network, and the list of enterprise user names.
//!
//! This module only builds the item names and the encoded values and checks them. The caller stores them with
//! [`crate::keychain::add`] or [`crate::keychain::update`]. Each item is keyed by the service [`SERVICE`] and an
//! account name made of a kind and the network name in hex, so names with spaces or slashes stay safe. Every
//! account name is at most 255 bytes, the keychain limit.

/// The keychain service that holds every Wi-Fi secret.
pub const SERVICE: &str = "bifrost.wifi";
/// The longest network name, in bytes.
pub const MAX_SSID: usize = 32;
/// The account name of the enterprise user name list.
pub const IDENTITY_LIST: &str = "eap-identities";

/// Why a Wi-Fi secret was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// the network name is empty or longer than [`MAX_SSID`] bytes
    SsidLength,
    /// a passphrase is shorter than 8 or longer than 63 characters, and is not a 64-digit hex key
    PasswordLength,
    /// a passphrase has a byte outside printable ASCII
    PasswordCharacters,
    /// a user name is empty, longer than 64 bytes, or has a control character
    UsernameInvalid,
    /// an enterprise password is empty, longer than 128 bytes, or has a control character
    PasswordInvalid,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn check_ssid(ssid: &str) -> Result<(), Error> {
    if ssid.is_empty() || ssid.len() > MAX_SSID {
        Err(Error::SsidLength)
    } else {
        Ok(())
    }
}

/// The account of a personal network's passphrase: `password:` and the network name in hex.
pub fn password_account(ssid: &str) -> Result<String, Error> {
    check_ssid(ssid)?;
    Ok(format!("password:{}", hex(ssid.as_bytes())))
}

/// The account of an enterprise network's user name and password: `eap:` and the network name in hex.
pub fn eap_account(ssid: &str) -> Result<String, Error> {
    check_ssid(ssid)?;
    Ok(format!("eap:{}", hex(ssid.as_bytes())))
}

/// Checks a passphrase. It is 8 to 63 printable ASCII characters, or a raw key of exactly 64 hex digits.
pub fn check_passphrase(pass: &str) -> Result<(), Error> {
    let b = pass.as_bytes();
    if b.len() == 64 && b.iter().all(|c| c.is_ascii_hexdigit()) {
        return Ok(());
    }
    if b.len() < 8 || b.len() > 63 {
        return Err(Error::PasswordLength);
    }
    if b.iter().all(|c| (0x20..=0x7e).contains(c)) {
        Ok(())
    } else {
        Err(Error::PasswordCharacters)
    }
}

/// The keychain item for a personal network: the account name and the passphrase bytes.
pub fn password_item(ssid: &str, pass: &str) -> Result<(String, Vec<u8>), Error> {
    let account = password_account(ssid)?;
    check_passphrase(pass)?;
    Ok((account, pass.as_bytes().to_vec()))
}

fn check_username(user: &str) -> Result<(), Error> {
    if user.is_empty() || user.len() > 64 || user.chars().any(|c| c.is_control()) {
        Err(Error::UsernameInvalid)
    } else {
        Ok(())
    }
}

/// The keychain item for an enterprise network. The value is the user name, a line feed, then the password.
/// The user name cannot hold a line feed, so the first one always ends it.
pub fn eap_item(ssid: &str, user: &str, pass: &str) -> Result<(String, Vec<u8>), Error> {
    let account = eap_account(ssid)?;
    check_username(user)?;
    if pass.is_empty() || pass.len() > 128 || pass.chars().any(|c| c.is_control()) {
        return Err(Error::PasswordInvalid);
    }
    Ok((account, format!("{}\n{}", user, pass).into_bytes()))
}

/// Splits an enterprise item's value into its user name and password. `None` when there is no line feed or the
/// bytes are not UTF-8.
pub fn parse_eap(value: &[u8]) -> Option<(String, String)> {
    let text = std::str::from_utf8(value).ok()?;
    let (user, pass) = text.split_once('\n')?;
    Some((user.to_string(), pass.to_string()))
}

/// The user names in the enterprise list value, in the order they were added. Empty entries are skipped.
pub fn identities(value: &[u8]) -> Vec<String> {
    std::str::from_utf8(value)
        .unwrap_or("")
        .split('\n')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// The enterprise list value with `user` added at the end, unless it is already there.
pub fn add_identity(value: &[u8], user: &str) -> Result<Vec<u8>, Error> {
    check_username(user)?;
    let mut list = identities(value);
    if !list.iter().any(|u| u.as_str() == user) {
        list.push(user.to_string());
    }
    Ok(list.join("\n").into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_hex_the_network_name() {
        assert_eq!(password_account("Home").unwrap(), "password:486f6d65");
        assert_eq!(eap_account("Home").unwrap(), "eap:486f6d65");
        assert_eq!(password_account("A B").unwrap(), "password:412042");
    }

    #[test]
    fn network_names_are_one_to_thirty_two_bytes() {
        assert_eq!(password_account(""), Err(Error::SsidLength));
        assert_eq!(eap_account(&"x".repeat(33)), Err(Error::SsidLength));
        assert!(password_account(&"x".repeat(32)).is_ok());
        assert_eq!(password_account(&"x".repeat(32)).unwrap().len(), 9 + 64);
    }

    #[test]
    fn passphrase_rules_are_eight_to_sixty_three_printable_or_a_hex_key() {
        assert_eq!(check_passphrase("short"), Err(Error::PasswordLength));
        assert_eq!(check_passphrase("12345678"), Ok(()));
        assert_eq!(check_passphrase(&"a".repeat(63)), Ok(()));
        assert_eq!(check_passphrase(&"g".repeat(64)), Err(Error::PasswordLength));
        assert_eq!(check_passphrase(&"f".repeat(64)), Ok(()));
        assert_eq!(check_passphrase("pass\tword1"), Err(Error::PasswordCharacters));
    }

    #[test]
    fn password_item_pairs_the_account_with_the_passphrase_bytes() {
        let (account, value) = password_item("Home", "correct horse").unwrap();
        assert_eq!(account, "password:486f6d65");
        assert_eq!(value, b"correct horse".to_vec());
        assert_eq!(password_item("Home", "short"), Err(Error::PasswordLength));
    }

    #[test]
    fn eap_item_round_trips_user_and_password() {
        let (account, value) = eap_item("Corp", "olibuijr", "secret").unwrap();
        assert_eq!(account, "eap:436f7270");
        assert_eq!(value, b"olibuijr\nsecret".to_vec());
        assert_eq!(parse_eap(&value), Some(("olibuijr".into(), "secret".into())));
        assert_eq!(parse_eap(b"no separator"), None);
        assert_eq!(eap_item("Corp", "bad\nuser", "x"), Err(Error::UsernameInvalid));
        assert_eq!(eap_item("Corp", "ok", ""), Err(Error::PasswordInvalid));
    }

    #[test]
    fn identity_list_adds_each_user_name_once() {
        let a = add_identity(b"", "anna").unwrap();
        assert_eq!(a, b"anna".to_vec());
        let b = add_identity(&a, "bjorn").unwrap();
        assert_eq!(b, b"anna\nbjorn".to_vec());
        assert_eq!(add_identity(&b, "anna").unwrap(), b);
        assert_eq!(identities(&b), vec!["anna".to_string(), "bjorn".to_string()]);
        assert_eq!(add_identity(b"", ""), Err(Error::UsernameInvalid));
    }
}
