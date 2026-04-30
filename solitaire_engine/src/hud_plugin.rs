//! Persistent in-game HUD: score, move count, elapsed time, mode badge,
//! daily-challenge constraint, and undo count.
//!
//! The HUD spawns once at startup and lives for the app's lifetime. Text is
//! refreshed whenever `GameStateResource` changes (which happens on every move
//! and every elapsed-time tick), so score, moves, and timer all stay current
//! without a separate tick system.

use bevy::prelude::*;
use solitaire_core::card::Suit;
use solitaire_core::game_state::{DrawMode, GameMode};
use solitaire_core::pile::PileType;

use crate::auto_complete_plugin::AutoCompleteState;
use crate::challenge_plugin::CHALLENGE_UNLOCK_LEVEL;
use crate::daily_challenge_plugin::DailyChallengeResource;
use crate::progress_plugin::ProgressResource;
use crate::settings_plugin::SettingsResource;
use crate::ui_theme::{
    scaled_duration, ACCENT_PRIMARY, ACCENT_SECONDARY, BG_ELEVATED, BG_ELEVATED_HI,
    BG_ELEVATED_PRESSED, BORDER_SUBTLE, MOTION_SCORE_PULSE_SECS, RADIUS_MD, RADIUS_SM,
    STATE_DANGER, STATE_INFO, STATE_SUCCESS, STATE_WARNING, TEXT_PRIMARY, TEXT_SECONDARY,
    TYPE_BODY, TYPE_BODY_LG, TYPE_CAPTION, TYPE_HEADLINE, VAL_SPACE_1, VAL_SPACE_2, VAL_SPACE_3,
};
use crate::events::{
    HelpRequestEvent, InfoToastEvent, NewGameRequestEvent, PauseRequestEvent,
    StartChallengeRequestEvent, StartDailyChallengeRequestEvent, StartTimeAttackRequestEvent,
    StartZenRequestEvent, ToggleAchievementsRequestEvent, ToggleLeaderboardRequestEvent,
    ToggleProfileRequestEvent, ToggleSettingsRequestEvent, ToggleStatsRequestEvent,
    UndoRequestEvent,
};
use crate::font_plugin::FontResource;
use crate::game_plugin::GameMutation;
use crate::resources::GameStateResource;
use crate::selection_plugin::SelectionState;
use crate::time_attack_plugin::TimeAttackResource;
use crate::ui_focus::{FocusGroup, Focusable};
use crate::ui_tooltip::Tooltip;

/// Marker on the score text node.
#[derive(Component, Debug)]
pub struct HudScore;

/// Marker on the move-count text node.
#[derive(Component, Debug)]
pub struct HudMoves;

/// Marker on the elapsed-time text node.
#[derive(Component, Debug)]
pub struct HudTime;

/// Marker on the mode badge text node.
#[derive(Component, Debug)]
pub struct HudMode;

/// Marker on the daily-challenge constraint text node.
///
/// Displays the active goal (time limit or score target) when a daily challenge
/// is in progress. Empty string when no challenge is active or the game is won.
#[derive(Component, Debug)]
pub struct HudChallenge;

/// Marker on the undo-count text node.
///
/// Shows how many undos have been used this game. Displayed in amber when
/// `undo_count > 0` because using undo blocks the no-undo achievement.
#[derive(Component, Debug)]
pub struct HudUndos;

/// Marker on the auto-complete badge text node.
///
/// Displays `"AUTO"` in green while `AutoCompleteState.active` is true;
/// empty string otherwise.
#[derive(Component, Debug)]
pub struct HudAutoComplete;

/// Marker on the stock-recycle counter text node.
///
/// Displays `"Recycles: N"` whenever `recycle_count > 0`, regardless of draw
/// mode, so the player can track stock recycling in both Draw-One and
/// Draw-Three (relevant to the `comeback` achievement). Hidden (empty string)
/// until the first recycle occurs.
#[derive(Component, Debug)]
pub struct HudRecycles;

/// Marker on the draw-cycle indicator text node.
///
/// Only shown in Draw-Three mode. Displays `"Cycle: N/3"` where N is the
/// number of cards that will be drawn on the next stock click
/// (`min(stock_len, 3)`). Shows `"Cycle: 0/3"` when the stock is empty
/// (recycle available). Hidden (empty string) in Draw-One mode or after the
/// game is won.
#[derive(Component, Debug)]
pub struct HudDrawCycle;

/// Marker on the keyboard-selection indicator text node.
///
/// Displays `"▶ {pile_name}"` while a pile is selected via Tab, or an empty
/// string when no pile is selected. Uses a light-yellow colour so it stands
/// out from the other white HUD items.
#[derive(Component, Debug)]
pub struct HudSelection;

/// Drives the score-readout pulse: scales the [`HudScore`] text from
/// 1.0 → 1.1 → 1.0 over [`MOTION_SCORE_PULSE_SECS`] (scaled by
/// [`AnimSpeed`](solitaire_data::AnimSpeed)). Inserted on the score
/// entity whenever the score increases; removed once `elapsed >=
/// duration`.
#[derive(Component, Debug, Clone, Copy)]
pub struct ScorePulse {
    /// Seconds elapsed since the pulse started.
    pub elapsed: f32,
    /// Total duration. Zero under `AnimSpeed::Instant` — the system
    /// snaps the scale back to 1.0 on first tick so no half-state
    /// is ever shown.
    pub duration: f32,
}

/// Marker on a transient floating "+N" text spawned next to the score
/// readout when the score jumps by [`SCORE_FLOATER_THRESHOLD`] or more.
/// Drifts upward and fades out over `MOTION_SCORE_PULSE_SECS * 2`,
/// then despawns. Kept rare/meaningful by the threshold gate.
#[derive(Component, Debug, Clone, Copy)]
pub struct ScoreFloater {
    /// Seconds elapsed since the floater spawned.
    pub elapsed: f32,
    /// Total lifetime. Zero under `AnimSpeed::Instant` — the system
    /// despawns it on first tick.
    pub duration: f32,
}

/// Tracks the score from the previous frame so the HUD can detect
/// changes without a `ScoreChangedEvent`. The plugin wires this to the
/// pulse + floater systems on every `Update`.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct PreviousScore(pub i32);

/// Score increase (in points) below which no floating "+N" is spawned.
/// 50 keeps the feedback for foundation drops and tableau-to-foundation
/// promotions; single-card placements (which can earn as little as +5)
/// stay quiet so the floater feels like a reward instead of noise.
pub const SCORE_FLOATER_THRESHOLD: i32 = 50;

/// Marker shared by every clickable HUD action button so a single
/// `paint_action_buttons` system can recolour them on hover/press without
/// each button needing its own paint handler.
#[derive(Component, Debug)]
pub struct ActionButton;

/// Marker on the "New Game" action button anchored top-right of the play
/// area. Click fires [`NewGameRequestEvent`]; the existing
/// `ConfirmNewGameScreen` modal handles confirmation when a game is in
/// progress.
#[derive(Component, Debug)]
pub struct NewGameButton;

/// Marker on the "Undo" action button. Click fires [`UndoRequestEvent`],
/// mirroring the `U` keyboard accelerator.
#[derive(Component, Debug)]
pub struct UndoButton;

/// Marker on the "Pause" action button. Click fires [`PauseRequestEvent`],
/// mirroring the `Esc` keyboard accelerator. The pause overlay's own resume
/// affordance dismisses it from the paused state.
#[derive(Component, Debug)]
pub struct PauseButton;

/// Marker on the "Help" action button. Click fires [`HelpRequestEvent`],
/// mirroring the `F1` keyboard accelerator.
#[derive(Component, Debug)]
pub struct HelpButton;

/// Marker on the "Modes" action button. Click toggles the [`ModesPopover`]
/// (a small dropdown panel) below the action bar. Each popover row starts
/// the corresponding game mode.
#[derive(Component, Debug)]
pub struct ModesButton;

/// Marker on the dropdown panel that opens below the [`ModesButton`].
/// Spawned on first click, despawned on second click or on mode select.
#[derive(Component, Debug)]
pub struct ModesPopover;

/// One row inside the [`ModesPopover`]. The variant carries which event
/// the click handler should fire — Classic uses `NewGameRequestEvent`
/// directly, the others go through their `Start*RequestEvent` so the
/// existing keyboard handler's level gate / resource setup runs.
#[derive(Component, Debug, Clone, Copy)]
pub enum ModeOption {
    Classic,
    DailyChallenge,
    Zen,
    Challenge,
    TimeAttack,
}

/// Marker on the "Menu" action button. Click toggles the [`MenuPopover`]
/// which exposes the Stats / Achievements / Profile / Settings /
/// Leaderboard overlays without needing the S/A/P/O/L hotkeys.
#[derive(Component, Debug)]
pub struct MenuButton;

/// Marker on the dropdown panel that opens below the [`MenuButton`].
#[derive(Component, Debug)]
pub struct MenuPopover;

/// One row inside the [`MenuPopover`]. The variant selects which
/// `Toggle*RequestEvent` the click handler fires.
#[derive(Component, Debug, Clone, Copy)]
pub enum MenuOption {
    Stats,
    Achievements,
    Profile,
    Settings,
    Leaderboard,
}

/// HUD Z-layer — above cards (which start at z=0) but below overlay screens.
/// Mirrors `ui_theme::Z_HUD` and is duplicated here only so the hud module
/// can use it as a `const` without a non-const expression in `ZIndex(...)`.
const Z_HUD: i32 = crate::ui_theme::Z_HUD;

/// Idle / hover / pressed colours shared by every action button. Aliased
/// to the theme tokens so the HUD picks up palette changes for free.
const ACTION_BTN_IDLE: Color = BG_ELEVATED;
const ACTION_BTN_HOVER: Color = BG_ELEVATED_HI;
const ACTION_BTN_PRESSED: Color = BG_ELEVATED_PRESSED;

