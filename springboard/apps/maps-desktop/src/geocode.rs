//! Place search through a Nominatim-compatible geocoder: the request URL and the reply parser.
//! Nominatim's usage policy asks for an identifying user agent and at most one request per second; the
//! `Suggestor` below sends suggestions after a pause in typing and never faster than that.

use bifrost_core::json::{self, Json};

pub const DEFAULT_BASE: &str = "https://nominatim.openstreetmap.org";
pub const DEFAULT_LIMIT: u8 = 5;

#[derive(Clone, Debug, PartialEq)]
pub struct Geocoder {
    base: String,
    user_agent: String,
    limit: u8,
}

/// One search result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub details: Details,
}

/// What the geocoder knows about a place besides its name and position (`addressdetails=1`, `extratags=1`). Every
/// field is empty when the reply did not carry it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    /// what kind of place it is ("restaurant", "city", ...), from the reply's `type`
    pub kind: String,
    /// street and number, then postcode with locality, then country, as separate lines
    pub address: Vec<String>,
    pub phone: String,
    pub website: String,
    pub opening_hours: String,
}

impl Details {
    pub fn is_empty(&self) -> bool {
        self.kind.is_empty() && self.address.is_empty() && self.phone.is_empty() && self.website.is_empty() && self.opening_hours.is_empty()
    }
}

impl Place {
    pub fn new(name: &str, lat: f64, lon: f64) -> Self {
        Place { name: name.to_string(), lat, lon, details: Details::default() }
    }

    /// The name up to the first comma: the title of a place card.
    pub fn title(&self) -> &str {
        self.name.split(',').next().unwrap_or(&self.name).trim()
    }
}

