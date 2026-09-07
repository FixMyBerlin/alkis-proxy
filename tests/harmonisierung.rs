//! End-to-End-Nachweis der Harmonisierung.
//!
//! Führt echte Antworten aller vier Quellschemata durch Parser und Adapter und
//! prüft, dass daraus ein einheitliches Zielschema entsteht. Die Fixtures unter
//! `tests/fixtures/` stammen aus GetFeature-Aufrufen gegen die produktiven
//! Landesdienste.

use alkis_proxy::adapters;
use alkis_proxy::config::{SchemaVariant, StateKey};
use alkis_proxy::model::Parcel;
use alkis_proxy::parse::{parse_geojson, parse_gml};

fn from_gml(xml: &str, schema: SchemaVariant, state: StateKey) -> Vec<Parcel> {
    parse_gml(xml)
        .expect("GML parsebar")
        .iter()
        .map(|raw| adapters::map(schema, raw, state).expect("Mapping gelingt"))
        .collect()
}

fn from_json(body: &str, schema: SchemaVariant, state: StateKey) -> Vec<Parcel> {
    parse_geojson(body)
        .expect("GeoJSON parsebar")
        .iter()
        .map(|raw| adapters::map(schema, raw, state).expect("Mapping gelingt"))
        .collect()
}

fn alle() -> Vec<(&'static str, Vec<Parcel>)> {
    vec![
        (
            "NW (AVE, GML, Default-Namespace)",
            from_gml(
                include_str!("fixtures/ave_nw.gml"),
                SchemaVariant::Ave,
                StateKey::Nw,
            ),
        ),
        (
            "TH (AVE, GML, Präfix, reduzierte Felder)",
            from_gml(
                include_str!("fixtures/ave_th.gml"),
                SchemaVariant::Ave,
                StateKey::Th,
            ),
        ),
        (
            "SH (INSPIRE, GML)",
            from_gml(
                include_str!("fixtures/inspire_sh.gml"),
                SchemaVariant::Inspire,
                StateKey::Sh,
            ),
        ),
        (
            "BE (eigenes Schema, GeoJSON)",
            from_json(
                include_str!("fixtures/berlin.json"),
                SchemaVariant::Berlin,
                StateKey::Be,
            ),
        ),
        (
            "BW (nora, GeoJSON, Polygon)",
            from_json(
                include_str!("fixtures/bw_nora.json"),
                SchemaVariant::BwNora,
                StateKey::Bw,
            ),
        ),
    ]
}

#[test]
fn alle_vier_schemata_liefern_dasselbe_zielschema() {
    for (label, parcels) in alle() {
        assert!(!parcels.is_empty(), "{label}: keine Flurstücke");

        for p in &parcels {
            let f = p.to_feature();
            let props = &f["properties"];

            // Struktur ist überall identisch — auch dort, wo die Quelle
            // die Information gar nicht kennt (dann null, nie fehlend).
            for key in [
                "parcelId",
                "parcelIdSource",
                "bundesland",
                "gemarkungSchluessel",
                "gemarkungName",
                "flur",
                "zaehler",
                "nenner",
                "flurstuecksnummer",
                "flaecheQm",
                "gemeindeSchluessel",
                "gemeindeName",
                "kreisName",
                "lagebezeichnung",
                "nutzung",
                "stand",
            ] {
                assert!(
                    props.get(key).is_some(),
                    "{label}: Property {key} fehlt im Ausgabeschema"
                );
            }

            // Pflichtangaben sind überall belegt.
            assert_eq!(f["type"], "Feature", "{label}");
            assert_eq!(f["geometry"]["type"], "MultiPolygon", "{label}");
            assert_eq!(
                props["parcelId"].as_str().map(str::len),
                Some(20),
                "{label}: Kennzeichen nicht kanonisch"
            );
            assert!(
                props["gemarkungSchluessel"].as_str().map(str::len) == Some(6),
                "{label}: Gemarkungsschlüssel nicht sechsstellig"
            );
            assert!(!props["zaehler"].as_str().unwrap().is_empty(), "{label}");
        }
    }
}

#[test]
fn kennzeichen_traegt_das_richtige_bundesland() {
    // Der Länderschlüssel steckt in den ersten beiden Stellen des Kennzeichens
    // und muss zu dem Land passen, dessen Dienst geantwortet hat.
    for (label, parcels) in alle() {
        for p in &parcels {
            assert_eq!(
                p.parcel_id.bundesland(),
                Some(p.bundesland),
                "{label}: Kennzeichen {} passt nicht zu {}",
                p.parcel_id.canonical(),
                p.bundesland
            );
        }
    }
}

