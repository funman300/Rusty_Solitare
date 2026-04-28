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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};
    use chrono::Utc;
    use jsonwebtoken::{encode, EncodingKey, Header};

    const SECRET: &str = "test_secret_for_middleware_unit_tests_only";

    fn make_token(user_id: &str, kind: &str, exp_offset_secs: i64) -> String {
        let exp = (Utc::now() + chrono::Duration::seconds(exp_offset_secs)).timestamp() as usize;
        let claims = Claims {
            sub: user_id.to_string(),
            exp,
            kind: kind.to_string(),
        };
        encode(&Header::default(), &claims, &EncodingKey::from_secret(SECRET.as_bytes())).unwrap()
    }

    // -----------------------------------------------------------------------
    // extract_bearer_token
    // -----------------------------------------------------------------------

    #[test]
    fn extract_bearer_token_returns_token_from_valid_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_static("Bearer my.jwt.token"),
        );
        assert_eq!(extract_bearer_token(&headers), Some("my.jwt.token".to_string()));
    }

    #[test]
    fn extract_bearer_token_returns_none_when_header_missing() {
        let headers = HeaderMap::new();
        assert_eq!(extract_bearer_token(&headers), None);
    }

    #[test]
    fn extract_bearer_token_returns_none_for_wrong_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            HeaderValue::from_static("Token my.jwt.token"),
        );
        assert_eq!(extract_bearer_token(&headers), None);
    }

    #[test]
    fn extract_bearer_token_returns_none_for_empty_value() {
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", HeaderValue::from_static(""));
        assert_eq!(extract_bearer_token(&headers), None);
    }

    // -----------------------------------------------------------------------
    // validate_access_token
    // -----------------------------------------------------------------------

    #[test]
    fn validate_access_token_accepts_valid_access_token() {
        let token = make_token("user-abc", "access", 3600);
        let claims = validate_access_token(&token, SECRET).expect("should accept valid access token");
        assert_eq!(claims.sub, "user-abc");
        assert_eq!(claims.kind, "access");
    }

    #[test]
    fn validate_access_token_rejects_refresh_token() {
        let token = make_token("user-abc", "refresh", 3600);
        let result = validate_access_token(&token, SECRET);
        assert!(result.is_err(), "refresh token must be rejected by access validator");
    }

    #[test]
    fn validate_access_token_rejects_expired_token() {
        // Use -7200 (2 hours past) to exceed jsonwebtoken's default 60-second leeway.
        let token = make_token("user-abc", "access", -7200);
        let result = validate_access_token(&token, SECRET);
        assert!(result.is_err(), "expired token must be rejected");
    }

    #[test]
    fn validate_access_token_rejects_wrong_secret() {
        let token = make_token("user-abc", "access", 3600);
        let result = validate_access_token(&token, "wrong_secret");
        assert!(result.is_err(), "token signed with different secret must be rejected");
    }

    // -----------------------------------------------------------------------
    // validate_refresh_token
    // -----------------------------------------------------------------------

    #[test]
    fn validate_refresh_token_accepts_valid_refresh_token() {
        let token = make_token("user-xyz", "refresh", 86400);
        let claims = validate_refresh_token(&token, SECRET).expect("should accept valid refresh token");
        assert_eq!(claims.sub, "user-xyz");
        assert_eq!(claims.kind, "refresh");
    }

    #[test]
    fn validate_refresh_token_rejects_access_token() {
        let token = make_token("user-xyz", "access", 86400);
        let result = validate_refresh_token(&token, SECRET);
        assert!(result.is_err(), "access token must be rejected by refresh validator");
    }

    #[test]
    fn validate_refresh_token_rejects_expired_token() {
        // Use -7200 (2 hours past) to exceed jsonwebtoken's default 60-second leeway.
        let token = make_token("user-xyz", "refresh", -7200);
        let result = validate_refresh_token(&token, SECRET);
        assert!(result.is_err(), "expired refresh token must be rejected");
    }
}
