//! Help books and the shell's help index: the counterpart of the `helpd` daemon.
//!
//! An app ships a help book in the directory its manifest names (`help = "help"`). The book is Markdown pages, one
//! per topic, each starting with front matter:
//!
//! ```text
//! ---
//! title: Open a file
//! keywords: open, double click, finder
//! ---
//! ## Open a file {#open}
//! ```
//!
//! A `{#id}` after a heading names its anchor (`help://files/open`); without one the anchor is the heading slugified.
//! `HelpIndex` reads every installed book (`rebuild`, on start and after an install or update), answers ranked
//! searches over titles, keywords, headings and body text, resolves `help://app/anchor` links, and searches the
//! menu items of the frontmost app for the Help menu's search field. Pure logic: the Help viewer window draws what
//! it returns.

use std::collections::BTreeMap;
use std::path::Path;

use crate::shell::store::install::parse_manifest;

/// The scheme of help links: `help://app/anchor`, `help://app/page`, `help://app/page#anchor` or `help://app`.
pub const SCHEME: &str = "help://";
/// A help page larger than this is refused (help pages are short; a huge file is a mistake).
const MAX_PAGE_BYTES: u64 = 256 * 1024;

/// A heading that can be linked to: `help://app/<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub id: String,
    pub title: String,
}

/// One help topic: a Markdown page of one app's book.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub app: String,
    /// The file name without `.md`.
    pub slug: String,
    pub title: String,
    pub keywords: Vec<String>,
    /// The headings that carry an anchor, in document order.
    pub anchors: Vec<Anchor>,
    /// The body after the front matter. Headings are rewritten without their `{#id}` marker.
    pub body: String,
}

/// A ranked search result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub app: String,
    pub app_name: String,
    pub slug: String,
    pub title: String,
    /// The first heading of the page that matches the query, to scroll to.
    pub anchor: Option<String>,
    pub score: u32,
}

/// Where a help link points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub app: String,
    pub slug: String,
    pub anchor: Option<String>,
}

/// One item of an app's menu bar, as the Help menu search sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// The menu it belongs to (`Edit`).
    pub menu: String,
    /// The item title (`Copy`).
    pub title: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrontMatter {
    pub title: Option<String>,
    pub keywords: Vec<String>,
}

fn unquote(v: &str) -> &str {
    let v = v.trim();
    v.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(v)
}

/// Splits `---` front matter from the body. A page without front matter is all body.
/// The front matter is flat `key: value` lines; `keywords` is a comma list, optionally in brackets.
pub fn parse_front_matter(text: &str) -> Result<(FrontMatter, &str), String> {
    let mut fm = FrontMatter::default();
    let Some(rest) = text.strip_prefix("---\n") else {
        return Ok((fm, text));
    };
    let mut offset = 4;
    let mut closed = false;
    for raw in rest.split_inclusive('\n') {
        offset += raw.len();
        let line = raw.trim_end();
        if line == "---" {
            closed = true;
            break;
        }
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once(':').ok_or_else(|| format!("front matter: expected key: value, got {line:?}"))?;
        let value = value.trim();
        match key.trim() {
            "title" => fm.title = Some(unquote(value).to_string()).filter(|t| !t.is_empty()),
            "keywords" => {
                let list = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')).unwrap_or(value);
                fm.keywords = list.split(',').map(|k| unquote(k).trim().to_string()).filter(|k| !k.is_empty()).collect();
            }
            _ => {}
        }
    }
    if !closed {
        return Err("front matter is not closed by ---".into());
    }
    Ok((fm, &text[offset.min(text.len())..]))
}

