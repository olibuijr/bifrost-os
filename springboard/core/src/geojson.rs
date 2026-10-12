//! GeoJSON reader: turns GeoJSON text into a flat list of features that a map can draw.
//!
//! This is the decoding half of the official GeoJSON decoder, in outline. It reads Feature, FeatureCollection and
//! GeometryCollection objects and the six geometry types. A position is longitude then latitude, with an optional
//! altitude. Collections are flattened: each member becomes one [`Feature`]. A top-level geometry becomes a feature
//! with no identifier and no properties. Drawing, indoor map data and the map's own layers are not part of this
//! module. The checks (coordinate ranges, minimum vertex counts, closed rings) follow RFC 7946 and reject the input
//! with an error rather than repairing it.

use crate::json::{self, Json};

/// A position in degrees, with an optional altitude in metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub longitude: f64,
    pub latitude: f64,
    pub altitude: Option<f64>,
}

/// The shape of a feature. The rings of a polygon: the first is the outer ring, the rest are holes.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    Point(Position),
    MultiPoint(Vec<Position>),
    LineString(Vec<Position>),
    MultiLineString(Vec<Vec<Position>>),
    Polygon(Vec<Vec<Position>>),
    MultiPolygon(Vec<Vec<Vec<Position>>>),
}

/// One decoded feature. `geometry` is `None` when the GeoJSON geometry is null.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    /// The feature's `id` as text. A numeric id is written without a fraction when it is whole.
    pub identifier: Option<String>,
    pub geometry: Option<Geometry>,
    /// The members of the properties object, in the order they appear. Empty when there are none.
    pub properties: Vec<(String, Json)>,
}

/// Decodes GeoJSON text into features. The error names the first problem found.
pub fn decode(text: &str) -> Result<Vec<Feature>, String> {
    let root = json::parse(text)?;
    let mut out = Vec::new();
    match kind(&root)? {
        "Feature" => out.push(feature(&root)?),
        "FeatureCollection" => {
            let members = root
                .get("features")
                .and_then(Json::as_array)
                .ok_or("a FeatureCollection needs a features array")?;
            for member in members {
                out.push(feature(member)?);
            }
        }
        "GeometryCollection" => {
            let members = root
                .get("geometries")
                .and_then(Json::as_array)
                .ok_or("a GeometryCollection needs a geometries array")?;
            for member in members {
                out.push(bare(geometry(member)?));
            }
        }
        _ => out.push(bare(geometry(&root)?)),
    }
    Ok(out)
}

fn bare(geometry: Geometry) -> Feature {
    Feature { identifier: None, geometry: Some(geometry), properties: Vec::new() }
}

fn kind(v: &Json) -> Result<&str, String> {
    v.get("type")
        .and_then(Json::as_str)
        .ok_or_else(|| "a GeoJSON object needs a type".to_string())
}

fn feature(v: &Json) -> Result<Feature, String> {
    if kind(v)? != "Feature" {
        return Err("a FeatureCollection member must be a Feature".into());
    }
    let identifier = match v.get("id") {
        None | Some(Json::Null) => None,
        Some(Json::Str(s)) => Some(s.clone()),
        Some(Json::Num(n)) => Some(number_id(*n)),
        Some(_) => return Err("a feature id must be a string or a number".into()),
    };
    let geometry = match v.get("geometry") {
        None => return Err("a Feature needs a geometry member".into()),
        Some(Json::Null) => None,
        Some(g) => Some(geometry(g)?),
    };
    let properties = match v.get("properties") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::Obj(members)) => members.clone(),
        Some(_) => return Err("feature properties must be an object or null".into()),
    };
    Ok(Feature { identifier, geometry, properties })
}

fn number_id(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 9.0e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn geometry(v: &Json) -> Result<Geometry, String> {
    let t = kind(v)?;
    let c = v
        .get("coordinates")
        .ok_or_else(|| format!("a {t} needs coordinates"))?;
    match t {
        "Point" => Ok(Geometry::Point(position(c)?)),
        "MultiPoint" => Ok(Geometry::MultiPoint(positions(c, 0)?)),
        "LineString" => Ok(Geometry::LineString(positions(c, 2)?)),
        "MultiLineString" => {
            let lines = array(c)?
                .iter()
                .map(|l| positions(l, 2))
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Geometry::MultiLineString(lines))
        }
        "Polygon" => Ok(Geometry::Polygon(rings(c)?)),
        "MultiPolygon" => {
            let polygons = array(c)?
                .iter()
                .map(rings)
                .collect::<Result<Vec<_>, String>>()?;
            Ok(Geometry::MultiPolygon(polygons))
        }
        other => Err(format!("unsupported geometry type {other}")),
    }
}

fn array(v: &Json) -> Result<&[Json], String> {
    v.as_array().ok_or_else(|| "coordinates must be an array".to_string())
}

fn position(v: &Json) -> Result<Position, String> {
    let a = array(v)?;
    if a.len() < 2 || a.len() > 3 {
        return Err("a position needs two or three numbers".into());
    }
    let num = |x: &Json| {
        x.number()
            .filter(|n| n.is_finite())
            .ok_or_else(|| "position values must be finite numbers".to_string())
    };
    let longitude = num(&a[0])?;
    let latitude = num(&a[1])?;
    if !(-180.0..=180.0).contains(&longitude) {
        return Err(format!("longitude {longitude} is outside -180 to 180"));
    }
    if !(-90.0..=90.0).contains(&latitude) {
        return Err(format!("latitude {latitude} is outside -90 to 90"));
    }
    let altitude = a.get(2).map(|x| num(x)).transpose()?;
    Ok(Position { longitude, latitude, altitude })
}

