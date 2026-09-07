//! GML-Parsing für WFS-Antworten.
//!
//! Bewusst *ein* generischer Parser für alle Landesschemata statt vier
//! spezialisierter: Er zerlegt eine `FeatureCollection` in [`RawFeature`]s aus
//! flachen Feldern plus Geometrie. Die Abbildung auf das Zielschema übernehmen
//! danach die Adapter in [`crate::adapters`].
//!
//! Zwei Eigenheiten der realen Dienste bestimmen das Design:
//!
//! * **Namespaces sind nicht verlässlich.** NRW liefert die Fachfelder im
//!   Default-Namespace (`<flstkennz>`), Thüringen mit Präfix (`<ave:flstkennz>`).
//!   Der Parser vergleicht deshalb ausschließlich lokale Namen.
//! * **Nicht jede Geometrie im Feature ist die Fläche.** INSPIRE-Features tragen
//!   neben `geometry` noch einen `referencePoint`. Nur Elemente aus
//!   [`GEOMETRY_HOLDERS`] werden als Flächengeometrie gelesen, der Punkt wird
//!   ignoriert.
//!
//! Die gelieferten Koordinaten bleiben im CRS der Quelle; die Umrechnung nach
//! EPSG:4326 passiert später in [`crate::crs`].

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::model::parcel::{PolygonRings, Ring};
use crate::model::Geometry;

/// Elemente, deren Inhalt als Flächengeometrie des Features gilt.
const GEOMETRY_HOLDERS: &[&str] = &["geometrie", "geometry", "geom", "position"];

/// GML-Geometrietypen, die auch ohne umschließendes Fachelement als Fläche
/// gelten. Der Saarland-Dienst (ArcGIS) hängt `gml:MultiSurface` direkt an das
/// Feature, ohne Wrapper.
const GEOMETRY_TYPES: &[&str] = &["MultiSurface", "Surface", "Polygon", "MultiPolygon"];

/// Kontexte, in denen eine Geometrie *nicht* die Fläche des Flurstücks ist.
/// INSPIRE-Features tragen zusätzlich einen `referencePoint`.
const GEOMETRY_EXCLUDED: &[&str] = &["referencePoint", "boundedBy", "Envelope"];

/// Elemente, die eine Sammlung von Features einleiten.
const MEMBER_ELEMENTS: &[&str] = &["member", "featureMember", "featureMembers"];

#[derive(Debug, thiserror::Error)]
pub enum GmlError {
    #[error("XML konnte nicht gelesen werden: {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("Der Dienst meldet eine OGC-Exception: {0}")]
    OgcException(String),
    #[error("Koordinate {value:?} ist keine Zahl")]
    BadCoordinate { value: String },
    #[error("Ring hat {count} Koordinatenwerte, was bei {dim} Dimensionen nicht aufgeht")]
    RaggedPosList { count: usize, dim: usize },
}

/// Ein Feature, wie es aus der Antwort fällt: flache Textfelder plus Geometrie.
#[derive(Debug, Default, Clone)]
pub struct RawFeature {
    /// Lokaler Elementname (kleingeschrieben) → Textinhalt. Bei mehrfachem
    /// Vorkommen gewinnt das erste, damit ein tiefer verschachteltes Element ein
    /// gleichnamiges Feld auf Feature-Ebene nicht überschreibt.
    ///
    /// Kleingeschrieben, weil die Dienste dieselben Felder unterschiedlich
    /// schreiben: Saarland liefert `FLSTKENNZ`, alle anderen `flstkennz`.
    pub fields: HashMap<String, String>,
    pub geometry: Option<Geometry>,
}

impl RawFeature {
    /// Erster belegter Wert unter den angegebenen Feldnamen.
    ///
    /// Die Landesschemata benennen dieselbe Information unterschiedlich; die
    /// Adapter geben deshalb eine Kandidatenliste an. Die Namen müssen
    /// kleingeschrieben sein — der Vergleich erfolgt ohne weitere Normalisierung,
    /// damit die Suche ein reiner Hash-Lookup bleibt.
    pub fn field(&self, names: &[&str]) -> Option<&str> {
        debug_assert!(
            names.iter().all(|n| n.chars().all(|c| !c.is_uppercase())),
            "Feldnamen müssen kleingeschrieben sein: {names:?}"
        );
        names
            .iter()
            .find_map(|n| self.fields.get(*n))
            .map(|s| s.as_str())
            .filter(|s| !s.is_empty())
    }

