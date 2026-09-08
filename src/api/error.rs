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
    /// Der Ausschnitt ist klein genug, enthält aber mehr Flurstücke, als eine
    /// Anfrage vollständig liefern kann.
    ///
    /// Aus demselben Grund ein Fehler wie [`ApiError::TooLarge`]: Eine
    /// abgeschnittene Antwort ist für einen Client mit Kartencache nicht von
    /// einer vollständigen zu unterscheiden — sie trägt keinen `next`-Link,
    /// weil der Dienst die Reste gar nicht kennt. QGIS verbucht den Ausschnitt
    /// dann als fertig geladen und fragt nicht wieder nach.
    #[error("{0}")]
    TooDense(String),
    /// Etwas ist schiefgegangen, was der Client nicht verursacht hat.
    ///
    /// Ebenfalls bewusst ein Fehler statt einer notdürftig zusammengesetzten
    /// Antwort: Lieber gar kein Ergebnis als ein stilles Teilergebnis mit
    /// Status 200 — siehe [`ApiError::TooLarge`].
    #[error("{0}")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, title) = match &self {
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "Ungültige Anfrage"),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "Nicht gefunden"),
            ApiError::TooLarge(_) => (StatusCode::BAD_REQUEST, "Ausschnitt zu groß"),
            ApiError::TooDense(_) => (StatusCode::BAD_REQUEST, "Zu viele Flurstücke"),
            ApiError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Interner Fehler"),
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
