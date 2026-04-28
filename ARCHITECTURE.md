# Solitaire Quest — Architecture Document

> **Version:** 1.1  
> **Language:** Rust (Edition 2021)  
> **Engine:** Bevy (latest stable)  
> **Last Updated:** 2026-04-20

---

## Table of Contents

1. [Project Overview](#1-project-overview)
2. [Workspace Structure](#2-workspace-structure)
3. [Crate Responsibilities](#3-crate-responsibilities)
4. [Data Flow](#4-data-flow)
5. [Game Engine Architecture](#5-game-engine-architecture)
6. [Persistence & Sync Architecture](#6-persistence--sync-architecture)
7. [Sync Server Architecture](#7-sync-server-architecture)
8. [Google Play Games Services (Android Future)](#8-google-play-games-services-android-future)
9. [Data Models](#9-data-models)
10. [API Reference](#10-api-reference)
11. [Merge Strategy](#11-merge-strategy)
12. [Achievement System](#12-achievement-system)
13. [Progression System](#13-progression-system)
14. [Audio System](#14-audio-system)
15. [Asset Pipeline](#15-asset-pipeline)
16. [Platform Targets](#16-platform-targets)
17. [Build & Development Guide](#17-build--development-guide)
18. [Deployment Guide](#18-deployment-guide)
19. [Security Model](#19-security-model)
20. [Testing Strategy](#20-testing-strategy)
21. [Decision Log](#21-decision-log)

---

## 1. Project Overview

Solitaire Quest is a cross-platform Klondike Solitaire game written in Rust, targeting macOS, Windows, and Linux desktops (iOS/Android as a stretch goal). It features a full progression system with XP, levels, achievements, daily challenges, and an optional self-hosted sync server so statistics and progress are available across all of a player's devices.

On Android (stretch goal), sync is enhanced with Google Play Games Services (GPGS) for native achievement popups, leaderboards, and cloud saves — sitting on top of the same underlying sync payload so data stays consistent regardless of which backend was used last.

### Sync Backend by Platform

| Platform | Primary Sync | Notes |
|---|---|---|
| macOS | Self-hosted server | Full feature set |
| Windows | Self-hosted server | Full feature set |
| Linux | Self-hosted server | Full feature set |
| Android (stretch) | Google Play Games Services | + server as fallback |
| iOS (stretch) | Self-hosted server | GPGS not supported on iOS |

### Design Principles

- **Offline first.** The local file is always the source of truth. Sync is additive, never destructive.
- **Pure core.** All game logic lives in a dependency-free Rust crate with no Bevy, no network, and no I/O. This keeps it fully unit-testable and portable.
- **No panics in game logic.** Every state transition returns `Result<_, MoveError>`. Panics are only acceptable in startup/configuration code.
- **One language, one repo.** The game client, sync client, shared types, and sync server are all Rust crates in a single Cargo workspace.
- **Plugin-based Bevy architecture.** Each major feature is a Bevy `Plugin`. Systems are small and single-purpose. Cross-system communication uses Bevy `Event`s.

---

## 2. Workspace Structure

```
solitaire_quest/
│
├── Cargo.toml                  # Workspace manifest
├── .env.example                # Server environment variable template
├── ARCHITECTURE.md             # This document
├── README.md                   # Player-facing readme
├── README_SERVER.md            # Self-hosting guide
├── Dockerfile                  # Multi-stage server build
├── docker-compose.yml          # Server + Caddy reverse proxy
│
├── assets/                     # All runtime assets (loaded via Bevy AssetServer)
│   ├── cards/
│   │   ├── faces/              # Card face sprites (suit + rank)
│   │   └── backs/              # Card back designs (back_0.png … back_4.png)
│   ├── backgrounds/            # Table backgrounds (bg_0.png … bg_4.png)
│   ├── fonts/                  # .ttf font files
│   └── audio/
│       ├── card_deal.ogg
│       ├── card_flip.ogg
│       ├── card_place.ogg
│       ├── card_invalid.ogg
│       ├── win_fanfare.ogg
│       └── ambient_loop.ogg
│
├── solitaire_core/             # Pure Rust game logic — zero external deps beyond rand/serde
├── solitaire_sync/             # Shared API types — used by client and server
├── solitaire_data/             # Persistence, sync client, settings
├── solitaire_engine/           # Bevy ECS systems, components, plugins
├── solitaire_server/           # Self-hosted sync server (Axum + SQLite)
├── solitaire_gpgs/             # Google Play Games Services bridge (Android only, stub until stretch goal)
└── solitaire_app/              # Main binary entry point
```

---

## 3. Crate Responsibilities

### `solitaire_core`
**Dependencies:** `rand`, `serde`, `chrono` only.

The entire game rules engine. No Bevy, no network, no file I/O. Designed to be tested in isolation with `cargo test -p solitaire_core`.

Owns:
- All game data models (`Card`, `Suit`, `Rank`, `Pile`, `GameState`)
- Move validation logic
- Scoring engine
- Undo stack
- Win / auto-complete detection
- Achievement unlock condition evaluation
- Seeded RNG for reproducible deals

### `solitaire_sync`
**Dependencies:** `serde`, `serde_json`, `uuid`, `chrono` only.

Shared API contract types imported by both the game client (`solitaire_data`) and the server (`solitaire_server`). Changing a type here is a breaking change on both sides — version carefully.

Owns:
- `SyncPayload`, `SyncResponse`, `ConflictReport`
- `ChallengeGoal`, `LeaderboardEntry`
- `ApiError` enum
- Merge logic (pure functions, no I/O)

### `solitaire_data`
**Dependencies:** `solitaire_core`, `solitaire_sync`, `serde_json`, `dirs`, `keyring`, `reqwest`, `tokio` (minimal).

All persistence and sync client code. No Bevy dependency — Bevy systems in `solitaire_engine` call into this crate via the `SyncPlugin`.

Owns:
- Local file read/write (atomic via `.tmp` → rename)
- `StatsSnapshot`, `PlayerProgress`, `AchievementRecord` persistence
- `SyncBackend` enum and backend selection
- Solitaire Server sync client (JWT auth, auto-refresh)
- OS keychain integration (`keyring`)
- `SyncProvider` trait — implemented by both `SolitaireServerClient` and `GpgsClient` (Android)

### `solitaire_gpgs` *(stub — implement when targeting Android)*
**Dependencies:** `solitaire_sync`, `jni` (Android only), `solitaire_data` trait impls.

Android-only crate, compiled only when `target_os = "android"`. Bridges the Google Play Games Services Java SDK via JNI.

Owns:
- `GpgsClient` implementing the `SyncProvider` trait from `solitaire_data`
- GPGS Saved Games API calls (load/save cloud save slot)
- GPGS Achievements API calls (unlock, reveal, increment)
- GPGS Leaderboards API calls (submit score, load scores)
- Google Sign-In token management (via JNI into Android SDK)
- Conversion between GPGS cloud save blob ↔ `SyncPayload`

> **Note:** This crate contains only a trait stub and compile-time stub implementations until Android support is actively developed. Do not implement JNI bindings until Phase: Android.

### `solitaire_engine`
**Dependencies:** `bevy`, `bevy_kira_audio`, `solitaire_core`, `solitaire_data`.

All Bevy-specific code. Structured as a collection of Plugins that `solitaire_app` registers.

Owns:
- Bevy ECS components and resources
- Rendering systems (card sprites, table, backgrounds)
- Drag-and-drop input handling
- Animation systems (slide, flip, win cascade, toast)
- All Bevy UI screens (Home, Stats, Achievements, Settings, Profile)
- Audio playback systems
- Sync status display

### `solitaire_server`
**Dependencies:** `solitaire_sync`, `axum`, `sqlx`, `jsonwebtoken`, `bcrypt`, `tower-governor`, `tracing`, `tokio`, `dotenvy`.

Standalone binary. Can be built and run independently of the game.

Owns:
- HTTP API (see Section 9)
- SQLite database schema and migrations
- Auth (registration, login, JWT issuance and refresh)
- Server-side merge logic (delegates to `solitaire_sync`)
- Rate limiting
- Daily challenge seed generation
- Leaderboard management

### `solitaire_app`
**Dependencies:** `bevy`, `solitaire_engine`.

Thin binary entry point. Registers all Bevy plugins and sets initial window properties.

---

## 4. Data Flow

### Game Loop (local, no sync)

```
User Input
    │
    ▼
Bevy InputSystem
    │  fires GameInputEvent
    ▼
GameLogicSystem (solitaire_engine)
    │  calls solitaire_core::GameState::move_cards() → Result
    ▼
GameStateResource updated
    │  fires StateChangedEvent
    ▼
RenderSystem          ScoreSystem          AchievementSystem
(update sprites)      (update score HUD)   (check unlock conditions)
                                                │
                                                │ fires AchievementUnlockedEvent
                                                ▼
                                          ToastSystem (Bevy UI popup)
                                          PersistenceSystem (write to disk)
```

### Sync Flow (on launch)

```
App starts
    │
    ▼
SyncPlugin::on_startup()
    │  spawns AsyncComputeTask
    ▼
solitaire_data::sync_pull()          ← dispatches to active SyncProvider
    │                                    SolitaireServerClient  (desktop / iOS)
    │                                    GpgsClient             (Android, future)
    ▼
solitaire_sync::merge(local, remote)
    │
    ▼
Write merged result to disk
    │  fires SyncCompleteEvent
    ▼
Bevy main thread reads updated StatsResource
```

### Sync Flow (on exit)

```
AppExit event
    │
    ▼
SyncPlugin::on_exit()
    │  blocking push (acceptable on exit, not on main loop)
    ▼
active SyncProvider::push(local)
    │  POST to server  — or —  GPGS Saved Games PUT (Android)
    ▼
Done
```

---

## 5. Game Engine Architecture

### Bevy Plugins

| Plugin | Key | Responsibility |
|---|---|---|
| `CardPlugin` | — | Card entity spawning, sprite management, drag-and-drop |
| `TablePlugin` | — | Pile markers, background, layout calculation |
| `AnimationPlugin` | — | Slide, flip, win cascade, toast animations |
| `FeedbackAnimPlugin` | — | Shake, settle, and deal-stagger animations |
| `AutoCompletePlugin` | Enter | Executes auto-complete when the HUD badge is lit |
| `AudioPlugin` | — | Sound effect and music playback via bevy_kira_audio |
| `InputPlugin` | — | Keyboard and mouse input routing |
| `CursorPlugin` | — | Custom cursor sprite during drag |
| `SelectionPlugin` | — | Keyboard-driven card selection |
| `GamePlugin` | N | Core game state resource, new-game flow, win/game-over overlays |
| `HudPlugin` | — | Score, move counter, timer, auto-complete badge |
| `StatsPlugin` | S | Stats overlay and persistence |
| `ProgressPlugin` | — | XP/level system, persistence |
| `AchievementPlugin` | A | Unlock evaluation, toast events, persistence |
| `DailyChallengePlugin` | — | Daily challenge resource and completion tracking |
| `WeeklyGoalsPlugin` | — | Weekly goal progress and completion events |
| `ChallengePlugin` | — | Challenge mode progression (seeded hard deals) |
| `TimeAttackPlugin` | — | 10-minute time-attack mode timer |
| `HomePlugin` | M | Main-menu overlay with keyboard shortcut reference |
| `ProfilePlugin` | P | Player profile overlay: level, XP, achievements, sync status |
| `SettingsPlugin` | O | Settings panel: audio, draw mode, theme, sync, cosmetics |
| `LeaderboardPlugin` | L | Leaderboard overlay |
| `HelpPlugin` | H | Help / controls overlay |
| `PausePlugin` | Esc | Pause and resume |
| `OnboardingPlugin` | — | First-run welcome screen |
| `SyncPlugin` | — | Async sync lifecycle (pull on start, push on exit, status display) |
| `WinSummaryPlugin` | — | Win cascade overlay and screen-shake effect |

### Key Bevy Resources

```rust
// Current game state — single source of truth for the active game
struct GameStateResource(GameState);

// Sync status shown in Settings screen
enum SyncStatus { Idle, Syncing, LastSynced(DateTime<Utc>), Error(String) }
struct SyncStatusResource(SyncStatus);

// Currently active drag operation
struct DragState {
    cards: Vec<u32>,          // card ids being dragged
    origin_pile: PileType,
    cursor_offset: Vec2,
    origin_z: f32,
}

// Loaded user data
struct StatsResource(StatsSnapshot);
struct ProgressResource(PlayerProgress);
struct AchievementsResource(Vec<AchievementRecord>);
struct SettingsResource(Settings);
```

### Key Bevy Events

```rust
// Input → Logic
struct MoveRequestEvent { from: PileType, to: PileType, count: usize }
struct DrawRequestEvent;
struct UndoRequestEvent;
struct NewGameRequestEvent { seed: Option<u64> }

// Logic → Rendering/UI
struct StateChangedEvent;
struct CardFlippedEvent(u32);
struct GameWonEvent { score: i32, time_seconds: u64 }
struct AchievementUnlockedEvent(AchievementRecord);
struct SyncCompleteEvent(Result<SyncResponse, String>);
```

### Layout System

Card and pile positions are calculated from window dimensions on startup and on every `WindowResized` event.

```
Window width  → card_width  = window_width  / 9.0   (7 columns + 2 margins)
Window height → card_height = card_width * 1.4       (standard card aspect ratio)
Pile spacing  → h_gap       = (window_width - 7 * card_width) / 8.0
```

Minimum window: 800×600. At this size cards are small but usable.

---

## 6. Persistence & Sync Architecture

### Local Storage

All files stored under `dirs::data_dir() / "solitaire_quest"/`:

```
~/.local/share/solitaire_quest/   (Linux)
~/Library/Application Support/solitaire_quest/   (macOS)
%APPDATA%\solitaire_quest\   (Windows)
│
├── stats.json          # StatsSnapshot
├── progress.json       # PlayerProgress (XP, level, unlocks, daily challenge)
├── achievements.json   # Vec<AchievementRecord>
├── settings.json       # Settings (draw mode, audio, theme, sync backend)
└── game_state.json     # In-progress game (saved on pause/exit, deleted on win/loss)
```

Atomic writes: all saves go to `filename.json.tmp` first, then `rename()` — ensuring a crash mid-write never corrupts saved data.

### `SyncProvider` Trait

All sync backends implement a single trait in `solitaire_data`. The `SyncPlugin` holds a `Box<dyn SyncProvider + Send + Sync>` and is backend-agnostic.

```rust
#[async_trait]
pub trait SyncProvider: Send + Sync {
    async fn pull(&self) -> Result<SyncPayload, SyncError>;
    async fn push(&self, payload: &SyncPayload) -> Result<SyncResponse, SyncError>;
    fn backend_name(&self) -> &'static str;
    fn is_authenticated(&self) -> bool;
}
```

Implementations:

| Struct | Backend | Platforms |
|---|---|---|
| `LocalOnlyProvider` | No-op (default) | All |
| `SolitaireServerClient` | Self-hosted server | All |
| `GpgsClient` *(future)* | Google Play Games Services | Android only |

Sync always runs on `bevy::tasks::AsyncComputeTaskPool` — the game thread is never blocked.

### Sync Backends (Settings enum)

```rust
pub enum SyncBackend {
    Local,
    SolitaireServer {
        url: String,
        username: String,
        // JWT access + refresh tokens stored in OS keychain
        // key: "solitaire_quest_server_{username}"
    },
    GooglePlayGames,
    // No credentials stored locally — auth managed by Google Sign-In SDK via JNI
    // Android only; selecting this on non-Android falls back to Local silently
}
```

### Solitaire Server Sync

On launch: `GET /api/sync/pull` with `Authorization: Bearer {access_token}`
On exit: `POST /api/sync/push` with payload

On 401: automatically attempt `POST /api/auth/refresh`, retry once, then surface error to user.
Credentials stored in OS keychain via `keyring` — never in plaintext on disk.

### Google Play Games Sync *(Android — future, see Section 8)*

Implemented in `solitaire_gpgs` crate. Uses the GPGS Saved Games API with named slot `"solitaire_quest_sync"`. The `GpgsClient` struct implements `SyncProvider` — the `SyncPlugin` treats it identically to `SolitaireServerClient`. The same `solitaire_sync::merge()` function applies regardless of which provider returned the remote data.

---

## 7. Sync Server Architecture

### Stack

| Component | Crate |
|---|---|
| HTTP framework | `axum` |
| Database | `sqlx` with SQLite |
| Auth | `jsonwebtoken` + `bcrypt` |
| Rate limiting | `tower-governor` |
| Logging | `tracing` + `tracing-subscriber` |
| Config | `dotenvy` |
| Shared types | `solitaire_sync` (workspace crate) |

### Database Schema

```sql
-- migrations/001_initial.sql

CREATE TABLE users (
    id          TEXT PRIMARY KEY,          -- UUID v4
    username    TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,           -- bcrypt, cost 12
    created_at  TEXT NOT NULL,             -- ISO 8601
    leaderboard_opt_in INTEGER DEFAULT 0
);

CREATE TABLE sync_state (
    user_id         TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    stats_json      TEXT NOT NULL,
    achievements_json TEXT NOT NULL,
    progress_json   TEXT NOT NULL,
    last_modified   TEXT NOT NULL
);

CREATE TABLE daily_challenges (
    date        TEXT PRIMARY KEY,          -- "YYYY-MM-DD"
    seed        INTEGER NOT NULL,
    goal_json   TEXT NOT NULL
);

CREATE TABLE leaderboard (
    user_id         TEXT REFERENCES users(id) ON DELETE CASCADE,
    display_name    TEXT NOT NULL,
    best_time_secs  INTEGER,
    best_score      INTEGER,
    recorded_at     TEXT NOT NULL,
    PRIMARY KEY (user_id)
);
```

### Request Lifecycle

```
Client Request
    │
    ▼
tower-governor (rate limiter — 10 req/min on /api/auth/*)
    │
    ▼
axum Router
    │
    ├─ /api/auth/*  → AuthHandler (no JWT required)
    │
    └─ /api/*       → JwtMiddleware → Handler
                           │
                           ├─ Validate JWT signature + expiry
                           ├─ Reject payload > 1MB
                           └─ Extract user_id for handler
```

### Daily Challenge Generation

If no row exists in `daily_challenges` for today's date, the server generates one on first request:

```rust
let seed = hash_date_to_u64("2026-04-19");  // deterministic, same for all players
let goal = generate_goal_from_seed(seed);    // seeded RNG picks goal type + params
```

This ensures all players worldwide get the same challenge for a given date, regardless of which server instance handles the request.

---

## 8. Google Play Games Services (Android Future)

> **Status: Stub only.** Do not implement JNI bindings until Android is actively targeted. The `solitaire_gpgs` crate exists in the workspace with a trait stub so the compiler enforces the interface contract from day one.

### Why GPGS on Android

Google Play Games Services provides first-class Android features that would otherwise require significant backend work:

| Feature | GPGS Provides | Our Alternative |
|---|---|---|
| Cloud saves | Saved Games API | Self-hosted server |
| Achievements | Native popups + Play profile | In-game toasts only |
| Leaderboards | Hosted by Google, visible in Play app | Server leaderboard |
| Auth | Google Sign-In, no registration | Username + password |

On Android, GPGS is the **primary** sync provider. The self-hosted server is the **fallback** if the player is not signed in or has no server configured. Both can be active simultaneously — a win pushes to both, pull merges from whichever responded last.

### Compatibility Reality

| Platform | GPGS Support |
|---|---|
| Android | ✅ Full |
| Windows | ✅ GPGS for PC (optional, separate SDK) |
| macOS | ❌ Not supported |
| Linux | ❌ Not supported |
| iOS | ❌ Not supported |

macOS, Linux, and iOS users always use the self-hosted server. This is why the server is the primary design and GPGS is an enhancement layer.

### `solitaire_gpgs` Crate Design

The crate is compiled only on Android (`#[cfg(target_os = "android")]`). On all other platforms the crate exports only the stub.

```rust
// solitaire_gpgs/src/lib.rs

#[cfg(target_os = "android")]
mod android;

#[cfg(not(target_os = "android"))]
mod stub;

pub use stub::GpgsClient;   // stub on desktop
#[cfg(target_os = "android")]
pub use android::GpgsClient; // real impl on Android
```

### JNI Bridge (Android implementation — future)

The real `GpgsClient` uses the `jni` crate to call into the GPGS Android SDK:

```
Rust GpgsClient
    │  jni::JNIEnv
    ▼
Java: com.google.android.gms.games.PlayGames
    ├── getSnapshotsClient()   → Saved Games (sync payload)
    ├── getAchievementsClient() → unlock / reveal
    └── getLeaderboardsClient() → submit score
```

Steps required when Android work begins:
1. Add `cargo-mobile2` to the build toolchain
2. Implement `GpgsClient` with `jni` bindings in `solitaire_gpgs/src/android.rs`
3. Add `GpgsClient: SyncProvider` impl — pull/push map to Saved Games load/save
4. Mirror achievement unlocks: on `AchievementUnlockedEvent`, call GPGS unlock API alongside in-game toast
5. Submit scores to GPGS leaderboard on `GameWonEvent`
6. Add Google Sign-In button to the Settings screen (Android build only, `#[cfg]` gated)

### Dual-Sync on Android

When both GPGS and the self-hosted server are configured, the `SyncPlugin` runs both providers concurrently and merges all three payloads (local + GPGS + server) using the same `solitaire_sync::merge()` function applied twice:

```
local ──────┐
             ├── merge() ──► intermediate ──┐
gpgs ────────┘                               ├── merge() ──► final
                                server ──────┘
```

---

## 9. Data Models

### Core Game Models (`solitaire_core`)

```rust
pub enum Suit { Clubs, Diamonds, Hearts, Spades }
pub enum Rank { Ace, Two, Three, Four, Five, Six, Seven, Eight, Nine, Ten, Jack, Queen, King }

pub struct Card {
    pub id: u32,
    pub suit: Suit,
    pub rank: Rank,
    pub face_up: bool,
}

pub enum PileType {
    Stock,
    Waste,
    Foundation(Suit),
    Tableau(usize),   // 0–6
}

pub enum DrawMode { DrawOne, DrawThree }

/// Active game mode. Classic is the default; others unlock at level 5.
pub enum GameMode { Classic, Zen, Challenge, TimeAttack }

pub enum MoveError {
    InvalidSource,
    InvalidDestination,
    EmptySource,
    RuleViolation(String),
    UndoStackEmpty,
    GameAlreadyWon,
}

pub struct GameState {
    pub piles: HashMap<PileType, Vec<Card>>,
    pub draw_mode: DrawMode,
    pub mode: GameMode,
    pub score: i32,
    pub move_count: u32,
    pub undo_count: u32,        // number of undos used in this game
    pub recycle_count: u32,     // number of stock recycles
    pub elapsed_seconds: u64,
    pub seed: u64,
    pub is_won: bool,
    pub is_auto_completable: bool,
    undo_stack: VecDeque<StateSnapshot>,   // private, max 64 (VecDeque for O(1) pop_front)
}
```

### Persistence Models (`solitaire_data`)

```rust
pub struct StatsSnapshot {
    pub games_played: u32,
    pub games_won: u32,
    pub games_lost: u32,
    pub win_streak_current: u32,
    pub win_streak_best: u32,
    pub avg_time_seconds: u64,
    pub fastest_win_seconds: u64,
    pub lifetime_score: u64,
    pub best_single_score: u32,
    pub draw_one_wins: u32,
    pub draw_three_wins: u32,
    pub last_modified: DateTime<Utc>,
}

pub struct PlayerProgress {
    pub total_xp: u64,
    pub level: u32,
    pub daily_challenge_last_completed: Option<NaiveDate>,
    pub daily_challenge_streak: u32,
    pub weekly_goal_progress: HashMap<String, u32>,
    pub unlocked_card_backs: Vec<usize>,
    pub unlocked_backgrounds: Vec<usize>,
    pub last_modified: DateTime<Utc>,
}

pub struct AchievementRecord {
    pub id: String,
    pub unlocked: bool,
    pub unlock_date: Option<DateTime<Utc>>,
    pub reward_granted: bool,
}

pub struct Settings {
    pub draw_mode: DrawMode,
    pub sfx_volume: f32,           // 0.0–1.0
    pub music_volume: f32,
    pub animation_speed: AnimSpeed,
    pub theme: Theme,
    pub sync_backend: SyncBackend, // Local | SolitaireServer | GooglePlayGames
    pub first_run_complete: bool,
}
```

---

## 10. API Reference

All endpoints are under the base URL configured by the user (e.g., `https://solitaire.example.com`).

### Authentication

| Method | Path | Auth | Body | Response |
|---|---|---|---|---|
| POST | `/api/auth/register` | None | `{username, password}` | `{access_token, refresh_token}` |
| POST | `/api/auth/login` | None | `{username, password}` | `{access_token, refresh_token}` |
| POST | `/api/auth/refresh` | None | `{refresh_token}` | `{access_token}` |

### Sync

| Method | Path | Auth | Body | Response |
|---|---|---|---|---|
| GET | `/api/sync/pull` | Bearer JWT | — | `SyncResponse` |
| POST | `/api/sync/push` | Bearer JWT | `SyncPayload` | `SyncResponse` |

### Game Data

| Method | Path | Auth | Body | Response |
|---|---|---|---|---|
| GET | `/api/daily-challenge` | None | — | `ChallengeGoal` |
| GET | `/api/leaderboard` | Bearer JWT | — | `Vec<LeaderboardEntry>` |
| POST | `/api/leaderboard/opt-in` | Bearer JWT | — | `{ok: true}` |

### Account Management

| Method | Path | Auth | Body | Response |
|---|---|---|---|---|
| DELETE | `/api/account` | Bearer JWT | — | `{ok: true}` |
| GET | `/health` | None | — | `{status, version}` |

### JWT Details

- Access token expiry: 24 hours
- Refresh token expiry: 30 days
- Algorithm: HS256
- Secret: `JWT_SECRET` environment variable (min 64 chars recommended)

---

## 11. Merge Strategy

Used identically by the `SolitaireServerClient`, `GpgsClient`, and server-side handler. Lives in `solitaire_sync` as a pure function with no I/O. Called once per provider when multiple backends are active simultaneously (e.g. GPGS + server on Android).

```rust
pub fn merge(local: &SyncPayload, remote: &SyncPayload) -> SyncPayload {
    SyncPayload {
        stats: StatsSnapshot {
            games_played:       max(local.stats.games_played, remote.stats.games_played),
            games_won:          max(local.stats.games_won, remote.stats.games_won),
            games_lost:         max(local.stats.games_lost, remote.stats.games_lost),
            win_streak_best:    max(local.stats.win_streak_best, remote.stats.win_streak_best),
            win_streak_current: max(local.stats.win_streak_current, remote.stats.win_streak_current),
            fastest_win_seconds: min(local.stats.fastest_win_seconds, remote.stats.fastest_win_seconds),
            best_single_score:  max(local.stats.best_single_score, remote.stats.best_single_score),
            lifetime_score:     max(local.stats.lifetime_score, remote.stats.lifetime_score),
            // avg_time recomputed from merged games_played/total_time
            last_modified: Utc::now(),
            ..
        },
        achievements: union_by_id(         // never remove an unlocked achievement
            &local.achievements,           // keep earliest unlock_date on conflict
            &remote.achievements,
        ),
        progress: PlayerProgress {
            total_xp:              max(local.progress.total_xp, remote.progress.total_xp),
            unlocked_card_backs:   union_vecs(&local.progress.unlocked_card_backs, &remote.progress.unlocked_card_backs),
            unlocked_backgrounds:  union_vecs(&local.progress.unlocked_backgrounds, &remote.progress.unlocked_backgrounds),
            // level recomputed from merged total_xp
            last_modified: Utc::now(),
            ..
        },
        last_modified: Utc::now(),
        ..
    }
}
```

**Conflict reporting:** Any case where local and remote have different values for the same field that cannot be merged deterministically (e.g., different daily challenge streak counts) is recorded in `Vec<ConflictReport>` and returned to the client for display — data is never silently discarded.

---

## 12. Achievement System

### Definition Structure

Achievements are defined as static data in `solitaire_core`. Runtime unlock state (`unlocked`, `unlock_date`, `reward_granted`) is stored separately in `solitaire_data`.

```rust
pub struct AchievementDef {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub icon: &'static str,
    pub secret: bool,
    pub reward: Option<Reward>,
    pub condition: fn(&GameState, &StatsSnapshot, &PlayerProgress) -> bool,
}
```

### Achievement List

| ID | Name | Condition | Secret | Reward |
|---|---|---|---|---|
| `first_win` | First Win | Win 1 game | No | — |
| `on_a_roll` | On a Roll | Win streak ≥ 3 | No | Card back #1 |
| `unstoppable` | Unstoppable | Win streak ≥ 10 | No | Background #1 |
| `century` | Century | 100 games played | No | — |
| `veteran` | Veteran | 500 games played | No | Badge |
| `speed_demon` | Speed Demon | Win in < 3 min | No | — |
| `lightning` | Lightning | Win in < 90 sec | No | Card back #2 |
| `high_scorer` | High Scorer | Score ≥ 5,000 | No | — |
| `point_machine` | Point Machine | Lifetime score ≥ 50,000 | No | Background #2 |
| `no_undo` | No Undo | Win without undo | No | +25 XP |
| `draw_three_master` | Draw 3 Master | 10 Draw 3 wins | No | Card back #3 |
| `perfectionist` | Perfectionist | Max possible score | No | Badge |
| `night_owl` | Night Owl | Play after midnight | No | — |
| `early_bird` | Early Bird | Play before 6am | No | — |
| `daily_devotee` | Daily Devotee | 7 daily challenges | No | Background #3 |
| `speed_and_skill` | ??? | Win < 90s without undo | Yes | Card back #4 |
| `comeback` | ??? | Win after 3+ stock recycles | Yes | Background #4 |
| `zen_winner` | ??? | Win in Zen Mode | Yes | Badge |

### Evaluation Timing

Achievement conditions are evaluated by `AchievementPlugin` on every `GameWonEvent` and `StateChangedEvent`. The plugin calls `solitaire_core::check_achievements()` which returns a `Vec<AchievementDef>` of newly unlocked achievements. The plugin then fires `AchievementUnlockedEvent` for each, which the toast and persistence systems handle independently.

### GPGS Mirroring *(Android, future)*

When the `GpgsClient` is active, every `AchievementUnlockedEvent` also triggers a GPGS `achievements.unlock()` call via JNI so the achievement appears in the player's Google Play profile. A static map in `solitaire_gpgs` maps our achievement IDs to GPGS achievement IDs (assigned in the Google Play Console). Mirroring is fire-and-forget — failures are logged but never block the in-game toast.

---

## 13. Progression System

### XP Sources

| Action | XP Awarded |
|---|---|
| Win a game | +50 |
| Fast win bonus (< 2 min) | +10 to +50 (scaled) |
| Win without undo | +25 |
| Complete daily challenge | +100 |
| Complete weekly goal | +75 |

### Level Formula

```
Levels 1–10:  level = floor(total_xp / 500)
Levels 11+:   level = 10 + floor((total_xp - 5000) / 1000)
```

### Special Modes (unlocked at level 5)

| Mode | Rules |
|---|---|
| **Time Attack** | Play as many games as possible in 10 minutes. Score = total wins. |
| **Challenge Mode** | Fixed hard seeds. No undo. Must win to advance. |
| **Zen Mode** | No timer. No score display. Ambient audio. No penalties. |

---

## 14. Audio System

Audio uses `bevy_kira_audio`. All sound files are `.ogg` (good compression, cross-platform, royalty-free).

| File | Trigger |
|---|---|
| `card_deal.ogg` | New game deal animation |
| `card_flip.ogg` | Card flips face-up |
| `card_place.ogg` | Valid card placement |
| `card_invalid.ogg` | Invalid move attempt |
| `win_fanfare.ogg` | Game won |
| `ambient_loop.ogg` | Looping background music (restarts seamlessly) |

Volume is controlled by two independent sliders in Settings (`sfx_volume`, `music_volume`), each stored in `Settings` and applied as `bevy_kira_audio` channel volumes.

Audio systems listen for Bevy events and never block the game thread.

---

## 15. Asset Pipeline

All assets are loaded through Bevy's `AssetServer`. No bytes are hardcoded in source.

### Card Sprites

Card faces can be either:
- A texture atlas (`assets/cards/atlas.png` + `atlas.ron` layout) — faster to load, preferred
- Individual files (`assets/cards/faces/2_of_clubs.png`, etc.) — easier to author

Card backs: `assets/cards/backs/back_0.png` through `back_4.png`. Additional backs unlocked via achievements are in the same folder, gated by `PlayerProgress::unlocked_card_backs`.

### Backgrounds

`assets/backgrounds/bg_0.png` through `bg_4.png`. Same unlock gating as card backs.

### Fonts

`assets/fonts/main.ttf` — used for card rank/suit text in Bevy UI.

---

## 16. Platform Targets

| Platform | Status | Primary Sync | Notes |
|---|---|---|---|
| macOS | Primary | Self-hosted server | x86_64 + Apple Silicon (universal binary via `cargo-lipo`) |
| Windows | Primary | Self-hosted server | x86_64, MSVC toolchain; optional GPGS for PC (future) |
| Linux | Primary | Self-hosted server | x86_64, tested on Ubuntu 22.04+ and Fedora 39+ |
| Android | Stretch | Google Play Games + server | `cargo-mobile2`, touch input, GPGS via JNI |
| iOS | Stretch | Self-hosted server | `cargo-mobile2`, touch input; GPGS unavailable on iOS |

Minimum Bevy window size enforced: 800×600. Desktop windows are freely resizable; layout recomputes on `WindowResized`.

---

## 17. Build & Development Guide

### Prerequisites

- Rust stable toolchain (via `rustup`)
- For Linux: `libasound2-dev`, `libudev-dev`, `libxkbcommon-dev`
- For macOS: Xcode Command Line Tools

### Common Commands

```bash
# Run the game (dev build with dynamic linking for fast compile)
cargo run -p solitaire_app --features bevy/dynamic_linking

# Run with release optimizations
cargo run -p solitaire_app --release

# Run all tests
cargo test --workspace

# Lint (must pass clean — no warnings allowed)
cargo clippy --workspace -- -D warnings

# Run the sync server locally
cargo run -p solitaire_server

# Build release binaries for all crates
cargo build --workspace --release
```

### Environment Variables (Server)

Copy `.env.example` to `.env` and fill in:

```
DATABASE_URL=sqlite://solitaire.db
JWT_SECRET=<generate with: openssl rand -hex 32>
SERVER_PORT=8080
ADMIN_USERNAME=admin          # optional, seeded on first run
```

### Fast Compile Setup

The workspace `Cargo.toml` includes:

```toml
[profile.dev]
opt-level = 1

[profile.dev.package."*"]
opt-level = 3

[profile.release]
opt-level = 3
lto = "thin"
```

Add `--features bevy/dynamic_linking` during development to dramatically reduce incremental compile times.

---

## 18. Deployment Guide

### Docker Compose (Recommended)

```bash
git clone https://github.com/yourname/solitaire_quest
cd solitaire_quest
cp .env.example .env
# Edit .env — set JWT_SECRET and SERVER_PORT
docker compose up -d
```

This starts the sync server + a Caddy reverse proxy with automatic TLS (provide your domain in `docker-compose.yml`).

### Systemd Service (Alternative)

```bash
cargo build -p solitaire_server --release
sudo cp target/release/solitaire_server /usr/local/bin/
sudo cp solitaire_server.service /etc/systemd/system/
sudo systemctl enable --now solitaire_server
```

### Backups

The entire server state is in a single SQLite file. Back it up by copying it:

```bash
sqlite3 solitaire.db ".backup backup_$(date +%Y%m%d).db"
```

Or just `cp solitaire.db backups/` — SQLite's WAL mode makes this safe while the server is running.

### Updating

```bash
git pull
cargo build -p solitaire_server --release
sudo systemctl restart solitaire_server
```

Migrations run automatically on startup via `sqlx::migrate!()`.

---

## 19. Security Model

| Concern | Mitigation |
|---|---|
| Password storage | bcrypt, cost factor 12 — never stored in plaintext |
| Token security | JWTs signed with HS256, stored in OS keychain via `keyring` crate |
| Token expiry | Access: 24h, Refresh: 30d |
| Brute force | `tower-governor`: 10 req/min per IP on `/api/auth/*` |
| Payload abuse | 1MB max request body, enforced by Axum middleware |
| Data deletion | `DELETE /api/account` removes all rows via `ON DELETE CASCADE` |
| TLS | Handled by reverse proxy (Caddy/nginx) — server runs plain HTTP internally |
| PII | Only username stored — no email, no real name required |
| Leaderboard | Opt-in only — display name chosen by user at opt-in time |

---

## 20. Testing Strategy

### Unit Tests (`solitaire_core`)

Every public function in `solitaire_core` has corresponding `#[test]` coverage:

- All legal move types (tableau→foundation, waste→tableau, etc.)
- All illegal move types and their `MoveError` variants
- Undo: state fully restored after 1, 5, and 64 undos
- Scoring: each action type, time bonus formula, floor at zero
- Win detection: true positive (complete foundation), true negative
- Auto-complete detection
- Seeded deal: same seed produces identical layout across 100 runs

### Unit Tests (`solitaire_sync`)

- Merge: each field merges correctly (max, min, union)
- Merge: idempotent (merging identical payloads returns identical payload)
- Merge: achievements never removed

### Integration Tests (`solitaire_server`)

Using `axum::test` and an in-memory SQLite database:

- Auth flow: register → login → access protected endpoint → refresh → access again
- Sync roundtrip: push payload → pull → verify merged response
- Rate limiting: 11th request within 1 minute returns 429
- Account deletion: all rows removed, subsequent JWT rejected

### Manual Test Checklist (per platform, per release)

- [ ] New game deals correctly, all 52 cards present
- [ ] Drag and drop works for all pile type combinations
- [ ] Win triggers cascade animation and score display
- [ ] Undo restores previous state visually and in data
- [ ] Stats persist across app restart
- [ ] Achievement toast appears and dismisses
- [ ] Server sync: register, login, push, pull on second machine
- [ ] Server sync: JWT refresh on 401 works transparently
- [ ] GPGS sync (Android only): sign in, unlock achievement, verify appears in Play Games app
- [ ] Dual sync (Android only): GPGS + server both configured, payloads merge correctly

---

## 21. Decision Log

| Decision | Rationale | Date |
|---|---|---|
| Bevy as game engine | Best-in-class Rust game engine; ECS architecture suits card game structure well; active ecosystem | 2026-04-19 |
| SQLite over Postgres for server | Single-file DB simplifies self-hosting enormously; a card game sync server will never need Postgres-scale throughput | 2026-04-19 |
| Shared `solitaire_sync` crate | Ensures client and server types are always identical; type errors caught at compile time rather than runtime | 2026-04-19 |
| `keyring` for credential storage | OS keychain is the correct place for secrets on all three desktop platforms; never store JWTs or passwords in plaintext files | 2026-04-19 |
| Atomic file writes (tmp → rename) | Prevents corrupt save files on crash or power loss with zero extra dependencies | 2026-04-19 |
| bcrypt cost 12 | Balances security and registration latency (~300ms on modern hardware); higher than default 10 | 2026-04-19 |
| No email required for server accounts | Reduces PII collected; simplifies self-hosted deployments; password reset handled by server admin if needed | 2026-04-19 |
| Self-hosted server as primary sync (not WebDAV) | A proper Rust server gives us auth, leaderboards, and daily challenge seeding for minimal extra effort over WebDAV, and removes a redundant backend | 2026-04-20 |
| `SyncProvider` trait, not `SyncBackend` match arms | Allows adding Google Play Games Services cleanly; `SyncPlugin` stays backend-agnostic and testable | 2026-04-20 |
| GPGS as Android enhancement, not replacement | GPGS has no macOS/Linux support; the server must remain universal, with GPGS layered on top for Android players | 2026-04-20 |
| Dropped WebDAV backend | Redundant once the self-hosted server exists; removing it reduces surface area and simplifies settings UI | 2026-04-20 |
| `solitaire_gpgs` crate stubbed from day one | Enforces the `SyncProvider` interface contract at compile time even before Android work begins; avoids architectural rework later | 2026-04-20 |
