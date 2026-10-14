//! Sign-in suggestions and save decisions for website logins. A sign-in form asks which saved logins fit the page it is
//! on, and after a successful sign-in it asks whether to save the password that was typed. Both answers are pure:
//! they read the logins and the page address and change nothing.
//!
//! A saved site fits a page when both name the same host, or when the page's host is a subdomain of the saved host.
//! A leading `www.` is ignored on both sides, so `www.example.com` and `example.com` name the same site. Ports, paths,
//! queries, fragments and user information are dropped before the comparison.

use springboard::credentials::Login;

/// The host a page address names, lowercased, without `www.`, port, path, query, fragment or user information. `None`
/// when the address names no host.
pub fn host_of(page: &str) -> Option<String> {
    let rest = match page.find("://") {
        Some(i) => &page[i + 3..],
        None => page,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_port = authority.rsplit('@').next().unwrap_or("");
    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
        bracketed.split(']').next().unwrap_or("")
    } else {
        host_port.split(':').next().unwrap_or("")
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    let host = host.strip_prefix("www.").map(str::to_string).unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// True when the saved `site` fits the `page` address: the same host, or a subdomain of the saved host.
pub fn site_matches(site: &str, page: &str) -> bool {
    let (Some(saved), Some(here)) = (host_of(site), host_of(page)) else { return false };
    here == saved || here.ends_with(&format!(".{saved}"))
}

/// The saved logins that fit `page`, the most recently written first. Logins written at the same time keep the order of
/// the site, then the account, so the list does not change between calls.
pub fn suggestions<'a>(logins: &'a [Login], page: &str) -> Vec<&'a Login> {
    let mut fit: Vec<&Login> = logins.iter().filter(|l| site_matches(&l.site, page)).collect();
    fit.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.site.cmp(&b.site).then_with(|| a.account.cmp(&b.account))));
    fit
}

/// What to do with a password a sign-in form submitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveDecision {
    /// The form submitted no password, so there is nothing to keep.
    Nothing,
    /// No login exists for the site and account: save a new one.
    Save,
    /// A login exists with the same password: nothing changes.
    Unchanged,
    /// A login exists with another password: replace it.
    Update,
}

/// Decides what to do with `submitted`, given the saved password for the same site and account (`None` when there is
/// none). The saved password is compared exactly; the submitted one is never empty for a save or an update.
pub fn save_decision(saved: Option<&str>, submitted: &str) -> SaveDecision {
    if submitted.is_empty() {
        return SaveDecision::Nothing;
    }
    match saved {
        None => SaveDecision::Save,
        Some(old) if old == submitted => SaveDecision::Unchanged,
        Some(_) => SaveDecision::Update,
    }
}
