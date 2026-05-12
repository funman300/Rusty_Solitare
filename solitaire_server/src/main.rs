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
//!
//! ## Admin subcommands
//!
//! Pass `--reset-password <username>` to reset a player's password instead
//! of starting the HTTP server. The new password is read from stdin (one line).
//! All active sessions for the user are invalidated so the player must log in
//! again with the new password.
//!
//! ```sh
//! # Interactive (password echoed to terminal):
//! ./solitaire_server --reset-password alice
//!
//! # Non-interactive / scripted:
//! echo "new_password" | ./solitaire_server --reset-password alice
//! ```

use solitaire_server::{build_router, AppState};
use sqlx::SqlitePool;
use std::{
    io::{self, BufRead},
    net::SocketAddr,
};

#[tokio::main]
async fn main() {
    // Load .env file if present (silently ignored when absent).
    dotenvy::dotenv().ok();

    // Initialise structured logging.
    tracing_subscriber::fmt::init();

    // Dispatch to admin subcommands before starting the HTTP server.
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--reset-password") {
        let username = args
            .get(pos + 1)
            .expect("--reset-password requires a username argument");
        run_reset_password(username).await;
        return;
    }

    run_server().await;
}

/// Connect to the database, read a new password from stdin, and reset the
/// password for `username`. Exits non-zero on any error.
async fn run_reset_password(username: &str) {
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let pool = SqlitePool::connect(&db_url)
        .await
        .expect("failed to connect to database");

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("database migration failed");

    // Read new password from stdin. Print the prompt to stderr so it doesn't
    // pollute stdout when the caller pipes the output.
    eprint!("New password for '{username}': ");
    let mut new_password = String::new();
    io::stdin()
        .lock()
        .read_line(&mut new_password)
        .expect("failed to read password from stdin");
    let new_password = new_password.trim_end_matches(['\n', '\r']);

    match solitaire_server::reset_password(&pool, username, new_password).await {
        Ok(()) => {
            println!("Password reset for '{username}'. All active sessions invalidated.");
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}

/// Start the HTTP server. Requires `DATABASE_URL`, `JWT_SECRET` (and
/// optionally `SERVER_PORT`) in the environment.
async fn run_server() {
    let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    // Load JWT_SECRET once at startup — a missing secret is a fatal configuration error.
    let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    let port: u16 = std::env::var("SERVER_PORT")
        .unwrap_or_else(|_| "8080".into())
        .parse()
        .expect("SERVER_PORT must be a valid port number");

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
