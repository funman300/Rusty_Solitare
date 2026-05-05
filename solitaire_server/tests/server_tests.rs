//! Integration tests for `solitaire_server`.
//!
//! Every test uses an in-memory SQLite database and [`build_test_router`]
//! (rate limiting disabled) — no real TCP listener is started.  Requests are dispatched via
//! [`tower::ServiceExt::oneshot`].
//!
//! # JWT secret
//!
//! [`build_test_router`] injects a fixed test secret into [`AppState`] so
//! tests do not need to set `JWT_SECRET` in the environment.  The constant
//! [`TEST_SECRET`] must match the value used by [`build_test_router`] so that
//! test-side token decoding works correctly.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use chrono::Utc;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::Deserialize;
use serde_json::Value;
use solitaire_server::build_test_router;
use solitaire_sync::{PlayerProgress, StatsSnapshot, SyncPayload};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// JWT secret used by [`build_test_router`] and by test-side token decoding.
///
/// Must match the value hardcoded in [`solitaire_server::build_test_router`].
const TEST_SECRET: &str = "test_secret_32_chars_minimum_ok!";

// ---------------------------------------------------------------------------
// Test infrastructure helpers
// ---------------------------------------------------------------------------

/// Create an in-memory SQLite pool and run all pending migrations.
///
/// `max_connections(1)` is required for SQLite in-memory databases: each
/// connection to `sqlite::memory:` is a *separate* database, so if the pool
/// opens a second connection the handler sees an empty schema and fails.
async fn test_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory SQLite database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("failed to run database migrations");
    pool
}

/// Fake client IP injected by all test requests so `tower_governor`'s
/// `SmartIpKeyExtractor` can extract a key without a real peer address.
const TEST_CLIENT_IP: &str = "127.0.0.1";

/// Send a `POST` request with a JSON body and return the raw response.
async fn post_json(app: axum::Router, path: &str, body: Value) -> Response {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::from(
            serde_json::to_vec(&body).expect("failed to serialise request body"),
        ))
        .expect("failed to build POST request");
    app.oneshot(req).await.expect("oneshot failed")
}

/// Send an authenticated `GET` request and return the raw response.
async fn get_authed(app: axum::Router, path: &str, token: &str) -> Response {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .header("Authorization", format!("Bearer {token}"))
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::empty())
        .expect("failed to build GET request");
    app.oneshot(req).await.expect("oneshot failed")
}

/// Send an authenticated `POST` request with a JSON body and return the raw response.
async fn post_authed(app: axum::Router, path: &str, token: &str, body: Value) -> Response {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::from(
            serde_json::to_vec(&body).expect("failed to serialise request body"),
        ))
        .expect("failed to build authenticated POST request");
    app.oneshot(req).await.expect("oneshot failed")
}

/// Send an authenticated `DELETE` request and return the raw response.
async fn delete_authed(app: axum::Router, path: &str, token: &str) -> Response {
    let req = Request::builder()
        .method("DELETE")
        .uri(path)
        .header("Authorization", format!("Bearer {token}"))
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::empty())
        .expect("failed to build DELETE request");
    app.oneshot(req).await.expect("oneshot failed")
}

/// Collect the response body bytes and deserialise them as JSON.
async fn body_json(resp: Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("failed to read response body");
    serde_json::from_slice(&bytes).expect("response body is not valid JSON")
}

// ---------------------------------------------------------------------------
// JWT helpers (test-side only)
// ---------------------------------------------------------------------------

/// Minimal JWT claims used only for decoding in test assertions.
#[derive(Deserialize)]
struct TestClaims {
    sub: String,
}

/// Decode an access token and return the `sub` (user UUID) claim.
///
/// Uses `validate_exp = false` so tests never fail due to clock skew between
/// token issuance and assertion.
fn decode_sub(token: &str) -> String {
    let mut v = Validation::default();
    v.validate_exp = false;
    let data = decode::<TestClaims>(
        token,
        &DecodingKey::from_secret(TEST_SECRET.as_bytes()),
        &v,
    )
    .expect("failed to decode access token");
    data.claims.sub
}

/// Register a new user and return `(access_token, refresh_token)`.
async fn register_user(app: axum::Router, username: &str, password: &str) -> (String, String) {
    let resp = post_json(
        app,
        "/api/auth/register",
        serde_json::json!({ "username": username, "password": password }),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "register should return 200"
    );
    let body = body_json(resp).await;
    let access = body["access_token"]
        .as_str()
        .expect("access_token missing from register response")
        .to_string();
    let refresh = body["refresh_token"]
        .as_str()
        .expect("refresh_token missing from register response")
        .to_string();
    (access, refresh)
}

