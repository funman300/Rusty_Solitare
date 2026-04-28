//! Solitaire Quest sync server entry point.
//!
//! Reads configuration from environment variables (via `dotenvy`), initialises
//! the SQLite database, runs migrations, then starts the Axum HTTP server.
//!
//! ## Required environment variables
//!
//! | Variable       | Description                                       |
//! |----------------|---------------------------------------------------|
//! | `DATABASE_URL` | SQLite connection string, e.g. `sqlite://sol.db`  |
//! | `JWT_SECRET`   | HS256 signing secret (min 32 chars recommended)   |
//!
//! ## Optional
//!
//! | Variable      | Default | Description                   |
//! |---------------|---------|-------------------------------|
//! | `SERVER_PORT` | `8080`  | TCP port to listen on         |

use solitaire_server::{build_router, AppState};
use sqlx::SqlitePool;
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    // Load .env file if present (silently ignored when absent).
    dotenvy::dotenv().ok();

    // Initialise structured logging.
    tracing_subscriber::fmt::init();

    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    // Load JWT_SECRET once at startup — a missing secret is a fatal configuration error.
    let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    let port: u16 = std::env::var("SERVER_PORT")
        .unwrap_or_else(|_| "8080".into())
        .parse()
        .expect("SERVER_PORT must be a valid port number");

    // Connect to SQLite and run pending migrations.
    let pool = SqlitePool::connect(&db_url)
        .await
        .expect("failed to connect to database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("database migration failed");

    tracing::info!("database ready at {db_url}");

    let state = AppState { pool, jwt_secret };
    let app = build_router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind TCP listener");

    axum::serve(listener, app)
        .await
        .expect("server error");
}
