//! Räumlicher Index über die indizierten OSM-Features (R-Tree, resident im
//! Speicher) und die Verschneidung eines Flurstücks mit seinen Treffern.
//!
//! Struktur entspricht `intersect.rs` in `ist-dieses-flurstueck-oeffentlich`,
//! arbeitet aber direkt in WGS84 (Grad) statt EPSG:25832 — siehe
//! `classification/mod.rs` für die Begründung, warum das für
//! Flächen*verhältnisse* auf Flurstücksgröße unproblematisch ist.
//!
//! Für die Verkehrsflächen-Erkennung (`rules::LineCoverageRule`) reichen
//! Verhältnisse allerdings nicht: Dort geht eine Nennbreite in Metern ein.
//! Deshalb liefert [`OsmContext`] zusätzlich metrische Größen, umgerechnet mit
//! einem lokalen äquirektangulären Faktor an der Flurstücks-Bbox (siehe
//! [`meter_pro_grad`]).

use std::f64::consts::PI;

use geo::{Area, BooleanOps, BoundingRect, Intersects};
use geo_types::{Geometry, LineString, MultiLineString, MultiPolygon};
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

    /// Ermittelt den OSM-Kontext (schneidende Features, Überlappungsanteile,
    /// Schnittlängen) für ein Flurstück.
    pub fn context_for<'a>(&'a self, geom: &MultiPolygon<f64>) -> OsmContext<'a> {
        let fs_area_grad = geom.unsigned_area();
        let Some(rect) = geom.bounding_rect() else {
            return OsmContext::default();
        };
        let (mx, my) = meter_pro_grad((rect.min().y + rect.max().y) / 2.0);
        let query = AABB::from_corners([rect.min().x, rect.min().y], [rect.max().x, rect.max().y]);

        let mut matches = Vec::new();
        for item in self.tree.locate_in_envelope_intersecting(query) {
            let feature = &self.features[item.idx];
            if let Some((overlap_ratio, clipped_length_m)) =
                verschneidung(geom, fs_area_grad, &feature.geom, mx, my)
            {
                matches.push(OsmMatch { feature, overlap_ratio, clipped_length_m });
            }
        }

        let umfang_m = umfang_m(geom, mx, my);
        let area_m2 = fs_area_grad * mx * my;
        OsmContext {
            matches,
            area_m2,
            compactness: kompaktheit(area_m2, umfang_m),
        }
    }
}

/// Ein OSM-Feature, das ein Flurstück schneidet.
#[derive(Debug, Clone, Copy)]
pub struct OsmMatch<'a> {
    pub feature: &'a OsmFeature,
    /// Flächenanteil an der Flurstücksfläche; `0.0` für Linien und Punkte.
    pub overlap_ratio: f64,
    /// Länge der Achse innerhalb des Flurstücks in Metern; `0.0` für Flächen
    /// und Punkte.
    pub clipped_length_m: f64,
}

/// Alle OSM-Treffer für ein Flurstück samt dessen Maßen — Eingabe der
/// Regel-Engine.
#[derive(Debug, Clone, Default)]
pub struct OsmContext<'a> {
    pub matches: Vec<OsmMatch<'a>>,
    /// Fläche des Flurstücks in Quadratmetern.
    pub area_m2: f64,
    /// Polsby-Popper-Kompaktheit `4πA/U²`: 1,0 für einen Kreis, gegen 0 für
    /// schmale, langgestreckte Flächen. Siehe `rules::MAX_KOMPAKTHEIT`.
    pub compactness: f64,
}

impl OsmContext<'_> {
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

    /// Anteil der Flurstücksfläche, den die Achsen mit diesem Flag
    /// beanspruchen: Schnittlänge × Nennbreite, summiert über alle Treffer und
    /// bei 1,0 gekappt.
    ///
    /// Summiert, nicht maximiert: Fahrbahn und separat gemappter Gehweg im
    /// selben Straßenflurstück gehören beide zur Verkehrsfläche, ebenso die
    /// parallelen Ways einer zweigleisigen Bahnstrecke. Die Kappung fängt die
    /// Doppelzählung ab, die dabei entstehen kann.
    pub fn line_coverage(&self, flag: u32) -> f64 {
        if self.area_m2 <= 0.0 {
            return 0.0;
        }
        let belegt: f64 = self
            .matches
            .iter()
            .filter(|m| m.feature.flags & flag != 0)
            .map(|m| m.clipped_length_m * f64::from(m.feature.width_dm) / 10.0)
            .sum();
        (belegt / self.area_m2).min(1.0)
    }
}

/// Meter je Grad Länge und je Grad Breite auf der Breite `lat` — lokale
/// äquirektanguläre Näherung. Auf Flurstücksgröße (< 1 km) liegt ihr Fehler
/// weit unter 0,1 % und damit weit unter der Unschärfe der Nennbreiten, die
/// damit verrechnet werden.
fn meter_pro_grad(lat: f64) -> (f64, f64) {
    (111_320.0 * lat.to_radians().cos(), 111_200.0)
}

