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


## Admin — Password Reset

If a player loses access to their account, the server binary includes a
built-in password reset command. Run it on the host (or inside the container)
with `DATABASE_URL` pointing at your database:

```bash
# Interactive (prompts for the new password):
DATABASE_URL=sqlite://./data/solitaire.db \
  ./solitaire_server --reset-password <username>

# Non-interactive (piped from a script or password manager):
echo "new_password" | \
  DATABASE_URL=sqlite://./data/solitaire.db \
    ./solitaire_server --reset-password <username>

# Inside a running Docker container:
docker compose exec server sh -c \
  'echo "new_password" | ./solitaire_server --reset-password alice'
```

On success the user's `password_hash` is updated and **all active refresh
tokens are deleted**, so every open session must log in again with the new
password. `JWT_SECRET` does not need to be set for this command.
