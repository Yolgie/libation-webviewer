//! Unified error type for axum handlers.
//!
//! Most route handlers compose their response inline (rendering a
//! tailored error template), but the download handler — and any
//! future handler whose error path is uniform — benefits from a
//! single `AppError` that flows through `?` and `IntoResponse`. The
//! type stays small on purpose: a status code and a plain-text body.
//! Handlers that need a richer template render the response directly.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug)]
pub struct AppError {
    pub status: StatusCode,
    pub message: String,
}

impl AppError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::internal(err.to_string())
    }
}

impl From<axum::http::Error> for AppError {
    fn from(err: axum::http::Error) -> Self {
        Self::internal(err.to_string())
    }
}