/// A link-safe name: lowercase ASCII letters and digits, other runs become one `-`.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// An ATX heading line: `(level, title without the {#id} marker, anchor id)`. The id is the `{#id}` marker, else
/// the slugified title (empty when the title has no ASCII letters or digits).
pub fn heading(line: &str) -> Option<(usize, String, String)> {
    let level = line.bytes().take_while(|b| *b == b'#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &line[level..];
    if !rest.starts_with(' ') {
        return None;
    }
    let text = rest.trim();
    let (title, id) = match text.rfind("{#") {
        Some(i) if text.ends_with('}') && i + 2 < text.len() - 1 => {
            (text[..i].trim_end().to_string(), text[i + 2..text.len() - 1].trim().to_string())
        }
        _ => (text.to_string(), String::new()),
    };
    let id = if id.is_empty() { slug(&title) } else { id };
    Some((level, title, id))
}

/// Parses one help page. `slug_name` is the file name without `.md`; the title is the front matter's `title`, else
/// the first heading, else the slug.
pub fn parse_page(app: &str, slug_name: &str, text: &str) -> Result<Page, String> {
    let (fm, body_text) = parse_front_matter(text)?;
    let mut anchors = Vec::new();
    let mut body = String::new();
    let mut first_title: Option<String> = None;
    for line in body_text.lines() {
        if let Some((level, title, id)) = heading(line) {
            if first_title.is_none() {
                first_title = Some(title.clone());
            }
            if !id.is_empty() {
                anchors.push(Anchor { id, title: title.clone() });
            }
            body.push_str(&"#".repeat(level));
            body.push(' ');
            body.push_str(&title);
        } else {
            body.push_str(line);
        }
        body.push('\n');
    }
    let title = fm.title.or(first_title).unwrap_or_else(|| slug_name.to_string());
    Ok(Page { app: app.to_string(), slug: slug_name.to_string(), title, keywords: fm.keywords, anchors, body })
}

/// Lowercase words of a text (letters and digits, any script).
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// The score of a page for the query terms, or None when some term matches nowhere (every term must match).
/// Weights: a title word 30 (prefix 15), a keyword 20 (prefix 10), a heading 8, a body word 1 to 4.
fn score_page(page: &Page, terms: &[String]) -> Option<(u32, Option<String>)> {
    let title = words(&page.title);
    let keys: Vec<String> = page.keywords.iter().flat_map(|k| words(k)).collect();
    let body = words(&page.body);
    let mut score = 0u32;
    let mut anchor: Option<String> = None;
    for t in terms {
        let mut matched = false;
        if title.iter().any(|w| w == t) {
            score += 30;
            matched = true;
        } else if title.iter().any(|w| w.starts_with(t.as_str())) {
            score += 15;
            matched = true;
        }
        if keys.iter().any(|w| w == t) {
            score += 20;
            matched = true;
        } else if keys.iter().any(|w| w.starts_with(t.as_str())) {
            score += 10;
            matched = true;
        }
        for a in &page.anchors {
            if words(&a.title).iter().any(|w| w.starts_with(t.as_str())) {
                score += 8;
                matched = true;
                if anchor.is_none() {
                    anchor = Some(a.id.clone());
                }
                break;
            }
        }
        let n = body.iter().filter(|w| w.starts_with(t.as_str())).count();
        if n > 0 {
            score += 1 + n.min(3) as u32;
            matched = true;
        }
        if !matched {
            return None;
        }
    }
    Some((score, anchor))
}

/// The built-in help books the shell itself ships (Settings is a shell page, not an installed app).
/// Each entry is `(app id, app name, pages)`.
pub fn builtin_books() -> Vec<(String, String, Vec<Page>)> {
    let app = "is.olibuijr.settings";
    let sources = [
        ("overview", include_str!("../../help/settings/overview.md")),
        ("appearance", include_str!("../../help/settings/appearance.md")),
        ("desktop-dock", include_str!("../../help/settings/desktop-dock.md")),
    ];
    let pages = sources.iter().filter_map(|(slug_name, text)| parse_page(app, slug_name, text).ok()).collect();
    vec![(app.to_string(), "Settings".to_string(), pages)]
}

/// The help index: every book the shell knows, keyed by app id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HelpIndex {
    /// app id -> display name
    apps: BTreeMap<String, String>,
    pages: Vec<Page>,
}

