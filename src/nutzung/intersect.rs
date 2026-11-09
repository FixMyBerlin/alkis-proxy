//! Fall 2: Länder ohne Flurstücksbezug in der Nutzungsebene.
//!
//! Vier Länder (RP, ST, TH über `ave:Nutzung`; BW über
//! `nora:v_al_tatsaechliche_nutzung`) führen die tatsächliche Nutzung nur als
//! lose Flächenebene, ohne Kennzeichen zum Flurstück. Die Zuordnung läuft
//! deshalb über eine geometrische Verschneidung — im Prinzip wie
//! `classification::index`, aber mit einem entscheidenden Unterschied: Beide
//! Ebenen stammen aus derselben amtlichen ALKIS-Erhebung, es gibt also keinen
//! Versatz wie zwischen ALKIS und OpenStreetMap. Eine Kompaktheits- oder
//! Formheuristik wie bei der OSM-Verkehrsflächenerkennung ist daher
//! überflüssig; ein niedriger Flächenanteil reicht, um Rundungs-Splitter an
//! gemeinsamen Grenzen von echten Überlappungen zu trennen.

use geo::{Area, BooleanOps, BoundingRect, Intersects, Rect};
use geo_types::MultiPolygon;

use crate::config::SchemaVariant;
use crate::nutzung::catalog;
use crate::nutzung::geom::to_multipolygon;
use crate::parse::RawFeature;

/// Ab welchem Flächenanteil an der Flurstücksfläche eine Nutzungsfläche
/// mitzählt. Beide Ebenen sind amtlich deckungsgleich digitalisiert; der Wert
/// filtert nur Berührungs-Artefakte an gemeinsamen Grenzen (Rundung beim
/// Clipping), keine echte Nutzung.
const MIN_ANTEIL: f64 = 0.01;

struct Flaeche {
    rect: Rect<f64>,
    geom: MultiPolygon<f64>,
    labels: Vec<String>,
}

/// Räumlicher Nachschlag über die Nutzungsflächen einer Kachel.
pub struct IntersectIndex(Vec<Flaeche>);

impl IntersectIndex {
    pub fn build(raws: &[RawFeature], schema: SchemaVariant) -> Self {
        let flaechen = raws
            .iter()
            .filter_map(|raw| {
                let labels = labels_for(raw, schema);
                if labels.is_empty() {
                    return None;
                }
                let geom = to_multipolygon(raw.geometry.as_ref()?)?;
                let rect = geom.bounding_rect()?;
                Some(Flaeche { rect, geom, labels })
            })
            .collect();
        IntersectIndex(flaechen)
    }

    /// `nutzungALKIS` für das Flurstück mit dieser (nativen) Geometrie.
    pub fn lookup(&self, geometry: &crate::model::Geometry) -> Option<String> {
        let fs = to_multipolygon(geometry)?;
        let fs_area = fs.unsigned_area();
        if fs_area <= 0.0 {
            return None;
        }
        let fs_rect = fs.bounding_rect()?;

        let mut gefunden: Vec<String> = Vec::new();
        for flaeche in &self.0 {
            if !rects_overlap(&fs_rect, &flaeche.rect) {
                continue;
            }
            if !fs.intersects(&flaeche.geom) {
                continue;
            }
            let anteil = fs.intersection(&flaeche.geom).unsigned_area() / fs_area;
            if anteil < MIN_ANTEIL {
                continue;
            }
            for label in &flaeche.labels {
                if !gefunden.iter().any(|v| v == label) {
                    gefunden.push(label.clone());
                }
            }
        }
        if gefunden.is_empty() {
            None
        } else {
            Some(gefunden.join(";"))
        }
    }
}

fn rects_overlap(a: &Rect<f64>, b: &Rect<f64>) -> bool {
    a.min().x <= b.max().x && a.max().x >= b.min().x && a.min().y <= b.max().y && a.max().y >= b.min().y
}

