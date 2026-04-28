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
pub mod sync;

use axum::{
    extract::DefaultBodyLimit,
    middleware as axum_middleware,
    routing::{delete, get, post},
    Router,
};
use sqlx::SqlitePool;
use std::sync::Arc;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};

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
    let protected = Router::new()
        .route("/api/sync/pull", get(sync::pull))
        .route("/api/sync/push", post(sync::push))
        .route("/api/leaderboard", get(leaderboard::get_leaderboard))
        .route("/api/leaderboard/opt-in", post(leaderboard::opt_in))
        .route("/api/leaderboard/opt-in", delete(leaderboard::opt_out))
        .route("/api/account", delete(auth::delete_account))
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            middleware::require_auth,
        ));

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
        .route("/health", get(health));

    Router::new()
        .merge(protected)
        .merge(auth_routes)
        .merge(public)
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
