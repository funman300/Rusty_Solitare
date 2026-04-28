//! Cross-system events used by the engine's plugins.

use bevy::prelude::Message;
use solitaire_core::game_state::GameMode;
use solitaire_core::pile::PileType;
use solitaire_data::AchievementRecord;

/// Request to move `count` cards from `from` to `to`. Fired by input systems,
/// consumed by `GamePlugin`.
#[derive(Message, Debug, Clone)]
pub struct MoveRequestEvent {
    pub from: PileType,
    pub to: PileType,
    pub count: usize,
}

/// Request to draw from the stock (or recycle waste when stock is empty).
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct DrawRequestEvent;

/// Request to undo the most recent state change.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct UndoRequestEvent;

/// Request to start a new game. `seed = None` uses a system-time seed.
/// `mode = None` reuses the current game's `GameMode`.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct NewGameRequestEvent {
    pub seed: Option<u64>,
    pub mode: Option<GameMode>,
}

/// Fired by `GamePlugin` after any successful state mutation. Rendering and
/// score-display systems listen for this to refresh.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct StateChangedEvent;

/// Fired by input/UI systems when a player attempts to drop dragged cards
/// on a real pile but the move violates the rules. Drives the
/// `card_invalid.wav` SFX. Not fired for drops in empty space.
#[derive(Message, Debug, Clone)]
pub struct MoveRejectedEvent {
    pub from: PileType,
    pub to: PileType,
    pub count: usize,
}

/// Fired once when the active game transitions to won.
#[derive(Message, Debug, Clone, Copy)]
pub struct GameWonEvent {
    pub score: i32,
    pub time_seconds: u64,
}

/// Fired when a card's face-up state changes during gameplay.
#[derive(Message, Debug, Clone, Copy)]
pub struct CardFlippedEvent(pub u32);

/// Fired by the flip animation at its midpoint — the instant the card face
/// becomes visible (scale.x crosses zero and the phase switches to ScalingUp).
///
/// Audio systems should listen to this event rather than `CardFlippedEvent`
/// so the flip sound is synchronised with the visual reveal, not the move
/// that triggered the animation.
#[derive(Message, Debug, Clone, Copy)]
pub struct CardFaceRevealedEvent(pub u32);

/// Achievement unlocked notification carrying the full `AchievementRecord` for
/// the newly unlocked achievement. Consumed by the toast renderer and any
/// persistence/UI systems that need unlock metadata.
#[derive(Message, Debug, Clone)]
pub struct AchievementUnlockedEvent(pub AchievementRecord);

/// Request to manually trigger a sync pull from the active backend.
///
/// Fired by the Settings panel "Sync Now" button. `SyncPlugin` responds by
/// starting a new pull task if one is not already in flight.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct ManualSyncRequestEvent;

/// Fired by `InputPlugin` when N is pressed while a game is in progress
/// but confirmation has not yet been received. The animation plugin shows
/// a "Press N again to confirm" toast. A second N press within the
/// confirmation window sends `NewGameRequestEvent`.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct NewGameConfirmEvent;

/// Generic informational toast message. Any system can fire this to display
/// a short string to the player, e.g. "Locked — reach level 5".
#[derive(Message, Debug, Clone)]
pub struct InfoToastEvent(pub String);

/// Fired by `ProgressPlugin` immediately after awarding XP for a win so the
/// animation layer can display a "+N XP" toast alongside the win cascade.
#[derive(Message, Debug, Clone, Copy)]
pub struct XpAwardedEvent {
    pub amount: u64,
}

/// Fired by `InputPlugin` when the player presses G to forfeit the current
/// game. Consumed by `StatsPlugin` which records the abandoned game,
/// persists stats, and starts a fresh deal.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct ForfeitEvent;

/// Fired when the player requests a hint (H key). Carries the source card ID
/// and destination pile for visual highlighting.
///
/// Consumed by `CardPlugin` (to apply `HintHighlight` on the card entity) and
/// `TablePlugin` (to tint the destination `PileMarker` gold for 2 s).
#[derive(Message, Debug, Clone)]
pub struct HintVisualEvent {
    /// The `Card::id` of the source card to be highlighted.
    pub source_card_id: u32,
    /// The destination pile whose `PileMarker` should be tinted gold.
    pub dest_pile: solitaire_core::pile::PileType,
}
