//! GeoJSON as map items: turns decoded GeoJSON (`bifrost_core::geojson`) into the map's own shapes. Points become
//! markers, line strings become polylines, and polygons become polygons with holes. A multi-part geometry is split
//! into one item per part. Feature identifiers and the `name` property are kept on each item.
//!
//! Nothing is drawn here, and nothing is fetched. A ring's closing repeat of its first position is dropped, because
//! [`MapPolygon`] implies the closing edge. The Indoor Mapping Data Format (IMDF) is not read by this module; its
//! feature collections are plain GeoJSON, so they decode here, but IMDF's own rules (venues, levels, units) are not
//! applied.

use crate::geo::Coordinate;
use crate::map_overlay::{MapPolygon, MapPolyline};
use bifrost_core::geojson::{self, Feature, GeoJsonObject, Geometry, Position};

/// One map item made from a GeoJSON geometry.
#[derive(Clone, Debug, PartialEq)]
pub enum MapFeature {
    /// A point, shown as a marker.
    Marker {
        coordinate: Coordinate,
        identifier: Option<String>,
        name: Option<String>,
    },
    /// An open line.
    Line {
        line: MapPolyline,
        identifier: Option<String>,
        name: Option<String>,
    },
    /// A closed polygon, with holes when the source has interior rings.
    Area {
        polygon: MapPolygon,
        identifier: Option<String>,
        name: Option<String>,
    },
}

/// Decodes GeoJSON bytes and converts every geometry into map items, in document order.
pub fn decode(data: &[u8]) -> Result<Vec<MapFeature>, String> {
    geojson::decode(data).map(|objects| from_objects(&objects))
}

/// Converts decoded GeoJSON objects into map items. A feature without a geometry gives no item.
pub fn from_objects(objects: &[GeoJsonObject]) -> Vec<MapFeature> {
    let mut out = Vec::new();
    for object in objects {
        match object {
            GeoJsonObject::Feature(feature) => out.extend(feature_items(feature)),
            GeoJsonObject::FeatureCollection(features) => out.extend(features.iter().flat_map(feature_items)),
            GeoJsonObject::Geometry(geometry) => out.extend(geometry_items(geometry, &None, &None)),
        }
    }
    out
}

fn feature_items(feature: &Feature) -> Vec<MapFeature> {
    let name = feature.properties.as_ref().and_then(|p| p.str_field("name"));
    match &feature.geometry {
        Some(geometry) => geometry_items(geometry, &feature.identifier, &name),
        None => Vec::new(),
    }
}

fn coordinate(p: Position) -> Option<Coordinate> {
    Coordinate::new(p.latitude, p.longitude)
}

fn coordinates(points: &[Position]) -> Option<Vec<Coordinate>> {
    points.iter().map(|p| coordinate(*p)).collect()
}

/// A ring as coordinates, without the repeat of the first position that closes it.
fn open_ring(ring: &[Position]) -> Option<Vec<Coordinate>> {
    let mut open = ring.to_vec();
    if open.len() > 1 && open.first() == open.last() {
        open.pop();
    }
    coordinates(&open)
}

/// The exterior ring first, then the holes, as [`MapPolygon`] takes them.
fn polygon(rings: &[Vec<Position>]) -> Option<MapPolygon> {
    let (outer, holes) = rings.split_first()?;
    let outer = open_ring(outer)?;
    let interior = holes.iter().map(|h| open_ring(h)).collect::<Option<Vec<_>>>()?;
    MapPolygon::new(outer, interior)
}

