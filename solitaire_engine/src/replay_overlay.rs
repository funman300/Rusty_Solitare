//! On-screen overlay shown while a recorded [`Replay`] plays back.
//!
//! The overlay is a thin top-of-window banner with three pieces of UI:
//!
//! - A "Replay" label on the left so the player knows the surface is
//!   under playback control rather than live input.
//! - A "Move N of M" progress indicator in the centre, recomputed every
//!   frame the cursor advances.
//! - A "Stop" button on the right that aborts playback and returns
//!   control to the player.
//!
//! When playback finishes ([`ReplayPlaybackState::Completed`]) the banner
//! label swaps to "Replay complete" and stays visible until the playback
//! core auto-clears the resource back to [`ReplayPlaybackState::Inactive`]
//! a few seconds later, at which point the overlay despawns.
//!
//! The overlay sits at z-layer [`Z_REPLAY_OVERLAY`] — above gameplay but
//! below every modal layer ([`Z_MODAL_SCRIM`] and up). That ordering lets
//! the player still open Settings, Pause, and Help during a replay; those
//! modals will render on top of the banner as expected.
//!
//! [`Replay`]: solitaire_data::Replay
//! [`Z_MODAL_SCRIM`]: crate::ui_theme::Z_MODAL_SCRIM

use bevy::prelude::*;

use crate::font_plugin::FontResource;
use crate::replay_playback::{stop_replay_playback, ReplayPlaybackState};
use crate::ui_modal::{spawn_modal_button, ButtonVariant};
use crate::ui_theme::{
    ACCENT_PRIMARY, BG_ELEVATED_HI, TEXT_PRIMARY, TYPE_BODY, TYPE_HEADLINE, VAL_SPACE_2,
    VAL_SPACE_4, Z_DROP_OVERLAY,
};

// ---------------------------------------------------------------------------
// Z-index — see `ui_theme::Z_MODAL_SCRIM` (200) for the next layer above.
// ---------------------------------------------------------------------------

/// `bevy::ui` `ZIndex` value for the replay overlay banner.
///
/// Numeric value is `Z_DROP_OVERLAY as i32 + 5 = 55`; chosen so the banner
/// sits clearly above the HUD top layer (`Z_HUD_TOP = 60` is intentionally
/// **below** modals, but the overlay needs to be above HUD readouts) yet
/// well below `Z_MODAL_SCRIM = 200` so Settings, Pause, and Help modals
/// continue to render on top of the overlay during a replay.
///
/// The `Z_DROP_OVERLAY + 5` formula in the spec is reproduced here as an
/// integer because `Z_DROP_OVERLAY` itself is a `f32` Sprite-space z used
/// for the drop-target overlay sprites — UI nodes use `i32` `ZIndex`, so
/// we materialise a separate constant rather than reuse the `f32` value.
pub const Z_REPLAY_OVERLAY: i32 = Z_DROP_OVERLAY as i32 + 5;

/// Total height of the banner in pixels. Thin enough to leave the
/// gameplay surface visible underneath, tall enough to comfortably fit
/// the headline-sized "Replay" label.
const BANNER_HEIGHT: f32 = 48.0;

/// Background colour alpha for the banner. `BG_ELEVATED_HI` at this alpha
/// reads as a clear "this is a UI strip" callout while still letting the
/// felt show through enough to anchor the banner to the play surface.
const BANNER_ALPHA: f32 = 0.92;

// ---------------------------------------------------------------------------
// Marker components
// ---------------------------------------------------------------------------

/// Marker on the banner's root `Node`. Used by the spawn / despawn /
/// progress-update systems to find the overlay.
#[derive(Component, Debug)]
pub struct ReplayOverlayRoot;

/// Marker on the left-hand banner label `Text`. Carries either "Replay"
/// (during playback) or "Replay complete" (once finished); the
/// completion-text-update system swaps the contents in place.
#[derive(Component, Debug)]
pub struct ReplayOverlayBannerText;

/// Marker on the centre progress `Text`. Updated every frame to reflect
/// the current `(cursor, total)` returned by
/// [`ReplayPlaybackState::progress`].
#[derive(Component, Debug)]
pub struct ReplayOverlayProgressText;

