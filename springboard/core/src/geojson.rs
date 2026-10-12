//! GeoJSON decoding: reads a GeoJSON document (RFC 7946 subset) into features, feature collections and bare
//! geometries, with positions as longitude and latitude in degrees. A document is one GeoJSON object, or an array of
//! them; the result keeps their order. Parsing uses the crate's JSON reader and nothing else. The geometry types are
//! plain vectors and carry no map drawing: a caller that shows a map converts them to its own shapes.
//!
//! Rules enforced: a position has at least two finite numbers within -180..=180 and -90..=90 (extra altitude values
//! are ignored); a line string has at least two positions; a linear ring has at least four positions and is closed
//! (its first and last positions are equal). A feature's `geometry` key must be present, and may be `null`.

use crate::json::{self, Json};

/// A position in degrees. GeoJSON writes it as `[longitude, latitude]`, and so does this struct's field order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    pub longitude: f64,
    pub latitude: f64,
}

/// The shape of a feature or a bare geometry. Polygon rings are the exterior ring first, then the holes.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    Point(Position),
    MultiPoint(Vec<Position>),
    LineString(Vec<Position>),
    MultiLineString(Vec<Vec<Position>>),
    Polygon(Vec<Vec<Position>>),
    MultiPolygon(Vec<Vec<Vec<Position>>>),
}

/// A GeoJSON feature. `identifier` is the `id` member as text (a whole number is written without a fraction).
/// `properties` is the JSON object, or `None` when the member is absent or `null`.
#[derive(Clone, Debug, PartialEq)]
pub struct Feature {
    pub identifier: Option<String>,
    pub geometry: Option<Geometry>,
    pub properties: Option<Json>,
}

/// One decoded GeoJSON object.
#[derive(Clone, Debug, PartialEq)]
pub enum GeoJsonObject {
    Feature(Feature),
    FeatureCollection(Vec<Feature>),
    Geometry(Geometry),
}

/// Decodes UTF-8 GeoJSON bytes into objects, in document order. Errors name the first rule that failed.
pub fn decode(data: &[u8]) -> Result<Vec<GeoJsonObject>, String> {
    let text = std::str::from_utf8(data).map_err(|_| "GeoJSON data is not UTF-8".to_string())?;
    match json::parse(text)? {
        Json::Arr(items) => items.iter().map(object).collect(),
        root => Ok(vec![object(&root)?]),
    }
}

fn object(v: &Json) -> Result<GeoJsonObject, String> {
    match v.str_field("type").as_deref() {
        Some("Feature") => Ok(GeoJsonObject::Feature(feature(v)?)),
        Some("FeatureCollection") => {
            let features = items(v.get("features").ok_or("collection has no features")?)?;
            Ok(GeoJsonObject::FeatureCollection(features.iter().map(feature).collect::<Result<_, _>>()?))
        }
        Some(_) => match geometry(v)? {
            Some(g) => Ok(GeoJsonObject::Geometry(g)),
            None => Err("a bare geometry cannot be null".into()),
        },
        None => Err("GeoJSON object has no type".into()),
    }
}

fn feature(v: &Json) -> Result<Feature, String> {
    if v.str_field("type").as_deref() != Some("Feature") {
        return Err("expected a Feature".into());
    }
    let identifier = match v.get("id") {
        None | Some(Json::Null) => None,
        Some(Json::Str(s)) => Some(s.clone()),
        Some(Json::Num(n)) if n.is_finite() && n.fract() == 0.0 => Some(format!("{}", *n as i64)),
        Some(_) => return Err("feature id must be a string or a whole number".into()),
    };
    let geometry = geometry(v.get("geometry").ok_or("feature has no geometry key")?)?;
    let properties = match v.get("properties") {
        None | Some(Json::Null) => None,
        Some(o) if matches!(o, Json::Obj(_)) => Some(o.clone()),
        Some(_) => return Err("feature properties must be an object or null".into()),
    };
    Ok(Feature { identifier, geometry, properties })
}

const SUPPORTED_GEOMETRY: [&str; 6] =
    ["Point", "MultiPoint", "LineString", "MultiLineString", "Polygon", "MultiPolygon"];

fn geometry(v: &Json) -> Result<Option<Geometry>, String> {
    if *v == Json::Null {
        return Ok(None);
    }
    let kind = v.str_field("type").ok_or("geometry has no type")?;
    if !SUPPORTED_GEOMETRY.contains(&kind.as_str()) {
        return Err(format!("unsupported geometry type {kind}"));
    }
    let c = v.get("coordinates").ok_or("geometry has no coordinates")?;
    let g = match kind.as_str() {
        "Point" => Geometry::Point(position(c)?),
        "MultiPoint" => Geometry::MultiPoint(positions(c)?),
        "LineString" => Geometry::LineString(line(c)?),
        "MultiLineString" => Geometry::MultiLineString(items(c)?.iter().map(line).collect::<Result<_, _>>()?),
        "Polygon" => Geometry::Polygon(rings(c)?),
        _ => Geometry::MultiPolygon(items(c)?.iter().map(rings).collect::<Result<_, _>>()?),
    };
    Ok(Some(g))
}