fn positions(v: &Json, min: usize) -> Result<Vec<Position>, String> {
    let list = array(v)?
        .iter()
        .map(position)
        .collect::<Result<Vec<_>, String>>()?;
    if list.len() < min {
        return Err(format!("expected at least {min} positions, found {}", list.len()));
    }
    Ok(list)
}

fn ring(v: &Json) -> Result<Vec<Position>, String> {
    let p = positions(v, 4)?;
    if p.first() != p.last() {
        return Err("a linear ring must end on its first position".into());
    }
    Ok(p)
}

fn rings(v: &Json) -> Result<Vec<Vec<Position>>, String> {
    let list = array(v)?
        .iter()
        .map(ring)
        .collect::<Result<Vec<_>, String>>()?;
    if list.is_empty() {
        return Err("a polygon needs at least one ring".into());
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(longitude: f64, latitude: f64) -> Position {
        Position { longitude, latitude, altitude: None }
    }

    #[test]
    fn feature_keeps_numeric_id_geometry_and_properties() {
        let text = r#"{"type":"Feature","id":7,"geometry":{"type":"Point","coordinates":[-21.9,64.1]},"properties":{"name":"Harpa"}}"#;
        let features = decode(text).unwrap();
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].identifier.as_deref(), Some("7"));
        assert_eq!(features[0].geometry, Some(Geometry::Point(point(-21.9, 64.1))));
        assert_eq!(features[0].properties, vec![("name".to_string(), Json::Str("Harpa".into()))]);
    }

    #[test]
    fn feature_collection_flattens_members_and_accepts_null_geometry() {
        let text = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"Point","coordinates":[0,0]},"properties":null},
            {"type":"Feature","id":"a-1","geometry":null,"properties":{}}]}"#;
        let features = decode(text).unwrap();
        assert_eq!(features.len(), 2);
        assert_eq!(features[0].identifier, None);
        assert_eq!(features[1].identifier.as_deref(), Some("a-1"));
        assert_eq!(features[1].geometry, None);
        assert!(features[1].properties.is_empty());
    }

    #[test]
    fn geometry_collection_and_altitude() {
        let text = r#"{"type":"GeometryCollection","geometries":[
            {"type":"Point","coordinates":[1,2,30]},
            {"type":"LineString","coordinates":[[0,0],[1,1]]}]}"#;
        let features = decode(text).unwrap();
        assert_eq!(features.len(), 2);
        assert_eq!(features[0].identifier, None);
        assert_eq!(
            features[0].geometry,
            Some(Geometry::Point(Position { longitude: 1.0, latitude: 2.0, altitude: Some(30.0) }))
        );
        match &features[1].geometry {
            Some(Geometry::LineString(v)) => assert_eq!(v.len(), 2),
            other => panic!("expected a line string, got {other:?}"),
        }
    }

    #[test]
    fn polygon_ring_must_close_and_have_four_positions() {
        let open = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1]]]}"#;
        assert!(decode(open).is_err());
        let short = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[0,0]]]}"#;
        assert!(decode(short).is_err());
        let closed = r#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}"#;
        match decode(closed).unwrap()[0].geometry.clone() {
            Some(Geometry::Polygon(rings)) => {
                assert_eq!(rings.len(), 1);
                assert_eq!(rings[0].len(), 4);
            }
            other => panic!("expected a polygon, got {other:?}"),
        }
    }

    #[test]
    fn multi_polygon_keeps_each_polygon() {
        let ring = "[[[0,0],[1,0],[1,1],[0,0]]]";
        let text = format!(r#"{{"type":"MultiPolygon","coordinates":[{ring},{ring}]}}"#);
        match decode(&text).unwrap()[0].geometry.clone() {
            Some(Geometry::MultiPolygon(polygons)) => {
                assert_eq!(polygons.len(), 2);
                assert_eq!(polygons[0].len(), 1);
            }
            other => panic!("expected a multi polygon, got {other:?}"),
        }
    }

    #[test]
    fn coordinate_ranges_are_checked_at_the_edges() {
        let ok = r#"{"type":"Point","coordinates":[180,-90]}"#;
        assert_eq!(decode(ok).unwrap()[0].geometry, Some(Geometry::Point(point(180.0, -90.0))));
        let bad_lat = r#"{"type":"Point","coordinates":[0,91]}"#;
        assert!(decode(bad_lat).is_err());
        let bad_lon = r#"{"type":"Point","coordinates":[-180.5,0]}"#;
        assert!(decode(bad_lon).is_err());
    }

    #[test]
    fn unknown_types_and_broken_text_are_errors() {
        assert!(decode(r#"{"type":"Circle","coordinates":[0,0]}"#).is_err());
        assert!(decode(r#"{"coordinates":[0,0]}"#).is_err());
        assert!(decode("{").is_err());
        assert!(decode(r#"{"type":"Feature","properties":null}"#).is_err());
    }
}
