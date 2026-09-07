//! HTTP-Schicht: OGC API Features.

pub mod collections;
pub mod error;
pub mod features;
pub mod health;
pub mod metrics;
pub mod openapi;

use std::sync::Arc;

use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;

use axum::http::HeaderMap;

use crate::cache::TileCache;
use crate::config::Settings;
use crate::upstream::{CircuitBreaker, UpstreamClient};
use metrics::Metrics;

pub struct AppState {
    pub settings: Settings,
    pub client: UpstreamClient,
    pub cache: TileCache,
    pub breaker: CircuitBreaker,
    pub metrics: Metrics,
}

pub type SharedState = Arc<AppState>;

/// Medientyp für Feature-Antworten, von OGC API Features Teil 1 vorgeschrieben.
pub const GEOJSON: &str = "application/geo+json";
/// Medientyp der API-Definition. Genau diese Schreibweise sucht QGIS im
/// `service-desc`-Link der Landing Page.
pub const OPENAPI: &str = "application/vnd.oai.openapi+json;version=3.0";
/// Medientyp für JSON-Schema-Dokumente, hier für `/queryables`.
pub const SCHEMA_JSON: &str = "application/schema+json";

/// JSON-Antwort mit einem anderen Medientyp als `application/json`.
///
/// `Json` setzt immer `application/json`. Für Feature-Antworten und die
/// API-Definition schreibt der Standard aber eigene Medientypen vor, deshalb
/// wird der Header hier ersetzt.
pub struct TypedJson(pub &'static str, pub Value);

impl IntoResponse for TypedJson {
    fn into_response(self) -> Response {
        let mut res = Json(self.1).into_response();
        res.headers_mut().insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static(self.0),
        );
        res
    }
}

/// Öffentliche Basis-URL des Dienstes, ohne abschließenden Schrägstrich.
///
/// OGC API Features verlangt absolute Links; Clients wie QGIS folgen ihnen
/// wörtlich. Hinter einem Reverse Proxy stimmt der `Host`-Header nicht
/// zwangsläufig mit der nach außen sichtbaren Adresse überein — dann setzt man
/// `ALKIS_PUBLIC_URL`.
pub fn base_url(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(configured) = &state.settings.public_url {
        return configured.trim_end_matches('/').to_string();
    }
    let host = headers
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    format!("{scheme}://{host}")
}

/// Antwort auf `OPTIONS`.
///
/// QGIS stellt vor dem Laden eines Layers eine OPTIONS-Anfrage, um zu prüfen,
/// ob der Dienst schreibbar ist. Ohne diese Route antwortet der Router mit
/// `405 Method Not Allowed`, was QGIS als Fehler protokolliert und den Layer
/// nicht laden lässt. Der Dienst ist ausschließlich lesend.
async fn options_read_only() -> impl IntoResponse {
    (
        StatusCode::NO_CONTENT,
        [
            ("allow", "GET, HEAD, OPTIONS"),
            // Erlaubt die Nutzung aus Webanwendungen heraus.
            ("access-control-allow-origin", "*"),
            ("access-control-allow-methods", "GET, HEAD, OPTIONS"),
            ("access-control-allow-headers", "*"),
        ],
    )
}

pub fn router(state: SharedState) -> Router {
    // Jede Route beantwortet zusätzlich OPTIONS; HEAD bedient axum automatisch
    // über den GET-Handler.
    Router::new()
        .route("/", get(collections::landing).options(options_read_only))
        .route("/api", get(openapi::api).options(options_read_only))
        .route(
            "/conformance",
            get(collections::conformance).options(options_read_only),
        )
        .route(
            "/collections",
            get(collections::list).options(options_read_only),
        )
        .route(
            "/collections/flurstuecke",
            get(collections::describe).options(options_read_only),
        )
        .route(
            "/collections/flurstuecke/queryables",
            get(collections::queryables).options(options_read_only),
        )
        .route(
            "/collections/flurstuecke/items",
            get(features::items).options(options_read_only),
        )
        .route(
            "/collections/flurstuecke/items/{id}",
            get(features::item).options(options_read_only),
        )
        .route("/health", get(health::health).options(options_read_only))
        .route("/metrics", get(health::metrics).options(options_read_only))
        .with_state(state)
}
