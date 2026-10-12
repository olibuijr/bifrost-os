//! DNS proxy policy: which DNS queries a proxy forwards, answers itself or refuses (std only).
//!
//! A DNS proxy sits between the apps on a machine and the resolver. It sees each query before the resolver does and
//! decides what happens to it. This module makes that decision and builds the answer the proxy sends back when it
//! does not forward. The model opens no socket and reads no traffic: the proxy hands it the bytes of one query, sends
//! whatever [`DnsProxy::decide`] returns, and forwards the query itself when told to.
//!
//! Rules of this model:
//! - A rule names a domain with the suffix rule from [`crate::dns_settings`]: `example.com` covers `example.com` and
//!   every name under it, and never `badexample.com`.
//! - The most specific matching rule wins, meaning the rule whose domain has the most labels. Adding a rule for a
//!   domain that already has one replaces it.
//! - A query with no matching rule is forwarded. A disabled proxy forwards every query.
//! - Only a query with exactly one question is matched. A query with more questions is forwarded unchanged.
//! - [`RuleAction::Refuse`] answers with the name-error response code (NXDOMAIN, 3) and no records.
//! - [`RuleAction::Answer`] answers an A query with one IPv4 address and an answer with no records and no error for
//!   other types, so that the name exists but has no data for them.

use crate::dns_message::{self, DnsError, Message, Question, RData, RType, Record, CLASS_IN};
use crate::dns_settings::{domain_matches, valid_domain};
use std::fmt;
use std::net::Ipv4Addr;

/// Time to live, in seconds, of an address the proxy answers itself.
pub const ANSWER_TTL: u32 = 60;
/// The response code for a name that does not exist.
pub const RCODE_NAME_ERROR: u8 = 3;
/// Mask of the response code in the low four bits of the second flags byte.
const RCODE_MASK: u8 = 0x0F;

/// What the proxy does with the queries a rule covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleAction {
    /// Pass the query to the resolver.
    Forward,
    /// Answer the name with this IPv4 address.
    Answer(Ipv4Addr),
    /// Answer the name-error response code.
    Refuse,
}

/// One rule: a domain and what to do with its queries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyRule {
    pub domain: String,
    pub action: RuleAction,
}

/// What the proxy does with one query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Pass the query, unchanged, to the resolver.
    Forward,
    /// Send these bytes back to the querier. They are a complete DNS response with the query's id.
    Reply(Vec<u8>),
}

/// Why a proxy setting or a query is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProxyError {
    /// The provider bundle identifier is empty.
    EmptyBundleId,
    /// The domain of a rule is not a valid dotted name.
    BadDomain(String),
    /// The bytes are not a DNS message this module can read.
    Malformed(DnsError),
    /// The message is a response, not a query.
    NotAQuery,
}

impl fmt::Display for ProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProxyError::EmptyBundleId => write!(f, "the provider bundle identifier is empty"),
            ProxyError::BadDomain(domain) => write!(f, "{domain:?} is not a valid domain"),
            ProxyError::Malformed(err) => write!(f, "the query is malformed: {err}"),
            ProxyError::NotAQuery => write!(f, "the message is a response, not a query"),
        }
    }
}

impl std::error::Error for ProxyError {}

/// The proxy configuration: the provider that runs it, whether it is on, and its rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DnsProxy {
    provider_bundle_id: String,
    enabled: bool,
    rules: Vec<ProxyRule>,
}

impl DnsProxy {
    /// A proxy that is off and has no rules. The bundle identifier names the provider that runs it.
    pub fn new(provider_bundle_id: &str) -> Result<Self, ProxyError> {
        let id = provider_bundle_id.trim();
        if id.is_empty() {
            return Err(ProxyError::EmptyBundleId);
        }
        Ok(DnsProxy { provider_bundle_id: id.to_string(), enabled: false, rules: Vec::new() })
    }

