//! Objects held on a cryptographic token: its private keys, public keys and certificates, and the uses each key allows.
//!
//! The card keeps the private key. This model holds only each object's label, kind and allowed uses, and hands out a
//! [`KeyRef`] for a signature or a decryption. It stores no key material, so nothing here can export a private key.
//! A key is used only while the token session is authenticated (see [`crate::token_watch::TokenSession`]); a locked or
//! logged-out session refuses every use. Object IDs start at 1 and are never reused on the same store.
use crate::token_watch::{TokenError, TokenSession};
use std::collections::BTreeMap;
use std::ops::BitOr;

/// The kind of an object on a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    PrivateKey,
    PublicKey,
    Certificate,
}

/// The uses a key allows, as a bit set. Use [`KeyUsage::contains`] to test one or more uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyUsage(u8);

impl KeyUsage {
    /// No use. Certificates and public keys carry this.
    pub const NONE: KeyUsage = KeyUsage(0);
    pub const SIGN: KeyUsage = KeyUsage(1);
    pub const DECRYPT: KeyUsage = KeyUsage(2);
    pub const DERIVE: KeyUsage = KeyUsage(4);

    /// `true` when every use in `other` is also allowed here. Every usage contains `NONE`.
    pub fn contains(self, other: KeyUsage) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_none(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for KeyUsage {
    type Output = KeyUsage;
    fn bitor(self, rhs: KeyUsage) -> KeyUsage {
        KeyUsage(self.0 | rhs.0)
    }
}

/// One object on the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenObject {
    pub object_id: u32,
    pub kind: ObjectKind,
    pub label: String,
    pub usage: KeyUsage,
}

/// A reference to a private key that the token has cleared for one use. It carries no key material.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyRef {
    pub object_id: u32,
    pub usage: KeyUsage,
}

/// The objects of one token, ordered by object ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenObjectStore {
    token_id: String,
    objects: BTreeMap<u32, TokenObject>,
    next_id: u32,
}

impl TokenObjectStore {
    pub fn new(token_id: &str) -> Self {
        TokenObjectStore { token_id: token_id.to_string(), objects: BTreeMap::new(), next_id: 1 }
    }

    pub fn token_id(&self) -> &str {
        &self.token_id
    }

    /// Adds an object and returns its ID. The label must not be empty. Only a private key may carry a use; a public
    /// key or a certificate with a use is `BadParameter`.
    pub fn add(&mut self, kind: ObjectKind, label: &str, usage: KeyUsage) -> Result<u32, TokenError> {
        if label.is_empty() {
            return Err(TokenError::BadParameter);
        }
        if kind != ObjectKind::PrivateKey && !usage.is_none() {
            return Err(TokenError::BadParameter);
        }
        let object_id = self.next_id;
        self.next_id += 1;
        self.objects.insert(object_id, TokenObject { object_id, kind, label: label.to_string(), usage });
        Ok(object_id)
    }

    pub fn get(&self, object_id: u32) -> Option<&TokenObject> {
        self.objects.get(&object_id)
    }

    /// Removes an object and returns it. An unknown ID is `ObjectNotFound`.
    pub fn remove(&mut self, object_id: u32) -> Result<TokenObject, TokenError> {
        self.objects.remove(&object_id).ok_or(TokenError::ObjectNotFound)
    }

    /// The objects of one kind whose uses include `usage`, in ID order. `KeyUsage::NONE` matches every object of
    /// the kind.
    pub fn find(&self, kind: ObjectKind, usage: KeyUsage) -> Vec<&TokenObject> {
        self.objects.values().filter(|o| o.kind == kind && o.usage.contains(usage)).collect()
    }

