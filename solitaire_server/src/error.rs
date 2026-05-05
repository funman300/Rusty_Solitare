//! Application-level error type with automatic HTTP response conversion.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

/// All errors that can be returned by the server.
///
/// Each variant maps to a specific HTTP status code when converted to a
/// response via [`IntoResponse`].
#[derive(Debug, Error)]
pub enum AppError {
    /// The request is missing a valid `Authorization: Bearer` header, or the
    /// JWT is expired / has an invalid signature.
    #[error("unauthorized")]
    Unauthorized,

    /// The supplied credentials (username / password) were incorrect.
    #[error("invalid credentials")]
    InvalidCredentials,

    /// The requested username is already registered.
    #[error("username already taken")]
    UsernameTaken,

    /// The client sent a malformed or invalid request body.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// The requested resource does not exist.
    #[error("not found: {0}")]
    NotFound(String),

    /// A database error occurred.
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    /// Password hashing failed.
    #[error("internal server error")]
    BcryptError(#[from] bcrypt::BcryptError),

    /// JSON serialization / deserialization failed.
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),

    /// A catch-all for unexpected internal failures.
    #[error("internal server error")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::Unauthorized | AppError::InvalidCredentials => {
                (StatusCode::UNAUTHORIZED, self.to_string())
            }
            AppError::UsernameTaken => (StatusCode::CONFLICT, self.to_string()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            AppError::Database(e) => {
                tracing::error!("database error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            AppError::BcryptError(e) => {
                tracing::error!("bcrypt error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            AppError::Json(e) => {
                tracing::error!("json error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            AppError::Internal(msg) => {
                tracing::error!("internal error: {msg}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
        };

        let body = Json(json!({ "error": message }));
        (status, body).into_response()
    }
}