impl Geocoder {
    pub fn new(base: &str, user_agent: &str) -> Result<Self, String> {
        let b = base.trim().trim_end_matches('/');
        if !(b.starts_with("https://") || b.starts_with("http://")) {
            return Err("the geocoder URL must start with http:// or https://".into());
        }
        if user_agent.trim().is_empty() {
            return Err("the geocoder needs a user agent".into());
        }
        Ok(Self { base: b.to_string(), user_agent: user_agent.trim().to_string(), limit: DEFAULT_LIMIT })
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// The search URL for a query. Empty queries are refused, so no request is made for a blank field.
    pub fn search_url(&self, query: &str) -> Result<String, String> {
        let q = query.trim();
        if q.is_empty() {
            return Err("the search is empty".into());
        }
        Ok(format!(
            "{}/search?format=jsonv2&addressdetails=1&extratags=1&limit={}&q={}",
            self.base,
            self.limit,
            percent_encode(q)
        ))
    }

    /// The places in a `format=jsonv2` reply. Coordinates arrive as strings; entries without them are skipped.
    pub fn parse(&self, body: &str) -> Result<Vec<Place>, String> {
        let doc = json::parse(body)?;
        let items = doc.as_array().ok_or("the geocoder reply is not a list")?;
        Ok(items
            .iter()
            .filter_map(|item| {
                let name = item.str_field("display_name")?;
                let lat = coord(item.get("lat")?)?;
                let lon = coord(item.get("lon")?)?;
                Some(Place { name, lat, lon, details: details_of(item) })
            })
            .collect())
    }
}

/// The address lines, kind, phone, website and opening hours of a reply entry.
fn details_of(item: &Json) -> Details {
    let mut d = Details { kind: item.str_field("type").unwrap_or_default().replace('_', " "), ..Details::default() };
    if let Some(a) = item.get("address") {
        let field = |k: &str| a.str_field(k).filter(|v| !v.is_empty());
        let street = match (field("road"), field("house_number")) {
            (Some(r), Some(n)) => Some(format!("{r} {n}")),
            (Some(r), None) => Some(r),
            _ => None,
        };
        let locality = ["city", "town", "village", "municipality", "suburb"].iter().find_map(|k| field(k));
        let post = match (field("postcode"), locality) {
            (Some(p), Some(l)) => Some(format!("{p} {l}")),
            (Some(p), None) => Some(p),
            (None, l) => l,
        };
        d.address = [street, post, field("country")].into_iter().flatten().collect();
    }
    if let Some(x) = item.get("extratags") {
        let first = |keys: &[&str]| keys.iter().find_map(|k| x.str_field(k)).filter(|v| !v.is_empty()).unwrap_or_default();
        d.phone = first(&["phone", "contact:phone"]);
        d.website = first(&["website", "contact:website", "url"]);
        d.opening_hours = first(&["opening_hours"]);
    }
    d
}

#[derive(Clone, Copy, PartialEq)]
enum Axis {
    Lat,
    Lon,
}

/// A number with an optional hemisphere letter: the signed value and the axis the letter names.
fn signed(token: &str) -> Option<(f64, Option<Axis>)> {
    let last = token.chars().last()?;
    let (body, sign, axis) = match last.to_ascii_uppercase() {
        'N' => (&token[..token.len() - 1], 1.0, Some(Axis::Lat)),
        'S' => (&token[..token.len() - 1], -1.0, Some(Axis::Lat)),
        'E' => (&token[..token.len() - 1], 1.0, Some(Axis::Lon)),
        'W' => (&token[..token.len() - 1], -1.0, Some(Axis::Lon)),
        _ => (token, 1.0, None),
    };
    let v: f64 = body.parse().ok()?;
    v.is_finite().then_some((v * sign, axis))
}

/// Latitude and longitude typed into the search field: "64.1466, -21.9426", "64.1466 -21.9426" or with hemisphere
/// letters ("64.1466 N, 21.9426 W"). `None` for anything else, so the text goes to the geocoder instead.
pub fn parse_coordinates(input: &str) -> Option<(f64, f64)> {
    let cleaned: String = input.trim().replace(['\u{b0}', ','], " ");
    let parts: Vec<&str> = cleaned.split_whitespace().collect();
    let (a, b) = match parts.as_slice() {
        [a, b] => ((*a).to_string(), (*b).to_string()),
        // "64.1466 N 21.9426 W"
        [a, ah, b, bh] => (format!("{a}{ah}"), format!("{b}{bh}")),
        _ => return None,
    };
    let (first, first_axis) = signed(&a)?;
    let (second, second_axis) = signed(&b)?;
    // hemisphere letters may come in either order ("21.9426 W 64.1466 N"); plain numbers are latitude then longitude
    let (lat, lon) = match (first_axis, second_axis) {
        (Some(Axis::Lon), Some(Axis::Lat)) => (second, first),
        (Some(x), Some(y)) if x == y => return None,
        (Some(Axis::Lon), None) | (None, Some(Axis::Lat)) => return None,
        _ => (first, second),
    };
    (lat.abs() <= 90.0 && lon.abs() <= 180.0).then_some((lat, lon))
}

/// Nominatim asks for at most one request per second.
pub const MIN_SEARCH_INTERVAL_MS: u64 = 1000;
/// Typing must pause this long before suggestions are asked for.
pub const TYPING_PAUSE_MS: u64 = 600;
/// Suggestions start at this many characters.
pub const MIN_SUGGEST_CHARS: usize = 3;

/// Decides when the text in the search field is sent to the geocoder for suggestions: after a pause in typing, never
/// faster than one request a second, never the same text twice, and not for text that is a coordinate.
#[derive(Clone, Debug, Default)]
pub struct Suggestor {
    pending: Option<(String, u64)>,
    last_sent: Option<u64>,
    last_query: String,
}

impl Suggestor {
    /// The field now holds `query`. Unchanged text keeps its original time.
    pub fn typed(&mut self, query: &str, now_ms: u64) {
        let q = query.trim();
        if self.pending.as_ref().is_some_and(|(p, _)| p == q) {
            return;
        }
        self.pending = (!q.is_empty()).then(|| (q.to_string(), now_ms));
    }

