# Solitaire Quest

A cross-platform Klondike Solitaire game written in Rust, featuring a full progression system with XP, levels, achievements, daily challenges, and optional self-hosted sync so your stats follow you across machines.

## Features

- **Klondike Solitaire** — Draw One and Draw Three modes
- **Progression** — XP, levels, unlockable card backs and backgrounds
- **18 Achievements** — including secret ones
- **Daily Challenge** — server-seeded so every player worldwide gets the same deal
- **Leaderboard** — opt-in, powered by your own self-hosted server
- **Special Modes** (unlocked at level 5): Zen, Time Attack, Challenge
- **Sync** — pull/push stats across devices via a self-hosted server
- **Color-blind mode** — blue tint on red-suit cards

## Building

**Prerequisites**

- Rust stable toolchain (`rustup install stable`)
- Linux: `libasound2-dev libudev-dev libxkbcommon-dev`
- macOS: Xcode Command Line Tools

```bash
# Fast development build
cargo run -p solitaire_app --features bevy/dynamic_linking

# Release build
cargo build -p solitaire_app --release
./target/release/solitaire_app
```

## Controls

| Key | Action |
|---|---|
| Left click / drag | Move cards |
| Right click | Highlight legal moves for a card |
| Space / D | Draw from stock |
| Z / Ctrl+Z | Undo |
| N | New game |
| S | Stats overlay |
| A | Achievements overlay |
| P | Profile overlay |
| O | Settings |
| L | Leaderboard |
| H | Help / controls |
| Enter | Auto-complete (when badge is lit) |
| Escape | Pause / clear selection |
| Arrow keys | Navigate card selection |

## Sync Server (optional)

To sync stats across machines, run the self-hosted server. See [README_SERVER.md](README_SERVER.md) for setup instructions.

Once the server is running, open **Settings → Sync Backend**, enter the server URL and your username, and register an account from within the game.

## Running Tests

```bash
# All tests
cargo test --workspace

# Just game logic (no display required)
cargo test -p solitaire_core -p solitaire_sync -p solitaire_data -p solitaire_server

# Lint
cargo clippy --workspace -- -D warnings
```

## Credits

Built on [Bevy](https://bevyengine.org/) and the wider Rust ecosystem (Tokio,
Axum, sqlx, Serde, kira, and many more). Card faces and the default card back
use xCards artwork (LGPL-3.0); the UI font is FiraMono-Medium (OFL). All audio
is synthesized programmatically by this project. See [CREDITS.md](CREDITS.md)
for the full list and license details.

## License

MIT — see [LICENSE](LICENSE).
