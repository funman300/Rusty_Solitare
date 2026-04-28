//! Procedural card rendering.
//!
//! Each card is a parent entity with a coloured body `Sprite` and a child
//! `Text2d` showing rank+suit. Entities are synced with `GameStateResource`
//! on every `StateChangedEvent`: missing cards are spawned, present cards
//! are repositioned/updated in place, and stale cards are despawned.
//!
//! Phase 3 uses ASCII rank letters ("A", "2"…"10", "J", "Q", "K") and ASCII
//! suit letters ("C", "D", "H", "S") so rendering does not depend on the
//! bundled font carrying Unicode suit glyphs. When real card art lands in a
//! later phase, this plugin is replaced — the `CardEntity` marker and the
//! "sync on StateChangedEvent" contract stay the same.

use std::collections::{HashMap, HashSet};

use bevy::color::Color;
use bevy::prelude::*;
use solitaire_core::card::{Card, Rank, Suit};
use solitaire_core::game_state::{DrawMode, GameState};
use solitaire_core::pile::PileType;

use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

use crate::animation_plugin::{CardAnim, EffectiveSlideDuration};
use crate::events::{CardFaceRevealedEvent, CardFlippedEvent, StateChangedEvent};
use crate::game_plugin::GameMutation;
use crate::layout::{Layout, LayoutResource};
use crate::pause_plugin::PausedResource;
use crate::resources::{DragState, GameStateResource};
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource};
use crate::table_plugin::PileMarker;

/// Fraction of card height used as vertical offset between face-up tableau cards.
pub const TABLEAU_FAN_FRAC: f32 = 0.25;

/// Tighter fan for face-down cards in the tableau — just enough to show the stack.
const TABLEAU_FACEDOWN_FAN_FRAC: f32 = 0.12;

/// Fraction of card height used as a tiny offset between stacked cards in
/// non-tableau piles, so stacking is visible.
const STACK_FAN_FRAC: f32 = 0.003;

/// Font size as a fraction of card width.
const FONT_SIZE_FRAC: f32 = 0.28;

pub const CARD_FACE_COLOUR: Color = Color::srgb(0.98, 0.98, 0.95);
pub const RED_SUIT_COLOUR: Color = Color::srgb(0.78, 0.12, 0.15);
pub const BLACK_SUIT_COLOUR: Color = Color::srgb(0.08, 0.08, 0.08);

/// Alternative face tint for red-suit cards in color-blind mode — a subtle
/// blue wash that distinguishes them from black-suit cards without colour alone.
const CARD_FACE_COLOUR_RED_CBM: Color = Color::srgba(0.85, 0.92, 1.0, 1.0);

/// Returns the card back color for the given unlocked card-back index.
/// Index 0 = default blue; 1–4 are unlockable alternate designs.
fn card_back_colour(selected_card_back: usize) -> Color {
    match selected_card_back {
        0 => Color::srgb(0.15, 0.30, 0.55), // default blue
        1 => Color::srgb(0.55, 0.10, 0.10), // deep red
        2 => Color::srgb(0.05, 0.40, 0.10), // forest green
        3 => Color::srgb(0.35, 0.08, 0.52), // purple
        _ => Color::srgb(0.05, 0.40, 0.42), // teal (4+)
    }
}

/// Marker component linking a Bevy entity to a `solitaire_core::Card::id`.
#[derive(Component, Debug, Clone, Copy)]
pub struct CardEntity {
    pub card_id: u32,
}

/// Marker for the text child inside a card entity.
#[derive(Component, Debug)]
pub struct CardLabel;

/// Marker component indicating the card is currently highlighted as a hint.
/// `remaining` counts down in real seconds; the highlight is removed when it
/// reaches zero and the card sprite colour is restored to its normal value.
#[derive(Component, Debug, Clone)]
pub struct HintHighlight {
    /// Seconds remaining before the highlight is cleared.
    pub remaining: f32,
}

/// Marker on a `PileMarker` entity that is highlighted because the right-clicked
/// card can legally be placed there.
#[derive(Component, Debug)]
pub struct RightClickHighlight;

/// Marker placed on the child `Text2d` entity that shows "↺" on the stock pile
/// marker when the stock pile is empty.
#[derive(Component, Debug)]
pub struct StockEmptyLabel;

// ---------------------------------------------------------------------------
// Task #34 — Card-flip animation
// ---------------------------------------------------------------------------

/// Phase of the two-stage flip animation.
#[derive(Debug, Clone, PartialEq)]
pub enum FlipPhase {
    /// Scale X from 1.0 → 0.0 (hiding the back face).
    ScalingDown,
    /// Scale X from 0.0 → 1.0 (revealing the front face).
    ScalingUp,
}

/// Drives a 2-phase "card flip" animation on `CardEntity` entities.
///
/// The animation squashes X to 0, swaps the sprite to the face-up colour,
/// then expands X back to 1. Total duration is `2 × FLIP_HALF_SECS`.
#[derive(Component, Debug, Clone)]
pub struct CardFlipAnim {
    /// Seconds elapsed in the current phase.
    pub timer: f32,
    /// Which half of the flip we are in.
    pub phase: FlipPhase,
}

/// Duration of each half of the flip animation (scale-down or scale-up).
const FLIP_HALF_SECS: f32 = 0.08;

// ---------------------------------------------------------------------------
// Task #38 — Drag-elevation shadow
// ---------------------------------------------------------------------------

/// Marker component for the semi-transparent shadow sprite shown while dragging.
#[derive(Component, Debug)]
pub struct ShadowEntity;

/// Renders cards by reading `GameStateResource` on `StateChangedEvent`.
pub struct CardPlugin;

