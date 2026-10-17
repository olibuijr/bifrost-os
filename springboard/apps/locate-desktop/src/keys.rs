//! The account's Find My key pair, kept in the BifrOSt keychain under this app. The private key never touches the app's
//! data folder. Keychain calls are asynchronous: a request id is held until its reply arrives in `App::keychain`, and
//! `on_reply` tells the window what changed. The first device that turns Share My Location on creates the pair.

use bifrost_app::controls::x25519::PrivateKey;
use bifrost_app::keychain::{self, Error, Reply};
use bifrost_app::Ctx;

/// The keychain service and account of the account private key.
pub const SERVICE: &str = "is.olibuijr.locate.desktop";
pub const ACCOUNT_KEY: &str = "findmy-account-key";

/// What a keychain reply changed, for the window to act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    /// The account private key was read from the keychain.
    Loaded,
    /// No usable account key is stored. This device has to create one or be approved.
    Missing,
    /// A new account key pair was stored; its public key is ready to publish.
    Stored,
}

#[derive(Default)]
pub struct Keys {
    private: Option<[u8; 32]>,
    get_request: Option<u32>,
    add_request: Option<u32>,
    creating: Option<[u8; 32]>,
}

impl Keys {
    /// The account private key, once it is loaded or stored.
    pub fn private(&self) -> Option<&[u8; 32]> {
        self.private.as_ref()
    }

    /// The public key of the held private key, the one a device publishes to the cloud.
    pub fn public(&self) -> Option<[u8; 32]> {
        let raw = self.private?;
        Some(PrivateKey::from_raw(&raw).ok()?.public_key().raw())
    }

    /// Whether a keychain call is still waiting for its reply.
    pub fn busy(&self) -> bool {
        self.get_request.is_some() || self.add_request.is_some()
    }

    /// Reads the account key from the keychain. Called once at start.
    pub fn load(&mut self, ctx: &Ctx) {
        if self.get_request.is_none() && self.private.is_none() {
            self.get_request = Some(keychain::get(ctx, SERVICE, ACCOUNT_KEY));
        }
    }

    /// Creates the account key pair from 32 random bytes when none is held and no store is pending. Returns the public
    /// key to publish; the private key is kept only once the keychain confirms the store.
    pub fn create(&mut self, ctx: &Ctx, random: &[u8]) -> Option<[u8; 32]> {
        if self.private.is_some() || self.busy() {
            return None;
        }
        let raw: [u8; 32] = random.try_into().ok()?;
        let public = PrivateKey::from_raw(&raw).ok()?.public_key().raw();
        self.creating = Some(raw);
        self.add_request = Some(keychain::add(ctx, SERVICE, ACCOUNT_KEY, &raw));
        Some(public)
    }

    /// Applies one keychain reply. Replies for other requests are ignored.
    pub fn on_reply(&mut self, reply: &Reply) -> Option<KeyEvent> {
        if self.get_request == Some(reply.request) {
            self.get_request = None;
            return Some(match &reply.result {
                Ok(bytes) => match <[u8; 32]>::try_from(bytes.as_slice()) {
                    Ok(key) => {
                        self.private = Some(key);
                        KeyEvent::Loaded
                    }
                    Err(_) => KeyEvent::Missing,
                },
                Err(_) => KeyEvent::Missing,
            });
        }
        if self.add_request == Some(reply.request) {
            self.add_request = None;
            return Some(match (reply.result.is_ok(), self.creating.take()) {
                (true, Some(raw)) => {
                    self.private = Some(raw);
                    KeyEvent::Stored
                }
                _ => KeyEvent::Missing,
            });
        }
        None
    }
}

/// True when a keychain error means "no item", the normal state before the first device enables sharing.
pub fn is_missing(e: &Error) -> bool {
    matches!(e, Error::ItemNotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(request: u32, result: Result<Vec<u8>, Error>) -> Reply {
        Reply { request, result }
    }

    #[test]
    fn a_stored_key_is_loaded_from_its_reply() {
        let mut k = Keys::default();
        k.get_request = Some(7);
        assert_eq!(k.on_reply(&reply(7, Ok(vec![9; 32]))), Some(KeyEvent::Loaded));
        assert_eq!(k.private(), Some(&[9u8; 32]));
        assert!(!k.busy());
    }

    #[test]
    fn no_item_or_a_wrong_length_means_no_key() {
        let mut k = Keys::default();
        k.get_request = Some(1);
        assert_eq!(k.on_reply(&reply(1, Err(Error::ItemNotFound))), Some(KeyEvent::Missing));
        assert!(k.private().is_none());
        k.get_request = Some(2);
        assert_eq!(k.on_reply(&reply(2, Ok(vec![1; 5]))), Some(KeyEvent::Missing));
        assert!(k.private().is_none());
    }

    #[test]
    fn a_reply_for_another_request_is_ignored() {
        let mut k = Keys::default();
        assert_eq!(k.on_reply(&reply(99, Ok(vec![1; 32]))), None);
    }

    #[test]
    fn a_new_key_is_kept_only_once_the_store_is_confirmed() {
        let mut k = Keys::default();
        k.creating = Some([5; 32]);
        k.add_request = Some(3);
        assert_eq!(k.on_reply(&reply(3, Err(Error::DuplicateItem))), Some(KeyEvent::Missing));
        assert!(k.private().is_none(), "a failed store keeps no key");
        k.creating = Some([5; 32]);
        k.add_request = Some(4);
        assert_eq!(k.on_reply(&reply(4, Ok(vec![]))), Some(KeyEvent::Stored));
        assert_eq!(k.private(), Some(&[5u8; 32]));
    }

    #[test]
    fn only_a_missing_item_counts_as_missing() {
        assert!(is_missing(&Error::ItemNotFound));
        assert!(!is_missing(&Error::DuplicateItem));
    }
}