fn items(v: &Json) -> Result<&[Json], String> {
    v.as_array().ok_or_else(|| "expected an array".to_string())
}

fn position(v: &Json) -> Result<Position, String> {
    let a = items(v)?;
    if a.len() < 2 {
        return Err("a position needs longitude and latitude".into());
    }
    let longitude = a[0].number().ok_or("longitude is not a number")?;
    let latitude = a[1].number().ok_or("latitude is not a number")?;
    if !(-180.0..=180.0).contains(&longitude) {
        return Err(format!("longitude {longitude} is outside -180..=180"));
    }
    if !(-90.0..=90.0).contains(&latitude) {
        return Err(format!("latitude {latitude} is outside -90..=90"));
    }
    Ok(Position { longitude, latitude })
}

fn positions(v: &Json) -> Result<Vec<Position>, String> {
    items(v)?.iter().map(position).collect()
}

fn line(v: &Json) -> Result<Vec<Position>, String> {
    let p = positions(v)?;
    if p.len() < 2 {
        return Err("a line string needs at least two positions".into());
    }
    Ok(p)
}

fn ring(v: &Json) -> Result<Vec<Position>, String> {
    let p = positions(v)?;
    if p.len() < 4 {
        return Err("a linear ring needs at least four positions".into());
    }
    if p.first() != p.last() {
        return Err("a linear ring must be closed".into());
    }
    Ok(p)
}

fn rings(v: &Json) -> Result<Vec<Vec<Position>>, String> {
    items(v)?.iter().map(ring).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> GeoJsonObject {
        decode(text.as_bytes()).unwrap().remove(0)
    }

    #[test]
    fn decodes_a_point_feature_with_string_id_and_properties() {
        let obj = one(r#"{"type":"Feature","id":"42","geometry":{"type":"Point","coordinates":[-21.5,64.25]},
            "properties":{"name":"Lab"}}"#);
        let GeoJsonObject::Feature(f) = obj else { panic!("expected a feature") };
        assert_eq!(f.identifier.as_deref(), Some("42"));
        assert_eq!(f.geometry, Some(Geometry::Point(Position { longitude: -21.5, latitude: 64.25 })));
        assert_eq!(f.properties.as_ref().unwrap().str_field("name").as_deref(), Some("Lab"));
    }

    #[test]
    fn writes_a_whole_numeric_id_without_a_fraction() {
        let obj = one(r#"{"type":"Feature","id":7,"geometry":null}"#);
        let GeoJsonObject::Feature(f) = obj else { panic!("expected a feature") };
        assert_eq!(f.identifier.as_deref(), Some("7"));
        assert_eq!(f.geometry, None);
        assert_eq!(f.properties, None);
    }

    #[test]
    fn keeps_features_of_a_collection_in_order_with_null_geometry() {
        let obj = one(r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","geometry":{"type":"LineString","coordinates":[[0,0],[1,1]]},"properties":null},
            {"type":"Feature","geometry":null,"properties":{}}]}"#);
        let GeoJsonObject::FeatureCollection(fs) = obj else { panic!("expected a collection") };
        assert_eq!(fs.len(), 2);
        assert!(matches!(fs[0].geometry, Some(Geometry::LineString(ref p)) if p.len() == 2));
        assert_eq!(fs[1].geometry, None);
    }

    #[test]
    fn keeps_both_rings_of_a_polygon_with_a_hole() {
        let obj = one(r#"{"type":"Polygon","coordinates":[
            [[0,0],[4,0],[4,4],[0,4],[0,0]],[[1,1],[2,1],[2,2],[1,1]]]}"#);
        let GeoJsonObject::Geometry(Geometry::Polygon(rings)) = obj else { panic!("expected a polygon") };
        assert_eq!(rings.len(), 2);
        assert_eq!(rings[0].len(), 5);
        assert_eq!(rings[1].len(), 4);
    }

    #[test]
    fn rejects_an_unclosed_ring() {
        let err = decode(br#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,1]]]}"#).unwrap_err();
        assert_eq!(err, "a linear ring must be closed");
    }

    #[test]
    fn rejects_a_ring_with_three_positions() {
        let err = decode(br#"{"type":"Polygon","coordinates":[[[0,0],[1,0],[0,0]]]}"#).unwrap_err();
        assert_eq!(err, "a linear ring needs at least four positions");
    }

    #[test]
    fn rejects_latitude_past_ninety_degrees() {
        let err = decode(br#"{"type":"Point","coordinates":[0,91]}"#).unwrap_err();
        assert_eq!(err, "latitude 91 is outside -90..=90");
    }

    #[test]
    fn decodes_an_array_of_objects_in_order_and_rejects_unknown_types() {
        let list = decode(br#"[{"type":"Point","coordinates":[1,2]},{"type":"Point","coordinates":[3,4]}]"#).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[1], GeoJsonObject::Geometry(Geometry::Point(Position { longitude: 3.0, latitude: 4.0 })));
        let err = decode(br#"{"type":"GeometryCollection","geometries":[]}"#).unwrap_err();
        assert_eq!(err, "unsupported geometry type GeometryCollection");
    }
}