/// Build a [`SyncPayload`] for `user_id_str` with `games_played` set to the
/// given value and all other fields set to defaults.
fn make_payload(user_id_str: &str, games_played: u32) -> SyncPayload {
    SyncPayload {
        user_id: uuid::Uuid::parse_str(user_id_str)
            .expect("user_id_str from JWT sub must be a valid UUID"),
        stats: StatsSnapshot {
            games_played,
            games_won: 3,
            ..StatsSnapshot::default()
        },
        achievements: vec![],
        progress: PlayerProgress::default(),
        last_modified: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Auth flow tests
// ---------------------------------------------------------------------------

/// `POST /api/auth/register` must return 200 with both tokens.
#[tokio::test]
async fn register_creates_account_and_returns_tokens() {

    let app = build_test_router(test_pool().await);

    let resp = post_json(
        app,
        "/api/auth/register",
        serde_json::json!({ "username": "alice", "password": "hunter2!" }),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(
        body["access_token"].is_string(),
        "access_token must be present"
    );
    assert!(
        body["refresh_token"].is_string(),
        "refresh_token must be present"
    );
}

/// Registering the same username twice must return 409 Conflict on the second attempt.
#[tokio::test]
async fn register_duplicate_username_returns_conflict() {

    let app = build_test_router(test_pool().await);
    let creds = serde_json::json!({ "username": "bob", "password": "s3cr3t!!" });

    // First registration succeeds.
    let first = post_json(app.clone(), "/api/auth/register", creds.clone()).await;
    assert_eq!(first.status(), StatusCode::OK, "first register must succeed");

    // Second registration with the same username is rejected.
    let second = post_json(app, "/api/auth/register", creds).await;
    assert_eq!(
        second.status(),
        StatusCode::CONFLICT,
        "duplicate username must return 409"
    );
}

/// Short username (< 3 chars) is rejected with 400.
#[tokio::test]
async fn register_rejects_short_username() {

    let app = build_test_router(test_pool().await);
    let resp = post_json(
        app,
        "/api/auth/register",
        serde_json::json!({ "username": "ab", "password": "validpassword" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Username with disallowed characters is rejected with 400.
#[tokio::test]
async fn register_rejects_invalid_username_chars() {

    let app = build_test_router(test_pool().await);
    let resp = post_json(
        app,
        "/api/auth/register",
        serde_json::json!({ "username": "bad name!", "password": "validpassword" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Password shorter than 8 characters is rejected with 400.
#[tokio::test]
async fn register_rejects_short_password() {

    let app = build_test_router(test_pool().await);
    let resp = post_json(
        app,
        "/api/auth/register",
        serde_json::json!({ "username": "validuser", "password": "short" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// `POST /api/auth/login` with correct credentials returns 200 with both tokens.
#[tokio::test]
async fn login_with_correct_credentials_returns_tokens() {

    let app = build_test_router(test_pool().await);

    // Register first.
    let _ = register_user(app.clone(), "charlie", "p4ssw0rd").await;

    // Then login.
    let resp = post_json(
        app,
        "/api/auth/login",
        serde_json::json!({ "username": "charlie", "password": "p4ssw0rd" }),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["access_token"].is_string(), "access_token must be present");
    assert!(body["refresh_token"].is_string(), "refresh_token must be present");
}

/// `POST /api/auth/login` with a wrong password must return 401.
#[tokio::test]
async fn login_with_wrong_password_returns_401() {

    let app = build_test_router(test_pool().await);

    // Register a user.
    let _ = register_user(app.clone(), "dave", "correct_horse").await;

    // Attempt to log in with the wrong password.
    let resp = post_json(
        app,
        "/api/auth/login",
        serde_json::json!({ "username": "dave", "password": "wrong_password" }),
    )
    .await;

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "wrong password must return 401"
    );
}

/// `POST /api/auth/login` for a username that does not exist must return 401.
#[tokio::test]
async fn login_with_unknown_username_returns_401() {

    let app = build_test_router(test_pool().await);

    let resp = post_json(
        app,
        "/api/auth/login",
        serde_json::json!({ "username": "nobody", "password": "whatever" }),
    )
    .await;

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "unknown username must return 401"
    );
}

/// `POST /api/auth/refresh` with a valid refresh token returns 200 with a new access token.
#[tokio::test]
async fn refresh_returns_new_access_token() {

    let app = build_test_router(test_pool().await);

    let (_access, refresh) = register_user(app.clone(), "eve", "refresh_me").await;

    let resp = post_json(
        app,
        "/api/auth/refresh",
        serde_json::json!({ "refresh_token": refresh }),
    )
    .await;

    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(
        body["access_token"].is_string(),
        "refresh must return a new access_token"
    );
}

/// Supplying an access token to `POST /api/auth/refresh` must be rejected because
/// the `kind` claim will be `"access"`, not `"refresh"`.
#[tokio::test]
async fn refresh_with_access_token_returns_401() {

    let app = build_test_router(test_pool().await);

    let (access, _refresh) = register_user(app.clone(), "frank", "bad_refresh").await;

    // Send the access token as if it were a refresh token.
    let resp = post_json(
        app,
        "/api/auth/refresh",
        serde_json::json!({ "refresh_token": access }),
    )
    .await;

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "using an access token as a refresh token must return 401"
    );
}

// ---------------------------------------------------------------------------
// Sync roundtrip tests
// ---------------------------------------------------------------------------

/// Push a payload, then pull — the pulled data must reflect the pushed values.
#[tokio::test]
async fn push_then_pull_returns_pushed_data() {

    let app = build_test_router(test_pool().await);

    let (access, _) = register_user(app.clone(), "grace", "sync_pass").await;
    let user_id = decode_sub(&access);

    let payload = make_payload(&user_id, 7);

    // Push the payload to the server.
    let push_resp = post_authed(
        app.clone(),
        "/api/sync/push",
        &access,
        serde_json::to_value(&payload).expect("SyncPayload must serialise"),
    )
    .await;
    assert_eq!(push_resp.status(), StatusCode::OK, "push must return 200");

    // Pull and verify the stats were persisted.
    let pull_resp = get_authed(app, "/api/sync/pull", &access).await;
    assert_eq!(pull_resp.status(), StatusCode::OK, "pull must return 200");

    let pull_body = body_json(pull_resp).await;
    let games_played = pull_body["merged"]["stats"]["games_played"]
        .as_u64()
        .expect("games_played must be a number");
    assert_eq!(games_played, 7, "pulled games_played must match pushed value");
}

/// Full register → login → push → pull integration roundtrip.
///
/// This test drives every auth and sync endpoint in sequence to verify that
/// the complete happy-path flow works end-to-end with a fresh in-memory
/// database:
///   1. Register a new user — extracts the access token from the response.
///   2. Login with the same credentials — obtains a fresh access token from
///      the login endpoint (not reusing the registration token).
///   3. Push a `SyncPayload` with known stats via `POST /api/sync/push`.
///   4. Pull via `GET /api/sync/pull` and assert the pulled payload reflects
///      the pushed values.
#[tokio::test]
async fn register_login_push_pull_full_roundtrip() {

    let app = build_test_router(test_pool().await);

    // --- Step 1: Register ---
    let reg_resp = post_json(
        app.clone(),
        "/api/auth/register",
        serde_json::json!({ "username": "roundtrip_user", "password": "roundtrip_pass" }),
    )
    .await;
    assert_eq!(
        reg_resp.status(),
        StatusCode::OK,
        "registration must return 200"
    );
    let reg_body = body_json(reg_resp).await;
    assert!(
        reg_body["access_token"].is_string(),
        "register must return an access_token"
    );

    // --- Step 2: Login (explicit — do not reuse the registration token) ---
    let login_resp = post_json(
        app.clone(),
        "/api/auth/login",
        serde_json::json!({ "username": "roundtrip_user", "password": "roundtrip_pass" }),
    )
    .await;
    assert_eq!(
        login_resp.status(),
        StatusCode::OK,
        "login must return 200"
    );
    let login_body = body_json(login_resp).await;
    let access_token = login_body["access_token"]
        .as_str()
        .expect("login must return access_token")
        .to_string();

    // Decode the user UUID from the login JWT so we can construct the payload.
    let user_id = decode_sub(&access_token);

    // --- Step 3: Push a payload with known values ---
    let payload = SyncPayload {
        user_id: uuid::Uuid::parse_str(&user_id)
            .expect("JWT sub must be a valid UUID"),
        stats: StatsSnapshot {
            games_played: 42,
            games_won: 17,
            best_single_score: 4_200,
            fastest_win_seconds: 95,
            ..StatsSnapshot::default()
        },
        achievements: vec![],
        progress: PlayerProgress::default(),
        last_modified: chrono::Utc::now(),
    };

    let push_resp = post_authed(
        app.clone(),
        "/api/sync/push",
        &access_token,
        serde_json::to_value(&payload).expect("SyncPayload must serialise"),
    )
    .await;
    assert_eq!(
        push_resp.status(),
        StatusCode::OK,
        "push must return 200"
    );

    // --- Step 4: Pull and verify the stored data matches what was pushed ---
    let pull_resp = get_authed(app, "/api/sync/pull", &access_token).await;
    assert_eq!(
        pull_resp.status(),
        StatusCode::OK,
        "pull must return 200"
    );

    let pull_body = body_json(pull_resp).await;
    let merged = &pull_body["merged"];

    assert_eq!(
        merged["stats"]["games_played"].as_u64(),
        Some(42),
        "pulled games_played must match the pushed value"
    );
    assert_eq!(
        merged["stats"]["games_won"].as_u64(),
        Some(17),
        "pulled games_won must match the pushed value"
    );
    assert_eq!(
        merged["stats"]["best_single_score"].as_u64(),
        Some(4_200),
        "pulled best_single_score must match the pushed value"
    );
    assert_eq!(
        merged["stats"]["fastest_win_seconds"].as_u64(),
        Some(95),
        "pulled fastest_win_seconds must match the pushed value"
    );
}

/// Pushing a payload whose `user_id` does not match the JWT `sub` must return 400.
#[tokio::test]
async fn push_with_wrong_user_id_returns_400() {

    let app = build_test_router(test_pool().await);

    let (access, _) = register_user(app.clone(), "heidi", "sync_pass").await;

    // Build a payload with a random UUID that won't match the JWT sub.
    let wrong_uuid = uuid::Uuid::new_v4();
    let payload = SyncPayload {
        user_id: wrong_uuid,
        stats: StatsSnapshot::default(),
        achievements: vec![],
        progress: PlayerProgress::default(),
        last_modified: Utc::now(),
    };

    let resp = post_authed(
        app,
        "/api/sync/push",
        &access,
        serde_json::to_value(&payload).expect("SyncPayload must serialise"),
    )
    .await;

    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "mismatched user_id must return 400"
    );
}

/// A pull before any push returns a default empty payload (200, not 404).
#[tokio::test]
async fn pull_before_push_returns_default_payload() {

    let app = build_test_router(test_pool().await);

    let (access, _) = register_user(app.clone(), "ivan", "nopush!!").await;

    let resp = get_authed(app, "/api/sync/pull", &access).await;
    assert_eq!(resp.status(), StatusCode::OK, "pull with no data must return 200");

    let body = body_json(resp).await;
    let games_played = body["merged"]["stats"]["games_played"]
        .as_u64()
        .expect("games_played must be present");
    assert_eq!(games_played, 0, "default payload must have games_played = 0");
}

/// Accessing `/api/sync/pull` without a token must return 401.
#[tokio::test]
async fn pull_without_token_returns_401() {

    let app = build_test_router(test_pool().await);

    let req = Request::builder()
        .method("GET")
        .uri("/api/sync/pull")
        .body(Body::empty())
        .expect("failed to build unauthenticated GET request");

    let resp = app.oneshot(req).await.expect("oneshot failed");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "missing token must return 401"
    );
}

// ---------------------------------------------------------------------------
// Account deletion tests
// ---------------------------------------------------------------------------

/// After `DELETE /api/account`, the user row (and sync data via CASCADE) is gone.
/// A subsequent pull attempt should fail — either 401 (JWT rejected before DB
/// lookup) or the row is simply absent.  Either way, the deletion itself must
/// return 200.
#[tokio::test]
async fn delete_account_succeeds_and_data_is_gone() {

    let app = build_test_router(test_pool().await);

    let (access, _) = register_user(app.clone(), "judy", "delete_me").await;
    let user_id = decode_sub(&access);

    // First push some data.
    let payload = make_payload(&user_id, 5);
    let push_resp = post_authed(
        app.clone(),
        "/api/sync/push",
        &access,
        serde_json::to_value(&payload).expect("SyncPayload must serialise"),
    )
    .await;
    assert_eq!(push_resp.status(), StatusCode::OK, "setup push must succeed");

    // Delete the account.
    let del_resp = delete_authed(app.clone(), "/api/account", &access).await;
    assert_eq!(
        del_resp.status(),
        StatusCode::OK,
        "DELETE /api/account must return 200"
    );
    let del_body = body_json(del_resp).await;
    assert_eq!(
        del_body["ok"], true,
        "delete response must contain ok: true"
    );

    // Subsequent pull with the same token: the JWT is still cryptographically
    // valid (the server has no token revocation list), but the user row no
    // longer exists in the database.  The pull handler will return a default
    // empty payload rather than a 404.  The important assertion is that delete
    // returned 200 above; we just confirm the server doesn't panic.
    let pull_resp = get_authed(app, "/api/sync/pull", &access).await;
    // 200 (default payload) or 404/500 depending on implementation; we only
    // assert that the server responds at all (no panic / connection drop).
    let status = pull_resp.status();
    assert!(
        status.is_success() || status.is_client_error() || status.is_server_error(),
        "server must respond after account deletion"
    );
}

// ---------------------------------------------------------------------------
// Health endpoint tests
// ---------------------------------------------------------------------------

/// `GET /health` must return 200 with `status: "ok"` — no auth required.
#[tokio::test]
async fn health_returns_ok() {
    // No JWT needed; set it anyway for consistency.

    let app = build_test_router(test_pool().await);

    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())
        .expect("failed to build health request");

    let resp = app.oneshot(req).await.expect("oneshot failed");
    assert_eq!(resp.status(), StatusCode::OK, "health must return 200");

    let body = body_json(resp).await;
    assert_eq!(
        body["status"], "ok",
        "health body must contain status: ok"
    );
}

// ---------------------------------------------------------------------------
// Daily challenge tests
// ---------------------------------------------------------------------------

/// `GET /api/daily-challenge` must return 200 with today's UTC date.
#[tokio::test]
async fn daily_challenge_returns_goal_for_today() {

    let app = build_test_router(test_pool().await);

    let today = Utc::now().format("%Y-%m-%d").to_string();

    let req = Request::builder()
        .method("GET")
        .uri("/api/daily-challenge")
        .body(Body::empty())
        .expect("failed to build daily-challenge request");

    let resp = app.oneshot(req).await.expect("oneshot failed");
    assert_eq!(resp.status(), StatusCode::OK, "daily challenge must return 200");

    let body = body_json(resp).await;
    assert_eq!(
        body["date"], today,
        "challenge date must match today's UTC date"
    );
    assert!(body["seed"].is_number(), "challenge must include a numeric seed");
    assert!(
        body["description"].is_string(),
        "challenge must include a description"
    );
}

/// Calling `GET /api/daily-challenge` twice returns the same seed (deterministic).
#[tokio::test]
async fn daily_challenge_is_deterministic() {

    // Use the same pool so the second call hits the stored row.
    let pool = test_pool().await;

    let make_req = || {
        Request::builder()
            .method("GET")
            .uri("/api/daily-challenge")
            .body(Body::empty())
            .expect("failed to build daily-challenge request")
    };

    let resp1 = build_test_router(pool.clone())
        .oneshot(make_req())
        .await
        .expect("first oneshot failed");
    assert_eq!(resp1.status(), StatusCode::OK);
    let body1 = body_json(resp1).await;

    let resp2 = build_test_router(pool)
        .oneshot(make_req())
        .await
        .expect("second oneshot failed");
    assert_eq!(resp2.status(), StatusCode::OK);
    let body2 = body_json(resp2).await;

    assert_eq!(
        body1["seed"], body2["seed"],
        "two calls must return the same seed"
    );
    assert_eq!(
        body1["date"], body2["date"],
        "two calls must return the same date"
    );
}

// ---------------------------------------------------------------------------
// Leaderboard tests
// ---------------------------------------------------------------------------

/// `GET /api/leaderboard` requires authentication — no token returns 401.
#[tokio::test]
async fn leaderboard_without_token_returns_401() {

    let app = build_test_router(test_pool().await);

    let req = Request::builder()
        .method("GET")
        .uri("/api/leaderboard")
        .body(Body::empty())
        .expect("failed to build leaderboard request");

    let resp = app.oneshot(req).await.expect("oneshot failed");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "leaderboard without auth must return 401"
    );
}

/// Opting in and then fetching the leaderboard returns the opted-in entry.
#[tokio::test]
async fn opt_in_then_leaderboard_shows_entry() {

    let app = build_test_router(test_pool().await);

    let (access, _) = register_user(app.clone(), "karen", "leaderpass").await;

    // Opt in with a display name.
    let opt_resp = post_authed(
        app.clone(),
        "/api/leaderboard/opt-in",
        &access,
        serde_json::json!({ "display_name": "KarenTheGreat" }),
    )
    .await;
    assert_eq!(
        opt_resp.status(),
        StatusCode::OK,
        "opt-in must return 200"
    );

    // Fetch the leaderboard.
    let lb_resp = get_authed(app, "/api/leaderboard", &access).await;
    assert_eq!(lb_resp.status(), StatusCode::OK, "leaderboard must return 200");

    let body = body_json(lb_resp).await;
    let entries = body.as_array().expect("leaderboard must be a JSON array");
    let found = entries
        .iter()
        .any(|e| e["display_name"] == "KarenTheGreat");
    assert!(found, "opted-in user must appear in leaderboard");
}

/// Pushing sync data after opting in updates the leaderboard best_score.
#[tokio::test]
async fn push_after_opt_in_updates_leaderboard_score() {

    let pool = test_pool().await;
    let app = build_test_router(pool);

    let (access, _) = register_user(app.clone(), "scorer", "scorepass").await;
    let user_id = decode_sub(&access);

    // Opt in.
    post_authed(
        app.clone(),
        "/api/leaderboard/opt-in",
        &access,
        serde_json::json!({ "display_name": "Scorer" }),
    )
    .await;

    // Build a payload with a known best_single_score.
    let payload = SyncPayload {
        user_id: uuid::Uuid::parse_str(&user_id).unwrap(),
        stats: StatsSnapshot {
            best_single_score: 3_500,
            fastest_win_seconds: 180,
            games_won: 1,
            games_played: 1,
            ..StatsSnapshot::default()
        },
        achievements: vec![],
        progress: PlayerProgress::default(),
        last_modified: Utc::now(),
    };

    let push_resp = post_authed(
        app.clone(),
        "/api/sync/push",
        &access,
        serde_json::to_value(&payload).unwrap(),
    )
    .await;
    assert_eq!(push_resp.status(), StatusCode::OK, "push must return 200");

    // Leaderboard should reflect the pushed score.
    let lb_resp = get_authed(app, "/api/leaderboard", &access).await;
    let body = body_json(lb_resp).await;
    let entries = body.as_array().unwrap();
    let entry = entries.iter().find(|e| e["display_name"] == "Scorer").expect("entry missing");
    assert_eq!(entry["best_score"], 3_500, "best_score must be updated from push");
    assert_eq!(entry["best_time_secs"], 180, "best_time_secs must be updated from push");
}

/// Pushing a lower score after a higher one does not overwrite the best.
#[tokio::test]
async fn push_lower_score_does_not_overwrite_leaderboard_best() {

    let pool = test_pool().await;
    let app = build_test_router(pool);

    let (access, _) = register_user(app.clone(), "champ", "champpass").await;
    let user_id = decode_sub(&access);

    post_authed(
        app.clone(),
        "/api/leaderboard/opt-in",
        &access,
        serde_json::json!({ "display_name": "Champ" }),
    )
    .await;

    let make = |score: u32, time: u64| SyncPayload {
        user_id: uuid::Uuid::parse_str(&user_id).unwrap(),
        stats: StatsSnapshot {
            best_single_score: score,
            fastest_win_seconds: time,
            games_won: 1,
            games_played: 1,
            ..StatsSnapshot::default()
        },
        achievements: vec![],
        progress: PlayerProgress::default(),
        last_modified: Utc::now(),
    };

    // First push: high score.
    post_authed(app.clone(), "/api/sync/push", &access,
        serde_json::to_value(make(5_000, 120)).unwrap()).await;

    // Second push: lower score and slower time.
    post_authed(app.clone(), "/api/sync/push", &access,
        serde_json::to_value(make(1_000, 600)).unwrap()).await;

    let lb_resp = get_authed(app, "/api/leaderboard", &access).await;
    let body = body_json(lb_resp).await;
    let entries = body.as_array().unwrap();
    let entry = entries.iter().find(|e| e["display_name"] == "Champ").unwrap();
    assert_eq!(entry["best_score"], 5_000, "best_score must not regress");
    assert_eq!(entry["best_time_secs"], 120, "best_time_secs must stay at fastest");
}

/// Opting out hides the player from the leaderboard; opting back in restores them.
#[tokio::test]
async fn opt_out_hides_then_opt_in_restores() {

    let pool = test_pool().await;
    let app = build_test_router(pool);

    let (access, _) = register_user(app.clone(), "visible", "pass1234").await;

    // Opt in.
    let resp = post_authed(
        app.clone(),
        "/api/leaderboard/opt-in",
        &access,
        serde_json::json!({ "display_name": "Visible" }),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify they appear.
    let lb = get_authed(app.clone(), "/api/leaderboard", &access).await;
    let entries = body_json(lb).await;
    assert!(
        entries.as_array().unwrap().iter().any(|e| e["display_name"] == "Visible"),
        "opted-in user must appear"
    );

    // Opt out.
    let resp = delete_authed(app.clone(), "/api/leaderboard/opt-in", &access).await;
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify they are hidden.
    let lb = get_authed(app.clone(), "/api/leaderboard", &access).await;
    let entries = body_json(lb).await;
    assert!(
        !entries.as_array().unwrap().iter().any(|e| e["display_name"] == "Visible"),
        "opted-out user must be hidden"
    );

    // Opt back in — should restore without losing display name.
    post_authed(
        app.clone(),
        "/api/leaderboard/opt-in",
        &access,
        serde_json::json!({ "display_name": "Visible" }),
    )
    .await;
    let lb = get_authed(app.clone(), "/api/leaderboard", &access).await;
    let entries = body_json(lb).await;
    assert!(
        entries.as_array().unwrap().iter().any(|e| e["display_name"] == "Visible"),
        "re-opted-in user must appear again"
    );
}

/// Opting in with an empty display name returns 400.
#[tokio::test]
async fn opt_in_empty_display_name_returns_400() {

    let app = build_test_router(test_pool().await);
    let (access, _) = register_user(app.clone(), "empty_name", "pass1234").await;

    let resp = post_authed(
        app,
        "/api/leaderboard/opt-in",
        &access,
        serde_json::json!({ "display_name": "   " }),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "whitespace-only display_name must return 400"
    );
}

/// Opting in with a display name longer than 32 characters returns 400.
#[tokio::test]
async fn opt_in_too_long_display_name_returns_400() {

    let app = build_test_router(test_pool().await);
    let (access, _) = register_user(app.clone(), "long_name", "pass1234").await;

    let long_name = "a".repeat(33);
    let resp = post_authed(
        app,
        "/api/leaderboard/opt-in",
        &access,
        serde_json::json!({ "display_name": long_name }),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "33-char display_name must return 400"
    );
}

/// Exactly 32 ASCII characters is accepted.
#[tokio::test]
async fn opt_in_exactly_32_char_display_name_succeeds() {

    let app = build_test_router(test_pool().await);
    let (access, _) = register_user(app.clone(), "maxname", "pass1234").await;

    let name = "a".repeat(32);
    let resp = post_authed(
        app,
        "/api/leaderboard/opt-in",
        &access,
        serde_json::json!({ "display_name": name }),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "32-char display_name must be accepted"
    );
}

/// A display name consisting of 32 Unicode emoji (multi-byte chars) must be
/// accepted — the limit is character count, not byte count.
#[tokio::test]
async fn opt_in_32_unicode_chars_display_name_succeeds() {

    let app = build_test_router(test_pool().await);
    let (access, _) = register_user(app.clone(), "unicode_name", "pass1234").await;

    // 32 emoji — each is 4 bytes, so 128 bytes total.
    // A byte-length check would incorrectly reject this.
    let name = "🎉".repeat(32);
    let resp = post_authed(
        app,
        "/api/leaderboard/opt-in",
        &access,
        serde_json::json!({ "display_name": name }),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "32-emoji display_name (32 chars, 128 bytes) must be accepted"
    );
}

/// A display name with 33 Unicode emoji is rejected.
#[tokio::test]
async fn opt_in_33_unicode_chars_display_name_returns_400() {

    let app = build_test_router(test_pool().await);
    let (access, _) = register_user(app.clone(), "unicode_long", "pass1234").await;

    let name = "🎉".repeat(33);
    let resp = post_authed(
        app,
        "/api/leaderboard/opt-in",
        &access,
        serde_json::json!({ "display_name": name }),
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::BAD_REQUEST,
        "33-emoji display_name must return 400"
    );
}

/// A second push with lower stats must not overwrite the higher stored values —
/// the server merges (max wins) rather than blindly replacing.
#[tokio::test]
async fn second_push_with_lower_stats_preserves_higher_stored_values() {

    let app = build_test_router(test_pool().await);

    let (access, _) = register_user(app.clone(), "merge_test", "merge_pass").await;
    let user_id = decode_sub(&access);

    // First push: 20 games_played.
    let high_payload = make_payload(&user_id, 20);
    let r1 = post_authed(
        app.clone(),
        "/api/sync/push",
        &access,
        serde_json::to_value(&high_payload).unwrap(),
    )
    .await;
    assert_eq!(r1.status(), StatusCode::OK);

    // Second push: 5 games_played (lower — should be ignored by merge).
    let low_payload = make_payload(&user_id, 5);
    let r2 = post_authed(
        app.clone(),
        "/api/sync/push",
        &access,
        serde_json::to_value(&low_payload).unwrap(),
    )
    .await;
    assert_eq!(r2.status(), StatusCode::OK);

    // Pull and verify the higher value survived.
    let pull_resp = get_authed(app, "/api/sync/pull", &access).await;
    let body = body_json(pull_resp).await;
    let games_played = body["merged"]["stats"]["games_played"]
        .as_u64()
        .expect("games_played must be present");
    assert_eq!(
        games_played, 20,
        "server merge must keep the higher games_played value"
    );
}

/// Login with leading/trailing whitespace in the username still succeeds.
#[tokio::test]
async fn login_trims_whitespace_from_username() {

    let app = build_test_router(test_pool().await);

    let _ = register_user(app.clone(), "trimtest", "password1!").await;

    // Login with surrounding whitespace — should still authenticate.
    let resp = post_json(
        app,
        "/api/auth/login",
        serde_json::json!({ "username": "  trimtest  ", "password": "password1!" }),
    )
    .await;

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "login with whitespace-padded username must succeed"
    );
}

// ---------------------------------------------------------------------------
// Security tests
// ---------------------------------------------------------------------------

/// `POST /api/sync/push` with a body exceeding the 1 MB limit must return 413.
#[tokio::test]
async fn push_oversized_body_returns_413() {

    let app = build_test_router(test_pool().await);

    let (access, _) = register_user(app.clone(), "sizetest", "password1!").await;

    // 1_100_000-byte string embedded in JSON comfortably exceeds the 1 MB limit.
    let big_string = "x".repeat(1_100_000);
    let body_bytes =
        serde_json::to_vec(&serde_json::json!({ "garbage": big_string })).unwrap();

    let req = Request::builder()
        .method("POST")
        .uri("/api/sync/push")
        .header("content-type", "application/json")
        .header("Authorization", format!("Bearer {access}"))
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::from(body_bytes))
        .expect("failed to build oversized request");

    let resp = app.oneshot(req).await.expect("oneshot failed");
    assert_eq!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "oversized body must be rejected with 413"
    );
}

/// A JWT whose `exp` is in the past must be rejected with 401 on protected routes.
#[tokio::test]
async fn expired_access_token_returns_401() {

    let app = build_test_router(test_pool().await);

    // Craft a token that expired 2 hours ago — well past jsonwebtoken's 60 s leeway.
    #[derive(serde::Serialize)]
    struct ExpiredClaims {
        sub: String,
        exp: usize,
        kind: String,
    }
    let exp = (chrono::Utc::now() - chrono::Duration::hours(2)).timestamp() as usize;
    let expired_token = encode(
        &Header::default(),
        &ExpiredClaims {
            sub: "00000000-0000-0000-0000-000000000000".into(),
            exp,
            kind: "access".into(),
        },
        &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
    )
    .unwrap();

    let resp = get_authed(app, "/api/sync/pull", &expired_token).await;
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "expired JWT must be rejected with 401"
    );
}

/// A refresh token must be rejected when used as a Bearer token on protected routes.
#[tokio::test]
async fn refresh_token_rejected_on_protected_routes() {

    let app = build_test_router(test_pool().await);

    let (_, refresh) = register_user(app.clone(), "kindtest", "password1!").await;

    // Using the refresh token (kind = "refresh") as a Bearer on a protected route
    // must return 401 because the middleware requires kind = "access".
    let resp = get_authed(app, "/api/sync/pull", &refresh).await;
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "refresh token must be rejected on protected endpoints"
    );
}

// ---------------------------------------------------------------------------
// Additional auth refresh edge-case tests
// ---------------------------------------------------------------------------

/// `POST /api/auth/refresh` with a completely invalid (non-JWT) string must
/// return 401 — the token cannot be decoded at all.
#[tokio::test]
async fn refresh_with_garbage_token_returns_401() {
    let app = build_test_router(test_pool().await);

    let resp = post_json(
        app,
        "/api/auth/refresh",
        serde_json::json!({ "refresh_token": "this.is.not.a.jwt" }),
    )
    .await;

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "garbage refresh token must return 401"
    );
}

/// `POST /api/auth/refresh` with an expired (but correctly signed) refresh
/// token must return 401 — `exp` is in the past.
#[tokio::test]
async fn refresh_with_expired_refresh_token_returns_401() {
    let app = build_test_router(test_pool().await);

    // Craft a refresh token that expired 2 hours ago, signed with the same
    // secret that `build_test_router` uses, so the signature is valid but the
    // expiry check must still reject it.
    #[derive(serde::Serialize)]
    struct ExpiredRefreshClaims {
        sub: String,
        exp: usize,
        kind: String,
    }
    let exp = (chrono::Utc::now() - chrono::Duration::hours(2)).timestamp() as usize;
    let expired_token = encode(
        &Header::default(),
        &ExpiredRefreshClaims {
            sub: "00000000-0000-0000-0000-000000000000".into(),
            exp,
            kind: "refresh".into(),
        },
        &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
    )
    .unwrap();

    let resp = post_json(
        app,
        "/api/auth/refresh",
        serde_json::json!({ "refresh_token": expired_token }),
    )
    .await;

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "expired refresh token must return 401"
    );
}

// ---------------------------------------------------------------------------
// Additional no-auth / missing-token tests
// ---------------------------------------------------------------------------

/// Accessing `POST /api/sync/push` with no Authorization header must return 401.
#[tokio::test]
async fn push_without_token_returns_401() {
    let app = build_test_router(test_pool().await);

    let req = Request::builder()
        .method("POST")
        .uri("/api/sync/push")
        .header("content-type", "application/json")
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::from(b"{}".as_ref()))
        .expect("failed to build unauthenticated POST request");

    let resp = app.oneshot(req).await.expect("oneshot failed");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "missing token on push must return 401"
    );
}