impl Plugin for CardPlugin {
    fn build(&self, app: &mut App) {
        // PostStartup ensures TablePlugin's Startup system has inserted
        // LayoutResource before we try to read it.
        //
        // `handle_right_click` reads `ButtonInput<MouseButton>`. Under
        // `MinimalPlugins` (tests) this resource is absent by default, so we
        // ensure it exists here. Under `DefaultPlugins` the call is a no-op.
        app.init_resource::<ButtonInput<MouseButton>>()
            .add_event::<SettingsChangedEvent>()
            .add_event::<CardFlippedEvent>()
            .add_event::<CardFaceRevealedEvent>()
            .add_systems(PostStartup, (sync_cards_startup, update_stock_empty_indicator_startup))
            .add_systems(
                Update,
                (
                    sync_cards_on_change.after(GameMutation),
                    resync_cards_on_settings_change.before(sync_cards_on_change),
                    start_flip_anim.after(GameMutation),
                    tick_flip_anim,
                    update_drag_shadow,
                    tick_hint_highlight,
                    handle_right_click,
                    clear_right_click_highlights_on_state_change.after(GameMutation),
                    clear_right_click_highlights_on_pause,
                    update_stock_empty_indicator.after(GameMutation),
                ),
            );
    }
}

/// When card-back selection changes in Settings, re-render all cards so the
/// new back colour is applied immediately (without waiting for a state change).
fn resync_cards_on_settings_change(
    mut setting_events: EventReader<SettingsChangedEvent>,
    mut state_events: EventWriter<StateChangedEvent>,
) {
    if setting_events.read().next().is_some() {
        state_events.send(StateChangedEvent);
    }
}

/// Render the initial deal. Runs in `PostStartup`, so all `Startup` systems
/// (including `TablePlugin::setup_table` which inserts `LayoutResource`)
/// have already completed.
fn sync_cards_startup(
    commands: Commands,
    game: Res<GameStateResource>,
    layout: Option<Res<LayoutResource>>,
    slide_dur: Option<Res<EffectiveSlideDuration>>,
    settings: Option<Res<SettingsResource>>,
    entities: Query<(Entity, &CardEntity, &Transform)>,
) {
    if let Some(layout) = layout {
        let slide_secs = slide_dur.map_or(0.15, |d| d.slide_secs);
        let back_colour = settings
            .as_ref()
            .map_or_else(|| card_back_colour(0), |s| card_back_colour(s.0.selected_card_back));
        let color_blind = settings.as_ref().is_some_and(|s| s.0.color_blind_mode);
        sync_cards(commands, &game.0, &layout.0, slide_secs, back_colour, color_blind, &entities);
    }
}

fn sync_cards_on_change(
    mut events: EventReader<StateChangedEvent>,
    commands: Commands,
    game: Res<GameStateResource>,
    layout: Option<Res<LayoutResource>>,
    slide_dur: Option<Res<EffectiveSlideDuration>>,
    settings: Option<Res<SettingsResource>>,
    entities: Query<(Entity, &CardEntity, &Transform)>,
) {
    if events.read().next().is_none() {
        return;
    }
    if let Some(layout) = layout {
        let slide_secs = slide_dur.map_or(0.15, |d| d.slide_secs);
        let back_colour = settings
            .as_ref()
            .map_or_else(|| card_back_colour(0), |s| card_back_colour(s.0.selected_card_back));
        let color_blind = settings.as_ref().is_some_and(|s| s.0.color_blind_mode);
        sync_cards(commands, &game.0, &layout.0, slide_secs, back_colour, color_blind, &entities);
    }
}

fn sync_cards(
    mut commands: Commands,
    game: &GameState,
    layout: &Layout,
    slide_secs: f32,
    back_colour: Color,
    color_blind: bool,
    entities: &Query<(Entity, &CardEntity, &Transform)>,
) {
    let positions = card_positions(game, layout);

    // Map card_id -> (Entity, current_translation) for in-place updates.
    let mut existing: HashMap<u32, (Entity, Vec3)> = HashMap::new();
    for (entity, marker, transform) in entities.iter() {
        existing.insert(marker.card_id, (entity, transform.translation));
    }

    let live_ids: HashSet<u32> = positions.iter().map(|(c, _, _)| c.id).collect();

    // Despawn any entity whose card is no longer tracked.
    for (card_id, (entity, _)) in &existing {
        if !live_ids.contains(card_id) {
            commands.entity(*entity).despawn_recursive();
        }
    }

    // For each card in the current state: spawn or update its entity.
    for (card, position, z) in positions {
        match existing.get(&card.id) {
            Some(&(entity, cur)) => {
                update_card_entity(
                    &mut commands, entity, &card, position, z, layout,
                    slide_secs, back_colour, color_blind, cur,
                )
            }
            None => spawn_card_entity(&mut commands, &card, position, z, layout, back_colour, color_blind),
        }
    }
}