#[test]
fn feature_ids_sind_bundesweit_eindeutig() {
    let mut seen = std::collections::HashSet::new();
    for (label, parcels) in alle() {
        for p in &parcels {
            assert!(
                seen.insert(p.feature_id()),
                "{label}: doppelte Feature-ID {}",
                p.feature_id()
            );
        }
    }
    assert_eq!(seen.len(), 10, "erwartet 2 Flurstücke aus 5 Ländern");
}

#[test]
fn geometrien_sind_geschlossene_ringe_mit_flaeche() {
    for (label, parcels) in alle() {
        for p in &parcels {
            assert!(!p.geometry.polygons.is_empty(), "{label}: keine Polygone");
            for poly in &p.geometry.polygons {
                let outer = &poly[0];
                assert!(outer.len() >= 4, "{label}: Ring zu kurz");
                assert_eq!(outer.first(), outer.last(), "{label}: Ring offen");
            }
        }
    }
}

#[test]
fn thueringen_funktioniert_trotz_fehlender_felder() {
    // TH liefert weder gemaschl noch flstnrzae — die strukturellen Angaben
    // müssen dennoch vollständig sein, weil sie aus dem Kennzeichen kommen.
    let ps = from_gml(
        include_str!("fixtures/ave_th.gml"),
        SchemaVariant::Ave,
        StateKey::Th,
    );
    let p = &ps[0];
    assert_eq!(p.bundesland, "TH");
    assert_eq!(p.parcel_id.gemarkung_schluessel().len(), 6);
    assert_eq!(p.parcel_id.flur().as_deref(), Some("126"));
    assert!(!p.parcel_id.zaehler().is_empty());
}

#[test]
fn inspire_rekonstruiert_die_fehlenden_felder() {
    // SH liefert nur Kennzeichen, Label, Fläche, Geometrie. Gemarkung, Flur,
    // Zähler und Nenner dürfen trotzdem nicht leer sein.
    let ps = from_gml(
        include_str!("fixtures/inspire_sh.gml"),
        SchemaVariant::Inspire,
        StateKey::Sh,
    );
    let p = &ps[0];
    assert_eq!(p.bundesland, "SH");
    assert_ne!(p.parcel_id.gemarkung(), "____");
    assert!(!p.parcel_id.zaehler().is_empty());
    // Klarnamen kennt INSPIRE nicht — das ist erwartet, kein Fehler.
    assert!(p.gemarkung_name.is_none());
    assert!(p.flaeche_qm.is_some(), "areaValue sollte vorhanden sein");
}

#[test]
fn berlin_setzt_keine_lagebezeichnung_aus_dem_objekttyp() {
    // Regression: bezeich enthält "AX_Flurstueck" und darf nicht als Lage
    // durchgereicht werden.
    let ps = from_json(
        include_str!("fixtures/berlin.json"),
        SchemaVariant::Berlin,
        StateKey::Be,
    );
    for p in &ps {
        assert!(
            p.lagebezeichnung.is_none(),
            "Berlin darf keine Lagebezeichnung setzen, hat aber {:?}",
            p.lagebezeichnung
        );
    }
}

#[test]
fn nordrhein_westfalen_mappt_die_klarnamen() {
    let ps = from_gml(
        include_str!("fixtures/ave_nw.gml"),
        SchemaVariant::Ave,
        StateKey::Nw,
    );
    let p = &ps[0];
    assert_eq!(p.gemarkung_name.as_deref(), Some("Köln"));
    assert_eq!(p.kreis_name.as_deref(), Some("Köln"));
    assert_eq!(p.gemeinde_schluessel.as_deref(), Some("05315000"));
    assert_eq!(p.stand.as_deref(), Some("2013-02-22"), "Z-Suffix entfernt");
    assert!(p.lagebezeichnung.as_deref().unwrap().contains("Marspfortengasse"));
    assert_eq!(p.flaeche_qm, Some(9.0));
    // Gegenprobe: der Gemarkungsschlüssel aus dem Kennzeichen deckt sich mit
    // dem, was NRW selbst als gemaschl liefert.
    assert_eq!(p.parcel_id.gemarkung_schluessel(), "054958");
}
