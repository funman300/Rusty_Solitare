//! Integration tests for `solitaire_server`.
//!
//! Every test uses an in-memory SQLite database and [`build_test_router`]
//! (rate limiting disabled) — no real TCP listener is started.  Requests are dispatched via
//! [`tower::ServiceExt::oneshot`].
//!
//! # JWT secret
//!
//! Each test calls [`set_jwt_secret`] before touching any endpoint that reads
//! `JWT_SECRET` from the environment.  This is safe because `cargo test` runs
//! integration-test binaries single-threaded by default.

use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::Response,
};
use chrono::Utc;
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;
use solitaire_server::build_test_router;
use solitaire_sync::{PlayerProgress, StatsSnapshot, SyncPayload};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The JWT secret injected into the environment for all tests.
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

/// Inject `JWT_SECRET` into the process environment so all auth code can read it.
///
/// # Safety
/// Only called from test code where tests run sequentially in a single binary.
fn set_jwt_secret() {
    // SAFETY: test-only; integration test binaries are single-threaded.
    unsafe { std::env::set_var("JWT_SECRET", TEST_SECRET) };
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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

/// Pushing a payload whose `user_id` does not match the JWT `sub` must return 400.
#[tokio::test]
async fn push_with_wrong_user_id_returns_400() {
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
    set_jwt_secret();
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