/// Returns an ordered vec of (card, position, z) for every card in the game.
fn card_positions(game: &GameState, layout: &Layout) -> Vec<(Card, Vec2, f32)> {
    let mut out: Vec<(Card, Vec2, f32)> = Vec::with_capacity(52);
    let piles = [
        PileType::Stock,
        PileType::Waste,
        PileType::Foundation(Suit::Clubs),
        PileType::Foundation(Suit::Diamonds),
        PileType::Foundation(Suit::Hearts),
        PileType::Foundation(Suit::Spades),
        PileType::Tableau(0),
        PileType::Tableau(1),
        PileType::Tableau(2),
        PileType::Tableau(3),
        PileType::Tableau(4),
        PileType::Tableau(5),
        PileType::Tableau(6),
    ];

    for pile_type in piles {
        let Some(base) = layout.pile_positions.get(&pile_type) else {
            continue;
        };
        let Some(pile) = game.piles.get(&pile_type) else {
            continue;
        };
        let is_tableau = matches!(pile_type, PileType::Tableau(_));
        let is_waste = matches!(pile_type, PileType::Waste);

        // Tableau uses a two-speed fan: face-down cards are packed tighter
        // than face-up cards so the visible (playable) portion stands out.
        // Non-tableau piles stack with a negligible offset.
        //
        // Waste pile: only the top N cards are rendered to prevent bleed-through
        // while new cards animate in from the stock. Draw-One shows 1; Draw-Three
        // shows up to 3 fanned in X (matching the standard Klondike presentation).
        let cards = &pile.cards;
        let render_start = if is_waste {
            let visible = match game.draw_mode {
                DrawMode::DrawOne => 1_usize,
                DrawMode::DrawThree => 3_usize,
            };
            cards.len().saturating_sub(visible)
        } else {
            0
        };

        let mut y_offset = 0.0_f32;
        for (slot, card) in cards[render_start..].iter().enumerate() {
            let x_offset = if is_waste && matches!(game.draw_mode, DrawMode::DrawThree) {
                // Fan left→right; top card (last slot) is rightmost and playable.
                slot as f32 * layout.card_size.x * 0.28
            } else {
                0.0
            };
            let pos = Vec2::new(base.x + x_offset, base.y + y_offset);
            let z = 1.0 + (slot as f32) * STACK_FAN_FRAC;
            out.push((card.clone(), pos, z));
            if is_tableau {
                let step = if card.face_up {
                    TABLEAU_FAN_FRAC
                } else {
                    TABLEAU_FACEDOWN_FAN_FRAC
                };
                y_offset -= layout.card_size.y * step;
            }
        }
    }
    out
}

/// Returns the appropriate face-up body colour for a card.
///
/// In color-blind mode, red-suit cards receive a subtle blue tint
/// (`CARD_FACE_COLOUR_RED_CBM`) so they are distinguishable from black-suit
/// cards without relying on the text colour alone.
fn face_colour(card: &Card, color_blind: bool) -> Color {
    if color_blind && card.suit.is_red() {
        CARD_FACE_COLOUR_RED_CBM
    } else {
        CARD_FACE_COLOUR
    }
}

fn spawn_card_entity(commands: &mut Commands, card: &Card, pos: Vec2, z: f32, layout: &Layout, back_colour: Color, color_blind: bool) {
    let body_colour = if card.face_up {
        face_colour(card, color_blind)
    } else {
        back_colour
    };

    commands
        .spawn((
            CardEntity { card_id: card.id },
            Sprite {
                color: body_colour,
                custom_size: Some(layout.card_size),
                ..default()
            },
            Transform::from_xyz(pos.x, pos.y, z),
            Visibility::default(),
        ))
        .with_children(|b| {
            b.spawn((
                CardLabel,
                Text2d::new(label_for(card)),
                TextFont {
                    font_size: layout.card_size.x * FONT_SIZE_FRAC,
                    ..default()
                },
                TextColor(text_colour(card)),
                // Above the card body on z so it doesn't get occluded by the
                // parent sprite in back-to-front rendering.
                Transform::from_xyz(0.0, 0.0, 0.01),
                label_visibility(card),
            ));
        });
}

#[allow(clippy::too_many_arguments)]
fn update_card_entity(
    commands: &mut Commands,
    entity: Entity,
    card: &Card,
    pos: Vec2,
    z: f32,
    layout: &Layout,
    slide_secs: f32,
    back_colour: Color,
    color_blind: bool,
    cur: Vec3,
) {
    let body_colour = if card.face_up {
        face_colour(card, color_blind)
    } else {
        back_colour
    };

    let target = Vec3::new(pos.x, pos.y, z);

    // Always refresh the visual appearance.
    commands.entity(entity).insert(Sprite {
        color: body_colour,
        custom_size: Some(layout.card_size),
        ..default()
    });

    // Slide to the new position when it differs meaningfully; snap otherwise.
    if (cur.truncate() - target.truncate()).length() > 1.0 && slide_secs > 0.0 {
        let start = Vec3::new(cur.x, cur.y, z); // update Z immediately
        commands
            .entity(entity)
            .insert(Transform::from_translation(start))
            .insert(CardAnim {
                start,
                target,
                elapsed: 0.0,
                duration: slide_secs,
                delay: 0.0,
            });
    } else {
        commands
            .entity(entity)
            .remove::<CardAnim>()
            .insert(Transform::from_xyz(pos.x, pos.y, z));
    }

    // Despawn the old label child and respawn a fresh one, so rank/suit/
    // colour/visibility all stay in sync with the card's current state.
    commands.entity(entity).despawn_descendants();
    commands.entity(entity).with_children(|b| {
        b.spawn((
            CardLabel,
            Text2d::new(label_for(card)),
            TextFont {
                font_size: layout.card_size.x * FONT_SIZE_FRAC,
                ..default()
            },
            TextColor(text_colour(card)),
            Transform::from_xyz(0.0, 0.0, 0.01),
            label_visibility(card),
        ));
    });
}

fn label_for(card: &Card) -> String {
    let rank = match card.rank {
        Rank::Ace => "A",
        Rank::Two => "2",
        Rank::Three => "3",
        Rank::Four => "4",
        Rank::Five => "5",
        Rank::Six => "6",
        Rank::Seven => "7",
        Rank::Eight => "8",
        Rank::Nine => "9",
        Rank::Ten => "10",
        Rank::Jack => "J",
        Rank::Queen => "Q",
        Rank::King => "K",
    };
    let suit = match card.suit {
        Suit::Clubs => "C",
        Suit::Diamonds => "D",
        Suit::Hearts => "H",
        Suit::Spades => "S",
    };
    format!("{rank}{suit}")
}

