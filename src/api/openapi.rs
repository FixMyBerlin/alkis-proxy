//! Die API-Definition unter `/api`.
//!
//! OGC API Features Teil 1 verlangt in Requirement 2, dass die Landing Page auf
//! eine API-Definition verweist, und in Requirement 3, dass diese per GET
//! abrufbar ist.
//!
//! Für QGIS ist das Dokument keine Formsache. Sein OAPIF-Provider liest daraus
//! `components.parameters.limit.schema.maximum` und `.default`, um die
//! Seitengröße zu bestimmen. Findet er sie nicht, greift in `QgsOapifProvider`
//! der Notnagel `mPageSize = 100` — der Layer holt dann 100 Objekte je Seite
//! und, ohne `next`-Link, überhaupt nur diese 100.
//!
//! QGIS leitet aus den
//! Parametern der `items`-Operation ab, wonach sich serverseitig filtern lässt.

use axum::extract::State;
use axum::http::HeaderMap;

use crate::api::{base_url, SharedState, TypedJson, OPENAPI};

pub async fn api(State(state): State<SharedState>, headers: HeaderMap) -> TypedJson {
    let base = base_url(&state, &headers);
    let json_response = |description: &str| {
        serde_json::json!({
            "200": {
                "description": description,
                "content": { "application/json": { "schema": { "type": "object" } } }
            }
        })
    };

    // Nur beschrieben, wenn die Collection auch existiert — sonst würde die
    // API-Definition einen Pfad bewerben, den `/collections` gar nicht führt.
    let mut paths = serde_json::json!({});
    if state.settings.osm_pbf_path.is_some() {
        paths["/collections/flurstuecke-nutzungsart"] = serde_json::json!({ "get": {
            "summary": "Beschreibung der Nutzungsart-Sammlung",
            "operationId": "describeCollectionNutzungsart",
            "responses": json_response("Beschreibung der Sammlung flurstuecke-nutzungsart.")
        }});
        paths["/collections/flurstuecke-nutzungsart/items"] = serde_json::json!({ "get": {
            "summary": "Flurstücke mit geschätzter Nutzungsart in einem Ausschnitt",
            "operationId": "getFeaturesNutzungsart",
            "parameters": [
                { "$ref": "#/components/parameters/bbox" },
                { "$ref": "#/components/parameters/limit" },
                { "$ref": "#/components/parameters/offset" },
                { "$ref": "#/components/parameters/state" }
            ],
            "responses": {
                "200": {
                    "description": "Die Flurstücke im angefragten Ausschnitt, \
                                     angereichert um category/rule/confidence/conflict.",
                    "content": { "application/geo+json": { "schema": { "type": "object" } } }
                },
                "400": {
                    "description": "Fehlerhafte Anfrage — auch, wenn der Ausschnitt zu weit \
                                    ist oder mehr Flurstücke enthält, als eine Anfrage \
                                    vollständig liefern kann.",
                    "content": { "application/problem+json": { "schema": { "type": "object" } } }
                }
            }
        }});
    }

    let mut document = serde_json::json!({
            "openapi": "3.0.3",
            "info": {
                "title": "ALKIS-Proxy",
                "description": "Flurstücke aus den Liegenschaftskatastern aller deutschen \
                                Bundesländer über eine einheitliche OGC-API-Features-Schnittstelle.",
                "version": env!("CARGO_PKG_VERSION"),
                "license": { "name": "MIT" }
            },
            "servers": [{ "url": base }],
            "components": {
                "parameters": {
                    // QGIS liest `default` und `maximum` genau hier heraus.
                    "limit": {
                        "name": "limit",
                        "in": "query",
                        "description": "Höchstzahl der Objekte in einer Antwort.",
                        "required": false,
                        "style": "form",
                        "explode": false,
                        "schema": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": state.settings.max_limit,
                            "default": state.settings.default_limit
                        }
                    },
                    "offset": {
                        "name": "offset",
                        "in": "query",
                        "description": "Startversatz innerhalb der Treffermenge; \
                                        Grundlage der Seitennavigation über den next-Link.",
                        "required": false,
                        "style": "form",
                        "explode": false,
                        "schema": { "type": "integer", "minimum": 0, "default": 0 }
                    },
                    "bbox": {
                        "name": "bbox",
                        "in": "query",
                        "description": "Ausschnitt in WGS84 als min_lon,min_lat,max_lon,max_lat. \
                                        Höchstens etwa 8 km Kantenlänge; größere \
                                        Ausschnitte werden mit 400 abgelehnt. Ebenso \
                                        abgelehnt wird ein kleinerer, aber so dicht \
                                        bebauter Ausschnitt, dass mehr Flurstücke darin \
                                        liegen als eine Anfrage liefern kann — ein \
                                        gekürztes Ergebnis wäre von einem vollständigen \
                                        nicht zu unterscheiden.",
                        "required": false,
                        "style": "form",
                        "explode": false,
                        "schema": {
                            "type": "array",
                            "minItems": 4,
                            "maxItems": 4,
                            "items": { "type": "number" }
                        }
                    },
                    "state": {
                        "name": "state",
                        "in": "query",
                        "description": "Beschränkung auf ein Bundesland (etwa NW). Ohne Angabe \
                                        bestimmt der Dienst die zuständigen Länder selbst.",
                        "required": false,
                        "style": "form",
                        "explode": false,
                        "schema": { "type": "string", "pattern": "^[A-Z]{2}$" }
                    }
                }
            },
            "paths": {
                "/": { "get": {
                    "summary": "Landing Page",
                    "operationId": "getLandingPage",
                    "responses": json_response("Einstiegspunkt mit Verweisen auf die übrigen Ressourcen.")
                }},
                "/api": { "get": {
                    "summary": "Diese API-Definition",
                    "operationId": "getApiDefinition",
                    "responses": json_response("Die API-Definition.")
                }},
                "/conformance": { "get": {
                    "summary": "Erfüllte Konformitätsklassen",
                    "operationId": "getConformanceDeclaration",
                    "responses": json_response("Liste der Konformitätsklassen.")
                }},
                "/collections": { "get": {
                    "summary": "Verfügbare Sammlungen",
                    "operationId": "getCollections",
                    "responses": json_response("Die Sammlungen des Dienstes.")
                }},
                "/collections/flurstuecke": { "get": {
                    "summary": "Beschreibung der Sammlung",
                    "operationId": "describeCollection",
                    "responses": json_response("Beschreibung der Sammlung flurstuecke.")
                }},
                "/collections/flurstuecke/queryables": { "get": {
                    "summary": "Feldbeschreibung der Sammlung",
                    "operationId": "getQueryables",
                    "responses": json_response("Die Felder der Sammlung als JSON Schema.")
                }},
                "/collections/flurstuecke/items": { "get": {
                    "summary": "Flurstücke in einem Ausschnitt",
                    "operationId": "getFeatures",
                    "parameters": [
                        { "$ref": "#/components/parameters/bbox" },
                        { "$ref": "#/components/parameters/limit" },
                        { "$ref": "#/components/parameters/offset" },
                        { "$ref": "#/components/parameters/state" }
                    ],
                    "responses": {
                        "200": {
                            "description": "Die Flurstücke im angefragten Ausschnitt.",
                            "content": { "application/geo+json": { "schema": { "type": "object" } } }
                        },
                        "400": {
                            "description": "Fehlerhafte Anfrage — auch, wenn der Ausschnitt \
                                        zu weit ist oder mehr Flurstücke enthält, als eine \
                                        Anfrage vollständig liefern kann.",
                            "content": { "application/problem+json": { "schema": { "type": "object" } } }
                        }
                    }
                }},
                "/collections/flurstuecke/items/{featureId}": { "get": {
                    "summary": "Einzelnes Flurstück",
                    "operationId": "getFeature",
                    "parameters": [{
                        "name": "featureId",
                        "in": "path",
                        "required": true,
                        "schema": { "type": "string" }
                    }],
                    "responses": {
                        "404": {
                            "description": "Ohne räumliche Anfrage nicht auflösbar.",
                            "content": { "application/problem+json": { "schema": { "type": "object" } } }
                        }
                    }
                }}
            }
        });

    if let Some(extra) = paths.as_object() {
        document["paths"]
            .as_object_mut()
            .expect("paths ist ein Objekt")
            .extend(extra.clone());
    }

    TypedJson(OPENAPI, document)
}
