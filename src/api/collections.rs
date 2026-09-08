//! Beschreibende Endpunkte nach OGC API Features Teil 1.
//!
//! Clients wie QGIS erkunden den Dienst über diese Endpunkte: Landing Page →
//! `/conformance` → `/collections` → `items`. Sie folgen dabei den `links`
//! wörtlich, weshalb dort absolute URLs stehen müssen.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Value};

use crate::api::{base_url, SharedState, TypedJson, SCHEMA_JSON};
use crate::config::states;

pub async fn landing(State(state): State<SharedState>, headers: HeaderMap) -> Json<Value> {
    let base = base_url(&state, &headers);
    Json(json!({
        "title": "ALKIS-Proxy",
        "description": "Flurstücke aller deutschen Bundesländer über eine einheitliche Schnittstelle.",
        "links": [
            { "rel": "self", "type": "application/json", "title": "Diese Seite", "href": base.clone() },
            { "rel": "root", "type": "application/json", "title": "Startseite", "href": base.clone() },
            { "rel": "conformance", "type": "application/json", "title": "Konformitätsklassen", "href": format!("{base}/conformance") },
            { "rel": "data", "type": "application/json", "title": "Sammlungen", "href": format!("{base}/collections") },
            // `service-desc` muss auf die API-Definition zeigen, nicht auf die
            // Sammlung (Requirement 2). QGIS wählt den Link zwar auch bei
            // falschem Medientyp aus, liest daraus dann aber keine Seitengröße
            // und lädt nur 100 Objekte je Layer.
            { "rel": "service-desc", "type": super::OPENAPI, "title": "API-Definition", "href": format!("{base}/api") }
        ]
    }))
}

/// Erfüllte Konformitätsklassen. QGIS und GDAL prüfen diese Liste, bevor sie
/// den Dienst als OGC API Features behandeln.
pub async fn conformance() -> Json<Value> {
    Json(json!({
        "conformsTo": [
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/core",
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/oas30",
            "http://www.opengis.net/spec/ogcapi-features-1/1.0/conf/geojson",
            "http://www.opengis.net/spec/ogcapi-features-3/1.0/conf/queryables"
        ]
    }))
}

/// Feldbeschreibung der Sammlung (OGC API Features Teil 3, `conf/queryables`).
///
/// Anders als der Name nahelegt, holt QGIS diesen Endpunkt *nicht* ab, um die
/// Attributtabelle zu füllen: `QgsOapifProvider` fragt ihn nur an, wenn der
/// Dienst CQL2-Text-Filterung deklariert — das tut dieser Dienst nicht. Die
/// Felder leitet QGIS stattdessen aus dem Schema-Sample von `items` ab. Der
/// Endpunkt bleibt dennoch, weil andere Clients ihn auswerten und die
/// deklarierte Konformitätsklasse ihn verlangt.
pub async fn queryables(State(state): State<SharedState>, headers: HeaderMap) -> TypedJson {
    let base = base_url(&state, &headers);
    let text = |title: &str| json!({ "type": "string", "title": title });
    TypedJson(SCHEMA_JSON, json!({
        "$schema": "https://json-schema.org/draft/2019-09/schema",
        "$id": format!("{base}/collections/flurstuecke/queryables"),
        "type": "object",
        "title": "Flurstücke (ALKIS)",
        "properties": {
            "parcelId": text("Flurstückskennzeichen (kanonisch, 20 Zeichen)"),
            "parcelIdSource": text("Quellattribut des Kennzeichens"),
            "bundesland": text("Bundesland-Kürzel"),
            "gemarkungSchluessel": text("Gemarkungsschlüssel (Land + Gemarkung)"),
            "gemarkungName": text("Gemarkung"),
            "flur": text("Flur (leer, wo das Land keine Fluren führt)"),
            "zaehler": text("Zähler der Flurstücksnummer"),
            "nenner": text("Nenner der Flurstücksnummer"),
            "flurstuecksnummer": text("Flurstücksnummer"),
            "flaecheQm": { "type": "number", "title": "Amtliche Fläche in m²" },
            "gemeindeSchluessel": text("Gemeindeschlüssel"),
            "gemeindeName": text("Gemeinde"),
            "kreisName": text("Kreis"),
            "lagebezeichnung": text("Lagebezeichnung"),
            "nutzung": text("Tatsächliche Nutzung"),
            "stand": text("Stand der Daten (ISO-Datum)")
        }
    }))
}

pub async fn list(State(state): State<SharedState>, headers: HeaderMap) -> Json<Value> {
    let base = base_url(&state, &headers);
    Json(json!({
        "collections": [collection_doc(&state, &base)],
        "links": [
            { "rel": "self", "type": "application/json", "title": "Sammlungen", "href": format!("{base}/collections") },
            { "rel": "root", "type": "application/json", "title": "Startseite", "href": base.clone() }
        ]
    }))
}

pub async fn describe(State(state): State<SharedState>, headers: HeaderMap) -> Json<Value> {
    let base = base_url(&state, &headers);
    Json(collection_doc(&state, &base))
}

/// Beschreibung der Sammlung inklusive Quellenangaben.
///
/// Die Attribution wird mitgeliefert, weil ein Teil der Länder unter DL-DE BY
/// 2.0 oder CC BY 4.0 steht und damit Namensnennung verlangt. Clients können
/// die Liste unmittelbar für den Kartenhinweis verwenden.
fn collection_doc(state: &SharedState, base: &str) -> Value {
    let sources: Vec<Value> = states::all()
        .iter()
        .filter(|s| s.endpoint.is_some())
        .map(|s| {
            let attribution = s.attribution.map(|a| {
                json!({
                    "text": a.text,
                    "url": a.url,
                    "license": a.license.label(),
                    "licenseUrl": a.license.url(),
                    "attributionRequired": a.license.requires_attribution(),
                })
            });
            json!({ "state": s.key.code(), "label": s.label, "attribution": attribution })
        })
        .collect();

    json!({
        "id": "flurstuecke",
        "title": "Flurstücke (ALKIS)",
        "description": "Flurstücke aus den Liegenschaftskatastern der Bundesländer, \
                        vereinheitlicht auf ein gemeinsames Attributschema. \
                        Anfragen benötigen einen Kartenausschnitt (bbox), \
                        maximal etwa 8 km Kantenlänge.",
        "itemType": "feature",
        "crs": ["http://www.opengis.net/def/crs/OGC/1.3/CRS84"],
        "storageCrs": "http://www.opengis.net/def/crs/OGC/1.3/CRS84",
        "extent": {
            "spatial": {
                "bbox": [[5.87, 47.27, 15.04, 55.06]],
                "crs": "http://www.opengis.net/def/crs/OGC/1.3/CRS84"
            }
        },
        "limits": { "default": state.settings.default_limit, "maximum": state.settings.max_limit },
        "sources": sources,
        "links": [
            { "rel": "self", "type": "application/json", "title": "Beschreibung", "href": format!("{base}/collections/flurstuecke") },
            { "rel": "root", "type": "application/json", "title": "Startseite", "href": base.to_string() },
            { "rel": "items", "type": "application/geo+json", "title": "Flurstücke", "href": format!("{base}/collections/flurstuecke/items") }
        ]
    })
}
