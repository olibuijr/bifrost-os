//! Control methods (`bifrost pc control call is.olibuijr.maps.desktop ...`): `maps.search` and `maps.route` call the same
//! `Geocoder` and `Router` the window uses, so a script sees what the person would. Both read the network; neither
//! changes the window.

use crate::geocode::{parse_coordinates, Geocoder, Place};
use crate::net::{Net, REPLY_LIMIT};
use crate::route::{self, Profile, Route, Router};
use bifrost_app::control::{RpcError, INVALID_PARAMS, INTERNAL_ERROR};
use bifrost_core::json::Json;

pub fn methods() -> Vec<&'static str> {
    vec!["maps.search", "maps.route"]
}

fn bad(message: &str) -> RpcError {
    RpcError::new(INVALID_PARAMS, message)
}

fn num(v: f64) -> Json {
    Json::Num(v)
}

fn place_json(p: &Place) -> Json {
    Json::Obj(vec![
        ("name".into(), Json::Str(p.name.clone())),
        ("lat".into(), num(p.lat)),
        ("lon".into(), num(p.lon)),
        ("kind".into(), Json::Str(p.details.kind.clone())),
        ("address".into(), Json::Arr(p.details.address.iter().map(|l| Json::Str(l.clone())).collect())),
        ("phone".into(), Json::Str(p.details.phone.clone())),
        ("website".into(), Json::Str(p.details.website.clone())),
        ("opening_hours".into(), Json::Str(p.details.opening_hours.clone())),
    ])
}

fn route_json(r: &Route) -> Json {
    Json::Obj(vec![
        ("distance_m".into(), num(r.distance_m)),
        ("duration_s".into(), num(r.duration_s)),
        ("approximate".into(), Json::Bool(r.approximate)),
        ("steps".into(), Json::Arr(r.steps.iter().map(|s| Json::Str(s.instruction())).collect())),
        ("points".into(), num(r.geometry.len() as f64)),
    ])
}

/// `maps.search {query}`: the places for a query (a typed coordinate answers itself).
pub fn search(net: &dyn Net, geocoder: &Geocoder, params: &Json) -> Result<Json, RpcError> {
    let query = params.str_field("query").filter(|q| !q.trim().is_empty()).ok_or_else(|| bad("query is required"))?;
    if let Some((lat, lon)) = parse_coordinates(&query) {
        return Ok(Json::Arr(vec![place_json(&Place::new(&crate::links::coordinates_text(lat, lon), lat, lon))]));
    }
    let url = geocoder.search_url(&query).map_err(|e| bad(&e))?;
    let body = net.get(&url, geocoder.user_agent(), REPLY_LIMIT).map_err(|e| RpcError::new(INTERNAL_ERROR, format!("search failed: {e}")))?;
    let places = geocoder.parse(&String::from_utf8_lossy(&body)).map_err(|e| RpcError::new(INTERNAL_ERROR, e))?;
    Ok(Json::Arr(places.iter().map(place_json).collect()))
}

fn point(params: &Json, key: &str) -> Result<(f64, f64), RpcError> {
    let text = params.str_field(key).ok_or_else(|| bad(&format!("{key} is required as \"lat,lon\"")))?;
    parse_coordinates(&text).ok_or_else(|| bad(&format!("{key} is not a coordinate")))
}

