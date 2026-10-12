//! The link a widget opens when it is tapped (`widgetURL(_:)` and `Link` in a widget). A link names an app and a
//! path inside it, written as `bifrost-app://<app>/<path>`. Pure model: parsing and validation only. The shell
//! routes a valid link to the app; nothing here opens a window.

/// The scheme every widget link starts with.
pub const SCHEME: &str = "bifrost-app://";

/// Longest app identifier accepted.
const MAX_APP_ID: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    app: String,
    path: String,
}

/// Why a link is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// The text does not start with `bifrost-app://`.
    NotAppScheme,
    /// The app identifier is empty.
    MissingApp,
    /// The app identifier has characters other than letters, digits, `-` and `.`, or is too long.
    BadAppId,
    /// The path has a `.` or `..` segment, or a `?`, `#` or whitespace character.
    BadPath,
}

impl Link {
    /// A link to `path` inside `app`. The path is normalised: empty segments are dropped, so `a//b/` becomes `/a/b`
    /// and an empty path becomes `/`.
    pub fn new(app: &str, path: &str) -> Result<Link, LinkError> {
        if !valid_app_id(app) {
            return Err(LinkError::BadAppId);
        }
        Ok(Link { app: app.to_string(), path: normalize_path(path)? })
    }

    /// Parses a full link. A link with no path goes to the app's root, `/`.
    pub fn parse(url: &str) -> Result<Link, LinkError> {
        let rest = url.strip_prefix(SCHEME).ok_or(LinkError::NotAppScheme)?;
        let (app, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        if app.is_empty() {
            return Err(LinkError::MissingApp);
        }
        Link::new(app, path)
    }

    pub fn app(&self) -> &str {
        &self.app
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The full link text, the form `parse` reads back.
    pub fn to_url(&self) -> String {
        format!("{}{}{}", SCHEME, self.app, self.path)
    }
}

fn valid_app_id(app: &str) -> bool {
    !app.is_empty()
        && app.len() <= MAX_APP_ID
        && app.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

fn normalize_path(path: &str) -> Result<String, LinkError> {
    if path.chars().any(|c| c == '?' || c == '#' || c.is_whitespace()) {
        return Err(LinkError::BadPath);
    }
    let mut out = String::new();
    for seg in path.split('/') {
        match seg {
            "" => {}
            "." | ".." => return Err(LinkError::BadPath),
            s => {
                out.push('/');
                out.push_str(s);
            }
        }
    }
    if out.is_empty() {
        out.push('/');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_splits_app_and_path() {
        let l = Link::parse("bifrost-app://notes/folder/x").unwrap();
        assert_eq!(l.app(), "notes");
        assert_eq!(l.path(), "/folder/x");
        assert_eq!(l.to_url(), "bifrost-app://notes/folder/x");
    }

    #[test]
    fn link_without_path_goes_to_root() {
        let l = Link::parse("bifrost-app://notes").unwrap();
        assert_eq!(l.path(), "/");
        assert_eq!(l.to_url(), "bifrost-app://notes/");
    }

    #[test]
    fn other_schemes_are_refused() {
        assert_eq!(Link::parse("https://example.com/a"), Err(LinkError::NotAppScheme));
        assert_eq!(Link::parse("bifrost-app:///x"), Err(LinkError::MissingApp));
    }

    #[test]
    fn dot_segments_are_refused() {
        assert_eq!(Link::parse("bifrost-app://notes/a/../b"), Err(LinkError::BadPath));
        assert_eq!(Link::new("notes", "./a"), Err(LinkError::BadPath));
    }

    #[test]
    fn query_fragment_and_space_are_refused() {
        assert_eq!(Link::parse("bifrost-app://notes/a?x=1"), Err(LinkError::BadPath));
        assert_eq!(Link::parse("bifrost-app://notes/a#top"), Err(LinkError::BadPath));
        assert_eq!(Link::new("notes", "/my file"), Err(LinkError::BadPath));
    }

    #[test]
    fn empty_segments_are_dropped() {
        let l = Link::new("notes", "a//b/").unwrap();
        assert_eq!(l.to_url(), "bifrost-app://notes/a/b");
    }

    #[test]
    fn bad_app_identifiers_are_refused() {
        assert_eq!(Link::new("No Good", "/"), Err(LinkError::BadAppId));
        assert_eq!(Link::new("", "/"), Err(LinkError::BadAppId));
        assert_eq!(Link::new("com.example-app.2", "/").unwrap().app(), "com.example-app.2");
    }
}
