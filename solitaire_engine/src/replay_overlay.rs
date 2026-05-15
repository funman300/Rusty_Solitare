//! On-screen overlay shown while a recorded [`Replay`] plays back.
//!
//! The overlay is a thin top-of-window banner with three pieces of UI:
//!
//! - A "▌ replay" label on the left so the player knows the surface is
//!   under playback control rather than live input.
//! - A "MOVE N/M" progress chip in the centre, recomputed every frame
//!   the cursor advances and bordered in `ACCENT_PRIMARY` so it
//!   reads as a discrete callout.
//! - A "Stop" button on the right that aborts playback and returns
//!   control to the player.
//!
//! When playback finishes ([`ReplayPlaybackState::Completed`]) the banner
//! label swaps to "▌ replay complete" and stays visible until the playback
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
use chrono::Datelike;

use crate::font_plugin::FontResource;
use crate::layout::LayoutResource;
use crate::events::{DrawRequestEvent, MoveRequestEvent, UndoRequestEvent};
use crate::replay_playback::{
    step_backwards_replay_playback, step_replay_playback, stop_replay_playback,
    toggle_pause_replay_playback, ReplayPlaybackState,
};
use solitaire_core::pile::PileType;
use solitaire_data::ReplayMove;
use crate::ui_modal::{spawn_modal_button, ButtonVariant};
use crate::ui_theme::{
    ACCENT_PRIMARY, BG_ELEVATED_HI, BORDER_SUBTLE, HighContrastBackground, HighContrastBorder,
    STATE_SUCCESS, STATE_SUCCESS_HC, TEXT_PRIMARY, TEXT_PRIMARY_HC, TEXT_SECONDARY, TYPE_BODY,
    TYPE_CAPTION, TYPE_HEADLINE, VAL_SPACE_1, VAL_SPACE_2, VAL_SPACE_4, Z_DROP_OVERLAY,
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

/// `bevy::ui` `ZIndex` for the full-screen tableau dim layer.
///
/// One rung below [`Z_REPLAY_OVERLAY`] (= 54) so the replay chrome
/// (banner + move-log panel) renders clearly on top while the dim scrim
/// darkens the card world beneath it. World-space sprites (cards,
/// badges, drop-target overlays) are always below any UI node regardless
/// of their Transform.z — the dim layer doesn't need to know their z
/// values.
const Z_REPLAY_DIM: i32 = Z_REPLAY_OVERLAY - 1;

/// Alpha for the tableau dim layer — 50 % opacity black. Dark enough
/// to visually separate the gameplay scene from the replay chrome
/// above it; light enough that card positions remain legible through
/// the scrim. Matches the mockup's "Game Peek Band at 50 % opacity"
/// spec in `docs/ui-mockups/replay-overlay-mobile.html`.
const TABLEAU_DIM_ALPHA: f32 = 0.5;

/// Total height of the banner in pixels. Thin enough to leave the
/// gameplay surface visible underneath, tall enough to comfortably fit
/// the headline-sized "▌ replay" label stacked above the
/// `TYPE_CAPTION` "GAME #YYYY-DDD" subtitle (the left column needs
/// ~26 + 2 + 11 = 39 px of inner content; banner = top row (59
/// flex-grow) + scrub track (1) + label row (16) + footer (16)
/// gives 92).
///
/// Growth history:
/// - 60 → 76 in the scrub-notch-labels commit to make room for the
///   `0%` / … / `100%` percentage labels under each notch.
/// - 76 → 92 in the keybind-footer commit to make room for the
///   vim-style mode line + keybind-hint footer at the bottom.
const BANNER_HEIGHT: f32 = 92.0;

/// Height of the label row that sits below the 1px scrub track and
/// carries the `0%` / `25%` / `50%` / `75%` / `100%` notch labels.
/// 16 px is enough for `TYPE_CAPTION` text (12 px font + 4 px breathing
/// room above the bottom edge).
const SCRUB_LABEL_ROW_HEIGHT: f32 = 16.0;

/// Height of the keybind-hint footer that sits below the notch-label
/// row. Carries a vim-style mode indicator on the left and a
/// keybind-hint on the right (`[SPACE] pause/resume`). 16 px matches
/// `SCRUB_LABEL_ROW_HEIGHT` for visual symmetry — `TYPE_CAPTION` text
/// (12 px) + 4 px breathing room.
const KEYBIND_FOOTER_HEIGHT: f32 = 16.0;

/// Fixed pixel width of the centred scrub-bar notch-label container.
/// Wide enough to hold the widest label ("100%" at 4 chars) while
/// narrower than the 25 % gap between adjacent notches (≈ banner_w
/// × 0.25; on a 320 px banner that's 80 px). A 36 px container
/// leaves ≥ 44 px of clearance on each side at the narrowest common
/// screen width.
///
/// Container width drives the `margin.left = -width / 2` centering
/// trick: the container's left edge is placed at `left: Percent(pct)`
/// and then shifted left by half its own width, so the container's
/// centre coincides with the notch line. `Justify::Center` then
/// renders the text centred within the container. This is the
/// CSS `translateX(-50%)` pattern adapted for Bevy 0.18 UI.
const SCRUB_LABEL_CENTER_WIDTH: f32 = 36.0;

/// How long a held arrow key waits before firing the next repeat
/// step. 100 ms = 10 steps/sec — fast enough to scrub through a
/// hundred-move replay in ~10 seconds while held, slow enough that
/// the player can release after a known number of steps. Initial
/// `just_pressed` always fires immediately; this interval gates
/// only the *repeat* fires while the key remains held.
const SCRUB_REPEAT_INTERVAL_SECS: f32 = 0.1;

/// Total height of the bottom-edge Move Log panel in pixels.
/// Sized for: header (`TYPE_CAPTION` 11) + 2 prev rows + active
/// row + 2 next rows (`TYPE_BODY` 14 each = 70) + row gaps (~10)
/// + vertical padding (~16) ≈ 107; round to 112.
///
/// Growth history:
/// - 56 in the move-log-panel-init commit (header + active row).
/// - 56 → 84 in the move-log-prev-rows commit (+ 2 prev rows).
/// - 84 → 112 in the move-log-next-rows commit (+ 2 next rows).
const MOVE_LOG_PANEL_HEIGHT: f32 = 112.0;

/// Number of "previous move" rows rendered above the active row
/// in the move-log panel. Tuned to fit the panel height comfortably
/// alongside the header + active row at `TYPE_BODY`. The active
/// row plus this many prev rows gives the player a 3-row window
/// onto recent move history.
const MOVE_LOG_PREV_ROWS: usize = 2;

/// Number of "next move" rows rendered below the active row.
/// Same logic as [`MOVE_LOG_PREV_ROWS`] — symmetric window
/// around the active row showing about-to-apply moves. For a
/// post-game replay these aren't spoilers (the game is already
/// won); for a future "live preview during play" use case the
/// preview-shape might need rethinking.
const MOVE_LOG_NEXT_ROWS: usize = 2;

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

/// Marker on the left-hand banner label `Text`. Carries either
/// "▌ replay" (during playback) or "▌ replay complete" (once
/// finished — the cursor-block prefix matches the splash boot-screen
/// idiom so the surface reads as a Terminal output line); the
/// completion-text-update system swaps the contents in place.
#[derive(Component, Debug)]
pub struct ReplayOverlayBannerText;

/// Marker on the centre progress `Text`. Updated every frame to reflect
/// the current `(cursor, total)` returned by
/// [`ReplayPlaybackState::progress`].
#[derive(Component, Debug)]
pub struct ReplayOverlayProgressText;

/// Marker on the **floating** progress chip — a 2D world-space text
/// entity rendered above the destination pile of the most-recently-
/// applied move. Sits independently of the banner overlay (which
/// lives in the UI tree and never moves) so the player can see
/// progress without breaking eye contact with the focal card.
///
/// Lifecycle matches the banner overlay: spawned by `spawn_overlay`
/// when a replay starts, despawned by `react_to_state_change` when
/// it ends. Position updated each frame by
/// `update_floating_progress_chip`. Hidden when cursor=0 (no moves
/// applied yet) or the last applied move was a `StockClick` (no
/// destination pile to follow).
#[derive(Component, Debug)]
pub struct ReplayFloatingProgressChip;

/// Marker on the right-hand "Stop" button. Click handler queries for this
/// and calls [`stop_replay_playback`] when an `Interaction::Pressed`
/// transition is seen.
#[derive(Component, Debug)]
pub struct ReplayStopButton;

/// Marker on the Pause / Resume button. Click handler queries for this
/// and calls [`toggle_pause_replay_playback`] on each press. The
/// button's label text is repainted in lockstep by
/// `update_pause_button_label` so it always reflects the action the
/// next click will perform ("Pause" while running, "Resume" while
/// paused).
#[derive(Component, Debug)]
pub struct ReplayPauseButton;

/// Marker on the Step button. Click handler queries for this and
/// calls [`step_replay_playback`] — only meaningful when paused
/// (clicks while running are no-ops because the tick loop would race
/// the manual advance). The button stays visually present but
/// unresponsive while the playback is running so the player has a
/// stable layout to scan.
#[derive(Component, Debug)]
pub struct ReplayStepButton;

/// Marker on the full-screen tableau dim layer spawned at the start of
/// every replay. The dim layer is a 100 % × 100 % `Node` at
/// [`Z_REPLAY_DIM`] (= `Z_REPLAY_OVERLAY - 1`) with a semi-transparent
/// black `BackgroundColor`. It darkens the card world so the replay
/// chrome reads clearly against it without obscuring card positions.
///
/// Carries no [`Interaction`] component — purely visual; pointer events
/// pass through to the underlying UI and world-space systems.
/// Despawned by `react_to_state_change` when the replay ends.
#[derive(Component, Debug)]
pub struct ReplayTableauDimLayer;

/// Marker on the small caption sitting below the "▌ replay"
/// headline. Carries `GAME #YYYY-DDD` (year + chrono ordinal) while a
/// replay is playing — a compact, monotonically-increasing identifier
/// that mirrors the `▌replay.tsx` / `GAME #2024-127` Terminal-output
/// motif from the mockup. The caption is empty in `Inactive` /
/// `Completed` since the replay is consumed when transitioning out
/// of `Playing` and the identifier is no longer recoverable from
/// state alone.
#[derive(Component, Debug)]
pub struct ReplayOverlayGameCaption;

/// Marker on the accent "fill" of the bottom-edge scrub bar. The
/// `Node`'s `width` is rewritten every frame the cursor advances to
/// `cursor / total` of the bar's full width, so the player has a
/// continuous visual cue of how far through the replay they are.
///
/// Distinct from the simpler text-based `ReplayOverlayProgressText`
/// (which spells out "MOVE N/M" in a chip): the scrub fill gives immediate
/// at-a-glance positioning; the text gives the exact numbers. Both
/// surfaces stay together because they answer the same question for
/// players with different scanning preferences.
#[derive(Component, Debug)]
pub struct ReplayOverlayScrubFill;

/// Marker for the WIN MOVE tick on the scrub bar — a small absolute-
/// positioned `Node` anchored at `replay.win_move_index / total` along
/// the track. Painted in [`STATE_SUCCESS`] so the player can see at a
/// glance where the winning move sits relative to the playback cursor.
///
/// Static — the position is set at spawn time and never changes during
/// playback (the underlying replay's `win_move_index` is immutable
/// while `Playing`). Despawned with the rest of the overlay tree when
/// the replay state transitions back to `Inactive`.
///
/// Spawned only when the active replay carries
/// [`Replay::win_move_index`](solitaire_data::Replay::win_move_index)
/// `= Some(_)` — older replays loaded from disk pre-date the field
/// and have no win index to surface.
#[derive(Component, Debug)]
pub struct ReplayOverlayWinMoveMarker;

/// Marker for the fixed-position notches on the scrub bar — five 1px
/// vertical ticks at 0 % / 25 % / 50 % / 75 % / 100 % that give the
/// player visual anchor points for "where am I, relative to the
/// quarter-marks of the replay." Mirrors the notch ladder in the
/// screen-takeover mockup at
/// `docs/ui-mockups/replay-overlay-mobile.html`.
///
/// Static — positions are set at spawn time and never change. The
/// notches paint in [`BORDER_SUBTLE`] which is the same colour as the
/// unfilled track, so visibility comes from extending the notch
/// **vertically past** the 1px track (5px tall, anchored 2px above
/// the track top) rather than from colour contrast. Same trick the
/// WIN MOVE marker uses.
#[derive(Component, Debug)]
pub struct ReplayOverlayScrubNotch;

/// Marker for the percentage labels under each scrub-bar notch
/// (`0%` / `25%` / `50%` / `75%` / `100%`). One label per notch;
/// labels live in a dedicated 16 px row below the 1 px scrub track
/// (the row that grew the banner from 60 → 76 px).
///
/// Positioning follows a "endpoints flush to edges, middle three
/// anchored at percentage" pattern: the leftmost label uses
/// `left: 0`, the rightmost uses `right: 0`, and the middle three
/// (`25%` / `50%` / `75%`) anchor at `left: Val::Percent(p)`. This
/// avoids overflow at 100 % without needing CSS-style
/// `translate-x: -50%` centering (which Bevy 0.18 UI doesn't have a
/// clean equivalent for) — the trade-off is a slight right-of-notch
/// offset on the middle three, which is visually subtle at the
/// `TYPE_CAPTION` font size.
#[derive(Component, Debug)]
pub struct ReplayOverlayScrubNotchLabel;

/// Per-arrow-key time-since-last-fire accumulators that drive the
/// continuous-scrub repeat behaviour for held arrow keys. Each
/// frame the key is held, the corresponding accumulator absorbs
/// `time.delta_secs()`; when it exceeds
/// [`SCRUB_REPEAT_INTERVAL_SECS`] the handler fires another step
/// and resets the accumulator.
///
/// `just_pressed` events bypass the accumulator entirely and fire
/// immediately — only *repeat* fires (while held) are gated by
/// the interval. Releases reset the accumulator to 0 so the next
/// fresh press fires immediately rather than at half-interval.
#[derive(Resource, Default, Debug)]
struct ReplayScrubKeyHold {
    left_held_secs: f32,
    right_held_secs: f32,
}

/// Marker on the keybind-hint footer row at the bottom edge of the
/// banner. Carries two `Text` children: a vim-style mode indicator
/// (`▌ NORMAL │ replay`) on the left and the keybind hint
/// (`[SPACE] pause/resume`) on the right. 1 px top border in
/// [`BORDER_SUBTLE`] separates it from the notch-label row above.
///
/// Surfaces the existing Space-key accelerator visually so the
/// UI-first contract from CLAUDE.md §3.3 (every player action has
/// a visible UI control) holds for keyboard accelerators too.
/// Future commits that wire ESC for stop or ← / → for scrub will
/// extend the right-hand text in lockstep — the footer always
/// reflects what's actually wired, never aspirational.
#[derive(Component, Debug)]
pub struct ReplayOverlayKeybindFooter;

/// Marker on the bottom-edge **Move Log** panel — a separate root
/// UI entity (not a child of the banner) that sits anchored to the
/// viewport's bottom edge. Carries a header (`▌ MOVE LOG · N/M`)
/// plus a row showing the most-recently-applied move.
///
/// Spawned by `spawn_overlay` alongside the banner and the
/// floating progress chip; despawned by `react_to_state_change`
/// on the same `Playing → Inactive` transition. Same lifecycle
/// pattern as `ReplayFloatingProgressChip` — a sibling root, not
/// a banner child, because it lives at a different screen anchor.
///
/// First slice of the move-log mockup at
/// `docs/ui-mockups/replay-overlay-mobile.html` § "Move Log Card".
/// Subsequent commits add prev/next rows and scrolling.
#[derive(Component, Debug)]
pub struct ReplayOverlayMoveLogPanel;

/// Marker on the move-log panel's header `Text`. Carries
/// `▌ MOVE LOG · N/M` while a replay is playing; the
/// `update_move_log_header` system repaints it as the cursor
/// advances.
#[derive(Component, Debug)]
pub struct ReplayOverlayMoveLogHeader;

/// Marker on the move-log panel's active-row `Text`. Carries the
/// most-recently-applied move's text (`47 │ waste → tableau 5`)
/// when `cursor > 0`; empty when no moves have been applied yet
/// (initial spawn) or in `Completed`/`Inactive` states. The
/// `update_move_log_active_row` system repaints it as the cursor
/// advances.
#[derive(Component, Debug)]
pub struct ReplayOverlayMoveLogActiveRow;

/// Marker on a "previous move" row above the active row.
/// `offset` is the 1-based distance backwards from the active
/// row: `offset = 1` is the move applied just before the active
/// one (e.g. cursor=47 → row reads "46 │ ..."), `offset = 2` is
/// the one before that, and so on. Up to [`MOVE_LOG_PREV_ROWS`]
/// rows render above the active row.
///
/// Empty text when there isn't enough history (`offset >= cursor`,
/// e.g. cursor=1 has no prev rows; cursor=2 has only the
/// `offset = 1` row populated).
#[derive(Component, Debug)]
pub struct ReplayOverlayMoveLogPrevRow {
    /// Distance backwards from the active row (1-based).
    pub offset: u8,
}

/// Marker on a "next move" row below the active row. `offset`
/// is the 1-based distance forward from the active row:
/// `offset = 1` is the move that will apply next
/// (`replay.moves[cursor]`, displayed as `cursor + 1`),
/// `offset = 2` is the one after that, and so on. Up to
/// [`MOVE_LOG_NEXT_ROWS`] rows render below the active row.
///
/// Empty text when there isn't enough remaining replay
/// (`cursor + offset - 1 >= moves.len()`, e.g. cursor=99 of
/// a 100-move replay shows offset 1 but offset 2 stays empty).
#[derive(Component, Debug)]
pub struct ReplayOverlayMoveLogNextRow {
    /// Distance forward from the active row (1-based).
    pub offset: u8,
}

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
        // Step-button handler dispatches into the same canonical move
        // / draw events that the tick loop fires. Register them
        // defensively here so this plugin can run under
        // `MinimalPlugins` without the playback plugin attached;
        // `add_message` is idempotent so the duplicate registration
        // in production (alongside `replay_playback`) is harmless.
        app.init_resource::<ReplayScrubKeyHold>()
            .add_message::<MoveRequestEvent>()
            .add_message::<DrawRequestEvent>()
            .add_message::<UndoRequestEvent>()
            .add_systems(
                Update,
                (
                    react_to_state_change,
                    update_banner_label,
                    update_progress_text,
                    update_floating_progress_chip,
                    update_scrub_fill,
                    update_move_log_header,
                    update_move_log_active_row,
                    update_move_log_prev_rows,
                    update_move_log_next_rows,
                    update_pause_button_label,
                    handle_pause_button,
                    handle_step_button,
                    handle_pause_keyboard,
                    handle_stop_keyboard,
                    handle_arrow_keyboard,
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
    floating_chips: Query<Entity, With<ReplayFloatingProgressChip>>,
    move_log_panels: Query<Entity, With<ReplayOverlayMoveLogPanel>>,
    dim_layers: Query<Entity, With<ReplayTableauDimLayer>>,
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
        // Floating chip lives outside the UI tree (world-space
        // entity), so the banner-root despawn doesn't reach it.
        // Despawn separately on the same state transition so both
        // disappear together when the replay ends.
        for entity in &floating_chips {
            commands.entity(entity).despawn();
        }
        // Move-log panel is also a separate root entity (sibling
        // of the banner anchored to the viewport's bottom edge),
        // so the banner-root despawn doesn't reach it either.
        for entity in &move_log_panels {
            commands.entity(entity).despawn();
        }
        // Tableau dim layer is also a separate root entity — same
        // pattern as the move-log panel.
        for entity in &dim_layers {
            commands.entity(entity).despawn();
        }
    }
    // The `should_be_visible && already_spawned` branch is a no-op here —
    // the per-frame text update systems below repaint the banner label
    // and progress readout in place without a respawn.
}

/// Spawns the banner — a flex-row Node anchored to the top edge of the
/// window with three children: the "▌ replay" / "▌ replay complete" label,
/// the centred progress text, and the right-aligned Stop button.
fn spawn_overlay(
    commands: &mut Commands,
    font_res: Option<&FontResource>,
    state: &ReplayPlaybackState,
) {
    let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();
    // Clone for the floating chip spawn that runs *after* the
    // banner's `.with_children(|banner| { ... })` closure consumes
    // the original `font_handle`. Cheap — Bevy's `Handle<Font>` is
    // `Arc`-backed, the clone bumps a refcount.
    let font_handle_for_floating = font_handle.clone();
    // Second clone for the scrub-bar label row and keybind footer
    // inside the outer banner closure. The inner top-row closure
    // consumes the original `font_handle` for the progress-chip
    // text, so by the time the outer closure reaches the
    // label-row / footer spawns the original is gone.
    // `font_handle_for_labels` is `.clone()`'d (never moved) inside
    // the labels closure, so it's still alive for the footer
    // spawn afterwards — single shared clone covers both.
    let font_handle_for_labels = font_handle.clone();
    // Third clone for the move-log panel — a separate root
    // entity spawned after the banner closure closes. Mirrors the
    // floating-chip clone reasoning.
    let font_handle_for_move_log = font_handle.clone();

    let banner_label = if state.is_completed() {
        "\u{258C} replay complete" // ▌ — cursor-block prefix; matches the splash boot-screen convention.
    } else {
        "\u{258C} replay" // ▌
    };
    let progress_label = format_progress(state);

    // Tableau dim layer — full-screen scrim at z = Z_REPLAY_DIM (= 54).
    // Spawned first so it sits behind the banner (z=55) and move-log (z=55)
    // in the UI stacking context. World-space sprites (cards, badges) are
    // always below any UI node, so the dim layer darkens the entire
    // gameplay scene without needing to touch card_plugin. No Interaction
    // component — purely visual.
    commands.spawn((
        ReplayTableauDimLayer,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, TABLEAU_DIM_ALPHA)),
        ZIndex(Z_REPLAY_DIM),
        GlobalZIndex(Z_REPLAY_DIM),
    ));

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
                // Column outer so the content row sits above the 1px
                // scrub bar at the bottom edge.
                flex_direction: FlexDirection::Column,
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
            // Top row: the existing content (label / progress / Stop).
            banner
                .spawn(Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    padding: UiRect::axes(VAL_SPACE_4, VAL_SPACE_2),
                    column_gap: VAL_SPACE_4,
                    ..default()
                })
                .with_children(|row| {
                    // Left: column with the accent "▌ replay" headline
                    // above and a small `GAME #YYYY-DDD` caption below.
                    // The caption mirrors the mockup's right-anchored
                    // game identifier but stays visually grouped with
                    // the headline so the two pieces of "this is a
                    // replay of game X" read as a single unit.
                    row.spawn(Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexStart,
                        row_gap: Val::Px(2.0),
                        ..default()
                    })
                    .with_children(|left| {
                        left.spawn((
                            ReplayOverlayBannerText,
                            Text::new(banner_label),
                            TextFont {
                                font: font_handle.clone(),
                                font_size: TYPE_HEADLINE,
                                ..default()
                            },
                            TextColor(ACCENT_PRIMARY),
                        ));
                        left.spawn((
                            ReplayOverlayGameCaption,
                            Text::new(format_game_caption(state).unwrap_or_default()),
                            TextFont {
                                font: font_handle.clone(),
                                font_size: TYPE_CAPTION,
                                ..default()
                            },
                            TextColor(TEXT_SECONDARY),
                        ));
                    });

                    // Centre: progress readout, wrapped in a 1 px
                    // ACCENT_PRIMARY-bordered chip so it reads as a
                    // discrete callout rather than free-floating
                    // text. No fill — the Terminal aesthetic gets
                    // depth from borders + tonal layering, not
                    // shadows. The marker stays on the inner Text so
                    // `update_progress_text` keeps working unchanged.
                    row.spawn((
                        Node {
                            border: UiRect::all(Val::Px(1.0)),
                            padding: UiRect::axes(VAL_SPACE_2, VAL_SPACE_1),
                            ..default()
                        },
                        BorderColor::all(ACCENT_PRIMARY),
                    ))
                    .with_children(|chip| {
                        chip.spawn((
                            ReplayOverlayProgressText,
                            Text::new(progress_label),
                            TextFont {
                                font: font_handle,
                                font_size: TYPE_BODY,
                                ..default()
                            },
                            TextColor(TEXT_PRIMARY),
                        ));
                    });

                    // Right: Stop button. Tertiary variant — the
                    // action is available but not the loudest element
                    // in the banner; the "Replay" primary accent owns
                    // that slot. `spawn_modal_button` gives us hover /
                    // press paint and focus rings for free via the
                    // existing `UiModalPlugin` paint system.
                    row.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: VAL_SPACE_2,
                        ..default()
                    })
                    .with_children(|wrap| {
                        // Pause / Resume label is set from the current
                        // state so a freshly-spawned overlay (which
                        // currently always starts unpaused) reads
                        // "Pause". `update_pause_button_label`
                        // repaints it whenever the state changes.
                        spawn_modal_button(
                            wrap,
                            ReplayPauseButton,
                            pause_button_label(state),
                            None,
                            ButtonVariant::Tertiary,
                            font_res,
                        );
                        spawn_modal_button(
                            wrap,
                            ReplayStepButton,
                            "Step",
                            None,
                            ButtonVariant::Tertiary,
                            font_res,
                        );
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

            // Bottom edge: 1px-tall scrub bar. Track in `BORDER_SUBTLE`,
            // fill in `ACCENT_PRIMARY`. The fill width is rewritten by
            // [`update_scrub_fill`] every tick the cursor advances.
            // Initial fill width matches the spawn-time progress so the
            // first-frame paint already reflects state instead of
            // popping from 0 → cursor on the first tick.
            let initial_scrub_pct = scrub_pct(state);
            let win_pct = win_move_marker_pct(state);
            banner
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(1.0),
                        ..default()
                    },
                    BackgroundColor(BORDER_SUBTLE),
                    // HC marker: bumps the 1 px track from #505050
                    // → #a0a0a0 under high-contrast mode. The track
                    // paints via BackgroundColor (it's a 1 px Node,
                    // not a border on a wider container) so the
                    // BorderColor-targeting HighContrastBorder marker
                    // doesn't apply — HighContrastBackground is the
                    // parallel primitive for this case.
                    HighContrastBackground::with_default(BORDER_SUBTLE),
                ))
                .with_children(|track| {
                    track.spawn((
                        ReplayOverlayScrubFill,
                        Node {
                            width: Val::Percent(initial_scrub_pct),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(ACCENT_PRIMARY),
                    ));
                    // WIN MOVE marker — small green tick anchored at
                    // `win_move_index / total`. Spawned only when the
                    // active replay carries the field; older replays
                    // pre-dating `win_move_index` simply don't get a
                    // marker. Centered vertically on the 1px track via
                    // a 3px-tall node offset 1px above the track top so
                    // 1px sits above and 1px below the track line.
                    if let Some(pct) = win_pct {
                        track.spawn((
                            ReplayOverlayWinMoveMarker,
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(pct),
                                top: Val::Px(-1.0),
                                width: Val::Px(2.0),
                                height: Val::Px(3.0),
                                ..default()
                            },
                            BackgroundColor(STATE_SUCCESS),
                            // HC bump: lime → brighter lime so the win
                            // marker reads clearly above the bumped
                            // notch ticks (BORDER_SUBTLE_HC gray) under
                            // high-contrast mode.
                            HighContrastBackground::with_hc(STATE_SUCCESS, STATE_SUCCESS_HC),
                        ));
                    }
                    // Fixed quarter-mark notches: five 1px vertical
                    // ticks at 0 / 25 / 50 / 75 / 100 % that give the
                    // player visual anchor points without needing to
                    // mentally bisect the bar. Painted in
                    // BORDER_SUBTLE — same colour as the unfilled
                    // track — so visibility comes from extending past
                    // the 1px track height (5px tall, anchored 2px
                    // above the track top) rather than colour
                    // contrast. Spawned *after* the WIN MOVE marker
                    // so a notch and the marker landing on the same
                    // percentage paint the marker on top.
                    for pct in scrub_notch_positions() {
                        track.spawn((
                            ReplayOverlayScrubNotch,
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(pct),
                                top: Val::Px(-2.0),
                                width: Val::Px(1.0),
                                height: Val::Px(5.0),
                                ..default()
                            },
                            BackgroundColor(BORDER_SUBTLE),
                            // Same HC-paint reasoning as the track
                            // above: 5 px tall × 1 px wide tick mark
                            // paints via BackgroundColor, so
                            // HighContrastBackground (not -Border) is
                            // the right marker.
                            HighContrastBackground::with_default(BORDER_SUBTLE),
                        ));
                    }
                });

            // Third banner row: percentage labels (`0%` / `25%` /
            // `50%` / `75%` / `100%`) under each scrub-bar notch.
            // Sibling of (not child of) the 1px track because labels
            // need their own vertical real estate (TYPE_CAPTION text
            // doesn't fit inside a 1px container). Position math:
            // track Node has `Val::Percent(p)` referencing the
            // banner's full width; this label row also has the
            // banner's full width, so labels at the same
            // percentages line up vertically with their notches.
            let labels = scrub_notch_labels();
            let positions = scrub_notch_positions();
            banner
                .spawn(Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(SCRUB_LABEL_ROW_HEIGHT),
                    position_type: PositionType::Relative,
                    ..default()
                })
                .with_children(|row| {
                    for (i, (label, pct)) in
                        labels.iter().zip(positions.iter()).enumerate()
                    {
                        // Endpoints flush to the row's edges; middle
                        // three labels use the `translateX(-50%)`
                        // pattern for Bevy 0.18 UI: a fixed-width
                        // container is placed at `left: Percent(pct)`
                        // then shifted left by half its own width via
                        // `margin.left: Px(-SCRUB_LABEL_CENTER_WIDTH/2)`.
                        // `Justify::Center` renders the text centred
                        // within the container so the text's visual
                        // centre coincides with the notch line.
                        let (node, justify) = if i == 0 {
                            (
                                Node {
                                    position_type: PositionType::Absolute,
                                    top: Val::Px(2.0),
                                    left: Val::Px(0.0),
                                    ..default()
                                },
                                Justify::Left,
                            )
                        } else if i == labels.len() - 1 {
                            (
                                Node {
                                    position_type: PositionType::Absolute,
                                    top: Val::Px(2.0),
                                    right: Val::Px(0.0),
                                    ..default()
                                },
                                Justify::Right,
                            )
                        } else {
                            (
                                Node {
                                    position_type: PositionType::Absolute,
                                    top: Val::Px(2.0),
                                    left: Val::Percent(*pct),
                                    width: Val::Px(SCRUB_LABEL_CENTER_WIDTH),
                                    margin: UiRect {
                                        left: Val::Px(-SCRUB_LABEL_CENTER_WIDTH / 2.0),
                                        ..default()
                                    },
                                    ..default()
                                },
                                Justify::Center,
                            )
                        };
                        row.spawn((
                            ReplayOverlayScrubNotchLabel,
                            node,
                            Text::new(*label),
                            TextLayout::new_with_justify(justify),
                            TextFont {
                                font: font_handle_for_labels.clone(),
                                font_size: TYPE_CAPTION,
                                ..default()
                            },
                            // TEXT_SECONDARY keeps the subdued visual
                            // hierarchy (caption, not headline) while
                            // staying readable against BG_ELEVATED_HI.
                            TextColor(TEXT_SECONDARY),
                        ));
                    }
                });

            // Fourth banner row: keybind-hint footer. Vim-style
            // mode line on the left (`▌ NORMAL │ replay`), keybind
            // hint on the right (`[SPACE] pause/resume`), 1px top
            // border in BORDER_SUBTLE separating it from the
            // labels row above. Surfaces the existing Space
            // accelerator visually so CLAUDE.md §3.3's UI-first
            // contract holds for keyboard accelerators too.
            banner
                .spawn((
                    ReplayOverlayKeybindFooter,
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(KEYBIND_FOOTER_HEIGHT),
                        flex_direction: FlexDirection::Row,
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        padding: UiRect::horizontal(VAL_SPACE_4),
                        border: UiRect::top(Val::Px(1.0)),
                        ..default()
                    },
                    BorderColor::all(BORDER_SUBTLE),
                    // Marker for `apply_high_contrast_borders`: bumps
                    // the 1 px top border from BORDER_SUBTLE (#505050)
                    // to BORDER_SUBTLE_HC (#a0a0a0) when
                    // `Settings::high_contrast_mode` is on. Without
                    // this the footer reads as floating loose under
                    // HC because the border that visually anchors it
                    // to the labels row above is near-invisible.
                    HighContrastBorder::with_default(BORDER_SUBTLE),
                ))
                .with_children(|footer| {
                    footer.spawn((
                        Text::new(keybind_footer_mode_text()),
                        TextFont {
                            font: font_handle_for_labels.clone(),
                            font_size: TYPE_CAPTION,
                            ..default()
                        },
                        TextColor(TEXT_SECONDARY),
                    ));
                    #[cfg(not(target_os = "android"))]
                    footer.spawn((
                        Text::new(keybind_footer_hint_text()),
                        TextFont {
                            font: font_handle_for_labels.clone(),
                            font_size: TYPE_CAPTION,
                            ..default()
                        },
                        TextColor(TEXT_SECONDARY),
                    ));
                });
        });

    // Floating progress chip — a 2D world-space `Text2d` rendered
    // above the destination pile of the most-recently-applied move.
    // Sibling of (not child of) the banner overlay because it lives
    // in world-space coordinates, not the UI tree. Spawned hidden;
    // `update_floating_progress_chip` shows + positions it on the
    // first frame the cursor advances past 0. Lifecycle matches
    // the banner overlay — `react_to_state_change` despawns both
    // when the replay state transitions back to `Inactive`.
    commands.spawn((
        ReplayFloatingProgressChip,
        Text2d::new(format_progress(state)),
        TextFont {
            font: font_handle_for_floating,
            font_size: TYPE_BODY,
            ..default()
        },
        TextColor(TEXT_PRIMARY),
        // High Z keeps the chip above every card stack
        // (Z_DROP_OVERLAY = 50, Z_STOCK_BADGE = 30, regular cards
        // stack to the low double digits at most).
        Transform::from_xyz(0.0, 0.0, 100.0),
        Visibility::Hidden,
    ));

    // Move-log panel — a separate root UI entity anchored to the
    // viewport's bottom edge. Carries a `▌ MOVE LOG · N/M` header
    // plus a row showing the most-recently-applied move.
    // Sibling-of-banner pattern (not a banner child) because the
    // panel lives at a different screen anchor and has its own
    // spawn/despawn lifecycle synced via `react_to_state_change`.
    let banner_bg = Color::srgba(
        BG_ELEVATED_HI.to_srgba().red,
        BG_ELEVATED_HI.to_srgba().green,
        BG_ELEVATED_HI.to_srgba().blue,
        BANNER_ALPHA,
    );
    commands
        .spawn((
            ReplayOverlayMoveLogPanel,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                bottom: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Px(MOVE_LOG_PANEL_HEIGHT),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                justify_content: JustifyContent::Center,
                padding: UiRect::axes(VAL_SPACE_4, VAL_SPACE_2),
                row_gap: VAL_SPACE_1,
                border: UiRect::top(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(banner_bg),
            BorderColor::all(BORDER_SUBTLE),
            // Same z-stack rationale as the banner — above gameplay,
            // below modals.
            ZIndex(Z_REPLAY_OVERLAY),
            GlobalZIndex(Z_REPLAY_OVERLAY),
            // HC marker so the top border bumps under HC mode.
            // Without it the panel reads as floating loose because
            // the border that anchors it to the gameplay area above
            // is near-invisible at #505050.
            HighContrastBorder::with_default(BORDER_SUBTLE),
        ))
        .with_children(|panel| {
            // Header row: `▌ MOVE LOG · N/M` in ACCENT_PRIMARY for
            // the cursor-block prefix consistency with the banner
            // headline.
            panel.spawn((
                ReplayOverlayMoveLogHeader,
                Text::new(format_move_log_header(state)),
                TextFont {
                    font: font_handle_for_move_log.clone(),
                    font_size: TYPE_CAPTION,
                    ..default()
                },
                TextColor(ACCENT_PRIMARY),
            ));
            // Prev rows — render above the active row in display
            // order (oldest first), so the active row sits at the
            // bottom of the visible window. Spawn from
            // MOVE_LOG_PREV_ROWS down to 1 (offset 2, then 1) so
            // the highest-offset (oldest) row is topmost in the
            // panel's flex column. Each carries
            // ReplayOverlayMoveLogPrevRow { offset } — the
            // per-frame system reads `offset` and recomputes the
            // text on cursor advance. Painted in TEXT_SECONDARY
            // so the active row stands out from context rows.
            for offset in (1..=MOVE_LOG_PREV_ROWS as u8).rev() {
                panel.spawn((
                    ReplayOverlayMoveLogPrevRow { offset },
                    Text::new(format_kth_recent_row(
                        state,
                        offset as usize + 1,
                    )),
                    TextFont {
                        font: font_handle_for_move_log.clone(),
                        font_size: TYPE_BODY,
                        ..default()
                    },
                    TextColor(TEXT_SECONDARY),
                ));
            }
            // Active move row. Wrapped in a Node with an
            // ACCENT_PRIMARY background so the row reads as
            // "current focus" — the player can scan vertically
            // and the highlighted row is the move that just
            // applied. Empty text at spawn time when cursor=0;
            // the per-frame update system populates it as the
            // cursor advances. Text colour is TEXT_PRIMARY_HC
            // (near-white) for contrast against the brick-red
            // background — same trick as the modal-button
            // primary-variant paint.
            panel
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        padding: UiRect::axes(VAL_SPACE_2, VAL_SPACE_1),
                        ..default()
                    },
                    BackgroundColor(ACCENT_PRIMARY),
                ))
                .with_children(|active| {
                    active.spawn((
                        ReplayOverlayMoveLogActiveRow,
                        Text::new(format_active_move_row(state)),
                        TextFont {
                            font: font_handle_for_move_log.clone(),
                            font_size: TYPE_BODY,
                            ..default()
                        },
                        TextColor(TEXT_PRIMARY_HC),
                    ));
                });
            // Next rows — render below the active row in display
            // order (offset 1 directly below active, then offset
            // 2). Same TEXT_SECONDARY de-emphasis as prev rows so
            // the active row stays the focal point. Empty text
            // late in the replay (when cursor + offset exceeds
            // moves.len()) — the panel under-fills gracefully.
            for offset in 1..=MOVE_LOG_NEXT_ROWS as u8 {
                panel.spawn((
                    ReplayOverlayMoveLogNextRow { offset },
                    Text::new(format_kth_next_row(state, offset as usize)),
                    TextFont {
                        font: font_handle_for_move_log.clone(),
                        font_size: TYPE_BODY,
                        ..default()
                    },
                    TextColor(TEXT_SECONDARY),
                ));
            }
        });
}