impl HelpIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn app_name(&self, app: &str) -> Option<&str> {
        self.apps.get(app).map(String::as_str)
    }

    /// The pages of one app's book, in file order (the table of contents).
    pub fn pages_of(&self, app: &str) -> Vec<&Page> {
        self.pages.iter().filter(|p| p.app == app).collect()
    }

    /// Replaces an app's book (an install or an update).
    pub fn set_book(&mut self, app: &str, app_name: &str, pages: Vec<Page>) {
        self.remove_book(app);
        self.apps.insert(app.to_string(), app_name.to_string());
        self.pages.extend(pages);
    }

    pub fn remove_book(&mut self, app: &str) {
        self.apps.remove(app);
        self.pages.retain(|p| p.app != app);
    }

    /// Reads one book directory: every `*.md` file at its top level, in name order. A page that does not parse fails
    /// the whole book, so a broken book is reported instead of half-indexed.
    pub fn read_book(app: &str, dir: &Path) -> Result<Vec<Page>, String> {
        let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".md"))
            .collect();
        names.sort();
        let mut pages = Vec::with_capacity(names.len());
        for name in names {
            let path = dir.join(&name);
            let size = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.len();
            if size > MAX_PAGE_BYTES {
                return Err(format!("{}: help page is larger than {MAX_PAGE_BYTES} bytes", path.display()));
            }
            let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            pages.push(parse_page(app, &name[..name.len() - 3], &text).map_err(|e| format!("{}: {e}", path.display()))?);
        }
        Ok(pages)
    }

    /// Rebuilds the whole index: the built-in books, then every installed app under `apps_dir`
    /// (`<id>/current/manifest.toml`, its book at the manifest's `help` directory). Returns the problems found
    /// (unreadable manifests, broken books); the other books are still indexed.
    pub fn rebuild(&mut self, apps_dir: &Path) -> Vec<String> {
        *self = HelpIndex::new();
        for (app, name, pages) in builtin_books() {
            self.set_book(&app, &name, pages);
        }
        let mut problems = Vec::new();
        let Ok(entries) = std::fs::read_dir(apps_dir) else {
            return problems;
        };
        let mut dirs: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs {
            let current = dir.join("current");
            let Ok(text) = std::fs::read_to_string(current.join("manifest.toml")) else {
                continue;
            };
            let manifest = match parse_manifest(&text) {
                Ok(m) => m,
                Err(e) => {
                    problems.push(format!("{}: {e}", current.display()));
                    continue;
                }
            };
            if manifest.help.is_empty() {
                continue;
            }
            if manifest.help.starts_with('/') || manifest.help.split('/').any(|s| s == "..") {
                problems.push(format!("{}: help must be a relative directory", current.display()));
                continue;
            }
            match Self::read_book(&manifest.id, &current.join(&manifest.help)) {
                Ok(pages) => self.set_book(&manifest.id, &manifest.name, pages),
                Err(e) => problems.push(e),
            }
        }
        problems
    }

    /// Ranked search over every book. Every query word must match somewhere on a page; a page scores more for a
    /// title, keyword or heading match than for body words. Ties sort by app, then page name.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let terms = words(query);
        if terms.is_empty() || limit == 0 {
            return Vec::new();
        }
        let mut hits: Vec<Hit> = self
            .pages
            .iter()
            .filter_map(|p| {
                let (score, anchor) = score_page(p, &terms)?;
                Some(Hit {
                    app: p.app.clone(),
                    app_name: self.app_name(&p.app).unwrap_or(p.app.as_str()).to_string(),
                    slug: p.slug.clone(),
                    title: p.title.clone(),
                    anchor,
                    score,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.app.cmp(&b.app)).then_with(|| a.slug.cmp(&b.slug)));
        hits.truncate(limit);
        hits
    }

    /// Resolves a help link. `help://app` is the app's first page. `help://app/<page>` is a page, and
    /// `help://app/<page>#<anchor>` a page scrolled to an anchor. `help://app/<anchor>` is the page that declares the
    /// anchor. None when the app has no book or nothing matches.
    pub fn resolve(&self, url: &str) -> Option<Target> {
        let rest = url.strip_prefix(SCHEME)?;
        let (app, path) = rest.split_once('/').unwrap_or((rest, ""));
        if app.is_empty() || !self.apps.contains_key(app) {
            return None;
        }
        let path = path.trim_matches('/');
        let pages = self.pages_of(app);
        let (name, frag) = match path.split_once('#') {
            Some((n, f)) => (n, Some(f)),
            None => (path, None),
        };
        if let Some(p) = pages.iter().find(|p| !name.is_empty() && p.slug == name) {
            let anchor = frag.filter(|f| p.anchors.iter().any(|a| a.id == *f)).map(str::to_string);
            return Some(Target { app: app.to_string(), slug: p.slug.clone(), anchor });
        }
        let wanted = frag.unwrap_or(name);
        if wanted.is_empty() {
            return pages.first().map(|p| Target { app: app.to_string(), slug: p.slug.clone(), anchor: None });
        }
        pages
            .iter()
            .find(|p| p.anchors.iter().any(|a| a.id == wanted))
            .map(|p| Target { app: app.to_string(), slug: p.slug.clone(), anchor: Some(wanted.to_string()) })
    }
}

