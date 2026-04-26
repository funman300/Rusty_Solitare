//! Axum middleware for JWT authentication.
//!
//! Extracts and validates the `Authorization: Bearer <token>` header, then
//! injects the authenticated `user_id` into request extensions so handlers
//! can access it via `Extension<AuthenticatedUser>`.

use axum::{
    extract::{FromRequestParts, Request},
    http::request::Parts,
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// The claims encoded in our JWT access tokens.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Subject — the user's UUID string.
    pub sub: String,
    /// Expiry timestamp (Unix seconds).
    pub exp: usize,
    /// Token kind: `"access"` or `"refresh"`.
    pub kind: String,
}

/// The authenticated user identity injected into request extensions after
/// successful JWT validation.
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    /// The authenticated user's UUID, as a string.
    pub user_id: String,
}

/// Axum middleware function that validates the Bearer JWT and injects
/// [`AuthenticatedUser`] into request extensions.
///
/// Returns `401 Unauthorized` if the token is missing, expired, or invalid.
pub async fn require_auth(
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let secret = std::env::var("JWT_SECRET")
        .map_err(|_| AppError::Internal("JWT_SECRET not set".into()))?;

    let token = extract_bearer_token(req.headers())
        .ok_or(AppError::Unauthorized)?;

    let claims = validate_access_token(&token, &secret)?;

    req.extensions_mut().insert(AuthenticatedUser {
        user_id: claims.sub,
    });

    Ok(next.run(req).await)
}

/// Extract the raw token string from `Authorization: Bearer <token>`.
fn extract_bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get("Authorization")?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?;
    Some(token.to_string())
}

/// Decode and validate a JWT access token, returning its claims on success.
pub fn validate_access_token(token: &str, secret: &str) -> Result<Claims, AppError> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::default();
    validation.validate_exp = true;

    let data = decode::<Claims>(token, &key, &validation)
        .map_err(|_| AppError::Unauthorized)?;

    if data.claims.kind != "access" {
        return Err(AppError::Unauthorized);
    }

    Ok(data.claims)
}

/// Decode and validate a JWT refresh token, returning its claims on success.
pub fn validate_refresh_token(token: &str, secret: &str) -> Result<Claims, AppError> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::default();
    validation.validate_exp = true;

    let data = decode::<Claims>(token, &key, &validation)
        .map_err(|_| AppError::Unauthorized)?;

    if data.claims.kind != "refresh" {
        return Err(AppError::Unauthorized);
    }

    Ok(data.claims)
}

// ---------------------------------------------------------------------------
// Axum extractor — allows handlers to receive AuthenticatedUser directly
// ---------------------------------------------------------------------------

#[axum::async_trait]
impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or(AppError::Unauthorized)
    }
}
