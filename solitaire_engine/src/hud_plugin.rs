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
use crate::ui_theme::{
    ACCENT_PRIMARY, ACCENT_SECONDARY, BG_ELEVATED, BG_ELEVATED_HI, BG_ELEVATED_PRESSED,
    BORDER_SUBTLE, RADIUS_MD, RADIUS_SM, STATE_DANGER, STATE_INFO, STATE_SUCCESS, STATE_WARNING,
    TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY, TYPE_BODY_LG, TYPE_CAPTION, TYPE_HEADLINE,
    VAL_SPACE_1, VAL_SPACE_2, VAL_SPACE_3,
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
            .add_systems(Startup, (spawn_hud, spawn_action_buttons))
            .add_systems(Update, update_hud.after(GameMutation))
            .add_systems(Update, announce_auto_complete.after(GameMutation))
            .add_systems(Update, update_selection_hud)
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
                    Text::new("Score: 0"),
                    font_score.clone(),
                    TextColor(TEXT_PRIMARY),
                ));
                t1.spawn((
                    HudMoves,
                    Text::new("Moves: 0"),
                    font_lg.clone(),
                    TextColor(TEXT_SECONDARY),
                ));
                t1.spawn((
                    HudTime,
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
                    Text::new(""),
                    font_body.clone(),
                    TextColor(ACCENT_PRIMARY),
                ));
                t2.spawn((
                    HudChallenge,
                    Text::new(""),
                    font_body.clone(),
                    TextColor(STATE_INFO),
                ));
                t2.spawn((
                    HudDrawCycle,
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
                    Text::new(""),
                    font_body.clone(),
                    TextColor(STATE_WARNING),
                ));
                t3.spawn((
                    HudRecycles,
                    Text::new(""),
                    font_body.clone(),
                    TextColor(STATE_WARNING),
                ));
                t3.spawn((
                    HudAutoComplete,
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
            spawn_action_button(row, MenuButton, "Menu \u{25BE}", None, &font);
            spawn_action_button(row, UndoButton, "Undo", Some("U"), &font);
            spawn_action_button(row, PauseButton, "Pause", Some("Esc"), &font);
            spawn_action_button(row, HelpButton, "Help", Some("F1"), &font);
            spawn_action_button(row, ModesButton, "Modes \u{25BE}", None, &font);
            spawn_action_button(row, NewGameButton, "New Game", Some("N"), &font);
        });
}

/// Spawns a single action button as a child of `row`. Each button shares
/// the same node geometry, idle colour, and `ActionButton` marker so
/// `paint_action_buttons` can recolour all of them with one query.
fn spawn_action_button<M: Component>(
    row: &mut ChildSpawnerCommands,
    marker: M,
    label: &str,
    hotkey: Option<&'static str>,
    font: &TextFont,
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

    let mut rows: Vec<(ModeOption, &'static str)> = vec![(ModeOption::Classic, "Classic")];
    if daily.is_some() {
        rows.push((ModeOption::DailyChallenge, "Daily Challenge"));
    }
    if level >= CHALLENGE_UNLOCK_LEVEL {
        rows.push((ModeOption::Zen, "Zen"));
        rows.push((ModeOption::Challenge, "Challenge"));
        rows.push((ModeOption::TimeAttack, "Time Attack"));
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
            for (option, label) in rows {
                panel
                    .spawn((
                        option,
                        ActionButton,
                        Button,
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

    let rows: [(MenuOption, &'static str); 5] = [
        (MenuOption::Stats, "Stats"),
        (MenuOption::Achievements, "Achievements"),
        (MenuOption::Profile, "Profile"),
        (MenuOption::Settings, "Settings"),
        (MenuOption::Leaderboard, "Leaderboard"),
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
            for (option, label) in rows {
                panel
                    .spawn((
                        option,
                        ActionButton,
                        Button,
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
}
