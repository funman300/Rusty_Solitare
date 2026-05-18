//! Authentication handlers: register, login, refresh, delete account,
//! current-user profile, and avatar upload.

use axum::{
    body::Bytes,
    extract::State,
    http::HeaderMap,
    Json,
};
use bcrypt::{hash, verify};
use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    error::AppError,
    middleware::{validate_refresh_token, AuthenticatedUser, Claims},
    AppState,
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

/// Response for `GET /api/me`.
#[derive(Debug, Serialize)]
pub struct MeResponse {
    pub id: String,
    pub username: String,
    pub avatar_url: Option<String>,
}

/// Successful refresh response — contains the new access token and the rotated
/// refresh token. The refresh token is always rotated: the client must store
/// the new value and discard the old one.
#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub refresh_token: String,
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

/// bcrypt work factor. Cost 12 ≈ 300 ms on modern hardware — balances security against registration latency.
pub const BCRYPT_COST: u32 = 12;

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
        jti: None,
    };
    encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| AppError::Internal(e.to_string()))
}

/// Encode a JWT refresh token (30-day expiry) for `user_id`.
///
/// Returns `(jwt_string, jti)`. The caller must insert the jti into
/// `refresh_tokens` before returning the JWT to the client.
pub fn make_refresh_token(user_id: &str, secret: &str) -> Result<(String, String), AppError> {
    let jti = Uuid::new_v4().to_string();
    let exp = (Utc::now() + chrono::Duration::days(30)).timestamp() as usize;
    let claims = Claims {
        sub: user_id.to_string(),
        exp,
        kind: "refresh".to_string(),
        jti: Some(jti.clone()),
    };
    let token = encode(&Header::default(), &claims, &EncodingKey::from_secret(secret.as_bytes()))
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok((token, jti))
}