fn geometry_items(geometry: &Geometry, identifier: &Option<String>, name: &Option<String>) -> Vec<MapFeature> {
    let marker = |p: &Position| {
        coordinate(*p).map(|c| MapFeature::Marker {
            coordinate: c,
            identifier: identifier.clone(),
            name: name.clone(),
        })
    };
    let line = |points: &[Position]| {
        coordinates(points)
            .and_then(MapPolyline::new)
            .map(|line| MapFeature::Line {
                line,
                identifier: identifier.clone(),
                name: name.clone(),
            })
    };
    let area = |rings: &[Vec<Position>]| {
        polygon(rings).map(|polygon| MapFeature::Area {
            polygon,
            identifier: identifier.clone(),
            name: name.clone(),
        })
    };
    match geometry {
        Geometry::Point(p) => marker(p).into_iter().collect(),
        Geometry::MultiPoint(points) => points.iter().filter_map(marker).collect(),
        Geometry::LineString(points) => line(points.as_slice()).into_iter().collect(),
        Geometry::MultiLineString(lines) => lines.iter().filter_map(|l| line(l.as_slice())).collect(),
        Geometry::Polygon(rings) => area(rings.as_slice()).into_iter().collect(),
        Geometry::MultiPolygon(polygons) => polygons.iter().filter_map(|r| area(r.as_slice())).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"type":"FeatureCollection","features":[
        {"type":"Feature","id":"stop-1","properties":{"name":"Harbour"},
         "geometry":{"type":"Point","coordinates":[-21.94,64.15]}},
        {"type":"Feature","properties":null,
         "geometry":{"type":"LineString","coordinates":[[-21.9,64.1],[-21.8,64.2]]}},
        {"type":"Feature",
         "geometry":{"type":"Polygon","coordinates":[
            [[-21.0,64.0],[-20.0,64.0],[-20.0,65.0],[-21.0,65.0],[-21.0,64.0]],
            [[-20.8,64.2],[-20.2,64.2],[-20.2,64.8],[-20.8,64.8],[-20.8,64.2]]]}},
        {"type":"Feature","geometry":null}
    ]}"#;

    #[test]
    fn feature_collection_becomes_one_item_per_feature_with_geometry() {
        let items = decode(SAMPLE.as_bytes()).expect("sample decodes");
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn point_keeps_identifier_and_name() {
        let items = decode(SAMPLE.as_bytes()).expect("sample decodes");
        match &items[0] {
            MapFeature::Marker {
                coordinate,
                identifier,
                name,
            } => {
                assert_eq!(coordinate.latitude, 64.15);
                assert_eq!(coordinate.longitude, -21.94);
                assert_eq!(identifier.as_deref(), Some("stop-1"));
                assert_eq!(name.as_deref(), Some("Harbour"));
            }
            other => panic!("expected a marker, got {other:?}"),
        }
    }

    #[test]
    fn line_has_its_positions_and_no_name() {
        let items = decode(SAMPLE.as_bytes()).expect("sample decodes");
        match &items[1] {
            MapFeature::Line { line, identifier, name } => {
                assert_eq!(line.coordinates.len(), 2);
                assert!(identifier.is_none());
                assert!(name.is_none());
                assert!(line.length_m() > 0.0);
            }
            other => panic!("expected a line, got {other:?}"),
        }
    }

    #[test]
    fn polygon_drops_closing_positions_and_keeps_the_hole() {
        let items = decode(SAMPLE.as_bytes()).expect("sample decodes");
        match &items[2] {
            MapFeature::Area { polygon, .. } => {
                assert_eq!(polygon.outer.len(), 4);
                assert_eq!(polygon.interior.len(), 1);
                assert_eq!(polygon.interior[0].len(), 4);
                assert!(polygon.area_m2() > 0.0);
                assert!(!polygon.contains(Coordinate::new(64.5, -20.5).expect("valid")));
            }
            other => panic!("expected an area, got {other:?}"),
        }
    }

    #[test]
    fn multi_point_gives_one_marker_per_part() {
        let items = decode(br#"{"type":"MultiPoint","coordinates":[[1,2],[3,4]]}"#).expect("decodes");
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| matches!(i, MapFeature::Marker { identifier: None, .. })));
    }

    #[test]
    fn multi_polygon_gives_one_area_per_part() {
        let text = r#"{"type":"MultiPolygon","coordinates":[
            [[[0,0],[1,0],[1,1],[0,0]]],
            [[[5,5],[6,5],[6,6],[5,5]]]]}"#;
        let items = decode(text.as_bytes()).expect("decodes");
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|i| matches!(i, MapFeature::Area { .. })));
    }

    #[test]
    fn invalid_geojson_is_an_error() {
        assert!(decode(b"not json").is_err());
        assert!(decode(br#"{"type":"Point","coordinates":[0,95]}"#).is_err());
    }
}
