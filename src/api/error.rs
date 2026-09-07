//! Fehlerantworten im Format RFC 7807 (`application/problem+json`).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    /// Der Ausschnitt überdeckt mehr Fläche, als eine Anfrage bedienen kann.
    ///
    /// Bewusst ein Fehler und keine leere Antwort: Clients mit
    /// Kartencache — QGIS etwa — merken sich, welche Regionen sie bereits
    /// vollständig geladen haben. Eine erfolgreiche leere Antwort auf einen
    /// weiten Ausschnitt markiert dort die ganze Region als „geladen, nichts
    /// drin“. Danach fragt der Client auch beim Hineinzoomen nicht mehr nach,
    /// und der Layer bleibt leer, bis er neu angelegt wird. Ein Fehlerstatus
    /// hinterlässt diesen Eintrag nicht.
    #[error("{0}")]
    TooLarge(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, title) = match &self {
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "Ungültige Anfrage"),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "Nicht gefunden"),
            ApiError::TooLarge(_) => (StatusCode::BAD_REQUEST, "Ausschnitt zu groß"),
        };
        // Ohne diese Zeile bleibt eine abgelehnte Anfrage im Log unsichtbar:
        // der Client bekommt 4xx, der Betreiber sieht nichts. Gerade bei
        // Formaten, die der Dienst nicht annimmt — etwa eine bbox mit sechs
        // Werten — ist das die einzige Spur.
        tracing::warn!(status = status.as_u16(), detail = %self, "Anfrage abgelehnt");
        let body = Json(json!({
            "type": "about:blank",
            "title": title,
            "status": status.as_u16(),
            "detail": self.to_string(),
        }));
        (status, [("content-type", "application/problem+json")], body).into_response()
    }
}