/// Renders the in-game HUD: score counter, move counter, elapsed timer, draw-mode indicator, and the auto-complete badge that lights up when the game is solvable without further input.
pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        // The click handlers write to messages registered elsewhere by their
        // owning plugins (`GamePlugin`, `PausePlugin`, `HelpPlugin`,
        // `challenge_plugin`, `daily_challenge_plugin`, `time_attack_plugin`,
        // `input_plugin`). Re-register defensively so the HUD plugin works in
        // isolation under `MinimalPlugins` (tests). `add_message` is
        // idempotent.
        app.add_message::<NewGameRequestEvent>()
            .add_message::<UndoRequestEvent>()
            .add_message::<PauseRequestEvent>()
            .add_message::<HelpRequestEvent>()
            .add_message::<StartZenRequestEvent>()
            .add_message::<StartChallengeRequestEvent>()
            .add_message::<StartTimeAttackRequestEvent>()
            .add_message::<StartDailyChallengeRequestEvent>()
            .add_message::<ToggleStatsRequestEvent>()
            .add_message::<ToggleAchievementsRequestEvent>()
            .add_message::<ToggleProfileRequestEvent>()
            .add_message::<ToggleSettingsRequestEvent>()
            .add_message::<ToggleLeaderboardRequestEvent>()
            .init_resource::<PreviousScore>()
            .add_systems(Startup, (spawn_hud, spawn_action_buttons))
            .add_systems(Update, update_hud.after(GameMutation))
            .add_systems(Update, announce_auto_complete.after(GameMutation))
            .add_systems(Update, update_selection_hud)
            .add_systems(
                Update,
                (
                    detect_score_change,
                    advance_score_pulse,
                    advance_score_floater,
                )
                    .chain()
                    .after(GameMutation),
            )
            .add_systems(
                Update,
                (
                    handle_new_game_button,
                    handle_undo_button,
                    handle_pause_button,
                    handle_help_button,
                    handle_modes_button,
                    handle_mode_option_click,
                    handle_menu_button,
                    handle_menu_option_click,
                    paint_action_buttons,
                ),
            );
    }
}

/// Spawns the in-game HUD as a 4-tier vertical column anchored to the
/// top-left of the play area.
///
/// Tiers (top to bottom):
///   1. **Primary** — Score (display weight) · Moves · Timer.
///      Always visible during gameplay.
///   2. **Mode context** — Mode badge · Daily-challenge constraint ·
///      Draw-cycle indicator. Each cell is empty when not relevant; the
///      row collapses visually when all cells are empty.
///   3. **Penalty / bonus** — Undos · Recycles · Auto-complete badge.
///      Both penalty counters share `STATE_WARNING` (the audit found
///      they were inconsistent: Undos amber, Recycles white).
///   4. **Selection** — keyboard-driven pile selector chip.
///
/// The audit identified the original single-row layout (10 readouts in
/// one horizontal flex row, 5+ colour families competing) as the
/// player's #1 complaint. This restructure groups by purpose, lets
/// transient items disappear cleanly, and uses the typography scale to
/// make Score the visual protagonist.
fn spawn_hud(font_res: Option<Res<FontResource>>, mut commands: Commands) {
    let font_handle = font_res.as_ref().map(|f| f.0.clone()).unwrap_or_default();
    let font_score = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_HEADLINE,
        ..default()
    };
    let font_lg = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_BODY_LG,
        ..default()
    };
    let font_body = TextFont {
        font: font_handle,
        font_size: TYPE_BODY,
        ..default()
    };

    let row_node = || Node {
        flex_direction: FlexDirection::Row,
        column_gap: VAL_SPACE_3,
        align_items: AlignItems::Baseline,
        ..default()
    };

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: VAL_SPACE_3,
                top: VAL_SPACE_2,
                flex_direction: FlexDirection::Column,
                row_gap: VAL_SPACE_1,
                ..default()
            },
            ZIndex(Z_HUD),
        ))
        .with_children(|hud| {
            // Tier 1 — primary readouts. Score is the protagonist (HEADLINE);
            // Moves and Timer are supporting context (BODY_LG, secondary tone).
            hud.spawn(row_node()).with_children(|t1| {
                t1.spawn((
                    HudScore,
                    Tooltip::new("Points earned this game. Hidden in Zen mode."),
                    Text::new("Score: 0"),
                    font_score.clone(),
                    TextColor(TEXT_PRIMARY),
                ));
                t1.spawn((
                    HudMoves,
                    Tooltip::new(
                        "Moves you've made this game. Counts placements and stock draws.",
                    ),
                    Text::new("Moves: 0"),
                    font_lg.clone(),
                    TextColor(TEXT_SECONDARY),
                ));
                t1.spawn((
                    HudTime,
                    Tooltip::new("Time on this game. Counts down in Time Attack."),
                    Text::new("0:00"),
                    font_lg.clone(),
                    TextColor(TEXT_SECONDARY),
                ));
            });

            // Tier 2 — mode context. Each cell is empty until update_hud
            // populates it (and clears it when no longer relevant), so the
            // row collapses when nothing in this tier applies.
            hud.spawn(row_node()).with_children(|t2| {
                t2.spawn((
                    HudMode,
                    Tooltip::new("Active game mode. Click Modes to switch."),
                    Text::new(""),
                    font_body.clone(),
                    TextColor(ACCENT_PRIMARY),
                ));
                t2.spawn((
                    HudChallenge,
                    Tooltip::new("Today's daily challenge target. Beat it for bonus XP."),
                    Text::new(""),
                    font_body.clone(),
                    TextColor(STATE_INFO),
                ));
                t2.spawn((
                    HudDrawCycle,
                    Tooltip::new("Cards drawn on the next stock click in Draw-Three."),
                    Text::new(""),
                    font_body.clone(),
                    TextColor(STATE_INFO),
                ));
            });

            // Tier 3 — penalty / bonus. Undos and Recycles share the
            // warning hue so they read as the same category ("you took a
            // penalty"); the auto-complete badge stays success-green.
            hud.spawn(row_node()).with_children(|t3| {
                t3.spawn((
                    HudUndos,
                    Tooltip::new(
                        "Undos used this game. Any undo blocks the No Undo achievement.",
                    ),
                    Text::new(""),
                    font_body.clone(),
                    TextColor(STATE_WARNING),
                ));
                t3.spawn((
                    HudRecycles,
                    Tooltip::new(
                        "Times you've recycled the stock. Three or more unlocks Comeback.",
                    ),
                    Text::new(""),
                    font_body.clone(),
                    TextColor(STATE_WARNING),
                ));
                t3.spawn((
                    HudAutoComplete,
                    Tooltip::new("Board is solvable from here. Press Enter to auto-finish."),
                    Text::new(""),
                    font_body.clone(),
                    TextColor(STATE_SUCCESS),
                ));
            });

            // Tier 4 — selection chip. Stays in HUD for now; a future
            // pass can reposition it next to the selected pile.
            hud.spawn(row_node()).with_children(|t4| {
                t4.spawn((
                    HudSelection,
                    Tooltip::new("Pile selected with Tab. Use arrows or Enter to act."),
                    Text::new(""),
                    font_body,
                    TextColor(ACCENT_SECONDARY),
                ));
            });
        });
}

/// Spawns the action button bar anchored to the top-right of the window.
/// Each child is a clickable button mirroring a keyboard accelerator —
/// per the UI-first principle (CLAUDE.md / ARCHITECTURE.md §1) the buttons
/// are the primary entry point and the hotkeys are optional.
///
/// Order (left → right): Undo, Pause, Help, New Game. New Game is rightmost
/// because it's the most consequential action; the destructive button sits
/// on its own visual edge.
fn spawn_action_buttons(font_res: Option<Res<FontResource>>, mut commands: Commands) {
    let font = TextFont {
        font: font_res.as_ref().map(|f| f.0.clone()).unwrap_or_default(),
        font_size: 16.0,
        ..default()
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: VAL_SPACE_3,
                top: VAL_SPACE_2,
                flex_direction: FlexDirection::Row,
                column_gap: VAL_SPACE_2,
                align_items: AlignItems::Center,
                ..default()
            },
            ZIndex(Z_HUD),
        ))
        .with_children(|row| {
            // Menu and Modes don't have a single hotkey accelerator
            // (each row inside their popover has its own); their button
            // labels carry the dropdown chevron in lieu of a key chip.
            //
            // The trailing `order` argument is the per-button index in
            // visual reading order (left → right). It feeds
            // `Focusable { group: Hud, order }` so Tab cycles the action
            // bar in the same order the eye scans it.
            spawn_action_button(
                row,
                MenuButton,
                "Menu \u{25BE}",
                None,
                "Open Stats, Achievements, Profile, Settings, or Leaderboard.",
                &font,
                0,
            );
            spawn_action_button(
                row,
                UndoButton,
                "Undo",
                Some("U"),
                "Take back your last move. Costs points and blocks No Undo.",
                &font,
                1,
            );
            spawn_action_button(
                row,
                PauseButton,
                "Pause",
                Some("Esc"),
                "Pause the game and freeze the timer.",
                &font,
                2,
            );
            spawn_action_button(
                row,
                HelpButton,
                "Help",
                Some("F1"),
                "Show controls, rules, and keyboard shortcuts.",
                &font,
                3,
            );
            spawn_action_button(
                row,
                ModesButton,
                "Modes \u{25BE}",
                None,
                "Switch modes: Classic, Daily, Zen, Challenge, Time Attack.",
                &font,
                4,
            );
            spawn_action_button(
                row,
                NewGameButton,
                "New Game",
                Some("N"),
                "Start a fresh deal. Confirms first if a game is in progress.",
                &font,
                5,
            );
        });
}