    /// Wie [`field`](Self::field), aber als Zahl geparst.
    pub fn number(&self, names: &[&str]) -> Option<f64> {
        self.field(names)?.replace(',', ".").parse().ok()
    }
}

/// Zerlegt eine GML-`FeatureCollection` in ihre Features.
pub fn parse_gml(xml: &str) -> Result<Vec<RawFeature>, GmlError> {
    // Fehlerantworten kommen als valides XML mit HTTP 200 — vor dem eigentlichen
    // Parsen abfangen, sonst liefern wir stillschweigend null Features.
    if let Some(msg) = extract_exception(xml) {
        return Err(GmlError::OgcException(msg));
    }

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut features: Vec<RawFeature> = Vec::new();
    let mut state = State::default();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                let name = local_name(e.name().as_ref());
                let dim = srs_dimension(&e);
                state.on_start(&name, dim);
            }
            Event::Text(t) => {
                let text = t.unescape().unwrap_or_default().trim().to_string();
                if !text.is_empty() {
                    state.on_text(&text)?;
                }
            }
            Event::End(e) => {
                let name = local_name(e.name().as_ref());
                state.on_end(&name, &mut features);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(features)
}

/// Wo im Dokument wir gerade sind.
#[derive(Default)]
struct State {
    /// Tiefe innerhalb des aktuellen Features; 0 = außerhalb.
    depth: usize,
    in_member: bool,
    /// Das gerade aufgebaute Feature.
    current: Option<RawFeature>,
    /// Name des zuletzt geöffneten einfachen Elements (Kandidat für ein Feld).
    pending_field: Option<String>,

    // --- Geometrie ---
    /// Tiefe, bei der der Geometriebereich begann; `None` = außerhalb.
    geometry_depth: Option<usize>,
    /// Fertige Polygone des aktuellen Features.
    polygons: Vec<PolygonRings>,
    /// Ringe des gerade offenen Polygons (Index 0 = äußerer Ring).
    rings: PolygonRings,
    /// Ob wir in einem `gml:interior` sind (Loch statt Außenring).
    in_interior: bool,
    /// Dimensionalität der aktuellen posList.
    srs_dim: usize,
    /// Ob der nächste Text eine posList ist.
    expect_poslist: bool,
    /// Tiefe, ab der Geometrien ignoriert werden (z. B. innerhalb von
    /// `referencePoint`); `None` = kein Ausschluss aktiv.
    excluded_depth: Option<usize>,
}

impl State {
    fn in_geometry(&self) -> bool {
        self.geometry_depth.is_some()
    }

    fn on_start(&mut self, name: &str, dim: Option<usize>) {
        if let Some(d) = dim {
            self.srs_dim = d;
        }

        if !self.in_member {
            if MEMBER_ELEMENTS.contains(&name) {
                self.in_member = true;
            }
            return;
        }

        // Erstes Element innerhalb von member: der Feature-Rumpf.
        if self.current.is_none() {
            self.current = Some(RawFeature::default());
            self.depth = 0;
            self.polygons.clear();
            self.rings.clear();
            self.geometry_depth = None;
            self.excluded_depth = None;
            self.srs_dim = 2;
            return;
        }

        self.depth += 1;

        if self.in_geometry() {
            match name {
                "Polygon" => self.rings.clear(),
                "interior" => self.in_interior = true,
                "exterior" => self.in_interior = false,
                "posList" => self.expect_poslist = true,
                _ => {}
            }
        } else if GEOMETRY_EXCLUDED.contains(&name) {
            self.excluded_depth.get_or_insert(self.depth);
        } else if self.excluded_depth.is_none()
            && (GEOMETRY_HOLDERS.contains(&name) || GEOMETRY_TYPES.contains(&name))
        {
            self.geometry_depth = Some(self.depth);
            if name == "Polygon" {
                self.rings.clear();
            }
        } else {
            self.pending_field = Some(name.to_string());
        }
    }

    fn on_text(&mut self, text: &str) -> Result<(), GmlError> {
        if self.expect_poslist {
            let ring = parse_pos_list(text, self.srs_dim)?;
            if !ring.is_empty() {
                // Außenring zuerst, Löcher dahinter — GML liefert exterior vor
                // interior, sodass die Reihenfolge des Anhängens bereits der
                // GeoJSON-Konvention entspricht.
                self.rings.push(ring);
            }
            self.expect_poslist = false;
            return Ok(());
        }

        if self.in_geometry() {
            return Ok(());
        }

        if let (Some(field), Some(feature)) = (self.pending_field.take(), self.current.as_mut()) {
            feature
                .fields
                .entry(field.to_ascii_lowercase())
                .or_insert_with(|| text.to_string());
        }
        Ok(())
    }