    fn worth_asking(&self) -> Option<&(String, u64)> {
        let p = self.pending.as_ref()?;
        let (q, _) = p;
        (q.chars().count() >= MIN_SUGGEST_CHARS && *q != self.last_query && parse_coordinates(q).is_none()).then_some(p)
    }

    /// The query to send now, if one is due. Call `sent` when it goes out.
    pub fn due(&self, now_ms: u64) -> Option<String> {
        let (q, at) = self.worth_asking()?;
        if now_ms.saturating_sub(*at) < TYPING_PAUSE_MS {
            return None;
        }
        if self.last_sent.is_some_and(|l| now_ms.saturating_sub(l) < MIN_SEARCH_INTERVAL_MS) {
            return None;
        }
        Some(q.clone())
    }

    /// When a suggestion request could next go out, so the app wakes for it. `None` with nothing waiting.
    pub fn next_due(&self, now_ms: u64) -> Option<u64> {
        let (_, at) = self.worth_asking()?;
        let pause = *at + TYPING_PAUSE_MS;
        let rate = self.last_sent.map_or(0, |l| l + MIN_SEARCH_INTERVAL_MS);
        Some(pause.max(rate).max(now_ms))
    }

    pub fn sent(&mut self, query: &str, now_ms: u64) {
        self.last_sent = Some(now_ms);
        self.last_query = query.trim().to_string();
    }

    /// A request made outside the suggestions (the person pressed Return) still counts against the rate limit.
    pub fn note_request(&mut self, now_ms: u64) {
        self.last_sent = Some(now_ms);
    }

    /// Milliseconds a Return-key search has to wait to respect the rate limit.
    pub fn wait_for_request(&self, now_ms: u64) -> u64 {
        self.last_sent.map_or(0, |l| (l + MIN_SEARCH_INTERVAL_MS).saturating_sub(now_ms))
    }
}

fn coord(v: &Json) -> Option<f64> {
    match v {
        Json::Str(_) => v.as_str()?.parse::<f64>().ok(),
        _ => v.number(),
    }
    .filter(|n| n.is_finite() && n.abs() <= 180.0)
}

/// Percent-encodes everything outside the RFC 3986 unreserved set, using spaces as `%20`.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decodes `%XX` escapes and `+` (as a space). Bytes that do not form UTF-8 are replaced.
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geocoder() -> Geocoder {
        Geocoder::new(DEFAULT_BASE, "BifrOSt-Maps/1.0.0 test").unwrap()
    }

    #[test]
    fn the_search_url_encodes_the_query() {
        let url = geocoder().search_url("  Laugavegur 1, Reykjavík ").unwrap();
        assert_eq!(
            url,
            "https://nominatim.openstreetmap.org/search?format=jsonv2&addressdetails=1&extratags=1&limit=5&q=Laugavegur%201%2C%20Reykjav%C3%ADk"
        );
    }

    #[test]
    fn a_blank_query_makes_no_request() {
        assert!(geocoder().search_url("   ").is_err());
    }

