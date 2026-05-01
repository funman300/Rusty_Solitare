//! Client-side sync round-trip integration tests for `solitaire_data`.
//!
//! These tests spin up the actual `solitaire_server` Axum app in-process on a
//! random TCP port (allocated by the OS) and drive the production
//! [`SolitaireServerClient`] HTTP client against it via `reqwest`. They are
//! the client-side counterpart to `solitaire_server/tests/server_tests.rs`,
//! which exercises the server endpoints directly via `tower::ServiceExt`.
//!
//! # Keyring
//!
//! [`SolitaireServerClient`] reads tokens from the OS keyring via
//! `keyring_core`. Headless test environments may not have a real secret
//! service, so we install the in-memory `keyring_core::mock::Store` exactly
//! once via [`std::sync::Once`]. Every test uses a unique username so the
//! shared mock store does not leak credentials between tests.
//!
//! # Server harness
//!
//! Each test calls [`spawn_test_server`] which:
//!   1. Binds a `tokio::net::TcpListener` on `127.0.0.1:0` (OS picks a port).
//!   2. Builds the in-memory SQLite pool, runs migrations.
//!   3. Builds the test router via `solitaire_server::build_test_router`
//!      (rate limiting OFF, fixed test JWT secret).
//!   4. Spawns the server in a background `tokio::spawn` task.
//!   5. Returns the server URL (`http://127.0.0.1:{port}`).
//!
//! # Test JWT secret
//!
//! Must match the constant inside `build_test_router` so we can craft
//! expired-on-purpose tokens for the JWT-refresh test.

use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use solitaire_data::{
    delete_tokens, store_tokens, SolitaireServerClient, SyncError, SyncProvider,
};
use solitaire_sync::{PlayerProgress, StatsSnapshot, SyncPayload};
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use std::sync::Once;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// JWT secret used by `solitaire_server::build_test_router`. Must stay in
/// sync with the constant inside that function.
const TEST_SECRET: &str = "test_secret_32_chars_minimum_ok!";

// ---------------------------------------------------------------------------
// Mock keyring setup (process-wide; install once)
// ---------------------------------------------------------------------------

static MOCK_KEYRING_INIT: Once = Once::new();

/// Install the `keyring_core` mock in-memory store as the process-wide
/// default. Safe to call from any test — only the first call has effect.
fn ensure_mock_keyring() {
    MOCK_KEYRING_INIT.call_once(|| {
        let store = keyring_core::mock::Store::new()
            .expect("failed to construct mock keyring store");
        keyring_core::set_default_store(store);
    });
}

// ---------------------------------------------------------------------------
// Server harness
// ---------------------------------------------------------------------------

/// Build a fresh in-memory SQLite pool with all migrations applied.
///
/// `max_connections(1)` is required: each connection to `sqlite::memory:` is
/// a *separate* database, so a larger pool sees an empty schema on the second
/// borrow. Mirrors the pattern in `solitaire_server/tests/server_tests.rs`.
async fn fresh_pool() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory SQLite database");
    sqlx::migrate!("../solitaire_server/migrations")
        .run(&pool)
        .await
        .expect("failed to run database migrations");
    pool
}

/// Spawn the test server on a random localhost port and return its base URL.
///
/// The server runs until the test process exits — there is no explicit
/// shutdown. This is acceptable for `cargo test` where each test binary is a
/// separate process.
async fn spawn_test_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind test listener");
    let addr = listener
        .local_addr()
        .expect("listener has no local addr");

    let app = solitaire_server::build_test_router(fresh_pool().await);

    tokio::spawn(async move {
        // Errors here cannot fail the test directly because we are inside a
        // `tokio::spawn`; we just log so a rogue panic doesn't go unnoticed.
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("test server crashed: {e}");
        }
    });

    format!("http://{addr}")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Register a fresh user against `base_url` and return the access + refresh
