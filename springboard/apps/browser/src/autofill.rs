//! Password AutoFill (Browser): File > AutoFill Password fills the sign-in form of the open page from the shared
//! passwords group (`bifrost_app::passwords`), the same logins the Passwords app and Settings list. The menu item is
//! enabled only on a page with a password field (`FORM_JS`, evaluated after each load).
//!
//! Two requests run in turn: `passwords::list` finds the newest login whose site matches the page's host, then
//! `passwords::fill` returns its secret. The secret goes straight into an eval script for the page and is never
//! logged or kept after the script is sent.
use std::collections::HashSet;

use bifrost_app::keychain_groups as keychain;
use bifrost_app::passwords::{self, Kind};
use bifrost_app::webview::WebViewId;
use bifrost_app::Ctx;
use bifrost_core::json::Json;

/// Reply prefix of [`FORM_JS`]: `bifrost-form:1` when the page has a password field, `:0` when it has none.
pub const FORM_PREFIX: &str = "bifrost-form:";
/// Reply prefix of the fill script: `bifrost-fill:1` when the fields were filled, `:0` when there is no password field.
pub const FILL_PREFIX: &str = "bifrost-fill:";
pub const FORM_JS: &str = "(function(){return 'bifrost-form:'+(document.querySelector('input[type=password]')?1:0)})()";

const FILL_PRE: &str = "(function(){var p=document.querySelector('input[type=password]');if(!p)return 'bifrost-fill:0';var u=null,c=(p.form||document).querySelectorAll('input:not([type]),input[type=text],input[type=email],input[type=tel]');for(var i=0;i<c.length;i++){if(c[i].compareDocumentPosition(p)&4)u=c[i];else break;}var d=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value');function s(e,v){d.set.call(e,v);e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));}if(u)s(u,";
const FILL_MID: &str = ");s(p,";
const FILL_POST: &str = ");return 'bifrost-fill:1'})()";

/// The host of a URL or a stored site: no scheme, port, path, user info, `www.`, lower case.
pub fn bare_host(site: &str) -> String {
    let rest = site.split_once("://").map_or(site, |(_, r)| r);
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = auth.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("");
    let host = host.trim().to_ascii_lowercase();
    host.strip_prefix("www.").map(str::to_string).unwrap_or(host)
}

/// True when a login stored for `site` applies to `host`: the same host, or a subdomain of it.
pub fn host_matches(site: &str, host: &str) -> bool {
    let (site, host) = (bare_host(site), bare_host(host));
    !site.is_empty() && !host.is_empty() && (host == site || host.ends_with(&format!(".{site}")))
}

/// The login to fill for `host` from the group's listing: the newest login (not a note or certificate) whose site
/// matches. Answers `(service, account)`, the arguments of `passwords::fill`.
pub fn pick(rows: &[keychain::Row], host: &str) -> Option<(String, String)> {
    rows.iter()
        .filter(|r| matches!(passwords::kind_of(&r.service), Kind::Login) && host_matches(&r.service, host))
        .max_by_key(|r| r.modified)
        .map(|r| (r.service.clone(), r.account.clone()))
}

