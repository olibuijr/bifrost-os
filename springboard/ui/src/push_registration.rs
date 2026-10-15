//! Remote and web push registration for a desktop: the device token a push service hands out, the per-app
//! registration state (which app, which topic, when it expires), and the checks on an incoming push (known token,
//! not expired, payload size). Web push adds a subscription (endpoint and keys) per origin. Pure std; the cloud
//! service and the transport live elsewhere.

use std::collections::BTreeMap;

pub const MAX_PAYLOAD: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub app_id: String,
    pub topic: String,
    pub token: String,
    pub expires_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    UnknownToken,
    Expired,
    TooLarge,
    TopicMismatch,
}

/// A hex device token of 32 to 64 bytes.
pub fn valid_token(t: &str) -> bool {
    let n = t.len();
    (64..=128).contains(&n) && n % 2 == 0 && t.chars().all(|c| c.is_ascii_hexdigit())
}

#[derive(Clone, Debug, Default)]
pub struct Registry {
    by_token: BTreeMap<String, Registration>,
}

impl Registry {
    /// Registers an app for pushes. An app that registers again for the same topic replaces its earlier token.
    pub fn register(&mut self, reg: Registration) -> bool {
        if !valid_token(&reg.token) || reg.topic.is_empty() || reg.app_id.is_empty() {
            return false;
        }
        self.by_token.retain(|_, r| !(r.app_id == reg.app_id && r.topic == reg.topic));
        self.by_token.insert(reg.token.clone(), reg);
        true
    }

    pub fn unregister(&mut self, app_id: &str) -> usize {
        let before = self.by_token.len();
        self.by_token.retain(|_, r| r.app_id != app_id);
        before - self.by_token.len()
    }

    /// Checks an incoming push and returns the app it is for.
    pub fn route(&self, token: &str, topic: &str, payload_len: usize, now: i64) -> Result<&str, Refusal> {
        let reg = self.by_token.get(token).ok_or(Refusal::UnknownToken)?;
        if reg.expires_at <= now {
            return Err(Refusal::Expired);
        }
        if reg.topic != topic {
            return Err(Refusal::TopicMismatch);
        }
        if payload_len > MAX_PAYLOAD {
            return Err(Refusal::TooLarge);
        }
        Ok(&reg.app_id)
    }

    /// Drops expired registrations; returns how many.
    pub fn purge(&mut self, now: i64) -> usize {
        let before = self.by_token.len();
        self.by_token.retain(|_, r| r.expires_at > now);
        before - self.by_token.len()
    }

    pub fn len(&self) -> usize {
        self.by_token.len()
    }
}

/// A web push subscription of one origin: the endpoint URL and the keys the sender encrypts with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subscription {
    pub origin: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

impl Subscription {
    /// Valid when the endpoint is an https URL and both keys are present.
    pub fn is_valid(&self) -> bool {
        self.endpoint.starts_with("https://") && !self.p256dh.is_empty() && !self.auth.is_empty() && self.origin.starts_with("https://")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(c: char) -> String {
        std::iter::repeat(c).take(64).collect()
    }

    fn reg(app: &str, topic: &str, tok: &str, exp: i64) -> Registration {
        Registration { app_id: app.into(), topic: topic.into(), token: tok.into(), expires_at: exp }
    }

    #[test]
    fn token_shape() {
        assert!(valid_token(&token('a')));
        assert!(!valid_token(&token('g')));
        assert!(!valid_token("abcd"));
    }

    #[test]
    fn route_checks_in_order() {
        let mut r = Registry::default();
        assert!(r.register(reg("is.app", "news", &token('a'), 1000)));
        assert_eq!(r.route(&token('a'), "news", 100, 500), Ok("is.app"));
        assert_eq!(r.route(&token('b'), "news", 100, 500), Err(Refusal::UnknownToken));
        assert_eq!(r.route(&token('a'), "news", 100, 1000), Err(Refusal::Expired));
        assert_eq!(r.route(&token('a'), "other", 100, 500), Err(Refusal::TopicMismatch));
        assert_eq!(r.route(&token('a'), "news", MAX_PAYLOAD + 1, 500), Err(Refusal::TooLarge));
    }

    #[test]
    fn reregistering_replaces_the_token() {
        let mut r = Registry::default();
        r.register(reg("is.app", "news", &token('a'), 1000));
        r.register(reg("is.app", "news", &token('b'), 1000));
        assert_eq!(r.len(), 1);
        assert!(r.route(&token('a'), "news", 1, 0).is_err());
        assert!(!r.register(reg("", "t", &token('c'), 1)));
        assert!(!r.register(reg("x", "t", "short", 1)));
    }

    #[test]
    fn purge_and_unregister() {
        let mut r = Registry::default();
        r.register(reg("a", "t", &token('a'), 100));
        r.register(reg("b", "t", &token('b'), 900));
        assert_eq!(r.purge(500), 1);
        assert_eq!(r.unregister("b"), 1);
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn web_subscription_validity() {
        let s = Subscription { origin: "https://x.is".into(), endpoint: "https://push.example/abc".into(), p256dh: "k".into(), auth: "a".into() };
        assert!(s.is_valid());
        assert!(!Subscription { endpoint: "http://push".into(), ..s.clone() }.is_valid());
        assert!(!Subscription { auth: String::new(), ..s }.is_valid());
    }
}
