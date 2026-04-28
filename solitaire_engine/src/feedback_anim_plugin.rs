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
//! After `StateChangedEvent` fires, `start_settle_anim` inserts `SettleAnim`
//! on the top card of every non-empty pile. `tick_settle_anim` applies a brief
//! Y-scale compression (`scale.y` 1.0 → 0.92 → 1.0 over 0.15 s) and removes
//! the component when elapsed ≥ 0.15 s.
//!
//! # Task #69 — Animated card deal on new game start
//!
//! When `NewGameRequestEvent` fires (on a fresh game, `move_count == 0`) or
//! `NewGameConfirmEvent` fires, `start_deal_anim` reads `LayoutResource` and
//! inserts a `CardAnim` on every card entity, sliding each card from the stock
//! pile's position to its current (final) position with a per-card stagger
//! derived from the current `AnimSpeed` setting:
//!
//! | `AnimSpeed`   | Stagger           |
//! |---------------|-------------------|
//! | `Normal`      | 0.04 s (default)  |
//! | `Fast`        | 0.02 s (half)     |
//! | `Instant`     | 0.00 s (no delay) |
//!
//! `deal_stagger_delay` is a pure helper exposed for unit testing.

use std::f32::consts::PI;

use bevy::prelude::*;
use solitaire_core::pile::PileType;
use solitaire_data::AnimSpeed;

use crate::animation_plugin::CardAnim;
use crate::card_plugin::CardEntity;
use crate::events::{MoveRejectedEvent, NewGameRequestEvent, StateChangedEvent};
use crate::game_plugin::GameMutation;
use crate::layout::LayoutResource;
use crate::pause_plugin::PausedResource;
use crate::resources::GameStateResource;
use crate::settings_plugin::SettingsResource;

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

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers the shake, settle, and deal animation systems.
pub struct FeedbackAnimPlugin;

impl Plugin for FeedbackAnimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                start_shake_anim.after(GameMutation),
                tick_shake_anim,
                start_settle_anim.after(GameMutation),
                tick_settle_anim,
                start_deal_anim.after(GameMutation),
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
    mut events: EventReader<MoveRejectedEvent>,
    game: Res<GameStateResource>,
    card_entities: Query<(Entity, &CardEntity, &Transform)>,
    mut commands: Commands,
) {
    for ev in events.read() {
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

/// Inserts `SettleAnim` on the top card of every non-empty pile when
/// `StateChangedEvent` fires.
fn start_settle_anim(
    mut events: EventReader<StateChangedEvent>,
    game: Res<GameStateResource>,
    card_entities: Query<(Entity, &CardEntity)>,
    mut commands: Commands,
) {
    if events.read().next().is_none() {
        return;
    }

    // Collect the id of the top card for each non-empty pile.
    let top_ids: Vec<u32> = game
        .0
        .piles
        .values()
        .filter_map(|p| p.cards.last().map(|c| c.id))
        .collect();

    for (entity, card_marker) in card_entities.iter() {
        if top_ids.contains(&card_marker.card_id) {
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
    mut events: EventReader<NewGameRequestEvent>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    settings: Option<Res<SettingsResource>>,
    card_entities: Query<(Entity, &Transform), With<CardEntity>>,
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
    let stagger_secs = speed
        .map(deal_stagger_secs_for_speed)
        .unwrap_or(DEAL_STAGGER_SECS);

    for (index, (entity, transform)) in card_entities.iter().enumerate() {
        let final_pos = transform.translation;
        commands.entity(entity).insert((
            Transform::from_translation(stock_start.with_z(final_pos.z)),
            CardAnim {
                start: stock_start.with_z(final_pos.z),
                target: final_pos,
                elapsed: 0.0,
                duration: DEAL_SLIDE_SECS,
                delay: deal_stagger_delay(index, stagger_secs),
            },
        ));
    }
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
}
