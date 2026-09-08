//! Zwei-Pass-Extraktion relevanter OSM-Ways aus einer lokalen PBF-Datei.
//!
//! Anders als im Referenz-Repo (`ist-dieses-flurstueck-oeffentlich/src/osm.rs`,
//! dort bbox-gefiltert und pro Anfrage neu gelesen) wird hier die komplette
//! Datei **einmalig** beim Start indiziert. Damit das bei einer
//! deutschlandweiten PBF nicht in zweistellige GB Speicher läuft, wird nicht
//! nach Tag-*Keys* gefiltert (das lässt >90 % aller Ways durch, siehe
//! Plan-Notizen), sondern nach Tag-*Werten*: [`scan_way_tags`] ist der
//! geschlossene Satz, den `rules.rs` tatsächlich abfragt. Nur Ways, für die
//! das Filter `Some` liefert, werden behalten — und von denen auch nur die
//! Knotenkoordinaten, die ihre Ringe tatsächlich brauchen.
//!
//! Seit der Verkehrsflächen-Erkennung fällt auch das Straßen- und Wegenetz
//! unter diesen Satz. Das ist der mit Abstand größte Posten: Für
//! Baden-Württemberg kommen rund zwei Millionen Ways mit etwa 15 Millionen
//! Koordinaten hinzu (grob 400 MB resident), deutschlandweit das Sechs- bis
//! Siebenfache. Wer nur die übrigen Regeln braucht, filtert die Datei vorab
//! mit `osmium tags-filter` (siehe README).

use std::path::Path;
use std::time::Instant;

use geo_types::{Coord, Geometry, LineString, Point, Polygon};
use osmpbf::{Element, ElementReader};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::classification::rules::scan_way_tags;
use crate::classification::{ClassificationError, OsmFeature};

/// Wie oft während eines Durchlaufs ein Fortschritts-Log geschrieben wird —
/// bei einer deutschlandweiten, ungefilterten PBF (hunderte Millionen
/// Elemente) sonst die einzige Spur, dass der Prozess noch lebt statt hängt.
const PROGRESS_EVERY: u64 = 20_000_000;

struct WayCandidate {
    flags: u32,
    width_dm: u16,
    flaeche: bool,
    refs: Vec<i64>,
}

