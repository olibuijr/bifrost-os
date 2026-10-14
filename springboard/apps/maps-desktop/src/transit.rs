//! Transit directions. A journey comes from an OpenTripPlanner-compatible planner (the GTFS journey planner API) when
//! one is configured as `transit_url` in settings.json. Without one the app offers no transit routes at all: unlike a
//! car route there is no honest straight-line stand-in for a bus timetable.
//!
//! The reply is read into the same `Route` the other profiles use. Each leg becomes one step whose text names the
//! line, the stops and the times, so the directions card and Print show a journey without a second layout.

use crate::route::{check_coord, Route, Step};
use bifrost_core::json::{self, Json};

/// The journey planner: the URL of an OpenTripPlanner router (for example
/// `https://otp.example.org/otp/routers/default`), or nothing configured.
#[derive(Clone, Debug, PartialEq)]
pub struct Planner {
    base: Option<String>,
}

/// Journeys asked for in one request.
const ITINERARIES: usize = 3;

impl Planner {
    /// `base` is the router URL. `None` or blank means transit is not configured.
    pub fn new(base: Option<&str>) -> Result<Self, String> {
        match base.map(|b| b.trim().trim_end_matches('/')).filter(|b| !b.is_empty()) {
            None => Ok(Self { base: None }),
            Some(b) if b.starts_with("https://") || b.starts_with("http://") => Ok(Self { base: Some(b.to_string()) }),
            Some(_) => Err("the transit URL must start with http:// or https://".into()),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.base.is_some()
    }

    /// The plan request from `from` to `to` as (lat, lon), leaving now. Errors when transit is not configured, so no
    /// request is made.
    pub fn plan_url(&self, from: (f64, f64), to: (f64, f64)) -> Result<String, String> {
        let base = self.base.as_deref().ok_or("transit is not configured")?;
        check_coord(from)?;
        check_coord(to)?;
        Ok(format!(
            "{base}/plan?fromPlace={:.6},{:.6}&toPlace={:.6},{:.6}&mode=TRANSIT,WALK&numItineraries={ITINERARIES}",
            from.0, from.1, to.0, to.1
        ))
    }
}

/// Reads a planner reply: every journey as a route, the planner's first choice first. A reply with an `error` is an
/// error that carries the planner's message.
pub fn parse_itineraries(body: &str) -> Result<Vec<Route>, String> {
    let doc = json::parse(body)?;
    if let Some(e) = doc.get("error") {
        let message = e.str_field("message").or_else(|| e.str_field("msg")).unwrap_or_else(|| "no journey".into());
        return Err(message);
    }
    let list = doc.get("plan").and_then(|p| p.get("itineraries")).and_then(Json::as_array).unwrap_or(&[]);
    let routes: Vec<Route> = list.iter().filter_map(journey_of).collect();
    if routes.is_empty() {
        return Err("the reply has no journey".into());
    }
    Ok(routes)
}

fn journey_of(item: &Json) -> Option<Route> {
    let legs = item.get("legs").and_then(Json::as_array)?;
    if legs.is_empty() {
        return None;
    }
    let mut steps = Vec::new();
    let mut geometry: Vec<(f64, f64)> = Vec::new();
    for leg in legs {
        let distance_m = leg.get("distance").and_then(Json::number).unwrap_or(0.0);
        steps.push(Step { name: leg_text(leg), maneuver: "transit".into(), distance_m });
        let line = leg.get("legGeometry").and_then(|g| g.str_field("points")).map(|p| decode_polyline(&p)).unwrap_or_default();
        if line.is_empty() {
            geometry.extend([place_of(leg.get("from")), place_of(leg.get("to"))].into_iter().flatten());
        } else {
            geometry.extend(line);
        }
    }
    let distance_m = steps.iter().map(|s| s.distance_m).sum();
    let duration_s = item.get("duration").and_then(Json::number).or_else(|| {
        let (start, end) = (item.get("startTime").and_then(Json::number)?, item.get("endTime").and_then(Json::number)?);
        Some(((end - start) / 1000.0).max(0.0))
    })?;
    Some(Route { distance_m, duration_s, steps, approximate: false, geometry })
}

fn place_of(p: Option<&Json>) -> Option<(f64, f64)> {
    let p = p?;
    Some((p.get("lat")?.number()?, p.get("lon")?.number()?))
}

/// The text of one leg: "Walk to Hlemmur" or "Bus 14 from Hlemmur 10:05 to Haholt 10:31".
fn leg_text(leg: &Json) -> String {
    let mode = leg.str_field("mode").unwrap_or_default();
    let name = |key: &str| leg.get(key).and_then(|p| p.str_field("name")).filter(|n| !n.is_empty());
    let offset = leg.get("agencyTimeZoneOffset").and_then(Json::number).unwrap_or(0.0);
    let at = |key: &str| leg.get(key).and_then(Json::number).map(|ms| clock(ms, offset));
    if mode == "WALK" || mode.is_empty() {
        return format!("Walk to {}", name("to").unwrap_or_else(|| "the next stop".into()));
    }
    let line = leg.str_field("routeShortName").filter(|s| !s.is_empty()).or_else(|| leg.str_field("route")).unwrap_or_default();
    let vehicle = mode_label(&mode);
    let head = if line.is_empty() { vehicle.to_string() } else { format!("{vehicle} {line}") };
    let stop = |place: Option<String>, time: Option<String>| match (place, time) {
        (Some(p), Some(t)) => format!("{p} {t}"),
        (Some(p), None) => p,
        (None, Some(t)) => t,
        (None, None) => "the stop".into(),
    };
    format!("{head} from {} to {}", stop(name("from"), at("startTime")), stop(name("to"), at("endTime")))
}

/// The vehicle word of a planner mode.
pub fn mode_label(mode: &str) -> &'static str {
    match mode {
        "BUS" => "Bus",
        "TRAM" => "Tram",
        "RAIL" => "Train",
        "SUBWAY" => "Metro",
        "FERRY" => "Ferry",
        "CABLE_CAR" => "Cable car",
        "GONDOLA" => "Gondola",
        "FUNICULAR" => "Funicular",
        _ => "Transit",
    }
}