/// Verschneidet ein Flurstück mit einem OSM-Feature. `None`, wenn sie sich
/// nicht berühren; sonst `(Flächenanteil, Schnittlänge in Metern)` — je nach
/// Geometrieart ist genau einer der beiden Werte belegt.
fn verschneidung(
    fs: &MultiPolygon<f64>,
    fs_area_grad: f64,
    osm: &Geometry<f64>,
    mx: f64,
    my: f64,
) -> Option<(f64, f64)> {
    match osm {
        Geometry::Polygon(p) => {
            let mp = MultiPolygon::new(vec![p.clone()]);
            Some((flaechenanteil(fs, fs_area_grad, &mp)?, 0.0))
        }
        Geometry::MultiPolygon(mp) => Some((flaechenanteil(fs, fs_area_grad, mp)?, 0.0)),
        Geometry::LineString(ls) => {
            let mls = MultiLineString::new(vec![ls.clone()]);
            Some((0.0, schnittlaenge_m(fs, &mls, mx, my)?))
        }
        Geometry::MultiLineString(mls) => Some((0.0, schnittlaenge_m(fs, mls, mx, my)?)),
        other => {
            if fs.intersects(other) {
                Some((0.0, 0.0))
            } else {
                None
            }
        }
    }
}

fn flaechenanteil(
    fs: &MultiPolygon<f64>,
    fs_area_grad: f64,
    other: &MultiPolygon<f64>,
) -> Option<f64> {
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
    // Beide Flächen stehen in Grad²; der Skalenfaktor kürzt sich im Verhältnis
    // heraus, die Umrechnung nach Metern wäre hier überflüssig.
    Some(if fs_area_grad > 0.0 { a / fs_area_grad } else { 0.0 })
}

/// Länge des Teils der Achse, der innerhalb des Flurstücks liegt, in Metern.
/// `None`, wenn sich beide nicht berühren.
///
/// Geclippt wird in Grad — die Topologie ist unter der affinen Skalierung nach
/// Metern invariant, die Schnittpunkte sind dieselben. Nur das Ergebnis wird
/// metrisch vermessen, was den Umweg über eine Vortransformation der ganzen
/// Achse erspart.
fn schnittlaenge_m(
    fs: &MultiPolygon<f64>,
    achse: &MultiLineString<f64>,
    mx: f64,
    my: f64,
) -> Option<f64> {
    if !fs.intersects(achse) {
        return None;
    }
    let innen = fs.clip(achse, false);
    Some(innen.iter().map(|ls| laenge_m(ls, mx, my)).sum())
}

fn laenge_m(ls: &LineString<f64>, mx: f64, my: f64) -> f64 {
    ls.lines()
        .map(|l| {
            let dx = (l.end.x - l.start.x) * mx;
            let dy = (l.end.y - l.start.y) * my;
            dx.hypot(dy)
        })
        .sum()
}

/// Umfang des Flurstücks in Metern, über alle Ringe (auch Löcher).
fn umfang_m(fs: &MultiPolygon<f64>, mx: f64, my: f64) -> f64 {
    fs.iter()
        .map(|p| {
            laenge_m(p.exterior(), mx, my)
                + p.interiors().iter().map(|r| laenge_m(r, mx, my)).sum::<f64>()
        })
        .sum()
}