/// Pure helper — returns the scrub-fill width as a percentage of the
/// track for the given playback state. `Completed` reads as 100 %;
/// `Inactive` and `Playing` with no progress read as 0 %.
fn scrub_pct(state: &ReplayPlaybackState) -> f32 {
    if state.is_completed() {
        return 100.0;
    }
    match state.progress() {
        Some((_, 0)) | None => 0.0,
        Some((cursor, total)) => {
            let frac = (cursor as f32 / total as f32).clamp(0.0, 1.0);
            frac * 100.0
        }
    }
}

/// Pure helper — returns the fixed scrub-bar notch positions as
/// percentages along the track. Five evenly-spaced notches at the
/// quarter-marks: `[0, 25, 50, 75, 100]`. Function (rather than
/// const) so the unit-test surface is obvious and a future
/// regression — e.g. someone simplifying to three notches — fails
/// at the helper test rather than at visual review.
fn scrub_notch_positions() -> [f32; 5] {
    [0.0, 25.0, 50.0, 75.0, 100.0]
}

/// Pure helper — returns the percentage-label text for each notch,
/// in left-to-right order. Paired with [`scrub_notch_positions`] so
/// `labels[i]` belongs at `positions[i]`. Lifted to a function for
/// the same reason as the positions helper: a clean unit-test
/// surface that fails at a regression (e.g. someone simplifying
/// `100%` → `MAX`) rather than at visual review.
fn scrub_notch_labels() -> [&'static str; 5] {
    ["0%", "25%", "50%", "75%", "100%"]
}

