//! Wandelt die interne [`Geometry`] (native CRS, wie sie `parse::gml`/
//! `parse::geojson` liefern) in `geo_types::MultiPolygon<f64>` — Eingabe für
//! die Verschneidung in [`crate::nutzung::intersect`].
//!
//! Eigenständig statt `classification::geom` mitzubenutzen: Jenes Modul
//! wandelt bereits serialisierte GeoJSON-`Value`s (WGS84, aus dem Cache), hier
//! liegt das interne [`Geometry`] direkt vor (natives CRS, vor der
//! Reprojektion) — andere Quelle, gleiche Zielstruktur.

use geo_types::{Coord, LineString, MultiPolygon, Polygon};

use crate::model::Geometry;

/// `None` bei leerer Geometrie oder einem Ring mit weniger als vier Punkten.
pub fn to_multipolygon(geom: &Geometry) -> Option<MultiPolygon<f64>> {
    let polygons: Vec<Polygon<f64>> = geom.polygons.iter().filter_map(|rings| polygon(rings)).collect();
    if polygons.is_empty() {
        return None;
    }
    Some(MultiPolygon::new(polygons))
}

fn polygon(rings: &[Vec<[f64; 2]>]) -> Option<Polygon<f64>> {
    let mut iter = rings.iter();
    let outer = ring(iter.next()?);
    if outer.0.len() < 4 {
        return None;
    }
    let holes: Vec<LineString<f64>> = iter.map(|r| ring(r)).collect();
    Some(Polygon::new(outer, holes))
}

fn ring(points: &[[f64; 2]]) -> LineString<f64> {
    LineString::new(points.iter().map(|p| Coord { x: p[0], y: p[1] }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadrat_wird_gelesen() {
        let geom = Geometry::from_polygon(vec![vec![
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
            [0.0, 0.0],
        ]]);
        let mp = to_multipolygon(&geom).unwrap();
        assert_eq!(mp.0.len(), 1);
        assert_eq!(mp.0[0].exterior().0.len(), 5);
    }

    #[test]
    fn leere_geometrie_liefert_none() {
        let geom = Geometry::multi_polygon(vec![]);
        assert!(to_multipolygon(&geom).is_none());
    }

    #[test]
    fn zu_kurzer_ring_liefert_none() {
        let geom = Geometry::from_polygon(vec![vec![[0.0, 0.0], [1.0, 1.0]]]);
        assert!(to_multipolygon(&geom).is_none());
    }

    #[test]
    fn loecher_werden_uebernommen() {
        let geom = Geometry::from_polygon(vec![
            vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]],
            vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 1.0]],
        ]);
        let mp = to_multipolygon(&geom).unwrap();
        assert_eq!(mp.0[0].interiors().len(), 1);
    }
}
