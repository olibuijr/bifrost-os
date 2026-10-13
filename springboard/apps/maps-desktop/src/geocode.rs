//! Place search through a Nominatim-compatible geocoder: the request URL and the reply parser.
//! Nominatim's usage policy asks for an identifying user agent and at most one request per second; the
//! app sends searches only when the person submits the field.

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
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
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
            "{}/search?format=jsonv2&limit={}&q={}",
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
                Some(Place { name, lat, lon })
            })
            .collect())
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
            "https://nominatim.openstreetmap.org/search?format=jsonv2&limit=5&q=Laugavegur%201%2C%20Reykjav%C3%ADk"
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
}
