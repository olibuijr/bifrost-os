//! Pane links: the `x-apple.systempreferences:` URL that opens a System Settings pane by its identifier, optionally at
//! an anchor after `?` (for example the Privacy pane's `Privacy_Camera` row). The builder and the reader are pure;
//! the host maps an identifier to its own page. Identifiers and anchors are plain tokens: letters, digits, `.`, `-`
//! and `_`.

/// The URL scheme System Settings registers for pane links.
pub const SCHEME: &str = "x-apple.systempreferences";

/// A parsed pane link: the pane identifier and the anchor inside it, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneLink {
    pub identifier: String,
    pub anchor: Option<String>,
}

/// Whether `s` is a non-empty plain token (an identifier or an anchor).
fn is_token(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// The link that opens `identifier`, at `anchor` when one is given. None when either part is not a plain token.
pub fn pane_url(identifier: &str, anchor: Option<&str>) -> Option<String> {
    if !is_token(identifier) {
        return None;
    }
    match anchor {
        None => Some(format!("{SCHEME}:{identifier}")),
        Some(a) if is_token(a) => Some(format!("{SCHEME}:{identifier}?{a}")),
        Some(_) => None,
    }
}

/// Reads a pane link back. None when the scheme is not `x-apple.systempreferences:`, the identifier is missing or not a
/// plain token, or a `?` is present with an empty or non-token anchor (the anchor is everything after the first `?`).
pub fn parse_pane_url(url: &str) -> Option<PaneLink> {
    let rest = url.strip_prefix(SCHEME)?.strip_prefix(':')?;
    let (identifier, anchor) = match rest.split_once('?') {
        Some((id, a)) => (id, Some(a)),
        None => (rest, None),
    };
    if !is_token(identifier) {
        return None;
    }
    let anchor = match anchor {
        None => None,
        Some(a) if is_token(a) => Some(a.to_string()),
        Some(_) => return None,
    };
    Some(PaneLink { identifier: identifier.to_string(), anchor })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISPLAYS: &str = "com.apple.preference.displays";
    const SECURITY: &str = "com.apple.preference.security";

    #[test]
    fn a_pane_url_names_the_pane_without_an_anchor() {
        assert_eq!(
            pane_url(DISPLAYS, None).as_deref(),
            Some("x-apple.systempreferences:com.apple.preference.displays")
        );
    }

    #[test]
    fn a_pane_url_opens_an_anchor_after_a_question_mark() {
        assert_eq!(
            pane_url(SECURITY, Some("Privacy_Camera")).as_deref(),
            Some("x-apple.systempreferences:com.apple.preference.security?Privacy_Camera")
        );
    }

    #[test]
    fn a_pane_url_refuses_parts_that_are_not_plain_tokens() {
        assert_eq!(pane_url("", None), None, "an empty identifier names nothing");
        assert_eq!(pane_url("com.example pane", None), None, "no spaces in an identifier");
        assert_eq!(pane_url("displays?x", None), None, "a query character cannot be part of the identifier");
        assert_eq!(pane_url(DISPLAYS, Some("")), None, "an empty anchor is not an anchor");
        assert_eq!(pane_url(DISPLAYS, Some("Privacy&x")), None, "an anchor is a plain token too");
    }

    #[test]
    fn a_link_reads_back_the_identifier_and_anchor_it_was_built_from() {
        let url = pane_url(SECURITY, Some("Privacy_Camera")).unwrap();
        assert_eq!(
            parse_pane_url(&url),
            Some(PaneLink { identifier: SECURITY.into(), anchor: Some("Privacy_Camera".into()) })
        );
        let url = pane_url(DISPLAYS, None).unwrap();
        assert_eq!(parse_pane_url(&url), Some(PaneLink { identifier: DISPLAYS.into(), anchor: None }));
    }

    #[test]
    fn a_link_with_another_scheme_is_not_a_pane_link() {
        assert_eq!(parse_pane_url("https://example.com/com.apple.preference.displays"), None);
        assert_eq!(parse_pane_url("x-apple.systempreferencesX:com.apple.preference.displays"), None);
        assert_eq!(parse_pane_url("com.apple.preference.displays"), None, "the scheme is required");
    }

    #[test]
    fn a_link_without_an_identifier_or_with_an_empty_anchor_is_refused() {
        assert_eq!(parse_pane_url("x-apple.systempreferences:"), None);
        assert_eq!(parse_pane_url("x-apple.systempreferences:?Privacy_Camera"), None);
        assert_eq!(parse_pane_url("x-apple.systempreferences:com.apple.preference.security?"), None);
    }

    #[test]
    fn a_link_splits_at_the_first_question_mark_only() {
        assert_eq!(parse_pane_url("x-apple.systempreferences:com.apple.preference.security?a?b"), None);
    }
}