/// Spawns a single action button as a child of `row`. Each button shares
/// the same node geometry, idle colour, and `ActionButton` marker so
/// `paint_action_buttons` can recolour all of them with one query.
///
/// `order` is the button's index inside the action bar (0 for the
/// leftmost). It propagates into the [`Focusable`] this function inserts
/// so Phase 2's keyboard focus ring cycles the HUD in visual order.
///
/// `tooltip` is the hover-reveal caption attached via [`Tooltip`]. Every
/// action button ships with one — there is no opt-out — because each button
/// represents a player-triggered action and benefits from a one-line
/// reminder of what it does.
#[allow(clippy::too_many_arguments)]
fn spawn_action_button<M: Component>(
    row: &mut ChildSpawnerCommands,
    marker: M,
    label: &str,
    hotkey: Option<&'static str>,
    tooltip: &'static str,
    font: &TextFont,
    order: i32,
) {
    let hotkey_font = TextFont {
        font: font.font.clone(),
        font_size: TYPE_CAPTION,
        ..default()
    };
    row.spawn((
        marker,
        ActionButton,
        Button,
        Tooltip::new(tooltip),
        // Joins the `Hud` focus group at the supplied order so Tab
        // cycles HUD buttons left-to-right under Phase 2. The HUD focus
        // ring still only engages when a HUD button is hovered (or in
        // future phases, when the player explicitly switches groups);
        // the marker just declares membership.
        Focusable {
            group: FocusGroup::Hud,
            order,
        },
        Node {
            padding: UiRect::axes(VAL_SPACE_3, VAL_SPACE_2),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
            column_gap: VAL_SPACE_2,
            ..default()
        },
        BackgroundColor(ACTION_BTN_IDLE),
        BorderColor::all(BORDER_SUBTLE),
    ))
    .with_children(|b| {
        b.spawn((Text::new(label), font.clone(), TextColor(TEXT_PRIMARY)));
        if let Some(key) = hotkey {
            // Hotkey hint rendered as a dim caption next to the label —
            // keeps the keyboard accelerator discoverable without
            // hijacking the button's primary affordance.
            b.spawn((Text::new(key), hotkey_font, TextColor(TEXT_SECONDARY)));
        }
    });
}

/// `Changed<Interaction>` filter ensures we only react on the frame the
/// interaction state transitions, avoiding repeat events while the button
/// is held down. Each click handler fires the corresponding request event,
/// which `pause_plugin` / `help_plugin` / `game_plugin` consume alongside
/// their existing keyboard handlers.
fn handle_new_game_button(
    interaction_query: Query<&Interaction, (With<NewGameButton>, Changed<Interaction>)>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
) {
    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed {
            new_game.write(NewGameRequestEvent::default());
        }
    }
}

fn handle_undo_button(
    interaction_query: Query<&Interaction, (With<UndoButton>, Changed<Interaction>)>,
    mut undo: MessageWriter<UndoRequestEvent>,
) {
    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed {
            undo.write(UndoRequestEvent);
        }
    }
}

fn handle_pause_button(
    interaction_query: Query<&Interaction, (With<PauseButton>, Changed<Interaction>)>,
    mut pause: MessageWriter<PauseRequestEvent>,
) {
    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed {
            pause.write(PauseRequestEvent);
        }
    }
}

fn handle_help_button(
    interaction_query: Query<&Interaction, (With<HelpButton>, Changed<Interaction>)>,
    mut help: MessageWriter<HelpRequestEvent>,
) {
    for interaction in &interaction_query {
        if *interaction == Interaction::Pressed {
            help.write(HelpRequestEvent);
        }
    }
}

/// Toggles the [`ModesPopover`]: spawns it on first click, despawns it on
/// second click. Mode rows are populated per the player's current level so
/// only unlocked options appear.
fn handle_modes_button(
    interaction_query: Query<&Interaction, (With<ModesButton>, Changed<Interaction>)>,
    popovers: Query<Entity, With<ModesPopover>>,
    progress: Option<Res<ProgressResource>>,
    daily: Option<Res<DailyChallengeResource>>,
    font_res: Option<Res<FontResource>>,
    mut commands: Commands,
) {
    let pressed = interaction_query
        .iter()
        .any(|i| *i == Interaction::Pressed);
    if !pressed {
        return;
    }
    if let Ok(entity) = popovers.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_modes_popover(
            &mut commands,
            progress.as_deref(),
            daily.as_deref(),
            font_res.as_deref(),
        );
    }
}

/// Spawns the modes popover anchored just below the action bar's right
/// edge. Always includes Classic; includes Daily Challenge when a daily
/// resource is loaded; includes Zen / Challenge / Time Attack once the
/// player reaches the challenge unlock level.
fn spawn_modes_popover(
    commands: &mut Commands,
    progress: Option<&ProgressResource>,
    daily: Option<&DailyChallengeResource>,
    font_res: Option<&FontResource>,
) {
    let level = progress.map_or(0, |p| p.0.level);
    let font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: 15.0,
        ..default()
    };

    // Each row carries a tooltip alongside its label so hover reveals
    // a one-line description of what the mode does — mirroring the
    // tooltips on the action-bar buttons that opened this popover.
    let mut rows: Vec<(ModeOption, &'static str, &'static str)> = vec![(
        ModeOption::Classic,
        "Classic",
        "Standard Klondike. Score, timer, and full progression.",
    )];
    if daily.is_some() {
        rows.push((
            ModeOption::DailyChallenge,
            "Daily Challenge",
            "Today's seeded deal. Same for every player worldwide.",
        ));
    }
    if level >= CHALLENGE_UNLOCK_LEVEL {
        rows.push((
            ModeOption::Zen,
            "Zen",
            "No timer, no score, no penalties. Just play.",
        ));
        rows.push((
            ModeOption::Challenge,
            "Challenge",
            "Hand-picked hard seeds. No undo allowed.",
        ));
        rows.push((
            ModeOption::TimeAttack,
            "Time Attack",
            "Win as many games as you can in ten minutes.",
        ));
    }

    commands
        .spawn((
            ModesPopover,
            Node {
                position_type: PositionType::Absolute,
                right: VAL_SPACE_3,
                top: Val::Px(50.0),
                flex_direction: FlexDirection::Column,
                row_gap: VAL_SPACE_1,
                padding: UiRect::all(VAL_SPACE_2),
                border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
                ..default()
            },
            BackgroundColor(BG_ELEVATED),
            ZIndex(Z_HUD + 5),
        ))
        .with_children(|panel| {
            for (option, label, tooltip) in rows {
                panel
                    .spawn((
                        option,
                        ActionButton,
                        Button,
                        Tooltip::new(tooltip),
                        Node {
                            padding: UiRect::axes(VAL_SPACE_3, Val::Px(6.0)),
                            justify_content: JustifyContent::FlexStart,
                            align_items: AlignItems::Center,
                            min_width: Val::Px(150.0),
                            border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                            ..default()
                        },
                        BackgroundColor(ACTION_BTN_IDLE),
                    ))
                    .with_children(|b| {
                        b.spawn((Text::new(label), font.clone(), TextColor(TEXT_PRIMARY)));
                    });
            }
        });
}

/// Dispatches the click on a popover row to the matching request event,
/// then despawns the popover.
///
/// Classic uses [`NewGameRequestEvent`] directly; the other modes use
/// their `Start*RequestEvent` so the existing keyboard handler runs
/// (level gates, `TimeAttackResource` setup, daily seed lookup, etc.) —
/// the popover stays a thin entry point and never duplicates that logic.
#[allow(clippy::too_many_arguments)]
fn handle_mode_option_click(
    interaction_query: Query<(&Interaction, &ModeOption), Changed<Interaction>>,
    popovers: Query<Entity, With<ModesPopover>>,
    mut new_game: MessageWriter<NewGameRequestEvent>,
    mut zen: MessageWriter<StartZenRequestEvent>,
    mut challenge: MessageWriter<StartChallengeRequestEvent>,
    mut time_attack: MessageWriter<StartTimeAttackRequestEvent>,
    mut daily: MessageWriter<StartDailyChallengeRequestEvent>,
    mut commands: Commands,
) {
    let mut clicked_any = false;
    for (interaction, option) in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }
        clicked_any = true;
        match option {
            ModeOption::Classic => {
                new_game.write(NewGameRequestEvent::default());
            }
            ModeOption::DailyChallenge => {
                daily.write(StartDailyChallengeRequestEvent);
            }
            ModeOption::Zen => {
                zen.write(StartZenRequestEvent);
            }
            ModeOption::Challenge => {
                challenge.write(StartChallengeRequestEvent);
            }
            ModeOption::TimeAttack => {
                time_attack.write(StartTimeAttackRequestEvent);
            }
        }
    }
    if clicked_any
        && let Ok(entity) = popovers.single() {
            commands.entity(entity).despawn();
        }
}

/// Toggles the [`MenuPopover`]: spawns it on first click, despawns it on
/// second click. The popover lists the five overlays previously only
/// reachable via the S / A / P / O / L hotkeys.
fn handle_menu_button(
    interaction_query: Query<&Interaction, (With<MenuButton>, Changed<Interaction>)>,
    popovers: Query<Entity, With<MenuPopover>>,
    font_res: Option<Res<FontResource>>,
    mut commands: Commands,
) {
    let pressed = interaction_query
        .iter()
        .any(|i| *i == Interaction::Pressed);
    if !pressed {
        return;
    }
    if let Ok(entity) = popovers.single() {
        commands.entity(entity).despawn();
    } else {
        spawn_menu_popover(&mut commands, font_res.as_deref());
    }
}

