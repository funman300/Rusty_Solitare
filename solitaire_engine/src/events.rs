//! Cross-system events used by the engine's plugins.

use bevy::prelude::Message;
use solitaire_core::card::Suit;
use solitaire_core::game_state::GameMode;
use solitaire_core::pile::PileType;
use solitaire_data::AchievementRecord;
use solitaire_sync::SyncResponse;

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
    /// `true` when this request originated from the user confirming the
    /// abandon-current-game modal (Y / Enter on `ConfirmNewGameScreen`).
    /// `handle_new_game` skips spawning the dialog when this is set,
    /// otherwise it would respawn the modal in the frame after the player
    /// presses Y (the despawn-on-Y has flushed by then) and the new game
    /// would never actually start.
    pub confirmed: bool,
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

/// Fired by `GamePlugin` whenever a successful move lands a card on a
/// foundation pile that, after the move, contains all 13 cards of its
/// suit (Ace → King). Drives the per-suit completion flourish — a brief
/// scale pulse on the King card and a golden tint on the foundation
/// pile marker — plus a short audio ping.
///
/// Fired once per per-suit completion. The fourth completion will
/// co-occur with `GameWonEvent` and the win cascade — they layer
/// cleanly because the flourish is purely decorative and lives on a
/// dedicated marker component.
///
/// This event is a UI/audio cue only. It does **not** cross
/// `solitaire_sync` and is not persisted.
#[derive(Message, Debug, Clone, Copy)]
pub struct FoundationCompletedEvent {
    /// Foundation pile slot (0..=3) that just reached 13 cards.
    pub slot: u8,
    /// The suit of the completed foundation, taken from the bottom card
    /// (always an Ace by construction).
    pub suit: Suit,
}

/// Fired by `StatsPlugin` when the player's `win_streak_current`
/// crosses one of the milestone thresholds in
/// [`crate::ui_theme::STREAK_MILESTONES`] (currently 3, 5, 10).
///
/// Fires only on the threshold crossing — i.e. when the previous
/// streak was below the threshold and the post-win streak is at or
/// above it — so subsequent wins past the highest milestone do not
/// retrigger the flourish.
///
/// Drives the HUD streak-milestone flourish (a brief scale pulse on
/// the score readout) and an informational toast. UI/audio cue only;
/// not persisted, not synchronised.
#[derive(Message, Debug, Clone, Copy)]
pub struct WinStreakMilestoneEvent {
    /// The new `win_streak_current` value at the moment the
    /// threshold was crossed. Always equal to a value in
    /// [`crate::ui_theme::STREAK_MILESTONES`].
    pub streak: u32,
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

/// Request to toggle the pause overlay. Fired by the HUD "Pause" button so
/// the same toggle path runs whether the player presses `Esc` or clicks.
/// Consumed by `pause_plugin::toggle_pause`, which honours the same drag /
/// game-over / selection guards either way.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct PauseRequestEvent;

/// Request to toggle the help / controls overlay. Fired by the HUD "Help"
/// button alongside the existing `F1` accelerator so the overlay is
/// reachable without a keyboard. Consumed by `help_plugin::toggle_help_screen`.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct HelpRequestEvent;

/// Request to start a Zen-mode game. Fired by the HUD Modes-popover "Zen"
/// row alongside the existing `Z` accelerator. The handler in
/// `input_plugin` enforces the level gate (Zen unlocks at level 5) and
/// shows an informational toast when locked.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct StartZenRequestEvent;

/// Request to start the next Challenge-mode game. Fired by the HUD
/// Modes-popover "Challenge" row alongside the existing `X` accelerator.
/// The handler in `challenge_plugin` enforces the level gate, picks the
/// next seed from `progress.challenge_index`, and writes the
/// corresponding `NewGameRequestEvent`.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct StartChallengeRequestEvent;

/// Request to start a Time Attack session. Fired by the HUD
/// Modes-popover "Time Attack" row alongside the existing `T`
/// accelerator. The handler in `time_attack_plugin` enforces the level
/// gate, initialises `TimeAttackResource`, and writes the corresponding
/// `NewGameRequestEvent`.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct StartTimeAttackRequestEvent;

/// Request to start today's Daily Challenge. Fired by the HUD
/// Modes-popover "Daily Challenge" row alongside the existing `C`
/// accelerator. The handler in `daily_challenge_plugin` reads
/// `DailyChallengeResource::seed` and writes a `NewGameRequestEvent`.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct StartDailyChallengeRequestEvent;

/// Request to toggle the Stats overlay. Fired by the HUD Menu-popover
/// "Stats" row alongside the existing `S` accelerator.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct ToggleStatsRequestEvent;

/// Request to toggle the Achievements overlay. Fired by the HUD
/// Menu-popover "Achievements" row alongside the existing `A` accelerator.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct ToggleAchievementsRequestEvent;

/// Request to toggle the Profile overlay. Fired by the HUD Menu-popover
/// "Profile" row alongside the existing `P` accelerator.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct ToggleProfileRequestEvent;

/// Request to toggle the Settings overlay. Fired by the HUD Menu-popover
/// "Settings" row alongside the existing `O` accelerator.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct ToggleSettingsRequestEvent;

/// Request to toggle the Leaderboard overlay. Fired by the HUD
/// Menu-popover "Leaderboard" row alongside the existing `L` accelerator.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct ToggleLeaderboardRequestEvent;

/// Fired by `SyncPlugin` after a pull task resolves and the merged result has
/// been persisted to disk. `Ok(SyncResponse)` carries the merged payload plus
/// any `ConflictReport`s the merge produced. `Err(String)` carries a
/// human-readable failure message (network, auth, serialization, etc.).
///
/// UI systems listen for this to refresh views without polling
/// `SyncStatusResource`. See [ARCHITECTURE.md §4](../../ARCHITECTURE.md).
#[derive(Message, Debug, Clone)]
pub struct SyncCompleteEvent(pub Result<SyncResponse, String>);

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

/// Request to open the forfeit-confirm modal. Fired by the `G` accelerator
/// and by the Pause modal's "Forfeit" button so the same modal opens
/// either way. Consumed by `PausePlugin`, which spawns
/// `ForfeitConfirmScreen` after checking that a game is in progress and
/// no forfeit modal is already showing. Confirmation inside that modal
/// then fires `ForfeitEvent` for `StatsPlugin` to consume.
#[derive(Message, Debug, Clone, Copy, Default)]
pub struct ForfeitRequestEvent;

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
