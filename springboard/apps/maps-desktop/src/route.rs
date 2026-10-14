//! Directions. A route comes from an OSRM-compatible routing server when one is configured. Without one the app
//! falls back to a straight-line estimate, which it labels as an estimate, so nothing is shown as a road route that
//! was not computed by a server. Offline place search runs over a local list of places, for when the geocoder is
//! unreachable or not configured.

use crate::geocode::Place;
use bifrost_core::json::{self, Json};

/// Mean Earth radius in metres, for the great-circle distance of the estimate.
const EARTH_RADIUS_M: f64 = 6_371_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Driving,
    Walking,
    Cycling,
    /// public transport; the journey comes from the transit planner (`transit`), not the OSRM server
    Transit,
}

impl Profile {
    /// The profile name in an OSRM-compatible URL.
    pub fn osrm_name(self) -> &'static str {
        match self {
            Profile::Driving => "driving",
            Profile::Walking => "foot",
            Profile::Cycling => "bike",
            Profile::Transit => "transit",
        }
    }

    /// A rough average speed in metres per second, used only for the straight-line estimate.
    pub fn estimate_speed_mps(self) -> f64 {
        match self {
            Profile::Driving => 13.9,
            Profile::Walking => 1.4,
            Profile::Cycling => 4.2,
            Profile::Transit => 8.3,
        }
    }
}

/// One manoeuvre of a route: what to do and how far to go on the step.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub name: String,
    pub maneuver: String,
    pub distance_m: f64,
}

impl Step {
    /// The step as an instruction: "Turn left onto Laugavegur", "Arrive at your destination".
    pub fn instruction(&self) -> String {
        let (kind, modifier) = self.maneuver.split_once(' ').unwrap_or((self.maneuver.as_str(), ""));
        let onto = if self.name.is_empty() { String::new() } else { format!(" onto {}", self.name) };
        let way = if self.name.is_empty() { String::new() } else { format!(" on {}", self.name) };
        match kind {
            "depart" => format!("Head out{way}"),
            "arrive" => "Arrive at your destination".to_string(),
            "turn" | "end of road" | "fork" | "on ramp" | "off ramp" if !modifier.is_empty() => format!("Turn {modifier}{onto}"),
            "roundabout" | "rotary" => format!("Take the roundabout{onto}"),
            "merge" => format!("Merge{onto}"),
            "transit" => self.name.clone(),
            "continue" | "new name" => format!("Continue{way}"),
            "" => format!("Continue{way}"),
            other if modifier.is_empty() => format!("{}{onto}", capitalise(other)),
            other => format!("{} {modifier}{onto}", capitalise(other)),
        }
    }
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
}

/// A computed route, or an estimate when `approximate` is set.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub distance_m: f64,
    pub duration_s: f64,
    pub steps: Vec<Step>,
    pub approximate: bool,
    /// the line of the route as (lat, lon) points, for drawing; empty when the server sent none
    pub geometry: Vec<(f64, f64)>,
}

/// The routing service: an OSRM-compatible base URL, or nothing configured.
#[derive(Clone, Debug, PartialEq)]
pub struct Router {
    base: Option<String>,
}

