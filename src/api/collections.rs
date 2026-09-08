//! Beschreibende Endpunkte nach OGC API Features Teil 1.
//!
//! Clients wie QGIS erkunden den Dienst über diese Endpunkte: Landing Page →
//! `/conformance` → `/collections` → `items`. Sie folgen dabei den `links`
//! wörtlich, weshalb dort absolute URLs stehen müssen.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Value};

use crate::api::features::current_year;
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
/// Feldbeschreibung, die beide Collections teilen — `flurstuecke-nutzungsart`
/// ergänzt vier weitere Felder, siehe `queryables_nutzungsart`.
fn base_queryable_properties() -> Value {
    let text = |title: &str| json!({ "type": "string", "title": title });
    json!({
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
    })
}

pub async fn queryables(State(state): State<SharedState>, headers: HeaderMap) -> TypedJson {
    let base = base_url(&state, &headers);
    TypedJson(SCHEMA_JSON, json!({
        "$schema": "https://json-schema.org/draft/2019-09/schema",
        "$id": format!("{base}/collections/flurstuecke/queryables"),
        "type": "object",
        "title": "Flurstücke (ALKIS)",
        "properties": base_queryable_properties()
    }))
}

/// Wie `queryables`, zusätzlich die vier Nutzungsart-Felder — dieselben
/// Namen wie im Referenz-Repo `ist-dieses-flurstueck-oeffentlich`
/// (`category`/`rule`/`confidence`/`conflict`).
pub async fn queryables_nutzungsart(State(state): State<SharedState>, headers: HeaderMap) -> TypedJson {
    let base = base_url(&state, &headers);
    let text = |title: &str| json!({ "type": "string", "title": title });
    let mut properties = base_queryable_properties();
    let props = properties.as_object_mut().expect("base_queryable_properties liefert ein Objekt");
    props.insert(
        "category".into(),
        text("Geschätzte Nutzungsart — privat/oeffentlich/bahn/unbekannt. \
              Ausschließlich aus OpenStreetMap abgeleitet, keine amtliche Eigentumsauskunft."),
    );
    props.insert("rule".into(), text("Name der greifenden Klassifikationsregel"));
    props.insert(
        "confidence".into(),
        json!({ "type": "number", "title": "Konfidenz der Schätzung (0.0–1.0)" }),
    );
    props.insert(
        "conflict".into(),
        text("Abweichende Kategorie einer anderen greifenden Regel, falls vorhanden"),
    );
    TypedJson(SCHEMA_JSON, json!({
        "$schema": "https://json-schema.org/draft/2019-09/schema",
        "$id": format!("{base}/collections/flurstuecke-nutzungsart/queryables"),
        "type": "object",
        "title": "Flurstücke mit Nutzungsart (ALKIS)",
        "properties": properties
    }))
}

