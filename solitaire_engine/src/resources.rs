//! Bevy resources owned by the engine crate.

use std::sync::Arc;

use bevy::math::Vec2;
use bevy::prelude::{warn, Resource};
use chrono::{DateTime, Utc};
use solitaire_core::game_state::GameState;
use solitaire_core::pile::PileType;

/// Wraps the currently active `GameState`. Single source of truth for the in-progress game.
#[derive(Resource, Debug, Clone)]
pub struct GameStateResource(pub GameState);

/// Tracks an in-progress drag operation.
///
/// When `cards` is empty there is no active drag. When non-empty, the listed
/// cards are being moved by the user and should be rendered at the cursor or
/// touch position.
///
/// # Drag threshold
///
/// A drag is *pending* when `!cards.is_empty() && !committed`. The drag does
/// not become *committed* (cards do not visually move) until the pointer has
/// moved at least `AnimationTuning::drag_threshold_px` pixels from `press_pos`.
/// This prevents accidental drags on quick taps, especially on touch screens.
#[derive(Resource, Debug, Clone)]
pub struct DragState {
    /// IDs of the cards being dragged (bottom-to-top stacking order).
    pub cards: Vec<u32>,
    /// Pile the drag originated from.
    pub origin_pile: Option<PileType>,
    /// World-space offset from the cursor/touch to the bottom card's centre.
    pub cursor_offset: Vec2,
    /// Z coordinate used for the dragged cards.
    pub origin_z: f32,
    /// Screen-space position (logical pixels) where the press/touch began.
    ///
    /// Used to measure whether the drag threshold has been crossed.
    pub press_pos: Vec2,
    /// Whether the drag threshold has been crossed and visual drag is active.
    ///
    /// Cards are only lifted and repositioned once `committed = true`.
    pub committed: bool,
    /// Touch ID driving this drag, or `None` for a mouse drag.
    pub active_touch_id: Option<u64>,
}

impl Default for DragState {
    fn default() -> Self {
        Self {
            cards: Vec::new(),
            origin_pile: None,
            cursor_offset: Vec2::ZERO,
            origin_z: 0.0,
            press_pos: Vec2::ZERO,
            committed: false,
            active_touch_id: None,
        }
    }
}

impl DragState {
    /// Returns `true` when no drag (pending or committed) is in progress.
    pub fn is_idle(&self) -> bool {
        self.cards.is_empty()
    }

    /// Returns `true` when a drag has been committed (cards are visually lifted).
    pub fn is_committed(&self) -> bool {
        self.committed
    }

    /// Resets all drag state to the idle/default values.
    pub fn clear(&mut self) {
        self.cards.clear();
        self.origin_pile = None;
        self.cursor_offset = Vec2::ZERO;
        self.origin_z = 0.0;
        self.press_pos = Vec2::ZERO;
        self.committed = false;
        self.active_touch_id = None;
    }
}

/// Current sync activity — shown in the settings screen.
///
/// Defined here rather than in `solitaire_data` because it is a UI/runtime
/// status value, not part of the persistence layer.
#[derive(Debug, Clone, Default)]
pub enum SyncStatus {
    #[default]
    Idle,
    Syncing,
    LastSynced(DateTime<Utc>),
    Error(String),
}

/// Bevy resource wrapping the current `SyncStatus`.
#[derive(Resource, Debug, Clone, Default)]
pub struct SyncStatusResource(pub SyncStatus);

/// Tracks which hint the player is currently cycling through.
///
/// Incremented on each H press so repeated presses reveal different moves.
/// Reset to `0` whenever the game state changes (move, draw, undo, new game).
#[derive(Resource, Debug, Clone, Default)]
pub struct HintCycleIndex(pub usize);

/// Remembers the vertical scroll offset of the Settings panel between open/close cycles.
///
/// Saved when the panel is despawned and restored on next spawn so the player
/// returns to the same position in the list without re-scrolling.
#[derive(Resource, Debug, Clone, Default)]
pub struct SettingsScrollPos(pub f32);

/// Set to `true` by an input system when a touch tap is consumed by game logic
/// (e.g. drawing from stock). `toggle_hud_on_tap` checks this flag on
/// `TouchPhase::Ended` and skips the HUD visibility toggle when set, then
/// resets it to `false` so subsequent taps behave normally.
#[derive(Resource, Debug, Clone, Default)]
pub struct GameInputConsumedResource(pub bool);

/// Shared Tokio runtime used by all async-task closures that need HTTP I/O.
///
/// Bevy's `AsyncComputeTaskPool` uses `async-executor` (not Tokio), so spawned
/// closures that call `reqwest`/`hyper` need a Tokio reactor. A single
/// multi-threaded runtime is built once at startup and its `Arc` cloned cheaply
/// into every network task — safe for concurrent `block_on` calls from multiple
/// worker threads.
#[derive(Resource, Clone)]
pub struct TokioRuntimeResource(pub Arc<tokio::runtime::Runtime>);

impl TokioRuntimeResource {
    /// Attempts to build the shared multi-threaded Tokio runtime.
    ///
    /// Returns `Err` if the OS refuses to create worker threads (e.g. resource
    /// limits on Android). Callers should log the error and disable sync
    /// features rather than panicking.
    pub fn new() -> Result<Self, tokio::io::Error> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        Ok(Self(Arc::new(rt)))
    }
}

impl Default for TokioRuntimeResource {
    fn default() -> Self {
        // Try multi-threaded first; fall back to current-thread (single
        // worker) if the OS refuses to create additional threads. Neither
        // path uses `.expect()` so this never panics at startup.
        match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(rt) => Self(Arc::new(rt)),
            Err(e) => {
                warn!(
                    "sync: failed to build multi-thread Tokio runtime ({e}); \
                     falling back to current-thread runtime"
                );
                // current_thread runtime never spawns OS threads, so it
                // succeeds even under tight sandboxing.
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect(
                        "current-thread Tokio runtime failed — \
                         the process cannot do any async I/O",
                    );
                Self(Arc::new(rt))
            }
        }
    }
}
