//! Map links: a search, a centre and a zoom level for the map app, written as one URL and read back from one.
//!
//! The link has the kinds of values that the official map links take as launch options: a search query (`q`), a
//! centre (`ll`, written as "latitude,longitude"), a zoom level (`z`) and a map style (`t`). The scheme is BifrOSt's
//! own `maps://`, and no service stands behind it. Building and reading a link is pure text. Opening it, and naming
//! the app that receives it, are left to the caller. The parameter names, the zoom range and the style names are this
//! module's own choices. Coordinates are written with five decimal places, about a metre, so a round trip keeps that
//! precision and no more. Unknown parameters are ignored. When a parameter repeats, the first one counts.

use crate::geo::Coordinate;

/// The scheme every link starts with.
pub const SCHEME: &str = "maps://";
/// Lowest zoom level a link may carry. This module's own range.
pub const MIN_ZOOM: u8 = 1;
/// Highest zoom level a link may carry. This module's own range.
pub const MAX_ZOOM: u8 = 20;

/// How the map draws its ground. `Standard` is the default and is left out of the link.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapStyle {
    #[default]
    Standard,
    Satellite,
    Hybrid,
}

impl MapStyle {
    fn token(self) -> &'static str {
        match self {
            MapStyle::Standard => "standard",
            MapStyle::Satellite => "satellite",
            MapStyle::Hybrid => "hybrid",
        }
    }

    fn from_token(s: &str) -> Option<Self> {
        match s {
            "standard" => Some(MapStyle::Standard),
            "satellite" => Some(MapStyle::Satellite),
            "hybrid" => Some(MapStyle::Hybrid),
            _ => None,
        }
    }
}

/// The values one map link carries. Every field is optional, so the empty link opens the map with no search.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapLink {
    pub query: Option<String>,
    pub center: Option<Coordinate>,
    pub zoom: Option<u8>,
    pub style: MapStyle,
}

impl MapLink {
    /// The link as text. Parameters come in the order `q`, `ll`, `z`, `t`. Blank queries and zoom levels outside
    /// `MIN_ZOOM..=MAX_ZOOM` are left out, and `t` is left out for the standard style.
    pub fn to_url(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(q) = self.query.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            parts.push(format!("q={}", encode(q)));
        }
        if let Some(c) = self.center {
            let text = format!("{:.5},{:.5}", c.latitude, c.longitude);
            parts.push(format!("ll={}", encode(&text)));
        }
        if let Some(z) = self.zoom.filter(|z| (MIN_ZOOM..=MAX_ZOOM).contains(z)) {
            parts.push(format!("z={z}"));
        }
        if self.style != MapStyle::Standard {
            parts.push(format!("t={}", self.style.token()));
        }
        if parts.is_empty() {
            SCHEME.to_string()
        } else {
            format!("{SCHEME}?{}", parts.join("&"))
        }
    }

    /// Reads a link made by `to_url`. `None` when the scheme is wrong, a value cannot be decoded, the centre is not a
    /// valid coordinate, the zoom is outside the range, the style is unknown, or a pair has no `=`.
    pub fn parse(url: &str) -> Option<MapLink> {
        let rest = url.strip_prefix(SCHEME)?;
        let query = if rest.is_empty() { "" } else { rest.strip_prefix('?')? };
        let mut link = MapLink::default();
        let mut seen: Vec<&str> = Vec::new();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, raw) = pair.split_once('=')?;
            let value = decode(raw)?;
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            match key {
                "q" => link.query = Some(value).filter(|v| !v.trim().is_empty()),
                "ll" => link.center = Some(parse_center(&value)?),
                "z" => {
                    let z: u8 = value.parse().ok()?;
                    if !(MIN_ZOOM..=MAX_ZOOM).contains(&z) {
                        return None;
                    }
                    link.zoom = Some(z);
                }
                "t" => link.style = MapStyle::from_token(&value)?,
                _ => {}
            }
        }
        Some(link)
    }
}

fn parse_center(v: &str) -> Option<Coordinate> {
    let (lat, lon) = v.split_once(',')?;
    Coordinate::new(lat.trim().parse().ok()?, lon.trim().parse().ok()?)
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Reads percent escapes and `+` (as a space). `None` for a malformed escape or bytes that are not UTF-8.
fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                if !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
                    return None;
                }
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harpa() -> Coordinate {
        Coordinate::new(64.1466, -21.9426).unwrap()
    }

    #[test]
    fn writes_parameters_in_fixed_order_with_escapes() {
        let link = MapLink {
            query: Some("Harpa".into()),
            center: Some(harpa()),
            zoom: Some(15),
            style: MapStyle::Satellite,
        };
        assert_eq!(link.to_url(), "maps://?q=Harpa&ll=64.14660%2C-21.94260&z=15&t=satellite");
    }

    #[test]
    fn escapes_spaces_and_ampersands_in_the_query() {
        let link = MapLink { query: Some(" Cafe & Bar ".into()), ..Default::default() };
        assert_eq!(link.to_url(), "maps://?q=Cafe%20%26%20Bar");
    }

    #[test]
    fn round_trip_restores_the_same_link() {
        let link = MapLink {
            query: Some("Harpa".into()),
            center: Some(harpa()),
            zoom: Some(15),
            style: MapStyle::Satellite,
        };
        assert_eq!(MapLink::parse(&link.to_url()), Some(link));
    }

    #[test]
    fn plus_sign_reads_as_a_space() {
        let link = MapLink::parse("maps://?q=Cafe+Bar").unwrap();
        assert_eq!(link.query.as_deref(), Some("Cafe Bar"));
        assert_eq!(link.style, MapStyle::Standard);
    }

    #[test]
    fn rejects_bad_zoom_centre_scheme_and_escapes() {
        assert_eq!(MapLink::parse("maps://?z=21"), None);
        assert_eq!(MapLink::parse("maps://?z=0"), None);
        assert_eq!(MapLink::parse("maps://?ll=91,0"), None);
        assert_eq!(MapLink::parse("https://maps.example/?q=x"), None);
        assert_eq!(MapLink::parse("maps://?q=%zz"), None);
        assert_eq!(MapLink::parse("maps://?t=sepia"), None);
    }

    #[test]
    fn first_repeated_parameter_wins_and_unknown_ones_are_ignored() {
        let link = MapLink::parse("maps://?foo=1&z=1&z=5&q=x").unwrap();
        assert_eq!(link.zoom, Some(1));
        assert_eq!(link.query.as_deref(), Some("x"));
    }

    #[test]
    fn empty_link_is_the_bare_scheme() {
        assert_eq!(MapLink::default().to_url(), "maps://");
        assert_eq!(MapLink::parse("maps://"), Some(MapLink::default()));
    }
}