/// Pure helper — returns the vim-style mode indicator text shown on
/// the left side of the keybind-hint footer row. `▌ NORMAL │ replay`
/// matches the `▌replay.tsx` motif from the splash boot-screen and
/// the screen-takeover mockup. The cursor block (`▌`) matches the
/// banner-label prefix; "NORMAL" is the vim mode (mockup parity);
/// "replay" identifies the surface.
fn keybind_footer_mode_text() -> &'static str {
    "\u{258C} NORMAL \u{2502} replay" // ▌ NORMAL │ replay
}

/// Pure helper — returns the keybind-hint text shown on the right
/// side of the keybind-hint footer row. Lists only the keys that
/// are *actually wired* today: the Space accelerator for
/// pause/resume, the ESC accelerator for stop, and the ← / →
/// accelerators for paused single-move stepping. The footer never
/// lists unimplemented keybinds (would lie to users).
#[cfg(not(target_os = "android"))]
fn keybind_footer_hint_text() -> &'static str {
    "[SPACE] pause/resume \u{00B7} [ESC] stop \u{00B7} [\u{2190}\u{2192}] step" // · separator
}

/// Pure helper — returns the WIN MOVE marker's left-edge position as
/// a percentage of the scrub track, or `None` when no marker should
/// be drawn.
///
/// `None` is returned in any of these cases:
/// - The state isn't `Playing` (no replay attached).
/// - The replay's `win_move_index` is `None` (older replay loaded
///   from disk pre-dating the field).
/// - The replay's move list is empty (shouldn't happen for real wins,
///   but guards the divide-by-zero).
///
/// The percentage clamps to `[0, 100]` so a malformed
/// `win_move_index >= total` (defensive — shouldn't happen) doesn't
/// position the marker outside the track.
fn win_move_marker_pct(state: &ReplayPlaybackState) -> Option<f32> {
    let ReplayPlaybackState::Playing { replay, .. } = state else {
        return None;
    };
    let idx = replay.win_move_index?;
    let total = replay.moves.len();
    if total == 0 {
        return None;
    }
    let frac = (idx as f32 / total as f32).clamp(0.0, 1.0);
    Some(frac * 100.0)
}

// ---------------------------------------------------------------------------
// Per-frame text updates
// ---------------------------------------------------------------------------