/// A JavaScript string literal for `s`: quotes, backslashes and control characters escaped, so no value can end the
/// literal or the script.
pub fn js_string(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '<' => out.push_str("\\u003C"),
            '\u{2028}' | '\u{2029}' => out.push_str(&format!("\\u{:04x}", ch as u32)),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The script that fills the last text-like field before the password field with `account`, then the password field
/// with `password`. Each value is set through the native setter and announced with input and change events, so pages
/// that keep their own state see it.
pub fn fill_js(account: &str, password: &str) -> String {
    format!("{FILL_PRE}{}{FILL_MID}{}{FILL_POST}", js_string(account), js_string(password))
}

#[derive(Debug)]
enum Stage {
    /// Waiting for the logins listing.
    Listing,
    /// Waiting for the secret of the chosen login.
    Reading { account: String },
}

#[derive(Debug)]
struct Pending {
    request: u32,
    view: WebViewId,
    host: String,
    stage: Stage,
}

#[derive(Default)]
pub struct AutoFill {
    /// Views whose last loaded page had a password field.
    forms: HashSet<WebViewId>,
    pending: Option<Pending>,
}

impl AutoFill {
    /// Takes the answer to [`FORM_JS`] or to the fill script. True when the reply was this module's.
    pub fn on_eval(&mut self, view: WebViewId, text: &str) -> bool {
        if let Some(flag) = text.strip_prefix(FORM_PREFIX) {
            if flag == "1" {
                self.forms.insert(view);
            } else {
                self.forms.remove(&view);
            }
            return true;
        }
        text.starts_with(FILL_PREFIX)
    }

    pub fn has_form(&self, view: WebViewId) -> bool {
        self.forms.contains(&view)
    }

    pub fn forget(&mut self, view: WebViewId) {
        self.forms.remove(&view);
        if self.pending.as_ref().is_some_and(|p| p.view == view) {
            self.pending = None;
        }
    }

    /// File > AutoFill Password: asks the group for its logins. False when the page has no host to match.
    pub fn start(&mut self, ctx: &Ctx, view: WebViewId, url: &str) -> bool {
        let host = bare_host(url);
        if host.is_empty() {
            return false;
        }
        let request = passwords::list(ctx);
        self.pending = Some(Pending { request, view, host, stage: Stage::Listing });
        true
    }

    /// Takes the answer to one of this module's requests. Answers the view and the script to run there once the
    /// password is known; `None` for other requests and while the flow waits for the next answer.
    pub fn on_reply(&mut self, ctx: &Ctx, request: u32, result: &Result<Json, String>) -> Option<(WebViewId, String)> {
        if self.pending.as_ref().is_none_or(|p| p.request != request) {
            return None;
        }
        let p = self.pending.take()?;
        match p.stage {
            Stage::Listing => {
                let rows = passwords::decode_list(result).unwrap_or_default();
                let Some((service, account)) = pick(&rows, &p.host) else {
                    ctx.log("autofill: no saved login for this site");
                    return None;
                };
                let next = passwords::fill(ctx, &service, &account);
                self.pending = Some(Pending { request: next, view: p.view, host: p.host, stage: Stage::Reading { account } });
                None
            }
            Stage::Reading { account } => match passwords::decode_password(result) {
                Ok(password) => Some((p.view, fill_js(&account, &password))),
                Err(_) => {
                    ctx.log("autofill: the saved password could not be read");
                    None
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(service: &str, account: &str, modified: u64) -> Row {
        Row { service: service.into(), account: account.into(), modified }
    }

    #[test]
    fn hosts_match_the_site_and_its_subdomains_only() {
        assert_eq!(bare_host("https://WWW.Example.com:8443/login?x#y"), "example.com");
        assert!(host_matches("example.com", "https://login.example.com/a"));
        assert!(host_matches("https://example.com", "www.example.com"));
        assert!(!host_matches("example.com", "evilexample.com"));
        assert!(!host_matches("login.example.com", "example.com"));
        assert!(!host_matches("", "example.com"));
    }

    #[test]
    fn the_newest_login_for_the_host_wins_and_notes_are_skipped() {
        let rows = vec![
            row("example.com", "old", 10),
            row("example.com", "new", 20),
            row("note:example.com", "note", 99),
            row("other.org", "x", 50),
        ];
        assert_eq!(pick(&rows, "www.example.com"), Some(("example.com".into(), "new".into())));
        assert_eq!(pick(&rows, "nothing.test"), None);
    }

    #[test]
    fn js_strings_cannot_end_the_literal() {
        assert_eq!(js_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(js_string("x\n</script>"), "\"x\\n\\u003C/script>\"");
        assert_eq!(js_string("\u{1}"), "\"\\u0001\"");
    }

    #[test]
    fn the_fill_script_carries_escaped_values() {
        let js = fill_js("me@x", "p\"w");
        assert!(js.starts_with(FILL_PRE));
        assert!(js.contains("\"me@x\""));
        assert!(js.contains("\"p\\\"w\""));
        assert!(!js.contains('\n'));
    }

    #[test]
    fn form_replies_set_and_clear_the_flag() {
        let mut a = AutoFill::default();
        let v: WebViewId = 7;
        assert!(a.on_eval(v, "bifrost-form:1"));
        assert!(a.has_form(v));
        assert!(a.on_eval(v, "bifrost-form:0"));
        assert!(!a.has_form(v));
        assert!(a.on_eval(v, "bifrost-fill:1"));
        assert!(!a.on_eval(v, "bifrost-fav:x"));
    }
}
