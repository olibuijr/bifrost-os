//! The Deildu site: category listings, search and paging (zero-based),
//! detail pages and the compatibility rule for releases.

use super::http::Client;
pub use deildu_rust::listings::{Item, Listings, SITE};
use scraper::{Html, Selector};
use url::Url;

/// Library tabs: name and the site's category id.
pub const CATEGORIES: [(&str, &str); 3] = [("Movies", "6"), ("TV", "8"), ("XXX", "4")];
const PAGE_LIMIT: usize = 4 * 1024 * 1024;
const ART_LIMIT: usize = 8 * 1024 * 1024;
const TORRENT_LIMIT: usize = 10 * 1024 * 1024;
const DESCRIPTION_CHARS: usize = 1500;

/// A fetch that may instead discover the session has expired.
#[derive(Clone, Debug, PartialEq)]
pub enum Fetch<T> {
    Ok(T),
    LoginNeeded,
    Err(String),
}

pub fn browse_url(cat: &str, page: u32, query: &str) -> String {
    let mut u = Url::parse(SITE).unwrap().join("browse.php").unwrap();
    u.query_pairs_mut().append_pair("cat", cat).append_pair("page", &page.to_string()).append_pair("search", query);
    u.to_string()
}

/// Whether a page is the sign-in page (by its URL or by a password field).
pub fn is_login_page(url: &str, html: &str) -> bool {
    if Url::parse(url).map(|u| u.path().to_ascii_lowercase().contains("login")).unwrap_or(false) {
        return true;
    }
    let doc = Html::parse_document(html);
    doc.select(&Selector::parse("input[type=password]").unwrap()).next().is_some() && !html.contains("browse.php")
}

fn fetch_html(client: &Client, url: &str) -> Fetch<(String, String)> {
    match client.get(url, None, PAGE_LIMIT) {
        Ok(r) if r.status == 200 => {
            let text = r.text();
            if is_login_page(&r.url, &text) {
                Fetch::LoginNeeded
            } else {
                Fetch::Ok((text, r.url))
            }
        }
        Ok(r) => Fetch::Err(format!("Site returned {}", r.status)),
        Err(e) => Fetch::Err(format!("Connection failed: {e}")),
    }
}

pub fn fetch_listings(client: &Client, cat: &str, page: u32, query: &str) -> Fetch<Listings> {
    match fetch_html(client, &browse_url(cat, page, query)) {
        Fetch::Ok((html, url)) => deildu_rust::listings::parse(&html, &url).map_or_else(Fetch::Err, Fetch::Ok),
        Fetch::LoginNeeded => Fetch::LoginNeeded,
        Fetch::Err(e) => Fetch::Err(e),
    }
}

/// What the detail page adds to a listing row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub description: String,
    pub artwork: Option<String>,
    pub torrent_url: Option<String>,
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The release description: the cell after a "Description"/"Lýsing" heading,
/// else the longest leaf table cell.
pub fn description(html: &str) -> String {
    let doc = Html::parse_document(html);
    let td = Selector::parse("td").unwrap();
    let cells: Vec<_> = doc.select(&td).collect();
    let heading = cells.iter().position(|c| matches!(squash(&c.text().collect::<String>()).to_lowercase().as_str(), "description" | "lýsing" | "lysing" | "descr"));
    let text = match heading.and_then(|i| cells.get(i + 1)) {
        Some(c) => squash(&c.text().collect::<String>()),
        None => {
            let table = Selector::parse("table").unwrap();
            cells.iter().filter(|c| c.select(&table).next().is_none()).map(|c| squash(&c.text().collect::<String>())).max_by_key(String::len).unwrap_or_default()
        }
    };
    let mut out: String = text.chars().take(DESCRIPTION_CHARS).collect();
    if text.chars().count() > DESCRIPTION_CHARS {
        out.push('…');
    }
    out
}

pub fn parse_details(html: &str, url: &str) -> Details {
    Details { description: description(html), artwork: deildu_rust::listings::artwork_link(html), torrent_url: deildu_rust::listings::download_link(html, url) }
}

pub fn fetch_details(client: &Client, url: &str) -> Fetch<Details> {
    match fetch_html(client, url) {
        Fetch::Ok((html, url)) => Fetch::Ok(parse_details(&html, &url)),
        Fetch::LoginNeeded => Fetch::LoginNeeded,
        Fetch::Err(e) => Fetch::Err(e),
    }
}

