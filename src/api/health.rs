//! Betriebsendpunkte.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::api::SharedState;
use crate::config::states;

pub async fn health(State(state): State<SharedState>) -> impl IntoResponse {
    let open = state.breaker.open_states();
    let configured = states::all().iter().filter(|s| s.endpoint.is_some()).count();
    // Drei Zustände statt eines Bool: "aus" (nicht konfiguriert) ist ein
    // gewollter Betriebszustand, kein Fehler — anders als "wird_indiziert",
    // das nur vorübergehend gilt.
    let classification = if state.settings.osm_pbf_path.is_none() {
        "aus"
    } else if state.classification.read().await.is_some() {
        "bereit"
    } else {
        "wird_indiziert"
    };
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "cache": if state.cache.is_enabled() { "redb" } else { "aus" },
        "statesConfigured": configured,
        "statesUnavailable": open,
        "classification": classification,
    }))
}

pub async fn metrics(State(state): State<SharedState>) -> impl IntoResponse {
    let open = state.breaker.open_states();
    (
        [("content-type", "text/plain; version=0.0.4")],
        state.metrics.render(&open, state.cache.is_enabled()),
    )
}
