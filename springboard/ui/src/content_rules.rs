//! Content-blocking rules: a pure matcher that decides whether a page's request is blocked or let through.
//! Patterns use a subset of the content-blocker url-filter syntax: `*` (any run of characters), `^` (a separator
//! or the end of the address), a leading `||` (the start of the host or of a label inside it), a leading `|` (the
//! start of the address) and a trailing `|` (the end of the address). Rules can be limited to resource types and to
//! page hosts. The caller supplies the request and the page address; nothing is fetched or stored here.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceType {
    Document,
    Script,
    Style,
    Image,
    Font,
    Media,
    Fetch,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Stop the request.
    Block,
    /// Let the request through, even when a block rule also matches it.
    Allow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    Block,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pattern: String,
    action: Action,
    types: Vec<ResourceType>,
    if_domains: Vec<String>,
    unless_domains: Vec<String>,
}

impl Rule {
    pub fn block(pattern: &str) -> Self {
        Self::new(pattern, Action::Block)
    }

    pub fn allow(pattern: &str) -> Self {
        Self::new(pattern, Action::Allow)
    }

    fn new(pattern: &str, action: Action) -> Self {
        Rule {
            pattern: pattern.trim().to_ascii_lowercase(),
            action,
            types: Vec::new(),
            if_domains: Vec::new(),
            unless_domains: Vec::new(),
        }
    }

    /// Limits the rule to these resource types. Without it the rule applies to every type.
    pub fn for_types(mut self, types: &[ResourceType]) -> Self {
        self.types = types.to_vec();
        self
    }

    /// The rule applies only on pages whose host is `domain` or a subdomain of it.
    pub fn if_domain(mut self, domain: &str) -> Self {
        self.if_domains.push(domain.to_ascii_lowercase());
        self
    }

    /// The rule does not apply on pages whose host is `domain` or a subdomain of it.
    pub fn unless_domain(mut self, domain: &str) -> Self {
        self.unless_domains.push(domain.to_ascii_lowercase());
        self
    }

