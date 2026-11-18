//! One JSON error shape for every failure:
//! `{"error": {"code": "...", "message": "...", "details": [{"path", "message"}]}}`
//! (`details` is present only when there is something to list).

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use lilanaya_engine::Issue;
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Vec<Issue>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        ApiError { status, code, message: message.into(), details: Vec::new() }
    }

    pub fn not_found(what: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", format!("{what} was not found"))
    }

    pub fn bad_request(code: &'static str, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    /// Something on our side broke. The detail goes to the log, not to the client.
    pub fn internal(detail: impl std::fmt::Display) -> Self {
        eprintln!("internal error: {detail}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", "Something went wrong on the server")
    }

    pub fn invalid(message: impl Into<String>, details: Vec<Issue>) -> Self {
        ApiError { details, ..Self::new(StatusCode::UNPROCESSABLE_ENTITY, "validation_failed", message) }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        Self::internal(format!("database: {e}"))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut error = json!({ "code": self.code, "message": self.message });
        if !self.details.is_empty() {
            error["details"] = json!(self.details);
        }
        (self.status, Json(json!({ "error": error }))).into_response()
    }
}