    /// Clears a private key for one use. The session must be for this token and authenticated. The key must be a
    /// private key that allows `usage`, which must name at least one use.
    pub fn use_key(&self, session: &TokenSession, object_id: u32, usage: KeyUsage) -> Result<KeyRef, TokenError> {
        if session.token_id() != self.token_id {
            return Err(TokenError::TokenNotFound);
        }
        session.require()?;
        if usage.is_none() {
            return Err(TokenError::BadParameter);
        }
        let object = self.objects.get(&object_id).ok_or(TokenError::ObjectNotFound)?;
        if object.kind != ObjectKind::PrivateKey || !object.usage.contains(usage) {
            return Err(TokenError::BadParameter);
        }
        Ok(KeyRef { object_id, usage })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authenticated(token: &str) -> TokenSession {
        let mut s = TokenSession::new(token);
        s.authenticate(true).unwrap();
        s
    }

    /// Keys 1 (sign), 2 (decrypt), 3 (sign and decrypt) and certificate 4.
    fn card() -> TokenObjectStore {
        let mut store = TokenObjectStore::new("card-1");
        assert_eq!(store.add(ObjectKind::PrivateKey, "sign key", KeyUsage::SIGN), Ok(1));
        assert_eq!(store.add(ObjectKind::PrivateKey, "decrypt key", KeyUsage::DECRYPT), Ok(2));
        assert_eq!(store.add(ObjectKind::PrivateKey, "both", KeyUsage::SIGN | KeyUsage::DECRYPT), Ok(3));
        assert_eq!(store.add(ObjectKind::Certificate, "leaf", KeyUsage::NONE), Ok(4));
        store
    }

    fn ids(objects: Vec<&TokenObject>) -> Vec<u32> {
        objects.iter().map(|o| o.object_id).collect()
    }

    #[test]
    fn usage_bits_combine_and_test() {
        let both = KeyUsage::SIGN | KeyUsage::DECRYPT;
        assert!(both.contains(KeyUsage::SIGN));
        assert!(both.contains(KeyUsage::DECRYPT));
        assert!(!both.contains(KeyUsage::DERIVE));
        assert!(!KeyUsage::SIGN.contains(both));
        assert!(KeyUsage::NONE.contains(KeyUsage::NONE));
    }

    #[test]
    fn add_refuses_an_empty_label_and_a_use_on_a_certificate() {
        let mut store = TokenObjectStore::new("card-1");
        assert_eq!(store.add(ObjectKind::PrivateKey, "", KeyUsage::SIGN), Err(TokenError::BadParameter));
        assert_eq!(store.add(ObjectKind::Certificate, "leaf", KeyUsage::SIGN), Err(TokenError::BadParameter));
        assert_eq!(store.add(ObjectKind::Certificate, "leaf", KeyUsage::NONE), Ok(1));
    }

    #[test]
    fn find_filters_by_kind_and_use() {
        let store = card();
        assert_eq!(ids(store.find(ObjectKind::PrivateKey, KeyUsage::SIGN)), vec![1, 3]);
        assert_eq!(ids(store.find(ObjectKind::PrivateKey, KeyUsage::DECRYPT)), vec![2, 3]);
        assert_eq!(ids(store.find(ObjectKind::PrivateKey, KeyUsage::DERIVE)), Vec::<u32>::new());
        assert_eq!(ids(store.find(ObjectKind::Certificate, KeyUsage::NONE)), vec![4]);
    }

    #[test]
    fn a_key_is_refused_until_the_session_authenticates() {
        let store = card();
        let locked = TokenSession::new("card-1");
        assert_eq!(store.use_key(&locked, 1, KeyUsage::SIGN), Err(TokenError::AuthenticationNeeded));
        let ok = authenticated("card-1");
        assert_eq!(store.use_key(&ok, 1, KeyUsage::SIGN), Ok(KeyRef { object_id: 1, usage: KeyUsage::SIGN }));
    }

    #[test]
    fn a_session_for_another_token_is_refused() {
        let store = card();
        let other = authenticated("card-2");
        assert_eq!(store.use_key(&other, 1, KeyUsage::SIGN), Err(TokenError::TokenNotFound));
    }

    #[test]
    fn a_key_is_cleared_only_for_a_use_it_allows() {
        let store = card();
        let ok = authenticated("card-1");
        assert_eq!(store.use_key(&ok, 2, KeyUsage::SIGN), Err(TokenError::BadParameter));
        assert_eq!(store.use_key(&ok, 3, KeyUsage::SIGN | KeyUsage::DECRYPT).map(|k| k.object_id), Ok(3));
        assert_eq!(store.use_key(&ok, 4, KeyUsage::SIGN), Err(TokenError::BadParameter));
        assert_eq!(store.use_key(&ok, 1, KeyUsage::NONE), Err(TokenError::BadParameter));
        assert_eq!(store.use_key(&ok, 9, KeyUsage::SIGN), Err(TokenError::ObjectNotFound));
    }

    #[test]
    fn remove_returns_the_object_once() {
        let mut store = card();
        assert_eq!(store.remove(1).map(|o| o.label), Ok("sign key".to_string()));
        assert!(store.get(1).is_none());
        assert_eq!(store.remove(1), Err(TokenError::ObjectNotFound));
        assert_eq!(store.add(ObjectKind::PrivateKey, "new", KeyUsage::SIGN), Ok(5));
    }
}