pub async fn list(State(state): State<SharedState>, headers: HeaderMap) -> Json<Value> {
    let base = base_url(&state, &headers);
    let mut collections = vec![collection_doc(&state, &base)];
    // Nur im Katalog, wenn ALKIS_OSM_PBF_PATH konfiguriert ist — ohne
    // Konfiguration existiert die Collection für Clients schlicht nicht,
    // exakt wie der Tile-Cache ohne ALKIS_CACHE_PATH.
    if state.settings.osm_pbf_path.is_some() {
        collections.push(collection_doc_nutzungsart(&state, &base));
    }
    Json(json!({
        "collections": collections,
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

pub async fn describe_nutzungsart(State(state): State<SharedState>, headers: HeaderMap) -> Json<Value> {
    let base = base_url(&state, &headers);
    Json(collection_doc_nutzungsart(&state, &base))
}

/// Beschreibung der Sammlung inklusive Quellenangaben.
///
/// Die Attribution wird mitgeliefert, weil ein Teil der Länder unter DL-DE BY
/// 2.0 oder CC BY 4.0 steht und damit Namensnennung verlangt. Clients können
/// die Liste unmittelbar für den Kartenhinweis verwenden.
fn collection_doc(state: &SharedState, base: &str) -> Value {
    let year = current_year();
    let sources: Vec<Value> = states::all()
        .iter()
        .filter(|s| s.endpoint.is_some())
        .map(|s| {
            let attribution = s.attribution.map(|a| {
                json!({
                    // Der fertige Vermerk, nicht die Vorlage: Ein Client soll
                    // ihn übernehmen können, ohne selbst ein Jahr einzusetzen.
                    "text": a.notice(year),
                    "url": a.url,
                    "license": a.license.label(),
                    "licenseUrl": a.license.url(),
                    "attributionRequired": a.license.requires_attribution(),
                })
            });
            json!({ "state": s.key.code(), "label": s.label, "attribution": attribution })
        })
        .collect();

    // Eine Zeile über alle angebundenen Länder, fertig für den Kartenhinweis.
    // `sources` bleibt daneben stehen, weil nur dort steht, welcher Vermerk zu
    // welchem Land gehört und welcher rechtlich zwingend ist.
    let alle: Vec<_> = states::all()
        .iter()
        .filter(|s| s.endpoint.is_some())
        .map(|s| s.key)
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
        "attribution": states::attribution_line(&alle, year),
        "sources": sources,
        "links": [
            { "rel": "self", "type": "application/json", "title": "Beschreibung", "href": format!("{base}/collections/flurstuecke") },
            { "rel": "root", "type": "application/json", "title": "Startseite", "href": base.to_string() },
            { "rel": "items", "type": "application/geo+json", "title": "Flurstücke", "href": format!("{base}/collections/flurstuecke/items") }
        ]
    })
}

/// Beschreibung der optionalen Nutzungsart-Collection — dieselben Flurstücke
/// wie `flurstuecke`, zusätzlich um eine OSM-basierte Schätzung angereichert
/// (`category`/`rule`/`confidence`/`conflict`). Taucht in `/collections` nur
/// auf, wenn `ALKIS_OSM_PBF_PATH` konfiguriert ist (siehe `list()`); die
/// Beschreibung selbst bleibt aber auch unkonfiguriert abrufbar, damit ein
/// direkter Aufruf nicht mit einem verwirrenden Fehler endet.
fn collection_doc_nutzungsart(state: &SharedState, base: &str) -> Value {
    let year = current_year();
    let sources: Vec<Value> = states::all()
        .iter()
        .filter(|s| s.endpoint.is_some())
        .map(|s| {
            let attribution = s.attribution.map(|a| {
                json!({
                    "text": a.notice(year),
                    "url": a.url,
                    "license": a.license.label(),
                    "licenseUrl": a.license.url(),
                    "attributionRequired": a.license.requires_attribution(),
                })
            });
            json!({ "state": s.key.code(), "label": s.label, "attribution": attribution })
        })
        .collect();
    let alle: Vec<_> = states::all()
        .iter()
        .filter(|s| s.endpoint.is_some())
        .map(|s| s.key)
        .collect();

    json!({
        "id": "flurstuecke-nutzungsart",
        "title": "Flurstücke mit Nutzungsart (ALKIS)",
        "description": "Wie die Sammlung flurstuecke, zusätzlich mit einer aus OpenStreetMap \
                        geschätzten Nutzungsart (category: privat/oeffentlich/bahn/unbekannt, \
                        dazu rule/confidence/conflict). Die Schätzung stammt ausschließlich aus \
                        OpenStreetMap, nie aus ALKIS-Zusatzquellen, und ist keine amtliche \
                        Eigentumsauskunft — die öffentlichen ALKIS-Daten enthalten aus \
                        Datenschutzgründen keine Eigentümerangaben. Anfragen benötigen einen \
                        Kartenausschnitt (bbox), maximal etwa 8 km Kantenlänge. Nur verfügbar, \
                        wenn der Betreiber eine lokale OSM-PBF-Datei konfiguriert hat.",
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
        "attribution": states::attribution_line(&alle, year),
        "sources": sources,
        "links": [
            { "rel": "self", "type": "application/json", "title": "Beschreibung", "href": format!("{base}/collections/flurstuecke-nutzungsart") },
            { "rel": "root", "type": "application/json", "title": "Startseite", "href": base.to_string() },
            { "rel": "items", "type": "application/geo+json", "title": "Flurstücke mit Nutzungsart", "href": format!("{base}/collections/flurstuecke-nutzungsart/items") }
        ]
    })
}
