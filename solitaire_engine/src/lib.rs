//! Bevy integration layer for Solitaire Quest.
//!
//! Currently exposes `GamePlugin` plus the resources and events it owns.
//! Additional plugins (`TablePlugin`, `CardPlugin`, `InputPlugin`,
//! `AnimationPlugin`, etc.) land in later sub-phases of Phase 3.

pub mod card_plugin;
pub mod events;
pub mod game_plugin;
pub mod input_plugin;
pub mod layout;
pub mod resources;
pub mod table_plugin;

pub use card_plugin::{CardEntity, CardLabel, CardPlugin};
pub use events::{
    CardFlippedEvent, DrawRequestEvent, GameWonEvent, MoveRequestEvent, NewGameRequestEvent,
    StateChangedEvent, UndoRequestEvent,
};
pub use game_plugin::{GameMutation, GamePlugin};
pub use input_plugin::InputPlugin;
pub use layout::{compute_layout, Layout, LayoutResource};
pub use resources::{DragState, GameStateResource, SyncStatus, SyncStatusResource};
pub use table_plugin::{PileMarker, TableBackground, TablePlugin};
