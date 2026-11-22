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

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use serde_json::Value;

    async fn sent(error: ApiError) -> (StatusCode, String, Value) {
        let response = error.into_response();
        let status = response.status();
        let kind = response.headers()["content-type"].to_str().unwrap().to_string();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, kind, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn every_error_is_one_json_object_with_a_code_and_a_message() {
        for (error, status, code, message) in [
            (ApiError::not_found("Run \"7\""), 404, "not_found", "Run \"7\" was not found"),
            (ApiError::bad_request("invalid_json", "The request body is not valid"), 400, "invalid_json", "The request body is not valid"),
            (ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too_many_runs", "Wait"), 429, "too_many_runs", "Wait"),
            (ApiError::invalid("Not valid", Vec::new()), 422, "validation_failed", "Not valid"),
        ] {
            let (got, kind, body) = sent(error).await;
            assert_eq!((got.as_u16(), kind.as_str()), (status, "application/json"));
            // Nothing but the code and the message: no empty list of details.
            assert_eq!(body, json!({ "error": { "code": code, "message": message } }));
        }
    }

    #[tokio::test]
    async fn problems_with_what_was_sent_are_listed_each_with_where_it_is() {
        let issues = vec![Issue::new("iterations", "must be between 1 and 10000"), Issue::new("actors[1]", "there is no strategy \"x\"")];
        let (status, _, body) = sent(ApiError::invalid("The run request is not valid", issues)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            body,
            json!({ "error": { "code": "validation_failed", "message": "The run request is not valid", "details": [
                { "path": "iterations", "message": "must be between 1 and 10000" },
                { "path": "actors[1]", "message": "there is no strategy \"x\"" },
            ] } })
        );
    }

    #[tokio::test]
    async fn what_went_wrong_inside_is_logged_and_not_told() {
        let secret = "connection to postgres://lilanaya:hunter2@db:5432 refused";
        for error in [ApiError::internal(secret), ApiError::from(sqlx::Error::Protocol(secret.into()))] {
            let (status, _, body) = sent(error).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(body, json!({ "error": { "code": "internal", "message": "Something went wrong on the server" } }));
        }
    }
}