    pub fn provider_bundle_id(&self) -> &str {
        &self.provider_bundle_id
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    /// The rules, in the order they were first added.
    pub fn rules(&self) -> &[ProxyRule] {
        &self.rules
    }

    /// Adds a rule, or replaces the action of the rule that has the same domain. A refused domain changes nothing.
    pub fn add_rule(&mut self, domain: &str, action: RuleAction) -> Result<(), ProxyError> {
        if !valid_domain(domain) {
            return Err(ProxyError::BadDomain(domain.to_string()));
        }
        let domain = crate::dns_settings::normalise_domain(domain);
        match self.rules.iter().position(|r| r.domain == domain) {
            Some(i) => self.rules[i].action = action,
            None => self.rules.push(ProxyRule { domain, action }),
        }
        Ok(())
    }

    /// Removes the rule for `domain`. Returns false when no rule has that domain.
    pub fn remove_rule(&mut self, domain: &str) -> bool {
        let domain = crate::dns_settings::normalise_domain(domain);
        let before = self.rules.len();
        self.rules.retain(|r| r.domain != domain);
        self.rules.len() != before
    }

    /// The action of the most specific rule that covers `name`, or None when no rule covers it.
    pub fn action_for(&self, name: &str) -> Option<RuleAction> {
        self.rules
            .iter()
            .filter(|r| domain_matches(name, &r.domain))
            .max_by_key(|r| r.domain.split('.').count())
            .map(|r| r.action)
    }

    /// Decides what to do with one query, given as the bytes of a DNS message.
    pub fn decide(&self, query: &[u8]) -> Result<Decision, ProxyError> {
        let msg = dns_message::decode(query).map_err(ProxyError::Malformed)?;
        if msg.response {
            return Err(ProxyError::NotAQuery);
        }
        if !self.enabled || msg.questions.len() != 1 {
            return Ok(Decision::Forward);
        }
        let question = &msg.questions[0];
        let action = match self.action_for(&question.name) {
            None | Some(RuleAction::Forward) => return Ok(Decision::Forward),
            Some(action) => action,
        };
        let reply = match action {
            RuleAction::Refuse => build_reply(&msg, question, None, RCODE_NAME_ERROR)?,
            RuleAction::Answer(addr) => {
                if question.rtype == RType::A && question.class == CLASS_IN {
                    build_reply(&msg, question, Some(addr), 0)?
                } else {
                    build_reply(&msg, question, None, 0)?
                }
            }
            RuleAction::Forward => unreachable!("forward is returned above"),
        };
        Ok(Decision::Reply(reply))
    }
}

/// Builds a response to `query` with the given response code and, when `addr` is given, one A record for its name.
fn build_reply(
    query: &Message,
    question: &Question,
    addr: Option<Ipv4Addr>,
    rcode: u8,
) -> Result<Vec<u8>, ProxyError> {
    let answers = match addr {
        Some(addr) => vec![Record {
            name: question.name.clone(),
            class: CLASS_IN,
            cache_flush: false,
            ttl: ANSWER_TTL,
            data: RData::A(addr.octets()),
        }],
        None => Vec::new(),
    };
    let reply = Message {
        id: query.id,
        response: true,
        authoritative: false,
        questions: query.questions.clone(),
        answers,
        authorities: Vec::new(),
        additionals: Vec::new(),
    };
    let mut bytes = dns_message::encode(&reply).map_err(ProxyError::Malformed)?;
    // The encoder leaves the response code zero. It sits in the low four bits of the second flags byte.
    bytes[3] = (bytes[3] & !RCODE_MASK) | (rcode & RCODE_MASK);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(id: u16, name: &str, rtype: RType) -> Vec<u8> {
        let msg = Message {
            id,
            response: false,
            authoritative: false,
            questions: vec![Question { name: name.to_string(), rtype, class: CLASS_IN, unicast_response: false }],
            answers: vec![],
            authorities: vec![],
            additionals: vec![],
        };
        dns_message::encode(&msg).unwrap()
    }

    fn proxy_with(rules: &[(&str, RuleAction)]) -> DnsProxy {
        let mut p = DnsProxy::new("com.example.dns-proxy").unwrap();
        p.enable();
        for (domain, action) in rules {
            p.add_rule(domain, *action).unwrap();
        }
        p
    }

    #[test]
    fn a_refused_name_gets_a_name_error_with_the_query_id_and_question() {
        let p = proxy_with(&[("example.com", RuleAction::Refuse)]);
        let decision = p.decide(&query(0x2A2B, "api.example.com", RType::A)).unwrap();
        let Decision::Reply(bytes) = decision else { panic!("a refused name is answered") };
        // Flags: response bit 0x8000 and response code 3 in the low nibble, so 0x8003.
        assert_eq!(&bytes[..4], &[0x2A, 0x2B, 0x80, 0x03]);
        // One question, no answers.
        assert_eq!(&bytes[4..12], &[0, 1, 0, 0, 0, 0, 0, 0]);
        let back = dns_message::decode(&bytes).unwrap();
        assert_eq!(back.questions[0].name, "api.example.com");
        assert!(back.answers.is_empty());
    }

    #[test]
    fn the_most_specific_rule_wins() {
        let p = proxy_with(&[
            ("example.com", RuleAction::Refuse),
            ("tracker.example.com", RuleAction::Forward),
        ]);
        assert_eq!(p.action_for("x.tracker.example.com"), Some(RuleAction::Forward));
        assert_eq!(p.action_for("y.example.com"), Some(RuleAction::Refuse));
        assert_eq!(p.action_for("example.org"), None);
        // The forward rule passes the query on rather than answering it.
        assert_eq!(p.decide(&query(1, "x.tracker.example.com", RType::A)).unwrap(), Decision::Forward);
    }

    #[test]
    fn a_domain_only_covers_names_on_a_label_boundary() {
        let p = proxy_with(&[("example.com", RuleAction::Refuse)]);
        assert_eq!(p.decide(&query(1, "badexample.com", RType::A)).unwrap(), Decision::Forward);
        assert!(matches!(p.decide(&query(1, "EXAMPLE.com.", RType::A)).unwrap(), Decision::Reply(_)));
    }

    #[test]
    fn an_answer_rule_gives_its_address_to_a_queries_and_no_data_to_others() {
        let p = proxy_with(&[("intranet.local", RuleAction::Answer(Ipv4Addr::new(10, 0, 0, 5)))]);
        let Decision::Reply(a) = p.decide(&query(9, "intranet.local", RType::A)).unwrap() else { panic!() };
        let back = dns_message::decode(&a).unwrap();
        assert_eq!(back.answers.len(), 1);
        assert_eq!(back.answers[0].data, RData::A([10, 0, 0, 5]));
        assert_eq!(back.answers[0].ttl, ANSWER_TTL);
        assert_eq!(a[3] & RCODE_MASK, 0);

        let Decision::Reply(aaaa) = p.decide(&query(9, "intranet.local", RType::Aaaa)).unwrap() else { panic!() };
        let back = dns_message::decode(&aaaa).unwrap();
        assert!(back.answers.is_empty());
        assert_eq!(aaaa[3] & RCODE_MASK, 0);
    }

    #[test]
    fn a_disabled_proxy_or_an_unclear_query_is_forwarded() {
        let mut p = proxy_with(&[("example.com", RuleAction::Refuse)]);
        p.disable();
        assert_eq!(p.decide(&query(1, "example.com", RType::A)).unwrap(), Decision::Forward);

        p.enable();
        // Two questions are passed on as they are.
        let two = Message {
            id: 3,
            response: false,
            authoritative: false,
            questions: vec![
                Question { name: "a.example.com".into(), rtype: RType::A, class: CLASS_IN, unicast_response: false },
                Question { name: "b.example.com".into(), rtype: RType::A, class: CLASS_IN, unicast_response: false },
            ],
            answers: vec![],
            authorities: vec![],
            additionals: vec![],
        };
        assert_eq!(p.decide(&dns_message::encode(&two).unwrap()).unwrap(), Decision::Forward);
    }

    #[test]
    fn responses_and_broken_bytes_are_refused() {
        let p = proxy_with(&[("example.com", RuleAction::Refuse)]);
        let mut response = query(4, "example.com", RType::A);
        response[2] |= 0x80;
        assert_eq!(p.decide(&response), Err(ProxyError::NotAQuery));
        assert!(matches!(p.decide(&[0, 1, 2]), Err(ProxyError::Malformed(_))));
    }

    #[test]
    fn rules_replace_by_domain_and_refused_settings_change_nothing() {
        let mut p = DnsProxy::new("com.example.dns-proxy").unwrap();
        p.add_rule("Example.COM.", RuleAction::Refuse).unwrap();
        p.add_rule("example.com", RuleAction::Forward).unwrap();
        assert_eq!(p.rules().len(), 1);
        assert_eq!(p.action_for("example.com"), Some(RuleAction::Forward));

        assert_eq!(p.add_rule("bad..name", RuleAction::Refuse), Err(ProxyError::BadDomain("bad..name".into())));
        assert_eq!(p.rules().len(), 1);
        assert!(p.remove_rule("example.com"));
        assert!(!p.remove_rule("example.com"));
        assert_eq!(DnsProxy::new("  ").unwrap_err(), ProxyError::EmptyBundleId);
    }
}