impl Router {
    /// `base` is the server root (for example `https://router.example.org`). `None` means routing is not configured.
    pub fn new(base: Option<&str>) -> Result<Self, String> {
        match base.map(|b| b.trim().trim_end_matches('/')).filter(|b| !b.is_empty()) {
            None => Ok(Self { base: None }),
            Some(b) if b.starts_with("https://") || b.starts_with("http://") => Ok(Self { base: Some(b.to_string()) }),
            Some(_) => Err("the routing URL must start with http:// or https://".into()),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.base.is_some()
    }

    /// The route request URL from `from` to `to` as (lat, lon). Coordinates go in lon,lat order, as OSRM expects.
    /// Errors when routing is not configured, so no request is made.
    pub fn route_url(&self, from: (f64, f64), to: (f64, f64), profile: Profile) -> Result<String, String> {
        if profile == Profile::Transit {
            return Err("transit journeys come from the transit planner".into());
        }
        let base = self.base.as_deref().ok_or("routing is not configured")?;
        check_coord(from)?;
        check_coord(to)?;
        Ok(format!(
            "{base}/route/v1/{}/{:.6},{:.6};{:.6},{:.6}?overview=full&geometries=geojson&steps=true&alternatives=true",
            profile.osrm_name(),
            from.1,
            from.0,
            to.1,
            to.0
        ))
    }
}

pub(crate) fn check_coord((lat, lon): (f64, f64)) -> Result<(), String> {
    if lat.is_finite() && lon.is_finite() && lat.abs() <= 90.0 && lon.abs() <= 180.0 {
        Ok(())
    } else {
        Err("a route needs coordinates within the world".into())
    }
}

/// Reads an OSRM-compatible reply: the first route with its steps. A reply whose `code` is not `Ok` is an error that
/// carries the server's message.
pub fn parse_route(body: &str) -> Result<Route, String> {
    parse_routes(body).map(|mut all| all.remove(0))
}

/// Every route of an OSRM-compatible reply (the best first, then the alternatives).
pub fn parse_routes(body: &str) -> Result<Vec<Route>, String> {
    let doc = json::parse(body)?;
    let code = doc.str_field("code").unwrap_or_default();
    if code != "Ok" {
        let message = doc.str_field("message").unwrap_or_else(|| "no route".into());
        return Err(format!("{code}: {message}"));
    }
    let list = doc.get("routes").and_then(Json::as_array).unwrap_or(&[]);
    let routes: Vec<Route> = list.iter().filter_map(route_of).collect();
    if routes.is_empty() {
        return Err("the reply has no route".into());
    }
    Ok(routes)
}

fn route_of(item: &Json) -> Option<Route> {
    let distance_m = item.get("distance").and_then(Json::number)?;
    let duration_s = item.get("duration").and_then(Json::number)?;
    let mut steps = Vec::new();
    for leg in item.get("legs").and_then(Json::as_array).unwrap_or(&[]) {
        for step in leg.get("steps").and_then(Json::as_array).unwrap_or(&[]) {
            let maneuver = step.get("maneuver").and_then(|m| m.str_field("type")).unwrap_or_default();
            let modifier = step.get("maneuver").and_then(|m| m.str_field("modifier")).unwrap_or_default();
            steps.push(Step {
                name: step.str_field("name").unwrap_or_default(),
                maneuver: if modifier.is_empty() { maneuver } else { format!("{maneuver} {modifier}") },
                distance_m: step.get("distance").and_then(Json::number).unwrap_or(0.0),
            });
        }
    }
    // GeoJSON positions are [lon, lat]
    let geometry = item
        .get("geometry")
        .and_then(|g| g.get("coordinates"))
        .and_then(Json::as_array)
        .map(|points| {
            points
                .iter()
                .filter_map(|p| {
                    let p = p.as_array()?;
                    Some((p.get(1)?.number()?, p.first()?.number()?))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(Route { distance_m, duration_s, steps, approximate: false, geometry })
}

/// Great-circle distance in metres between two (lat, lon) points.
pub fn haversine_m(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lat1, lat2) = (a.0.to_radians(), b.0.to_radians());
    let dlat = (b.0 - a.0).to_radians();
    let dlon = (b.1 - a.1).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

/// The straight-line estimate between two points for a profile. It has no steps and is marked approximate.
pub fn estimate(from: (f64, f64), to: (f64, f64), profile: Profile) -> Route {
    let distance_m = haversine_m(from, to);
    Route { distance_m, duration_s: distance_m / profile.estimate_speed_mps(), steps: Vec::new(), approximate: true, geometry: vec![from, to] }
}

/// Places in `places` whose name contains every word of `query` (case-insensitive), in list order, at most `limit`.
/// A blank query finds nothing. This is the offline search; it needs no network.
pub fn search_offline<'a>(places: &'a [Place], query: &str, limit: usize) -> Vec<&'a Place> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    places
        .iter()
        .filter(|p| {
            let name = p.name.to_lowercase();
            words.iter().all(|w| name.contains(w.as_str()))
        })
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const REYKJAVIK: (f64, f64) = (64.1466, -21.9426);
    const AKUREYRI: (f64, f64) = (65.6835, -18.1262);

    #[test]
    fn no_routing_server_means_no_route_request() {
        let r = Router::new(None).unwrap();
        assert!(!r.is_configured());
        assert!(r.route_url(REYKJAVIK, AKUREYRI, Profile::Driving).is_err());
        assert!(Router::new(Some("  ")).unwrap().route_url(REYKJAVIK, AKUREYRI, Profile::Driving).is_err());
    }

    #[test]
    fn a_bad_routing_url_is_refused() {
        assert!(Router::new(Some("ftp://router")).is_err());
    }

    #[test]
    fn the_route_url_puts_longitude_first_and_names_the_profile() {
        let r = Router::new(Some("https://router.example.org/")).unwrap();
        let url = r.route_url(REYKJAVIK, AKUREYRI, Profile::Walking).unwrap();
        assert_eq!(
            url,
            "https://router.example.org/route/v1/foot/-21.942600,64.146600;-18.126200,65.683500?overview=full&geometries=geojson&steps=true&alternatives=true"
        );
        assert!(r.route_url((91.0, 0.0), AKUREYRI, Profile::Driving).is_err());
    }

    #[test]
    fn an_osrm_reply_gives_the_route_and_its_steps() {
        let body = r#"{"code":"Ok","routes":[{"distance":432.5,"duration":61.2,"legs":[{"steps":[
            {"distance":300.0,"duration":40.0,"name":"Laugavegur","maneuver":{"type":"depart","modifier":"left"}},
            {"distance":132.5,"duration":21.2,"name":"","maneuver":{"type":"arrive"}}]}]}]}"#;
        let route = parse_route(body).unwrap();
        assert!(!route.approximate);
        assert_eq!(route.distance_m, 432.5);
        assert_eq!(route.duration_s, 61.2);
        assert_eq!(route.steps.len(), 2);
        assert_eq!(route.steps[0].maneuver, "depart left");
        assert_eq!(route.steps[0].name, "Laugavegur");
        assert_eq!(route.steps[1].maneuver, "arrive");
    }

    #[test]
    fn a_server_error_is_reported_with_its_message() {
        let err = parse_route(r#"{"code":"NoRoute","message":"Impossible route between points"}"#).unwrap_err();
        assert_eq!(err, "NoRoute: Impossible route between points");
        assert!(parse_route("not json").is_err());
        assert!(parse_route(r#"{"code":"Ok","routes":[]}"#).is_err());
    }

    #[test]
    fn the_straight_line_distance_is_about_right() {
        // Reykjavik to Akureyri is about 248 km as the crow flies (about 171 km north-south and 177 km east-west at 65 N);
        // the earlier 228 km figure was a wrong expectation, the great-circle formula is right.
        let d = haversine_m(REYKJAVIK, AKUREYRI);
        assert!((d - 248_000.0).abs() < 4_000.0, "{d}");
        assert_eq!(haversine_m(REYKJAVIK, REYKJAVIK), 0.0);
    }

    #[test]
    fn an_estimate_is_marked_approximate_and_has_no_steps() {
        let e = estimate(REYKJAVIK, AKUREYRI, Profile::Driving);
        assert!(e.approximate);
        assert!(e.steps.is_empty());
        assert!((e.duration_s - e.distance_m / 13.9).abs() < 1e-6);
        assert!(estimate(REYKJAVIK, AKUREYRI, Profile::Walking).duration_s > e.duration_s);
    }

    #[test]
    fn offline_search_matches_every_word_in_list_order_and_respects_the_limit() {
        let places = vec![
            Place::new("Laugavegur, Reykjavík", 64.14, -21.93),
            Place::new("Hverfisgata, Reykjavík", 64.15, -21.94),
            Place::new("Laugavegur, Akureyri", 65.68, -18.12),
        ];
        let found = search_offline(&places, "LAUGAVEGUR reykjavík", 5);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].lat, 64.14);
        assert_eq!(search_offline(&places, "laugavegur", 1).len(), 1);
        assert_eq!(search_offline(&places, "laugavegur", 5).len(), 2);
        assert!(search_offline(&places, "   ", 5).is_empty());
    }

    #[test]
    fn alternatives_and_the_line_are_parsed() {
        let body = r#"{"code":"Ok","routes":[
            {"distance":1000.0,"duration":100.0,"geometry":{"type":"LineString","coordinates":[[-21.9,64.1],[-21.8,64.2]]},"legs":[]},
            {"distance":1200.0,"duration":90.0,"geometry":{"coordinates":[[-21.9,64.1],[-21.7,64.3]]},"legs":[]},
            {"duration":5.0}]}"#;
        let all = parse_routes(body).unwrap();
        assert_eq!(all.len(), 2, "the entry without a distance is skipped");
        assert_eq!(all[0].geometry, vec![(64.1, -21.9), (64.2, -21.8)]);
        assert_eq!(all[1].duration_s, 90.0);
        assert_eq!(parse_route(body).unwrap().distance_m, 1000.0);
        assert!(parse_routes(r#"{"code":"Ok","routes":[]}"#).is_err());
    }

    #[test]
    fn the_request_asks_for_alternatives_and_the_line() {
        let url = Router::new(Some("https://r.example")).unwrap().route_url(REYKJAVIK, AKUREYRI, Profile::Driving).unwrap();
        assert!(url.contains("alternatives=true") && url.contains("overview=full") && url.contains("geometries=geojson"));
    }

    #[test]
    fn steps_read_as_instructions() {
        let step = |m: &str, n: &str| Step { name: n.into(), maneuver: m.into(), distance_m: 10.0 };
        assert_eq!(step("depart left", "Laugavegur").instruction(), "Head out on Laugavegur");
        assert_eq!(step("turn right", "Snorrabraut").instruction(), "Turn right onto Snorrabraut");
        assert_eq!(step("turn left", "").instruction(), "Turn left");
        assert_eq!(step("arrive", "").instruction(), "Arrive at your destination");
        assert_eq!(step("roundabout", "Hringbraut").instruction(), "Take the roundabout onto Hringbraut");
        assert_eq!(step("continue straight", "Route 1").instruction(), "Continue on Route 1");
    }
}