/// Spawns the menu popover anchored just below the action bar, with one
/// row per overlay. Each row dispatches its corresponding
/// `Toggle*RequestEvent` so the existing toggle handler runs (and the
/// HUD never duplicates spawn / despawn / fetch logic).
fn spawn_menu_popover(commands: &mut Commands, font_res: Option<&FontResource>) {
    let font = TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: 15.0,
        ..default()
    };

    // Each row carries a tooltip alongside its label so hover reveals
    // a one-line description of what each overlay shows — mirroring
    // the tooltips on the action-bar buttons that opened this popover.
    let rows: [(MenuOption, &'static str, &'static str); 5] = [
        (
            MenuOption::Stats,
            "Stats",
            "Lifetime totals: wins, streaks, fastest time, best score.",
        ),
        (
            MenuOption::Achievements,
            "Achievements",
            "Browse unlocked achievements and the rewards still ahead.",
        ),
        (
            MenuOption::Profile,
            "Profile",
            "Your level, XP progress, and sync status.",
        ),
        (
            MenuOption::Settings,
            "Settings",
            "Audio, animations, theme, draw mode, and sync.",
        ),
        (
            MenuOption::Leaderboard,
            "Leaderboard",
            "Top players from your sync server. Opt in from Profile.",
        ),
    ];

    commands
        .spawn((
            MenuPopover,
            Node {
                position_type: PositionType::Absolute,
                right: VAL_SPACE_3,
                top: Val::Px(50.0),
                flex_direction: FlexDirection::Column,
                row_gap: VAL_SPACE_1,
                padding: UiRect::all(VAL_SPACE_2),
                border_radius: BorderRadius::all(Val::Px(RADIUS_MD)),
                ..default()
            },
            BackgroundColor(BG_ELEVATED),
            ZIndex(Z_HUD + 5),
        ))
        .with_children(|panel| {
            for (option, label, tooltip) in rows {
                panel
                    .spawn((
                        option,
                        ActionButton,
                        Button,
                        Tooltip::new(tooltip),
                        Node {
                            padding: UiRect::axes(VAL_SPACE_3, Val::Px(6.0)),
                            justify_content: JustifyContent::FlexStart,
                            align_items: AlignItems::Center,
                            min_width: Val::Px(150.0),
                            border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                            ..default()
                        },
                        BackgroundColor(ACTION_BTN_IDLE),
                    ))
                    .with_children(|b| {
                        b.spawn((Text::new(label), font.clone(), TextColor(TEXT_PRIMARY)));
                    });
            }
        });
}

/// Dispatches the click on a menu row to the matching toggle event,
/// then despawns the popover.
#[allow(clippy::too_many_arguments)]
fn handle_menu_option_click(
    interaction_query: Query<(&Interaction, &MenuOption), Changed<Interaction>>,
    popovers: Query<Entity, With<MenuPopover>>,
    mut stats: MessageWriter<ToggleStatsRequestEvent>,
    mut achievements: MessageWriter<ToggleAchievementsRequestEvent>,
    mut profile: MessageWriter<ToggleProfileRequestEvent>,
    mut settings: MessageWriter<ToggleSettingsRequestEvent>,
    mut leaderboard: MessageWriter<ToggleLeaderboardRequestEvent>,
    mut commands: Commands,
) {
    let mut clicked_any = false;
    for (interaction, option) in &interaction_query {
        if *interaction != Interaction::Pressed {
            continue;
        }
        clicked_any = true;
        match option {
            MenuOption::Stats => {
                stats.write(ToggleStatsRequestEvent);
            }
            MenuOption::Achievements => {
                achievements.write(ToggleAchievementsRequestEvent);
            }
            MenuOption::Profile => {
                profile.write(ToggleProfileRequestEvent);
            }
            MenuOption::Settings => {
                settings.write(ToggleSettingsRequestEvent);
            }
            MenuOption::Leaderboard => {
                leaderboard.write(ToggleLeaderboardRequestEvent);
            }
        }
    }
    if clicked_any
        && let Ok(entity) = popovers.single() {
            commands.entity(entity).despawn();
        }
}

/// Visual feedback for every action button — paints idle / hover / pressed
/// states by mutating `BackgroundColor` whenever the interaction state
/// changes. One query covers all action buttons via the shared
/// `ActionButton` marker.
#[allow(clippy::type_complexity)]
fn paint_action_buttons(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (With<ActionButton>, Changed<Interaction>),
    >,
) {
    for (interaction, mut bg) in &mut buttons {
        bg.0 = match interaction {
            Interaction::Pressed => ACTION_BTN_PRESSED,
            Interaction::Hovered => ACTION_BTN_HOVER,
            Interaction::None => ACTION_BTN_IDLE,
        };
    }
}

/// Formats a time-limit value in seconds as `"mm:ss"` for HUD display.
///
/// For example `format_time_limit(300)` returns `"5:00"`.
pub fn format_time_limit(secs: u64) -> String {
    let m = secs / 60;
    let s = secs % 60;
    format!("{m}:{s:02}")
}

// ---------------------------------------------------------------------------
// Score-change feedback (G2)
//
// The flow for each Update tick:
//   1. `detect_score_change` diffs `GameStateResource.score` against
//      `PreviousScore`. On any positive delta it inserts/refreshes
//      `ScorePulse` on the score readout; on a delta ≥
//      `SCORE_FLOATER_THRESHOLD` it also spawns a floating "+N" UI text
//      anchored just below the score.
//   2. `advance_score_pulse` ticks the pulse component, applies the
//      triangular 1.0 → 1.1 → 1.0 scale curve, and removes the
//      component on completion.
//   3. `advance_score_floater` drifts each floater upward, fades it to
//      transparent, and despawns it when its lifetime expires.
//
// The threshold of 50 (a foundation promotion's typical bonus) keeps
// floaters rare and meaningful — see `SCORE_FLOATER_THRESHOLD`.
// ---------------------------------------------------------------------------

/// Triangular 1.0 → 1.1 → 1.0 curve used by the score pulse. Pure
/// function so the test suite can assert on the curve directly
/// without spinning up a Bevy app.
///
/// The brief proposed `if t < 0.5 { 1.0 + 0.2*t } else { 1.2 - 0.2*(t-0.5) }`,
/// but that yields a discontinuity at t=0.5 (jumps from 1.1 → 1.2) and
/// ends at 1.1 instead of 1.0. The corrected form below preserves the
/// intent ("1.0 → 1.1 → 1.0 over the duration") with a continuous
/// triangle peaking at 1.1.
fn score_pulse_scale(t: f32) -> f32 {
    let clamped = t.clamp(0.0, 1.0);
    if clamped < 0.5 {
        1.0 + 0.2 * clamped
    } else {
        1.1 - 0.2 * (clamped - 0.5)
    }
}

/// Vertical pixels the floating "+N" drifts up over its lifetime.
const FLOATER_DRIFT_PX: f32 = 40.0;

/// Diffs the current `GameStateResource.score` against
/// [`PreviousScore`]. On a positive delta:
///
/// - Inserts (or refreshes) a [`ScorePulse`] on every [`HudScore`] entity
///   so the readout pulses 1.0 → 1.1 → 1.0.
/// - When the delta is ≥ [`SCORE_FLOATER_THRESHOLD`], spawns a floating
///   "+N" UI text in `ACCENT_PRIMARY` anchored just below the score
///   readout (see the doc comment on [`ScoreFloater`] for why this is a
///   UI Node rather than a `Text2d`).
fn detect_score_change(
    game: Res<GameStateResource>,
    settings: Option<Res<SettingsResource>>,
    mut prev: ResMut<PreviousScore>,
    font_res: Option<Res<FontResource>>,
    score_q: Query<Entity, With<HudScore>>,
    mut commands: Commands,
) {
    let current = game.0.score;
    let delta = current - prev.0;
    prev.0 = current;
    if delta <= 0 {
        return;
    }

    let speed = settings
        .as_ref()
        .map(|s| s.0.animation_speed)
        .unwrap_or_default();
    let pulse_secs = scaled_duration(MOTION_SCORE_PULSE_SECS, speed);
    let floater_secs = scaled_duration(MOTION_SCORE_PULSE_SECS * 2.0, speed);

    // Refresh ScorePulse on every score readout entity (in practice
    // there's exactly one, but iterating is cheaper than asserting).
    for entity in &score_q {
        commands.entity(entity).insert(ScorePulse {
            elapsed: 0.0,
            duration: pulse_secs,
        });
    }

    if delta < SCORE_FLOATER_THRESHOLD {
        return;
    }

    let font = TextFont {
        font: font_res.as_ref().map(|f| f.0.clone()).unwrap_or_default(),
        font_size: TYPE_BODY_LG,
        ..default()
    };
    // Spawned as an absolutely-positioned UI Node so the floater rides
    // the same screen-coordinate system as the score readout. Using a
    // `Text2d` here would require translating UI layout coordinates to
    // world space every frame; a UI node piggybacks on the same
    // anchoring `update_hud` already uses for the score and stays
    // testable under `MinimalPlugins`.
    commands.spawn((
        ScoreFloater {
            elapsed: 0.0,
            duration: floater_secs,
        },
        Node {
            position_type: PositionType::Absolute,
            // Anchored next to the HUD column; matches the
            // `spawn_hud` left/top offsets so the floater appears
            // overlaid on the score line and drifts up from there.
            left: VAL_SPACE_3,
            top: Val::Px(0.0),
            ..default()
        },
        ZIndex(Z_HUD + 10),
        Text::new(format!("+{delta}")),
        font,
        TextColor(ACCENT_PRIMARY),
    ));
}

