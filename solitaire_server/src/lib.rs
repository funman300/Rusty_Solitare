//! Solitaire Quest sync server library.
//!
//! Exposes [`build_router`] so integration tests can construct the full Axum
//! application against an in-memory SQLite database without starting a real
//! TCP listener.

pub mod auth;
pub mod challenge;
pub mod error;
pub mod leaderboard;
pub mod middleware;
pub mod replays;
pub mod sync;

pub use auth::reset_password;

use axum::{
    extract::DefaultBodyLimit,
    middleware as axum_middleware,
    response::Html,
    routing::{delete, get, post},
    Router,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use sqlx::SqlitePool;
use std::sync::Arc;
use tower_governor::{
    errors::GovernorError,
    governor::GovernorConfigBuilder,
    key_extractor::{KeyExtractor, SmartIpKeyExtractor},
    GovernorLayer,
};
use tower_http::services::ServeDir;

/// Rate-limiting key extractor for authenticated endpoints.
///
/// Extracts the authenticated user's UUID from the `Authorization: Bearer` JWT
/// so each user gets their own bucket. Falls back to the client IP address when
/// the header is absent or the token fails signature verification — this
/// protects the server from unauthenticated request floods while ensuring
/// legitimate users are always identified by identity rather than IP.
///
/// Expiry is intentionally **not** checked here: `require_auth` validates the
/// full token (including `exp`) and returns 401. Counting an expired token
/// against the user's bucket is harmless and avoids returning 500 (the
/// `UnableToExtractKey` outcome) for a request that would get 401 anyway.
#[derive(Clone)]
struct UserIdKeyExtractor {
    jwt_secret: String,
}

impl KeyExtractor for UserIdKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &axum::http::Request<T>) -> Result<Self::Key, GovernorError> {
        if let Some(user_id) = self.try_extract_user_id(req.headers()) {
            return Ok(user_id);
        }
        // Fall back to IP so unauthenticated bursts don't bypass throttling.
        SmartIpKeyExtractor
            .extract(req)
            .map(|ip| ip.to_string())
    }
}

impl UserIdKeyExtractor {
    fn try_extract_user_id(&self, headers: &axum::http::HeaderMap) -> Option<String> {
        let value = headers.get("Authorization")?.to_str().ok()?;
        let token = value.strip_prefix("Bearer ")?;
        let key = DecodingKey::from_secret(self.jwt_secret.as_bytes());
        let mut validation = Validation::default();
        validation.validate_exp = false;
        decode::<middleware::Claims>(token, &key, &validation)
            .ok()
            .map(|d| d.claims.sub)
    }
}

/// Shared application state injected into every Axum handler via [`axum::extract::State`].
///
/// Loaded once at startup so a missing `JWT_SECRET` causes an immediate startup
/// failure rather than a 500 error on the first request.
#[derive(Clone)]
pub struct AppState {
    /// SQLite connection pool.
    pub pool: SqlitePool,
    /// HS256 signing secret for JWT access and refresh tokens.
    pub jwt_secret: String,
}

/// Construct the full Axum [`Router`].
///
/// Separated from `main` so it can be instantiated in integration tests without
/// starting a real TCP listener.
pub fn build_router(state: AppState) -> Router {
    build_router_inner(state, true)
}

/// Construct the router without rate limiting.
///
/// Intended for integration tests only — do not use in production.
/// Create an in-memory SQLite pool and run all pending migrations.
///
/// `max_connections(1)` is required for SQLite in-memory databases: every
/// additional connection sees an empty schema.
///
/// Exposed so integration tests in other crates (e.g. `solitaire_data`) can
/// boot a real server without duplicating the migration boilerplate.
#[doc(hidden)]
pub async fn build_test_pool() -> SqlitePool {
    use sqlx::sqlite::SqlitePoolOptions;
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

/// Uses a fixed test JWT secret (`"test_secret_32_chars_minimum_ok!"`) so
/// integration tests do not need to set `JWT_SECRET` in the environment.
#[doc(hidden)]
pub fn build_test_router(pool: SqlitePool) -> Router {
    let state = AppState {
        pool,
        jwt_secret: "test_secret_32_chars_minimum_ok!".to_string(),
    };
    build_router_inner(state, false)
}

fn build_router_inner(state: AppState, rate_limit: bool) -> Router {
    // Protected routes require a valid JWT (injected by require_auth middleware).
    let protected_base = Router::new()
        .route("/api/sync/pull", get(sync::pull))
        .route("/api/sync/push", post(sync::push))
        .route("/api/replays", post(replays::upload))
        .route("/api/leaderboard", get(leaderboard::get_leaderboard))
        .route("/api/leaderboard/opt-in", post(leaderboard::opt_in))
        .route("/api/leaderboard/opt-in", delete(leaderboard::opt_out))
        .route("/api/account", delete(auth::delete_account))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            middleware::require_auth,
        ));

    // Per-user rate limit on protected endpoints: 10-request burst, then 1
    // token replenished every 10 seconds (6/min steady-state). This prevents
    // a single compromised account from hammering the 1 MB sync/push endpoint.
    let protected = if rate_limit {
        let governor_conf = Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(UserIdKeyExtractor {
                    jwt_secret: state.jwt_secret.clone(),
                })
                .per_second(10)
                .burst_size(10)
                .finish()
                .expect("invalid sync governor config"),
        );
        protected_base.layer(GovernorLayer::new(governor_conf))
    } else {
        protected_base
    };

    // Auth endpoints — rate-limited in production, unrestricted in tests.
    let auth_routes = Router::new()
        .route("/api/auth/register", post(auth::register))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/refresh", post(auth::refresh));

    let auth_routes = if rate_limit {
        // Rate limiter: 10 requests per minute per IP.
        // burst_size = 10, replenish every 6 seconds = 10/min steady-state.
        let governor_conf = Arc::new(
            GovernorConfigBuilder::default()
                .key_extractor(SmartIpKeyExtractor)
                .per_second(6)
                .burst_size(10)
                .finish()
                .expect("invalid governor config"),
        );
        auth_routes.layer(GovernorLayer::new(governor_conf))
    } else {
        auth_routes
    };

    // Public endpoints (no auth, no rate limit beyond defaults).
    let public = Router::new()
        .route("/api/daily-challenge", get(challenge::daily_challenge))
        .route("/api/replays/recent", get(replays::recent))
        .route("/api/replays/{id}", get(replays::get_by_id))
        .route("/health", get(health));

    // Replay web UI: a single HTML page served at `/replays/:id` plus a
    // ServeDir for the static assets (`web/index.html`, `web/replay.css`,
    // and the wasm-bindgen-generated `web/pkg/`). The HTML page is the
    // same regardless of `:id` — it reads the path from `location` in JS
    // and fetches the replay JSON from `/api/replays/:id`.
    let web = Router::new()
        .route(
            "/replays/{id}",
            get(|| async { Html(include_str!("../web/index.html")) }),
        )
        .nest_service("/web", ServeDir::new("solitaire_server/web"));

    Router::new()
        .merge(protected)
        .merge(auth_routes)
        .merge(public)
        .merge(web)
        // Reject request bodies larger than 1 MB.
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(state)
}

/// `GET /health` — simple liveness probe, no auth required.
async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