    pub fn action(&self) -> Action {
        self.action
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    fn applies_on(&self, page_host: &str) -> bool {
        (self.if_domains.is_empty() || self.if_domains.iter().any(|d| domain_matches(page_host, d)))
            && !self.unless_domains.iter().any(|d| domain_matches(page_host, d))
    }

    fn matches(&self, url: &str, host: &str, resource: ResourceType, page_host: &str) -> bool {
        (self.types.is_empty() || self.types.contains(&resource))
            && self.applies_on(page_host)
            && pattern_matches(&self.pattern, url, host)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleList {
    rules: Vec<Rule>,
}

impl RuleList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Decides one request made by the page at `page_url`. A matching allow rule wins over every block rule,
    /// whatever the order they were added in. Otherwise a matching block rule stops the request.
    pub fn check(&self, url: &str, page_url: &str, resource: ResourceType) -> Verdict {
        let url = url.to_ascii_lowercase();
        let page = page_url.to_ascii_lowercase();
        let host = host_of(&url);
        let page_host = host_of(&page);
        let hits = |action: Action| {
            self.rules.iter().any(|r| r.action == action && r.matches(&url, host, resource, page_host))
        };
        if hits(Action::Allow) || !hits(Action::Block) {
            Verdict::Allow
        } else {
            Verdict::Block
        }
    }
}

/// The host of an address: the text after `scheme://`, up to a path, query, fragment or port.
fn host_of(url: &str) -> &str {
    let rest = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    let end = rest.find(|c: char| matches!(c, '/' | '?' | '#' | ':')).unwrap_or(rest.len());
    &rest[..end]
}

/// `host` is `domain` or a subdomain of it.
fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || (host.len() > domain.len() + 1
            && host.ends_with(domain)
            && host.as_bytes()[host.len() - domain.len() - 1] == b'.')
}

/// Whether the (lowercase) pattern matches somewhere in the (lowercase) address, under the anchors it declares.
fn pattern_matches(pattern: &str, url: &str, host: &str) -> bool {
    let (domain_anchored, rest) = match pattern.strip_prefix("||") {
        Some(rest) => (true, rest),
        None => (false, pattern),
    };
    let (start_anchored, rest) = if domain_anchored {
        (false, rest)
    } else {
        match rest.strip_prefix('|') {
            Some(rest) => (true, rest),
            None => (false, rest),
        }
    };
    let (end_anchored, body) = match rest.strip_suffix('|') {
        Some(body) => (true, body),
        None => (false, rest),
    };
    let bytes = url.as_bytes();
    let starts: Vec<usize> = if start_anchored {
        vec![0]
    } else if domain_anchored {
        let host_start = url.find("://").map_or(0, |i| i + 3);
        let host_end = host_start + host.len();
        std::iter::once(host_start)
            .chain(url[host_start..host_end].match_indices('.').map(|(i, _)| host_start + i + 1))
            .collect()
    } else {
        (0..=bytes.len()).collect()
    };
    starts.into_iter().any(|s| glob(body.as_bytes(), &bytes[s..], end_anchored))
}

/// Matches `pattern` at the start of `s`. With `end_anchored` the match must use all of `s`.
fn glob(pattern: &[u8], s: &[u8], end_anchored: bool) -> bool {
    match pattern.split_first() {
        None => !end_anchored || s.is_empty(),
        Some((&b'*', rest)) => (0..=s.len()).any(|k| glob(rest, &s[k..], end_anchored)),
        Some((&b'^', rest)) => {
            if s.is_empty() {
                glob(rest, s, end_anchored)
            } else if is_separator(s[0]) {
                glob(rest, &s[1..], end_anchored)
            } else {
                false
            }
        }
        Some((&c, rest)) => s.first() == Some(&c) && glob(rest, &s[1..], end_anchored),
    }
}

fn is_separator(c: u8) -> bool {
    !(c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'%'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://news.example.com/front";

    fn blocked(list: &RuleList, url: &str) -> bool {
        list.check(url, PAGE, ResourceType::Script) == Verdict::Block
    }

    #[test]
    fn an_empty_list_lets_everything_through() {
        let list = RuleList::new();
        assert!(list.is_empty());
        assert_eq!(list.check("https://ads.example.net/x", PAGE, ResourceType::Image), Verdict::Allow);
    }

    #[test]
    fn plain_patterns_match_anywhere_in_the_address() {
        let mut list = RuleList::new();
        list.add(Rule::block("/tracker.js"));
        assert!(blocked(&list, "https://cdn.example.org/static/tracker.js?v=2"));
        assert!(!blocked(&list, "https://cdn.example.org/static/app.js"));
    }

    #[test]
    fn a_domain_anchor_matches_the_host_and_its_subdomains_only() {
        let mut list = RuleList::new();
        list.add(Rule::block("||ads.example.net^"));
        assert!(blocked(&list, "https://ads.example.net/x"));
        assert!(blocked(&list, "https://eu.ads.example.net/x"));
        assert!(!blocked(&list, "https://badads.example.net/x"));
        assert!(!blocked(&list, "https://example.net/ads.example.net/x"));
    }

    #[test]
    fn star_and_caret_work_inside_a_pattern() {
        let mut list = RuleList::new();
        list.add(Rule::block("/promo/*.gif"));
        list.add(Rule::block("example.org^"));
        assert!(blocked(&list, "https://x.org/promo/2024/spring.gif"));
        assert!(!blocked(&list, "https://x.org/promo/2024/spring.png"));
        assert!(blocked(&list, "https://example.org/a"));
        assert!(blocked(&list, "https://example.org"));
        assert!(!blocked(&list, "https://example.organic.com/a"));
    }

    #[test]
    fn a_leading_bar_anchors_the_start_and_a_trailing_bar_the_end() {
        let mut list = RuleList::new();
        list.add(Rule::block("|https://ads.example.com/"));
        list.add(Rule::block(".swf|"));
        assert!(blocked(&list, "https://ads.example.com/unit"));
        assert!(!blocked(&list, "https://www.example.com/https://ads.example.com/unit"));
        assert!(blocked(&list, "https://game.example.com/movie.swf"));
        assert!(!blocked(&list, "https://game.example.com/movie.swf?autoplay=1"));
    }

    #[test]
    fn resource_types_limit_a_rule() {
        let mut list = RuleList::new();
        list.add(Rule::block("/metrics/").for_types(&[ResourceType::Script]));
        assert_eq!(list.check("https://a.example.com/metrics/x.js", PAGE, ResourceType::Script), Verdict::Block);
        assert_eq!(list.check("https://a.example.com/metrics/x.js", PAGE, ResourceType::Image), Verdict::Allow);
    }

    #[test]
    fn page_domains_include_and_exclude_rules() {
        let mut list = RuleList::new();
        list.add(Rule::block("/widget.js").if_domain("example.com"));
        list.add(Rule::block("/gallery.js").unless_domain("photos.example.com"));
        assert_eq!(list.check("https://cdn.test/widget.js", "https://shop.example.com/", ResourceType::Script), Verdict::Block);
        assert_eq!(list.check("https://cdn.test/widget.js", "https://other.org/", ResourceType::Script), Verdict::Allow);
        assert_eq!(list.check("https://cdn.test/gallery.js", "https://photos.example.com/a", ResourceType::Script), Verdict::Allow);
        assert_eq!(list.check("https://cdn.test/gallery.js", "https://blog.org/", ResourceType::Script), Verdict::Block);
    }

    #[test]
    fn an_allow_rule_overrides_block_rules_whatever_their_order() {
        let mut list = RuleList::new();
        list.add(Rule::allow("||cdn.example.com/lib/"));
        list.add(Rule::block("||example.com^"));
        assert!(!blocked(&list, "https://cdn.example.com/lib/x.js"));
        assert!(blocked(&list, "https://cdn.example.com/ad.js"));
    }
}
