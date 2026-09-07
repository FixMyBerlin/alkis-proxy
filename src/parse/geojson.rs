//! Direktpfad für Dienste, die bereits GeoJSON liefern (Berlin, Baden-Württemberg,
//! Rheinland-Pfalz).
//!
//! Das Ergebnis ist dasselbe [`RawFeature`] wie beim GML-Pfad. Dadurch sind die
//! Adapter formatunabhängig: Sie sehen nicht, ob eine Antwort als XML oder JSON
//! hereinkam.

use serde_json::Value;

use crate::model::parcel::{PolygonRings, Ring};
use crate::model::Geometry;
use crate::parse::RawFeature;

#[derive(Debug, thiserror::Error)]
pub enum GeoJsonError {
    #[error("Antwort ist kein gültiges JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Erwartet wurde eine FeatureCollection, gefunden: {found}")]
    NotAFeatureCollection { found: String },
    #[error("Der Dienst meldet eine OGC-Exception: {0}")]
    OgcException(String),
    #[error("Geometrietyp {kind:?} wird nicht unterstützt")]
    UnsupportedGeometry { kind: String },
}

/// Zerlegt eine GeoJSON-`FeatureCollection` in ihre Features.
pub fn parse_geojson(body: &str) -> Result<Vec<RawFeature>, GeoJsonError> {
    let root: Value = serde_json::from_str(body)?;

    // Manche Dienste liefern Fehler als JSON mit HTTP 200.
    if let Some(msg) = root.get("exceptions").and_then(exception_text) {
        return Err(GeoJsonError::OgcException(msg));
    }

    match root.get("type").and_then(Value::as_str) {
        Some("FeatureCollection") => {}
        other => {
            return Err(GeoJsonError::NotAFeatureCollection {
                found: other.unwrap_or("kein type-Feld").to_string(),
            })
        }
    }

    let features = root
        .get("features")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();

    features.iter().map(convert_feature).collect()
}

fn convert_feature(f: &Value) -> Result<RawFeature, GeoJsonError> {
    let mut raw = RawFeature::default();

    if let Some(props) = f.get("properties").and_then(Value::as_object) {
        for (k, v) in props {
            if let Some(s) = scalar_to_string(v) {
                raw.fields.insert(k.to_ascii_lowercase(), s);
            }
        }
    }

    // Die Feature-ID kann als Kennzeichen-Fallback dienen.
    if let Some(id) = f.get("id").and_then(Value::as_str) {
        raw.fields.entry("id".into()).or_insert_with(|| id.to_string());
    }

    if let Some(geom) = f.get("geometry") {
        raw.geometry = convert_geometry(geom)?;
    }

    Ok(raw)
}

/// Skalare Property-Werte als String; Objekte und Arrays werden übergangen.
fn scalar_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// GeoJSON-Geometrie → [`Geometry`]. `Polygon` wird zu `MultiPolygon`
/// hochgestuft (Baden-Württemberg liefert einzelne Polygone).
fn convert_geometry(g: &Value) -> Result<Option<Geometry>, GeoJsonError> {
    let kind = match g.get("type").and_then(Value::as_str) {
        Some(k) => k,
        None => return Ok(None),
    };
    let coords = match g.get("coordinates") {
        Some(c) => c,
        None => return Ok(None),
    };

    let polygons = match kind {
        "Polygon" => vec![rings(coords)],
        "MultiPolygon" => coords
            .as_array()
            .map(|ps| ps.iter().map(rings).collect())
            .unwrap_or_default(),
        other => {
            return Err(GeoJsonError::UnsupportedGeometry {
                kind: other.to_string(),
            })
        }
    };

    let geom = Geometry::multi_polygon(polygons);
    Ok((!geom.is_empty()).then_some(geom))
}

fn rings(v: &Value) -> PolygonRings {
    v.as_array()
        .map(|rs| rs.iter().map(ring).collect())
        .unwrap_or_default()
}

fn ring(v: &Value) -> Ring {
    v.as_array()
        .map(|ps| {
            ps.iter()
                .filter_map(|p| {
                    let a = p.as_array()?;
                    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
                })
                .collect()
        })
        .unwrap_or_default()
}

fn exception_text(v: &Value) -> Option<String> {
    let first = v.as_array()?.first()?;
    let text = first
        .get("exceptionText")
        .or_else(|| first.get("text"))
        .and_then(Value::as_str)?;
    Some(text.chars().take(300).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BERLIN: &str = include_str!("../../tests/fixtures/berlin.json");
    const BW: &str = include_str!("../../tests/fixtures/bw_nora.json");

    #[test]
    fn berlin_wird_gelesen() {
        let fs = parse_geojson(BERLIN).unwrap();
        assert_eq!(fs.len(), 2);
        let f = &fs[0];
        assert!(f.field(&["fsko"]).is_some());
        assert!(f.field(&["namgmk"]).is_some());
        // Zahlen kommen als Text an und lassen sich zurückparsen.
        assert!(f.number(&["afl"]).is_some());
        let g = f.geometry.as_ref().unwrap();
        assert!(!g.is_empty());
    }

    #[test]
    fn baden_wuerttemberg_polygon_wird_hochgestuft() {
        let fs = parse_geojson(BW).unwrap();
        assert_eq!(fs.len(), 2);
        let f = &fs[0];
        assert!(f.field(&["flurstueckskennzeichen"]).is_some());
        // BW liefert "Polygon"; nach der Konvertierung liegt ein MultiPolygon vor.
        let g = f.geometry.as_ref().unwrap();
        assert_eq!(g.polygons.len(), 1);
        assert!(g.polygons[0][0].len() > 3);
    }

    #[test]
    fn koordinaten_bleiben_im_nativen_crs() {
        // BW wurde in EPSG:25832 angefragt.
        let fs = parse_geojson(BW).unwrap();
        let [x, y] = fs[0].geometry.as_ref().unwrap().polygons[0][0][0];
        assert!(x > 100_000.0, "x={x} sieht nicht nach UTM aus");
        assert!(y > 5_000_000.0, "y={y} sieht nicht nach UTM aus");
    }

    #[test]
    fn nicht_featurecollection_wird_abgelehnt() {
        let err = parse_geojson(r#"{"type":"Feature"}"#).unwrap_err();
        assert!(matches!(err, GeoJsonError::NotAFeatureCollection { .. }));
    }

    #[test]
    fn leere_collection_ist_kein_fehler() {
        let fs = parse_geojson(r#"{"type":"FeatureCollection","features":[]}"#).unwrap();
        assert!(fs.is_empty());
    }

    #[test]
    fn punktgeometrie_wird_abgelehnt() {
        let body = r#"{"type":"FeatureCollection","features":[
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[1,2]}}]}"#;
        assert!(matches!(
            parse_geojson(body),
            Err(GeoJsonError::UnsupportedGeometry { .. })
        ));
    }
}