    fn on_end(&mut self, name: &str, features: &mut Vec<RawFeature>) {
        if !self.in_member {
            return;
        }

        if self.excluded_depth == Some(self.depth) {
            self.excluded_depth = None;
        }

        if self.in_geometry() {
            if name == "Polygon" && !self.rings.is_empty() {
                self.polygons.push(std::mem::take(&mut self.rings));
            }
            if self.geometry_depth == Some(self.depth) {
                self.geometry_depth = None;
            }
        }

        if MEMBER_ELEMENTS.contains(&name) {
            // Feature abschließen.
            if let Some(mut feature) = self.current.take() {
                // Ein Polygon ohne umschließendes MultiSurface wird hier eingesammelt.
                if !self.rings.is_empty() {
                    self.polygons.push(std::mem::take(&mut self.rings));
                }
                if !self.polygons.is_empty() {
                    feature.geometry =
                        Some(Geometry::multi_polygon(std::mem::take(&mut self.polygons)));
                }
                features.push(feature);
            }
            self.in_member = false;
            self.depth = 0;
            self.pending_field = None;
            return;
        }

        self.depth = self.depth.saturating_sub(1);
        self.pending_field = None;
    }
}

/// `srsDimension`-Attribut, sofern vorhanden.
fn srs_dimension(e: &quick_xml::events::BytesStart) -> Option<usize> {
    e.attributes().flatten().find_map(|a| {
        (local_name(a.key.as_ref()) == "srsDimension")
            .then(|| String::from_utf8_lossy(&a.value).parse().ok())
            .flatten()
    })
}

/// Lokaler Name ohne Namespace-Präfix.
fn local_name(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => s.to_string(),
    }
}

/// `"x1 y1 x2 y2 …"` → Ring aus `[x, y]`-Paaren.
///
/// Höhere Dimensionen (`srsDimension="3"`) werden gelesen, die Z-Komponente aber
/// verworfen — GeoJSON-Konsumenten erwarten hier durchweg 2D.
fn parse_pos_list(text: &str, dim: usize) -> Result<Ring, GmlError> {
    let dim = dim.max(2);
    let values: Vec<&str> = text.split_ascii_whitespace().collect();
    if values.is_empty() {
        return Ok(Vec::new());
    }
    if !values.len().is_multiple_of(dim) {
        return Err(GmlError::RaggedPosList {
            count: values.len(),
            dim,
        });
    }

    let mut ring = Ring::with_capacity(values.len() / dim);
    for chunk in values.chunks(dim) {
        let x = chunk[0]
            .parse::<f64>()
            .map_err(|_| GmlError::BadCoordinate {
                value: chunk[0].to_string(),
            })?;
        let y = chunk[1]
            .parse::<f64>()
            .map_err(|_| GmlError::BadCoordinate {
                value: chunk[1].to_string(),
            })?;
        ring.push([x, y]);
    }
    Ok(ring)
}