/// "10:05" for a time in milliseconds since the epoch, shifted by the agency's offset from UTC (milliseconds).
pub fn clock(ms: f64, offset_ms: f64) -> String {
    let secs = ((ms + offset_ms) / 1000.0).floor() as i64;
    let day = secs.rem_euclid(86_400);
    format!("{:02}:{:02}", day / 3600, day % 3600 / 60)
}

/// Decodes an encoded polyline of five decimal places into (lat, lon) points. A damaged string gives the points read
/// before the damage.
pub fn decode_polyline(text: &str) -> Vec<(f64, f64)> {
    let bytes = text.as_bytes();
    let (mut i, mut lat, mut lon) = (0usize, 0i64, 0i64);
    let mut out = Vec::new();
    while i < bytes.len() {
        let mut parts = [0i64; 2];
        for part in parts.iter_mut() {
            let (mut shift, mut value) = (0u32, 0i64);
            loop {
                let Some(&b) = bytes.get(i) else { return out };
                i += 1;
                let c = i64::from(b) - 63;
                if !(0..64).contains(&c) || shift > 60 {
                    return out;
                }
                value |= (c & 0x1f) << shift;
                shift += 5;
                if c < 0x20 {
                    break;
                }
            }
            *part = if value & 1 != 0 { !(value >> 1) } else { value >> 1 };
        }
        lat += parts[0];
        lon += parts[1];
        out.push((lat as f64 / 1e5, lon as f64 / 1e5));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const REYKJAVIK: (f64, f64) = (64.1466, -21.9426);
    const KOPAVOGUR: (f64, f64) = (64.1090, -21.9100);

    const REPLY: &str = r#"{"plan":{"itineraries":[
        {"duration":1500,"startTime":1790000000000,"endTime":1790001500000,"legs":[
          {"mode":"WALK","distance":300.5,"startTime":1790000000000,"endTime":1790000300000,"agencyTimeZoneOffset":0,
           "from":{"name":"Origin","lat":64.1466,"lon":-21.9426},"to":{"name":"Hlemmur","lat":64.1427,"lon":-21.9148},
           "legGeometry":{"points":"_p~iF~ps|U_ulLnnqC"}},
          {"mode":"BUS","routeShortName":"14","distance":5200,"startTime":1790000400000,"endTime":1790001400000,"agencyTimeZoneOffset":0,
           "from":{"name":"Hlemmur","lat":64.1427,"lon":-21.9148},"to":{"name":"Hamraborg","lat":64.1090,"lon":-21.9100}}
        ]},
        {"duration":2100,"legs":[{"mode":"WALK","distance":2400,"from":{"name":"Origin","lat":64.1466,"lon":-21.9426},"to":{"name":"Destination","lat":64.1090,"lon":-21.9100}}]}
    ]}}"#;

    #[test]
    fn the_planner_url_carries_both_ends_and_the_modes() {
        let p = Planner::new(Some("https://otp.example.org/otp/routers/default/")).unwrap();
        assert!(p.is_configured());
        let url = p.plan_url(REYKJAVIK, KOPAVOGUR).unwrap();
        assert_eq!(
            url,
            "https://otp.example.org/otp/routers/default/plan?fromPlace=64.146600,-21.942600&toPlace=64.109000,-21.910000&mode=TRANSIT,WALK&numItineraries=3"
        );
    }

    #[test]
    fn without_a_planner_no_request_is_made() {
        let p = Planner::new(None).unwrap();
        assert!(!p.is_configured());
        assert!(p.plan_url(REYKJAVIK, KOPAVOGUR).is_err());
        assert!(!Planner::new(Some("  ")).unwrap().is_configured());
        assert!(Planner::new(Some("ftp://x")).is_err());
        let ok = Planner::new(Some("http://localhost:8080/otp")).unwrap();
        assert!(ok.plan_url((91.0, 0.0), KOPAVOGUR).is_err());
    }

    #[test]
    fn a_journey_lists_its_legs_with_the_line_stops_and_times() {
        let routes = parse_itineraries(REPLY).unwrap();
        assert_eq!(routes.len(), 2);
        let first = &routes[0];
        assert!(!first.approximate);
        assert_eq!(first.duration_s, 1500.0);
        assert_eq!(first.steps.len(), 2);
        assert_eq!(first.steps[0].instruction(), "Walk to Hlemmur");
        // 1790000400000 ms is 14:20 UTC
        assert_eq!(first.steps[1].instruction(), "Bus 14 from Hlemmur 14:20 to Hamraborg 14:36");
        assert!((first.distance_m - 5500.5).abs() < 1e-6);
    }

    #[test]
    fn the_line_of_a_journey_comes_from_the_leg_geometry_or_the_leg_ends() {
        let routes = parse_itineraries(REPLY).unwrap();
        // two decoded points for the walk, then the two ends of the bus leg
        assert_eq!(routes[0].geometry.len(), 4);
        assert!((routes[0].geometry[0].0 - 38.5).abs() < 1e-9);
        assert_eq!(routes[0].geometry[3], KOPAVOGUR);
        assert_eq!(routes[1].geometry, vec![REYKJAVIK, KOPAVOGUR]);
    }

    #[test]
    fn a_planner_error_and_an_empty_plan_are_errors() {
        let e = parse_itineraries(r#"{"error":{"id":404,"msg":"no path","message":"No trip found"}}"#).unwrap_err();
        assert_eq!(e, "No trip found");
        assert!(parse_itineraries(r#"{"plan":{"itineraries":[]}}"#).is_err());
        assert!(parse_itineraries("not json").is_err());
    }

    #[test]
    fn times_follow_the_agency_offset_and_wrap_the_day() {
        assert_eq!(clock(1_790_000_400_000.0, 0.0), "14:20");
        assert_eq!(clock(1_790_000_400_000.0, 3_600_000.0), "15:20");
        assert_eq!(clock(60_000.0, -120_000.0), "23:59");
    }

    #[test]
    fn a_polyline_decodes_and_a_damaged_one_stops_early() {
        let pts = decode_polyline("_p~iF~ps|U_ulLnnqC_mqNvxq`@");
        assert_eq!(pts.len(), 3);
        assert!((pts[1].0 - 40.7).abs() < 1e-9 && (pts[1].1 + 120.95).abs() < 1e-9);
        assert!((pts[2].0 - 43.252).abs() < 1e-9 && (pts[2].1 + 126.453).abs() < 1e-9);
        assert_eq!(decode_polyline("_p~iF~ps|U_ulL").len(), 1);
        assert!(decode_polyline("").is_empty());
    }

    #[test]
    fn vehicle_words() {
        assert_eq!(mode_label("RAIL"), "Train");
        assert_eq!(mode_label("SUBWAY"), "Metro");
        assert_eq!(mode_label("SPACESHIP"), "Transit");
    }
}