/// Marker on the right-hand "Stop" button. Click handler queries for this
/// and calls [`stop_replay_playback`] when an `Interaction::Pressed`
/// transition is seen.
#[derive(Component, Debug)]
pub struct ReplayStopButton;

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Bevy plugin that registers every system needed to drive the replay
/// overlay's lifecycle.
///
/// The plugin is independent of [`crate::replay_playback::ReplayPlaybackPlugin`]
/// — it only reads the shared `ReplayPlaybackState` resource. Tests insert
/// the resource manually and exercise the overlay in isolation.
pub struct ReplayOverlayPlugin;

impl Plugin for ReplayOverlayPlugin {
    fn build(&self, app: &mut App) {
        // The systems are ordered so that, on a single frame:
        //   1. The state-watcher spawns or despawns the overlay if the
        //      `ReplayPlaybackState` resource changed.
        //   2. The completion-text update swaps the banner label when the
        //      state is `Completed`.
        //   3. The progress-text update writes the latest "Move N of M".
        //   4. The Stop-button click handler reads `Interaction::Pressed`
        //      and calls `stop_replay_playback` (which mutates the state).
        // Putting Stop last means a click in frame N is observed by
        // `react_to_state_change` in frame N+1, which then despawns the
        // overlay in response — a clean state-driven loop.
        app.add_systems(
            Update,
            (
                react_to_state_change,
                update_banner_label,
                update_progress_text,
                handle_stop_button,
            )
                .chain(),
        );
    }
}

// ---------------------------------------------------------------------------
// Spawning
// ---------------------------------------------------------------------------

/// Reads [`ReplayPlaybackState`] every time the resource changes and either
/// spawns or despawns the overlay accordingly. Treats the resource as the
/// single source of truth — the spawn / despawn decision is derived from
/// `is_playing() || is_completed()` rather than tracking previous-state
/// transitions explicitly, which keeps the system stateless.
fn react_to_state_change(
    mut commands: Commands,
    state: Res<ReplayPlaybackState>,
    existing: Query<Entity, With<ReplayOverlayRoot>>,
    font_res: Option<Res<FontResource>>,
) {
    if !state.is_changed() {
        return;
    }

    let should_be_visible = state.is_playing() || state.is_completed();
    let already_spawned = existing.iter().next().is_some();

    if should_be_visible && !already_spawned {
        spawn_overlay(&mut commands, font_res.as_deref(), &state);
    } else if !should_be_visible && already_spawned {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
    }
    // The `should_be_visible && already_spawned` branch is a no-op here —
    // the per-frame text update systems below repaint the banner label
    // and progress readout in place without a respawn.
}

/// Spawns the banner — a flex-row Node anchored to the top edge of the
/// window with three children: the "Replay" / "Replay complete" label,
/// the centred progress text, and the right-aligned Stop button.
fn spawn_overlay(
    commands: &mut Commands,
    font_res: Option<&FontResource>,
    state: &ReplayPlaybackState,
) {
    let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();

    let banner_label = if state.is_completed() {
        "Replay complete"
    } else {
        "Replay"
    };
    let progress_label = format_progress(state);

    let banner_bg = Color::srgba(
        BG_ELEVATED_HI.to_srgba().red,
        BG_ELEVATED_HI.to_srgba().green,
        BG_ELEVATED_HI.to_srgba().blue,
        BANNER_ALPHA,
    );

    commands
        .spawn((
            ReplayOverlayRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Px(BANNER_HEIGHT),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect::axes(VAL_SPACE_4, VAL_SPACE_2),
                column_gap: VAL_SPACE_4,
                ..default()
            },
            BackgroundColor(banner_bg),
            // Pin the banner to its z layer in both the local and the
            // global stacking context — `GlobalZIndex` matters because
            // the overlay is a top-level Node (no parent), and Bevy 0.18
            // has historically had subtle stacking-context drift here.
            ZIndex(Z_REPLAY_OVERLAY),
            GlobalZIndex(Z_REPLAY_OVERLAY),
        ))
        .with_children(|banner| {
            // Left: "Replay" label in the loud yellow accent so it reads
            // unmistakably as a non-gameplay surface.
            banner.spawn((
                ReplayOverlayBannerText,
                Text::new(banner_label),
                TextFont {
                    font: font_handle.clone(),
                    font_size: TYPE_HEADLINE,
                    ..default()
                },
                TextColor(ACCENT_PRIMARY),
            ));

            // Centre: progress readout — neutral primary text colour so
            // the eye treats it as data, not a callout.
            banner.spawn((
                ReplayOverlayProgressText,
                Text::new(progress_label),
                TextFont {
                    font: font_handle,
                    font_size: TYPE_BODY,
                    ..default()
                },
                TextColor(TEXT_PRIMARY),
            ));

            // Right: Stop button. Tertiary variant — the action is
            // available but not the loudest element in the banner; the
            // "Replay" yellow accent owns that slot. `spawn_modal_button`
            // gives us hover / press paint and focus rings for free via
            // the existing `UiModalPlugin` paint system.
            banner
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: VAL_SPACE_2,
                    ..default()
                })
                .with_children(|wrap| {
                    spawn_modal_button(
                        wrap,
                        ReplayStopButton,
                        "Stop",
                        None,
                        ButtonVariant::Tertiary,
                        font_res,
                    );
                });
        });
}

