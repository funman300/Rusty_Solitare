# Solitaire Quest

A cross-platform Klondike Solitaire game written in Rust, with a card-theme
system, full progression (XP / levels / achievements / daily challenges), and
optional self-hosted sync so your stats follow you across machines.

## Features

- **Klondike Solitaire** — Draw One and Draw Three modes; foundations are
  unlocked (any Ace lands in any empty slot, the slot then claims that suit)
- **Card themes** — bundled hayeah/playing-cards-assets default plus
  user-installable themes (drop a directory under the data dir or import a
  zip from Settings → Cosmetic)
- **Modern HUD** — reserved top band keeps cards from crowding the score
  readout; the action bar auto-fades when the cursor leaves it so it can't
  compete with the play surface
- **Drag feel** — every legal drop target is highlighted in green during
  drag; cards cast a soft drop shadow that lifts when picked up; the stock
  pile shows a remaining-count chip so you can see how close you are to a
  recycle
- **Keyboard navigation** — Tab cycles focus through buttons, arrow keys
  move within picker rows, Enter activates; works across every modal and
  the HUD action bar
- **Progression** — XP, levels, unlockable card backs and backgrounds
- **18 Achievements** — including secret ones
- **Daily Challenge** — server-seeded so every player worldwide gets the
  same deal
- **Leaderboard** — opt-in, powered by your own self-hosted server
- **Special Modes** (unlocked at level 5): Zen, Time Attack, Challenge
- **Sync** — pull/push stats across devices via a self-hosted server
- **Color-blind mode** — blue tint on red-suit cards alongside the suit
  glyph

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

Every action also has a visible UI button — keyboard shortcuts are optional
accelerators.

| Key | Action |
|---|---|
| Left click / drag | Move cards |
| Double click | Auto-move card to its best legal destination |
| Right click | Highlight legal moves for a card |
| Space / D | Draw from stock |
| U | Undo |
| H | Hint (highlight a legal move) |
| N | New game |
| Z | Zen mode |
| G | Forfeit (during pause) |
| Tab / Shift+Tab | Cycle keyboard focus |
| Enter | Activate focused button / auto-complete (when badge is lit) |
| Esc | Pause / dismiss modal |
| F1 | Help / controls |
| F11 | Toggle fullscreen |
| S / A / P / O / L / M | Stats / Achievements / Profile / Settings / Leaderboard / Menu |

## Card themes

The default theme ships embedded in the binary, so the game runs
self-contained with no external assets. To install another theme, drop a
directory containing a `theme.ron` manifest plus 53 SVG files (52 faces +
1 back) under the platform data dir's `themes/` folder, or import a zip
from **Settings → Cosmetic**. The picker chip lights up the moment a new
theme is registered. Themes are SVG-based, so they rasterise cleanly at
whatever resolution the window happens to be.

## Sync Server (optional)

To sync stats across machines, run the self-hosted server. See
[README_SERVER.md](README_SERVER.md) for setup instructions.

Once the server is running, open **Settings → Sync Backend**, enter the
server URL and your username, and register an account from within the
game.

## Running Tests

```bash
# All tests (982 passing as of v0.11.0)
cargo test --workspace

# Just game logic (no display required)
cargo test -p solitaire_core -p solitaire_sync -p solitaire_data -p solitaire_server

# Lint
cargo clippy --workspace --all-targets -- -D warnings
```

## Credits

Built on [Bevy](https://bevyengine.org/) and the wider Rust ecosystem
(Tokio, Axum, sqlx, Serde, kira, and many more). Card faces come from
[hayeah/playing-cards-assets](https://github.com/hayeah/playing-cards-assets)
(MIT, derived from the public-domain `vector-playing-cards` library); the
default card back is original work; the UI font is FiraMono-Medium (OFL).
All audio is synthesized programmatically by this project. See
[CREDITS.md](CREDITS.md) for the full list and license details.

## License

MIT — see [LICENSE](LICENSE).