/// Insert a jti row into `refresh_tokens`. Must be called immediately after
/// [`make_refresh_token`] and before the token is sent to the client.
async fn store_refresh_jti(
    pool: &sqlx::SqlitePool,
    jti: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let expires_at = (Utc::now() + chrono::Duration::days(30)).to_rfc3339();
    sqlx::query!(
        "INSERT INTO refresh_tokens (jti, user_id, expires_at) VALUES (?, ?, ?)",
        jti,
        user_id,
        expires_at
    )
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `POST /api/auth/register` — create a new account and return tokens.
/// Minimum and maximum allowed username lengths.
const USERNAME_MIN: usize = 3;
const USERNAME_MAX: usize = 32;
/// Minimum password length.
pub const PASSWORD_MIN: usize = 8;

/// Returns `true` if every character in `s` is ASCII alphanumeric or `_`.
fn username_chars_ok(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub async fn register(
    State(state): State<AppState>,
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
    .fetch_optional(&state.pool)
    .await?
    .flatten();

    if existing.is_some() {
        tracing::warn!(username = %username, "register: username already taken");
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
    .execute(&state.pool)
    .await?;

    let access_token = make_access_token(&user_id, &state.jwt_secret)?;
    let (refresh_token, refresh_jti) = make_refresh_token(&user_id, &state.jwt_secret)?;
    store_refresh_jti(&state.pool, &refresh_jti, &user_id).await?;

    Ok(Json(AuthResponse {
        access_token,
        refresh_token,
    }))
}

/// `POST /api/auth/login` — verify credentials and return tokens.
pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<AuthRequest>,
) -> Result<Json<AuthResponse>, AppError> {
    let username = body.username.trim().to_string();
    let row = sqlx::query_as!(
        UserRow,
        "SELECT id, password_hash FROM users WHERE username = ?",
        username
    )
    .fetch_optional(&state.pool)
    .await?;

    let row = row.ok_or(AppError::InvalidCredentials)?;
    let row_id = row.id.ok_or_else(|| AppError::Internal("user id missing".into()))?;
    let row_hash = row.password_hash.ok_or_else(|| AppError::Internal("password hash missing".into()))?;

    let valid = verify(&body.password, &row_hash)?;
    if !valid {
        tracing::warn!(username = %username, "login: invalid password");
        return Err(AppError::InvalidCredentials);
    }

    let access_token = make_access_token(&row_id, &state.jwt_secret)?;
    let (refresh_token, refresh_jti) = make_refresh_token(&row_id, &state.jwt_secret)?;
    store_refresh_jti(&state.pool, &refresh_jti, &row_id).await?;

    Ok(Json(AuthResponse {
        access_token,
        refresh_token,
    }))
}

/// `POST /api/auth/refresh` — exchange a valid refresh token for a new token pair.
///
/// The incoming refresh token is consumed (its jti row is deleted) and a new
/// refresh token is issued. Using a consumed token returns 401. Tokens issued
/// before rotation was enabled (no `jti` claim) are also rejected with 401 —
/// the player must re-login once after upgrading the server.
///
/// Expired rows from other sessions are pruned on each successful call.
pub async fn refresh(
    State(state): State<AppState>,
    Json(body): Json<RefreshRequest>,
) -> Result<Json<RefreshResponse>, AppError> {
    let claims = validate_refresh_token(&body.refresh_token, &state.jwt_secret)?;

    // Tokens without jti predate rotation — require re-login.
    let jti = claims.jti.ok_or(AppError::Unauthorized)?;

    // Verify this jti is still live (not yet consumed or from a deleted account).
    // SQLite TEXT columns are always nullable in sqlx; flatten the double-Option.
    let exists: Option<String> = sqlx::query_scalar!(
        "SELECT jti FROM refresh_tokens WHERE jti = ?",
        jti
    )
    .fetch_optional(&state.pool)
    .await?
    .flatten();

    if exists.is_none() {
        return Err(AppError::Unauthorized);
    }

    // Consume the old token before issuing new ones. If the insert below
    // fails, the user loses this session (must re-login) — safe by design.
    sqlx::query!("DELETE FROM refresh_tokens WHERE jti = ?", jti)
        .execute(&state.pool)
        .await?;

    let new_access = make_access_token(&claims.sub, &state.jwt_secret)?;
    let (new_refresh, new_jti) = make_refresh_token(&claims.sub, &state.jwt_secret)?;
    store_refresh_jti(&state.pool, &new_jti, &claims.sub).await?;

    // Prune expired rows from all sessions on each successful rotation.
    // The expires_at index makes this a cheap index-backed scan.
    let now = Utc::now().to_rfc3339();
    sqlx::query!("DELETE FROM refresh_tokens WHERE expires_at < ?", now)
        .execute(&state.pool)
        .await?;

    Ok(Json(RefreshResponse {
        access_token: new_access,
        refresh_token: new_refresh,
    }))
}

/// `DELETE /api/account` — permanently delete the authenticated user's account.
///
/// All related rows (sync_state, refresh_tokens, leaderboard) are removed
/// via `ON DELETE CASCADE` in the schema.
pub async fn delete_account(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<Json<serde_json::Value>, AppError> {
    sqlx::query!("DELETE FROM users WHERE id = ?", user.user_id)
        .execute(&state.pool)
        .await?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `GET /api/me` — return the authenticated user's id, username, and avatar URL.
pub async fn get_me(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<Json<MeResponse>, AppError> {
    struct Row {
        username: Option<String>,
        avatar_url: Option<String>,
    }
    let row = sqlx::query_as!(
        Row,
        "SELECT username, avatar_url FROM users WHERE id = ?",
        user.user_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("user not found".into()))?;

    Ok(Json(MeResponse {
        id: user.user_id,
        username: row.username.ok_or(AppError::Unauthorized)?,
        avatar_url: row.avatar_url,
    }))
}

/// Allowed MIME types for uploaded avatars.
const ALLOWED_IMAGE_TYPES: &[&str] = &["image/jpeg", "image/png", "image/webp", "image/gif"];
/// Maximum avatar upload size in bytes (1 MB).
const AVATAR_MAX_BYTES: usize = 1024 * 1024;

/// `PUT /api/me/avatar` — upload a new avatar image (raw bytes, ≤ 1 MB).
///
/// The `Content-Type` header must be one of `image/jpeg`, `image/png`,
/// `image/webp`, or `image/gif`. The previous avatar file is replaced in-place.
pub async fn upload_avatar(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<MeResponse>, AppError> {
    let mime = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let ext = if mime.contains("jpeg") || mime.contains("jpg") {
        "jpg"
    } else if mime.contains("png") {
        "png"
    } else if mime.contains("webp") {
        "webp"
    } else if mime.contains("gif") {
        "gif"
    } else {
        return Err(AppError::BadRequest(
            "avatar must be image/jpeg, image/png, image/webp, or image/gif".into(),
        ));
    };

    if !ALLOWED_IMAGE_TYPES.iter().any(|t| mime.starts_with(t)) {
        return Err(AppError::BadRequest("unsupported image type".into()));
    }
    if body.len() > AVATAR_MAX_BYTES {
        return Err(AppError::BadRequest("avatar must be ≤ 1 MB".into()));
    }

    // Write to avatars/ directory, replacing any previous file for this user.
    std::fs::create_dir_all("avatars").map_err(|e| AppError::Internal(e.to_string()))?;
    let filename = format!("{}.{}", user.user_id, ext);
    let path = std::path::Path::new("avatars").join(&filename);
    let tmp_path = std::path::Path::new("avatars").join(format!("{}.{}.tmp", user.user_id, ext));
    // Write to a temp file then atomically rename so concurrent readers never
    // see a partially-written avatar.
    std::fs::write(&tmp_path, &body).map_err(|e| AppError::Internal(e.to_string()))?;
    std::fs::rename(&tmp_path, &path).map_err(|e| AppError::Internal(e.to_string()))?;
    // Remove stale files with other extensions after the atomic rename.
    for old_ext in &["jpg", "png", "webp", "gif"] {
        if *old_ext != ext {
            let _ = std::fs::remove_file(
                std::path::Path::new("avatars").join(format!("{}.{}", user.user_id, old_ext)),
            );
        }
    }

    let avatar_url = format!("/avatars/{filename}");
    sqlx::query!(
        "UPDATE users SET avatar_url = ? WHERE id = ?",
        avatar_url,
        user.user_id
    )
    .execute(&state.pool)
    .await?;

    let username: Option<String> = sqlx::query_scalar!(
        "SELECT username FROM users WHERE id = ?",
        user.user_id
    )
    .fetch_optional(&state.pool)
    .await?;

    Ok(Json(MeResponse {
        id: user.user_id,
        username: username.ok_or(AppError::Unauthorized)?,
        avatar_url: Some(avatar_url),
    }))
}

// ---------------------------------------------------------------------------
// Admin helpers (CLI use only — not exposed via HTTP)
// ---------------------------------------------------------------------------

/// Reset the password for `username` to `new_password`.
///
/// On success:
/// - The `password_hash` column in `users` is overwritten with a fresh bcrypt
///   hash of `new_password`.
/// - **All** active refresh tokens for the user are deleted, forcing every
///   existing session to re-authenticate before it can issue new access tokens.
///
/// Returns `AppError::NotFound` when no account with `username` exists.
/// Returns `AppError::BadRequest` when `new_password` is shorter than
/// [`PASSWORD_MIN`].
pub async fn reset_password(
    pool: &sqlx::SqlitePool,
    username: &str,
    new_password: &str,
) -> Result<(), AppError> {
    if new_password.len() < PASSWORD_MIN {
        return Err(AppError::BadRequest(format!(
            "password must be at least {PASSWORD_MIN} characters"
        )));
    }

    let user_id: Option<String> = sqlx::query_scalar!(
        "SELECT id FROM users WHERE username = ?",
        username
    )
    .fetch_optional(pool)
    .await?
    .flatten();

    let user_id =
        user_id.ok_or_else(|| AppError::NotFound(format!("user '{username}' not found")))?;

    let new_hash = hash(new_password, BCRYPT_COST)?;

    sqlx::query!(
        "UPDATE users SET password_hash = ? WHERE id = ?",
        new_hash,
        user_id
    )
    .execute(pool)
    .await?;

    // Invalidate all active sessions — the user must log in again with the
    // new password before refresh tokens work.
    sqlx::query!("DELETE FROM refresh_tokens WHERE user_id = ?", user_id)
        .execute(pool)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, DecodingKey, Validation};

    const TEST_SECRET: &str = "test_secret_for_unit_tests_only";

    fn decode_claims(token: &str) -> Claims {
        let mut validation = Validation::default();
        validation.leeway = 60;
        decode::<Claims>(
            token,
            &DecodingKey::from_secret(TEST_SECRET.as_bytes()),
            &validation,
        )
        .unwrap()
        .claims
    }

    #[test]
    fn make_access_token_decodes_with_correct_claims() {
        let token = make_access_token("user-123", TEST_SECRET).unwrap();
        let claims = decode_claims(&token);
        assert_eq!(claims.sub, "user-123");
        assert_eq!(claims.kind, "access");
        assert!(claims.jti.is_none(), "access token must not carry a jti");
        let now = Utc::now().timestamp() as usize;
        assert!(claims.exp > now + 86_400 - 60);
        assert!(claims.exp < now + 86_400 + 60);
    }

    #[test]
    fn make_refresh_token_decodes_with_correct_claims() {
        let (token, jti) = make_refresh_token("user-456", TEST_SECRET).unwrap();
        let claims = decode_claims(&token);
        assert_eq!(claims.sub, "user-456");
        assert_eq!(claims.kind, "refresh");
        assert_eq!(
            claims.jti.as_deref(),
            Some(jti.as_str()),
            "jti in JWT must match returned jti"
        );
        assert!(!jti.is_empty(), "jti must be non-empty");
        let now = Utc::now().timestamp() as usize;
        assert!(claims.exp > now + 30 * 86_400 - 60);
        assert!(claims.exp < now + 30 * 86_400 + 60);
    }

    #[test]
    fn make_refresh_token_generates_unique_jtis() {
        let (_, jti1) = make_refresh_token("u", TEST_SECRET).unwrap();
        let (_, jti2) = make_refresh_token("u", TEST_SECRET).unwrap();
        assert_ne!(jti1, jti2, "each call must produce a unique jti");
    }

    #[test]
    fn make_access_token_wrong_secret_fails_decode() {
        let token = make_access_token("user-789", TEST_SECRET).unwrap();
        let result = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(b"wrong_secret"),
            &Validation::default(),
        );
        assert!(result.is_err(), "decoding with wrong secret must fail");
    }

    #[test]
    fn access_and_refresh_tokens_have_different_kinds() {
        let access = make_access_token("u", TEST_SECRET).unwrap();
        let (refresh, _jti) = make_refresh_token("u", TEST_SECRET).unwrap();
        let a_claims = decode_claims(&access);
        let r_claims = decode_claims(&refresh);
        assert_ne!(a_claims.kind, r_claims.kind);
    }

    #[test]
    fn username_chars_ok_accepts_alphanumeric_and_underscore() {
        assert!(username_chars_ok("alice"));
        assert!(username_chars_ok("Alice_123"));
        assert!(username_chars_ok("UPPER_case_99"));
    }

    #[test]
    fn username_chars_ok_rejects_special_chars() {
        assert!(!username_chars_ok("ali ce"));   // space
        assert!(!username_chars_ok("ali-ce"));   // hyphen
        assert!(!username_chars_ok("ali.ce"));   // dot
        assert!(!username_chars_ok("ali@ce"));   // at
        assert!(!username_chars_ok("ali!ce"));   // exclamation
    }

    #[test]
    fn username_chars_ok_accepts_empty_string() {
        // The length check in `register` guards against empty usernames;
        // this function only validates characters, so empty is technically ok.
        assert!(username_chars_ok(""));
    }

    #[test]
    fn username_chars_ok_rejects_unicode_letters() {
        // Non-ASCII characters must be rejected even if they look like letters.
        assert!(!username_chars_ok("héro"));
        assert!(!username_chars_ok("用户"));
    }
}