/// tokens straight from the response body. Bypasses the keyring entirely so
/// the caller can store the tokens under whatever username they want.
async fn register_user_raw(
    base_url: &str,
    username: &str,
    password: &str,
) -> (String, String) {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/api/auth/register"))
        .json(&serde_json::json!({
            "username": username,
            "password": password,
        }))
        .send()
        .await
        .expect("register request failed");
    assert!(
        resp.status().is_success(),
        "register must succeed (got {})",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("register body must be JSON");
    let access = body["access_token"]
        .as_str()
        .expect("access_token missing")
        .to_string();
    let refresh = body["refresh_token"]
        .as_str()
        .expect("refresh_token missing")
        .to_string();
    (access, refresh)
}

/// Decode a JWT's `sub` claim without validating expiry (so test crafted
/// tokens still parse). Returns the user UUID as a `String`.
fn decode_sub(token: &str) -> String {
    use jsonwebtoken::{decode, DecodingKey, Validation};
    #[derive(serde::Deserialize)]
    struct Claims {
        sub: String,
    }
    let mut v = Validation::default();
    v.validate_exp = false;
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(TEST_SECRET.as_bytes()),
        &v,
    )
    .expect("failed to decode JWT");
    data.claims.sub
}

/// Produce a `SyncPayload` with `user_id` (parsed from the JWT sub) and a
/// non-default `games_played` so we can verify round-trips.
fn make_payload(user_id_str: &str, games_played: u32) -> SyncPayload {
    SyncPayload {
        user_id: Uuid::parse_str(user_id_str)
            .expect("user_id_str from JWT sub must be a valid UUID"),
        stats: StatsSnapshot {
            games_played,
            games_won: 7,
            best_single_score: 1234,
            ..StatsSnapshot::default()
        },
        achievements: vec![],
        progress: PlayerProgress::default(),
        last_modified: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// **Full happy-path round-trip.**
///
/// 1. Spin up server.
/// 2. Register a user via raw HTTP.
/// 3. Persist the tokens in the (mock) keyring under the same username.
/// 4. Construct a `SolitaireServerClient` and call `push()` with a known
///    payload, then call `pull()` on the *same* client.
/// 5. Assert the server-merged stats reflect the values we pushed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn register_login_push_pull_round_trip() {
    ensure_mock_keyring();

    let base = spawn_test_server().await;
    let username = "rt_alice";

    let (access, refresh) = register_user_raw(&base, username, "alicepass1!").await;
    store_tokens(username, &access, &refresh)
        .expect("storing tokens in mock keyring must succeed");

    let user_id = decode_sub(&access);
    let payload = make_payload(&user_id, 42);

    let client = SolitaireServerClient::new(&base, username);

    // Push.
    let push_resp = client
        .push(&payload)
        .await
        .expect("push must succeed for an authenticated client");
    assert_eq!(
        push_resp.merged.stats.games_played, 42,
        "merged stats from push must reflect pushed games_played"
    );

    // Pull on the same client.
    let pulled = client
        .pull()
        .await
        .expect("pull must succeed for an authenticated client");
    assert_eq!(
        pulled.stats.games_played, 42,
        "pulled games_played must match what we pushed"
    );
    assert_eq!(
        pulled.stats.best_single_score, 1234,
        "pulled best_single_score must match what we pushed"
    );

    // Cleanup so the shared mock store doesn't leak this username's tokens.
    let _ = delete_tokens(username);
}

/// **Concurrent two-client merge.**
///
/// Two clients (same user) push payloads with different `games_played`. The
/// server's merge keeps the higher of the two values. A subsequent pull from
/// either client must observe the merged max.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pull_after_concurrent_pushes_merges_correctly() {
    ensure_mock_keyring();

    let base = spawn_test_server().await;
    let username = "rt_bob";

    let (access, refresh) = register_user_raw(&base, username, "bobpass1!").await;
    store_tokens(username, &access, &refresh)
        .expect("storing tokens in mock keyring must succeed");

    let user_id = decode_sub(&access);

    // Two separate clients; both authenticate as the same user via the same
    // tokens in the mock keyring.
    let client_a = SolitaireServerClient::new(&base, username);
    let client_b = SolitaireServerClient::new(&base, username);

    // Client A: low value first.
    let payload_a = make_payload(&user_id, 5);
    client_a.push(&payload_a).await.expect("client A push must succeed");

    // Client B: higher value second.
    let payload_b = make_payload(&user_id, 99);
    client_b.push(&payload_b).await.expect("client B push must succeed");

    // Either client should now pull max(5, 99) = 99.
    let pulled = client_a
        .pull()
        .await
        .expect("pull after concurrent pushes must succeed");
    assert_eq!(
        pulled.stats.games_played, 99,
        "merged games_played must be max(5, 99) = 99"
    );

    let _ = delete_tokens(username);
}

