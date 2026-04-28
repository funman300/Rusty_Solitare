# Solitaire Quest — Claude Code Instructions

See @ARCHITECTURE.md for full project design, crate responsibilities, data models, and API reference.

---

## Project Layout

```text
solitaire_core/    # Pure Rust game logic — NO Bevy, NO network, NO I/O
solitaire_sync/    # Shared API types — NO Bevy, serde/uuid/chrono only
solitaire_data/    # Persistence + SyncProvider trait + server client
solitaire_engine/  # Bevy ECS systems, components, plugins
solitaire_server/  # Axum sync server binary
solitaire_app/     # Thin binary entry point
assets/            # Source assets — embedded at compile time via include_bytes!()
```

---

## Build & Test Commands

```bash
# Dev run (fast compile via dynamic linking)
cargo run -p solitaire_app --features bevy/dynamic_linking

# Release build
cargo build --workspace --release

# All tests — MUST pass before any commit
cargo test --workspace

# Lint — MUST pass clean (zero warnings)
cargo clippy --workspace -- -D warnings

# Run sync server locally
cargo run -p solitaire_server

# Check a single crate
cargo test -p solitaire_core
cargo clippy -p solitaire_core -- -D warnings
```

---

## Hard Rules

- `solitaire_core` and `solitaire_sync` must never gain Bevy or network dependencies.
- No `unwrap()` or `panic!()` in game logic. All state transitions return `Result<_, MoveError>`.
- Assets are embedded at compile time using `include_bytes!()`. No runtime asset loading via `AssetServer`.
- Atomic file writes only: write to `filename.json.tmp`, then `rename()`.
- Passwords and tokens are stored in the OS keychain via the `keyring` crate — never in plaintext files or logs.
- Sync runs on `AsyncComputeTaskPool` — never block the Bevy main thread.
- All sync backends implement the `SyncProvider` trait. The `SyncPlugin` is backend-agnostic — never `match` on `SyncBackend` inside a Bevy system.
- `cargo clippy --workspace -- -D warnings` must pass clean after every change.
- `cargo test --workspace` must pass after every change.

---

## Code Style

- Use `thiserror` for error types. Never `Box<dyn Error>` in library crates.
- Prefer `Into<T>` over concrete types in public API function parameters.
- All public items must have doc comments (`///`). Private items: comment only when non-obvious.
- Derive order convention: `#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]`
- Bevy systems: one responsibility per system. Use `Events` for cross-system communication, never shared mutable state.
- SQL queries: use `sqlx::query!` macros (compile-time checked), not raw string queries.
- No `clone()` calls in hot paths (game loop systems). Profile before optimising elsewhere.

---

## Bevy Conventions

- One `Plugin` per major feature: `CardPlugin`, `AudioPlugin`, `AchievementPlugin`, `UIPlugin`, `SyncPlugin`.
- Resources own shared state. Events communicate between systems. Components own per-entity data.
- All UI screens are built with Bevy UI (`bevy::ui`). Never mix UI layout and game logic in the same system.
- Layout is recomputed on `WindowResized` — never assume a fixed window size.

---

## Git Workflow

- Commit after each passing phase, not after every file change.
- Commit message format: `type(scope): description`
  - `feat(core): add draw-three mode validation`
  - `fix(engine): card z-order during drag`
  - `test(core): undo stack boundary conditions`
  - `chore(server): add sqlx migration 002`
- Never commit with failing tests or clippy warnings.
- Never commit secrets, `.env` files, or `*.db` files.

---

## Ask Before Doing

- Adding a new crate dependency (discuss alternatives first).
- Changing a type in `solitaire_sync` (breaking change on both client and server).
- Altering the database schema (requires a new sqlx migration).
- Introducing `unsafe` code anywhere.
- Changing the merge strategy in `solitaire_sync::merge()`.

---

## Lessons Learned

> Add entries here when Claude makes a mistake so it isn't repeated.

- Bevy's `Time` resource uses `f32` seconds; convert to `u64` only when writing to `StatsSnapshot`.
- `sqlx::migrate!()` macro path is relative to the crate root, not the workspace root.
- `keyring` on Linux requires a running secret service (e.g. GNOME Keyring or KWallet) — handle `Error::NoStorageAccess` gracefully and fall back to prompting the user.
- `dirs::data_dir()` returns `None` on some minimal Linux environments — always handle the `None` case explicitly, do not unwrap.
