# Stage 1 — builder
# Compiles the solitaire_server binary in release mode.
# Requires a pre-generated .sqlx/ query cache (run `cargo sqlx prepare --workspace`
# before building the image so sqlx macros work without a live database).
FROM rust:slim AS builder

WORKDIR /app

RUN apt-get update \
    && apt-get install -y pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY . .

# Tell sqlx to use the cached query metadata instead of a live database.
ENV SQLX_OFFLINE=true

RUN cargo build --release -p solitaire_server

# Stage 2 — runtime
# Minimal image that only contains the compiled binary and its runtime deps.
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y libssl3 ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/solitaire_server /usr/local/bin/solitaire_server

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/solitaire_server"]