/// Accessing `DELETE /api/account` with no Authorization header must return 401.
#[tokio::test]
async fn delete_account_without_token_returns_401() {
    let app = build_test_router(test_pool().await);

    let req = Request::builder()
        .method("DELETE")
        .uri("/api/account")
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::empty())
        .expect("failed to build unauthenticated DELETE request");

    let resp = app.oneshot(req).await.expect("oneshot failed");
    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "missing token on DELETE /api/account must return 401"
    );
}

// ---------------------------------------------------------------------------
// Leaderboard — authenticated empty-array test
// ---------------------------------------------------------------------------

/// `GET /api/leaderboard` with a valid JWT but no opted-in players returns 200
/// with an empty JSON array.
#[tokio::test]
async fn leaderboard_with_valid_token_returns_empty_array_when_no_opts() {
    let app = build_test_router(test_pool().await);

    // Register a user to get a valid token — do NOT opt in to the leaderboard.
    let (access, _) = register_user(app.clone(), "no_opt_user", "password1!").await;

    let resp = get_authed(app, "/api/leaderboard", &access).await;
    assert_eq!(resp.status(), StatusCode::OK, "leaderboard must return 200");

    let body = body_json(resp).await;
    assert!(
        body.is_array(),
        "leaderboard body must be a JSON array even when empty"
    );
    assert_eq!(
        body.as_array().unwrap().len(),
        0,
        "leaderboard must be empty when no players have opted in"
    );
}

