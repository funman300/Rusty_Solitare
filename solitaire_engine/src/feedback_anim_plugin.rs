//! Card feedback animations: shake on invalid move, settle on valid placement,
//! and animated deal on new game start.
//!
//! # Task #54 — Shake animation on invalid move target
//!
//! When `MoveRejectedEvent` fires, a `ShakeAnim` component is inserted on every
//! card entity that belongs to the destination pile (`MoveRejectedEvent::to`).
//! The component stores the card's original X position and an elapsed counter.
//! Each frame, `tick_shake_anim` displaces `transform.translation.x` with a
//! damped sine wave and removes the component after 0.3 s.
//!
//! # Task #55 — Settle/bounce on valid placement
//!
//! `start_settle_anim` listens for `MoveRequestEvent` and `DrawRequestEvent` so
//! the bounce is **scoped to the cards that just moved**, not every top card on
//! the board. For a move it bounces the top `count` cards of the destination
//! pile; for a draw it bounces the top card of the waste. Undos are skipped so
//! reverting a move doesn't replay the placement feedback. `tick_settle_anim`
//! applies a brief Y-scale compression (`scale.y` 1.0 → 0.92 → 1.0 over 0.15 s)
//! and removes the component when elapsed ≥ 0.15 s.
//!
//! # Task #69 — Animated card deal on new game start
//!
//! When `NewGameRequestEvent` fires (on a fresh game, `move_count == 0`),
//! `start_deal_anim` reads `LayoutResource` and
//! inserts a `CardAnim` on every card entity, sliding each card from the stock
//! pile's position to its current (final) position with a per-card stagger
//! derived from the current `AnimSpeed` setting plus a deterministic ±10 %
//! jitter per card so the deal feels organic instead of mechanical:
//!
//! | `AnimSpeed`   | Base stagger      |
//! |---------------|-------------------|
//! | `Normal`      | 0.04 s (default)  |
//! | `Fast`        | 0.02 s (half)     |
//! | `Instant`     | 0.00 s (no delay) |
//!
//! `deal_stagger_delay` and `deal_stagger_jitter` are pure helpers exposed for
//! unit testing.

use std::collections::hash_map::DefaultHasher;
use std::f32::consts::PI;
use std::hash::{Hash, Hasher};

use bevy::prelude::*;
use solitaire_core::pile::PileType;
use solitaire_data::AnimSpeed;

use crate::animation_plugin::CardAnim;
use crate::card_plugin::CardEntity;
use crate::events::{
    DrawRequestEvent, FoundationCompletedEvent, MoveRejectedEvent, MoveRequestEvent,
    NewGameRequestEvent,
};
use crate::game_plugin::GameMutation;
use crate::layout::LayoutResource;
use crate::pause_plugin::PausedResource;
use crate::resources::GameStateResource;
use crate::settings_plugin::SettingsResource;
use crate::table_plugin::PileMarker;
use crate::ui_theme::{
    FOUNDATION_FLOURISH_PEAK_SCALE, MOTION_FOUNDATION_FLOURISH_SECS, STATE_SUCCESS,
};

// ---------------------------------------------------------------------------
// Shared constants
// ---------------------------------------------------------------------------

/// Duration of the shake animation in seconds.
const SHAKE_SECS: f32 = 0.3;
/// Angular frequency (radians/s) of the shake sine wave.
const SHAKE_OMEGA: f32 = 40.0;
/// Peak displacement of the shake in world units.
const SHAKE_AMPLITUDE: f32 = 6.0;

/// Duration of the settle animation in seconds.
const SETTLE_SECS: f32 = 0.15;
/// Maximum Y-scale compression at the midpoint of the settle animation.
const SETTLE_MIN_SCALE: f32 = 0.92;

/// Per-card stagger delay for the deal animation in seconds.
pub const DEAL_STAGGER_SECS: f32 = 0.04;
/// Duration of each card's slide during the deal animation in seconds.
pub const DEAL_SLIDE_SECS: f32 = 0.25;

// ---------------------------------------------------------------------------
// Task #54 — Shake animation component
// ---------------------------------------------------------------------------

/// Drives a horizontal shake animation.
///
/// Inserted on card entities belonging to the destination pile of a rejected
/// move. Removed automatically when `elapsed >= SHAKE_SECS`.
#[derive(Component, Debug, Clone)]
pub struct ShakeAnim {
    /// Seconds elapsed since the shake began.
    pub elapsed: f32,
    /// The card's original X position (restored when the component is removed).
    pub origin_x: f32,
}

