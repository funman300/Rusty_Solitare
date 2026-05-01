//! Bevy integration layer for Solitaire Quest.

pub mod card_animation;
pub mod achievement_plugin;
pub mod animation_plugin;
pub mod auto_complete_plugin;
pub mod audio_plugin;
pub mod card_plugin;
pub mod font_plugin;
pub mod feedback_anim_plugin;
pub mod challenge_plugin;
pub mod cursor_plugin;
pub mod daily_challenge_plugin;
pub mod events;
pub mod game_plugin;
pub mod help_plugin;
pub mod home_plugin;
pub mod hud_plugin;
pub mod leaderboard_plugin;
pub mod input_plugin;
pub mod layout;
pub mod onboarding_plugin;
pub mod pause_plugin;
pub mod profile_plugin;
pub mod settings_plugin;
pub mod progress_plugin;
pub mod resources;
pub mod selection_plugin;
pub mod splash_plugin;
pub mod stats_plugin;
pub mod sync_plugin;
pub mod table_plugin;
pub mod time_attack_plugin;
pub mod ui_focus;
pub mod ui_modal;
pub mod ui_theme;
pub mod ui_tooltip;
pub mod weekly_goals_plugin;
pub mod win_summary_plugin;

pub use achievement_plugin::{AchievementPlugin, AchievementsResource, AchievementsScreen};
pub use challenge_plugin::{
    challenge_progress_label, ChallengeAdvancedEvent, ChallengePlugin, CHALLENGE_UNLOCK_LEVEL,
};
pub use daily_challenge_plugin::{
    DailyChallengeCompletedEvent, DailyChallengePlugin, DailyChallengeResource,
};
pub use progress_plugin::{LevelUpEvent, ProgressPlugin, ProgressResource, ProgressUpdate};
pub use weekly_goals_plugin::{WeeklyGoalCompletedEvent, WeeklyGoalsPlugin};
pub use animation_plugin::{ActiveToast, AnimationPlugin, CardAnim, ToastEntity, ToastQueue};
pub use card_animation::{
    CardAnimation, CardAnimationPlugin, MotionCurve, WinCascadePlugin,
    retarget_animation, sample_curve, compute_duration, cascade_delay, micro_vary,
    HoverState, InputBuffer, BufferedInput,
    win_scatter_targets, WIN_CASCADE_INTERVAL_SECS, DEAL_INTERVAL_SECS,
    MIN_DURATION_SECS, MAX_DURATION_SECS,
    AnimationChain,
    AnimationTuning, InputPlatform,
    FrameTimeDiagnostics, DIAG_WINDOW_SIZE,
};
pub use feedback_anim_plugin::{
    deal_stagger_delay, deal_stagger_secs_for_speed, shake_offset, settle_scale,
    FeedbackAnimPlugin, SettleAnim, ShakeAnim,
};
pub use auto_complete_plugin::AutoCompletePlugin;
pub use audio_plugin::{AudioPlugin, AudioState, SoundLibrary};
pub use card_plugin::{
    CardEntity, CardImageSet, CardLabel, CardPlugin, HintHighlight, HintHighlightTimer,
    RightClickHighlight, RightClickHighlightTimer,
};
pub use font_plugin::{FontPlugin, FontResource};
pub use cursor_plugin::CursorPlugin;
pub use events::{
    AchievementUnlockedEvent, CardFaceRevealedEvent, CardFlippedEvent, DrawRequestEvent,
    ForfeitEvent, ForfeitRequestEvent, GameWonEvent, HelpRequestEvent, HintVisualEvent,
    InfoToastEvent, ManualSyncRequestEvent, MoveRejectedEvent, MoveRequestEvent,
    NewGameConfirmEvent, NewGameRequestEvent, PauseRequestEvent, StartChallengeRequestEvent,
    StartDailyChallengeRequestEvent, StartTimeAttackRequestEvent, StartZenRequestEvent,
    StateChangedEvent, SyncCompleteEvent, ToggleAchievementsRequestEvent,
    ToggleLeaderboardRequestEvent, ToggleProfileRequestEvent, ToggleSettingsRequestEvent,
    ToggleStatsRequestEvent, UndoRequestEvent, XpAwardedEvent,
};
pub use game_plugin::{ConfirmNewGameScreen, GameMutation, GameOverScreen, GamePlugin, GameStatePath};
pub use help_plugin::{HelpPlugin, HelpScreen};
pub use home_plugin::{HomePlugin, HomeScreen};
pub use hud_plugin::{
    ActionButton, HelpButton, HudAutoComplete, HudPlugin, MenuButton, MenuOption, MenuPopover,
    ModeOption, ModesButton, ModesPopover, NewGameButton, PauseButton, UndoButton,
};
pub use leaderboard_plugin::{LeaderboardPlugin, LeaderboardResource, LeaderboardScreen};
pub use input_plugin::InputPlugin;
pub use onboarding_plugin::{OnboardingPlugin, OnboardingScreen};
pub use pause_plugin::{ForfeitConfirmScreen, PausePlugin, PauseScreen, PausedResource};
pub use profile_plugin::{ProfilePlugin, ProfileScreen};
pub use settings_plugin::{
    PendingWindowGeometry, SettingsChangedEvent, SettingsPlugin, SettingsResource, SettingsScreen,
    SFX_STEP, WINDOW_GEOMETRY_DEBOUNCE_SECS,
};
pub use layout::{compute_layout, Layout, LayoutResource};
pub use resources::{DragState, GameStateResource, HintCycleIndex, SettingsScrollPos, SyncStatus, SyncStatusResource};
pub use selection_plugin::{SelectionHighlight, SelectionPlugin, SelectionState};
pub use splash_plugin::{SplashAge, SplashPlugin, SplashRoot};
pub use stats_plugin::{StatsPlugin, StatsResource, StatsScreen, StatsUpdate};
pub use sync_plugin::{SyncPlugin, SyncProviderResource};
pub use ui_focus::{Disabled, FocusGroup, Focusable, FocusedButton, UiFocusPlugin};
pub use ui_modal::{
    spawn_modal, spawn_modal_actions, spawn_modal_body_text, spawn_modal_button,
    spawn_modal_header, ButtonVariant, ModalActions, ModalBody, ModalButton, ModalCard,
    ModalHeader, ModalScrim, UiModalPlugin,
};
pub use ui_tooltip::{Tooltip, UiTooltipPlugin};
pub use table_plugin::{
    BackgroundImageSet, HintPileHighlight, PileMarker, TableBackground, TablePlugin,
};
pub use time_attack_plugin::{
    TimeAttackEndedEvent, TimeAttackPlugin, TimeAttackResource, TIME_ATTACK_DURATION_SECS,
};
pub use win_summary_plugin::{
    format_win_time, ScreenShakeResource, SessionAchievements, WinSummaryPending, WinSummaryPlugin,
};
