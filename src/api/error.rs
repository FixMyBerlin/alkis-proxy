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
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, title) = match &self {
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, "Ungültige Anfrage"),
            ApiError::NotFound(_) => (StatusCode::NOT_FOUND, "Nicht gefunden"),
        };
        let body = Json(json!({
            "type": "about:blank",
            "title": title,
            "status": status.as_u16(),
            "detail": self.to_string(),
        }));
        (status, [("content-type", "application/problem+json")], body).into_response()
    }
}