    #[test]
    fn replies_parse_into_places_and_skip_broken_entries() {
        let body = r#"[
            {"display_name":"Reykjavík, Iceland","lat":"64.1466","lon":"-21.9426"},
            {"display_name":"No coordinates"},
            {"display_name":"Bad","lat":"north","lon":"0"}
        ]"#;
        let places = geocoder().parse(body).unwrap();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].name, "Reykjavík, Iceland");
        assert!((places[0].lat - 64.1466).abs() < 1e-9);
        assert!((places[0].lon + 21.9426).abs() < 1e-9);
    }

    #[test]
    fn a_reply_that_is_not_a_list_is_an_error() {
        assert!(geocoder().parse(r#"{"error":"rate limited"}"#).is_err());
        assert!(geocoder().parse("not json").is_err());
    }

    #[test]
    fn the_geocoder_refuses_bad_settings() {
        assert!(Geocoder::new("ftp://x", "ua").is_err());
        assert!(Geocoder::new(DEFAULT_BASE, " ").is_err());
    }

    #[test]
    fn address_and_extra_tags_become_place_details() {
        let body = r#"[{"display_name":"Café Paris, Austurstræti 14, Reykjavík, Iceland","lat":"64.1470","lon":"-21.9390","type":"cafe",
            "address":{"road":"Austurstræti","house_number":"14","postcode":"101","city":"Reykjavík","country":"Iceland"},
            "extratags":{"contact:phone":"+354 551 1020","website":"https://cafeparis.example","opening_hours":"Mo-Su 08:00-22:00"}}]"#;
        let p = &geocoder().parse(body).unwrap()[0];
        assert_eq!(p.title(), "Café Paris");
        assert_eq!(p.details.kind, "cafe");
        assert_eq!(p.details.address, vec!["Austurstræti 14", "101 Reykjavík", "Iceland"]);
        assert_eq!(p.details.phone, "+354 551 1020");
        assert_eq!(p.details.website, "https://cafeparis.example");
        assert_eq!(p.details.opening_hours, "Mo-Su 08:00-22:00");
        let bare = &geocoder().parse(r#"[{"display_name":"X","lat":"1","lon":"2"}]"#).unwrap()[0];
        assert!(bare.details.is_empty());
    }

    #[test]
    fn coordinates_are_recognised_in_the_common_notations() {
        assert_eq!(parse_coordinates("64.1466, -21.9426"), Some((64.1466, -21.9426)));
        assert_eq!(parse_coordinates("  64.1466 -21.9426 "), Some((64.1466, -21.9426)));
        assert_eq!(parse_coordinates("64.1466 N, 21.9426 W"), Some((64.1466, -21.9426)));
        assert_eq!(parse_coordinates("64.1466N 21.9426W"), Some((64.1466, -21.9426)));
        assert_eq!(parse_coordinates("21.9426 W 64.1466 N"), Some((64.1466, -21.9426)));
        assert_eq!(parse_coordinates("33.87 S 151.21 E"), Some((-33.87, 151.21)));
        assert_eq!(parse_coordinates("120, 10"), None);
        assert_eq!(parse_coordinates("12 N 14 N"), None);
        assert_eq!(parse_coordinates("Laugavegur 12"), None);
        assert_eq!(parse_coordinates("1.5"), None);
    }

    #[test]
    fn suggestions_wait_for_a_pause_and_for_the_rate_limit() {
        let mut s = Suggestor::default();
        s.typed("Lau", 1000);
        assert_eq!(s.due(1100), None, "still typing");
        assert_eq!(s.due(1000 + TYPING_PAUSE_MS), Some("Lau".into()));
        s.sent("Lau", 1700);
        s.typed("Laug", 1800);
        assert_eq!(s.due(2500), None, "less than a second since the last request");
        assert_eq!(s.due(2700), Some("Laug".into()));
        assert_eq!(s.next_due(1900), Some(2700));
        // the same text twice is not sent twice
        s.sent("Laug", 2700);
        s.typed("Laug", 5000);
        assert_eq!(s.due(9000), None);
        assert_eq!(s.next_due(9000), None);
    }

    #[test]
    fn short_text_and_coordinates_are_not_sent_for_suggestions() {
        let mut s = Suggestor::default();
        s.typed("ab", 0);
        assert_eq!(s.due(10_000), None);
        s.typed("64.1, -21.9", 0);
        assert_eq!(s.due(10_000), None);
        s.typed("   ", 0);
        assert_eq!(s.due(10_000), None);
        assert_eq!(s.wait_for_request(5), 0);
        s.note_request(100);
        assert_eq!(s.wait_for_request(300), 800);
    }

    #[test]
    fn percent_decoding_reverses_encoding() {
        assert_eq!(percent_decode(&percent_encode("Laugavegur 1, Reykjavík")), "Laugavegur 1, Reykjavík");
        assert_eq!(percent_decode("a+b%20c"), "a b c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
    }
}
