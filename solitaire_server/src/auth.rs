//! Authentication handlers: register, login, refresh, delete account.

use axum::{extract::State, Json};
use bcrypt::{hash, verify};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    error::AppError,
    middleware::{validate_refresh_token, AuthenticatedUser, Claims},
};

// ---------------------------------------------------------------------------
// Request / response shapes
// ---------------------------------------------------------------------------

/// Body for `POST /api/auth/register` and `POST /api/auth/login`.
#[derive(Debug, Deserialize)]
pub struct AuthRequest {
    pub username: String,
    pub password: String,
}

/// Body for `POST /api/auth/refresh`.
#[derive(Debug, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Successful auth response — contains both tokens.
#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub access_token: String,
    pub refresh_token: String,
}

/// Successful refresh response — contains only the new access token.
#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
}

// ---------------------------------------------------------------------------
// Internal database row type
// ---------------------------------------------------------------------------

/// User row fetched from the database during login.
/// Fields are `Option<String>` because sqlx treats all SQLite TEXT columns
/// as nullable regardless of the NOT NULL constraint in the schema.
struct UserRow {
    id: Option<String>,
    password_hash: Option<String>,
}

// ---------------------------------------------------------------------------
// bcrypt cost used for password hashing
// ---------------------------------------------------------------------------

/// bcrypt cost factor. Per ARCHITECTURE.md §19 this must be 12.
const BCRYPT_COST: u32 = 12;

// ---------------------------------------------------------------------------
// Token generation helpers
// ---------------------------------------------------------------------------

/// Encode a JWT access token (24-hour expiry) for `user_id`.
pub fn make_access_token(user_id: &str, secret: &str) -> Result<String, AppError> {
    let exp = (Utc::now() + chrono::Duration::hours(24)).timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        exp,
        kind: "access".to_string(),
    };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// Encode a JWT refresh token (30-day expiry) for `user_id`.
pub fn make_refresh_token(user_id: &str, secret: &str) -> Result<String, AppError> {
    let exp = (Utc::now() + chrono::Duration::days(30)).timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        exp,
        kind: "refresh".to_string(),
    };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| AppError::Internal(e.to_string()))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/auth/register` — create a new account and return tokens.
/// Minimum and maximum allowed username lengths.
const USERNAME_MIN: usize = 3;
const USERNAME_MAX: usize = 32;
/// Minimum password length.
const PASSWORD_MIN: usize = 8;

/// Returns `true` if every character in `s` is ASCII alphanumeric or `_`.
fn username_chars_ok(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub async fn register(
    State(pool): State<SqlitePool>,
    Json(body): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    // Validate username: 3–32 characters, alphanumeric + underscores only.
    let trimmed = body.username.trim();
    if trimmed.len() < USERNAME_MIN || trimmed.len() > USERNAME_MAX {
        return Err(AppError::BadRequest(format!(
            "username must be {USERNAME_MIN}–{USERNAME_MAX} characters"
        )));
    }
    if !username_chars_ok(trimmed) {
        return Err(AppError::BadRequest(
            "username may only contain letters, digits, and underscores".into(),
        ));
    }
    // Validate password: minimum 8 characters.
    if body.password.len() < PASSWORD_MIN {
        return Err(AppError::BadRequest(format!(
            "password must be at least {PASSWORD_MIN} characters"
        )));
    }

    let username = trimmed.to_string();

    // Check for duplicate username. SQLite returns TEXT as nullable so we
    // flatten the Option<Option<String>> produced by fetch_optional.
    let existing: Option<String> = sqlx::query_scalar!(
        "SELECT id FROM users WHERE username = ?",
        username
    )
    .fetch_optional(&pool)
    .await?
    .flatten();

    if existing.is_some() {
        return Err(AppError::UsernameTaken);
    }

    let user_id = Uuid::new_v4().to_string();
    let password_hash = hash(&body.password, BCRYPT_COST)?;
    let now = Utc::now().to_rfc3339();

    sqlx::query!(
        "INSERT INTO users (id, username, password_hash, created_at) VALUES (?, ?, ?, ?)",
        user_id,
        username,
        password_hash,
        now
    )
    .execute(&pool)
    .await?;

    let secret = std::env::var("JWT_SECRET")
        .map_err(|_| AppError::Internal("JWT_SECRET not set".into()))?;

    Ok(Json(AuthResponse {
        access_token: make_access_token(&user_id, &secret)?,
        refresh_token: make_refresh_token(&user_id, &secret)?,
    }))
}

/// `POST /api/auth/login` — verify credentials and return tokens.
pub async fn login(
    State(pool): State<SqlitePool>,
    Json(body): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let username = body.username.trim().to_string();
    let row = sqlx::query_as!(
        UserRow,
        "SELECT id, password_hash FROM users WHERE username = ?",
        username
    )
    .fetch_optional(&pool)
    .await?;

    let row = row.ok_or(AppError::InvalidCredentials)?;
    let row_id = row.id.ok_or_else(|| AppError::Internal("user id missing".into()))?;
    let row_hash = row.password_hash.ok_or_else(|| AppError::Internal("password hash missing".into()))?;

    let valid = verify(&body.password, &row_hash)?;
    if !valid {
        return Err(AppError::InvalidCredentials);
    }

    let secret = std::env::var("JWT_SECRET")
        .map_err(|_| AppError::Internal("JWT_SECRET not set".into()))?;

    Ok(Json(AuthResponse {
        access_token: make_access_token(&row_id, &secret)?,
        refresh_token: make_refresh_token(&row_id, &secret)?,
    }))
}

/// `POST /api/auth/refresh` — exchange a refresh token for a new access token.
pub async fn refresh(
    Json(body): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, AppError> {
    let secret = std::env::var("JWT_SECRET")
        .map_err(|_| AppError::Internal("JWT_SECRET not set".into()))?;

    let claims = validate_refresh_token(&body.refresh_token, &secret)?;

    Ok(Json(RefreshResponse {
        access_token: make_access_token(&claims.sub, &secret)?,
    }))
}

/// `DELETE /api/account` — permanently delete the authenticated user's account.
///
/// All related rows are removed via `ON DELETE CASCADE` in the schema.
pub async fn delete_account(
    State(pool): State<SqlitePool>,
    user: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, AppError> {
    sqlx::query!("DELETE FROM users WHERE id = ?", user.user_id)
        .execute(&pool)
        .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}