fn fetch_bytes(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>, String> {
    match client.get(url, None, limit) {
        Ok(r) if r.status == 200 && !r.body.is_empty() => Ok(r.body),
        Ok(r) => Err(format!("{} from {}", r.status, Url::parse(&r.url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default())),
        Err(e) => Err(e.to_string()),
    }
}

pub fn fetch_artwork(client: &Client, url: &str) -> Result<Vec<u8>, String> {
    fetch_bytes(client, url, ART_LIMIT)
}

pub fn fetch_torrent(client: &Client, url: &str) -> Result<Vec<u8>, String> {
    fetch_bytes(client, url, TORRENT_LIMIT)
}

/// What a release name says about playability: HEVC/x265/MKV/AVI releases are
/// out; an explicit MP4/M4V with H.264 is in; anything else needs the torrent.
pub fn name_hint(title: &str) -> Option<bool> {
    let t = title.to_ascii_lowercase().replace('.', " ");
    let has = |tags: &[&str]| tags.iter().any(|tag| t.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == *tag));
    if has(&["x265", "h265", "hevc", "mkv", "avi"]) {
        Some(false)
    } else if has(&["mp4", "m4v"]) && has(&["x264", "h264", "avc"]) {
        Some(true)
    } else {
        None
    }
}

/// Display title: the release name without its quality tags.
pub fn clean_title(title: &str) -> String {
    let t = deildu_rust::listings::media_title(title);
    if t.is_empty() {
        title.to_string()
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deildu::http::tests::{ok, Fake};
    use crate::deildu::http::Jar;

    pub const BROWSE_HTML: &str = r#"<html><body><table>
      <tr><td>1</td><td><a href="browse.php?cat=6">Movies</a></td><td><a href="details.php?id=101" title="Some.Film.2024.1080p.WEB-DL.x264-GRP">Some.Film.2024.1080p.WEB-DL.x264-GRP</a>
        <a href="download.php/101/film.torrent">dl</a></td><td></td><td></td><td></td><td>1.4 GB</td><td></td><td>12</td><td>3</td></tr>
      <tr><td>2</td><td><a href="browse.php?cat=6">Movies</a></td><td><a href="details.php?id=102" title="Other.Film.2023.720p.x265-GRP">Other.Film.2023.720p.x265-GRP</a></td><td></td><td></td><td></td><td>700 MB</td><td></td><td>2</td><td>0</td></tr>
      </table><a href="browse.php?cat=6&page=1">next</a></body></html>"#;

    #[test]
    fn urls_and_categories() {
        assert_eq!(browse_url("6", 0, ""), "https://www.deildu.net/browse.php?cat=6&page=0&search=");
        assert_eq!(browse_url("8", 2, "the deputy"), "https://www.deildu.net/browse.php?cat=8&page=2&search=the+deputy");
        assert_eq!(CATEGORIES.map(|c| c.1), ["6", "8", "4"]);
    }

    #[test]
    fn login_page_detection() {
        assert!(is_login_page("https://www.deildu.net/login.php?returnto=x", ""));
        assert!(is_login_page("https://www.deildu.net/", "<form><input type=password></form>"));
        assert!(!is_login_page("https://www.deildu.net/browse.php", r#"<a href="browse.php?cat=6">x</a><input type=password>"#));
    }

    #[test]
    fn listings_come_from_the_core_parser_with_login_bounce() {
        let fake = Fake::new(vec![&ok(BROWSE_HTML), "HTTP/1.1 302 Found\r\nLocation: /login.php\r\nContent-Length: 0\r\n\r\n", &ok("<input type=password>"), "HTTP/1.1 500 Oops\r\nContent-Length: 0\r\n\r\n"]);
        let c = Client::new(fake, Jar::default());
        let Fetch::Ok(l) = fetch_listings(&c, "6", 0, "") else { panic!() };
        assert_eq!(l.items.len(), 2);
        assert_eq!(l.items[0].id, "101");
        assert_eq!(l.items[0].info, "1.4 GB · 12 seeders · 3 leechers");
        assert!(l.items[0].torrent_url.ends_with("/download.php/101/film.torrent"));
        assert!(l.next);
        assert!(matches!(fetch_listings(&c, "6", 0, ""), Fetch::LoginNeeded));
        assert!(matches!(fetch_listings(&c, "6", 0, ""), Fetch::Err(e) if e.contains("500")));
    }

    #[test]
    fn details_description_artwork_and_torrent() {
        let html = r#"<html><table>
          <tr><td class="heading">Lýsing</td><td>A  film about\n things.<br>More.</td></tr>
          <tr><td>Img</td><td><img src="http://lookpic.com/a.jpg"></td></tr>
          <tr><td><a href="download.php/5/x.torrent">get</a></td></tr></table></html>"#;
        let d = parse_details(html, "https://www.deildu.net/details.php?id=5");
        assert_eq!(d.description, "A film about\\n things.More.");
        assert_eq!(d.artwork.as_deref(), Some("https://lookpic.com/a.jpg"));
        assert!(d.torrent_url.unwrap().ends_with("/download.php/5/x.torrent"));
        let plain = "<table><tr><td>short</td><td>a much longer leaf cell with the story</td><td><table><tr><td>nested nested nested nested nested</td></tr></table></td></tr></table>";
        assert_eq!(description(plain), "a much longer leaf cell with the story");
        let long = format!("<table><tr><td>{}</td></tr></table>", "x".repeat(2000));
        assert_eq!(description(&long).chars().count(), DESCRIPTION_CHARS + 1);
    }

    #[test]
    fn fetches_bytes_with_status_checks() {
        let fake = Fake::new(vec![&ok("JPEGDATA"), "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"]);
        let c = Client::new(fake, Jar::default());
        assert_eq!(fetch_artwork(&c, "https://www.deildu.net/a.jpg").unwrap(), b"JPEGDATA");
        assert_eq!(fetch_torrent(&c, "https://www.deildu.net/download.php/1/x.torrent").unwrap_err(), "404 from www.deildu.net");
    }

    #[test]
    fn name_hints_follow_the_compatibility_rule() {
        assert_eq!(name_hint("Film.2024.1080p.x265-GRP"), Some(false));
        assert_eq!(name_hint("Film 2024 HEVC"), Some(false));
        assert_eq!(name_hint("Show.S01E01.720p.MKV"), Some(false));
        assert_eq!(name_hint("Film.2024.1080p.WEB-DL.x264.MP4-GRP"), Some(true));
        assert_eq!(name_hint("Film.2024.1080p.WEB-DL.x264-GRP"), None);
        assert_eq!(clean_title("Some.Film.2024.1080p.WEB-DL.x264-GRP"), "Some Film 2024");
        assert_eq!(clean_title("1080p"), "1080p", "never blank");
    }
}