/// Computes the horizontal displacement of the shake animation at the given
/// elapsed time.
///
/// Returns `origin_x + sin(elapsed * SHAKE_OMEGA) * SHAKE_AMPLITUDE *
/// (1.0 - elapsed / SHAKE_SECS)`. At `elapsed == 0.0` the sin term is 0, so
/// the displacement is 0. At `elapsed == SHAKE_SECS` the envelope is 0, so the
/// displacement is also 0.
///
/// This is a pure function exposed for unit testing without Bevy.
pub fn shake_offset(elapsed: f32, origin_x: f32) -> f32 {
    let envelope = 1.0 - (elapsed / SHAKE_SECS).min(1.0);
    origin_x + (elapsed * SHAKE_OMEGA).sin() * SHAKE_AMPLITUDE * envelope
}

// ---------------------------------------------------------------------------
// Task #55 — Settle animation component
// ---------------------------------------------------------------------------

/// Drives a brief Y-scale compression (bounce) animation.
///
/// Inserted on the top card entity of every non-empty pile after a successful
/// move (`StateChangedEvent`). Removed automatically when `elapsed >= SETTLE_SECS`.
#[derive(Component, Debug, Clone, Default)]
pub struct SettleAnim {
    /// Seconds elapsed since the settle animation began.
    pub elapsed: f32,
}

/// Computes the Y scale of the settle animation at the given elapsed time.
///
/// At `elapsed == 0.0` the scale is 1.0 (no compression). At the midpoint
/// (`elapsed == SETTLE_SECS / 2`) the scale reaches its minimum (`SETTLE_MIN_SCALE ≈ 0.92`).
/// At `elapsed == SETTLE_SECS` the scale returns to 1.0.
///
/// This is a pure function exposed for unit testing without Bevy.
pub fn settle_scale(elapsed: f32) -> f32 {
    let t = (elapsed / SETTLE_SECS).min(1.0);
    1.0 - (1.0 - SETTLE_MIN_SCALE) * (t * PI).sin()
}

// ---------------------------------------------------------------------------
// Task #69 — Stagger delay helpers
// ---------------------------------------------------------------------------

/// Returns the per-card stagger delay in seconds for the given `AnimSpeed`.
///
/// | `AnimSpeed`   | Returned value |
/// |---------------|----------------|
/// | `Normal`      | `DEAL_STAGGER_SECS` (0.04 s) |
/// | `Fast`        | `DEAL_STAGGER_SECS / 2` (0.02 s) |
/// | `Instant`     | `0.0` — all cards appear simultaneously |
///
/// This is a pure function exposed for unit testing without Bevy.
pub fn deal_stagger_secs_for_speed(speed: &AnimSpeed) -> f32 {
    match speed {
        AnimSpeed::Normal => DEAL_STAGGER_SECS,
        AnimSpeed::Fast => DEAL_STAGGER_SECS / 2.0,
        AnimSpeed::Instant => 0.0,
    }
}

/// Returns the stagger delay in seconds for card at position `index` during the
/// deal animation, given a per-card stagger interval.
///
/// `delay = index * stagger_secs`
///
/// This is a pure function exposed for unit testing without Bevy.
pub fn deal_stagger_delay(index: usize, stagger_secs: f32) -> f32 {
    index as f32 * stagger_secs
}