/// Advances every [`ScorePulse`], scaling its entity's `Transform`
/// using [`score_pulse_scale`]. Removes the component once
/// `elapsed >= duration` (or immediately under
/// [`AnimSpeed::Instant`](solitaire_data::AnimSpeed) where duration is
/// 0) and pins the scale back to 1.0 so no float drift survives.
fn advance_score_pulse(
    time: Res<Time>,
    mut commands: Commands,
    mut q: Query<(Entity, &mut ScorePulse, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (entity, mut pulse, mut transform) in &mut q {
        let t = if pulse.duration <= 0.0 {
            1.0
        } else {
            pulse.elapsed += dt;
            (pulse.elapsed / pulse.duration).clamp(0.0, 1.0)
        };
        let scale = score_pulse_scale(t);
        transform.scale = Vec3::new(scale, scale, 1.0);
        if t >= 1.0 {
            transform.scale = Vec3::ONE;
            commands.entity(entity).remove::<ScorePulse>();
        }
    }
}

/// Advances every [`ScoreFloater`]: drifts the node upward by up to
/// [`FLOATER_DRIFT_PX`] and fades the text colour to transparent over
/// its lifetime. Despawns the entity once `elapsed >= duration`.
fn advance_score_floater(
    time: Res<Time>,
    mut commands: Commands,
    mut nodes: Query<(Entity, &mut ScoreFloater, &mut Node, &mut TextColor)>,
) {
    let dt = time.delta_secs();
    for (entity, mut floater, mut node, mut color) in &mut nodes {
        let t = if floater.duration <= 0.0 {
            1.0
        } else {
            floater.elapsed += dt;
            (floater.elapsed / floater.duration).clamp(0.0, 1.0)
        };
        // Drift upward: top decreases as t grows. Starting top=0 keeps
        // the floater on the score line; ending at -FLOATER_DRIFT_PX
        // pulls it up off the readout.
        node.top = Val::Px(-FLOATER_DRIFT_PX * t);
        // Linear fade: ACCENT_PRIMARY at t=0 → fully transparent at t=1.
        let mut c = ACCENT_PRIMARY;
        c.set_alpha(1.0 - t);
        color.0 = c;
        if t >= 1.0 {
            commands.entity(entity).despawn();
        }
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn update_hud(
    game: Res<GameStateResource>,
    time_attack: Option<Res<TimeAttackResource>>,
    daily: Option<Res<DailyChallengeResource>>,
    auto_complete: Option<Res<AutoCompleteState>>,
    mut score_q: Query<
        &mut Text,
        (
            With<HudScore>,
            Without<HudMoves>,
            Without<HudTime>,
            Without<HudMode>,
            Without<HudChallenge>,
            Without<HudUndos>,
            Without<HudAutoComplete>,
            Without<HudRecycles>,
            Without<HudDrawCycle>,
            Without<HudSelection>,
        ),
    >,
    mut moves_q: Query<
        &mut Text,
        (
            With<HudMoves>,
            Without<HudScore>,
            Without<HudTime>,
            Without<HudMode>,
            Without<HudChallenge>,
            Without<HudUndos>,
            Without<HudAutoComplete>,
            Without<HudRecycles>,
            Without<HudDrawCycle>,
            Without<HudSelection>,
        ),
    >,
    mut time_q: Query<
        &mut Text,
        (
            With<HudTime>,
            Without<HudScore>,
            Without<HudMoves>,
            Without<HudMode>,
            Without<HudChallenge>,
            Without<HudUndos>,
            Without<HudAutoComplete>,
            Without<HudRecycles>,
            Without<HudDrawCycle>,
            Without<HudSelection>,
        ),
    >,
    mut mode_q: Query<
        &mut Text,
        (
            With<HudMode>,
            Without<HudScore>,
            Without<HudMoves>,
            Without<HudTime>,
            Without<HudChallenge>,
            Without<HudUndos>,
            Without<HudAutoComplete>,
            Without<HudRecycles>,
            Without<HudDrawCycle>,
            Without<HudSelection>,
        ),
    >,
    mut challenge_q: Query<
        (&mut Text, &mut TextColor),
        (
            With<HudChallenge>,
            Without<HudScore>,
            Without<HudMoves>,
            Without<HudTime>,
            Without<HudMode>,
            Without<HudUndos>,
            Without<HudAutoComplete>,
            Without<HudRecycles>,
            Without<HudDrawCycle>,
            Without<HudSelection>,
        ),
    >,
    mut undos_q: Query<
        (&mut Text, &mut TextColor),
        (
            With<HudUndos>,
            Without<HudScore>,
            Without<HudMoves>,
            Without<HudTime>,
            Without<HudMode>,
            Without<HudChallenge>,
            Without<HudAutoComplete>,
            Without<HudRecycles>,
            Without<HudDrawCycle>,
            Without<HudSelection>,
        ),
    >,
    mut auto_q: Query<
        &mut Text,
        (
            With<HudAutoComplete>,
            Without<HudScore>,
            Without<HudMoves>,
            Without<HudTime>,
            Without<HudMode>,
            Without<HudChallenge>,
            Without<HudUndos>,
            Without<HudRecycles>,
            Without<HudDrawCycle>,
            Without<HudSelection>,
        ),
    >,
    mut recycles_q: Query<
        &mut Text,
        (
            With<HudRecycles>,
            Without<HudScore>,
            Without<HudMoves>,
            Without<HudTime>,
            Without<HudMode>,
            Without<HudChallenge>,
            Without<HudUndos>,
            Without<HudAutoComplete>,
            Without<HudDrawCycle>,
            Without<HudSelection>,
        ),
    >,
    mut draw_cycle_q: Query<
        &mut Text,
        (
            With<HudDrawCycle>,
            Without<HudScore>,
            Without<HudMoves>,
            Without<HudTime>,
            Without<HudMode>,
            Without<HudChallenge>,
            Without<HudUndos>,
            Without<HudAutoComplete>,
            Without<HudRecycles>,
            Without<HudSelection>,
        ),
    >,
) {
    let ta_active = time_attack.as_ref().is_some_and(|ta| ta.active);

    // Score, moves, mode, challenge, and undos only need updating when game state changes.
    if game.is_changed() {
        let g = &game.0;
        let is_zen = g.mode == GameMode::Zen;
        if let Ok(mut t) = score_q.single_mut() {
            // Zen mode suppresses score display per spec ("No score display").
            **t = if is_zen {
                String::new()
            } else {
                format!("Score: {}", g.score)
            };
        }
        if let Ok(mut t) = moves_q.single_mut() {
            **t = format!("Moves: {}", g.move_count);
        }
        if let Ok(mut t) = mode_q.single_mut() {
            **t = match g.mode {
                GameMode::Classic => match g.draw_mode {
                    DrawMode::DrawOne => String::new(),
                    DrawMode::DrawThree => "Draw 3".to_string(),
                },
                GameMode::Zen => "ZEN".to_string(),
                GameMode::Challenge => "CHALLENGE".to_string(),
                GameMode::TimeAttack => "TIME ATTACK".to_string(),
            };
        }

        // --- Daily challenge constraint (with time-low colour warning) ---
        if let Ok((mut t, mut color)) = challenge_q.single_mut() {
            if g.is_won {
                **t = String::new();
            } else if let Some(dc) = daily.as_deref() {
                **t = challenge_hud_text(dc);
                if let Some(max_secs) = dc.max_time_secs {
                    let remaining = max_secs.saturating_sub(g.elapsed_seconds);
                    *color = TextColor(challenge_time_color(remaining));
                }
            } else {
                **t = String::new();
            }
        }

        // --- Undo count ---
        if let Ok((mut t, mut color)) = undos_q.single_mut() {
            let count = g.undo_count;
            if count == 0 {
                **t = String::new();
                *color = TextColor(TEXT_PRIMARY);
            } else {
                **t = format!("Undos: {count}");
                // STATE_WARNING signals "you took a penalty" — same hue
                // as the Recycles counter so they read as one category.
                *color = TextColor(STATE_WARNING);
            }
        }

        // --- Recycle counter (both modes, hidden until first recycle) ---
        if let Ok(mut t) = recycles_q.single_mut() {
            **t = if g.recycle_count > 0 {
                format!("Recycles: {}", g.recycle_count)
            } else {
                String::new()
            };
        }

        // --- Draw-cycle indicator (Draw-Three mode only) ---
        if let Ok(mut t) = draw_cycle_q.single_mut() {
            **t = if g.is_won || g.draw_mode != DrawMode::DrawThree {
                // Hide when not in Draw-Three or after the game is won.
                String::new()
            } else {
                let stock_len = g.piles[&solitaire_core::pile::PileType::Stock].cards.len();
                let next_draw = stock_len.min(3);
                format!("Cycle: {next_draw}/3")
            };
        }
    }

    // Time display: show Time Attack countdown every frame when active;
    // Zen mode suppresses the timer per spec ("No timer") — cleared unconditionally
    // every frame so it disappears immediately on the frame Z is pressed.
    // Otherwise show game elapsed time (updates once per second via game.is_changed()).
    let is_zen = game.0.mode == GameMode::Zen;
    let update_time = (ta_active || game.is_changed()) && !is_zen;
    if update_time {
        if let Ok(mut t) = time_q.single_mut() {
            if let Some(ta) = time_attack.as_ref().filter(|ta| ta.active) {
                let remaining = ta.remaining_secs.max(0.0) as u64;
                let m = remaining / 60;
                let s = remaining % 60;
                **t = format!("{m}:{s:02}");
            } else {
                let secs = game.0.elapsed_seconds;
                let m = secs / 60;
                let s = secs % 60;
                **t = format!("{m}:{s:02}");
            }
        }
    } else if is_zen {
        // Clear the time display immediately whenever Zen mode is active —
        // do not guard on game.is_changed() so it clears on the same frame
        // the player presses Z, before any move is made.
        if let Ok(mut t) = time_q.single_mut() {
            **t = String::new();
        }
    }

    // --- Auto-complete badge ---
    // Reflects the AutoCompleteState resource; update whenever it changes or game changes.
    let ac_active = auto_complete.as_ref().is_some_and(|ac| ac.active);
    let ac_changed = auto_complete.as_ref().is_some_and(|ac| ac.is_changed());
    if (ac_changed || game.is_changed())
        && let Ok(mut t) = auto_q.single_mut() {
            **t = if ac_active {
                "AUTO".to_string()
            } else {
                String::new()
            };
        }
}

/// Updates the `HudSelection` text node to show which pile is Tab-selected.
///
/// Displays `"▶ {pile_name}"` while `SelectionState::selected_pile` is `Some`,
/// or an empty string when no pile is selected. Runs every frame so the
/// indicator stays in sync with the selection resource.
fn update_selection_hud(
    selection: Option<Res<SelectionState>>,
    mut q: Query<&mut Text, With<HudSelection>>,
) {
    let Ok(mut t) = q.single_mut() else { return };
    let label = match selection.as_deref().and_then(|s| s.selected_pile.as_ref()) {
        None => String::new(),
        Some(PileType::Waste) => "▶ Waste".to_string(),
        Some(PileType::Stock) => "▶ Stock".to_string(),
        Some(PileType::Foundation(suit)) => {
            let s = match suit {
                Suit::Clubs => "Clubs",
                Suit::Diamonds => "Diamonds",
                Suit::Hearts => "Hearts",
                Suit::Spades => "Spades",
            };
            format!("▶ {s} Foundation")
        }
        Some(PileType::Tableau(idx)) => format!("▶ Column {}", idx + 1),
    };
    **t = label;
}

/// Fires `InfoToastEvent("Auto-completing...")` exactly once each time
/// `AutoCompleteState` transitions from inactive to active. Uses a `Local<bool>`
/// to debounce so the toast only appears on the leading edge.
fn announce_auto_complete(
    auto_complete: Option<Res<AutoCompleteState>>,
    mut toast: MessageWriter<InfoToastEvent>,
    mut was_active: Local<bool>,
) {
    let now_active = auto_complete.as_ref().is_some_and(|ac| ac.active);
    if now_active && !*was_active {
        toast.write(InfoToastEvent("Auto-completing...".to_string()));
    }
    *was_active = now_active;
}

/// Builds the HUD text for the active daily challenge constraints.
///
/// Returns `"Limit: mm:ss"` when a time limit is set, `"Goal: N pts"` when a
/// score target is set, or an empty string when the challenge has no extra
/// constraints.
fn challenge_hud_text(dc: &DailyChallengeResource) -> String {
    if let Some(secs) = dc.max_time_secs {
        format!("Limit: {}", format_time_limit(secs))
    } else if let Some(score) = dc.target_score {
        format!("Goal: {score} pts")
    } else {
        String::new()
    }
}

/// Returns the colour for the challenge time-limit HUD label based on
/// seconds remaining. Uses theme tokens so the urgency ramp picks up
/// palette changes for free.
///
/// | Remaining   | Token            |
/// |-------------|------------------|
/// | ≥ 60 s      | `STATE_INFO`     |
/// | 30 – 59 s   | `STATE_WARNING`  |
/// | < 30 s      | `STATE_DANGER`   |
pub fn challenge_time_color(remaining: u64) -> Color {
    if remaining < 30 {
        STATE_DANGER
    } else if remaining < 60 {
        STATE_WARNING
    } else {
        STATE_INFO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;
    use chrono::Local;
    use solitaire_core::game_state::{DrawMode, GameState};

    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(HudPlugin);
        app.update();
        app
    }

    #[test]
    fn hud_plugin_registers_without_panic() {
        let _app = headless_app();
    }

    #[test]
    fn update_hud_runs_after_game_mutation_without_panic() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new(42, DrawMode::DrawOne);
        app.update();
    }

    fn read_hud_text<M: Component>(app: &mut App) -> String {
        app.world_mut()
            .query_filtered::<&Text, With<M>>()
            .iter(app.world())
            .next()
            .map(|t| t.0.clone())
            .unwrap_or_default()
    }

    #[test]
    fn score_reflects_game_state() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0.score = 750;
        app.update();
        assert_eq!(read_hud_text::<HudScore>(&mut app), "Score: 750");
    }

    #[test]
    fn moves_reflects_game_state() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0.move_count = 42;
        app.update();
        assert_eq!(read_hud_text::<HudMoves>(&mut app), "Moves: 42");
    }

    #[test]
    fn draw_three_mode_shows_draw_3_badge() {
        use solitaire_core::game_state::GameMode;
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new_with_mode(42, DrawMode::DrawThree, GameMode::Classic);
        app.update();
        assert_eq!(read_hud_text::<HudMode>(&mut app), "Draw 3");
    }

    #[test]
    fn zen_mode_hides_score() {
        use solitaire_core::game_state::GameMode;
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new_with_mode(42, DrawMode::DrawOne, GameMode::Zen);
        app.world_mut().resource_mut::<GameStateResource>().0.score = 999;
        app.update();
        // Zen mode spec: "No score display" → text must be empty.
        assert_eq!(read_hud_text::<HudScore>(&mut app), "");
    }

    #[test]
    fn time_display_uses_mm_ss_format() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0.elapsed_seconds = 125;
        app.update();
        // 125 seconds = 2 minutes 5 seconds → "2:05"
        assert_eq!(read_hud_text::<HudTime>(&mut app), "2:05");
    }

    // -----------------------------------------------------------------------
    // format_time_limit (pure function)
    // -----------------------------------------------------------------------

    #[test]
    fn format_time_limit_300_is_5_00() {
        assert_eq!(format_time_limit(300), "5:00");
    }

    #[test]
    fn format_time_limit_zero() {
        assert_eq!(format_time_limit(0), "0:00");
    }

    #[test]
    fn format_time_limit_pads_seconds() {
        assert_eq!(format_time_limit(65), "1:05");
    }

    // -----------------------------------------------------------------------
    // challenge_hud_text (pure function)
    // -----------------------------------------------------------------------

    #[test]
    fn challenge_hud_text_shows_time_limit() {
        let dc = DailyChallengeResource {
            date: Local::now().date_naive(),
            seed: 1,
            goal_description: None,
            target_score: None,
            max_time_secs: Some(300),
        };
        assert_eq!(challenge_hud_text(&dc), "Limit: 5:00");
    }

    #[test]
    fn challenge_hud_text_shows_score_goal() {
        let dc = DailyChallengeResource {
            date: Local::now().date_naive(),
            seed: 1,
            goal_description: None,
            target_score: Some(4000),
            max_time_secs: None,
        };
        assert_eq!(challenge_hud_text(&dc), "Goal: 4000 pts");
    }

    #[test]
    fn challenge_hud_text_empty_when_no_constraints() {
        let dc = DailyChallengeResource {
            date: Local::now().date_naive(),
            seed: 1,
            goal_description: None,
            target_score: None,
            max_time_secs: None,
        };
        assert_eq!(challenge_hud_text(&dc), "");
    }

    #[test]
    fn challenge_time_color_above_60_is_info() {
        let c = challenge_time_color(61);
        assert_eq!(c, STATE_INFO);
    }

    #[test]
    fn challenge_time_color_exactly_60_is_info() {
        let c = challenge_time_color(60);
        assert_eq!(c, STATE_INFO);
    }

    #[test]
    fn challenge_time_color_59_is_warning() {
        let c = challenge_time_color(59);
        assert_eq!(c, STATE_WARNING);
    }

    #[test]
    fn challenge_time_color_30_is_warning() {
        let c = challenge_time_color(30);
        assert_eq!(c, STATE_WARNING);
    }

    #[test]
    fn challenge_time_color_29_is_danger() {
        let c = challenge_time_color(29);
        assert_eq!(c, STATE_DANGER);
    }

    #[test]
    fn challenge_time_color_zero_is_danger() {
        let c = challenge_time_color(0);
        assert_eq!(c, STATE_DANGER);
    }

    // -----------------------------------------------------------------------
    // HudChallenge in-app tests
    // -----------------------------------------------------------------------

    #[test]
    fn challenge_hud_empty_when_no_daily_resource() {
        // No DailyChallengeResource inserted → HudChallenge must be empty.
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0.score = 1; // force change
        app.update();
        assert_eq!(read_hud_text::<HudChallenge>(&mut app), "");
    }

    #[test]
    fn challenge_hud_shows_time_limit_when_resource_present() {
        let mut app = headless_app();
        app.world_mut().insert_resource(DailyChallengeResource {
            date: Local::now().date_naive(),
            seed: 42,
            goal_description: Some("Win fast".to_string()),
            target_score: None,
            max_time_secs: Some(300),
        });
        app.world_mut().resource_mut::<GameStateResource>().0.score = 1; // force change
        app.update();
        assert_eq!(read_hud_text::<HudChallenge>(&mut app), "Limit: 5:00");
    }

    #[test]
    fn challenge_hud_shows_score_goal_when_resource_present() {
        let mut app = headless_app();
        app.world_mut().insert_resource(DailyChallengeResource {
            date: Local::now().date_naive(),
            seed: 42,
            goal_description: None,
            target_score: Some(4000),
            max_time_secs: None,
        });
        app.world_mut().resource_mut::<GameStateResource>().0.score = 1;
        app.update();
        assert_eq!(read_hud_text::<HudChallenge>(&mut app), "Goal: 4000 pts");
    }

    #[test]
    fn challenge_hud_clears_on_win() {
        let mut app = headless_app();
        app.world_mut().insert_resource(DailyChallengeResource {
            date: Local::now().date_naive(),
            seed: 42,
            goal_description: None,
            target_score: None,
            max_time_secs: Some(300),
        });
        // Mark the game as won — HudChallenge should be empty.
        app.world_mut().resource_mut::<GameStateResource>().0.is_won = true;
        app.update();
        assert_eq!(read_hud_text::<HudChallenge>(&mut app), "");
    }

    // -----------------------------------------------------------------------
    // HudUndos in-app tests
    // -----------------------------------------------------------------------

    #[test]
    fn undos_hud_empty_at_game_start() {
        let mut app = headless_app();
        app.update();
        assert_eq!(read_hud_text::<HudUndos>(&mut app), "");
    }

    #[test]
    fn undos_hud_shows_count_after_undo() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0.undo_count = 3;
        app.update();
        assert_eq!(read_hud_text::<HudUndos>(&mut app), "Undos: 3");
    }

    // -----------------------------------------------------------------------
    // HudAutoComplete in-app tests (Task #56)
    // -----------------------------------------------------------------------

    fn headless_app_with_auto_complete() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(HudPlugin);
        app.init_resource::<AutoCompleteState>();
        app.update();
        app
    }

    #[test]
    fn auto_complete_badge_shows_auto_when_active() {
        let mut app = headless_app_with_auto_complete();
        app.world_mut().resource_mut::<AutoCompleteState>().active = true;
        // Also trigger game state change so the update fires.
        app.world_mut().resource_mut::<GameStateResource>().0.move_count += 1;
        app.update();
        assert_eq!(read_hud_text::<HudAutoComplete>(&mut app), "AUTO");
    }

    #[test]
    fn auto_complete_badge_empty_when_inactive() {
        let mut app = headless_app_with_auto_complete();
        // active is false by default.
        app.world_mut().resource_mut::<GameStateResource>().0.move_count += 1;
        app.update();
        assert_eq!(read_hud_text::<HudAutoComplete>(&mut app), "");
    }

    // -----------------------------------------------------------------------
    // HudRecycles in-app tests
    // -----------------------------------------------------------------------

    #[test]
    fn recycles_hud_hidden_when_zero_in_draw_one_mode() {
        let mut app = headless_app();
        // Draw-One, no recycles yet — text must be empty.
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new(42, DrawMode::DrawOne);
        app.update();
        assert_eq!(read_hud_text::<HudRecycles>(&mut app), "");
    }

    #[test]
    fn recycles_hud_hidden_when_zero_in_draw_three_mode() {
        let mut app = headless_app();
        // Draw-Three, no recycles yet — text must also be empty.
        app.world_mut().resource_mut::<GameStateResource>().0 =
            GameState::new(42, DrawMode::DrawThree);
        app.update();
        assert_eq!(read_hud_text::<HudRecycles>(&mut app), "");
    }

    #[test]
    fn recycles_hud_shows_count_draw_three() {
        let mut app = headless_app();
        let mut gs = GameState::new(42, DrawMode::DrawThree);
        gs.recycle_count = 3;
        app.world_mut().resource_mut::<GameStateResource>().0 = gs;
        app.update();
        assert_eq!(read_hud_text::<HudRecycles>(&mut app), "Recycles: 3");
    }

    #[test]
    fn recycles_hud_shows_count_draw_one() {
        let mut app = headless_app();
        // Draw-One with recycle_count > 0 must now show the counter too.
        let mut gs = GameState::new(42, DrawMode::DrawOne);
        gs.recycle_count = 2;
        app.world_mut().resource_mut::<GameStateResource>().0 = gs;
        app.update();
        assert_eq!(read_hud_text::<HudRecycles>(&mut app), "Recycles: 2");
    }

    // -----------------------------------------------------------------------
    // Score-change feedback (G2)
    // -----------------------------------------------------------------------

    /// Tells `TimePlugin` to advance by `secs` on every subsequent
    /// `app.update()`. Mirrors the helper in `ui_modal::tests`; kept
    /// local to avoid coupling the two test modules.
    fn set_manual_time_step(app: &mut App, secs: f32) {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(secs),
        ));
    }

    /// Counts entities matching component `M` currently in the world.
    fn count_with<M: Component>(app: &mut App) -> usize {
        app.world_mut().query::<&M>().iter(app.world()).count()
    }

    /// A score jump ≥ `SCORE_FLOATER_THRESHOLD` spawns a floating
    /// `ScoreFloater` entity coloured `ACCENT_PRIMARY`. The pulse
    /// component is also inserted on the score readout — both signals
    /// fire from the same delta detection.
    #[test]
    fn score_increase_above_threshold_spawns_floater_in_accent_primary() {
        let mut app = headless_app();
        // Pin `Time::delta_secs()` to 0 so the floater's RGB and alpha
        // can be asserted exactly: with Automatic strategy a few ms
        // of wall-clock time leaks in between updates and the alpha
        // drifts below 1.0 by `dt / lifetime`.
        set_manual_time_step(&mut app, 0.0);
        // Initial state has score=0; bumping by 50 (the threshold)
        // is the smallest jump that triggers the floater.
        app.world_mut().resource_mut::<GameStateResource>().0.score = 50;
        app.update();

        // One floater should now exist.
        let count = count_with::<ScoreFloater>(&mut app);
        assert_eq!(count, 1, "expected a single ScoreFloater for a +50 jump");

        // Its TextColor must be ACCENT_PRIMARY at full alpha. The
        // detect system spawns the floater coloured ACCENT_PRIMARY
        // and at dt=0 the first advance tick leaves alpha = 1.0.
        let world = app.world_mut();
        let mut q = world.query::<(&ScoreFloater, &TextColor)>();
        let (_floater, color) = q.iter(world).next().expect("floater missing TextColor");
        assert_eq!(color.0, ACCENT_PRIMARY);
    }

    /// After enough time for `MOTION_SCORE_PULSE_SECS * 2` to elapse
    /// the floater has reached the end of its lifetime and despawned.
    #[test]
    fn score_floater_despawns_after_full_lifetime() {
        let mut app = headless_app();
        app.world_mut().resource_mut::<GameStateResource>().0.score = 100;
        app.update();
        assert_eq!(count_with::<ScoreFloater>(&mut app), 1);

        // Advance by a delta well past the floater's lifetime — the
        // single oversized tick clamps t at 1.0 and the entity is
        // despawned in the same `Update`.
        set_manual_time_step(&mut app, MOTION_SCORE_PULSE_SECS * 2.0 * 2.0 + 0.1);
        app.update();
        app.update(); // first update propagates the new strategy; second runs the system with non-zero dt.

        assert_eq!(
            count_with::<ScoreFloater>(&mut app),
            0,
            "floater should have despawned after its full lifetime"
        );
    }

    /// A small score change (below the threshold) inserts a pulse on
    /// the readout but never spawns a floater — keeping the floating
    /// "+N" reserved for meaningful score jumps.
    #[test]
    fn score_increase_below_threshold_does_not_spawn_floater() {
        let mut app = headless_app();
        // +5 mirrors a single tableau-to-foundation move; well below
        // the 50-point threshold so the floater path stays dormant.
        app.world_mut().resource_mut::<GameStateResource>().0.score = 5;
        app.update();
        assert_eq!(
            count_with::<ScoreFloater>(&mut app),
            0,
            "delta of +5 must not spawn a floater"
        );
    }

    /// The triangular pulse curve hits its peak (1.1) at t=0.5 and
    /// returns to 1.0 at the endpoints. Pure-function check that
    /// guards the curve shape against future tweaks.
    #[test]
    fn score_pulse_scale_is_triangular() {
        assert!((score_pulse_scale(0.0) - 1.0).abs() < 1e-6);
        assert!((score_pulse_scale(0.5) - 1.1).abs() < 1e-6);
        assert!((score_pulse_scale(1.0) - 1.0).abs() < 1e-6);
        // Values outside [0,1] are clamped before the curve runs.
        assert!((score_pulse_scale(-0.2) - 1.0).abs() < 1e-6);
        assert!((score_pulse_scale(2.0) - 1.0).abs() < 1e-6);
    }

    // -----------------------------------------------------------------------
    // Phase 2: keyboard focus ring — HUD action bar
    // -----------------------------------------------------------------------

    /// Returns the `Focusable` carried by the unique entity matching
    /// marker `M`. Helper for the HUD focus tests.
    fn focusable_for<M: Component>(app: &mut App) -> Focusable {
        app.world_mut()
            .query_filtered::<&Focusable, With<M>>()
            .iter(app.world())
            .next()
            .copied()
            .unwrap_or_else(|| panic!("no Focusable on the {} button", std::any::type_name::<M>()))
    }

    #[test]
    fn hud_buttons_get_focusable_marker() {
        let mut app = headless_app();
        // Every action-bar button is in `FocusGroup::Hud`.
        for f in [
            focusable_for::<MenuButton>(&mut app),
            focusable_for::<UndoButton>(&mut app),
            focusable_for::<PauseButton>(&mut app),
            focusable_for::<HelpButton>(&mut app),
            focusable_for::<ModesButton>(&mut app),
            focusable_for::<NewGameButton>(&mut app),
        ] {
            assert_eq!(
                f.group,
                FocusGroup::Hud,
                "every HUD action button must be in FocusGroup::Hud"
            );
        }
    }

    /// Returns the tooltip string carried by the unique entity matching
    /// marker `M`. Panics if zero or more than one such entity exists,
    /// which is the invariant we want to enforce for HUD readouts and
    /// action buttons (each marker is spawned exactly once).
    fn tooltip_for<M: Component>(app: &mut App) -> String {
        let mut q = app
            .world_mut()
            .query_filtered::<&Tooltip, With<M>>();
        let world = app.world();
        let mut iter = q.iter(world);
        let first = iter
            .next()
            .unwrap_or_else(|| {
                panic!(
                    "expected a Tooltip on the {} entity",
                    std::any::type_name::<M>()
                )
            })
            .0
            .clone()
            .into_owned();
        assert!(
            iter.next().is_none(),
            "expected exactly one Tooltip-bearing entity for {}",
            std::any::type_name::<M>()
        );
        first
    }

    /// Every HUD readout and action button must spawn with a `Tooltip`
    /// carrying the approved canonical microcopy. Mirrors the structure
    /// of `hud_buttons_get_focusable_marker` (Phase 2 focus test) so the
    /// invariant — one marker entity, one tooltip, exact text — is
    /// asserted consistently across every element.
    #[test]
    fn hud_elements_carry_expected_tooltip_strings() {
        let mut app = headless_app();

        // HUD readouts (left column, top to bottom).
        assert_eq!(
            tooltip_for::<HudScore>(&mut app),
            "Points earned this game. Hidden in Zen mode."
        );
        assert_eq!(
            tooltip_for::<HudMoves>(&mut app),
            "Moves you've made this game. Counts placements and stock draws."
        );
        assert_eq!(
            tooltip_for::<HudTime>(&mut app),
            "Time on this game. Counts down in Time Attack."
        );
        assert_eq!(
            tooltip_for::<HudMode>(&mut app),
            "Active game mode. Click Modes to switch."
        );
        assert_eq!(
            tooltip_for::<HudChallenge>(&mut app),
            "Today's daily challenge target. Beat it for bonus XP."
        );
        assert_eq!(
            tooltip_for::<HudDrawCycle>(&mut app),
            "Cards drawn on the next stock click in Draw-Three."
        );
        assert_eq!(
            tooltip_for::<HudUndos>(&mut app),
            "Undos used this game. Any undo blocks the No Undo achievement."
        );
        assert_eq!(
            tooltip_for::<HudRecycles>(&mut app),
            "Times you've recycled the stock. Three or more unlocks Comeback."
        );
        assert_eq!(
            tooltip_for::<HudAutoComplete>(&mut app),
            "Board is solvable from here. Press Enter to auto-finish."
        );
        assert_eq!(
            tooltip_for::<HudSelection>(&mut app),
            "Pile selected with Tab. Use arrows or Enter to act."
        );

        // Action bar (left to right).
        assert_eq!(
            tooltip_for::<MenuButton>(&mut app),
            "Open Stats, Achievements, Profile, Settings, or Leaderboard."
        );
        assert_eq!(
            tooltip_for::<UndoButton>(&mut app),
            "Take back your last move. Costs points and blocks No Undo."
        );
        assert_eq!(
            tooltip_for::<PauseButton>(&mut app),
            "Pause the game and freeze the timer."
        );
        assert_eq!(
            tooltip_for::<HelpButton>(&mut app),
            "Show controls, rules, and keyboard shortcuts."
        );
        assert_eq!(
            tooltip_for::<ModesButton>(&mut app),
            "Switch modes: Classic, Daily, Zen, Challenge, Time Attack."
        );
        assert_eq!(
            tooltip_for::<NewGameButton>(&mut app),
            "Start a fresh deal. Confirms first if a game is in progress."
        );
    }

    /// Every interior row of the Modes and Menu popovers must carry a
    /// `Tooltip`. The popovers open from action-bar buttons whose own
    /// tooltips are already covered above; this test extends the
    /// invariant inward so hover discoverability is uniform across the
    /// HUD's nested controls.
    ///
    /// We invoke the popover spawn helpers directly with a maxed-out
    /// `ProgressResource` and a `DailyChallengeResource` so every row
    /// branch fires (Classic, Daily, Zen, Challenge, Time Attack).
    /// Headless click simulation isn't needed — the contract under
    /// test is "every popover row spawns with a tooltip", which is a
    /// property of the spawn helpers themselves.
    #[test]
    fn popover_rows_carry_tooltip_strings() {
        use crate::progress_plugin::ProgressResource;
        use solitaire_sync::progress::PlayerProgress;

        let mut app = headless_app();

        // Force every mode row to render: level past the challenge
        // unlock threshold, plus a daily challenge resource so the
        // Daily row appears.
        let progress = ProgressResource(PlayerProgress {
            level: CHALLENGE_UNLOCK_LEVEL,
            ..Default::default()
        });
        let daily = DailyChallengeResource {
            date: Local::now().date_naive(),
            seed: 1,
            goal_description: None,
            target_score: None,
            max_time_secs: None,
        };

        // Spawn both popovers via their helpers. Mirrors how the click
        // handlers invoke them in production — we just skip the click.
        {
            let world = app.world_mut();
            let mut commands = world.commands();
            spawn_modes_popover(&mut commands, Some(&progress), Some(&daily), None);
            spawn_menu_popover(&mut commands, None);
            world.flush();
        }
        app.update();

        // Every ModeOption-tagged entity must also carry a Tooltip,
        // and the count must match the five canonical modes.
        let mut mode_q = app
            .world_mut()
            .query_filtered::<&Tooltip, With<ModeOption>>();
        let mode_tooltips: Vec<String> = mode_q
            .iter(app.world())
            .map(|t| t.0.clone().into_owned())
            .collect();
        assert_eq!(
            mode_tooltips.len(),
            5,
            "expected a tooltip on each of the 5 mode rows, got {}",
            mode_tooltips.len()
        );
        // Every approved mode tooltip string must be present somewhere
        // among the ModeOption rows. Order isn't asserted — the spawn
        // order test elsewhere already covers that.
        for expected in [
            "Standard Klondike. Score, timer, and full progression.",
            "Today's seeded deal. Same for every player worldwide.",
            "No timer, no score, no penalties. Just play.",
            "Hand-picked hard seeds. No undo allowed.",
            "Win as many games as you can in ten minutes.",
        ] {
            assert!(
                mode_tooltips.iter().any(|s| s == expected),
                "missing mode tooltip: {expected:?}"
            );
        }

        // Same contract for MenuOption rows: five entries, each with a
        // tooltip, exact strings matching the approved microcopy.
        let mut menu_q = app
            .world_mut()
            .query_filtered::<&Tooltip, With<MenuOption>>();
        let menu_tooltips: Vec<String> = menu_q
            .iter(app.world())
            .map(|t| t.0.clone().into_owned())
            .collect();
        assert_eq!(
            menu_tooltips.len(),
            5,
            "expected a tooltip on each of the 5 menu rows, got {}",
            menu_tooltips.len()
        );
        for expected in [
            "Lifetime totals: wins, streaks, fastest time, best score.",
            "Browse unlocked achievements and the rewards still ahead.",
            "Your level, XP progress, and sync status.",
            "Audio, animations, theme, draw mode, and sync.",
            "Top players from your sync server. Opt in from Profile.",
        ] {
            assert!(
                menu_tooltips.iter().any(|s| s == expected),
                "missing menu tooltip: {expected:?}"
            );
        }
    }

    #[test]
    fn hud_button_order_matches_spawn_order() {
        let mut app = headless_app();
        // Visual reading order (left → right): Menu, Undo, Pause, Help,
        // Modes, New Game. Their `order` fields must be 0..=5 in that
        // order so Tab cycles them as the player reads them.
        assert_eq!(focusable_for::<MenuButton>(&mut app).order, 0);
        assert_eq!(focusable_for::<UndoButton>(&mut app).order, 1);
        assert_eq!(focusable_for::<PauseButton>(&mut app).order, 2);
        assert_eq!(focusable_for::<HelpButton>(&mut app).order, 3);
        assert_eq!(focusable_for::<ModesButton>(&mut app).order, 4);
        assert_eq!(focusable_for::<NewGameButton>(&mut app).order, 5);
    }

    #[test]
    fn hud_focus_only_engages_when_button_hovered() {
        // Phase 2 declares membership in `FocusGroup::Hud`; the
        // engagement rule lives in `handle_focus_keys`. Two halves to
        // this test:
        //   (a) no modal + no hover ⇒ Tab is a no-op (Phase 1 contract
        //       still holds when nothing is hovered).
        //   (b) no modal + a HUD button hovered ⇒ Tab advances
        //       `FocusedButton` to a Hud-grouped entity.
        use crate::ui_focus::{FocusedButton, UiFocusPlugin};
        use crate::ui_modal::UiModalPlugin;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(UiModalPlugin)
            .add_plugins(UiFocusPlugin)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(HudPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.update();

        // (a) Sanity: HUD buttons exist and are focusable, but no
        // modal open and no hover ⇒ FocusedButton stays None.
        assert!(
            app.world().resource::<FocusedButton>().0.is_none(),
            "no modal open, no auto-focus"
        );

        // Press Tab. With no modal and no hover, `handle_focus_keys`
        // resolves no active group and returns early — Tab must not
        // advance the HUD focus ring on its own.
        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release_all();
            input.clear();
            input.press(KeyCode::Tab);
        }
        app.update();

        assert!(
            app.world().resource::<FocusedButton>().0.is_none(),
            "Tab with no modal and no Hud hover must not engage the HUD focus ring"
        );

        // (b) Hover the Menu button — the leftmost HUD action — and
        // Tab. The Hud-group cycle should pick a Hud-tagged entity.
        let menu_entity = app
            .world_mut()
            .query_filtered::<Entity, With<MenuButton>>()
            .iter(app.world())
            .next()
            .expect("MenuButton entity should exist");
        app.world_mut()
            .entity_mut(menu_entity)
            .insert(Interaction::Hovered);

        {
            let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            input.release_all();
            input.clear();
            input.press(KeyCode::Tab);
        }
        app.update();

        let focused = app
            .world()
            .resource::<FocusedButton>()
            .0
            .expect("Tab with a HUD button hovered must engage the HUD focus ring");
        // The focused entity must itself be Hud-grouped (i.e. one of
        // the action-bar buttons), not anything else in the world.
        let focusable = app
            .world()
            .entity(focused)
            .get::<Focusable>()
            .expect("focused entity must carry Focusable");
        assert_eq!(
            focusable.group,
            FocusGroup::Hud,
            "Hud-engaged Tab must focus a Hud-grouped entity"
        );
    }
}
