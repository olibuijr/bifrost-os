//! Objects a cryptographic token stores (keys, certificates and identities), read as keychain-like items.
//! Pure model: a driver fills the store with `TokenObjectStore::add` when it reads the token. Objects that
//! require authentication are returned only to a session that the driver has authenticated with the same token.
//! The store holds the item descriptions only; no key material is kept here.
use std::collections::BTreeMap;

use crate::token_watch::{TokenError, TokenSession};

/// The kind of an object on a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObjectClass {
    Key,
    Certificate,
    Identity,
}

/// One object on a token: its ID, kind, user-visible label and whether reading it needs authentication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenObject {
    object_id: String,
    class: ObjectClass,
    label: String,
    requires_auth: bool,
}

impl TokenObject {
    pub fn new(object_id: &str, class: ObjectClass, label: &str, requires_auth: bool) -> Self {
        TokenObject { object_id: object_id.to_string(), class, label: label.to_string(), requires_auth }
    }

    pub fn object_id(&self) -> &str {
        &self.object_id
    }

    pub fn class(&self) -> ObjectClass {
        self.class
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn requires_auth(&self) -> bool {
        self.requires_auth
    }
}

/// The objects of one token, ordered by object ID. Object IDs are unique within the token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenObjectStore {
    token_id: String,
    objects: BTreeMap<String, TokenObject>,
}

impl TokenObjectStore {
    pub fn new(token_id: &str) -> Self {
        TokenObjectStore { token_id: token_id.to_string(), objects: BTreeMap::new() }
    }

    pub fn token_id(&self) -> &str {
        &self.token_id
    }

    /// Adds an object read from the token. An empty or already present object ID is `BadParameter`.
    pub fn add(&mut self, object: TokenObject) -> Result<(), TokenError> {
        if object.object_id.is_empty() || self.objects.contains_key(&object.object_id) {
            return Err(TokenError::BadParameter);
        }
        self.objects.insert(object.object_id.clone(), object);
        Ok(())
    }

    /// The objects of one class, or all of them when `class` is None, ordered by object ID. Descriptions are
    /// listed without authentication, as the labels are shown before a user unlocks the token.
    pub fn list(&self, class: Option<ObjectClass>) -> Vec<&TokenObject> {
        self.objects.values().filter(|o| class.map_or(true, |c| o.class == c)).collect()
    }

    /// Returns one object for use. The session must belong to this token, otherwise `TokenNotFound`. An unknown
    /// object is `ObjectNotFound`. A protected object needs an authenticated session, otherwise
    /// `AuthenticationNeeded`.
    pub fn find(&self, session: &TokenSession, object_id: &str) -> Result<&TokenObject, TokenError> {
        if session.token_id() != self.token_id {
            return Err(TokenError::TokenNotFound);
        }
        let object = self.objects.get(object_id).ok_or(TokenError::ObjectNotFound)?;
        if object.requires_auth {
            session.require()?;
        }
        Ok(object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> TokenObjectStore {
        let mut s = TokenObjectStore::new("card-1");
        s.add(TokenObject::new("obj-2", ObjectClass::Certificate, "Work certificate", false)).unwrap();
        s.add(TokenObject::new("obj-1", ObjectClass::Key, "Signing key", true)).unwrap();
        s.add(TokenObject::new("obj-3", ObjectClass::Identity, "Work identity", true)).unwrap();
        s
    }

    #[test]
    fn object_ids_must_be_unique_and_not_empty() {
        let mut s = store();
        assert_eq!(s.add(TokenObject::new("obj-1", ObjectClass::Key, "again", false)), Err(TokenError::BadParameter));
        assert_eq!(s.add(TokenObject::new("", ObjectClass::Key, "none", false)), Err(TokenError::BadParameter));
    }

    #[test]
    fn list_is_sorted_by_object_id_and_filters_by_class() {
        let s = store();
        let all: Vec<&str> = s.list(None).into_iter().map(|o| o.object_id()).collect();
        assert_eq!(all, vec!["obj-1", "obj-2", "obj-3"]);
        let keys: Vec<&str> = s.list(Some(ObjectClass::Key)).into_iter().map(|o| o.label()).collect();
        assert_eq!(keys, vec!["Signing key"]);
        assert!(s.list(Some(ObjectClass::Certificate)).iter().all(|o| !o.requires_auth()));
    }

    #[test]
    fn a_session_of_another_token_finds_nothing() {
        let s = store();
        let other = TokenSession::new("card-2");
        assert_eq!(s.find(&other, "obj-2").err(), Some(TokenError::TokenNotFound));
    }

    #[test]
    fn an_unknown_object_is_object_not_found() {
        let s = store();
        let session = TokenSession::new("card-1");
        assert_eq!(s.find(&session, "obj-9").err(), Some(TokenError::ObjectNotFound));
    }

    #[test]
    fn an_open_object_is_readable_without_authentication() {
        let s = store();
        let session = TokenSession::new("card-1");
        let found = s.find(&session, "obj-2").unwrap();
        assert_eq!(found.label(), "Work certificate");
    }

    #[test]
    fn a_protected_object_needs_an_authenticated_session() {
        let s = store();
        let mut session = TokenSession::new("card-1");
        assert_eq!(s.find(&session, "obj-1").err(), Some(TokenError::AuthenticationNeeded));
        session.authenticate(true).unwrap();
        assert_eq!(s.find(&session, "obj-1").unwrap().class(), ObjectClass::Key);
        session.logout();
        assert_eq!(s.find(&session, "obj-3").err(), Some(TokenError::AuthenticationNeeded));
    }
}