/// Extrahiert alle relevanten Ways aus der PBF-Datei unter `path`.
pub fn extract_features(path: &Path) -> Result<Vec<OsmFeature>, ClassificationError> {
    let started = Instant::now();

    // --- Pass 1: relevante Ways + die Menge ihrer referenzierten Node-IDs ---
    let mut candidates: Vec<WayCandidate> = Vec::new();
    let mut needed_nodes: FxHashSet<i64> = FxHashSet::default();
    let mut ways_scanned: u64 = 0;

    let reader = ElementReader::from_path(path)?;
    reader.for_each(|element| {
        let Element::Way(way) = element else {
            return;
        };
        ways_scanned += 1;
        if ways_scanned.is_multiple_of(PROGRESS_EVERY) {
            tracing::info!(
                ways_scanned,
                relevant = candidates.len(),
                elapsed_s = started.elapsed().as_secs(),
                "Nutzungsart-Index: Pass 1 (Ways) läuft"
            );
        }

        let Some(tags) = scan_way_tags(way.tags()) else {
            return;
        };

        let refs: Vec<i64> = way.refs().collect();
        needed_nodes.extend(refs.iter().copied());
        candidates.push(WayCandidate {
            flags: tags.flags,
            width_dm: tags.width_dm,
            flaeche: tags.flaeche,
            refs,
        });
    })?;

    tracing::info!(
        ways_scanned,
        relevant_ways = candidates.len(),
        referenzierte_knoten = needed_nodes.len(),
        dauer_s = started.elapsed().as_secs(),
        "Nutzungsart-Index: Pass 1 (Ways) abgeschlossen"
    );

    // --- Pass 2: Koordinaten nur für tatsächlich benötigte Knoten ---
    let mut nodes: FxHashMap<i64, (f64, f64)> = FxHashMap::default();
    nodes.reserve(needed_nodes.len());
    let mut nodes_scanned: u64 = 0;

    let reader = ElementReader::from_path(path)?;
    reader.for_each(|element| {
        let (id, lon, lat) = match element {
            Element::Node(n) => (n.id(), n.lon(), n.lat()),
            Element::DenseNode(n) => (n.id(), n.lon(), n.lat()),
            _ => return,
        };
        nodes_scanned += 1;
        if nodes_scanned.is_multiple_of(PROGRESS_EVERY) {
            tracing::info!(
                nodes_scanned,
                aufgeloest = nodes.len(),
                elapsed_s = started.elapsed().as_secs(),
                "Nutzungsart-Index: Pass 2 (Knoten) läuft"
            );
        }
        if needed_nodes.contains(&id) {
            nodes.insert(id, (lon, lat));
        }
    })?;

    let aufgeloest = needed_nodes.iter().filter(|id| nodes.contains_key(id)).count();
    tracing::info!(
        nodes_scanned,
        benoetigt = needed_nodes.len(),
        aufgeloest,
        fehlend = needed_nodes.len() - aufgeloest,
        dauer_s = started.elapsed().as_secs(),
        "Nutzungsart-Index: Pass 2 (Knoten) abgeschlossen"
    );
    // Ab hier nicht mehr gebraucht — Speicher früh freigeben, bevor die
    // Geometrien (die nächste große Allokation) gebaut werden.
    drop(needed_nodes);

    // --- Geometrien zusammenbauen ---
    let mut features = Vec::with_capacity(candidates.len());
    let mut verworfen = 0usize;
    for candidate in candidates {
        let mut coords: Vec<Coord<f64>> = Vec::with_capacity(candidate.refs.len());
        let mut complete = true;
        for node_id in &candidate.refs {
            match nodes.get(node_id) {
                Some(&(lon, lat)) => coords.push(Coord { x: lon, y: lat }),
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if !complete || coords.len() < 2 {
            verworfen += 1;
            continue;
        }
        features.push(OsmFeature {
            flags: candidate.flags,
            width_dm: candidate.width_dm,
            geom: build_geometry(coords, candidate.width_dm > 0 && !candidate.flaeche),
        });
    }

    tracing::info!(
        features = features.len(),
        verworfen,
        grund = "unvollstaendige Knotenliste",
        dauer_gesamt_s = started.elapsed().as_secs(),
        "Nutzungsart-Index: Geometrien gebaut"
    );

    Ok(features)
}

/// Baut aus einer Koordinatenkette eine passende Geometrie: geschlossene
/// Kette (>=4 Punkte) -> Polygon, ein Punkt -> Point, sonst LineString.
///
/// `ist_achse` erzwingt einen LineString auch bei geschlossener Kette. Ohne
/// das würden Kreisverkehre und Wendeschleifen zu Polygonen und damit an der
/// Deckungsgrad-Regel vorbeilaufen, die für sie zuständig ist.
fn build_geometry(coords: Vec<Coord<f64>>, ist_achse: bool) -> Geometry<f64> {
    let closed = !ist_achse && coords.len() >= 4 && coords.first() == coords.last();
    if closed {
        Geometry::Polygon(Polygon::new(LineString::new(coords), vec![]))
    } else if coords.len() == 1 {
        Geometry::Point(Point::from(coords[0]))
    } else {
        Geometry::LineString(LineString::new(coords))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geschlossene_kette_wird_zu_polygon() {
        let coords = vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 1.0, y: 0.0 },
            Coord { x: 1.0, y: 1.0 },
            Coord { x: 0.0, y: 0.0 },
        ];
        assert!(matches!(build_geometry(coords, false), Geometry::Polygon(_)));
    }

    #[test]
    fn offene_kette_wird_zu_linestring() {
        let coords = vec![Coord { x: 0.0, y: 0.0 }, Coord { x: 1.0, y: 1.0 }];
        assert!(matches!(build_geometry(coords, false), Geometry::LineString(_)));
    }

    #[test]
    fn einzelner_punkt_wird_zu_point() {
        let coords = vec![Coord { x: 0.0, y: 0.0 }];
        assert!(matches!(build_geometry(coords, false), Geometry::Point(_)));
    }

    #[test]
    fn geschlossene_achse_bleibt_ein_linestring() {
        // Kreisverkehr: geschlossener Way, aber keine Fläche.
        let coords = vec![
            Coord { x: 0.0, y: 0.0 },
            Coord { x: 1.0, y: 0.0 },
            Coord { x: 1.0, y: 1.0 },
            Coord { x: 0.0, y: 0.0 },
        ];
        assert!(matches!(build_geometry(coords, true), Geometry::LineString(_)));
    }

    #[test]
    fn fehlende_testdatei_liefert_einen_fehler_keine_panik() {
        let result = extract_features(Path::new("/nicht/vorhanden.osm.pbf"));
        assert!(result.is_err());
    }
}