/// `maps.route {from, to, profile?}`: `from` and `to` are "lat,lon"; the profile is driving, walking or cycling. With no
/// routing server set up the answer is the labelled straight-line estimate.
pub fn route(net: &dyn Net, router: &Router, user_agent: &str, params: &Json) -> Result<Json, RpcError> {
    let (from, to) = (point(params, "from")?, point(params, "to")?);
    let profile = match params.str_field("profile").as_deref() {
        None | Some("driving") => Profile::Driving,
        Some("walking") => Profile::Walking,
        Some("cycling") => Profile::Cycling,
        Some(other) => return Err(bad(&format!("unknown profile {other}"))),
    };
    let routes = match router.route_url(from, to, profile) {
        Err(_) => vec![route::estimate(from, to, profile)],
        Ok(url) => {
            let body = net.get(&url, user_agent, REPLY_LIMIT).map_err(|e| RpcError::new(INTERNAL_ERROR, format!("routing failed: {e}")))?;
            route::parse_routes(&String::from_utf8_lossy(&body)).map_err(|e| RpcError::new(INTERNAL_ERROR, e))?
        }
    };
    Ok(Json::Arr(routes.iter().map(route_json).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::fake::Fake;
    use bifrost_core::json;

    fn params(text: &str) -> Json {
        json::parse(text).unwrap()
    }

    #[test]
    fn search_asks_the_geocoder_with_the_user_agent() {
        let net = Fake::with("https://nominatim.example.org/search", br#"[{"display_name":"Perlan, Reykjavik","lat":"64.12","lon":"-21.92","type":"museum"}]"#);
        let g = Geocoder::new("https://nominatim.example.org", "Test/1").unwrap();
        let out = search(&*net, &g, &params(r#"{"query":"perlan"}"#)).unwrap();
        let first = &out.as_array().unwrap()[0];
        assert_eq!(first.str_field("name").unwrap(), "Perlan, Reykjavik");
        assert_eq!(first.str_field("kind").unwrap(), "museum");
        assert_eq!(net.seen.lock().unwrap()[0].1, "Test/1");
    }

    #[test]
    fn search_needs_a_query_and_answers_coordinates_itself() {
        let net = Fake::with("none", b"");
        let g = Geocoder::new("https://nominatim.example.org", "Test/1").unwrap();
        assert_eq!(search(&*net, &g, &params("{}")).unwrap_err().code, INVALID_PARAMS);
        let out = search(&*net, &g, &params(r#"{"query":"64.1, -21.9"}"#)).unwrap();
        assert_eq!(out.as_array().unwrap().len(), 1);
        assert!(net.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn route_without_a_server_is_the_estimate() {
        let net = Fake::with("none", b"");
        let r = Router::new(None).unwrap();
        let out = route(&*net, &r, "Test/1", &params(r#"{"from":"64.1,-21.9","to":"64.2,-21.8","profile":"walking"}"#)).unwrap();
        let first = &out.as_array().unwrap()[0];
        assert!(matches!(first.get("approximate"), Some(Json::Bool(true))));
    }

    #[test]
    fn route_with_a_server_reads_every_alternative() {
        let body = br#"{"code":"Ok","routes":[{"distance":1000,"duration":120,"geometry":{"coordinates":[[-21.9,64.1],[-21.8,64.2]]},"legs":[]},{"distance":1200,"duration":140,"geometry":{"coordinates":[]},"legs":[]}]}"#;
        let net = Fake::with("https://router.example.org/route/v1/driving/", body);
        let r = Router::new(Some("https://router.example.org")).unwrap();
        let out = route(&*net, &r, "Test/1", &params(r#"{"from":"64.1,-21.9","to":"64.2,-21.8"}"#)).unwrap();
        assert_eq!(out.as_array().unwrap().len(), 2);
    }

    #[test]
    fn route_refuses_bad_parameters() {
        let net = Fake::with("none", b"");
        let r = Router::new(None).unwrap();
        assert_eq!(route(&*net, &r, "T", &params(r#"{"from":"x","to":"64.2,-21.8"}"#)).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(route(&*net, &r, "T", &params(r#"{"from":"64.1,-21.9","to":"64.2,-21.8","profile":"flying"}"#)).unwrap_err().code, INVALID_PARAMS);
        assert_eq!(route(&*net, &r, "T", &params(r#"{"to":"64.2,-21.8"}"#)).unwrap_err().code, INVALID_PARAMS);
    }
}