/// Returns a deterministic ±10 % jitter factor for `card_id`.
///
/// Hashes `card_id` with `DefaultHasher` and maps the low bits into a value in
/// `0.0..=1.0`, then re-centres into `-0.1..=0.1`. The same card id always
/// produces the same factor so deals are reproducible (important for
/// seed-based testing and replay), while a 52-card deal still feels organic
/// because each card's offset varies.
///
/// Multiply a base stagger interval by `1.0 + deal_stagger_jitter(card_id)` to
/// apply the jitter.
pub fn deal_stagger_jitter(card_id: u32) -> f32 {
    let mut hasher = DefaultHasher::new();
    card_id.hash(&mut hasher);
    let jitter_norm = (hasher.finish() % 1000) as f32 / 1000.0; // 0.0..=1.0
    (jitter_norm - 0.5) * 0.2 // ±0.1 == ±10 %
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers the shake, settle, deal, and foundation-completion flourish
/// animation systems.
pub struct FeedbackAnimPlugin;

impl Plugin for FeedbackAnimPlugin {
    fn build(&self, app: &mut App) {
        // Register the events this plugin consumes so it can run in isolation
        // under `MinimalPlugins` (e.g. unit tests) without depending on other
        // plugins to register them. Double-registration is idempotent in Bevy.
        app.add_message::<MoveRequestEvent>()
            .add_message::<DrawRequestEvent>()
            .add_message::<MoveRejectedEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<FoundationCompletedEvent>()
            .add_systems(
                Update,
                (
                    start_shake_anim.after(GameMutation),
                    tick_shake_anim,
                    start_settle_anim.after(GameMutation),
                    tick_settle_anim,
                    start_deal_anim.after(GameMutation),
                    start_foundation_flourish.after(GameMutation),
                    tick_foundation_flourish,
                ),
            );
    }
}

// ---------------------------------------------------------------------------
// Task #54 — Shake systems
// ---------------------------------------------------------------------------

/// Inserts `ShakeAnim` on all card entities belonging to the destination pile
/// when a `MoveRejectedEvent` fires.
fn start_shake_anim(
    mut events: MessageReader<MoveRejectedEvent>,
    game: Res<GameStateResource>,
    settings: Option<Res<SettingsResource>>,
    card_entities: Query<(Entity, &CardEntity, &Transform)>,
    mut commands: Commands,
) {
    let reduce_motion = settings.as_deref().is_some_and(|s| s.0.reduce_motion_mode);
    for ev in events.read() {
        if reduce_motion {
            continue;
        }
        let dest_pile = &ev.to;
        // Collect the card ids that belong to the destination pile.
        let Some(pile) = game.0.piles.get(dest_pile) else { continue };
        let dest_card_ids: Vec<u32> = pile.cards.iter().map(|c| c.id).collect();

        if dest_card_ids.is_empty() {
            continue;
        }

        for (entity, card_marker, transform) in card_entities.iter() {
            if dest_card_ids.contains(&card_marker.card_id) {
                commands.entity(entity).insert(ShakeAnim {
                    elapsed: 0.0,
                    origin_x: transform.translation.x,
                });
            }
        }
    }
}

/// Advances `ShakeAnim` each frame and removes it once the animation completes.
///
/// Applies `translation.x = shake_offset(elapsed, origin_x)`. When done,
/// restores `translation.x = origin_x` so the card is left at its correct
/// position. Skipped while the game is paused.
fn tick_shake_anim(
    mut commands: Commands,
    time: Res<Time>,
    paused: Option<Res<PausedResource>>,
    mut anims: Query<(Entity, &mut Transform, &mut ShakeAnim)>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    let dt = time.delta_secs();
    for (entity, mut transform, mut anim) in &mut anims {
        anim.elapsed += dt;
        if anim.elapsed >= SHAKE_SECS {
            transform.translation.x = anim.origin_x;
            commands.entity(entity).remove::<ShakeAnim>();
        } else {
            transform.translation.x = shake_offset(anim.elapsed, anim.origin_x);
        }
    }
}

// ---------------------------------------------------------------------------
// Task #55 — Settle systems
// ---------------------------------------------------------------------------

/// Inserts `SettleAnim` only on the cards that just moved — the top `count`
/// cards of the move destination, or the top of the waste pile for a draw.
///
/// Triggered by `MoveRequestEvent` and `DrawRequestEvent`. Undo and other
/// state-mutations are deliberately skipped: replaying the placement bounce on
/// an undo would feel like the rejected-move shake fired by mistake. Note this
/// runs before the move resolves in `GameMutation`, so we read the destination
/// pile **after** the request has been accepted by reading the up-to-date game
/// state for both readers — the schedule labels the system `.after(GameMutation)`
/// to ensure that ordering.
fn start_settle_anim(
    mut moves: MessageReader<MoveRequestEvent>,
    mut draws: MessageReader<DrawRequestEvent>,
    game: Res<GameStateResource>,
    card_entities: Query<(Entity, &CardEntity)>,
    mut commands: Commands,
) {
    // Build the list of card ids that should bounce this frame from every
    // queued request; multiple events can fire in the same frame (e.g. a move
    // followed by a draw via keyboard accelerators).
    let mut bounce_ids: Vec<u32> = Vec::new();

    for ev in moves.read() {
        if let Some(pile) = game.0.piles.get(&ev.to) {
            // The moved cards land on top — take the last `count` ids.
            let n = ev.count.min(pile.cards.len());
            if n > 0 {
                let start = pile.cards.len() - n;
                bounce_ids.extend(pile.cards[start..].iter().map(|c| c.id));
            }
        }
    }

    if draws.read().next().is_some()
        && let Some(pile) = game.0.piles.get(&PileType::Waste)
        && let Some(top) = pile.cards.last()
    {
        bounce_ids.push(top.id);
    }

    if bounce_ids.is_empty() {
        return;
    }

    for (entity, card_marker) in card_entities.iter() {
        if bounce_ids.contains(&card_marker.card_id) {
            commands.entity(entity).insert(SettleAnim::default());
        }
    }
}

/// Advances `SettleAnim` each frame and removes it once the animation completes.
///
/// Applies `transform.scale.y = settle_scale(elapsed)`. Restores scale to 1.0
/// when done. Skipped while the game is paused.
fn tick_settle_anim(
    mut commands: Commands,
    time: Res<Time>,
    paused: Option<Res<PausedResource>>,
    mut anims: Query<(Entity, &mut Transform, &mut SettleAnim)>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    let dt = time.delta_secs();
    for (entity, mut transform, mut anim) in &mut anims {
        anim.elapsed += dt;
        if anim.elapsed >= SETTLE_SECS {
            transform.scale.y = 1.0;
            commands.entity(entity).remove::<SettleAnim>();
        } else {
            transform.scale.y = settle_scale(anim.elapsed);
        }
    }
}

// ---------------------------------------------------------------------------
// Task #69 — Deal animation system
// ---------------------------------------------------------------------------

/// Inserts `CardAnim` on every card entity when a new game starts, sliding
/// each card from the stock pile position to its final position with a
/// per-card stagger derived from the current `AnimSpeed` setting.
///
/// Triggered by `NewGameRequestEvent` (when the new game has `move_count == 0`)
/// and fires the deal animation for every card entity currently in the world.
/// The stagger is looked up from `SettingsResource` via `deal_stagger_secs_for_speed`.
fn start_deal_anim(
    mut events: MessageReader<NewGameRequestEvent>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    settings: Option<Res<SettingsResource>>,
    card_entities: Query<(Entity, &CardEntity, &Transform)>,
    mut commands: Commands,
) {
    if events.read().next().is_none() {
        return;
    }
    // Only animate a fresh deal (no moves made yet).
    if game.0.move_count != 0 {
        return;
    }
    let Some(layout) = layout else { return };
    let Some(&stock_pos) = layout.0.pile_positions.get(&PileType::Stock) else { return };
    let stock_start = Vec3::new(stock_pos.x, stock_pos.y, 0.0);

    let speed = settings.as_ref().map(|s| &s.0.animation_speed);
    let stagger_secs = speed.map_or(DEAL_STAGGER_SECS, deal_stagger_secs_for_speed);

    for (index, (entity, card_marker, transform)) in card_entities.iter().enumerate() {
        let final_pos = transform.translation;
        // ±10 % jitter, deterministic per card id, so the deal feels organic
        // without losing reproducibility (a given seed still produces the
        // same per-card stagger pattern across runs).
        let per_card_stagger = stagger_secs * (1.0 + deal_stagger_jitter(card_marker.card_id));
        commands.entity(entity).insert((
            Transform::from_translation(stock_start.with_z(final_pos.z)),
            CardAnim {
                start: stock_start.with_z(final_pos.z),
                target: final_pos,
                elapsed: 0.0,
                duration: DEAL_SLIDE_SECS,
                delay: deal_stagger_delay(index, per_card_stagger),
            },
        ));
    }
}

// ---------------------------------------------------------------------------
// Foundation-completion flourish
// ---------------------------------------------------------------------------

/// Drives the per-foundation completion flourish on the King card that
/// just landed on a foundation pile (Ace → King, 13 cards).
///
/// Inserted on the King's `CardEntity` when `FoundationCompletedEvent`
/// fires; removed once `elapsed >= duration`. Decorative only — does
/// not block input or interfere with the win cascade, settle, or hint
/// systems (those operate on different markers and read the same
/// `Transform.scale` coordinate non-conflictingly because the flourish
/// finishes in well under a second).
#[derive(Component, Debug, Clone, Copy)]
pub struct FoundationFlourish {
    /// Foundation slot (0..=3) this flourish is celebrating.
    pub foundation_slot: u8,
    /// Seconds elapsed since the flourish began.
    pub elapsed: f32,
    /// Total animation length in seconds.
    pub duration: f32,
}

/// Drives a brief golden tint on the foundation `PileMarker` whose
/// foundation just completed. Stores the marker's original colour so
/// it can be restored when the timer expires.
///
/// Inserted alongside (and concurrent with) `FoundationFlourish` on the
/// matching `PileMarker` entity. The system runs independently of the
/// existing `HintPileHighlight` so the two never share state — a hint
/// landing during a completion flourish (highly unlikely in practice
/// since the foundation just completed) won't corrupt either party's
/// `original_color` snapshot.
#[derive(Component, Debug, Clone, Copy)]
pub struct FoundationMarkerFlourish {
    /// Seconds elapsed since the tint was applied.
    pub elapsed: f32,
    /// Total animation length in seconds.
    pub duration: f32,
    /// The pile marker's sprite colour before the tint was applied —
    /// restored when the timer expires.
    pub original_color: Color,
}

/// Pure helper for unit tests — returns the per-frame scale factor for
/// the foundation flourish at `elapsed_secs` over `duration_secs`.
///
/// Triangular curve, mirroring `score_pulse_scale` in `hud_plugin`:
/// at `t = 0.0` returns `1.0`, at `t = 0.5` returns
/// [`FOUNDATION_FLOURISH_PEAK_SCALE`] (1.15), at `t = 1.0` returns
/// `1.0`. Out-of-range values are clamped so the King never freezes
/// at a non-1.0 scale on the frame after the flourish ends.
///
/// Returns `1.0` whenever `duration_secs <= 0.0` so callers running
/// under `AnimSpeed::Instant` (zeroed durations) skip the flourish
/// without dividing by zero.
pub fn foundation_flourish_scale(elapsed_secs: f32, duration_secs: f32) -> f32 {
    if duration_secs <= 0.0 {
        return 1.0;
    }
    let t = (elapsed_secs / duration_secs).clamp(0.0, 1.0);
    let peak = FOUNDATION_FLOURISH_PEAK_SCALE;
    if t < 0.5 {
        // Climb from 1.0 at t=0 to peak at t=0.5.
        1.0 + (peak - 1.0) * (t / 0.5)
    } else {
        // Descend from peak at t=0.5 back to 1.0 at t=1.0.
        peak - (peak - 1.0) * ((t - 0.5) / 0.5)
    }
}

/// Inserts `FoundationFlourish` on the King card entity at the
/// completed foundation and `FoundationMarkerFlourish` on its
/// `PileMarker`. The King is identified as the *top* card of the
/// foundation pile after the move — by definition the 13th card,
/// always rank King by foundation rules.
fn start_foundation_flourish(
    mut events: MessageReader<FoundationCompletedEvent>,
    game: Res<GameStateResource>,
    settings: Option<Res<SettingsResource>>,
    card_entities: Query<(Entity, &CardEntity)>,
    mut pile_markers: Query<(Entity, &PileMarker, &Sprite, Option<&FoundationMarkerFlourish>)>,
    mut commands: Commands,
) {
    let reduce_motion = settings.as_deref().is_some_and(|s| s.0.reduce_motion_mode);
    for ev in events.read() {
        if reduce_motion {
            continue;
        }
        let pile_type = PileType::Foundation(ev.slot);
        // Top card of the completed foundation is the King.
        let Some(king_id) = game
            .0
            .piles
            .get(&pile_type)
            .and_then(|p| p.cards.last())
            .map(|c| c.id)
        else {
            continue;
        };

        // Tag the King's card entity.
        for (entity, card_marker) in card_entities.iter() {
            if card_marker.card_id == king_id {
                commands.entity(entity).insert(FoundationFlourish {
                    foundation_slot: ev.slot,
                    elapsed: 0.0,
                    duration: MOTION_FOUNDATION_FLOURISH_SECS,
                });
            }
        }

        // Tint the matching PileMarker. Snapshot the current colour so
        // tick_foundation_flourish can restore it; if a stale flourish
        // is somehow still active, reuse its `original_color` so we
        // don't capture the gold tint as the new "original".
        for (entity, pile_marker, sprite, existing) in pile_markers.iter_mut() {
            if pile_marker.0 != pile_type {
                continue;
            }
            let original_color = existing.map_or(sprite.color, |f| f.original_color);
            commands.entity(entity).insert(FoundationMarkerFlourish {
                elapsed: 0.0,
                duration: MOTION_FOUNDATION_FLOURISH_SECS,
                original_color,
            });
        }
    }
}

/// Advances both the King's scale pulse and the foundation marker's
/// gold tint each frame. Removes both components once their timers
/// expire, restoring the King's `Transform.scale` to `Vec3::ONE` and
/// the marker's sprite colour to its captured original.
///
/// Skipped while paused so a player who hits Esc mid-flourish doesn't
/// see frozen scaled state (the next unpause tick resumes from the
/// stored `elapsed`).
#[allow(clippy::type_complexity)]
fn tick_foundation_flourish(
    mut commands: Commands,
    time: Res<Time>,
    paused: Option<Res<PausedResource>>,
    mut card_anims: Query<(Entity, &mut Transform, &mut FoundationFlourish)>,
    mut marker_anims: Query<
        (Entity, &mut Sprite, &mut FoundationMarkerFlourish),
        Without<FoundationFlourish>,
    >,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    let dt = time.delta_secs();

    // Advance the King's scale pulse.
    for (entity, mut transform, mut anim) in &mut card_anims {
        anim.elapsed += dt;
        if anim.elapsed >= anim.duration {
            // Restore identity scale so the card sits at its normal size
            // for the next frame's transform sync.
            transform.scale = Vec3::ONE;
            commands.entity(entity).remove::<FoundationFlourish>();
        } else {
            let s = foundation_flourish_scale(anim.elapsed, anim.duration);
            transform.scale = Vec3::new(s, s, 1.0);
        }
    }

    // Advance the foundation marker's gold tint. Held flat for the
    // first half of the duration and faded back to the original colour
    // over the second half — feels celebratory without bleeding into
    // the next move's drop-target highlights.
    for (entity, mut sprite, mut anim) in &mut marker_anims {
        anim.elapsed += dt;
        if anim.elapsed >= anim.duration {
            sprite.color = anim.original_color;
            commands.entity(entity).remove::<FoundationMarkerFlourish>();
        } else {
            let t = (anim.elapsed / anim.duration).clamp(0.0, 1.0);
            // Lerp factor: 1.0 (full tint) for the first half, then
            // ramps down linearly to 0.0 (original colour) by the end.
            let mix = if t < 0.5 { 1.0 } else { 1.0 - (t - 0.5) / 0.5 };
            sprite.color = lerp_color(anim.original_color, STATE_SUCCESS, mix);
        }
    }
}

/// Linear interpolation between two `Color`s in sRGB space. Pulled out
/// as a small helper so the `tick_foundation_flourish` body stays
/// readable; sRGB-space lerping is fine for a brief decorative tint
/// (a perceptually-uniform space would be overkill).
fn lerp_color(from: Color, to: Color, t: f32) -> Color {
    let from = from.to_srgba();
    let to = to.to_srgba();
    let t = t.clamp(0.0, 1.0);
    Color::srgba(
        from.red + (to.red - from.red) * t,
        from.green + (to.green - from.green) * t,
        from.blue + (to.blue - from.blue) * t,
        from.alpha + (to.alpha - from.alpha) * t,
    )
}

// ---------------------------------------------------------------------------
// Unit tests (pure functions only — no Bevy world required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // Task #54 tests

    #[test]
    fn shake_offset_at_elapsed_zero_returns_origin_x() {
        // sin(0) == 0, so displacement must equal origin_x regardless of
        // SHAKE_AMPLITUDE or envelope.
        let origin_x = 42.0;
        let result = shake_offset(0.0, origin_x);
        assert!(
            (result - origin_x).abs() < 1e-5,
            "shake_offset at elapsed=0 must equal origin_x, got {result}"
        );
    }

    #[test]
    fn shake_offset_at_elapsed_shake_secs_returns_origin_x() {
        // At elapsed == SHAKE_SECS the envelope is 0, so the result must equal
        // origin_x regardless of the sine value.
        let origin_x = 100.0;
        let result = shake_offset(SHAKE_SECS, origin_x);
        assert!(
            (result - origin_x).abs() < 1e-5,
            "shake_offset at elapsed=SHAKE_SECS must equal origin_x (envelope=0), got {result}"
        );
    }

    // Task #55 tests

    #[test]
    fn settle_scale_at_elapsed_zero_is_one() {
        let scale = settle_scale(0.0);
        assert!(
            (scale - 1.0).abs() < 1e-5,
            "settle_scale at elapsed=0 must be 1.0, got {scale}"
        );
    }

    #[test]
    fn settle_scale_at_midpoint_is_approximately_settle_min() {
        // At elapsed == SETTLE_SECS / 2, sin(PI/2) == 1.0, so scale should be
        // at the minimum: 1.0 - (1.0 - SETTLE_MIN_SCALE) = SETTLE_MIN_SCALE.
        let scale = settle_scale(SETTLE_SECS / 2.0);
        assert!(
            (scale - SETTLE_MIN_SCALE).abs() < 1e-4,
            "settle_scale at midpoint must be ~{SETTLE_MIN_SCALE}, got {scale}"
        );
    }

    // Task #69 tests

    #[test]
    fn deal_stagger_delay_zero_index_is_zero() {
        assert_eq!(deal_stagger_delay(0, DEAL_STAGGER_SECS), 0.0);
    }

    #[test]
    fn deal_stagger_delay_returns_index_times_stagger() {
        let stagger = DEAL_STAGGER_SECS;
        for i in 0..52 {
            let expected = i as f32 * stagger;
            let actual = deal_stagger_delay(i, stagger);
            assert!(
                (actual - expected).abs() < 1e-6,
                "deal_stagger_delay({i}, {stagger}) expected {expected}, got {actual}"
            );
        }
    }

    #[test]
    fn deal_stagger_secs_normal_is_constant() {
        assert!((deal_stagger_secs_for_speed(&AnimSpeed::Normal) - DEAL_STAGGER_SECS).abs() < 1e-6);
    }

    #[test]
    fn deal_stagger_secs_fast_is_half_normal() {
        let fast = deal_stagger_secs_for_speed(&AnimSpeed::Fast);
        let normal = deal_stagger_secs_for_speed(&AnimSpeed::Normal);
        assert!(
            (fast - normal / 2.0).abs() < 1e-6,
            "Fast stagger must be half of Normal, got fast={fast} normal={normal}"
        );
    }

    #[test]
    fn deal_stagger_secs_instant_is_zero() {
        assert_eq!(deal_stagger_secs_for_speed(&AnimSpeed::Instant), 0.0);
    }

    #[test]
    fn deal_stagger_delay_instant_is_always_zero() {
        let stagger = deal_stagger_secs_for_speed(&AnimSpeed::Instant);
        for i in 0..52 {
            assert_eq!(
                deal_stagger_delay(i, stagger),
                0.0,
                "Instant speed must produce zero delay for index {i}"
            );
        }
    }

    // Step 9 — deal stagger jitter helper

    #[test]
    fn deal_stagger_jitter_is_within_ten_percent() {
        // Every card id in 0..256 must produce a jitter factor in ±10 %.
        for card_id in 0u32..256 {
            let j = deal_stagger_jitter(card_id);
            assert!(
                (-0.1..=0.1).contains(&j),
                "deal_stagger_jitter({card_id}) = {j} is outside ±10 %"
            );
        }
    }

    #[test]
    fn deal_stagger_jitter_is_deterministic() {
        // Same card id must always produce the same jitter factor.
        for card_id in [0u32, 7, 51, 999_999] {
            assert!(
                (deal_stagger_jitter(card_id) - deal_stagger_jitter(card_id)).abs() < 1e-9,
                "deal_stagger_jitter({card_id}) is not deterministic"
            );
        }
    }

    // Foundation-flourish curve tests

    /// Triangular curve must be 1.0 at t=0, peak at t=0.5, and 1.0 at t=1.
    #[test]
    fn foundation_flourish_scale_curves_through_one_one_one() {
        let dur = MOTION_FOUNDATION_FLOURISH_SECS;
        assert!(
            (foundation_flourish_scale(0.0, dur) - 1.0).abs() < 1e-5,
            "flourish scale at t=0 must be 1.0"
        );
        assert!(
            (foundation_flourish_scale(dur / 2.0, dur) - FOUNDATION_FLOURISH_PEAK_SCALE).abs() < 1e-5,
            "flourish scale at midpoint must be FOUNDATION_FLOURISH_PEAK_SCALE"
        );
        assert!(
            (foundation_flourish_scale(dur, dur) - 1.0).abs() < 1e-5,
            "flourish scale at t=duration must return to 1.0"
        );
    }

    /// Out-of-range values are clamped, not extrapolated. Important so the
    /// King never ends up at a non-1.0 scale on the frame after the
    /// flourish ends (which would race against the despawn / restore step
    /// in `tick_foundation_flourish`).
    #[test]
    fn foundation_flourish_scale_clamps_out_of_range() {
        let dur = MOTION_FOUNDATION_FLOURISH_SECS;
        // Negative elapsed clamps to 0 → scale 1.0.
        assert!((foundation_flourish_scale(-1.0, dur) - 1.0).abs() < 1e-5);
        // Past-end clamps to t=1 → scale 1.0.
        assert!((foundation_flourish_scale(dur * 5.0, dur) - 1.0).abs() < 1e-5);
    }

    /// Zero duration (e.g. `AnimSpeed::Instant`) returns identity, never
    /// divides by zero.
    #[test]
    fn foundation_flourish_scale_zero_duration_is_one() {
        assert!((foundation_flourish_scale(0.0, 0.0) - 1.0).abs() < 1e-5);
        assert!((foundation_flourish_scale(0.5, 0.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn deal_stagger_jitter_varies_across_card_ids() {
        // 52 cards should produce more than a couple distinct jitter factors;
        // a constant function would return one function for all ids.
        use std::collections::HashSet;
        let unique: HashSet<u64> = (0u32..52)
            .map(|id| (deal_stagger_jitter(id) * 1e6) as i64 as u64)
            .collect();
        assert!(
            unique.len() > 10,
            "expected > 10 distinct jitter factors for 52 cards, got {}",
            unique.len()
        );
    }

    // -----------------------------------------------------------------------
    // Reduce-motion gates — ShakeAnim, FoundationFlourish
    // -----------------------------------------------------------------------

    /// `start_shake_anim` must not insert `ShakeAnim` when `reduce_motion_mode`
    /// is on, even when the event targets a pile that has card entities present.
    #[test]
    fn shake_anim_skipped_under_reduce_motion() {
        use bevy::ecs::message::Messages;
        use solitaire_core::game_state::{DrawMode, GameState};
        use solitaire_data::Settings;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(FeedbackAnimPlugin);
        app.insert_resource(GameStateResource(GameState::new(1, DrawMode::DrawOne)));
        app.insert_resource(SettingsResource(Settings {
            reduce_motion_mode: true,
            ..Settings::default()
        }));
        app.update();

        // Pick a card from Tableau(0) so the event refers to a real pile.
        let dest_pile = PileType::Tableau(0);
        let card_id = app
            .world()
            .resource::<GameStateResource>()
            .0
            .piles
            .get(&dest_pile)
            .and_then(|p| p.cards.last())
            .map(|c| c.id)
            .expect("Tableau(0) should have at least one card in a fresh game");

        // Spawn a minimal CardEntity matching that id so the system would
        // find it and insert ShakeAnim if the gate were absent.
        app.world_mut().spawn((
            CardEntity { card_id },
            Transform::default(),
        ));

        app.world_mut()
            .resource_mut::<Messages<MoveRejectedEvent>>()
            .write(MoveRejectedEvent {
                from: PileType::Stock,
                to: dest_pile,
                count: 1,
            });
        app.update();

        let shake_count = app
            .world_mut()
            .query::<&ShakeAnim>()
            .iter(app.world())
            .count();
        assert_eq!(shake_count, 0, "ShakeAnim must not be inserted under reduce-motion");
    }

    /// `start_foundation_flourish` must not insert `FoundationFlourish` when
    /// `reduce_motion_mode` is on.
    #[test]
    fn foundation_flourish_skipped_under_reduce_motion() {
        use bevy::ecs::message::Messages;
        use solitaire_core::game_state::{DrawMode, GameState};
        use solitaire_data::Settings;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(FeedbackAnimPlugin);
        app.insert_resource(GameStateResource(GameState::new(1, DrawMode::DrawOne)));
        app.insert_resource(SettingsResource(Settings {
            reduce_motion_mode: true,
            ..Settings::default()
        }));
        app.update();

        app.world_mut()
            .resource_mut::<Messages<FoundationCompletedEvent>>()
            .write(FoundationCompletedEvent {
                slot: 0,
                suit: solitaire_core::card::Suit::Spades,
            });
        app.update();

        let flourish_count = app
            .world_mut()
            .query::<&FoundationFlourish>()
            .iter(app.world())
            .count();
        assert_eq!(flourish_count, 0, "FoundationFlourish must not be inserted under reduce-motion");
    }
}
