//! Links in and out: the links the Share menu makes for a place or a route, and the `maps:` and `geo:` links the app
//! opens. Parameters follow the common map-link vocabulary: `q` (a search or a labelled position), `ll` (the centre),
//! `z` (zoom), `saddr` and `daddr` (route start and end), `dirflg` (travel mode).

use crate::geocode::{parse_coordinates, percent_decode, percent_encode};
use crate::route::Profile;

/// A route end: a position, or text to look up first.
#[derive(Clone, Debug, PartialEq)]
pub enum Endpoint {
    Coordinate(f64, f64),
    Text(String),
}

/// What an opened link asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkAction {
    /// search for text
    Search(String),
    /// centre on a position, optionally at a zoom, optionally with a label for a pin
    Show { lat: f64, lon: f64, zoom: Option<f64>, label: Option<String> },
    /// directions; a missing `from` means the current centre or location
    Route { from: Option<Endpoint>, to: Endpoint, profile: Profile },
}

fn coordinate_text(lat: f64, lon: f64) -> String {
    format!("{lat:.5},{lon:.5}")
}

/// A link to a place, for Copy Link and the share sheet.
pub fn place_link(lat: f64, lon: f64, label: &str) -> String {
    let label = label.trim();
    if label.is_empty() {
        format!("maps:?ll={}", coordinate_text(lat, lon))
    } else {
        format!("maps:?ll={}&q={}", coordinate_text(lat, lon), percent_encode(label))
    }
}

/// A link to a route from one position to another.
pub fn route_link(from: (f64, f64), to: (f64, f64), profile: Profile) -> String {
    format!("maps:?saddr={}&daddr={}&dirflg={}", coordinate_text(from.0, from.1), coordinate_text(to.0, to.1), dirflg(profile))
}

/// A position as text for Copy Coordinates: "64.14660, -21.94260".
pub fn coordinates_text(lat: f64, lon: f64) -> String {
    format!("{lat:.5}, {lon:.5}")
}

fn dirflg(p: Profile) -> &'static str {
    match p {
        Profile::Driving => "d",
        Profile::Walking => "w",
        Profile::Cycling => "b",
    }
}

fn profile_of(flag: &str) -> Profile {
    match flag {
        "w" => Profile::Walking,
        "b" => Profile::Cycling,
        _ => Profile::Driving,
    }
}

fn endpoint(text: &str) -> Option<Endpoint> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    Some(parse_coordinates(t).map_or_else(|| Endpoint::Text(t.to_string()), |(lat, lon)| Endpoint::Coordinate(lat, lon)))
}