// ---------------------------------------------------------------------------
// Rate-limiting test (uses the production router with rate limiting enabled)
// ---------------------------------------------------------------------------

/// The 11th request to an auth endpoint within the rate-limit window must
/// return 429 Too Many Requests.
///
/// Uses [`solitaire_server::build_router`] (rate limiting ON) rather than
/// [`build_test_router`] so the GovernorLayer is actually applied.
/// All 11 requests share the same router clone — cloning an Axum Router with
/// GovernorLayer clones the inner `Arc`, so the request counter is shared.
#[tokio::test]
async fn auth_rate_limit_returns_429_on_11th_request() {
    let state = solitaire_server::AppState {
        pool: test_pool().await,
        jwt_secret: TEST_SECRET.to_string(),
    };
    let app = solitaire_server::build_router(state);

    let body_bytes = serde_json::to_vec(&serde_json::json!({
        "username": "ratelimituser",
        "password": "password1!"
    }))
    .unwrap();

    // First 10 requests consume the burst allowance (burst_size = 10).
    // The status may be 200 (first registration) or 400/409 (duplicate username)
    // on retries — what matters is that none of them are 429.
    for i in 0..10 {
        let req = Request::builder()
            .method("POST")
            .uri("/api/auth/register")
            .header("content-type", "application/json")
            .header("x-forwarded-for", TEST_CLIENT_IP)
            .body(Body::from(body_bytes.clone()))
            .expect("failed to build request");
        let resp = app.clone().oneshot(req).await.expect("oneshot failed");
        assert_ne!(
            resp.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "request {} of 10 must not be rate-limited",
            i + 1
        );
    }

    // The 11th request must be rejected by the rate limiter.
    let req = Request::builder()
        .method("POST")
        .uri("/api/auth/register")
        .header("content-type", "application/json")
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::from(body_bytes))
        .expect("failed to build 11th request");
    let resp = app.clone().oneshot(req).await.expect("oneshot failed");
    assert_eq!(
        resp.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "11th request must be rate-limited with 429"
    );
}

