//! Optionale Nutzungsart-Schätzung für Flurstücke, ausschließlich aus
//! OpenStreetMap abgeleitet (nie aus ALKIS-Zusatzquellen).
//!
//! Aktiv nur, wenn `ALKIS_OSM_PBF_PATH` auf eine lokale `.osm.pbf`-Datei
//! zeigt (`config/settings.rs`). Die Datei wird beim Start einmalig indiziert
//! (`osm_extract` + `index`); jede Bbox-Anfrage klassifiziert danach live
//! gegen den residenten Index (`rules`) — es wird kein Klassifikationsergebnis
//! vorberechnet oder gecacht, nur die rohen OSM-Geometrien liegen resident vor.
//!
//! Straßen, Wege und Bahnstrecken liegen in OSM als Achsen vor, Flurstücke
//! sind Flächen. Übersetzt wird das über den Deckungsgrad (Schnittlänge ×
//! Nennbreite / Flurstücksfläche) statt über einen vorberechneten Puffer —
//! siehe `rules.rs`, Abschnitt „Verkehrsflächen".
//!
//! Referenz für Regel-Logik und Attribute:
//! `ist-dieses-flurstueck-oeffentlich` (separates Repo). Dessen
//! Offline-Batch-Pipeline (ganze PBF pro Lauf neu lesen, GeoPackage schreiben)
//! wird hier nicht übernommen — siehe `osm_extract.rs` für die
//! speicherbewusste Alternative.

pub mod geom;
pub mod index;
pub mod osm_extract;
pub mod rules;

use std::path::Path;
use std::time::Instant;

use geo_types::{Geometry, MultiPolygon};

pub use rules::{Category, Classification};

#[derive(Debug, thiserror::Error)]
pub enum ClassificationError {
    #[error("OSM-PBF konnte nicht gelesen werden: {0}")]
    Pbf(#[from] osmpbf::Error),
}

/// Ein relevantes OSM-Objekt: Geometrie (WGS84) + die Regel-Prädikate, die
/// seine Tags erfüllen, als Bitmaske statt als `HashMap<String, String>` —
/// bei Millionen residenten Features (deutschlandweite PBF) macht das den
/// Unterschied zwischen brauchbarem und untragbarem Speicherbedarf.
#[derive(Debug)]
pub struct OsmFeature {
    pub flags: u32,
    /// Nennbreite der Verkehrsfläche in Dezimetern, aus den Tags abgeleitet
    /// (`rules::scan_way_tags`). `0` für alles, was nicht als Achse ausgewertet
    /// wird — Gebäude, Landnutzungsflächen. Zwei Byte je Feature; die
    /// Alternative wäre, die Breite pro Anfrage erneut aus den Tags zu
    /// bestimmen, die dafür resident bleiben müssten.
    pub width_dm: u16,
    pub geom: Geometry<f64>,
}

/// Resident gehaltener Klassifikations-Index: OSM-Features + Regelsatz.
pub struct ClassificationIndex {
    spatial: index::SpatialIndex,
    rules: Vec<Box<dyn rules::Rule>>,
}

impl ClassificationIndex {
    /// Liest und indiziert eine lokale OSM-PBF-Datei. Läuft typischerweise in
    /// `spawn_blocking` — bei einer deutschlandweiten Datei im
    /// Minutenbereich, siehe Logging in `osm_extract`.
    pub fn build(path: &Path) -> Result<Self, ClassificationError> {
        let started = Instant::now();
        let features = osm_extract::extract_features(path)?;
        let count = features.len();
        let spatial = index::SpatialIndex::build(features);
        tracing::info!(
            features = count,
            dauer_s = started.elapsed().as_secs(),
            "Nutzungsart-Index bereit"
        );
        Ok(ClassificationIndex {
            spatial,
            rules: rules::default_rules(),
        })
    }

    /// Ein Index ohne OSM-Features, mit dem vollständigen Regelsatz. Für
    /// Tests, die nur die Belegung der vier Properties prüfen und dafür keine
    /// PBF-Datei einlesen sollen.
    #[cfg(test)]
    pub fn leer_fuer_tests() -> Self {
        ClassificationIndex {
            spatial: index::SpatialIndex::build(Vec::new()),
            rules: rules::default_rules(),
        }
    }

    /// Wie viele OSM-Features im Index liegen — für Diagnose/Tests.
    pub fn len(&self) -> usize {
        self.spatial.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spatial.is_empty()
    }

    /// Schätzt die Nutzungsart eines Flurstücks anhand der schneidenden
    /// OSM-Features.
    pub fn classify(&self, geom: &MultiPolygon<f64>) -> Classification {
        let ctx = self.spatial.context_for(geom);
        rules::classify(&ctx, &self.rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ohne_treffer_ist_das_ergebnis_unbekannt() {
        let index = ClassificationIndex {
            spatial: index::SpatialIndex::build(vec![]),
            rules: rules::default_rules(),
        };
        let leer = MultiPolygon::<f64>::new(vec![]);
        let result = index.classify(&leer);
        assert_eq!(result.category, Category::Unknown);
    }
}