/// Reads a `maps:` or `geo:` link. `None` for any other scheme and for links that ask for nothing.
pub fn parse_link(link: &str) -> Option<LinkAction> {
    let link = link.trim();
    let (scheme, rest) = link.split_once(':')?;
    let rest = rest.trim_start_matches("//");
    let (head, query) = rest.split_once('?').unwrap_or((rest, ""));
    let params: Vec<(String, String)> = query
        .split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            Some((k.to_ascii_lowercase(), percent_decode(v)))
        })
        .collect();
    let param = |name: &str| params.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str()).filter(|v| !v.is_empty());
    let zoom = param("z").or_else(|| param("zoom")).and_then(|z| z.parse::<f64>().ok());

    match scheme.to_ascii_lowercase().as_str() {
        "geo" => {
            // geo:lat,lon[,alt][;u=..][?q=..&z=..]
            let pos = head.split(';').next().unwrap_or("");
            let mut nums = pos.split(',').filter_map(|n| n.trim().parse::<f64>().ok());
            let (lat, lon) = (nums.next()?, nums.next()?);
            if lat.abs() > 90.0 || lon.abs() > 180.0 {
                return None;
            }
            // geo:0,0?q=text means "search for text"
            if lat == 0.0 && lon == 0.0 {
                return param("q").map(|q| LinkAction::Search(q.to_string()));
            }
            // a label in the form q=lat,lon(Label)
            let label = param("q").and_then(|q| q.split_once('(')).map(|(_, l)| l.trim_end_matches(')').to_string());
            Some(LinkAction::Show { lat, lon, zoom, label })
        }
        "maps" | "map" => {
            if let Some(to) = param("daddr").and_then(endpoint) {
                let from = param("saddr").and_then(endpoint);
                return Some(LinkAction::Route { from, to, profile: profile_of(param("dirflg").unwrap_or("d")) });
            }
            let centre = param("ll").and_then(parse_coordinates);
            let query = param("q").or_else(|| param("address")).or_else(|| param("query"));
            match (centre, query) {
                (Some((lat, lon)), q) => Some(LinkAction::Show { lat, lon, zoom, label: q.map(str::to_string) }),
                (None, Some(q)) => match parse_coordinates(q) {
                    Some((lat, lon)) => Some(LinkAction::Show { lat, lon, zoom, label: None }),
                    None => Some(LinkAction::Search(q.to_string())),
                },
                (None, None) => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_link_reads_back_as_the_same_place() {
        let link = place_link(64.1466, -21.9426, "Café Paris, Reykjavík");
        assert_eq!(link, "maps:?ll=64.14660,-21.94260&q=Caf%C3%A9%20Paris%2C%20Reykjav%C3%ADk");
        assert_eq!(
            parse_link(&link),
            Some(LinkAction::Show { lat: 64.1466, lon: -21.9426, zoom: None, label: Some("Café Paris, Reykjavík".into()) })
        );
        assert_eq!(place_link(1.0, 2.0, "  "), "maps:?ll=1.00000,2.00000");
    }

    #[test]
    fn a_route_link_reads_back_as_the_same_route() {
        let link = route_link((64.1, -21.9), (65.7, -18.1), Profile::Walking);
        assert_eq!(link, "maps:?saddr=64.10000,-21.90000&daddr=65.70000,-18.10000&dirflg=w");
        assert_eq!(
            parse_link(&link),
            Some(LinkAction::Route { from: Some(Endpoint::Coordinate(64.1, -21.9)), to: Endpoint::Coordinate(65.7, -18.1), profile: Profile::Walking })
        );
    }

    #[test]
    fn text_ends_are_looked_up_and_a_missing_start_means_here() {
        assert_eq!(
            parse_link("maps://?daddr=Akureyri"),
            Some(LinkAction::Route { from: None, to: Endpoint::Text("Akureyri".into()), profile: Profile::Driving })
        );
        assert_eq!(parse_link("maps:?q=Laugavegur+12"), Some(LinkAction::Search("Laugavegur 12".into())));
        assert_eq!(parse_link("maps:?q=64.1,-21.9"), Some(LinkAction::Show { lat: 64.1, lon: -21.9, zoom: None, label: None }));
        assert_eq!(parse_link("maps:?ll=64.1,-21.9&z=14"), Some(LinkAction::Show { lat: 64.1, lon: -21.9, zoom: Some(14.0), label: None }));
    }

    #[test]
    fn geo_links_give_a_position_a_search_or_a_labelled_pin() {
        assert_eq!(parse_link("geo:64.1466,-21.9426"), Some(LinkAction::Show { lat: 64.1466, lon: -21.9426, zoom: None, label: None }));
        assert_eq!(parse_link("geo:64.1,-21.9;u=35?z=12"), Some(LinkAction::Show { lat: 64.1, lon: -21.9, zoom: Some(12.0), label: None }));
        assert_eq!(parse_link("geo:0,0?q=Hallgr%C3%ADmskirkja"), Some(LinkAction::Search("Hallgrímskirkja".into())));
        assert_eq!(
            parse_link("geo:0,0?q=64.14,-21.92(Church)"),
            Some(LinkAction::Search("64.14,-21.92(Church)".into()))
        );
        assert_eq!(
            parse_link("geo:64.14,-21.92?q=64.14,-21.92(Church)"),
            Some(LinkAction::Show { lat: 64.14, lon: -21.92, zoom: None, label: Some("Church".into()) })
        );
    }

    #[test]
    fn other_links_and_empty_requests_are_ignored() {
        assert_eq!(parse_link("https://example.org"), None);
        assert_eq!(parse_link("maps:"), None);
        assert_eq!(parse_link("geo:95,0"), None);
        assert_eq!(parse_link("geo:abc"), None);
        assert_eq!(parse_link("not a link"), None);
    }

    #[test]
    fn copied_coordinates_parse_back() {
        let text = coordinates_text(64.1466, -21.9426);
        assert_eq!(text, "64.14660, -21.94260");
        assert_eq!(parse_coordinates(&text), Some((64.1466, -21.9426)));
    }
}
