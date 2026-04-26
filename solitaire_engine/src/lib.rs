//! Bevy integration layer for Solitaire Quest.

pub mod achievement_plugin;
pub mod animation_plugin;
pub mod card_plugin;
pub mod challenge_plugin;
pub mod daily_challenge_plugin;
pub mod events;
pub mod game_plugin;
pub mod help_plugin;
pub mod input_plugin;
pub mod layout;
pub mod progress_plugin;
pub mod resources;
pub mod stats_plugin;
pub mod table_plugin;
pub mod time_attack_plugin;
pub mod weekly_goals_plugin;

pub use achievement_plugin::{AchievementPlugin, AchievementsResource};
pub use challenge_plugin::{
    challenge_progress_label, ChallengeAdvancedEvent, ChallengePlugin, CHALLENGE_UNLOCK_LEVEL,
};
pub use daily_challenge_plugin::{
    DailyChallengeCompletedEvent, DailyChallengePlugin, DailyChallengeResource,
};
pub use progress_plugin::{LevelUpEvent, ProgressPlugin, ProgressResource, ProgressUpdate};
pub use weekly_goals_plugin::{WeeklyGoalCompletedEvent, WeeklyGoalsPlugin};
pub use animation_plugin::{AnimationPlugin, CardAnim};
pub use card_plugin::{CardEntity, CardLabel, CardPlugin};
pub use events::{
    AchievementUnlockedEvent, CardFlippedEvent, DrawRequestEvent, GameWonEvent, MoveRequestEvent,
    NewGameRequestEvent, StateChangedEvent, UndoRequestEvent,
};
pub use game_plugin::{GameMutation, GamePlugin};
pub use help_plugin::{HelpPlugin, HelpScreen};
pub use input_plugin::InputPlugin;
pub use layout::{compute_layout, Layout, LayoutResource};
pub use resources::{DragState, GameStateResource, SyncStatus, SyncStatusResource};
pub use stats_plugin::{StatsPlugin, StatsResource, StatsScreen, StatsUpdate};
pub use table_plugin::{PileMarker, TableBackground, TablePlugin};
pub use time_attack_plugin::{
    TimeAttackEndedEvent, TimeAttackPlugin, TimeAttackResource, TIME_ATTACK_DURATION_SECS,
};
