//! Signing in: the site's login form is fetched, filled and posted directly
//! (the bifrost app used WebKit for this). The cookie jar is the session; it is
//! kept in memory and saved 0600 under the app's data directory.

use super::http::{Client, Jar};
use super::site::{self, SITE};
use scraper::{ElementRef, Html, Selector};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use url::Url;

const LIMIT: usize = 2 * 1024 * 1024;

/// The sign-in form as found on the login page.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    /// absolute URL to POST to
    pub action: String,
    pub user_field: String,
    pub pass_field: String,
    /// hidden inputs and named submit buttons, sent as they are
    pub extra: Vec<(String, String)>,
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).unwrap()
}

fn input_type(e: &ElementRef) -> String {
    e.value().attr("type").unwrap_or("text").to_ascii_lowercase()
}

/// Finds the form with a password field and names its inputs.
pub fn parse_form(html: &str, base: &str) -> Option<Form> {
    let base = Url::parse(base).ok()?;
    let doc = Html::parse_document(html);
    let inputs = sel("input");
    for form in doc.select(&sel("form")) {
        let all: Vec<ElementRef> = form.select(&inputs).collect();
        let Some(pass) = all.iter().find(|i| input_type(i) == "password") else { continue };
        let pass_field = pass.value().attr("name").unwrap_or("password").to_string();
        let user_field = all
            .iter()
            .filter(|i| matches!(input_type(i).as_str(), "text" | "email"))
            .map(|i| i.value().attr("name").unwrap_or("username"))
            .find(|n| !n.is_empty())
            .unwrap_or("username")
            .to_string();
        let extra = all
            .iter()
            .filter(|i| matches!(input_type(i).as_str(), "hidden" | "submit"))
            .filter_map(|i| Some((i.value().attr("name")?.to_string(), i.value().attr("value").unwrap_or("").to_string())))
            .filter(|(n, _)| !n.is_empty() && n != &user_field && n != &pass_field)
            .collect();
        let action = base.join(form.value().attr("action").unwrap_or("")).ok()?;
        if action.origin() != base.origin() {
            continue;
        }
        return Some(Form { action: action.to_string(), user_field, pass_field, extra });
    }
    None
}

/// What the site's login page is expected to look like when it cannot be
/// read, following the TBDev convention Deildu's other pages share.
/// LEAD: verify on the device that a sign-in with a real account succeeds;
/// if it fails, compare the form fields of the live login page with these.
fn conventional_form() -> Form {
    Form { action: format!("{SITE}takelogin.php"), user_field: "username".into(), pass_field: "password".into(), extra: vec![] }
}

/// Signs in; Ok when a listing page is reachable afterwards.
pub fn login(client: &Client, user: &str, pass: &str) -> Result<(), String> {
    let page = client.get(&format!("{SITE}login.php"), None, LIMIT).map_err(|e| format!("Connection failed: {e}"))?;
    if page.status != 200 {
        return Err(format!("Login page returned {}", page.status));
    }
    let form = parse_form(&page.text(), &page.url).unwrap_or_else(conventional_form);
    let mut fields: Vec<(&str, &str)> = vec![(&form.user_field, user), (&form.pass_field, pass)];
    fields.extend(form.extra.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    client.post_form(&form.action, &fields, LIMIT).map_err(|e| format!("Sign-in failed: {e}"))?;
    let check = client.get(&site::browse_url("6", 0, ""), None, LIMIT).map_err(|e| format!("Connection failed: {e}"))?;
    if site::is_login_page(&check.url, &check.text()) {
        return Err("Sign-in failed. Check the username and password.".into());
    }
    Ok(())
}

pub fn session_path(dir: &Path) -> PathBuf {
    dir.join("session")
}

/// Saves the jar readable by the owner only.
pub fn save_session(dir: &Path, jar: &Jar) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(session_path(dir))?;
    f.write_all(jar.save().as_bytes())
}

pub fn load_session(dir: &Path) -> Jar {
    std::fs::read_to_string(session_path(dir)).map(|s| Jar::load(&s)).unwrap_or_default()
}

pub fn clear_session(dir: &Path) {
    let _ = std::fs::remove_file(session_path(dir));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deildu::http::tests::{ok, Fake};
    use std::os::unix::fs::PermissionsExt;

    const LOGIN_HTML: &str = r#"<html><body>
      <form method="post" action="takelogin.php">
        <input type="hidden" name="returnto" value="/" />
        <input type="text" name="username" />
        <input type="password" name="password" />
        <input type="submit" name="login" value="Sign in" />
      </form></body></html>"#;

    #[test]
    fn parses_the_form_with_hidden_fields() {
        let f = parse_form(LOGIN_HTML, "https://www.deildu.net/login.php").unwrap();
        assert_eq!(f.action, "https://www.deildu.net/takelogin.php");
        assert_eq!((f.user_field.as_str(), f.pass_field.as_str()), ("username", "password"));
        assert_eq!(f.extra, vec![("returnto".to_string(), "/".to_string()), ("login".to_string(), "Sign in".to_string())]);
        assert!(parse_form("<form><input name=q></form>", "https://x/").is_none(), "no password field");
        let foreign = r#"<form action="https://evil.net/x"><input type=password name=p></form>"#;
        assert!(parse_form(foreign, "https://www.deildu.net/login.php").is_none(), "cross-origin form");
        let bare = r#"<form action="/in"><input name=u><input type=password name=p></form>"#;
        assert_eq!(parse_form(bare, "https://h/login").unwrap().user_field, "u");
    }

    #[test]
    fn login_posts_the_form_and_checks_a_listing() {
        let listing = ok(r#"<html><a href="/browse.php?cat=6">Movies</a></html>"#);
        let fake = Fake::new(vec![&ok(LOGIN_HTML), "HTTP/1.1 302 Found\r\nLocation: /my.php\r\nSet-Cookie: pass=1\r\nContent-Length: 0\r\n\r\n", &ok("<html>my</html>"), &listing]);
        let c = Client::new(fake.clone(), Jar::default());
        assert_eq!(login(&c, "me", "pw"), Ok(()));
        let reqs = fake.requests.lock().unwrap().clone();
        assert!(reqs[1].starts_with("POST /takelogin.php"));
        assert!(reqs[1].ends_with("username=me&password=pw&returnto=%2F&login=Sign+in"));
        assert!(reqs[3].starts_with("GET /browse.php?cat=6&page=0&search= "));
        assert!(reqs[3].contains("Cookie: pass=1"));
    }

    #[test]
    fn login_fails_when_the_listing_bounces_to_the_login_page() {
        let fake = Fake::new(vec![&ok(LOGIN_HTML), &ok("Login failed"), "HTTP/1.1 302 Found\r\nLocation: /login.php\r\nContent-Length: 0\r\n\r\n", &ok(LOGIN_HTML)]);
        let c = Client::new(fake, Jar::default());
        assert!(login(&c, "me", "bad").unwrap_err().contains("Sign-in failed"));
        let down = Fake::new(vec![]);
        assert!(login(&Client::new(down, Jar::default()), "me", "pw").unwrap_err().contains("Connection failed"));
    }

    #[test]
    fn session_file_is_private_and_round_trips() {
        let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/test-deildu-auth"));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load_session(&dir).is_empty());
        let mut jar = Jar::default();
        jar.set("www.deildu.net", "uid=7");
        save_session(&dir, &jar).unwrap();
        let mode = std::fs::metadata(session_path(&dir)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(load_session(&dir), jar);
        clear_session(&dir);
        assert!(load_session(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