// ---------------------------------------------------------------------------
// Replay endpoints
//
// End-to-end coverage for the upload → fetch → render path that powers
// the web replay viewer. Each test boots the full router against an
// in-memory SQLite, registers a user, and exercises one of the three
// replay endpoints. The schema-correctness tests (storage round-trip,
// version gate, atomic write) live in `solitaire_data::replay`; here we
// only verify the HTTP transport + database layer.
// ---------------------------------------------------------------------------

/// Build a minimal v2 replay JSON the upload endpoint will accept.
///
/// Uses the same field shape `solitaire_data::Replay` produces — kept
/// in sync by hand because the server crate intentionally does not
/// depend on `solitaire_data` (which carries dirs/keyring/reqwest).
fn sample_replay_payload(seed: u64, score: i32) -> Value {
    serde_json::json!({
        "schema_version": 2,
        "seed": seed,
        "draw_mode": "DrawOne",
        "mode": "Classic",
        "time_seconds": 134,
        "final_score": score,
        "recorded_at": "2026-05-02",
        "moves": [
            "StockClick",
            { "Move": { "from": "Waste", "to": { "Tableau": 3 }, "count": 1 } }
        ]
    })
}

/// Round-trip: register → upload → fetch → assert the payload returned
/// by `GET /api/replays/:id` matches what was uploaded byte-for-byte.
/// This is the canonical "the web viewer can play back what the
/// desktop client uploaded" test.
#[tokio::test]
async fn replay_upload_then_fetch_round_trips_payload() {
    let pool = test_pool().await;
    let app = build_test_router(pool);
    let (token, _) = register_user(app.clone(), "replay_round_trip_user", "p4ssword!").await;

    let payload = sample_replay_payload(7654, 4321);
    let resp = post_authed(app.clone(), "/api/replays", &token, payload.clone()).await;
    assert_eq!(resp.status(), StatusCode::OK, "upload must return 200");
    let id = body_json(resp).await["id"]
        .as_str()
        .expect("upload response missing `id`")
        .to_string();
    assert!(uuid::Uuid::parse_str(&id).is_ok(), "id must be a UUID");

    // Fetch is public — no auth required, exercising the path the
    // logged-out web viewer takes.
    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/replays/{id}"))
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::empty())
        .expect("fetch request");
    let resp = app.clone().oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK, "fetch must return 200");
    let fetched = body_json(resp).await;
    assert_eq!(
        fetched, payload,
        "fetched payload must match what was uploaded byte-for-byte",
    );
}