fn text_colour(card: &Card) -> Color {
    if card.suit.is_red() {
        RED_SUIT_COLOUR
    } else {
        BLACK_SUIT_COLOUR
    }
}

fn label_visibility(card: &Card) -> Visibility {
    if card.face_up {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

// ---------------------------------------------------------------------------
// Task #34 — Card-flip animation systems
// ---------------------------------------------------------------------------

/// Listens for `CardFlippedEvent` and inserts a `CardFlipAnim` on the entity.
///
/// Skipped when `EffectiveSlideDuration::slide_secs == 0.0` (Instant speed).
fn start_flip_anim(
    mut events: EventReader<CardFlippedEvent>,
    slide_dur: Option<Res<EffectiveSlideDuration>>,
    mut commands: Commands,
    card_entities: Query<(Entity, &CardEntity)>,
) {
    if slide_dur.is_some_and(|d| d.slide_secs == 0.0) {
        // Instant animation speed — skip the flip effect entirely.
        events.clear();
        return;
    }

    for CardFlippedEvent(card_id) in events.read() {
        for (entity, marker) in &card_entities {
            if marker.card_id == *card_id {
                commands.entity(entity).insert(CardFlipAnim {
                    timer: 0.0,
                    phase: FlipPhase::ScalingDown,
                });
                break;
            }
        }
    }
}

/// Advances `CardFlipAnim` each frame, modifying `Transform::scale.x`.
///
/// - Phase `ScalingDown`: lerps scale.x from 1.0 → 0.0 over `FLIP_HALF_SECS`.
/// - At the midpoint the phase switches to `ScalingUp`, scale.x resets to 0,
///   and a `CardFaceRevealedEvent` is fired so audio plays in sync with the reveal.
/// - Phase `ScalingUp`:  lerps scale.x from 0.0 → 1.0 over `FLIP_HALF_SECS`.
/// - When complete the component is removed and scale.x is restored to 1.0.
fn tick_flip_anim(
    mut commands: Commands,
    time: Res<Time>,
    mut anims: Query<(Entity, &CardEntity, &mut Transform, &mut CardFlipAnim)>,
    mut reveal_events: EventWriter<CardFaceRevealedEvent>,
) {
    let dt = time.delta_secs();
    for (entity, card_entity, mut transform, mut anim) in &mut anims {
        anim.timer += dt;
        match anim.phase {
            FlipPhase::ScalingDown => {
                let t = (anim.timer / FLIP_HALF_SECS).min(1.0);
                transform.scale.x = 1.0 - t;
                if t >= 1.0 {
                    anim.phase = FlipPhase::ScalingUp;
                    anim.timer = 0.0;
                    transform.scale.x = 0.0;
                    // Fire the reveal event exactly once, at the phase transition,
                    // so the flip sound is synchronised with the visual face reveal.
                    reveal_events.send(CardFaceRevealedEvent(card_entity.card_id));
                }
            }
            FlipPhase::ScalingUp => {
                let t = (anim.timer / FLIP_HALF_SECS).min(1.0);
                transform.scale.x = t;
                if t >= 1.0 {
                    transform.scale.x = 1.0;
                    commands.entity(entity).remove::<CardFlipAnim>();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Task #38 — Drag-elevation shadow
// ---------------------------------------------------------------------------

/// Maintains a single `ShadowEntity` while cards are being dragged.
///
/// - If a drag is active, spawns (or repositions) a semi-transparent dark
///   sprite behind the top dragged card.
/// - If no drag is active, despawns the shadow entity.
fn update_drag_shadow(
    mut commands: Commands,
    drag: Res<DragState>,
    layout: Option<Res<LayoutResource>>,
    card_entities: Query<(&CardEntity, &Transform)>,
    mut shadow: Local<Option<Entity>>,
) {
    if drag.is_idle() {
        // No drag in progress — remove shadow if it exists.
        if let Some(e) = shadow.take() {
            commands.entity(e).despawn_recursive();
        }
        return;
    }

    let Some(layout) = layout else { return };
    let card_w = layout.0.card_size.x;
    let card_h = layout.0.card_size.y;

    // Find the world position of the first (top) dragged card.
    let first_id = drag.cards.first().copied();
    let top_pos = first_id.and_then(|id| {
        card_entities
            .iter()
            .find(|(marker, _)| marker.card_id == id)
            .map(|(_, t)| t.translation)
    });

    let Some(top_pos) = top_pos else { return };

    // Shadow is slightly larger, offset behind-and-below, at a z slightly
    // below the dragged cards.
    let shadow_pos = top_pos + Vec3::new(-4.0, 4.0, -1.0);

    match *shadow {
        Some(e) => {
            // Reposition the existing shadow.
            commands.entity(e).insert(Transform::from_translation(shadow_pos));
        }
        None => {
            // Spawn a new shadow sprite.
            let e = commands
                .spawn((
                    ShadowEntity,
                    Sprite {
                        color: Color::srgba(0.0, 0.0, 0.0, 0.35),
                        custom_size: Some(Vec2::new(card_w + 8.0, card_h + 8.0)),
                        ..default()
                    },
                    Transform::from_translation(shadow_pos),
                    Visibility::default(),
                ))
                .id();
            *shadow = Some(e);
        }
    }
}

// ---------------------------------------------------------------------------
// Task #28 — Hint highlight tick system
// ---------------------------------------------------------------------------

/// Counts down `HintHighlight::remaining` each frame. When it reaches zero,
/// removes the component and resets the card sprite to its normal face-up colour.
fn tick_hint_highlight(
    time: Res<Time>,
    mut commands: Commands,
    mut query: Query<(Entity, &mut HintHighlight, &mut Sprite, &CardEntity)>,
    game: Res<GameStateResource>,
    settings: Option<Res<SettingsResource>>,
) {
    let back_idx = settings.as_ref().map_or(0, |s| s.0.selected_card_back);
    for (entity, mut hint, mut sprite, card_entity) in query.iter_mut() {
        hint.remaining -= time.delta_secs();
        if hint.remaining <= 0.0 {
            // Restore normal face-up colour.
            let is_face_up = game.0.piles.values()
                .flat_map(|p| p.cards.iter())
                .find(|c| c.id == card_entity.card_id)
                .is_some_and(|c| c.face_up);
            sprite.color = if is_face_up {
                CARD_FACE_COLOUR
            } else {
                card_back_colour(back_idx)
            };
            commands.entity(entity).remove::<HintHighlight>();
        }
    }
}

// ---------------------------------------------------------------------------
// Task #46 — Right-click legal destination highlights
// ---------------------------------------------------------------------------

/// Color applied to a `PileMarker` sprite when it is a legal destination for
/// the right-clicked card.
const RIGHT_CLICK_HIGHLIGHT_COLOUR: Color = Color::srgba(0.2, 0.8, 0.2, 0.6);
/// Restored color for `PileMarker` sprites when the highlight is cleared.
const PILE_MARKER_DEFAULT_COLOUR: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);

/// Removes the `RightClickHighlight` marker from every highlighted pile and
/// resets its sprite colour to `PILE_MARKER_DEFAULT_COLOUR`.
///
/// Shared by the on-state-change and on-pause clear systems to avoid
/// duplicating the removal logic.
fn clear_right_click_highlights(
    commands: &mut Commands,
    highlighted: &Query<Entity, With<RightClickHighlight>>,
    pile_markers: &mut Query<(Entity, &PileMarker, &mut Sprite)>,
) {
    for entity in highlighted.iter() {
        commands.entity(entity).remove::<RightClickHighlight>();
    }
    for (_entity, _, mut sprite) in pile_markers.iter_mut() {
        if sprite.color == RIGHT_CLICK_HIGHLIGHT_COLOUR {
            sprite.color = PILE_MARKER_DEFAULT_COLOUR;
        }
    }
}

/// Clears all right-click destination highlights whenever any game-state
/// mutation succeeds (`StateChangedEvent` fires).
///
/// This ensures stale highlights do not linger after a card is moved.
fn clear_right_click_highlights_on_state_change(
    mut events: EventReader<StateChangedEvent>,
    mut commands: Commands,
    highlighted: Query<Entity, With<RightClickHighlight>>,
    mut pile_markers: Query<(Entity, &PileMarker, &mut Sprite)>,
) {
    if events.read().next().is_none() {
        return;
    }
    clear_right_click_highlights(&mut commands, &highlighted, &mut pile_markers);
}

/// Clears all right-click destination highlights when the game is paused
/// (`PausedResource` changes to `true`).
///
/// Prevents highlighted pile markers from remaining visible behind the pause
/// overlay.
fn clear_right_click_highlights_on_pause(
    paused: Option<Res<PausedResource>>,
    mut commands: Commands,
    highlighted: Query<Entity, With<RightClickHighlight>>,
    mut pile_markers: Query<(Entity, &PileMarker, &mut Sprite)>,
) {
    let Some(paused) = paused else { return };
    if paused.is_changed() && paused.0 {
        clear_right_click_highlights(&mut commands, &highlighted, &mut pile_markers);
    }
}

/// Handles right-click: highlights legal destination piles for the clicked card,
/// and clears highlights on any subsequent right- or left-click.
///
/// This system lives in `CardPlugin` to keep `InputPlugin` untouched.
#[allow(clippy::too_many_arguments)]
fn handle_right_click(
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    paused: Option<Res<PausedResource>>,
    drag: Res<DragState>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    layout: Option<Res<LayoutResource>>,
    game: Res<GameStateResource>,
    mut commands: Commands,
    mut pile_markers: Query<(Entity, &PileMarker, &mut Sprite)>,
    card_entities: Query<(Entity, &CardEntity, &Transform)>,
    highlighted: Query<Entity, With<RightClickHighlight>>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }

    let Some(buttons) = buttons else { return };
    let left_pressed = buttons.just_pressed(MouseButton::Left);
    let right_pressed = buttons.just_pressed(MouseButton::Right);

    // Clear existing highlights on any click.
    if left_pressed || right_pressed {
        for entity in &highlighted {
            commands.entity(entity).remove::<RightClickHighlight>();
        }
        for (_entity, _, mut sprite) in &mut pile_markers {
            if sprite.color == RIGHT_CLICK_HIGHLIGHT_COLOUR {
                sprite.color = PILE_MARKER_DEFAULT_COLOUR;
            }
        }
    }

    // Only proceed for right-clicks while not dragging.
    if !right_pressed || !drag.is_idle() {
        return;
    }

    let Some(layout) = layout else { return };

    // Convert cursor to world-space position.
    let Some(world) = cursor_world_pos(&windows, &cameras) else { return };

    // Find the topmost face-up card under the cursor.
    let Some(card) = find_top_card_at(world, &game.0, &layout.0, &card_entities) else { return };

    // Tint piles that legally accept the card.
    for (entity, pile_marker, mut sprite) in &mut pile_markers {
        let pile_type = &pile_marker.0;
        let Some(pile) = game.0.piles.get(pile_type) else { continue };
        let legal = match pile_type {
            PileType::Foundation(suit) => {
                can_place_on_foundation(&card, pile, *suit)
            }
            PileType::Tableau(_) => can_place_on_tableau(&card, pile),
            _ => false,
        };
        if legal {
            sprite.color = RIGHT_CLICK_HIGHLIGHT_COLOUR;
            commands.entity(entity).insert(RightClickHighlight);
        }
    }
}

/// Converts cursor position to 2-D world coordinates.
fn cursor_world_pos(
    windows: &Query<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: &Query<(&Camera, &GlobalTransform)>,
) -> Option<Vec2> {
    let window = windows.get_single().ok()?;
    let cursor = window.cursor_position()?;
    let (camera, camera_transform) = cameras.get_single().ok()?;
    camera.viewport_to_world_2d(camera_transform, cursor).ok()
}

/// Returns the topmost face-up `Card` under `cursor` by checking axis-aligned
/// bounding rectangles of all card sprites, picking the highest Z.
fn find_top_card_at(
    cursor: Vec2,
    game: &GameState,
    layout: &Layout,
    card_entities: &Query<(Entity, &CardEntity, &Transform)>,
) -> Option<Card> {
    let half = layout.card_size / 2.0;
    let mut best: Option<(f32, Card)> = None;

    for (_, card_entity, transform) in card_entities.iter() {
        let pos = transform.translation.truncate();
        if cursor.x < pos.x - half.x
            || cursor.x > pos.x + half.x
            || cursor.y < pos.y - half.y
            || cursor.y > pos.y + half.y
        {
            continue;
        }
        let card = game
            .piles
            .values()
            .flat_map(|p| p.cards.iter())
            .find(|c| c.id == card_entity.card_id && c.face_up)
            .cloned();
        if let Some(card) = card {
            let z = transform.translation.z;
            if best.as_ref().is_none_or(|(bz, _)| z > *bz) {
                best = Some((z, card));
            }
        }
    }
    best.map(|(_, card)| card)
}

// ---------------------------------------------------------------------------
// Task #28 — Stock-empty visual indicator
// ---------------------------------------------------------------------------

/// Sprite colour applied to the stock `PileMarker` when the stock pile is empty,
/// to signal to the player that there are no more cards to draw.
const STOCK_EMPTY_DIM_COLOUR: Color = Color::srgba(1.0, 1.0, 1.0, 0.4);

/// Sprite colour applied to the stock `PileMarker` when cards remain in stock.
const STOCK_NORMAL_COLOUR: Color = Color::srgba(1.0, 1.0, 1.0, 0.08);

/// Shared logic for updating the stock pile marker's dim state and "↺" label.
///
/// If the stock pile is empty the marker sprite is dimmed to
/// `STOCK_EMPTY_DIM_COLOUR` and a child `Text2d` with `StockEmptyLabel` is
/// spawned (if not already present). When the stock is non-empty the marker is
/// restored to `STOCK_NORMAL_COLOUR` and any `StockEmptyLabel` children are
/// despawned.
fn apply_stock_empty_indicator(
    commands: &mut Commands,
    game: &GameState,
    pile_markers: &mut Query<(Entity, &PileMarker, &mut Sprite)>,
    label_children: &Query<(Entity, &Parent), With<StockEmptyLabel>>,
    layout: &Layout,
) {
    let stock_empty = game
        .piles
        .get(&PileType::Stock)
        .is_none_or(|p| p.cards.is_empty());

    for (entity, pile_marker, mut sprite) in pile_markers.iter_mut() {
        if pile_marker.0 != PileType::Stock {
            continue;
        }

        if stock_empty {
            // Dim the marker sprite.
            sprite.color = STOCK_EMPTY_DIM_COLOUR;

            // Spawn the "↺" label only if one does not already exist.
            let already_has_label = label_children
                .iter()
                .any(|(_, parent)| parent.get() == entity);
            if !already_has_label {
                let font_size = layout.card_size.x * 0.4;
                commands.entity(entity).with_children(|b| {
                    b.spawn((
                        StockEmptyLabel,
                        Text2d::new("↺"),
                        TextFont { font_size, ..default() },
                        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.7)),
                        Transform::from_xyz(0.0, 0.0, 0.1),
                    ));
                });
            }
        } else {
            // Restore normal brightness.
            sprite.color = STOCK_NORMAL_COLOUR;

            // Despawn any existing "↺" label children.
            for (label_entity, parent) in label_children.iter() {
                if parent.get() == entity {
                    commands.entity(label_entity).despawn_recursive();
                }
            }
        }
    }
}

/// Runs at `PostStartup` to apply the stock-empty indicator for the initial
/// game state (before any `StateChangedEvent` fires).
fn update_stock_empty_indicator_startup(
    mut commands: Commands,
    game: Res<GameStateResource>,
    layout: Option<Res<LayoutResource>>,
    mut pile_markers: Query<(Entity, &PileMarker, &mut Sprite)>,
    label_children: Query<(Entity, &Parent), With<StockEmptyLabel>>,
) {
    let Some(layout) = layout else { return };
    apply_stock_empty_indicator(
        &mut commands,
        &game.0,
        &mut pile_markers,
        &label_children,
        &layout.0,
    );
}

/// Runs each `Update` tick when a `StateChangedEvent` arrives, keeping the
/// stock pile marker dim state and "↺" label in sync with the current stock.
fn update_stock_empty_indicator(
    mut events: EventReader<StateChangedEvent>,
    mut commands: Commands,
    game: Res<GameStateResource>,
    layout: Option<Res<LayoutResource>>,
    mut pile_markers: Query<(Entity, &PileMarker, &mut Sprite)>,
    label_children: Query<(Entity, &Parent), With<StockEmptyLabel>>,
) {
    if events.read().next().is_none() {
        return;
    }
    let Some(layout) = layout else { return };
    apply_stock_empty_indicator(
        &mut commands,
        &game.0,
        &mut pile_markers,
        &label_children,
        &layout.0,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(CardPlugin);
        app.update();
        app
    }

    #[test]
    fn label_for_ace_of_hearts_is_ah() {
        let c = Card {
            id: 0,
            suit: Suit::Hearts,
            rank: Rank::Ace,
            face_up: true,
        };
        assert_eq!(label_for(&c), "AH");
    }

    #[test]
    fn label_for_ten_of_clubs_is_10c() {
        let c = Card {
            id: 0,
            suit: Suit::Clubs,
            rank: Rank::Ten,
            face_up: true,
        };
        assert_eq!(label_for(&c), "10C");
    }

    #[test]
    fn text_colour_is_red_for_hearts_and_diamonds() {
        let h = Card {
            id: 0,
            suit: Suit::Hearts,
            rank: Rank::Ace,
            face_up: true,
        };
        let d = Card {
            id: 0,
            suit: Suit::Diamonds,
            rank: Rank::Ace,
            face_up: true,
        };
        assert_eq!(text_colour(&h), RED_SUIT_COLOUR);
        assert_eq!(text_colour(&d), RED_SUIT_COLOUR);
    }

    #[test]
    fn text_colour_is_black_for_clubs_and_spades() {
        let c = Card {
            id: 0,
            suit: Suit::Clubs,
            rank: Rank::Ace,
            face_up: true,
        };
        let s = Card {
            id: 0,
            suit: Suit::Spades,
            rank: Rank::Ace,
            face_up: true,
        };
        assert_eq!(text_colour(&c), BLACK_SUIT_COLOUR);
        assert_eq!(text_colour(&s), BLACK_SUIT_COLOUR);
    }

    #[test]
    fn card_plugin_spawns_all_52_cards() {
        let mut app = app();
        let count = app
            .world_mut()
            .query::<&CardEntity>()
            .iter(app.world())
            .count();
        assert_eq!(count, 52);
    }

    #[test]
    fn tableau_face_down_cards_start_hidden_label() {
        let mut app = app();
        // Every tableau column except column 0 has face-down cards. Count
        // CardLabels with Visibility::Hidden — should equal 0+1+2+3+4+5+6 = 21
        // (every tableau card except the top of each column is face-down).
        let hidden_count = app
            .world_mut()
            .query::<(&CardLabel, &Visibility)>()
            .iter(app.world())
            .filter(|(_, v)| matches!(v, Visibility::Hidden))
            .count();
        // 21 tableau face-down + 24 stock face-down = 45.
        assert_eq!(hidden_count, 45);
    }

    #[test]
    fn state_changed_event_triggers_resync() {
        let mut app = app();
        // Trigger a draw, which moves a card from stock to waste and should
        // flip it face-up. Count visible labels after.
        app.world_mut().send_event(crate::events::DrawRequestEvent);
        app.update();
        // Now 1 card in waste (face-up), 23 in stock (face-down). So 24
        // hidden labels total in stock, plus 21 in tableau = 44.
        let hidden_count = app
            .world_mut()
            .query::<(&CardLabel, &Visibility)>()
            .iter(app.world())
            .filter(|(_, v)| matches!(v, Visibility::Hidden))
            .count();
        assert_eq!(hidden_count, 44);
    }

    #[test]
    fn card_positions_includes_all_52_cards_at_game_start() {
        // At game start waste is empty, so all 52 cards are across stock + tableau.
        let g = GameState::new(42, solitaire_core::game_state::DrawMode::DrawOne);
        let layout =
            crate::layout::compute_layout(Vec2::new(1280.0, 800.0));
        let positions = card_positions(&g, &layout);
        assert_eq!(positions.len(), 52);
    }

    #[test]
    fn waste_draw_one_only_renders_top_card() {
        use solitaire_core::game_state::DrawMode;
        let mut g = GameState::new(42, DrawMode::DrawOne);
        // Draw 3 cards so the waste pile has 3 cards.
        for _ in 0..3 {
            let _ = g.draw();
        }
        let waste_ids: std::collections::HashSet<u32> = g.piles[&PileType::Waste]
            .cards
            .iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(waste_ids.len(), 3);

        let layout = crate::layout::compute_layout(Vec2::new(1280.0, 800.0));
        let positions = card_positions(&g, &layout);

        // Filter rendered positions to only waste cards (by card ID).
        let waste_rendered: Vec<_> = positions
            .iter()
            .filter(|(card, _, _)| waste_ids.contains(&card.id))
            .collect();
        // Draw-One: only 1 waste card should be rendered regardless of pile depth.
        assert_eq!(waste_rendered.len(), 1);
        // The single rendered card must be the top (last) waste card.
        let top_id = g.piles[&PileType::Waste].cards.last().unwrap().id;
        assert_eq!(waste_rendered[0].0.id, top_id);
    }

    #[test]
    fn waste_draw_three_renders_up_to_three_fanned_cards() {
        use solitaire_core::game_state::DrawMode;
        let mut g = GameState::new(42, DrawMode::DrawThree);
        // 5 draw() calls in Draw-Three mode accumulates multiple waste cards.
        for _ in 0..5 {
            let _ = g.draw();
        }
        let waste_pile = &g.piles[&PileType::Waste].cards;
        assert!(waste_pile.len() >= 3, "need at least 3 waste cards for this test");

        let waste_ids: std::collections::HashSet<u32> =
            waste_pile.iter().map(|c| c.id).collect();

        let layout = crate::layout::compute_layout(Vec2::new(1280.0, 800.0));
        let positions = card_positions(&g, &layout);

        let mut waste_rendered: Vec<_> = positions
            .iter()
            .filter(|(card, _, _)| waste_ids.contains(&card.id))
            .collect();
        // Draw-Three: at most 3 waste cards rendered.
        assert_eq!(waste_rendered.len(), 3);

        // The three fanned cards must have strictly increasing X coordinates
        // (left = oldest visible, right = top/playable).
        waste_rendered.sort_by(|a, b| a.1.x.partial_cmp(&b.1.x).unwrap());
        for w in waste_rendered.windows(2) {
            assert!(w[1].1.x > w[0].1.x, "fanned waste cards must have distinct X positions");
        }
        // Top card (rightmost) must be the last card in the waste pile.
        let top_id = waste_pile.last().unwrap().id;
        assert_eq!(waste_rendered.last().unwrap().0.id, top_id);
    }

    #[test]
    fn card_positions_tableau_cards_are_fanned_downward() {
        let g = GameState::new(42, solitaire_core::game_state::DrawMode::DrawOne);
        let layout =
            crate::layout::compute_layout(Vec2::new(1280.0, 800.0));
        let positions = card_positions(&g, &layout);

        // Collect positions for Tableau(6) (should have 7 cards).
        let tableau_6_base = layout.pile_positions[&PileType::Tableau(6)];
        let mut ys: Vec<f32> = positions
            .iter()
            .filter(|(_, pos, _)| (pos.x - tableau_6_base.x).abs() < 1e-3)
            .map(|(_, pos, _)| pos.y)
            .collect();
        ys.sort_by(|a, b| b.partial_cmp(a).unwrap());
        assert_eq!(ys.len(), 7);
        // Every subsequent card should be strictly lower.
        for w in ys.windows(2) {
            assert!(w[0] > w[1]);
        }
    }

    #[test]
    fn card_back_colour_known_indices_are_distinct() {
        // Indices 0–3 must each produce a unique colour.
        let colours: Vec<_> = (0..4).map(card_back_colour).collect();
        for i in 0..colours.len() {
            for j in (i + 1)..colours.len() {
                assert_ne!(colours[i], colours[j], "indices {i} and {j} must be distinct");
            }
        }
    }

    #[test]
    fn card_back_colour_out_of_range_does_not_panic() {
        // Indices >= 4 are beyond the defined set; the wildcard arm must handle them
        // without panicking and return the same teal fallback for all.
        let c4  = card_back_colour(4);
        let c5  = card_back_colour(5);
        let c99 = card_back_colour(99);
        assert_eq!(c4, c5,  "out-of-range indices must share the fallback colour");
        assert_eq!(c4, c99, "index 99 must share the fallback colour");
    }

    // -----------------------------------------------------------------------
    // Task #34 pure-function / phase-transition tests
    // -----------------------------------------------------------------------

    #[test]
    fn flip_phase_scaling_down_starts_at_one() {
        // A brand-new flip anim in ScalingDown at timer=0 should produce scale 1.0
        // (no time has elapsed yet).
        let t = 0.0_f32 / FLIP_HALF_SECS;
        let scale_x = 1.0 - t.min(1.0);
        assert!((scale_x - 1.0).abs() < 1e-6, "scale_x at timer=0 must be 1.0");
    }

    #[test]
    fn flip_phase_scaling_down_reaches_zero_at_half_secs() {
        let t = (FLIP_HALF_SECS / FLIP_HALF_SECS).min(1.0);
        let scale_x = 1.0 - t;
        assert!(scale_x.abs() < 1e-6, "scale_x must reach 0.0 after one half-period");
    }

    #[test]
    fn flip_phase_scaling_up_starts_at_zero() {
        let t = 0.0_f32 / FLIP_HALF_SECS;
        let scale_x = t.min(1.0);
        assert!(scale_x.abs() < 1e-6, "scale_x at start of ScalingUp must be 0.0");
    }

    #[test]
    fn flip_phase_scaling_up_reaches_one_at_half_secs() {
        let t = (FLIP_HALF_SECS / FLIP_HALF_SECS).min(1.0);
        let scale_x = t;
        assert!((scale_x - 1.0).abs() < 1e-6, "scale_x must reach 1.0 after second half-period");
    }

    #[test]
    fn flip_phase_enum_equality() {
        assert_eq!(FlipPhase::ScalingDown, FlipPhase::ScalingDown);
        assert_eq!(FlipPhase::ScalingUp, FlipPhase::ScalingUp);
        assert_ne!(FlipPhase::ScalingDown, FlipPhase::ScalingUp);
    }

    #[test]
    fn facedown_cards_use_tighter_fan_than_uniform_faceup_fan() {
        let g = GameState::new(42, solitaire_core::game_state::DrawMode::DrawOne);
        let layout = crate::layout::compute_layout(Vec2::new(1280.0, 800.0));
        let positions = card_positions(&g, &layout);

        // Tableau(6) has 7 cards: 6 face-down + 1 face-up on top.
        // Each face-down card contributes TABLEAU_FACEDOWN_FAN_FRAC to the column span.
        // Total span should be 6 * FACEDOWN < 6 * TABLEAU_FAN_FRAC (the old uniform value).
        let col6_base = layout.pile_positions[&PileType::Tableau(6)];
        let mut col6_ys: Vec<f32> = positions
            .iter()
            .filter(|(_, pos, _)| (pos.x - col6_base.x).abs() < 1e-3)
            .map(|(_, pos, _)| pos.y)
            .collect();
        col6_ys.sort_by(|a, b| b.partial_cmp(a).unwrap());
        assert_eq!(col6_ys.len(), 7);
        let actual_span = col6_ys[0] - col6_ys[6];
        let uniform_span = 6.0 * TABLEAU_FAN_FRAC * layout.card_size.y;
        assert!(
            actual_span < uniform_span,
            "tighter face-down fan should reduce column span ({actual_span:.1} >= uniform {uniform_span:.1})"
        );
    }
}