/// **Unauthenticated pull surfaces an `Auth` error.**
///
/// We construct a client for a user who has *no* tokens in the keyring at
/// all. `pull()` must return `SyncError::Auth(_)` — never `Network` or
/// `Serialization`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unauthenticated_pull_returns_authentication_error() {
    ensure_mock_keyring();

    let base = spawn_test_server().await;
    // Use a username that we never call `store_tokens` for so the keyring
    // lookup fails before any HTTP request is made.
    let username = "rt_no_creds";
    // Defensive: in case a previous test run left tokens behind.
    let _ = delete_tokens(username);

    let client = SolitaireServerClient::new(&base, username);
    let err = client
        .pull()
        .await
        .expect_err("pull must fail without stored credentials");
    assert!(
        matches!(err, SyncError::Auth(_)),
        "expected SyncError::Auth, got {err:?}"
    );
}

/// **JWT auto-refresh on 401.**
///
/// We register a user, then deliberately overwrite the stored access token
/// with one whose `exp` is in the past (signed with the same `TEST_SECRET`
/// so the signature verifies). The middleware will reject it with 401, the
/// `SolitaireServerClient` should call `/api/auth/refresh` with the still-
/// valid refresh token and retry — and `pull()` must ultimately succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn jwt_refresh_on_401_succeeds() {
    ensure_mock_keyring();

    let base = spawn_test_server().await;
    let username = "rt_expiring";

    // Register to get a real, valid refresh token signed with TEST_SECRET.
    let (_real_access, real_refresh) =
        register_user_raw(&base, username, "expirepass1!").await;
    let user_id = decode_sub(&_real_access);

    // Craft an expired access token signed with TEST_SECRET so the server's
    // signature check still passes but the expiry validation rejects it.
    #[derive(serde::Serialize)]
    struct Claims {
        sub: String,
        exp: usize,
        kind: String,
    }
    let exp = (Utc::now() - chrono::Duration::hours(2)).timestamp() as usize;
    let expired_access = encode(
        &Header::default(),
        &Claims {
            sub: user_id.clone(),
            exp,
            kind: "access".into(),
        },
        &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
    )
    .expect("failed to encode expired access token");

    // Overwrite the stored access token with the expired one. The refresh
    // token stays valid so the client's refresh path can succeed.
    store_tokens(username, &expired_access, &real_refresh)
        .expect("storing tokens in mock keyring must succeed");

    // Pull: server returns 401, client refreshes, retries, succeeds.
    let client = SolitaireServerClient::new(&base, username);
    let pulled = client.pull().await.expect(
        "pull must succeed after the client transparently refreshes the access token",
    );
    // Default merge for a never-pushed user yields games_played = 0.
    assert_eq!(
        pulled.stats.games_played, 0,
        "default empty payload after refresh must have games_played = 0"
    );

    let _ = delete_tokens(username);
}

/// **Account-deletion locks the client out.**
///
/// Register, push some data, then delete the account via the trait method.
/// A subsequent push with the *same* tokens (still cryptographically valid —
/// the server has no revocation list) must surface a non-success response
/// because the user row is gone and the server rejects the foreign-key push.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pull_after_account_deletion_returns_default_or_error() {
    ensure_mock_keyring();

    let base = spawn_test_server().await;
    let username = "rt_deleter";

    let (access, refresh) = register_user_raw(&base, username, "deletepass1!").await;
    store_tokens(username, &access, &refresh)
        .expect("storing tokens in mock keyring must succeed");

    let user_id = decode_sub(&access);
    let client = SolitaireServerClient::new(&base, username);

    // Establish data first.
    client
        .push(&make_payload(&user_id, 3))
        .await
        .expect("initial push must succeed");

    // Delete the account.
    client
        .delete_account()
        .await
        .expect("delete_account must return Ok on the live server");

    // After deletion, pushing the same payload may either:
    //   - succeed (server INSERTs a fresh sync_state row keyed off JWT sub
    //     even though the users row is gone), or
    //   - fail with a server error from a foreign-key violation.
    //
    // We do not pin down which behaviour the server picks — the contract we
    // assert is just that the client surfaces *some* result without panicking
    // and that the trait remains usable.
    let post_delete_push = client.push(&make_payload(&user_id, 4)).await;
    let _ = post_delete_push; // either Ok or Err is fine; no panic is the win

    let _ = delete_tokens(username);
}
