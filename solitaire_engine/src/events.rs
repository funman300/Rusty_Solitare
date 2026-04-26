//! Cross-system events used by the engine's plugins.

use bevy::prelude::Event;
use solitaire_core::game_state::GameMode;
use solitaire_core::pile::PileType;

/// Request to move `count` cards from `from` to `to`. Fired by input systems,
/// consumed by `GamePlugin`.
#[derive(Event, Debug, Clone)]
pub struct MoveRequestEvent {
    pub from: PileType,
    pub to: PileType,
    pub count: usize,
}

/// Request to draw from the stock (or recycle waste when stock is empty).
#[derive(Event, Debug, Clone, Copy, Default)]
pub struct DrawRequestEvent;

/// Request to undo the most recent state change.
#[derive(Event, Debug, Clone, Copy, Default)]
pub struct UndoRequestEvent;

/// Request to start a new game. `seed = None` uses a system-time seed.
/// `mode = None` reuses the current game's `GameMode`.
#[derive(Event, Debug, Clone, Copy, Default)]
pub struct NewGameRequestEvent {
    pub seed: Option<u64>,
    pub mode: Option<GameMode>,
}

/// Fired by `GamePlugin` after any successful state mutation. Rendering and
/// score-display systems listen for this to refresh.
#[derive(Event, Debug, Clone, Copy, Default)]
pub struct StateChangedEvent;

/// Fired by input/UI systems when a player attempts to drop dragged cards
/// on a real pile but the move violates the rules. Drives the
/// `card_invalid.wav` SFX. Not fired for drops in empty space.
#[derive(Event, Debug, Clone)]
pub struct MoveRejectedEvent {
    pub from: PileType,
    pub to: PileType,
    pub count: usize,
}

/// Fired once when the active game transitions to won.
#[derive(Event, Debug, Clone, Copy)]
pub struct GameWonEvent {
    pub score: i32,
    pub time_seconds: u64,
}

/// Fired when a card's face-up state changes during gameplay.
#[derive(Event, Debug, Clone, Copy)]
pub struct CardFlippedEvent(pub u32);

/// Achievement unlocked notification — name of the achievement.
///
/// Uses `String` as a placeholder; replaced with `AchievementRecord` in Phase 5.
#[derive(Event, Debug, Clone)]
pub struct AchievementUnlockedEvent(pub String);
