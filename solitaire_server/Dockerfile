# --- Build stage ---
FROM rust:1.95-slim AS builder

WORKDIR /build

# Install musl tools for a fully static binary and sqlx-cli for compile-time
# query checking (SQLX_OFFLINE=true skips the live-DB check at build time).
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy only the files needed to build the server crate.
# Layer order: workspace manifests first so dependency fetches are cached.
COPY Cargo.toml Cargo.lock ./
COPY solitaire_core/Cargo.toml     ./solitaire_core/Cargo.toml
COPY solitaire_sync/Cargo.toml     ./solitaire_sync/Cargo.toml
COPY solitaire_data/Cargo.toml     ./solitaire_data/Cargo.toml
COPY solitaire_engine/Cargo.toml   ./solitaire_engine/Cargo.toml
COPY solitaire_server/Cargo.toml   ./solitaire_server/Cargo.toml
COPY solitaire_app/Cargo.toml      ./solitaire_app/Cargo.toml
COPY solitaire_assetgen/Cargo.toml ./solitaire_assetgen/Cargo.toml
COPY solitaire_wasm/Cargo.toml     ./solitaire_wasm/Cargo.toml

# Stub every workspace crate so `cargo fetch --locked` resolves the full
# dependency graph without requiring source files beyond Cargo.toml.
RUN for crate in solitaire_core solitaire_sync solitaire_data solitaire_engine \
        solitaire_server solitaire_app solitaire_assetgen solitaire_wasm; do \
      mkdir -p $crate/src && echo "pub fn _stub() {}" > $crate/src/lib.rs; \
    done && \
    echo "fn main() {}" > solitaire_server/src/main.rs && \
    echo "fn main() {}" > solitaire_app/src/main.rs && \
    echo "fn main() {}" > solitaire_assetgen/src/main.rs

RUN cargo fetch --locked

# Now copy real source and build in release mode.
COPY solitaire_core/src ./solitaire_core/src
COPY solitaire_sync/src  ./solitaire_sync/src
COPY solitaire_server/src ./solitaire_server/src
COPY solitaire_server/web ./solitaire_server/web
COPY solitaire_server/migrations ./solitaire_server/migrations
# sqlx offline query cache — required when SQLX_OFFLINE=true so the
# compile-time macros don't need a live database.
COPY .sqlx ./.sqlx

ENV SQLX_OFFLINE=true
RUN cargo build --release --locked -p solitaire_server --bin solitaire_server

# --- Runtime stage ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/solitaire_server ./server
# Migrations are embedded via sqlx::migrate!("./migrations") at compile time.
# Static web assets are served via ServeDir at runtime from these paths:
#   /app/solitaire_server/web  →  /web route
#   /app/assets                →  /assets route
COPY solitaire_server/web ./solitaire_server/web
COPY assets ./assets

ENV SERVER_PORT=8080
EXPOSE 8080

ENTRYPOINT ["./server"]
