//! Räumlicher Index über die indizierten OSM-Features (R-Tree, resident im
//! Speicher) und die Verschneidung eines Flurstücks mit seinen Treffern.
//!
//! Struktur entspricht `intersect.rs` in `ist-dieses-flurstueck-oeffentlich`,
//! arbeitet aber direkt in WGS84 (Grad) statt EPSG:25832 — siehe
//! `classification/mod.rs` für die Begründung, warum das für
//! Flächen*verhältnisse* auf Flurstücksgröße unproblematisch ist.

use geo::{Area, BooleanOps, BoundingRect, Intersects};
use geo_types::{Geometry, MultiPolygon};
use rstar::{RTree, RTreeObject, AABB};

use crate::classification::OsmFeature;

/// Ein OSM-Feature im R-Tree, referenziert über seinen Index in der
/// Feature-Liste (spart eine zweite Kopie der Geometrie im Baum).
struct IndexedFeature {
    idx: usize,
    envelope: AABB<[f64; 2]>,
}

impl RTreeObject for IndexedFeature {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

/// Resident gehaltener räumlicher Index über alle indizierten OSM-Features.
pub struct SpatialIndex {
    features: Vec<OsmFeature>,
    tree: RTree<IndexedFeature>,
}

impl SpatialIndex {
    pub fn build(features: Vec<OsmFeature>) -> Self {
        let items: Vec<IndexedFeature> = features
            .iter()
            .enumerate()
            .filter_map(|(idx, f)| {
                let rect = f.geom.bounding_rect()?;
                Some(IndexedFeature {
                    idx,
                    envelope: AABB::from_corners(
                        [rect.min().x, rect.min().y],
                        [rect.max().x, rect.max().y],
                    ),
                })
            })
            .collect();
        let tree = RTree::bulk_load(items);
        SpatialIndex { features, tree }
    }

    pub fn len(&self) -> usize {
        self.features.len()
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    /// Ermittelt den OSM-Kontext (schneidende Features + Überlappungsanteile)
    /// für ein Flurstück.
    pub fn context_for<'a>(&'a self, geom: &MultiPolygon<f64>) -> OsmContext<'a> {
        let fs_area = geom.unsigned_area();
        let Some(rect) = geom.bounding_rect() else {
            return OsmContext::default();
        };
        let query = AABB::from_corners([rect.min().x, rect.min().y], [rect.max().x, rect.max().y]);

        let mut matches = Vec::new();
        for item in self.tree.locate_in_envelope_intersecting(query) {
            let feature = &self.features[item.idx];
            if let Some(ratio) = overlap_ratio(geom, fs_area, &feature.geom) {
                matches.push(OsmMatch { feature, overlap_ratio: ratio });
            }
        }
        OsmContext { matches }
    }
}

/// Ein OSM-Feature, das ein Flurstück schneidet, mit Flächen-Überlappungsanteil
/// (0.0 für Linien/Punkte).
#[derive(Debug, Clone, Copy)]
pub struct OsmMatch<'a> {
    pub feature: &'a OsmFeature,
    pub overlap_ratio: f64,
}

/// Alle OSM-Treffer für ein Flurstück — Eingabe der Regel-Engine.
#[derive(Debug, Clone, Default)]
pub struct OsmContext<'a> {
    pub matches: Vec<OsmMatch<'a>>,
}

impl<'a> OsmContext<'a> {
    /// Ob irgendein Treffer das Flag trägt.
    pub fn any_flag(&self, flag: u32) -> bool {
        self.matches.iter().any(|m| m.feature.flags & flag != 0)
    }

    /// Größter Überlappungsanteil eines Treffers, der das Flag trägt.
    pub fn max_overlap(&self, flag: u32) -> f64 {
        self.matches
            .iter()
            .filter(|m| m.feature.flags & flag != 0)
            .map(|m| m.overlap_ratio)
            .fold(0.0, f64::max)
    }
}

/// Überlappungsanteil (0..1) eines OSM-Features an der Flurstücksfläche.
/// Für Linien/Punkte: `Some(0.0)` bei Schnitt, sonst `None`.
fn overlap_ratio(fs: &MultiPolygon<f64>, fs_area: f64, osm: &Geometry<f64>) -> Option<f64> {
    match osm {
        Geometry::Polygon(p) => {
            let mp = MultiPolygon::new(vec![p.clone()]);
            area_ratio(fs, fs_area, &mp)
        }
        Geometry::MultiPolygon(mp) => area_ratio(fs, fs_area, mp),
        other => {
            if fs.intersects(other) {
                Some(0.0)
            } else {
                None
            }
        }
    }
}

fn area_ratio(fs: &MultiPolygon<f64>, fs_area: f64, other: &MultiPolygon<f64>) -> Option<f64> {
    // Billiger Intersects-Vorfilter, bevor das teure Polygon-Clipping läuft:
    // die meisten R-Tree-Kandidaten überlappen sich nur in der Bounding-Box.
    if !fs.intersects(other) {
        return None;
    }
    let inter = fs.intersection(other);
    let a = inter.unsigned_area();
    if a <= 0.0 {
        return None;
    }
    Some(if fs_area > 0.0 { a / fs_area } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo_types::{Coord, LineString, Polygon};

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> MultiPolygon<f64> {
        MultiPolygon::new(vec![Polygon::new(
            LineString::new(vec![
                Coord { x: x0, y: y0 },
                Coord { x: x1, y: y0 },
                Coord { x: x1, y: y1 },
                Coord { x: x0, y: y1 },
                Coord { x: x0, y: y0 },
            ]),
            vec![],
        )])
    }

    #[test]
    fn volle_ueberdeckung_ergibt_ratio_eins() {
        let fs = square(0.0, 0.0, 1.0, 1.0);
        let osm = Geometry::MultiPolygon(square(-1.0, -1.0, 2.0, 2.0));
        let ratio = overlap_ratio(&fs, fs.unsigned_area(), &osm).unwrap();
        assert!((ratio - 1.0).abs() < 1e-9, "{ratio}");
    }

    #[test]
    fn index_findet_ueberlappendes_feature() {
        let feature = OsmFeature {
            flags: crate::classification::rules::flags::PARK,
            geom: Geometry::MultiPolygon(square(0.0, 0.0, 10.0, 10.0)),
        };
        let index = SpatialIndex::build(vec![feature]);
        assert_eq!(index.len(), 1);
        let ctx = index.context_for(&square(1.0, 1.0, 2.0, 2.0));
        assert_eq!(ctx.matches.len(), 1);
        assert!(ctx.any_flag(crate::classification::rules::flags::PARK));
    }

    #[test]
    fn index_ignoriert_entferntes_feature() {
        let feature = OsmFeature {
            flags: crate::classification::rules::flags::PARK,
            geom: Geometry::MultiPolygon(square(100.0, 100.0, 101.0, 101.0)),
        };
        let index = SpatialIndex::build(vec![feature]);
        let ctx = index.context_for(&square(0.0, 0.0, 1.0, 1.0));
        assert!(ctx.matches.is_empty());
    }
}