/// Sucht eine OGC-Fehlermeldung im Dokument.
fn extract_exception(xml: &str) -> Option<String> {
    if !xml.contains("ExceptionReport") {
        return None;
    }
    let start = xml.find("ExceptionText")?;
    let after = &xml[start..];
    let open = after.find('>')? + 1;
    let close = after[open..].find('<')? + open;
    let msg = after[open..close].trim();
    Some(if msg.is_empty() {
        "unbekannter Fehler".to_string()
    } else {
        msg.chars().take(300).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NW: &str = include_str!("../../tests/fixtures/ave_nw.gml");
    const TH: &str = include_str!("../../tests/fixtures/ave_th.gml");
    const SH: &str = include_str!("../../tests/fixtures/inspire_sh.gml");

    #[test]
    fn nrw_default_namespace_wird_gelesen() {
        let fs = parse_gml(NW).unwrap();
        assert_eq!(fs.len(), 2);
        let f = &fs[0];
        // NRW liefert die Fachfelder ohne Präfix.
        assert_eq!(f.field(&["flstkennz"]), Some("05495803101089______"));
        assert_eq!(f.field(&["gemarkung"]), Some("Köln"));
        assert_eq!(f.field(&["gemaschl"]), Some("054958"));
        assert_eq!(f.field(&["flur"]), Some("031"));
        assert_eq!(f.number(&["flaeche"]), Some(9.0));
        assert!(f.field(&["lagebeztxt"]).unwrap().contains("Marspfortengasse"));
    }

    #[test]
    fn thueringen_mit_namespace_praefix_wird_gelesen() {
        let fs = parse_gml(TH).unwrap();
        assert_eq!(fs.len(), 2);
        let f = &fs[0];
        // Trotz "ave:"-Präfix greift derselbe lokale Name.
        assert!(f.field(&["flstkennz"]).is_some());
        // Thüringen kennt gemaschl/flstnrzae nicht — darf kein Fehler sein.
        assert_eq!(f.field(&["gemaschl"]), None);
        assert_eq!(f.field(&["flstnrzae"]), None);
        assert!(f.field(&["flurstnr"]).is_some(), "TH nutzt flurstnr");
    }

    #[test]
    fn inspire_liest_flaeche_nicht_den_referenzpunkt() {
        let fs = parse_gml(SH).unwrap();
        assert_eq!(fs.len(), 2);
        let f = &fs[0];
        assert!(f.field(&["nationalcadastralreference"]).is_some());
        let g = f.geometry.as_ref().expect("Flächengeometrie");
        // Der referencePoint (gml:Point) darf nicht als Fläche durchgehen:
        // ein Ring braucht mehr als zwei Stützpunkte.
        assert_eq!(g.polygons.len(), 1);
        assert!(
            g.polygons[0][0].len() > 3,
            "Ring hat nur {} Punkte — vermutlich wurde der referencePoint gelesen",
            g.polygons[0][0].len()
        );
    }

    #[test]
    fn geometrie_wird_als_multipolygon_geliefert() {
        for (name, xml) in [("NW", NW), ("TH", TH), ("SH", SH)] {
            let fs = parse_gml(xml).unwrap();
            for f in &fs {
                let g = f.geometry.as_ref().unwrap_or_else(|| panic!("{name}: keine Geometrie"));
                assert!(!g.is_empty(), "{name}: leere Geometrie");
                for poly in &g.polygons {
                    let outer = &poly[0];
                    assert!(outer.len() >= 4, "{name}: Ring zu kurz");
                    assert_eq!(
                        outer.first(),
                        outer.last(),
                        "{name}: Ring ist nicht geschlossen"
                    );
                }
            }
        }
    }

    #[test]
    fn koordinaten_liegen_im_nativen_crs() {
        // NRW wurde in EPSG:25832 angefragt: Rechtswert ~356 km, Hochwert ~5645 km.
        let fs = parse_gml(NW).unwrap();
        let [x, y] = fs[0].geometry.as_ref().unwrap().polygons[0][0][0];
        assert!((350_000.0..360_000.0).contains(&x), "x={x}");
        assert!((5_640_000.0..5_650_000.0).contains(&y), "y={y}");
    }

    #[test]
    fn ogc_exception_wird_als_fehler_gemeldet() {
        let xml = r#"<?xml version="1.0"?>
            <ows:ExceptionReport xmlns:ows="http://www.opengis.net/ows/1.1">
              <ows:Exception exceptionCode="InvalidParameterValue">
                <ows:ExceptionText>Unknown typename foo</ows:ExceptionText>
              </ows:Exception>
            </ows:ExceptionReport>"#;
        let err = parse_gml(xml).unwrap_err();
        assert!(matches!(err, GmlError::OgcException(ref m) if m.contains("Unknown typename")));
    }

    #[test]
    fn poslist_parsing() {
        assert_eq!(
            parse_pos_list("1.0 2.0 3.0 4.0", 2).unwrap(),
            vec![[1.0, 2.0], [3.0, 4.0]]
        );
        // Dritte Dimension wird verworfen.
        assert_eq!(
            parse_pos_list("1.0 2.0 9.9 3.0 4.0 9.9", 3).unwrap(),
            vec![[1.0, 2.0], [3.0, 4.0]]
        );
        assert!(matches!(
            parse_pos_list("1.0 2.0 3.0", 2),
            Err(GmlError::RaggedPosList { .. })
        ));
        assert!(matches!(
            parse_pos_list("a b", 2),
            Err(GmlError::BadCoordinate { .. })
        ));
    }
}