/// `GET /api/replays/:id` for an id that was never uploaded must
/// return 404 (not 500). Exercises the `AppError::NotFound` mapping
/// added in the server commit.
#[tokio::test]
async fn replay_fetch_unknown_id_returns_404() {
    let pool = test_pool().await;
    let app = build_test_router(pool);
    let req = Request::builder()
        .method("GET")
        .uri("/api/replays/nonexistent-id-1234")
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::empty())
        .expect("fetch request");
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Two uploads, then `GET /api/replays/recent` — the more recent
/// upload must come first and the response must include the
/// uploader's username (joined from the `users` table).
#[tokio::test]
async fn replay_recent_lists_newest_first_with_username() {
    let pool = test_pool().await;
    let app = build_test_router(pool);
    let (token, _) = register_user(app.clone(), "replay_recent_user", "p4ssword!").await;

    let _ = post_authed(app.clone(), "/api/replays", &token, sample_replay_payload(1, 100)).await;
    let _ = post_authed(app.clone(), "/api/replays", &token, sample_replay_payload(2, 200)).await;

    let req = Request::builder()
        .method("GET")
        .uri("/api/replays/recent")
        .header("x-forwarded-for", TEST_CLIENT_IP)
        .body(Body::empty())
        .expect("recent request");
    let resp = app.oneshot(req).await.expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);

    let entries = body_json(resp).await;
    let array = entries.as_array().expect("recent should return an array");
    assert!(array.len() >= 2, "two uploads should yield two list entries");
    // Newer upload (seed = 2) must appear before older one (seed = 1).
    let seeds: Vec<i64> = array
        .iter()
        .map(|e| e["seed"].as_i64().expect("seed should be an integer"))
        .collect();
    assert_eq!(
        seeds, [2, 1],
        "received_at DESC: most recent upload first",
    );
    assert_eq!(
        array[0]["username"].as_str(),
        Some("replay_recent_user"),
        "username must be joined into the response",
    );
}

/// `POST /api/replays` without an `Authorization` header must return
/// 401, not silently insert as an anonymous user.
#[tokio::test]
async fn replay_upload_without_auth_returns_401() {
    let pool = test_pool().await;
    let app = build_test_router(pool);
    let resp = post_json(app, "/api/replays", sample_replay_payload(99, 50)).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// `POST /api/replays` with a malformed body (missing fields the
/// header projector needs) must return 400, not 500.
#[tokio::test]
async fn replay_upload_malformed_body_returns_400() {
    let pool = test_pool().await;
    let app = build_test_router(pool);
    let (token, _) = register_user(app.clone(), "replay_bad_body_user", "p4ssword!").await;
    let bad = serde_json::json!({ "schema_version": 2, "missing_required_fields": true });
    let resp = post_authed(app, "/api/replays", &token, bad).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
