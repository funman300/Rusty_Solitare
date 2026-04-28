# Database Migrations

Migrations are run automatically at server startup via `sqlx::migrate!("./migrations")`.

## Naming convention

```
NNN_description.sql
```

- `NNN` — zero-padded three-digit sequence number (`001`, `002`, …)
- `description` — snake_case description of what the migration does

Examples:
```
001_initial.sql
002_add_user_display_name.sql
003_weekly_goals_table.sql
```

`sqlx` tracks which migrations have run in the `_sqlx_migrations` table and only applies new ones. Never edit or delete an existing migration file after it has been applied to any database — add a new migration instead.

## Adding a migration

1. Create `migrations/NNN_description.sql` where `NNN` is the next available number.
2. Write idempotent SQL (`CREATE TABLE IF NOT EXISTS`, `ALTER TABLE … ADD COLUMN IF NOT EXISTS`, etc.) where possible.
3. Update the sqlx offline query cache so the server builds without a live DB:
   ```bash
   export DATABASE_URL=sqlite://solitaire.db
   sqlx database create
   sqlx migrate run --source solitaire_server/migrations
   cargo sqlx prepare --workspace
   ```
4. Commit both the migration file and the updated `.sqlx/` query cache together.

## Current schema

See `001_initial.sql` for the full initial schema: `users`, `sync_state`, `daily_challenges`, `leaderboard`.