fn kompaktheit(area_m2: f64, umfang_m: f64) -> f64 {
    if umfang_m <= 0.0 {
        return 0.0;
    }
    4.0 * PI * area_m2 / (umfang_m * umfang_m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classification::rules::flags;
    use geo_types::{Coord, Polygon};

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

    /// Rechteck in Metern um einen Punkt bei 48° N, als WGS84-Polygon.
    fn rechteck_m(breite_m: f64, hoehe_m: f64) -> MultiPolygon<f64> {
        let (mx, my) = meter_pro_grad(48.0);
        square(9.0, 48.0, 9.0 + breite_m / mx, 48.0 + hoehe_m / my)
    }

    fn feature(flags: u32, width_dm: u16, geom: Geometry<f64>) -> OsmFeature {
        OsmFeature { flags, width_dm, geom }
    }

    #[test]
    fn volle_ueberdeckung_ergibt_ratio_eins() {
        let fs = square(0.0, 0.0, 1.0, 1.0);
        let osm = Geometry::MultiPolygon(square(-1.0, -1.0, 2.0, 2.0));
        let (ratio, _) = verschneidung(&fs, fs.unsigned_area(), &osm, 1.0, 1.0).unwrap();
        assert!((ratio - 1.0).abs() < 1e-9, "{ratio}");
    }

    #[test]
    fn index_findet_ueberlappendes_feature() {
        let f = feature(flags::PARK, 0, Geometry::MultiPolygon(square(0.0, 0.0, 10.0, 10.0)));
        let index = SpatialIndex::build(vec![f]);
        assert_eq!(index.len(), 1);
        let ctx = index.context_for(&square(1.0, 1.0, 2.0, 2.0));
        assert_eq!(ctx.matches.len(), 1);
        assert!(ctx.any_flag(flags::PARK));
    }

    #[test]
    fn index_ignoriert_entferntes_feature() {
        let f = feature(flags::PARK, 0, Geometry::MultiPolygon(square(100.0, 100.0, 101.0, 101.0)));
        let index = SpatialIndex::build(vec![f]);
        let ctx = index.context_for(&square(0.0, 0.0, 1.0, 1.0));
        assert!(ctx.matches.is_empty());
    }

    #[test]
    fn flaeche_und_umfang_stimmen_in_metern() {
        // 8 × 200 m bei 48° N.
        let fs = rechteck_m(8.0, 200.0);
        let (mx, my) = meter_pro_grad(48.0);
        let area = fs.unsigned_area() * mx * my;
        assert!((area - 1600.0).abs() < 1.0, "Fläche {area}");
        let umfang = umfang_m(&fs, mx, my);
        assert!((umfang - 416.0).abs() < 1.0, "Umfang {umfang}");
    }

    #[test]
    fn kompaktheit_trennt_strassen_von_baugrundstuecken() {
        let (mx, my) = meter_pro_grad(48.0);
        let kompakt = |b, h| {
            let fs = rechteck_m(b, h);
            kompaktheit(fs.unsigned_area() * mx * my, umfang_m(&fs, mx, my))
        };
        assert!(kompakt(8.0, 200.0) < 0.15, "{}", kompakt(8.0, 200.0));
        assert!(kompakt(3.0, 500.0) < 0.05, "{}", kompakt(3.0, 500.0));
        assert!(kompakt(6.0, 30.0) > 0.4, "{}", kompakt(6.0, 30.0));
        assert!(kompakt(20.0, 20.0) > 0.7, "{}", kompakt(20.0, 20.0));
    }

    #[test]
    fn achse_laengs_durch_das_flurstueck_deckt_es_ab() {
        // Straßenflurstück 8 × 200 m mit einer Achse auf der Mittellinie.
        let fs = rechteck_m(8.0, 200.0);
        let (mx, my) = meter_pro_grad(48.0);
        let achse = LineString::new(vec![
            Coord { x: 9.0 + 4.0 / mx, y: 48.0 },
            Coord { x: 9.0 + 4.0 / mx, y: 48.0 + 200.0 / my },
        ]);
        let f = feature(flags::ROAD, 65, Geometry::LineString(achse));
        let index = SpatialIndex::build(vec![f]);
        let ctx = index.context_for(&fs);

        assert_eq!(ctx.matches.len(), 1);
        let laenge = ctx.matches[0].clipped_length_m;
        assert!((laenge - 200.0).abs() < 1.0, "Länge {laenge}");
        // 200 m × 6,5 m / 1600 m² = 0,81
        let deckung = ctx.line_coverage(flags::ROAD);
        assert!((deckung - 0.8125).abs() < 0.02, "Deckung {deckung}");
    }

    #[test]
    fn achse_wird_an_der_flurstuecksgrenze_abgeschnitten() {
        // Dieselbe Achse, aber viermal so lang wie das Flurstück: gezählt
        // werden darf nur der innenliegende Teil.
        let fs = rechteck_m(8.0, 200.0);
        let (mx, my) = meter_pro_grad(48.0);
        let achse = LineString::new(vec![
            Coord { x: 9.0 + 4.0 / mx, y: 48.0 - 300.0 / my },
            Coord { x: 9.0 + 4.0 / mx, y: 48.0 + 500.0 / my },
        ]);
        let f = feature(flags::ROAD, 65, Geometry::LineString(achse));
        let index = SpatialIndex::build(vec![f]);
        let ctx = index.context_for(&fs);
        let laenge = ctx.matches[0].clipped_length_m;
        assert!((laenge - 200.0).abs() < 1.0, "Länge {laenge}");
    }

    #[test]
    fn achse_daneben_liefert_keinen_treffer() {
        let fs = rechteck_m(8.0, 200.0);
        let (mx, my) = meter_pro_grad(48.0);
        // Parallel im Abstand von 30 m — außerhalb, aber in derselben Bbox-Höhe.
        let achse = LineString::new(vec![
            Coord { x: 9.0 + 30.0 / mx, y: 48.0 },
            Coord { x: 9.0 + 30.0 / mx, y: 48.0 + 200.0 / my },
        ]);
        let f = feature(flags::ROAD, 65, Geometry::LineString(achse));
        let index = SpatialIndex::build(vec![f]);
        assert!(index.context_for(&fs).matches.is_empty());
    }

    #[test]
    fn ohne_flaeche_ist_die_deckung_null_statt_unendlich() {
        let ctx = OsmContext::default();
        assert_eq!(ctx.line_coverage(flags::ROAD), 0.0);
    }
}