/// Indices of the menu items the Help menu search shows for `query`, best first. Every query word must start a
/// word of the item's title (a whole-word title match ranks higher), or of its menu's title. Enabled items rank above
/// disabled ones, which are still listed (dimmed), as the reference Help menu does.
pub fn search_menu(items: &[MenuItem], query: &str) -> Vec<usize> {
    let terms = words(query);
    if terms.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(u32, usize)> = Vec::new();
    'items: for (i, item) in items.iter().enumerate() {
        let title = words(&item.title);
        let menu = words(&item.menu);
        let mut score = 0u32;
        for t in &terms {
            if title.iter().any(|w| w == t) {
                score += 20;
            } else if title.iter().any(|w| w.starts_with(t.as_str())) {
                score += 10;
            } else if menu.iter().any(|w| w.starts_with(t.as_str())) {
                score += 3;
            } else {
                continue 'items;
            }
        }
        if item.enabled {
            score += 5;
        }
        scored.push((score, i));
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "---\ntitle: Open a file\nkeywords: open, double click, finder\n---\n## Open a file {#open}\nDouble click a file to open it in its app.\n## Save a copy {#copy}\nChoose Duplicate to keep a copy.\n";

    fn page(app: &str, slug_name: &str, text: &str) -> Page {
        parse_page(app, slug_name, text).unwrap()
    }

    fn index() -> HelpIndex {
        let mut ix = HelpIndex::new();
        ix.set_book(
            "is.olibuijr.files.desktop",
            "Files",
            vec![
                page("is.olibuijr.files.desktop", "open", PAGE),
                page("is.olibuijr.files.desktop", "tags", "---\ntitle: Tags\nkeywords: [label, color]\n---\n## Colour tags {#colour}\nGive a file a tag.\n"),
            ],
        );
        ix.set_book(
            "is.olibuijr.textedit.desktop",
            "TextEdit",
            vec![page("is.olibuijr.textedit.desktop", "format", "# Format text\nUse the Format menu to make text bold. Open the font panel to change the font.\n")],
        );
        ix
    }

    #[test]
    fn front_matter_gives_title_and_keywords() {
        let (fm, body) = parse_front_matter(PAGE).unwrap();
        assert_eq!(fm.title.as_deref(), Some("Open a file"));
        assert_eq!(fm.keywords, vec!["open", "double click", "finder"]);
        assert!(body.starts_with("## Open a file"));
    }

    #[test]
    fn front_matter_accepts_bracketed_keywords_and_quotes() {
        let (fm, _) = parse_front_matter("---\ntitle: \"Quoted: title\"\nkeywords: [a, 'b c']\n---\nbody").unwrap();
        assert_eq!(fm.title.as_deref(), Some("Quoted: title"));
        assert_eq!(fm.keywords, vec!["a", "b c"]);
    }

    #[test]
    fn front_matter_is_optional_and_rejects_bad_input() {
        let (fm, body) = parse_front_matter("# Plain\ntext").unwrap();
        assert_eq!(fm, FrontMatter::default());
        assert_eq!(body, "# Plain\ntext");
        assert!(parse_front_matter("---\ntitle: never closed\n").is_err());
        assert!(parse_front_matter("---\nno colon here\n---\n").is_err());
    }

    #[test]
    fn title_falls_back_to_first_heading_then_slug() {
        assert_eq!(page("a.b", "one", "## Second level\ntext").title, "Second level");
        assert_eq!(page("a.b", "two", "just text").title, "two");
    }

    #[test]
    fn heading_anchors_use_the_marker_or_the_slug() {
        assert_eq!(heading("## Save a copy {#copy}"), Some((2, "Save a copy".to_string(), "copy".to_string())));
        assert_eq!(heading("### Get Info"), Some((3, "Get Info".to_string(), "get-info".to_string())));
        assert_eq!(heading("#no-space"), None);
        assert_eq!(heading("plain"), None);
        let p = page("a.b", "x", PAGE);
        assert_eq!(p.anchors.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["open", "copy"]);
        assert!(p.body.contains("## Save a copy\n"), "the marker is stripped from the body: {:?}", p.body);
    }

    #[test]
    fn search_ranks_title_over_keyword_over_body() {
        let ix = index();
        let hits = ix.search("open", 10);
        assert_eq!(hits[0].slug, "open", "title and heading match first");
        assert_eq!(hits[0].anchor.as_deref(), Some("open"));
        assert!(hits.iter().any(|h| h.app == "is.olibuijr.textedit.desktop"), "body words still match");
        let tag = ix.search("label", 10);
        assert_eq!(tag.len(), 1);
        assert_eq!(tag[0].slug, "tags");
        assert_eq!(tag[0].app_name, "Files");
    }

    #[test]
    fn every_query_word_must_match_and_prefixes_match() {
        let ix = index();
        assert!(ix.search("open nonsense", 10).is_empty());
        let hits = ix.search("dupl", 10);
        assert_eq!(hits.len(), 1, "a prefix of a body word matches");
        assert_eq!(hits[0].slug, "open");
        assert!(ix.search("   ", 10).is_empty());
        assert_eq!(ix.search("open", 1).len(), 1, "the limit holds");
    }

    #[test]
    fn set_book_replaces_an_update_and_remove_drops_it() {
        let mut ix = index();
        let before = ix.page_count();
        ix.set_book("is.olibuijr.files.desktop", "Files", vec![page("is.olibuijr.files.desktop", "only", "# Only\nnew text")]);
        assert_eq!(ix.page_count(), before - 1);
        assert_eq!(ix.pages_of("is.olibuijr.files.desktop").len(), 1);
        ix.remove_book("is.olibuijr.textedit.desktop");
        assert_eq!(ix.app_name("is.olibuijr.textedit.desktop"), None);
        assert!(ix.search("bold", 5).is_empty());
    }

    #[test]
    fn links_resolve_to_page_and_anchor() {
        let ix = index();
        let app = "is.olibuijr.files.desktop";
        assert_eq!(ix.resolve("help://is.olibuijr.files.desktop/copy"), Some(Target { app: app.into(), slug: "open".into(), anchor: Some("copy".into()) }));
        assert_eq!(ix.resolve("help://is.olibuijr.files.desktop/tags#colour"), Some(Target { app: app.into(), slug: "tags".into(), anchor: Some("colour".into()) }));
        assert_eq!(ix.resolve("help://is.olibuijr.files.desktop/tags#missing"), Some(Target { app: app.into(), slug: "tags".into(), anchor: None }));
        assert_eq!(ix.resolve("help://is.olibuijr.files.desktop/tags"), Some(Target { app: app.into(), slug: "tags".into(), anchor: None }));
        assert_eq!(ix.resolve("help://is.olibuijr.files.desktop"), Some(Target { app: app.into(), slug: "open".into(), anchor: None }));
        assert_eq!(ix.resolve("help://is.olibuijr.files.desktop/#colour").map(|t| t.slug), Some("tags".to_string()));
    }

    #[test]
    fn unknown_links_do_not_resolve() {
        let ix = index();
        assert_eq!(ix.resolve("help://nobody.app/open"), None);
        assert_eq!(ix.resolve("help://is.olibuijr.files.desktop/nothing-here"), None);
        assert_eq!(ix.resolve("https://is.olibuijr.files.desktop/open"), None);
        assert_eq!(ix.resolve("help://"), None);
    }

    #[test]
    fn menu_search_ranks_whole_words_and_enabled_items() {
        let items = vec![
            MenuItem { menu: "Edit".into(), title: "Copy".into(), enabled: true },
            MenuItem { menu: "Edit".into(), title: "Copy Path".into(), enabled: true },
            MenuItem { menu: "File".into(), title: "Duplicate".into(), enabled: true },
            MenuItem { menu: "Edit".into(), title: "Paste".into(), enabled: false },
            MenuItem { menu: "Edit".into(), title: "Cut".into(), enabled: true },
        ];
        assert_eq!(search_menu(&items, "copy"), vec![0, 1], "whole-word title matches, in menu order");
        assert_eq!(search_menu(&items, "cop"), vec![0, 1]);
        assert_eq!(search_menu(&items, "edit"), vec![0, 1, 4, 3], "a menu name matches its items, enabled first");
        assert_eq!(search_menu(&items, "paste"), vec![3], "disabled items are still listed");
        assert!(search_menu(&items, "print").is_empty());
        assert!(search_menu(&items, "").is_empty());
    }

    #[test]
    fn builtin_settings_book_parses_with_anchors() {
        let books = builtin_books();
        assert_eq!(books.len(), 1);
        let (app, name, pages) = &books[0];
        assert_eq!(name, "Settings");
        assert_eq!(pages.len(), 3);
        assert!(pages.iter().all(|p| !p.anchors.is_empty()), "every built-in page declares anchors");
        let mut ix = HelpIndex::new();
        ix.set_book(app, name, pages.clone());
        assert!(!ix.search("dock", 5).is_empty());
    }

    #[test]
    fn read_book_reads_pages_in_name_order_and_refuses_broken_ones() {
        let dir = std::env::temp_dir().join(format!("bifrost-helpbook-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.md"), "## Second {#two}\nbody").unwrap();
        std::fs::write(dir.join("a.md"), "## First\nbody").unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        let pages = HelpIndex::read_book("x.y", &dir).unwrap();
        assert_eq!(pages.iter().map(|p| p.slug.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        std::fs::write(dir.join("c.md"), "---\ntitle: broken\n").unwrap();
        assert!(HelpIndex::read_book("x.y", &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