/// Nutzungsart(en) eines einzelnen Rohfeatures der Nutzungsebene, auf die
/// kanonische Schreibweise gebracht (siehe [`catalog`]).
///
/// `weitere_nutzung_name` (Baden-Württemberg) trägt trotz des Namens **keine**
/// zweite Nutzungsart, sondern einen reinen Überlappungs-Marker — beobachtet
/// wurde ausschließlich der feste Wert `"Überlagernd"`
/// (`weitere_nutzung_id: 1000`), nie ein zweiter Kategoriename. Das Feld wird
/// deshalb bewusst nicht gelesen; eine echte zweite Nutzung an derselben
/// Stelle kommt stattdessen automatisch über ein zweites, überlappendes
/// `v_al_tatsaechliche_nutzung`-Polygon herein — die Verschneidung oben
/// findet beide.
fn labels_for(raw: &RawFeature, schema: SchemaVariant) -> Vec<String> {
    let feld = match schema {
        SchemaVariant::Ave => "nutzart",
        SchemaVariant::BwNora => "objektname",
        SchemaVariant::Inspire | SchemaVariant::Berlin => return Vec::new(),
    };
    raw.field(&[feld]).map(catalog::canonical).into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Geometry;

    fn quadrat(x0: f64, y0: f64, x1: f64, y1: f64) -> Geometry {
        Geometry::from_polygon(vec![vec![
            [x0, y0],
            [x1, y0],
            [x1, y1],
            [x0, y1],
            [x0, y0],
        ]])
    }

    fn nutzungsflaeche(x0: f64, y0: f64, x1: f64, y1: f64, nutzart: &str) -> RawFeature {
        let mut raw = RawFeature::default();
        raw.fields.insert("nutzart".into(), nutzart.into());
        raw.geometry = Some(quadrat(x0, y0, x1, y1));
        raw
    }

    #[test]
    fn volle_ueberdeckung_liefert_die_nutzung() {
        let raws = vec![nutzungsflaeche(0.0, 0.0, 10.0, 10.0, "Wald")];
        let index = IntersectIndex::build(&raws, SchemaVariant::Ave);
        let fs = quadrat(2.0, 2.0, 4.0, 4.0);
        assert_eq!(index.lookup(&fs).as_deref(), Some("Wald"));
    }

    #[test]
    fn zwei_ueberlappende_flaechen_werden_semikolongetrennt() {
        let raws = vec![
            nutzungsflaeche(0.0, 0.0, 5.0, 10.0, "Landwirtschaft"),
            nutzungsflaeche(5.0, 0.0, 10.0, 10.0, "Weg"),
        ];
        let index = IntersectIndex::build(&raws, SchemaVariant::Ave);
        let fs = quadrat(0.0, 0.0, 10.0, 10.0);
        let ergebnis = index.lookup(&fs).unwrap();
        assert!(ergebnis.contains("Landwirtschaft"));
        assert!(ergebnis.contains("Weg"));
        assert_eq!(ergebnis.split(';').count(), 2);
    }

    #[test]
    fn geringfuegige_beruehrung_zaehlt_nicht() {
        // Nutzungsfläche ragt nur 1 % der Flurstücksfläche hinein — Rauschen
        // an der gemeinsamen Grenze, keine echte Nutzung.
        let raws = vec![nutzungsflaeche(9.9, 0.0, 20.0, 10.0, "Straßenverkehr")];
        let index = IntersectIndex::build(&raws, SchemaVariant::Ave);
        let fs = quadrat(0.0, 0.0, 10.0, 10.0);
        assert!(index.lookup(&fs).is_none());
    }

    #[test]
    fn entfernte_flaeche_wird_nicht_gefunden() {
        let raws = vec![nutzungsflaeche(100.0, 100.0, 110.0, 110.0, "Gewässer")];
        let index = IntersectIndex::build(&raws, SchemaVariant::Ave);
        let fs = quadrat(0.0, 0.0, 10.0, 10.0);
        assert!(index.lookup(&fs).is_none());
    }

    #[test]
    fn bw_liest_objektname() {
        let mut raw = RawFeature::default();
        raw.fields.insert("objektname".into(), "Weg".into());
        raw.geometry = Some(quadrat(0.0, 0.0, 10.0, 10.0));
        let index = IntersectIndex::build(&[raw], SchemaVariant::BwNora);
        let fs = quadrat(2.0, 2.0, 4.0, 4.0);
        assert_eq!(index.lookup(&fs).as_deref(), Some("Weg"));
    }

    #[test]
    fn bw_weitere_nutzung_name_ist_kein_zweiter_wert() {
        // Regression: weitere_nutzung_name ist ein Überlappungs-Marker
        // ("Überlagernd"), keine zweite Nutzungsart — durfte nie mit
        // ";" angehängt werden.
        let mut raw = RawFeature::default();
        raw.fields.insert("objektname".into(), "Industrie- und Gewerbefläche".into());
        raw.fields.insert("weitere_nutzung_id".into(), "1000".into());
        raw.fields.insert("weitere_nutzung_name".into(), "Überlagernd".into());
        raw.geometry = Some(quadrat(0.0, 0.0, 10.0, 10.0));
        let index = IntersectIndex::build(&[raw], SchemaVariant::BwNora);
        let fs = quadrat(2.0, 2.0, 4.0, 4.0);
        assert_eq!(index.lookup(&fs).as_deref(), Some("Industrie- und Gewerbefläche"));
    }

    #[test]
    fn thueringer_schreibweise_wird_harmonisiert() {
        let raws = vec![nutzungsflaeche(0.0, 0.0, 10.0, 10.0, "Strassenverkehr")];
        let index = IntersectIndex::build(&raws, SchemaVariant::Ave);
        let fs = quadrat(2.0, 2.0, 4.0, 4.0);
        assert_eq!(index.lookup(&fs).as_deref(), Some("Straßenverkehr"));
    }

    #[test]
    fn ohne_treffer_bleibt_das_flurstueck_unbelegt() {
        let index = IntersectIndex::build(&[], SchemaVariant::Ave);
        let fs = quadrat(0.0, 0.0, 10.0, 10.0);
        assert!(index.lookup(&fs).is_none());
    }
}
