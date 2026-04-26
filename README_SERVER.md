# Solitaire Quest — Self-Hosting Guide

## Prerequisites

- Docker and Docker Compose
- `openssl` for generating a JWT secret

## Quick start

1. Clone the repo and enter it.
2. Copy the example environment file and fill in your values:
   ```bash
   cp .env.example .env
   # Edit .env: set JWT_SECRET and SOLITAIRE_DOMAIN
   ```
3. (First time only) Generate the sqlx query cache so the server builds without a live database:
   ```bash
   cargo install sqlx-cli --no-default-features --features rustls,sqlite
   export DATABASE_URL=sqlite://solitaire.db
   sqlx database create
   sqlx migrate run --source solitaire_server/migrations
   cargo sqlx prepare --workspace
   rm solitaire.db   # the real DB lives in ./data/ at runtime
   ```
4. Start everything:
   ```bash
   docker compose up -d
   ```
5. The server is now reachable at `https://<SOLITAIRE_DOMAIN>`.

## Backups

The entire server state is one SQLite file at `./data/solitaire.db`. Back it up with:
```bash
sqlite3 ./data/solitaire.db ".backup backup_$(date +%Y%m%d).db"
```

## Updating

```bash
git pull
docker compose build
docker compose up -d
```
