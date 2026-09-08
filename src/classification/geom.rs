//! Wandelt die GeoJSON-Geometrie eines bereits serialisierten Flurstück-Features
//! (Format wie von `model::Parcel::to_feature()` erzeugt — immer
//! `MultiPolygon`, WGS84, sieben Nachkommastellen) in `geo_types::MultiPolygon<f64>`
//! für die Verschneidung mit dem OSM-Index.
//!
//! Bewusst kein neuer `geojson`-Crate: Das Format ist vollständig bekannt und
//! selbst erzeugt (siehe `model/parcel.rs`), ein paar Zeilen im Stil von
//! `parse/geojson.rs::rings`/`ring` reichen — nur in Gegenrichtung.

use geo_types::{Coord, LineString, MultiPolygon, Polygon};
use serde_json::Value;

/// Liest die Geometrie eines GeoJSON-`Feature`-Objekts. `None`, wenn das
/// Feature keine, eine leere oder eine nicht unterstützte Geometrie hat.
pub fn multipolygon_from_feature(feature: &Value) -> Option<MultiPolygon<f64>> {
    let geometry = feature.get("geometry")?;
    if geometry.get("type")?.as_str()? != "MultiPolygon" {
        return None;
    }
    let coords = geometry.get("coordinates")?.as_array()?;
    let polygons: Vec<Polygon<f64>> = coords.iter().filter_map(polygon).collect();
    if polygons.is_empty() {
        return None;
    }
    Some(MultiPolygon::new(polygons))
}

fn polygon(v: &Value) -> Option<Polygon<f64>> {
    let rings = v.as_array()?;
    let mut iter = rings.iter();
    let outer = ring(iter.next()?)?;
    if outer.0.len() < 4 {
        return None;
    }
    let holes: Vec<LineString<f64>> = iter.filter_map(ring).collect();
    Some(Polygon::new(outer, holes))
}

fn ring(v: &Value) -> Option<LineString<f64>> {
    let points = v.as_array()?;
    let coords: Vec<Coord<f64>> = points
        .iter()
        .filter_map(|p| {
            let a = p.as_array()?;
            Some(Coord {
                x: a.first()?.as_f64()?,
                y: a.get(1)?.as_f64()?,
            })
        })
        .collect();
    Some(LineString::new(coords))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn multipolygon_wird_gelesen() {
        let feature = json!({
            "type": "Feature",
            "geometry": {
                "type": "MultiPolygon",
                "coordinates": [[[[9.0, 48.0], [9.1, 48.0], [9.1, 48.1], [9.0, 48.0]]]]
            }
        });
        let mp = multipolygon_from_feature(&feature).unwrap();
        assert_eq!(mp.0.len(), 1);
        assert_eq!(mp.0[0].exterior().0.len(), 4);
    }

    #[test]
    fn fehlende_geometrie_liefert_none() {
        assert!(multipolygon_from_feature(&json!({"type": "Feature"})).is_none());
    }

    #[test]
    fn anderer_geometrietyp_liefert_none() {
        let feature = json!({
            "type": "Feature",
            "geometry": { "type": "Point", "coordinates": [9.0, 48.0] }
        });
        assert!(multipolygon_from_feature(&feature).is_none());
    }

    #[test]
    fn loecher_werden_uebernommen() {
        let feature = json!({
            "type": "Feature",
            "geometry": {
                "type": "MultiPolygon",
                "coordinates": [[
                    [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]],
                    [[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 1.0]]
                ]]
            }
        });
        let mp = multipolygon_from_feature(&feature).unwrap();
        assert_eq!(mp.0[0].interiors().len(), 1);
    }
}