/// Overwrites the banner label whenever the resource changes — covers the
/// `Playing → Completed` transition by swapping "▌ replay" for
/// "▌ replay complete" in place without despawning the overlay.
fn update_banner_label(
    state: Res<ReplayPlaybackState>,
    mut q: Query<&mut Text, With<ReplayOverlayBannerText>>,
) {
    if !state.is_changed() {
        return;
    }
    let label = if state.is_completed() {
        "\u{258C} replay complete" // ▌
    } else if state.is_playing() {
        "\u{258C} replay" // ▌
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

/// Repositions the floating progress chip above the destination
/// pile of the most-recently-applied move and repaints its text.
///
/// The chip is hidden when:
/// - the cursor is at 0 (no moves applied yet — chip would have
///   nowhere meaningful to land), OR
/// - the most-recently-applied move was a `StockClick` (no
///   destination pile — stock-click feedback already lives at
///   the stock pile and we don't want the chip to jitter back
///   to the stock pile every cycle).
///
/// When visible, the chip's world-space `Transform.translation`
/// is set to the destination pile's centre plus a fixed upward
/// offset (`card_size.y * 0.6`) so the chip floats just above
/// the top edge of the card. World-space placement (rather than
/// UI-space + camera projection) keeps the math trivial and means
/// the chip stays correctly positioned through window resizes
/// without any extra wiring — `LayoutResource` already drives
/// every other piece of pile geometry.
fn update_floating_progress_chip(
    state: Res<ReplayPlaybackState>,
    layout: Option<Res<LayoutResource>>,
    mut chips: Query<
        (&mut Transform, &mut Visibility, &mut Text2d),
        With<ReplayFloatingProgressChip>,
    >,
) {
    let Some(layout) = layout else {
        return;
    };

    // Resolve the destination pile of the last-applied move (if
    // any). `cursor` is the index of the *next* move to apply, so
    // the most-recently-applied move sits at `cursor - 1`.
    let dest_pile = match state.as_ref() {
        ReplayPlaybackState::Playing { replay, cursor, .. } if *cursor > 0 => {
            match &replay.moves[cursor - 1] {
                ReplayMove::Move { to, .. } => Some(to.clone()),
                ReplayMove::StockClick => None,
            }
        }
        _ => None,
    };

    let Some(world_pos) = dest_pile
        .as_ref()
        .and_then(|p| layout.0.pile_positions.get(p).copied())
    else {
        // Nothing to point at — hide every chip and exit.
        for (_, mut visibility, _) in chips.iter_mut() {
            *visibility = Visibility::Hidden;
        }
        return;
    };

    // Position above the destination pile by ~60 % of a card
    // height. Half a card lifts above the centre, the extra 10 %
    // is breathing room above the top edge so the chip doesn't
    // visually clip the card.
    let above = Vec2::new(0.0, layout.0.card_size.y * 0.6);
    let target = (world_pos + above).extend(100.0);
    let label = format_progress(&state);

    for (mut transform, mut visibility, mut text2d) in chips.iter_mut() {
        transform.translation = target;
        *visibility = Visibility::Inherited;
        if **text2d != label {
            **text2d = label.clone();
        }
    }
}

/// Repaints the move-log panel's `▌ MOVE LOG · N/M` header text
/// whenever [`ReplayPlaybackState`] changes. Cheap — early-exits
/// when nothing moved so an idle replay leaves the text mesh
/// untouched.
fn update_move_log_header(
    state: Res<ReplayPlaybackState>,
    mut q: Query<&mut Text, With<ReplayOverlayMoveLogHeader>>,
) {
    if !state.is_changed() {
        return;
    }
    let label = format_move_log_header(&state);
    for mut text in &mut q {
        **text = label.clone();
    }
}

/// Repaints the move-log panel's active-row text whenever
/// [`ReplayPlaybackState`] changes. Same change-detection guard
/// as the header updater. Empty string at `cursor == 0` (no move
/// applied yet) and in non-`Playing` states; populated otherwise.
fn update_move_log_active_row(
    state: Res<ReplayPlaybackState>,
    mut q: Query<&mut Text, With<ReplayOverlayMoveLogActiveRow>>,
) {
    if !state.is_changed() {
        return;
    }
    let label = format_active_move_row(&state);
    for mut text in &mut q {
        **text = label.clone();
    }
}

/// Repaints every "previous move" row text whenever
/// [`ReplayPlaybackState`] changes. Each row's `offset` is read
/// from the marker; `k = offset + 1` feeds [`format_kth_recent_row`]
/// (active is k=1, prev offset 1 is k=2, prev offset 2 is k=3).
/// Rows with `offset >= cursor` paint as empty — the panel
/// gracefully under-fills early in a replay without spurious
/// "out-of-range" text.
fn update_move_log_prev_rows(
    state: Res<ReplayPlaybackState>,
    mut q: Query<(&ReplayOverlayMoveLogPrevRow, &mut Text)>,
) {
    if !state.is_changed() {
        return;
    }
    for (row, mut text) in &mut q {
        let label = format_kth_recent_row(&state, row.offset as usize + 1);
        **text = label;
    }
}

/// Repaints every "next move" row text whenever
/// [`ReplayPlaybackState`] changes. Symmetric to the prev-row
/// updater but feeds [`format_kth_next_row`]. Rows where
/// `cursor + offset > moves.len()` paint as empty — the panel
/// gracefully under-fills late in a replay (e.g. final moves)
/// without spurious out-of-range text.
fn update_move_log_next_rows(
    state: Res<ReplayPlaybackState>,
    mut q: Query<(&ReplayOverlayMoveLogNextRow, &mut Text)>,
) {
    if !state.is_changed() {
        return;
    }
    for (row, mut text) in &mut q {
        let label = format_kth_next_row(&state, row.offset as usize);
        **text = label;
    }
}

/// Repaints the bottom-edge accent scrub fill to mirror cursor progress.
/// Same change-detection guard as the text updaters — the overlay
/// already early-exits when nothing moved, so an idle replay leaves the
/// scrub bar's `Node` untouched.
fn update_scrub_fill(
    state: Res<ReplayPlaybackState>,
    mut q: Query<&mut Node, With<ReplayOverlayScrubFill>>,
) {
    if !state.is_changed() {
        return;
    }
    let pct = scrub_pct(&state);
    for mut node in &mut q {
        node.width = Val::Percent(pct);
    }
}

/// Pure helper — formats the `GAME #YYYY-DDD` caption for the given
/// state. Returns `None` for `Inactive` / `Completed` (the replay is
/// consumed when transitioning out of `Playing`, so the identifier
/// isn't recoverable from state in those branches); spawn-time
/// callers fall back to an empty string.
///
/// Year + chrono ordinal (`{year}-{ordinal:03}`) gives a compact
/// monotonically-increasing identifier shaped like `2026-127` — same
/// shape as the mockup's `GAME #2024-127` motif.
fn format_game_caption(state: &ReplayPlaybackState) -> Option<String> {
    match state {
        ReplayPlaybackState::Playing { replay, .. } => Some(format!(
            "GAME #{}-{:03}",
            replay.recorded_at.year(),
            replay.recorded_at.ordinal()
        )),
        ReplayPlaybackState::Inactive | ReplayPlaybackState::Completed => None,
    }
}

/// Pure helper — formats the centre progress readout for the given state.
/// Exposed at module scope so the spawn path and the per-frame update
/// path produce the exact same string.
fn format_progress(state: &ReplayPlaybackState) -> String {
    match state.progress() {
        // `MOVE N/M` (uppercase + slash) reads as a Terminal output
        // line and matches the floating-chip motif in the mockup at
        // `docs/ui-mockups/replay-overlay-mobile.html`.
        Some((cursor, total)) => format!("MOVE {cursor}/{total}"),
        None if state.is_completed() => "REPLAY COMPLETE".to_string(),
        None => String::new(),
    }
}

/// Pure helper — formats a [`PileType`] as a short, lowercase,
/// 1-indexed display string for the move-log row. `Foundation(2)`
/// renders as `"foundation 3"` rather than `"foundation 2"` so
/// players see human-friendly numbers; the underlying enum
/// remains 0-indexed.
///
/// Returns `String` rather than `&'static str` because the
/// `Foundation` / `Tableau` variants need formatting; the static
/// variants (`Stock`, `Waste`) still allocate but the cost is
/// trivial against the per-frame update cadence.
fn format_pile(p: &PileType) -> String {
    match p {
        PileType::Stock => "stock".to_string(),
        PileType::Waste => "waste".to_string(),
        PileType::Foundation(i) => format!("foundation {}", i + 1),
        PileType::Tableau(i) => format!("tableau {}", i + 1),
    }
}

/// Pure helper — formats a [`ReplayMove`] as the body of a
/// move-log row. `StockClick` reads as `"stock cycle"`; `Move`
/// reads as `"{from} → {to}"` using [`format_pile`] for both
/// endpoints. The `count` field is omitted from the row body —
/// at row scale it adds visual noise without meaningful
/// information for the typical 1-card moves.
fn format_move_body(m: &ReplayMove) -> String {
    match m {
        ReplayMove::StockClick => "stock cycle".to_string(),
        ReplayMove::Move { from, to, .. } => {
            format!("{} \u{2192} {}", format_pile(from), format_pile(to))
        }
    }
}

/// Pure helper — formats the move-log panel's header text. Reads
/// `▌ MOVE LOG · N/M` while playing, where `N` is the count of
/// moves applied so far and `M` is the total in the replay. The
/// cursor-block prefix (`▌`) matches the splash and replay-banner
/// motifs. Empty in `Inactive` (no replay attached); reads
/// `▌ MOVE LOG · COMPLETE` in `Completed`.
fn format_move_log_header(state: &ReplayPlaybackState) -> String {
    match state {
        ReplayPlaybackState::Playing { replay, cursor, .. } => {
            format!("\u{258C} MOVE LOG \u{00B7} {}/{}", cursor, replay.moves.len())
        }
        ReplayPlaybackState::Completed => "\u{258C} MOVE LOG \u{00B7} COMPLETE".to_string(),
        ReplayPlaybackState::Inactive => String::new(),
    }
}

/// Pure helper — formats the kth-most-recently-applied move's row
/// text. `k = 1` is the active row (`replay.moves[cursor - 1]`,
/// displayed as `"{cursor} │ {body}"`). `k = 2` is the row above
/// that (`moves[cursor - 2]` displayed as `"{cursor - 1} │ {body}"`),
/// and so on.
///
/// Returns the empty string in any of these cases:
/// - State isn't `Playing` (no replay attached).
/// - `k == 0` (no kth-most-recent for k=0; the active is k=1).
/// - `k > cursor` (not enough history — e.g. cursor=2 has rows
///   for k=1 and k=2 only, k=3 returns empty).
/// - The move list is shorter than expected (defensive guard).
fn format_kth_recent_row(state: &ReplayPlaybackState, k: usize) -> String {
    let ReplayPlaybackState::Playing { replay, cursor, .. } = state else {
        return String::new();
    };
    if k == 0 || k > *cursor {
        return String::new();
    }
    let zero_idx = *cursor - k;
    let Some(m) = replay.moves.get(zero_idx) else {
        return String::new();
    };
    let display_idx = *cursor - k + 1;
    format!("{} \u{2502} {}", display_idx, format_move_body(m))
}

/// Pure helper — formats the kth-NEXT move's row text. `k = 1`
/// is the move that will apply next (`replay.moves[cursor]`,
/// displayed as `cursor + 1`); `k = 2` is the move after that,
/// and so on.
///
/// Returns the empty string in any of these cases:
/// - State isn't `Playing` (no replay attached).
/// - `k == 0` (degenerate; the active is k=1 of *recent*, not
///   *next*).
/// - `cursor + k - 1 >= moves.len()` (not enough remaining
///   replay — late in the move list, the trailing next rows
///   stay empty).
fn format_kth_next_row(state: &ReplayPlaybackState, k: usize) -> String {
    let ReplayPlaybackState::Playing { replay, cursor, .. } = state else {
        return String::new();
    };
    if k == 0 {
        return String::new();
    }
    let zero_idx = *cursor + k - 1;
    let Some(m) = replay.moves.get(zero_idx) else {
        return String::new();
    };
    let display_idx = *cursor + k;
    format!("{} \u{2502} {}", display_idx, format_move_body(m))
}

/// Pure helper — formats the active-row text for the move-log
/// panel. Wraps [`format_kth_recent_row`] with `k=1` and prepends
/// a `▶` focus marker so the active row reads visually distinct
/// from prev rows even before the highlight background lands.
/// Returns empty when there's no row to render (cursor=0 or
/// non-`Playing` state) — never `"▶ "` alone, which would paint
/// a stray prefix.
fn format_active_move_row(state: &ReplayPlaybackState) -> String {
    let body = format_kth_recent_row(state, 1);
    if body.is_empty() {
        return String::new();
    }
    format!("\u{25B6} {body}") // ▶
}

// ---------------------------------------------------------------------------
// Playback-control button handlers
// ---------------------------------------------------------------------------

/// Pure helper — returns the label the Pause / Resume button should
/// carry for the given state. "Pause" while running, "Resume" while
/// paused, empty otherwise (the button is despawned with the rest of
/// the overlay tree on transitions to `Inactive` / `Completed`, so
/// the empty branch only fires for one frame around state changes).
fn pause_button_label(state: &ReplayPlaybackState) -> &'static str {
    match state {
        ReplayPlaybackState::Playing { paused: true, .. } => "Resume",
        ReplayPlaybackState::Playing { paused: false, .. } => "Pause",
        ReplayPlaybackState::Inactive | ReplayPlaybackState::Completed => "",
    }
}

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

/// Watches the Pause / Resume button for `Interaction::Pressed`
/// transitions. On a click, toggles the `paused` flag via
/// [`toggle_pause_replay_playback`]. The label repaint happens in
/// [`update_pause_button_label`] on the same frame the state mutation
/// flushes.
fn handle_pause_button(
    mut state: ResMut<ReplayPlaybackState>,
    buttons: Query<&Interaction, (With<ReplayPauseButton>, Changed<Interaction>)>,
) {
    if !buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    toggle_pause_replay_playback(&mut state);
}

/// Watches the Step button for `Interaction::Pressed` transitions. On
/// a click, advances exactly one move via [`step_replay_playback`].
/// No-op while playback is unpaused (would race the tick loop) — the
/// guard lives inside `step_replay_playback`.
fn handle_step_button(
    mut state: ResMut<ReplayPlaybackState>,
    mut moves_writer: MessageWriter<MoveRequestEvent>,
    mut draws_writer: MessageWriter<DrawRequestEvent>,
    buttons: Query<&Interaction, (With<ReplayStepButton>, Changed<Interaction>)>,
) {
    if !buttons.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    step_replay_playback(&mut state, &mut moves_writer, &mut draws_writer);
}

/// Repaints the Pause / Resume button's label whenever
/// [`ReplayPlaybackState`] changes. Walks from the marked button
/// entity to its single child [`Text`] so the spawn path doesn't need
/// a second marker on the inner node.
fn update_pause_button_label(
    state: Res<ReplayPlaybackState>,
    buttons: Query<&Children, With<ReplayPauseButton>>,
    mut texts: Query<&mut Text>,
) {
    if !state.is_changed() {
        return;
    }
    let label = pause_button_label(&state);
    if label.is_empty() {
        // Overlay is mid-teardown; the button entity will despawn
        // this frame anyway. Skip the repaint to avoid touching a
        // doomed entity.
        return;
    }
    for children in &buttons {
        for child in children.iter() {
            if let Ok(mut text) = texts.get_mut(child) {
                text.0 = label.to_string();
                break;
            }
        }
    }
}

/// Watches `Space` for the keyboard pause / resume accelerator.
/// UI-first contract from CLAUDE.md §3.3 is satisfied by the on-
/// screen Pause / Resume button; this is the optional accelerator.
/// No-op when the playback isn't `Playing` (e.g. while a modal is
/// open and the player is using `Space` for something else).
fn handle_pause_keyboard(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut state: ResMut<ReplayPlaybackState>,
) {
    let Some(keys) = keys else { return };
    if !keys.just_pressed(KeyCode::Space) {
        return;
    }
    toggle_pause_replay_playback(&mut state);
}

/// Watches the arrow keys for the paused step / scrub
/// accelerators. UI-first contract from CLAUDE.md §3.3 is
/// satisfied by the on-screen Step button (forward only); these
/// are the optional accelerators that also surface a backwards
/// step plus continuous scrub.
///
/// Both keys are paused-only — the underlying step helpers
/// hard-gate via destructure on `paused: true`. Pressing → during
/// running playback or ← at cursor 0 are silent no-ops; the
/// player learns "pause first, then arrow."
///
/// **Single press fires once immediately**
/// (`just_pressed`). **Holding** the key triggers continuous
/// scrub at [`SCRUB_REPEAT_INTERVAL_SECS`] cadence (10 steps/sec
/// at 100 ms): the per-key accumulator on
/// [`ReplayScrubKeyHold`] absorbs `time.delta_secs()` each frame
/// the key is held, fires + resets when the threshold is hit, and
/// resets to 0 on key release so the next fresh press fires
/// immediately. This matches the mockup's `[← →] scrub`
/// terminology while keeping single-press = single-step semantics.
fn handle_arrow_keyboard(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    time: Res<Time>,
    mut hold: ResMut<ReplayScrubKeyHold>,
    mut state: ResMut<ReplayPlaybackState>,
    mut moves_writer: MessageWriter<MoveRequestEvent>,
    mut draws_writer: MessageWriter<DrawRequestEvent>,
    mut undo_writer: MessageWriter<UndoRequestEvent>,
) {
    let Some(keys) = keys else { return };
    let dt = time.delta_secs();

    // Right (forward step) — initial press fires immediately;
    // held repeats fire when the accumulator crosses the interval.
    if keys.just_pressed(KeyCode::ArrowRight) {
        step_replay_playback(&mut state, &mut moves_writer, &mut draws_writer);
        hold.right_held_secs = 0.0;
    } else if keys.pressed(KeyCode::ArrowRight) {
        hold.right_held_secs += dt;
        if hold.right_held_secs >= SCRUB_REPEAT_INTERVAL_SECS {
            step_replay_playback(&mut state, &mut moves_writer, &mut draws_writer);
            hold.right_held_secs = 0.0;
        }
    } else {
        hold.right_held_secs = 0.0;
    }

    // Left (backwards step) — symmetric to the right path.
    if keys.just_pressed(KeyCode::ArrowLeft) {
        step_backwards_replay_playback(&mut state, &mut undo_writer);
        hold.left_held_secs = 0.0;
    } else if keys.pressed(KeyCode::ArrowLeft) {
        hold.left_held_secs += dt;
        if hold.left_held_secs >= SCRUB_REPEAT_INTERVAL_SECS {
            step_backwards_replay_playback(&mut state, &mut undo_writer);
            hold.left_held_secs = 0.0;
        }
    } else {
        hold.left_held_secs = 0.0;
    }
}

/// Watches `Esc` for the keyboard stop accelerator. UI-first
/// contract from CLAUDE.md §3.3 is satisfied by the on-screen
/// Stop button; this is the optional accelerator.
///
/// Cross-plugin coordination: `pause_plugin::toggle_pause` also
/// listens for `Esc` and would otherwise open the pause modal on
/// the same press. The conflict is resolved by `toggle_pause`
/// gating itself on `ReplayPlaybackState::is_playing()` —
/// symmetrical to the existing `forfeit_screens` /
/// `other_modal_scrims` defer-if pattern in that system. So during
/// an active replay this handler owns the `Esc` press and the
/// pause modal stays closed.
///
/// No-op when the playback isn't `Playing` (the resource may still
/// exist as `Inactive` or `Completed`; only `Playing` means a
/// replay is on screen for the player to stop).
fn handle_stop_keyboard(
    mut commands: Commands,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut state: ResMut<ReplayPlaybackState>,
) {
    let Some(keys) = keys else { return };
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if !state.is_playing() {
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
    /// the banner label reads "▌ replay".
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
                paused: false,
            },
        );
        app.update();

        assert_eq!(
            overlay_root_count(&mut app),
            1,
            "exactly one ReplayOverlayRoot must spawn on Inactive → Playing",
        );
        assert_eq!(banner_text(&mut app), "\u{258C} replay");
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
                paused: false,
            },
        );
        app.update();

        assert_eq!(progress_text(&mut app), "MOVE 5/10");
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
                paused: false,
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

    /// Lifecycle: the floating progress chip spawns alongside the
    /// banner overlay when playback starts, and despawns when
    /// playback ends. (Position correctness needs `LayoutResource`,
    /// which isn't set up in this headless fixture; the lifecycle
    /// test below is what's load-bearing for the spawn/despawn
    /// pairing.)
    #[test]
    fn floating_chip_spawns_and_despawns_with_overlay() {
        let mut app = headless_app();
        // Inactive → no chip.
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ReplayFloatingProgressChip>()
                .iter(app.world())
                .count(),
            0,
            "no floating chip while playback is Inactive",
        );

        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(5),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ReplayFloatingProgressChip>()
                .iter(app.world())
                .count(),
            1,
            "floating chip must spawn when playback starts",
        );

        set_state(&mut app, ReplayPlaybackState::Inactive);
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ReplayFloatingProgressChip>()
                .iter(app.world())
                .count(),
            0,
            "floating chip must despawn when playback ends",
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
                paused: false,
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
    /// must read "▌ replay complete".
    #[test]
    fn overlay_text_changes_on_completed() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(7),
                cursor: 7,
                secs_to_next: 0.0,
                paused: false,
            },
        );
        app.update();
        assert_eq!(banner_text(&mut app), "\u{258C} replay");

        set_state(&mut app, ReplayPlaybackState::Completed);
        app.update();

        assert_eq!(
            overlay_root_count(&mut app),
            1,
            "overlay must remain spawned while in Completed state",
        );
        assert_eq!(
            banner_text(&mut app),
            "\u{258C} replay complete",
            "banner label must swap on Playing → Completed",
        );
    }

    /// Read the current `Node.width` of the unique scrub-fill entity as
    /// a percentage. Assertions can then compare against expected
    /// `cursor / total` ratios without poking ECS internals at the call
    /// site.
    fn scrub_fill_pct(app: &mut App) -> f32 {
        let mut q = app
            .world_mut()
            .query_filtered::<&Node, With<ReplayOverlayScrubFill>>();
        let node = q
            .iter(app.world())
            .next()
            .expect("scrub-fill node must exist while overlay is spawned");
        match node.width {
            Val::Percent(p) => p,
            other => panic!("scrub fill width must be Val::Percent; got {other:?}"),
        }
    }

    /// Pure-helper guard. Locks in the four corners of `scrub_pct` so a
    /// future refactor of `ReplayPlaybackState::progress()` can't
    /// silently regress the visual cue: `Inactive → 0 %`,
    /// `Playing { cursor: 0, total: N } → 0 %`,
    /// `Playing { cursor: N/2, total: N } → 50 %`,
    /// `Completed → 100 %`.
    #[test]
    fn scrub_pct_covers_state_corners() {
        assert_eq!(scrub_pct(&ReplayPlaybackState::Inactive), 0.0);
        assert_eq!(scrub_pct(&ReplayPlaybackState::Completed), 100.0);
        assert_eq!(
            scrub_pct(&ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            }),
            0.0,
        );
        assert_eq!(
            scrub_pct(&ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 5,
                secs_to_next: 0.5,
                paused: false,
            }),
            50.0,
        );
        assert_eq!(
            scrub_pct(&ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 10,
                secs_to_next: 0.5,
                paused: false,
            }),
            100.0,
        );
    }

    /// Read the current text content of the unique GAME-caption entity.
    fn game_caption_text(app: &mut App) -> String {
        let mut q = app
            .world_mut()
            .query_filtered::<&Text, With<ReplayOverlayGameCaption>>();
        q.iter(app.world())
            .next()
            .map(|t| t.0.clone())
            .unwrap_or_default()
    }

    /// Pure-helper guard. `Inactive` / `Completed` carry no replay
    /// reference so the caption is `None`; `Playing` formats the
    /// recorded-date as `GAME #YYYY-DDD` with a 3-digit zero-padded
    /// ordinal. Locks all three branches so a future refactor can't
    /// silently regress the identifier shape.
    #[test]
    fn format_game_caption_covers_state_corners() {
        assert_eq!(format_game_caption(&ReplayPlaybackState::Inactive), None);
        assert_eq!(format_game_caption(&ReplayPlaybackState::Completed), None);

        // 2026-05-02 is the 122nd day of 2026 (Jan = 31, Feb = 28,
        // Mar = 31, Apr = 30, May 2 = 122). Synthetic_replay always
        // uses this date so the assertion is stable.
        assert_eq!(
            format_game_caption(&ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 5,
                secs_to_next: 0.5,
                paused: false,
            }),
            Some("GAME #2026-122".to_string()),
        );

        // Single-digit ordinal must zero-pad to three digits — pin
        // the format string in case someone simplifies to `{}-{}`.
        let mut early_january = synthetic_replay(10);
        early_january.recorded_at = NaiveDate::from_ymd_opt(2026, 1, 5).expect("valid date");
        assert_eq!(
            format_game_caption(&ReplayPlaybackState::Playing {
                replay: early_january,
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            }),
            Some("GAME #2026-005".to_string()),
        );
    }

    /// End-to-end: spawning the overlay paints the GAME caption with
    /// the active replay's recorded date in `YYYY-DDD` form. Caption
    /// is empty for `Completed` since the replay is consumed.
    #[test]
    fn overlay_game_caption_shows_replay_date() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(game_caption_text(&mut app), "GAME #2026-122");

        // Caption empties out on Playing → Completed because
        // `format_game_caption` returns None and the spawn-path
        // helper falls through to `unwrap_or_default()`.
        // The overlay itself stays spawned in `Completed`.
        set_state(&mut app, ReplayPlaybackState::Completed);
        app.update();
        assert_eq!(
            overlay_root_count(&mut app),
            1,
            "overlay must remain spawned while in Completed state",
        );
    }

    /// End-to-end: the spawn path must paint the scrub fill at the
    /// initial cursor's percentage, and the per-frame `update_scrub_fill`
    /// system must repaint it as the cursor advances. Mirrors the shape
    /// of `overlay_progress_text_reflects_cursor`.
    #[test]
    fn overlay_scrub_fill_tracks_cursor() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(8),
                cursor: 2,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            scrub_fill_pct(&mut app),
            25.0,
            "spawn-time fill must reflect the initial cursor",
        );

        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(8),
                cursor: 6,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            scrub_fill_pct(&mut app),
            75.0,
            "update_scrub_fill must repaint width on cursor advance",
        );

        set_state(&mut app, ReplayPlaybackState::Completed);
        app.update();
        assert_eq!(
            scrub_fill_pct(&mut app),
            100.0,
            "Completed state must read as a fully-filled track",
        );
    }

    // -----------------------------------------------------------------------
    // win_move_marker_pct + ReplayOverlayWinMoveMarker spawn behaviour
    // -----------------------------------------------------------------------

    fn win_marker_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&ReplayOverlayWinMoveMarker>()
            .iter(app.world())
            .count()
    }

    #[test]
    fn win_move_marker_pct_is_none_for_inactive() {
        assert_eq!(win_move_marker_pct(&ReplayPlaybackState::Inactive), None);
    }

    #[test]
    fn win_move_marker_pct_is_none_for_completed() {
        // `Completed` carries no replay so the marker has no data to
        // anchor against — the overlay treats this as "no marker".
        assert_eq!(win_move_marker_pct(&ReplayPlaybackState::Completed), None);
    }

    #[test]
    fn win_move_marker_pct_is_none_when_replay_lacks_field() {
        // Synthetic replay constructor leaves win_move_index as None
        // (legacy / pre-`ab857bb` path).
        let state = ReplayPlaybackState::Playing {
            replay: synthetic_replay(10),
            cursor: 0,
            secs_to_next: 0.5,
            paused: false,
        };
        assert_eq!(win_move_marker_pct(&state), None);
    }

    #[test]
    fn win_move_marker_pct_is_some_at_correct_position() {
        // 10 moves, win at index 9 → marker sits at 90 % of the track.
        // Matches the recording semantic: cursor reaches the marker
        // exactly when the about-to-apply move IS the win move.
        let state = ReplayPlaybackState::Playing {
            replay: synthetic_replay(10).with_win_move_index(Some(9)),
            cursor: 0,
            secs_to_next: 0.5,
            paused: false,
        };
        assert_eq!(win_move_marker_pct(&state), Some(90.0));
    }

    #[test]
    fn win_move_marker_pct_clamps_to_track_bounds() {
        // Defensive: if a malformed replay carried `win_move_index >=
        // total`, the marker must still sit on the track, not past it.
        let state = ReplayPlaybackState::Playing {
            replay: synthetic_replay(5).with_win_move_index(Some(99)),
            cursor: 0,
            secs_to_next: 0.5,
            paused: false,
        };
        assert_eq!(win_move_marker_pct(&state), Some(100.0));
    }

    #[test]
    fn marker_spawned_when_replay_has_win_move_index() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(8).with_win_move_index(Some(7)),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            win_marker_count(&mut app),
            1,
            "marker entity must spawn when replay carries Some(win_move_index)"
        );
    }

    #[test]
    fn marker_not_spawned_when_replay_lacks_win_move_index() {
        let mut app = headless_app();
        // Default constructor → win_move_index: None (legacy replay).
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(8),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            win_marker_count(&mut app),
            0,
            "no marker should spawn for a replay pre-dating the field"
        );
    }

    #[test]
    fn marker_despawns_when_replay_state_returns_to_inactive() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(8).with_win_move_index(Some(7)),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(win_marker_count(&mut app), 1);

        set_state(&mut app, ReplayPlaybackState::Inactive);
        app.update();
        assert_eq!(
            win_marker_count(&mut app),
            0,
            "marker must despawn with the rest of the overlay tree"
        );
    }

    /// The WIN MOVE marker carries `HighContrastBackground::with_hc(
    /// STATE_SUCCESS, STATE_SUCCESS_HC)` so the lime bumps to brighter
    /// lime under HC mode rather than to a neutral gray. Pin the
    /// presence of the marker so a future refactor can't accidentally
    /// drop it and silently regress HC legibility.
    #[test]
    fn win_move_marker_carries_hc_background_marker() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(8).with_win_move_index(Some(7)),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();

        let mut q = app
            .world_mut()
            .query_filtered::<&HighContrastBackground, With<ReplayOverlayWinMoveMarker>>();
        let marker = q
            .iter(app.world())
            .next()
            .expect("WIN MOVE marker must carry HighContrastBackground");
        assert_eq!(
            marker.default_color,
            STATE_SUCCESS,
            "default colour must be STATE_SUCCESS"
        );
        assert_eq!(
            marker.hc_color,
            STATE_SUCCESS_HC,
            "HC colour must be STATE_SUCCESS_HC (brighter lime, not gray)"
        );
    }

    // -----------------------------------------------------------------------
    // scrub_notch_positions + ReplayOverlayScrubNotch spawn behaviour
    // -----------------------------------------------------------------------

    fn scrub_notch_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&ReplayOverlayScrubNotch>()
            .iter(app.world())
            .count()
    }

    /// Pure-helper guard. Locks in the five-notch ladder at the
    /// quarter-marks. A future simplification to fewer notches (or a
    /// shift to non-quarter spacing) must touch this test, surfacing
    /// the visual change at review time.
    #[test]
    fn scrub_notch_positions_are_quarter_marks() {
        assert_eq!(
            scrub_notch_positions(),
            [0.0, 25.0, 50.0, 75.0, 100.0],
            "scrub notches must sit at the five quarter-mark percentages",
        );
    }

    /// Five notch entities spawn alongside the rest of the overlay
    /// tree on `Inactive → Playing`. Cardinality matches
    /// `scrub_notch_positions().len()`.
    #[test]
    fn scrub_notches_spawn_with_overlay() {
        let mut app = headless_app();
        app.update();
        assert_eq!(scrub_notch_count(&mut app), 0);

        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            scrub_notch_count(&mut app),
            scrub_notch_positions().len(),
            "exactly one notch entity per quarter-mark must spawn",
        );
    }

    /// Each spawned notch carries `HighContrastBackground` so the
    /// existing `update_high_contrast_backgrounds` system bumps
    /// `BORDER_SUBTLE` → `BORDER_SUBTLE_HC` under HC mode.
    /// Five-of-five — every notch tagged.
    #[test]
    fn scrub_notches_carry_high_contrast_background_marker() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();

        let count = app
            .world_mut()
            .query_filtered::<&HighContrastBackground, With<ReplayOverlayScrubNotch>>()
            .iter(app.world())
            .count();
        assert_eq!(
            count,
            scrub_notch_positions().len(),
            "every notch must carry HighContrastBackground for HC repaint coverage",
        );
    }

    /// The 1 px scrub track also carries `HighContrastBackground` so
    /// the unfilled portion bumps under HC. The fill (ACCENT_PRIMARY,
    /// brick-red) doesn't need a marker — accent colours are
    /// already saturated and don't need an HC variant.
    #[test]
    fn scrub_track_carries_high_contrast_background_marker() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();

        // Track is the parent Node of the scrub-fill. Find it by
        // walking up from `ReplayOverlayScrubFill` to its parent.
        let world = app.world_mut();
        let mut fill_q = world.query_filtered::<Entity, With<ReplayOverlayScrubFill>>();
        let fill = fill_q
            .iter(world)
            .next()
            .expect("scrub fill must exist while overlay is spawned");
        let mut parent_q = world.query::<&ChildOf>();
        let parent = parent_q
            .get(world, fill)
            .map(|p| p.parent())
            .expect("scrub fill must have a parent (the track)");
        let mut hc_q = world.query::<&HighContrastBackground>();
        assert!(
            hc_q.get(world, parent).is_ok(),
            "scrub track Node (parent of scrub fill) must carry HighContrastBackground",
        );
    }

    /// Notches share the overlay tree's lifecycle — they despawn on
    /// `Playing → Inactive` along with the banner root.
    #[test]
    fn scrub_notches_despawn_with_overlay() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(scrub_notch_count(&mut app), 5);

        set_state(&mut app, ReplayPlaybackState::Inactive);
        app.update();
        assert_eq!(
            scrub_notch_count(&mut app),
            0,
            "notches must despawn with the rest of the overlay tree",
        );
    }

    fn scrub_notch_label_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&ReplayOverlayScrubNotchLabel>()
            .iter(app.world())
            .count()
    }

    /// Returns the rendered text of every `ReplayOverlayScrubNotchLabel`
    /// in left-to-right order — the iteration order isn't guaranteed by
    /// the ECS query, so callers needing a stable order must sort.
    fn scrub_notch_label_texts(app: &mut App) -> Vec<String> {
        let world = app.world_mut();
        let mut q = world.query_filtered::<&Text, With<ReplayOverlayScrubNotchLabel>>();
        q.iter(world).map(|t| t.0.clone()).collect()
    }

    /// Pure-helper guard for the label strings. Pairs with
    /// `scrub_notch_positions_are_quarter_marks` — same length, same
    /// order, so `labels[i]` belongs at `positions[i]`.
    #[test]
    fn scrub_notch_labels_are_quarter_mark_percents() {
        assert_eq!(
            scrub_notch_labels(),
            ["0%", "25%", "50%", "75%", "100%"],
            "scrub notch labels must read as the five quarter-mark percentages",
        );
        assert_eq!(
            scrub_notch_labels().len(),
            scrub_notch_positions().len(),
            "labels and positions must remain paired one-to-one",
        );
    }

    /// Five label entities spawn alongside the rest of the overlay.
    /// Cardinality matches `scrub_notch_labels().len()`.
    #[test]
    fn scrub_notch_labels_spawn_with_overlay() {
        let mut app = headless_app();
        app.update();
        assert_eq!(scrub_notch_label_count(&mut app), 0);

        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            scrub_notch_label_count(&mut app),
            scrub_notch_labels().len(),
            "exactly one label entity per notch must spawn",
        );
    }

    /// Each spawned label carries one of the helper's strings — pins
    /// the spawn-path against drift between the helper and the actual
    /// painted text.
    #[test]
    fn scrub_notch_labels_carry_helper_strings() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();

        let mut texts = scrub_notch_label_texts(&mut app);
        texts.sort();
        let mut expected: Vec<String> = scrub_notch_labels()
            .iter()
            .map(|s| s.to_string())
            .collect();
        expected.sort();
        assert_eq!(
            texts, expected,
            "spawned label texts must equal the helper's strings (set equality, ECS order is not guaranteed)",
        );
    }

    /// Labels share the overlay tree's lifecycle — they despawn on
    /// `Playing → Inactive` along with the banner root.
    #[test]
    fn scrub_notch_labels_despawn_with_overlay() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(scrub_notch_label_count(&mut app), 5);

        set_state(&mut app, ReplayPlaybackState::Inactive);
        app.update();
        assert_eq!(
            scrub_notch_label_count(&mut app),
            0,
            "labels must despawn with the rest of the overlay tree",
        );
    }

    fn keybind_footer_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&ReplayOverlayKeybindFooter>()
            .iter(app.world())
            .count()
    }

    /// Returns every `Text` rendered as a descendant of the
    /// keybind-footer row. Used to assert the mode + hint texts
    /// appear inside the footer without requiring per-text markers.
    fn keybind_footer_text_set(app: &mut App) -> Vec<String> {
        let world = app.world_mut();
        // Find the footer entity, then walk its descendants for `Text`.
        let mut footer_q = world.query_filtered::<Entity, With<ReplayOverlayKeybindFooter>>();
        let Some(footer) = footer_q.iter(world).next() else {
            return Vec::new();
        };
        let mut child_q = world.query::<&Children>();
        let Ok(children) = child_q.get(world, footer) else {
            return Vec::new();
        };
        let child_entities: Vec<Entity> = children.iter().collect();
        let mut text_q = world.query::<&Text>();
        child_entities
            .into_iter()
            .filter_map(|e| text_q.get(world, e).ok().map(|t| t.0.clone()))
            .collect()
    }

    /// Pure-helper guards for the static text strings. Pin both
    /// helpers so a future refactor that reformats the mode line
    /// or extends the hint with un-wired keybinds fails at the
    /// helper test rather than at visual review.
    #[test]
    fn keybind_footer_helpers_carry_expected_text() {
        assert_eq!(
            keybind_footer_mode_text(),
            "\u{258C} NORMAL \u{2502} replay",
            "mode line must read as the cursor-block + NORMAL + bar + replay motif",
        );
        assert_eq!(
            keybind_footer_hint_text(),
            "[SPACE] pause/resume \u{00B7} [ESC] stop \u{00B7} [\u{2190}\u{2192}] step",
            "hint text must list all three wired keybind groups (Space → pause/resume, Esc → stop, ←→ → step) separated by middle dots",
        );
    }

    /// Footer entity spawns alongside the rest of the overlay tree
    /// on `Inactive → Playing`. Cardinality is exactly one — the
    /// footer is a singleton row, not a per-keybind multiple.
    #[test]
    fn keybind_footer_spawns_with_overlay() {
        let mut app = headless_app();
        app.update();
        assert_eq!(keybind_footer_count(&mut app), 0);

        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            keybind_footer_count(&mut app),
            1,
            "exactly one keybind-footer row must spawn with the overlay",
        );
    }

    /// Spawned footer carries both helper strings as direct-child
    /// `Text` content — pins the spawn-path against drift between
    /// the helpers and the actual painted text.
    #[test]
    fn keybind_footer_paints_helper_strings() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();

        let texts = keybind_footer_text_set(&mut app);
        assert!(
            texts.contains(&keybind_footer_mode_text().to_string()),
            "footer must contain the mode-line text; got {texts:?}",
        );
        assert!(
            texts.contains(&keybind_footer_hint_text().to_string()),
            "footer must contain the keybind-hint text; got {texts:?}",
        );
    }

    /// Spawned footer carries `HighContrastBorder` so the existing
    /// `apply_high_contrast_borders` system bumps the 1 px top
    /// border under HC mode. Without this the footer reads as
    /// floating loose under HC.
    #[test]
    fn keybind_footer_carries_high_contrast_border_marker() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();

        let mut q = app
            .world_mut()
            .query_filtered::<&HighContrastBorder, With<ReplayOverlayKeybindFooter>>();
        let marker = q.iter(app.world()).next();
        assert!(
            marker.is_some(),
            "footer must carry HighContrastBorder so `apply_high_contrast_borders` picks it up under HC mode",
        );
    }

    /// Footer shares the overlay tree's lifecycle — it despawns on
    /// `Playing → Inactive` along with the banner root.
    #[test]
    fn keybind_footer_despawns_with_overlay() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(keybind_footer_count(&mut app), 1);

        set_state(&mut app, ReplayPlaybackState::Inactive);
        app.update();
        assert_eq!(
            keybind_footer_count(&mut app),
            0,
            "footer must despawn with the rest of the overlay tree",
        );
    }

    /// Notches are independent of `win_move_index` — a replay with no
    /// win marker still gets the full five-notch ladder (notches give
    /// quarter-mark anchor points; the win marker is an additional
    /// overlay on top of them, not a replacement).
    #[test]
    fn scrub_notches_spawn_even_without_win_marker() {
        let mut app = headless_app();
        // Default constructor → win_move_index: None.
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(8),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            scrub_notch_count(&mut app),
            5,
            "notches and win marker are independent — no marker doesn't drop the notches",
        );
    }

    // -----------------------------------------------------------------------
    // Move Log panel: helpers + spawn cardinality + lifecycle
    // -----------------------------------------------------------------------

    fn move_log_panel_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&ReplayOverlayMoveLogPanel>()
            .iter(app.world())
            .count()
    }

    fn move_log_header_text(app: &mut App) -> String {
        let mut q = app
            .world_mut()
            .query_filtered::<&Text, With<ReplayOverlayMoveLogHeader>>();
        q.iter(app.world())
            .next()
            .map(|t| t.0.clone())
            .unwrap_or_default()
    }

    fn move_log_active_row_text(app: &mut App) -> String {
        let mut q = app
            .world_mut()
            .query_filtered::<&Text, With<ReplayOverlayMoveLogActiveRow>>();
        q.iter(app.world())
            .next()
            .map(|t| t.0.clone())
            .unwrap_or_default()
    }

    /// Pile formatter pins the "lowercase + 1-indexed" contract.
    /// `Foundation(2)` displays as `"foundation 3"` rather than
    /// the underlying 0-index — players see human-friendly numbers.
    #[test]
    fn format_pile_uses_one_indexed_lowercase_names() {
        use solitaire_core::pile::PileType;
        assert_eq!(format_pile(&PileType::Stock), "stock");
        assert_eq!(format_pile(&PileType::Waste), "waste");
        assert_eq!(format_pile(&PileType::Foundation(0)), "foundation 1");
        assert_eq!(format_pile(&PileType::Foundation(2)), "foundation 3");
        assert_eq!(format_pile(&PileType::Tableau(0)), "tableau 1");
        assert_eq!(format_pile(&PileType::Tableau(6)), "tableau 7");
    }

    /// Move-body formatter renders `StockClick` as a label and
    /// `Move` as a `from → to` arrow. The `count` field is
    /// deliberately omitted — at row scale it adds noise.
    #[test]
    fn format_move_body_handles_both_variants() {
        use solitaire_core::pile::PileType;
        use solitaire_data::ReplayMove;
        assert_eq!(format_move_body(&ReplayMove::StockClick), "stock cycle");
        assert_eq!(
            format_move_body(&ReplayMove::Move {
                from: PileType::Waste,
                to: PileType::Tableau(4),
                count: 1,
            }),
            "waste \u{2192} tableau 5",
            "Move variant must render as `{{from}} → {{to}}` with 1-indexed pile numbers",
        );
    }

    /// Header text covers all three state branches:
    /// `Playing` → `▌ MOVE LOG · N/M`,
    /// `Completed` → `▌ MOVE LOG · COMPLETE`,
    /// `Inactive` → empty.
    #[test]
    fn format_move_log_header_covers_state_branches() {
        let playing = ReplayPlaybackState::Playing {
            replay: synthetic_replay(10),
            cursor: 3,
            secs_to_next: 0.5,
            paused: false,
        };
        assert_eq!(format_move_log_header(&playing), "\u{258C} MOVE LOG \u{00B7} 3/10");
        assert_eq!(
            format_move_log_header(&ReplayPlaybackState::Completed),
            "\u{258C} MOVE LOG \u{00B7} COMPLETE",
        );
        assert_eq!(format_move_log_header(&ReplayPlaybackState::Inactive), "");
    }

    /// Active-row text is empty at cursor 0 (no move applied yet)
    /// and populated otherwise. The displayed index is 1-based —
    /// when cursor=N, the most-recently-applied move is at
    /// `replay.moves[N - 1]` and the row reads `"N | ..."`.
    #[test]
    fn format_active_move_row_handles_cursor_zero_and_positive() {
        let cursor_zero = ReplayPlaybackState::Playing {
            replay: synthetic_replay(10),
            cursor: 0,
            secs_to_next: 0.5,
            paused: false,
        };
        assert_eq!(
            format_active_move_row(&cursor_zero),
            "",
            "cursor=0 means no move applied yet; row stays empty",
        );

        let cursor_three = ReplayPlaybackState::Playing {
            replay: synthetic_replay(10),
            cursor: 3,
            secs_to_next: 0.5,
            paused: false,
        };
        // synthetic_replay produces all StockClicks, so the body
        // is "stock cycle". The displayed index is 3 (cursor),
        // matching the most-recently-applied move at moves[2].
        // Active row carries the `▶` focus prefix; prev rows
        // (kth-recent for k>1) don't.
        assert_eq!(
            format_active_move_row(&cursor_three),
            "\u{25B6} 3 \u{2502} stock cycle",
            "active row must read `▶ cursor │ {{move body}}` with the 1-based displayed index",
        );
    }

    /// Move-log panel spawns alongside the rest of the overlay
    /// tree on `Inactive → Playing`. Cardinality is exactly one
    /// (singleton bottom-edge panel).
    #[test]
    fn move_log_panel_spawns_with_overlay() {
        let mut app = headless_app();
        app.update();
        assert_eq!(move_log_panel_count(&mut app), 0);

        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            move_log_panel_count(&mut app),
            1,
            "exactly one move-log panel must spawn with the overlay",
        );
    }

    /// Spawned panel's header reads `▌ MOVE LOG · N/M` matching
    /// the helper output for the active state. Pins the spawn-path
    /// against drift between the helper and the actual painted
    /// text.
    #[test]
    fn move_log_panel_header_paints_helper_string() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(8),
                cursor: 2,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            move_log_header_text(&mut app),
            "\u{258C} MOVE LOG \u{00B7} 2/8",
        );
    }

    /// Active-row text repaints when the cursor advances. Drives
    /// the resource through cursor=0 → cursor=2 transitions and
    /// asserts the row text follows.
    #[test]
    fn move_log_active_row_repaints_on_cursor_advance() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            move_log_active_row_text(&mut app),
            "",
            "cursor=0 must paint an empty row",
        );

        // Advance cursor to 2 (most-recently-applied move is moves[1]).
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 2,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            move_log_active_row_text(&mut app),
            "\u{25B6} 2 \u{2502} stock cycle",
            "active row must repaint to the cursor's position when state changes (with ▶ prefix)",
        );
    }

    /// `format_kth_recent_row` covers the active-row helper for
    /// `k=1` and the prev-row helpers for `k>1`. Pins the "k larger
    /// than cursor returns empty" branch so under-filled panels
    /// early in a replay don't paint stale text.
    #[test]
    fn format_kth_recent_row_handles_in_range_and_out_of_range() {
        let state_at_three = ReplayPlaybackState::Playing {
            replay: synthetic_replay(10),
            cursor: 3,
            secs_to_next: 0.5,
            paused: false,
        };
        // k=1 → active (most recent applied). cursor=3 → display=3.
        assert_eq!(
            format_kth_recent_row(&state_at_three, 1),
            "3 \u{2502} stock cycle",
        );
        // k=2 → row above active. display=2.
        assert_eq!(
            format_kth_recent_row(&state_at_three, 2),
            "2 \u{2502} stock cycle",
        );
        // k=3 → second-prev row. display=1.
        assert_eq!(
            format_kth_recent_row(&state_at_three, 3),
            "1 \u{2502} stock cycle",
        );
        // k=4 — exceeds cursor, no history that far back.
        assert_eq!(
            format_kth_recent_row(&state_at_three, 4),
            "",
            "k > cursor must return empty (panel under-fills gracefully)",
        );
        // k=0 — degenerate, no kth-most-recent for k=0.
        assert_eq!(format_kth_recent_row(&state_at_three, 0), "");
    }

    fn move_log_prev_row_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&ReplayOverlayMoveLogPrevRow>()
            .iter(app.world())
            .count()
    }

    fn move_log_prev_row_text_at_offset(app: &mut App, offset: u8) -> String {
        let world = app.world_mut();
        let mut q = world.query::<(&ReplayOverlayMoveLogPrevRow, &Text)>();
        for (row, text) in q.iter(world) {
            if row.offset == offset {
                return text.0.clone();
            }
        }
        String::new()
    }

    /// `MOVE_LOG_PREV_ROWS` prev rows spawn with the panel — one
    /// per offset 1..=N. Cardinality matches the constant.
    #[test]
    fn move_log_prev_rows_spawn_with_panel() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 3,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            move_log_prev_row_count(&mut app),
            MOVE_LOG_PREV_ROWS,
            "exactly MOVE_LOG_PREV_ROWS prev rows must spawn with the panel",
        );
    }

    /// Each prev row's text at spawn time matches the helper
    /// output for its offset. Pins the spawn path against drift
    /// between marker offset and rendered text.
    #[test]
    fn move_log_prev_rows_paint_helper_strings_at_spawn() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 5,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();

        // offset 1 → k=2 → display=4
        assert_eq!(
            move_log_prev_row_text_at_offset(&mut app, 1),
            "4 \u{2502} stock cycle",
        );
        // offset 2 → k=3 → display=3
        assert_eq!(
            move_log_prev_row_text_at_offset(&mut app, 2),
            "3 \u{2502} stock cycle",
        );
    }

    /// Prev rows repaint as the cursor advances. Drives the
    /// resource through cursor=2 → cursor=5 and asserts the texts
    /// follow.
    #[test]
    fn move_log_prev_rows_repaint_on_cursor_advance() {
        let mut app = headless_app();
        // Start at cursor=2: offset 1 → k=2 → display=1, offset 2 → k=3 → empty (k > cursor).
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 2,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            move_log_prev_row_text_at_offset(&mut app, 1),
            "1 \u{2502} stock cycle",
        );
        assert_eq!(
            move_log_prev_row_text_at_offset(&mut app, 2),
            "",
            "offset 2 (k=3) must be empty when cursor=2 (no history that far back)",
        );

        // Advance to cursor=5 — both offsets now have history.
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 5,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            move_log_prev_row_text_at_offset(&mut app, 1),
            "4 \u{2502} stock cycle",
            "offset 1 must repaint to k=2 of new cursor (display=4)",
        );
        assert_eq!(
            move_log_prev_row_text_at_offset(&mut app, 2),
            "3 \u{2502} stock cycle",
        );
    }

    fn move_log_next_row_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&ReplayOverlayMoveLogNextRow>()
            .iter(app.world())
            .count()
    }

    fn move_log_next_row_text_at_offset(app: &mut App, offset: u8) -> String {
        let world = app.world_mut();
        let mut q = world.query::<(&ReplayOverlayMoveLogNextRow, &Text)>();
        for (row, text) in q.iter(world) {
            if row.offset == offset {
                return text.0.clone();
            }
        }
        String::new()
    }

    /// `format_kth_next_row` covers the about-to-apply preview
    /// for `k=1` (the very next move) and beyond. Pins the
    /// "k=0 returns empty" + "out-of-range returns empty" cases
    /// alongside in-range correctness.
    #[test]
    fn format_kth_next_row_handles_in_range_and_out_of_range() {
        let state_at_three = ReplayPlaybackState::Playing {
            replay: synthetic_replay(10),
            cursor: 3,
            secs_to_next: 0.5,
            paused: false,
        };
        // k=1 → moves[3], display=4
        assert_eq!(
            format_kth_next_row(&state_at_three, 1),
            "4 \u{2502} stock cycle",
        );
        // k=2 → moves[4], display=5
        assert_eq!(
            format_kth_next_row(&state_at_three, 2),
            "5 \u{2502} stock cycle",
        );
        // k=8 — moves[10], out of range for a 10-move replay.
        assert_eq!(
            format_kth_next_row(&state_at_three, 8),
            "",
            "k beyond moves.len() must return empty (panel under-fills late in replay)",
        );
        // k=0 — degenerate.
        assert_eq!(format_kth_next_row(&state_at_three, 0), "");
    }

    /// `MOVE_LOG_NEXT_ROWS` next rows spawn with the panel —
    /// one per offset 1..=N. Cardinality matches the constant.
    #[test]
    fn move_log_next_rows_spawn_with_panel() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 3,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            move_log_next_row_count(&mut app),
            MOVE_LOG_NEXT_ROWS,
            "exactly MOVE_LOG_NEXT_ROWS next rows must spawn with the panel",
        );
    }

    /// Each next row's text at spawn time matches the helper
    /// output for its offset.
    #[test]
    fn move_log_next_rows_paint_helper_strings_at_spawn() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 5,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();

        // offset 1 → moves[5], display=6
        assert_eq!(
            move_log_next_row_text_at_offset(&mut app, 1),
            "6 \u{2502} stock cycle",
        );
        // offset 2 → moves[6], display=7
        assert_eq!(
            move_log_next_row_text_at_offset(&mut app, 2),
            "7 \u{2502} stock cycle",
        );
    }

    /// Next rows under-fill late in the replay. With a 10-move
    /// replay at cursor=9: offset 1 → moves[9] (display 10),
    /// offset 2 → moves[10] (out of range, empty).
    #[test]
    fn move_log_next_rows_underfill_at_replay_end() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 9,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            move_log_next_row_text_at_offset(&mut app, 1),
            "10 \u{2502} stock cycle",
            "offset 1 (k=1) must populate when cursor < moves.len()",
        );
        assert_eq!(
            move_log_next_row_text_at_offset(&mut app, 2),
            "",
            "offset 2 (k=2) must be empty when cursor + k - 1 >= moves.len()",
        );
    }

    /// Active row sits inside a wrapper Node with
    /// `BackgroundColor(ACCENT_PRIMARY)` so it reads as "current
    /// focus" against the panel background. Validates the wrapper
    /// is present and carries the expected colour.
    #[test]
    fn active_row_wrapper_carries_accent_primary_background() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 3,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();

        // Find the active-row Text entity, then walk to its
        // parent — that's the wrapper Node which should carry
        // the highlight BackgroundColor.
        let world = app.world_mut();
        let mut row_q = world.query_filtered::<Entity, With<ReplayOverlayMoveLogActiveRow>>();
        let row = row_q
            .iter(world)
            .next()
            .expect("active row Text entity must exist while overlay is spawned");
        let mut parent_q = world.query::<&ChildOf>();
        let parent = parent_q
            .get(world, row)
            .map(|p| p.parent())
            .expect("active row must have a parent (the highlight wrapper)");
        let mut bg_q = world.query::<&BackgroundColor>();
        let bg = bg_q
            .get(world, parent)
            .expect("active row's parent must carry BackgroundColor (highlight)");
        assert_eq!(
            bg.0, ACCENT_PRIMARY,
            "active-row wrapper background must be ACCENT_PRIMARY for the focus highlight",
        );
    }

    /// Active-row Text uses TEXT_PRIMARY_HC for legible contrast
    /// against the brick-red ACCENT_PRIMARY background. Without
    /// this the default TEXT_PRIMARY (#d0d0d0) on red would have
    /// borderline contrast; the HC variant (#f5f5f5) keeps the
    /// row readable.
    #[test]
    fn active_row_text_uses_high_contrast_color_for_highlight() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 3,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();

        let world = app.world_mut();
        let mut q = world
            .query_filtered::<&TextColor, With<ReplayOverlayMoveLogActiveRow>>();
        let color = q
            .iter(world)
            .next()
            .expect("active row TextColor must exist");
        assert_eq!(
            color.0, TEXT_PRIMARY_HC,
            "active row text colour must be TEXT_PRIMARY_HC for contrast against the highlight",
        );
    }

    /// Active-row text starts with the `▶` focus marker prefix.
    /// Pure-helper guard — pins the prefix so a future refactor
    /// dropping it has to also update this test.
    #[test]
    fn active_row_format_includes_focus_prefix() {
        let state = ReplayPlaybackState::Playing {
            replay: synthetic_replay(10),
            cursor: 5,
            secs_to_next: 0.5,
            paused: false,
        };
        let row = format_active_move_row(&state);
        assert!(
            row.starts_with('\u{25B6}'),
            "active-row format must start with ▶ focus marker; got {row:?}",
        );
        // Cursor=0 still returns empty, never just the prefix.
        let cursor_zero = ReplayPlaybackState::Playing {
            replay: synthetic_replay(10),
            cursor: 0,
            secs_to_next: 0.5,
            paused: false,
        };
        assert_eq!(
            format_active_move_row(&cursor_zero),
            "",
            "cursor=0 must return empty (no stray prefix on empty row)",
        );
    }

    /// Panel shares the overlay tree's lifecycle — it despawns on
    /// `Playing → Inactive` along with the banner root.
    #[test]
    fn move_log_panel_despawns_with_overlay() {
        let mut app = headless_app();
        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(10),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(move_log_panel_count(&mut app), 1);

        set_state(&mut app, ReplayPlaybackState::Inactive);
        app.update();
        assert_eq!(
            move_log_panel_count(&mut app),
            0,
            "panel must despawn with the rest of the overlay tree",
        );
    }

    // -----------------------------------------------------------------------
    // pause_button_label + pause / step click handlers + keyboard accelerator
    // -----------------------------------------------------------------------

    /// Read the current text content of the unique pause / resume button.
    fn pause_button_text(app: &mut App) -> String {
        let world = app.world_mut();
        let mut button_q = world.query_filtered::<&Children, With<ReplayPauseButton>>();
        let children: Vec<Entity> = button_q
            .iter(world)
            .next()
            .map(|c| c.iter().collect())
            .unwrap_or_default();
        let mut text_q = world.query::<&Text>();
        for child in children {
            if let Ok(text) = text_q.get(world, child) {
                return text.0.clone();
            }
        }
        String::new()
    }

    /// Find the unique entity carrying the given button marker.
    fn unique_button<M: Component>(app: &mut App) -> Entity {
        let world = app.world_mut();
        let mut q = world.query_filtered::<Entity, With<M>>();
        q.iter(world).next().expect("button entity must exist")
    }

    fn pressed_paused_state(replay_len: usize, cursor: usize) -> ReplayPlaybackState {
        ReplayPlaybackState::Playing {
            replay: synthetic_replay(replay_len),
            cursor,
            secs_to_next: 0.5,
            paused: true,
        }
    }

    fn running_state(replay_len: usize, cursor: usize) -> ReplayPlaybackState {
        ReplayPlaybackState::Playing {
            replay: synthetic_replay(replay_len),
            cursor,
            secs_to_next: 0.5,
            paused: false,
        }
    }

    #[test]
    fn pause_button_label_reads_pause_when_running() {
        assert_eq!(pause_button_label(&running_state(5, 0)), "Pause");
    }

    #[test]
    fn pause_button_label_reads_resume_when_paused() {
        assert_eq!(pause_button_label(&pressed_paused_state(5, 0)), "Resume");
    }

    #[test]
    fn pause_button_label_is_empty_off_state() {
        assert_eq!(pause_button_label(&ReplayPlaybackState::Inactive), "");
        assert_eq!(pause_button_label(&ReplayPlaybackState::Completed), "");
    }

    #[test]
    fn pause_button_text_swaps_when_state_pauses() {
        let mut app = headless_app();
        set_state(&mut app, running_state(5, 0));
        app.update();
        assert_eq!(pause_button_text(&mut app), "Pause");

        set_state(&mut app, pressed_paused_state(5, 0));
        app.update();
        assert_eq!(
            pause_button_text(&mut app),
            "Resume",
            "label must repaint to Resume on the frame the state pauses"
        );
    }

    #[test]
    fn pause_button_click_toggles_paused_flag() {
        let mut app = headless_app();
        set_state(&mut app, running_state(5, 0));
        app.update();

        let button = unique_button::<ReplayPauseButton>(&mut app);
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();

        match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { paused, .. } => {
                assert!(*paused, "click must flip running → paused");
            }
            other => panic!("expected Playing, got {other:?}"),
        }
    }

    #[test]
    fn step_button_click_advances_cursor_while_paused() {
        let mut app = headless_app();
        set_state(&mut app, pressed_paused_state(5, 0));
        app.update();

        let button = unique_button::<ReplayStepButton>(&mut app);
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();

        match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { cursor, paused, .. } => {
                assert_eq!(*cursor, 1, "step must advance the cursor by exactly one");
                assert!(*paused, "step must leave the paused flag untouched");
            }
            other => panic!("expected Playing, got {other:?}"),
        }
    }

    #[test]
    fn step_button_click_is_noop_while_running() {
        let mut app = headless_app();
        set_state(&mut app, running_state(5, 0));
        app.update();

        let button = unique_button::<ReplayStepButton>(&mut app);
        app.world_mut()
            .entity_mut(button)
            .insert(Interaction::Pressed);
        app.update();

        match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { cursor, paused, .. } => {
                assert_eq!(*cursor, 0, "running-step must not race the tick loop");
                assert!(!*paused);
            }
            other => panic!("expected Playing, got {other:?}"),
        }
    }

    /// Pressing Esc while a replay is playing resets the state to
    /// `Inactive` (same end-state as clicking the Stop button).
    /// Mirrors `space_keyboard_toggles_paused_flag` for the stop
    /// accelerator.
    #[test]
    fn esc_keyboard_stops_active_replay() {
        let mut app = headless_app();
        // The keyboard handler reads `Option<Res<ButtonInput<KeyCode>>>`
        // and no-ops when missing — provide it for this test.
        app.init_resource::<ButtonInput<KeyCode>>();
        set_state(&mut app, running_state(5, 0));
        app.update();
        assert_eq!(overlay_root_count(&mut app), 1);

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();

        assert!(
            matches!(
                app.world().resource::<ReplayPlaybackState>(),
                ReplayPlaybackState::Inactive
            ),
            "Esc must reset state to Inactive while replay is Playing",
        );

        // One more tick — `react_to_state_change` despawns the overlay
        // in response to the state going Inactive.
        app.update();
        assert_eq!(
            overlay_root_count(&mut app),
            0,
            "overlay must despawn the frame after Esc stops the replay",
        );
    }

    /// Esc is a no-op when the replay isn't `Playing` — covers
    /// `Inactive` (no replay attached) and `Completed` (auto-clear
    /// underway). The handler must stay quiet so the global Esc
    /// listeners (pause modal, etc.) own those frames.
    #[test]
    fn esc_keyboard_is_noop_when_not_playing() {
        let mut app = headless_app();
        app.init_resource::<ButtonInput<KeyCode>>();
        // Resource defaults to Inactive — no replay attached.
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();

        // State stays Inactive — no spurious mutation.
        assert!(matches!(
            app.world().resource::<ReplayPlaybackState>(),
            ReplayPlaybackState::Inactive
        ));
    }

    /// The keybind-footer hint text now lists both wired
    /// accelerators (Space + Esc). Lock the format so a future edit
    /// that drops one or the other has to also update this test.
    #[test]
    fn keybind_footer_hint_lists_space_and_esc() {
        let hint = keybind_footer_hint_text();
        assert!(
            hint.contains("[SPACE]"),
            "hint must surface the Space accelerator; got {hint:?}",
        );
        assert!(
            hint.contains("[ESC]"),
            "hint must surface the Esc accelerator; got {hint:?}",
        );
    }

    /// Hint must also list the arrow-key step accelerators.
    /// Pinned separately from the Space + Esc test so a future
    /// regression that drops only the arrows is caught here even
    /// if the Space + Esc check still passes.
    #[test]
    fn keybind_footer_hint_lists_arrow_steps() {
        let hint = keybind_footer_hint_text();
        assert!(
            hint.contains("\u{2190}\u{2192}"),
            "hint must surface the ←→ step accelerators; got {hint:?}",
        );
        assert!(
            hint.contains("step"),
            "hint must label the arrow accelerators as 'step' \
             (matches what's wired — single-move step, not continuous scrub); got {hint:?}",
        );
    }

    /// Pressing → while paused advances the cursor by exactly one
    /// — same end-state as clicking the on-screen Step button.
    #[test]
    fn arrow_right_keyboard_advances_cursor_while_paused() {
        let mut app = headless_app();
        app.init_resource::<ButtonInput<KeyCode>>();
        set_state(&mut app, pressed_paused_state(5, 0));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowRight);
        app.update();

        match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { cursor, paused, .. } => {
                assert_eq!(
                    *cursor, 1,
                    "→ must advance the cursor by exactly one while paused",
                );
                assert!(
                    *paused,
                    "→ must leave the paused flag untouched",
                );
            }
            other => panic!("expected Playing, got {other:?}"),
        }
    }

    /// Pressing → while running is a no-op — the existing
    /// `step_replay_playback` guard prevents racing the tick loop.
    #[test]
    fn arrow_right_keyboard_is_noop_while_running() {
        let mut app = headless_app();
        app.init_resource::<ButtonInput<KeyCode>>();
        set_state(&mut app, running_state(5, 0));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowRight);
        app.update();

        match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { cursor, paused, .. } => {
                assert_eq!(*cursor, 0, "→ must not race the tick loop");
                assert!(!*paused);
            }
            other => panic!("expected Playing, got {other:?}"),
        }
    }

    /// Pressing ← while paused with cursor > 0 decrements the
    /// cursor by exactly one. The corresponding game-state reversal
    /// happens when `handle_undo` reads the dispatched
    /// `UndoRequestEvent` — that's covered in the playback core's
    /// integration test, not here.
    #[test]
    fn arrow_left_keyboard_decrements_cursor_while_paused() {
        let mut app = headless_app();
        app.init_resource::<ButtonInput<KeyCode>>();
        // Start paused at cursor=3 so there's room to step backwards.
        set_state(&mut app, pressed_paused_state(5, 3));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowLeft);
        app.update();

        match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { cursor, paused, .. } => {
                assert_eq!(
                    *cursor, 2,
                    "← must decrement the cursor by exactly one while paused",
                );
                assert!(
                    *paused,
                    "← must leave the paused flag untouched",
                );
            }
            other => panic!("expected Playing, got {other:?}"),
        }
    }

    /// Pressing ← at cursor 0 is a no-op (nothing to rewind past).
    #[test]
    fn arrow_left_keyboard_is_noop_at_cursor_zero() {
        let mut app = headless_app();
        app.init_resource::<ButtonInput<KeyCode>>();
        set_state(&mut app, pressed_paused_state(5, 0));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowLeft);
        app.update();

        match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { cursor, .. } => {
                assert_eq!(*cursor, 0, "← at cursor 0 must be a no-op");
            }
            other => panic!("expected Playing, got {other:?}"),
        }
    }

    /// Holding → for one full repeat interval fires a second step
    /// after the initial just_pressed. Drives `Time::delta_secs`
    /// via `TimeUpdateStrategy::ManualDuration` so the test is
    /// deterministic.
    #[test]
    fn arrow_right_keyboard_repeats_while_held() {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;

        let mut app = headless_app();
        app.init_resource::<ButtonInput<KeyCode>>();
        // Drive each frame as a SCRUB_REPEAT_INTERVAL_SECS step so
        // every update past the just_pressed crosses the threshold.
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(SCRUB_REPEAT_INTERVAL_SECS),
        ));
        // Start paused at cursor 0 so there's room to step forward.
        set_state(&mut app, pressed_paused_state(10, 0));
        app.update();

        // Press the key (just_pressed fires once → cursor 1).
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowRight);
        app.update();
        let cursor_after_press = match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { cursor, .. } => *cursor,
            _ => panic!("expected Playing"),
        };
        assert_eq!(
            cursor_after_press, 1,
            "just_pressed must fire once on the press frame",
        );

        // Hold (no new just_pressed; held → accumulator crosses
        // threshold next frame → second fire).
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::ArrowRight);
        app.update();
        let cursor_after_hold = match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { cursor, .. } => *cursor,
            _ => panic!("expected Playing"),
        };
        assert!(
            cursor_after_hold >= 2,
            "held key must fire at least one repeat after the threshold; got cursor={cursor_after_hold}",
        );
    }

    /// Releasing the key resets the per-key accumulator so the
    /// next fresh press fires immediately rather than at half-
    /// interval. Validates the `else { reset to 0 }` branch.
    #[test]
    fn arrow_keyboard_release_resets_accumulator() {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;

        let mut app = headless_app();
        app.init_resource::<ButtonInput<KeyCode>>();
        // Drive sub-threshold ticks so the accumulator builds but
        // never fires while held.
        let half_interval = SCRUB_REPEAT_INTERVAL_SECS * 0.5;
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(half_interval),
        ));
        set_state(&mut app, pressed_paused_state(10, 5));
        app.update();

        // Hold for a sub-threshold tick (no fire expected: no
        // just_pressed, accumulator at 0.05s < 0.1s threshold).
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowRight);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear_just_pressed(KeyCode::ArrowRight);
        app.update();

        // Release (the else-branch should reset right_held_secs
        // to 0). Then verify by holding for another sub-threshold
        // tick — if the accumulator reset properly, no fire.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::ArrowRight);
        app.update();
        let hold = app.world().resource::<ReplayScrubKeyHold>();
        assert_eq!(
            hold.right_held_secs, 0.0,
            "release must reset the per-key accumulator to 0",
        );
    }

    /// Pressing ← while running is a no-op — same hard-gate
    /// rationale as the forward-step paused-only check.
    #[test]
    fn arrow_left_keyboard_is_noop_while_running() {
        let mut app = headless_app();
        app.init_resource::<ButtonInput<KeyCode>>();
        set_state(&mut app, running_state(5, 3));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ArrowLeft);
        app.update();

        match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { cursor, paused, .. } => {
                assert_eq!(*cursor, 3, "← must not race the tick loop");
                assert!(!*paused);
            }
            other => panic!("expected Playing, got {other:?}"),
        }
    }

    #[test]
    fn space_keyboard_toggles_paused_flag() {
        let mut app = headless_app();
        // The keyboard handler reads `Option<Res<ButtonInput<KeyCode>>>`
        // and no-ops when missing — provide it for this test.
        app.init_resource::<ButtonInput<KeyCode>>();
        set_state(&mut app, running_state(5, 0));
        app.update();

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        app.update();

        match app.world().resource::<ReplayPlaybackState>() {
            ReplayPlaybackState::Playing { paused, .. } => {
                assert!(*paused, "Space must toggle running → paused");
            }
            other => panic!("expected Playing, got {other:?}"),
        }
    }

    /// The tableau dim layer spawns alongside the banner when playback
    /// starts and despawns when the replay ends. Mirrors
    /// `floating_chip_spawns_and_despawns_with_overlay` for the dim layer.
    #[test]
    fn dim_layer_spawns_and_despawns_with_overlay() {
        let mut app = headless_app();

        // Inactive → no dim layer yet.
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ReplayTableauDimLayer>()
                .iter(app.world())
                .count(),
            0,
            "no dim layer while playback is Inactive",
        );

        set_state(
            &mut app,
            ReplayPlaybackState::Playing {
                replay: synthetic_replay(5),
                cursor: 0,
                secs_to_next: 0.5,
                paused: false,
            },
        );
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ReplayTableauDimLayer>()
                .iter(app.world())
                .count(),
            1,
            "dim layer must spawn when playback starts",
        );

        set_state(&mut app, ReplayPlaybackState::Inactive);
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&ReplayTableauDimLayer>()
                .iter(app.world())
                .count(),
            0,
            "dim layer must despawn when playback ends",
        );
    }

    /// The dim layer is a full-screen node (100 % × 100 %) at a lower
    /// z-index than the replay chrome (z = Z_REPLAY_DIM < Z_REPLAY_OVERLAY).
    /// Lock the z-ordering so a future refactor of the z constants can't
    /// silently flip the intended stacking.
    #[test]
    fn dim_layer_z_is_below_replay_chrome() {
        const { assert!(Z_REPLAY_DIM < Z_REPLAY_OVERLAY) }
    }
}
