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
COPY solitaire_sync/Cargo.toml ./solitaire_sync/Cargo.toml
COPY solitaire_server/Cargo.toml ./solitaire_server/Cargo.toml
COPY solitaire_core/Cargo.toml ./solitaire_core/Cargo.toml

# Stub every crate source so `cargo fetch` succeeds without full source.
RUN mkdir -p solitaire_sync/src solitaire_server/src solitaire_core/src && \
    echo "pub fn _stub() {}" > solitaire_sync/src/lib.rs && \
    echo "pub fn _stub() {}" > solitaire_core/src/lib.rs && \
    echo "pub fn _stub() {}" > solitaire_server/src/lib.rs && \
    echo "fn main() {}" > solitaire_server/src/main.rs

RUN cargo fetch --locked

# Now copy real source and build in release mode.
COPY solitaire_core/src ./solitaire_core/src
COPY solitaire_sync/src  ./solitaire_sync/src
COPY solitaire_server/src ./solitaire_server/src
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

COPY --from=builder /build/target/release/solitaire_server ./solitaire_server
# Migrations are embedded via sqlx::migrate!("./migrations") relative to the
# crate root at compile time — they do not need to be copied here.

ENV SERVER_PORT=8080
EXPOSE 8080

ENTRYPOINT ["./solitaire_server"]