// ---------------------------------------------------------------------------
// Per-frame text updates
// ---------------------------------------------------------------------------

/// Overwrites the banner label whenever the resource changes — covers the
/// `Playing → Completed` transition by swapping "Replay" for
/// "Replay complete" in place without despawning the overlay.
fn update_banner_label(
    state: Res<ReplayPlaybackState>,
    mut q: Query<&mut Text, With<ReplayOverlayBannerText>>,
) {
    if !state.is_changed() {
        return;
    }
    let label = if state.is_completed() {
        "Replay complete"
    } else if state.is_playing() {
        "Replay"
    } else {
        return;
    };
    for mut text in &mut q {
        **text = label.to_string();
    }
}

/// Repaints the "Move N of M" centre readout every frame the cursor moves.
/// Cheap — early-exits if the resource has not changed since the last
/// frame so idle replays don't churn the text mesh.
fn update_progress_text(
    state: Res<ReplayPlaybackState>,
    mut q: Query<&mut Text, With<ReplayOverlayProgressText>>,
) {
    if !state.is_changed() {
        return;
    }
    let label = format_progress(&state);
    for mut text in &mut q {
        **text = label.clone();
    }
}

/// Pure helper — formats the centre progress readout for the given state.
/// Exposed at module scope so the spawn path and the per-frame update
/// path produce the exact same string.
fn format_progress(state: &ReplayPlaybackState) -> String {
    match state.progress() {
        Some((cursor, total)) => format!("Move {cursor} of {total}"),
        None if state.is_completed() => "Replay complete".to_string(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Stop button handler
// ---------------------------------------------------------------------------

/// Watches the Stop button for `Interaction::Pressed` transitions. On a
/// click, calls [`stop_replay_playback`] which resets the state to
/// `Inactive`; the next frame's `react_to_state_change` then despawns
/// the overlay.
fn handle_stop_button(
    mut commands: Commands,
    mut state: ResMut<ReplayPlaybackState>,
    buttons: Query<&Interaction, (With<ReplayStopButton>, Changed<Interaction>)>,
) {
    if !buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    stop_replay_playback(&mut commands, &mut state);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use solitaire_core::game_state::{DrawMode, GameMode};
    use solitaire_data::{Replay, ReplayMove};

    /// Build a minimal but well-formed [`Replay`] with `move_count` no-op
    /// `StockClick` entries. Tests only ever read `replay.moves.len()`
    /// (denominator of the progress indicator), so the move kind is
    /// irrelevant beyond producing the right count.
    fn synthetic_replay(move_count: usize) -> Replay {
        Replay::new(
            42,
            DrawMode::DrawOne,
            GameMode::Classic,
            120,
            1_000,
            NaiveDate::from_ymd_opt(2026, 5, 2).expect("valid date"),
            (0..move_count).map(|_| ReplayMove::StockClick).collect(),
        )
    }

    /// Build a test app that has the overlay plugin but **not** the
    /// playback plugin — tests insert `ReplayPlaybackState` manually so
    /// they can drive every state transition deterministically.
    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(ReplayOverlayPlugin);
        app.init_resource::<ReplayPlaybackState>();
        app
    }

    /// Count `ReplayOverlayRoot` entities in the world — the overlay's
    /// presence/absence is the spawn-test's primary observable.
    fn overlay_root_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&ReplayOverlayRoot>()
            .iter(app.world())
            .count()
    }

    /// Read the current text content of the unique progress-text entity.
    fn progress_text(app: &mut App) -> String {
        let mut q = app
            .world_mut()
            .query_filtered::<&Text, With<ReplayOverlayProgressText>>();
        q.iter(app.world())
            .next()
            .map(|t| t.0.clone())
            .unwrap_or_default()
    }

    /// Read the current text content of the unique banner-label entity.
    fn banner_text(app: &mut App) -> String {
        let mut q = app
            .world_mut()
            .query_filtered::<&Text, With<ReplayOverlayBannerText>>();
        q.iter(app.world())
            .next()
            .map(|t| t.0.clone())
            .unwrap_or_default()
    }

    /// Set the playback resource without going through the playback core.
    fn set_state(app: &mut App, state: ReplayPlaybackState) {
        app.world_mut().insert_resource(state);
    }

    /// Find the unique `ReplayStopButton` entity for the click-handler
    /// test. There must be exactly one.
    fn stop_button_entity(app: &mut App) -> Entity {
        let mut q = app
            .world_mut()
            .query_filtered::<Entity, With<ReplayStopButton>>();
        q.iter(app.world())
            .next()
            .expect("Stop button must exist while overlay is spawned")
    }

    /// Going `Inactive → Playing` spawns exactly one overlay root and
    /// the banner label reads "Replay".
    #[test]
    fn overlay_spawns_when_playback_starts() {
        let mut app = headless_app();
        // First update with the default `Inactive` resource — overlay
        // must not exist yet.
        app.update();
        assert_eq!(overlay_root_count(&mut app), 0);

        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
            },
        );
        app.update();

        assert_eq!(
            overlay_root_count(&mut app),
            1,
            "exactly one ReplayOverlayRoot must spawn on Inactive → Playing",
        );
        assert_eq!(banner_text(&mut app), "Replay");
    }

    /// The progress-text entity reads `"Move {cursor} of {total}"` for a
    /// well-formed `Playing` state.
    #[test]
    fn overlay_progress_text_reflects_cursor() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 5,
                secs_to_next: 0.5,
            },
        );
        app.update();

        assert_eq!(progress_text(&mut app), "Move 5 of 10");
    }

    /// Pressing the Stop button resets the state back to `Inactive` and
    /// the next frame's `react_to_state_change` despawns the overlay.
    /// Mirrors the synthetic `Interaction::Pressed` insertion pattern
    /// used elsewhere in the engine for headless click tests.
    #[test]
    fn overlay_stop_button_click_clears_playback() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
            },
        );
        app.update();
        assert_eq!(overlay_root_count(&mut app), 1);

        let stop = stop_button_entity(&mut app);
        app.world_mut()
            .entity_mut(stop)
            .insert(Interaction::Pressed);
        // Tick once: the click handler runs late in the frame and resets
        // the state to `Inactive`.
        app.update();

        // State must be back to Inactive.
        let state = app.world().resource::<ReplayPlaybackState>();
        assert!(
            matches!(state, ReplayPlaybackState::Inactive),
            "Stop click must reset ReplayPlaybackState to Inactive; got {state:?}",
        );

        // One more tick — `react_to_state_change` sees the resource
        // change to Inactive and despawns the overlay.
        app.update();
        assert_eq!(
            overlay_root_count(&mut app),
            0,
            "overlay must despawn the frame after state returns to Inactive",
        );
    }

    /// Manually flipping the resource back to `Inactive` (e.g. via the
    /// playback core's auto-clear after `Completed`) tears the overlay
    /// down without any further input.
    #[test]
    fn overlay_despawns_when_playback_returns_to_inactive() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(3),
                cursor: 1,
                secs_to_next: 0.5,
            },
        );
        app.update();
        assert_eq!(overlay_root_count(&mut app), 1);

        set_state(&mut app, ReplayPlaybackState::Inactive);
        app.update();

        assert_eq!(
            overlay_root_count(&mut app),
            0,
            "overlay must despawn on Playing → Inactive transition",
        );
    }

    /// On `Playing → Completed` the banner label updates in place rather
    /// than respawning. The overlay must still be present, and the label
    /// must read "Replay complete".
    #[test]
    fn overlay_text_changes_on_completed() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(7),
                cursor: 7,
                secs_to_next: 0.0,
            },
        );
        app.update();
        assert_eq!(banner_text(&mut app), "Replay");

        set_state(&mut app, ReplayPlaybackState::Completed);
        app.update();

        assert_eq!(
            overlay_root_count(&mut app),
            1,
            "overlay must remain spawned while in Completed state",
        );
        assert_eq!(
            banner_text(&mut app),
            "Replay complete",
            "banner label must swap on Playing → Completed",
        );
    }
}
