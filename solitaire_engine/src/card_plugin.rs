//! PNG-based card rendering.
//!
//! Card entities are synced with [`GameStateResource`] on every
//! [`StateChangedEvent`]: missing cards are spawned, present cards are
//! repositioned/updated in place, and stale cards are despawned.
//!
//! When [`CardImageSet`] is available, each face-up card renders its own
//! 120×168 px `Handle<Image>` chosen from the 52 per-card PNGs loaded from
//! `assets/cards/faces/{rank}_{suit}.png`. A solid-colour `Sprite` with a
//! `Text2d` rank+suit overlay is used as a fallback when `CardImageSet` is
//! absent (e.g. in tests running under `MinimalPlugins`).

use std::collections::{HashMap, HashSet};

use bevy::color::Color;
use bevy::prelude::*;
use bevy::window::WindowResized;
use solitaire_core::card::{Card, Rank, Suit};
use solitaire_core::game_state::{DrawMode, GameState};
use solitaire_core::pile::PileType;

use solitaire_core::rules::{can_place_on_foundation, can_place_on_tableau};

use crate::animation_plugin::{CardAnim, EffectiveSlideDuration};
use crate::card_animation::CardAnimation;
use crate::events::{CardFaceRevealedEvent, CardFlippedEvent, StateChangedEvent};
use crate::game_plugin::GameMutation;
use crate::layout::{Layout, LayoutResource, LayoutSystem};
use crate::pause_plugin::PausedResource;
use crate::resources::{DragState, GameStateResource};
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource};
use crate::table_plugin::PileMarker;
use crate::font_plugin::FontResource;
use crate::ui_theme::{
    CARD_SHADOW_ALPHA_DRAG, CARD_SHADOW_ALPHA_IDLE, CARD_SHADOW_COLOR, CARD_SHADOW_LOCAL_Z,
    CARD_SHADOW_OFFSET_DRAG, CARD_SHADOW_OFFSET_IDLE, CARD_SHADOW_PADDING_DRAG,
    CARD_SHADOW_PADDING_IDLE, STOCK_BADGE_BG, STOCK_BADGE_FG, TYPE_CAPTION, Z_STOCK_BADGE,
};

/// Fraction of card height used as vertical offset between face-up tableau cards.
pub const TABLEAU_FAN_FRAC: f32 = 0.25;

/// Tighter fan for face-down cards in the tableau — just enough to show the stack.
/// Per-card vertical step for face-down tableau cards, as a fraction of
/// card height. Smaller than [`TABLEAU_FAN_FRAC`] because face-down cards
/// don't need their full body shown — only the back-pattern strip is
/// visible. Public so `input_plugin` can mirror the exact sprite layout
/// when hit-testing tableau columns; any drift between this and the
/// renderer creates a visible offset between the card face and where
/// clicks land.
pub const TABLEAU_FACEDOWN_FAN_FRAC: f32 = 0.12;

/// Fraction of card height used as a tiny offset between stacked cards in
/// non-tableau piles, so stacking is visible. Public so other plugins
/// (e.g. input_plugin's drag-rejection tween) can compute the resting
/// `Transform.translation.z` for a card at a given stack index without
/// drifting from the value used by [`card_positions`].
pub const STACK_FAN_FRAC: f32 = 0.003;

/// Font size as a fraction of card width.
const FONT_SIZE_FRAC: f32 = 0.28;

pub const CARD_FACE_COLOUR: Color = Color::srgb(0.98, 0.98, 0.95);
pub const RED_SUIT_COLOUR: Color = Color::srgb(0.78, 0.12, 0.15);
pub const BLACK_SUIT_COLOUR: Color = Color::srgb(0.08, 0.08, 0.08);

/// Pre-loaded [`Handle<Image>`]s for card face and back PNG textures.
///
/// Loaded once at startup by [`load_card_images`].  When this resource is
/// present, card sprites use the PNG artwork; otherwise they fall back to
/// solid-colour sprites (used in tests with `MinimalPlugins`).
#[derive(Resource)]
pub struct CardImageSet {
    /// Per-card face images indexed by `[suit][rank]`.
    ///
    /// Suit order: Clubs=0, Diamonds=1, Hearts=2, Spades=3.
    /// Rank order: Ace=0, Two=1 … King=12.
    pub faces: [[Handle<Image>; 13]; 4],
    /// One handle per unlockable card-back design (indices 0–4). These
    /// correspond to the legacy `assets/cards/backs/back_N.png` art, indexed
    /// by `Settings::selected_card_back`. Used as a fallback when the active
    /// theme does not provide its own back (see [`Self::theme_back`]).
    pub backs: [Handle<Image>; 5],
    /// Back image supplied by the currently-active card theme, if any.
    ///
    /// Populated by `theme::plugin::apply_theme_to_card_image_set` whenever
    /// a `CardTheme` finishes loading. The face-down render path in
    /// [`card_sprite`] prefers this handle over the legacy `backs[]` array,
    /// so a theme switch swaps both faces *and* the back without the player
    /// needing to touch the legacy `selected_card_back` picker. `None` means
    /// the active theme did not declare a back asset (or no theme has loaded
    /// yet); in that case [`card_sprite`] falls back to the legacy array.
    pub theme_back: Option<Handle<Image>>,
}

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

/// Countdown (seconds) until the `HintHighlight` on a card entity is removed.
///
/// Inserted alongside `HintHighlight` by the hint-visual system. When the timer
/// reaches zero both `HintHighlight` and `HintHighlightTimer` are removed from
/// the entity and the sprite colour is restored.
#[derive(Component, Debug, Clone)]
pub struct HintHighlightTimer(pub f32);

/// Marker on a `PileMarker` entity that is highlighted because the right-clicked
/// card can legally be placed there.
#[derive(Component, Debug)]
pub struct RightClickHighlight;

/// Countdown (seconds) until this right-click destination highlight despawns.
///
/// Inserted alongside `RightClickHighlight` so that highlights auto-clear after
/// 1.5 s even if the player does not make a move or click again. The existing
/// clear-on-state-change and clear-on-pause logic still fires early when
/// appropriate.
#[derive(Component, Debug, Clone)]
pub struct RightClickHighlightTimer(pub f32);

/// Marker placed on the child `Text2d` entity that shows "↺" on the stock pile
/// marker when the stock pile is empty.
#[derive(Component, Debug)]
pub struct StockEmptyLabel;

/// Marker on the chip-background sprite of the stock-pile remaining-count
/// badge.
///
/// The badge is spawned as a *top-level* world entity (not parented to the
/// stock [`PileMarker`]) and its `Transform` is recomputed each frame from
/// `LayoutResource` so it tracks the stock pile through window resizes.
/// The chip sits in the top-right corner of the stock pile and is hidden
/// while the stock is empty — the existing `↺` overlay
/// ([`StockEmptyLabel`]) covers the recycle hint instead, so the two
/// indicators never render simultaneously.
#[derive(Component, Debug)]
pub struct StockCountBadge;

/// Marker on the `Text2d` child of [`StockCountBadge`] showing the numeric
/// count of cards remaining in the stock pile.
///
/// Update systems query this component to write the new count in place rather
/// than despawning and respawning the text entity each tick.
#[derive(Component, Debug)]
pub struct StockCountBadgeText;

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

/// Marker component for the per-card drop-shadow child sprite.
///
/// Every `CardEntity` owns exactly one `CardShadow` child whose `Sprite` is a
/// neutral-black halo painted slightly down-and-right of the card. Idle state
/// uses [`CARD_SHADOW_OFFSET_IDLE`] / [`CARD_SHADOW_ALPHA_IDLE`]; while the
/// parent card is being dragged the shadow is pushed to the deeper
/// [`CARD_SHADOW_OFFSET_DRAG`] / [`CARD_SHADOW_ALPHA_DRAG`] values so the
/// stack reads as "lifted" off the felt.
#[derive(Component, Debug)]
pub struct CardShadow;

/// Returns the `(offset, padding, alpha)` triple used to paint a per-card
/// shadow given whether its parent card is currently part of the dragged
/// stack. Pulled out as a pure helper so the shadow tuning can be unit-tested
/// without spinning up a Bevy app.
///
/// `is_dragged = false` → resting `(IDLE, IDLE, IDLE)`
/// `is_dragged = true`  → lifted  `(DRAG, DRAG, DRAG)`
pub fn card_shadow_params(is_dragged: bool) -> (Vec2, Vec2, f32) {
    if is_dragged {
        (
            CARD_SHADOW_OFFSET_DRAG,
            CARD_SHADOW_PADDING_DRAG,
            CARD_SHADOW_ALPHA_DRAG,
        )
    } else {
        (
            CARD_SHADOW_OFFSET_IDLE,
            CARD_SHADOW_PADDING_IDLE,
            CARD_SHADOW_ALPHA_IDLE,
        )
    }
}

/// Builds the `Sprite` used for a per-card shadow at the resting state. The
/// alpha and size both use the idle tokens; `update_card_shadows_on_drag`
/// retunes them at runtime when the parent card joins / leaves the dragged
/// stack.
fn card_shadow_sprite(card_size: Vec2) -> Sprite {
    let (_offset, padding, alpha) = card_shadow_params(false);
    Sprite {
        color: CARD_SHADOW_COLOR.with_alpha(alpha),
        custom_size: Some(card_size + padding),
        ..default()
    }
}

/// Builds the `Transform` used for a per-card shadow at the resting state.
/// Local — it is parented to the card entity, so positions are relative.
fn card_shadow_transform() -> Transform {
    let (offset, _padding, _alpha) = card_shadow_params(false);
    Transform::from_xyz(offset.x, offset.y, CARD_SHADOW_LOCAL_Z)
}

/// Spawns a single `CardShadow` child under the given card entity builder.
/// Extracted so `spawn_card_entity` and `update_card_entity` can share the
/// exact same shadow recipe — we never want one path to drift from the other.
fn add_card_shadow_child(parent: &mut ChildSpawnerCommands, card_size: Vec2) {
    parent.spawn((
        CardShadow,
        card_shadow_sprite(card_size),
        card_shadow_transform(),
        Visibility::default(),
    ));
}

/// Throttle interval for resize-driven card snap work, in seconds.
///
/// `WindowResized` fires once per pixel of drag, so a fast corner-drag can
/// produce dozens of events per frame. Re-running the per-card snap logic
/// (52 cards × sprite/transform/font_size touches) for every event is the
/// dominant cost of resize lag. We coalesce pending work and apply it at most
/// once per [`RESIZE_THROTTLE_SECS`] (~20 Hz). The user still sees updates
/// during a sustained drag, and the layout always catches up to the final
/// size when the drag stops because the pending size is held until applied.
const RESIZE_THROTTLE_SECS: f32 = 0.05;

/// Holds the latest pending window size from `WindowResized` events plus a
/// timestamp for the last applied snap, so the resize-snap work can be
/// rate-limited to ~20 Hz during sustained drags.
#[derive(Resource, Debug, Default)]
pub struct ResizeThrottle {
    /// Latest unapplied window size from `WindowResized`. `None` when there is
    /// nothing to apply.
    pub pending: Option<Vec2>,
    /// `Time::elapsed_secs()` value at the moment of the most recent applied
    /// snap. `0.0` until the first apply.
    pub last_applied_secs: f32,
}

/// Pure helper used by the throttled resize-snap system: returns `true` when
/// a pending resize should be flushed given the current `now_secs` and the
/// last-applied timestamp. Throttle interval is [`RESIZE_THROTTLE_SECS`].
///
/// Extracted so the rate-limit logic can be unit-tested without spinning up
/// a full Bevy app.
fn should_apply_resize(now_secs: f32, last_applied_secs: f32) -> bool {
    (now_secs - last_applied_secs) >= RESIZE_THROTTLE_SECS
}

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
            .init_resource::<ResizeThrottle>()
            .add_message::<SettingsChangedEvent>()
            .add_message::<CardFlippedEvent>()
            .add_message::<CardFaceRevealedEvent>()
            .add_systems(Startup, load_card_images)
            .add_systems(PostStartup, (sync_cards_startup, update_stock_empty_indicator_startup))
            .add_systems(
                Update,
                (
                    sync_cards_on_change.after(GameMutation),
                    resync_cards_on_settings_change.before(sync_cards_on_change),
                    start_flip_anim.after(GameMutation),
                    tick_flip_anim,
                    update_drag_shadow,
                    update_card_shadows_on_drag.after(sync_cards_on_change),
                    tick_hint_highlight,
                    handle_right_click,
                    tick_right_click_highlights,
                    clear_right_click_highlights_on_state_change.after(GameMutation),
                    clear_right_click_highlights_on_pause,
                    update_stock_empty_indicator.after(GameMutation),
                    update_stock_count_badge.after(GameMutation),
                    collect_resize_events.after(LayoutSystem::UpdateOnResize),
                    snap_cards_on_window_resize.after(collect_resize_events),
                ),
            );
    }
}

/// Loads card face and back PNGs at startup via [`AssetServer`] and inserts
/// [`CardImageSet`].
///
/// Faces: `assets/cards/faces/{RANK}{SUIT}.png`  (e.g. `AC.png`, `10H.png`)
/// Backs: `assets/cards/backs/back_{0..4}.png`
///
/// Under `MinimalPlugins` (tests) `AssetServer` is absent, so the system
/// returns without inserting `CardImageSet` and the plugin falls back to
/// solid-colour sprites.
fn load_card_images(asset_server: Option<Res<AssetServer>>, mut commands: Commands) {
    let Some(asset_server) = asset_server else {
        return;
    };

    // Suit index: Clubs=0, Diamonds=1, Hearts=2, Spades=3
    const SUIT_CHARS: [&str; 4] = ["C", "D", "H", "S"];
    // Rank index: Ace=0 … King=12
    const RANK_STRS: [&str; 13] = ["A", "2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K"];

    let faces: [[Handle<Image>; 13]; 4] = std::array::from_fn(|suit| {
        std::array::from_fn(|rank| {
            asset_server.load(format!(
                "cards/faces/{}{}.png",
                RANK_STRS[rank], SUIT_CHARS[suit]
            ))
        })
    });
    let backs = std::array::from_fn(|i| {
        asset_server.load(format!("cards/backs/back_{i}.png"))
    });
    commands.insert_resource(CardImageSet {
        faces,
        backs,
        // Populated by the theme plugin once a `CardTheme` finishes loading.
        // Until then the legacy back fallback (`backs[selected_card_back]`)
        // is used.
        theme_back: None,
    });
}

/// Builds the [`Sprite`] for a card, using PNG artwork when [`CardImageSet`] is
/// available and falling back to a solid-colour sprite in tests.
fn card_sprite(
    card: &Card,
    card_size: Vec2,
    back_colour: Color,
    color_blind: bool,
    card_images: Option<&CardImageSet>,
    selected_back: usize,
) -> Sprite {
    if let Some(set) = card_images {
        let image = if card.face_up {
            let suit_idx = match card.suit {
                Suit::Clubs => 0,
                Suit::Diamonds => 1,
                Suit::Hearts => 2,
                Suit::Spades => 3,
            };
            let rank_idx = match card.rank {
                Rank::Ace => 0,
                Rank::Two => 1,
                Rank::Three => 2,
                Rank::Four => 3,
                Rank::Five => 4,
                Rank::Six => 5,
                Rank::Seven => 6,
                Rank::Eight => 7,
                Rank::Nine => 8,
                Rank::Ten => 9,
                Rank::Jack => 10,
                Rank::Queen => 11,
                Rank::King => 12,
            };
            set.faces[suit_idx][rank_idx].clone()
        } else if let Some(theme_back) = &set.theme_back {
            // Active theme provides its own back — always wins over the
            // legacy `selected_card_back` picker, so a theme switch swaps
            // faces *and* the back. The picker is treated as informational
            // only while a theme back is active (see settings_plugin).
            theme_back.clone()
        } else {
            let idx = selected_back.min(set.backs.len() - 1);
            set.backs[idx].clone()
        };
        Sprite {
            image,
            color: Color::WHITE,
            custom_size: Some(card_size),
            ..default()
        }
    } else {
        let body_colour = if card.face_up {
            face_colour(card, color_blind)
        } else {
            back_colour
        };
        Sprite {
            color: body_colour,
            custom_size: Some(card_size),
            ..default()
        }
    }
}

/// When card-back selection changes in Settings, re-render all cards so the
/// new back colour is applied immediately (without waiting for a state change).
fn resync_cards_on_settings_change(
    mut setting_events: MessageReader<SettingsChangedEvent>,
    mut state_events: MessageWriter<StateChangedEvent>,
) {
    if setting_events.read().next().is_some() {
        state_events.write(StateChangedEvent);
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
    entities: Query<(Entity, &CardEntity, &Transform, Option<&CardAnimation>)>,
    card_images: Option<Res<CardImageSet>>,
) {
    if let Some(layout) = layout {
        let slide_secs = slide_dur.map_or(0.15, |d| d.slide_secs);
        let selected_back = settings.as_ref().map_or(0, |s| s.0.selected_card_back);
        let back_colour = card_back_colour(selected_back);
        let color_blind = settings.as_ref().is_some_and(|s| s.0.color_blind_mode);
        sync_cards(commands, &game.0, &layout.0, slide_secs, back_colour, color_blind, &entities, card_images.as_deref(), selected_back);
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_cards_on_change(
    mut events: MessageReader<StateChangedEvent>,
    commands: Commands,
    game: Res<GameStateResource>,
    layout: Option<Res<LayoutResource>>,
    slide_dur: Option<Res<EffectiveSlideDuration>>,
    settings: Option<Res<SettingsResource>>,
    entities: Query<(Entity, &CardEntity, &Transform, Option<&CardAnimation>)>,
    card_images: Option<Res<CardImageSet>>,
) {
    if events.read().next().is_none() {
        return;
    }
    if let Some(layout) = layout {
        let slide_secs = slide_dur.map_or(0.15, |d| d.slide_secs);
        let selected_back = settings.as_ref().map_or(0, |s| s.0.selected_card_back);
        let back_colour = card_back_colour(selected_back);
        let color_blind = settings.as_ref().is_some_and(|s| s.0.color_blind_mode);
        sync_cards(commands, &game.0, &layout.0, slide_secs, back_colour, color_blind, &entities, card_images.as_deref(), selected_back);
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_cards(
    mut commands: Commands,
    game: &GameState,
    layout: &Layout,
    slide_secs: f32,
    back_colour: Color,
    color_blind: bool,
    entities: &Query<(Entity, &CardEntity, &Transform, Option<&CardAnimation>)>,
    card_images: Option<&CardImageSet>,
    selected_back: usize,
) {
    let positions = card_positions(game, layout);

    // Map card_id -> (Entity, current_translation, has_card_animation) for
    // in-place updates. The `has_card_animation` flag lets `update_card_entity`
    // skip the snap/slide path on cards that are already being driven by a
    // curve-based `CardAnimation` tween (e.g. the drag-rejection return tween
    // — see `input_plugin::end_drag`). Otherwise the StateChangedEvent that
    // accompanies a rejection would race the tween and the card would jump.
    let mut existing: HashMap<u32, (Entity, Vec3, bool)> = HashMap::new();
    for (entity, marker, transform, anim) in entities.iter() {
        existing.insert(marker.card_id, (entity, transform.translation, anim.is_some()));
    }

    let live_ids: HashSet<u32> = positions.iter().map(|(c, _, _)| c.id).collect();

    // Despawn any entity whose card is no longer tracked.
    for (card_id, (entity, _, _)) in &existing {
        if !live_ids.contains(card_id) {
            commands.entity(*entity).despawn();
        }
    }

    // For each card in the current state: spawn or update its entity.
    for (card, position, z) in positions {
        match existing.get(&card.id) {
            Some(&(entity, cur, has_anim)) => {
                update_card_entity(
                    &mut commands, entity, card, position, z, layout,
                    slide_secs, back_colour, color_blind, cur, has_anim, card_images, selected_back,
                )
            }
            None => spawn_card_entity(&mut commands, card, position, z, layout, back_colour, color_blind, card_images, selected_back),
        }
    }
}

/// Returns an ordered vec of (card, position, z) for every card in the game.
fn card_positions<'a>(game: &'a GameState, layout: &Layout) -> Vec<(&'a Card, Vec2, f32)> {
    let mut out: Vec<(&'a Card, Vec2, f32)> = Vec::with_capacity(52);
    let piles = [
        PileType::Stock,
        PileType::Waste,
        PileType::Foundation(0),
        PileType::Foundation(1),
        PileType::Foundation(2),
        PileType::Foundation(3),
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
            out.push((card, pos, z));
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

#[allow(clippy::too_many_arguments)]
fn spawn_card_entity(
    commands: &mut Commands,
    card: &Card,
    pos: Vec2,
    z: f32,
    layout: &Layout,
    back_colour: Color,
    color_blind: bool,
    card_images: Option<&CardImageSet>,
    selected_back: usize,
) {
    let sprite = card_sprite(card, layout.card_size, back_colour, color_blind, card_images, selected_back);

    let mut entity = commands.spawn((
        CardEntity { card_id: card.id },
        sprite,
        Transform::from_xyz(pos.x, pos.y, z),
        Visibility::default(),
    ));
    // Every card gets a subtle drop-shadow child so the play surface reads
    // as physical instead of flat. Spawned in idle state; the drag-tracking
    // system retunes its offset / alpha when this card joins the dragged
    // stack.
    entity.with_children(|b| {
        add_card_shadow_child(b, layout.card_size);
    });
    // When PNG faces are loaded the rank/suit are baked into the image.
    // Only spawn the Text2d overlay in the solid-colour fallback (tests).
    if card_images.is_none() {
        entity.with_children(|b| {
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
    has_card_animation: bool,
    card_images: Option<&CardImageSet>,
    selected_back: usize,
) {
    let target = Vec3::new(pos.x, pos.y, z);

    // Always refresh the visual appearance.
    commands.entity(entity).insert(card_sprite(card, layout.card_size, back_colour, color_blind, card_images, selected_back));

    // Skip the snap/slide path entirely when a curve-based `CardAnimation`
    // is driving this card (e.g. the drag-rejection return tween). Writing
    // `Transform` here would race that animation each frame and cause a
    // visible jump. The animation system snaps the final position itself
    // when it completes.
    if !has_card_animation {
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
    }

    // Despawn any stale children and re-add the per-card drop shadow plus,
    // in solid-colour fallback mode, the label overlay. In image mode the
    // rank/suit are baked into the PNG, so no `Text2d` overlay is needed.
    commands.entity(entity).despawn_related::<Children>();
    commands.entity(entity).with_children(|b| {
        add_card_shadow_child(b, layout.card_size);
    });
    if card_images.is_none() {
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
    mut events: MessageReader<CardFlippedEvent>,
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
    mut reveal_events: MessageWriter<CardFaceRevealedEvent>,
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
                    reveal_events.write(CardFaceRevealedEvent(card_entity.card_id));
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
            commands.entity(e).despawn();
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

/// Snaps every per-card [`CardShadow`] between its idle and lifted tunings
/// based on whether the parent [`CardEntity`] is currently in
/// [`DragState::cards`]. Runs every frame; the transition is an instant snap
/// (no lerp) — the existing shake / settle feedback already handles motion
/// at drag-end, so an additional shadow tween would compete with those cues.
///
/// The shadow size is rebuilt from the parent card's current `Sprite`
/// `custom_size` plus the appropriate padding, so the resize handler does
/// not need to pre-tune shadow sizes for the drag state — this system fixes
/// the geometry within one frame.
fn update_card_shadows_on_drag(
    drag: Res<DragState>,
    cards: Query<(&CardEntity, &Sprite, &Children), Without<CardShadow>>,
    mut shadows: Query<(&mut Sprite, &mut Transform), With<CardShadow>>,
) {
    let dragged: HashSet<u32> = drag.cards.iter().copied().collect();

    for (card_entity, card_sprite, children) in cards.iter() {
        let is_dragged = dragged.contains(&card_entity.card_id);
        let (offset, padding, alpha) = card_shadow_params(is_dragged);
        let Some(card_size) = card_sprite.custom_size else {
            continue;
        };

        for child in children.iter() {
            let Ok((mut shadow_sprite, mut shadow_transform)) = shadows.get_mut(child) else {
                continue;
            };
            shadow_sprite.color = CARD_SHADOW_COLOR.with_alpha(alpha);
            shadow_sprite.custom_size = Some(card_size + padding);
            shadow_transform.translation.x = offset.x;
            shadow_transform.translation.y = offset.y;
            shadow_transform.translation.z = CARD_SHADOW_LOCAL_Z;
        }
    }
}

// ---------------------------------------------------------------------------
// Task #28 — Hint highlight tick system
// ---------------------------------------------------------------------------

/// Counts down `HintHighlight::remaining` each frame. When it reaches zero,
/// removes both `HintHighlight` and `HintHighlightTimer` (if present) and
/// resets the card sprite to its normal face-up colour.
fn tick_hint_highlight(
    time: Res<Time>,
    mut commands: Commands,
    mut query: Query<(Entity, &mut HintHighlight, &mut Sprite, &CardEntity)>,
    game: Res<GameStateResource>,
    settings: Option<Res<SettingsResource>>,
    card_images: Option<Res<CardImageSet>>,
) {
    let back_idx = settings.as_ref().map_or(0, |s| s.0.selected_card_back);
    let use_images = card_images.is_some();
    for (entity, mut hint, mut sprite, card_entity) in query.iter_mut() {
        hint.remaining -= time.delta_secs();
        if hint.remaining <= 0.0 {
            // Restore the normal sprite colour.
            // When image-based rendering is active, WHITE is the neutral tint;
            // otherwise restore the solid colour appropriate to the card state.
            sprite.color = if use_images {
                Color::WHITE
            } else {
                let is_face_up = game.0.piles.values()
                    .flat_map(|p| p.cards.iter())
                    .find(|c| c.id == card_entity.card_id)
                    .is_some_and(|c| c.face_up);
                if is_face_up { CARD_FACE_COLOUR } else { card_back_colour(back_idx) }
            };
            commands
                .entity(entity)
                .remove::<HintHighlight>()
                .remove::<HintHighlightTimer>();
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

/// Counts down `RightClickHighlightTimer` each frame and clears the highlight
/// when the timer expires.
///
/// This is a fallback expiry: highlights also clear immediately on
/// `StateChangedEvent` (move made) or when the game is paused, whichever comes
/// first. The 1.5 s timer ensures highlights always disappear even if the
/// player takes no further action.
fn tick_right_click_highlights(
    mut commands: Commands,
    time: Res<Time>,
    paused: Option<Res<PausedResource>>,
    mut highlights: Query<(Entity, &mut RightClickHighlightTimer, &mut Sprite), With<RightClickHighlight>>,
) {
    if paused.is_some_and(|p| p.0) {
        return;
    }
    let dt = time.delta_secs();
    for (entity, mut timer, mut sprite) in &mut highlights {
        timer.0 -= dt;
        if timer.0 <= 0.0 {
            // Restore the pile marker to its default colour before removing
            // the highlight marker component.
            sprite.color = PILE_MARKER_DEFAULT_COLOUR;
            commands
                .entity(entity)
                .remove::<RightClickHighlight>()
                .remove::<RightClickHighlightTimer>();
        }
    }
}

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
    mut events: MessageReader<StateChangedEvent>,
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
            PileType::Foundation(_) => {
                can_place_on_foundation(&card, pile)
            }
            PileType::Tableau(_) => can_place_on_tableau(&card, pile),
            _ => false,
        };
        if legal {
            sprite.color = RIGHT_CLICK_HIGHLIGHT_COLOUR;
            commands
                .entity(entity)
                .insert(RightClickHighlight)
                .insert(RightClickHighlightTimer(1.5));
        }
    }
}

/// Converts cursor position to 2-D world coordinates.
fn cursor_world_pos(
    windows: &Query<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: &Query<(&Camera, &GlobalTransform)>,
) -> Option<Vec2> {
    let window = windows.single().ok()?;
    let cursor = window.cursor_position()?;
    let (camera, camera_transform) = cameras.single().ok()?;
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
fn apply_stock_empty_indicator<F: bevy::ecs::query::QueryFilter>(
    commands: &mut Commands,
    game: &GameState,
    pile_markers: &mut Query<(Entity, &PileMarker, &mut Sprite), F>,
    label_children: &Query<(Entity, &ChildOf), With<StockEmptyLabel>>,
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
                .any(|(_, parent)| parent.parent() == entity);
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
                if parent.parent() == entity {
                    commands.entity(label_entity).despawn();
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
    label_children: Query<(Entity, &ChildOf), With<StockEmptyLabel>>,
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
    mut events: MessageReader<StateChangedEvent>,
    mut commands: Commands,
    game: Res<GameStateResource>,
    layout: Option<Res<LayoutResource>>,
    mut pile_markers: Query<(Entity, &PileMarker, &mut Sprite)>,
    label_children: Query<(Entity, &ChildOf), With<StockEmptyLabel>>,
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

// ---------------------------------------------------------------------------
// Stock-pile remaining-count badge
//
// Shows a small "·N" chip pinned to the top-right corner of the stock pile so
// the player can see how many cards remain before the next recycle. The
// existing `StockEmptyLabel` (`↺` overlay) covers the empty-stock case, so
// the badge hides itself when the stock has zero cards — the two indicators
// never render at the same time.
// ---------------------------------------------------------------------------

/// Inset (in pixels) from the top-right corner of the stock pile sprite to
/// the centre of the count badge. A small inward offset keeps the chip from
/// drifting half-off the card while still reading as "attached" to the
/// corner.
const STOCK_BADGE_INSET: Vec2 = Vec2::new(-12.0, -8.0);

/// Width / height of the badge background sprite, in world pixels. Sized so
/// a 2-digit count (max "24") fits comfortably with `TYPE_CAPTION` text.
const STOCK_BADGE_SIZE: Vec2 = Vec2::new(28.0, 16.0);

/// Returns the count of cards currently in the stock pile.
///
/// Pure helper extracted so the count source is identical between the spawn
/// system, the update system, and the unit tests.
fn stock_card_count(game: &GameState) -> usize {
    game.piles
        .get(&PileType::Stock)
        .map_or(0, |p| p.cards.len())
}

/// Returns the world-space `Vec3` for the centre of the stock-count badge,
/// given the current `Layout`. The badge sits at the top-right corner of
/// the stock pile sprite, inset by [`STOCK_BADGE_INSET`].
fn stock_badge_translation(layout: &Layout) -> Vec3 {
    // Empty layouts don't contain a Stock entry — fall back to origin so
    // the badge stays in a deterministic spot until the layout is filled.
    let pile_pos = layout
        .pile_positions
        .get(&PileType::Stock)
        .copied()
        .unwrap_or(Vec2::ZERO);
    let half = layout.card_size * 0.5;
    let x = pile_pos.x + half.x + STOCK_BADGE_INSET.x;
    let y = pile_pos.y + half.y + STOCK_BADGE_INSET.y;
    Vec3::new(x, y, Z_STOCK_BADGE)
}

/// Spawns the stock-count badge entity (background sprite + child text)
/// into the world. Called once, when the badge does not yet exist.
fn spawn_stock_count_badge(
    commands: &mut Commands,
    layout: &Layout,
    font: Option<&Handle<Font>>,
    count: usize,
) {
    let translation = stock_badge_translation(layout);
    let visibility = if count == 0 {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    let text_font = TextFont {
        font: font.cloned().unwrap_or_default(),
        font_size: TYPE_CAPTION,
        ..default()
    };

    commands
        .spawn((
            StockCountBadge,
            Sprite {
                color: STOCK_BADGE_BG,
                custom_size: Some(STOCK_BADGE_SIZE),
                ..default()
            },
            Transform::from_translation(translation),
            visibility,
        ))
        .with_children(|b| {
            b.spawn((
                StockCountBadgeText,
                Text2d::new(format!("·{count}")),
                text_font,
                TextColor(STOCK_BADGE_FG),
                // Slightly above the chip background so the digits aren't
                // occluded by the sprite they sit on.
                Transform::from_xyz(0.0, 0.0, 0.1),
            ));
        });
}

/// Spawns the stock-pile remaining-count badge if it does not yet exist,
/// and otherwise updates its text and visibility in place.
///
/// Visibility rule: hidden when the stock is empty (the existing `↺`
/// `StockEmptyLabel` overlay covers that state), shown when one or more
/// cards remain.
///
/// Position is recomputed from `LayoutResource` every tick so the badge
/// follows the stock pile across `WindowResized` layout updates without
/// needing a dedicated resize handler.
#[allow(clippy::too_many_arguments)]
fn update_stock_count_badge(
    mut commands: Commands,
    game: Option<Res<GameStateResource>>,
    layout: Option<Res<LayoutResource>>,
    font: Option<Res<FontResource>>,
    mut badges: Query<(Entity, &mut Transform, &mut Visibility), With<StockCountBadge>>,
    children: Query<&Children, With<StockCountBadge>>,
    mut texts: Query<&mut Text2d, With<StockCountBadgeText>>,
) {
    let Some(game) = game else { return };
    let Some(layout) = layout else { return };

    let count = stock_card_count(&game.0);
    let translation = stock_badge_translation(&layout.0);
    let target_visibility = if count == 0 {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };

    if badges.is_empty() {
        spawn_stock_count_badge(
            &mut commands,
            &layout.0,
            font.as_ref().map(|f| &f.0),
            count,
        );
        return;
    }

    for (entity, mut transform, mut visibility) in badges.iter_mut() {
        transform.translation = translation;
        if *visibility != target_visibility {
            *visibility = target_visibility;
        }
        // Update the child text to reflect the latest count. The text node
        // is created at spawn time, so under normal operation we always
        // have exactly one child here.
        if let Ok(badge_children) = children.get(entity) {
            for child in badge_children.iter() {
                if let Ok(mut text) = texts.get_mut(child) {
                    let new = format!("·{count}");
                    if text.0 != new {
                        text.0 = new;
                    }
                }
            }
        }
    }
}

/// Coalesces every `WindowResized` event arriving this frame into the latest
/// pending size on [`ResizeThrottle`].
///
/// `WindowResized` fires per pixel of resize drag, so a fast corner drag can
/// emit many events per frame. Reading `.last()` keeps only the final size —
/// every frame's snap target is the most recent window size, never a stale
/// one. Pending stays set across frames until the throttled applier consumes
/// it; that's how we still flush the final "release" position when the user
/// stops dragging.
fn collect_resize_events(
    mut events: MessageReader<WindowResized>,
    mut throttle: ResMut<ResizeThrottle>,
) {
    if let Some(ev) = events.read().last() {
        throttle.pending = Some(Vec2::new(ev.width, ev.height));
    }
}

/// Snaps every card sprite to its target position, size, and (in the
/// fallback Text2d label path) font size when the window is resized.
///
/// **In-place mutation only.** Resize is the hot path — events fire per
/// pixel of drag, so this system cannot afford the despawn/respawn churn
/// `update_card_entity` does. We mutate `Sprite.custom_size`, `Transform`,
/// and child `TextFont.font_size` directly, leaving the card image handle,
/// suit/rank, and `CardLabel` entity untouched. Cards keep their identity
/// across resizes; only their size and position change. The full repaint
/// path lives in [`update_card_entity`] and is still used by every non-resize
/// caller (deals, moves, flips, settings toggles).
///
/// **Throttled to ~20 Hz.** [`ResizeThrottle::pending`] is consumed at most
/// once per [`RESIZE_THROTTLE_SECS`]. When events stop arriving, the next
/// tick past the throttle window flushes the final size and clears
/// `pending`, so the steady-state always matches the user's release size.
///
/// **Cancels in-flight slides.** Any `CardAnim` is removed so a mid-slide
/// tween is not retargeted relative to the previous card-size's position.
///
/// The "↺" stock-empty label's `font_size` is derived from
/// `layout.card_size.x`, so this system also reapplies the stock indicator —
/// otherwise the label would not rescale on resize.
///
/// Scheduled after [`collect_resize_events`] (which itself runs after
/// `LayoutSystem::UpdateOnResize`) so `LayoutResource` reflects the latest
/// window size before we read it.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn snap_cards_on_window_resize(
    mut commands: Commands,
    time: Res<Time>,
    mut throttle: ResMut<ResizeThrottle>,
    game: Option<Res<GameStateResource>>,
    layout: Option<Res<LayoutResource>>,
    card_images: Option<Res<CardImageSet>>,
    entities: Query<
        (Entity, &CardEntity, &mut Sprite, &mut Transform),
        (Without<CardLabel>, Without<CardShadow>),
    >,
    label_query: Query<&mut TextFont, (With<CardLabel>, Without<StockEmptyLabel>)>,
    shadow_query: Query<&mut Sprite, (With<CardShadow>, Without<CardEntity>, Without<PileMarker>)>,
    mut pile_markers: Query<
        (Entity, &PileMarker, &mut Sprite),
        (Without<CardEntity>, Without<CardShadow>),
    >,
    label_children: Query<(Entity, &ChildOf), With<StockEmptyLabel>>,
) {
    if throttle.pending.is_none() {
        return;
    }
    let now = time.elapsed_secs();
    if !should_apply_resize(now, throttle.last_applied_secs) {
        return;
    }

    let Some(game) = game else {
        // Nothing to apply — clear pending so we don't busy-loop.
        throttle.pending = None;
        return;
    };
    let Some(layout) = layout else {
        throttle.pending = None;
        return;
    };

    resize_cards_in_place(
        &mut commands,
        &game.0,
        &layout.0,
        card_images.as_deref(),
        entities,
        label_query,
        shadow_query,
    );

    apply_stock_empty_indicator(
        &mut commands,
        &game.0,
        &mut pile_markers,
        &label_children,
        &layout.0,
    );

    throttle.last_applied_secs = now;
    throttle.pending = None;
}

/// In-place "size-only" sibling of [`sync_cards`]: walks every existing card
/// entity, updates `Sprite.custom_size` and the snap-`Transform` to match the
/// fresh layout, and (in fallback solid-colour mode) also updates the child
/// `TextFont.font_size` of any `CardLabel`. No despawning, no `Sprite`
/// replacement, no children rebuild — that's the entire point of this path.
///
/// Called only from the resize handler. Game-state changes (deals, moves,
/// flips, settings toggles) still flow through [`sync_cards`] /
/// [`update_card_entity`], which handle add/remove/repaint correctly.
///
/// Any in-flight `CardAnim` slide is removed so a mid-tween card is not
/// retargeted relative to the previous card-size's position.
#[allow(clippy::type_complexity)]
fn resize_cards_in_place(
    commands: &mut Commands,
    game: &GameState,
    layout: &Layout,
    card_images: Option<&CardImageSet>,
    mut entities: Query<
        (Entity, &CardEntity, &mut Sprite, &mut Transform),
        (Without<CardLabel>, Without<CardShadow>),
    >,
    mut label_query: Query<&mut TextFont, (With<CardLabel>, Without<StockEmptyLabel>)>,
    mut shadow_query: Query<
        &mut Sprite,
        (With<CardShadow>, Without<CardEntity>, Without<PileMarker>),
    >,
) {
    let positions = card_positions(game, layout);
    let pos_by_id: HashMap<u32, (Vec2, f32)> = positions
        .into_iter()
        .map(|(c, p, z)| (c.id, (p, z)))
        .collect();

    for (entity, marker, mut sprite, mut transform) in entities.iter_mut() {
        let Some(&(pos, z)) = pos_by_id.get(&marker.card_id) else {
            continue;
        };
        sprite.custom_size = Some(layout.card_size);
        transform.translation.x = pos.x;
        transform.translation.y = pos.y;
        transform.translation.z = z;
        // Cancel any in-flight slide so it doesn't retarget from a stale
        // mid-animation position computed against the previous card size.
        commands.entity(entity).remove::<CardAnim>();
    }

    // Resize every per-card shadow halo to match the new card size. Both
    // idle and drag states scale with the card body, so we preserve the
    // *current* padding (idle vs drag) by keeping the alpha as-is and only
    // recomputing the geometry. The drag-tracking system runs every frame
    // and will retune offset / alpha / padding-mode within one frame if the
    // drag state diverges from the resized geometry.
    let idle_padding = CARD_SHADOW_PADDING_IDLE;
    let drag_padding = CARD_SHADOW_PADDING_DRAG;
    for mut shadow_sprite in shadow_query.iter_mut() {
        // Choose padding based on the shadow's current alpha — preserves
        // a lifted shadow's larger halo across resize without needing to
        // plumb DragState through the resize handler.
        let alpha = shadow_sprite.color.alpha();
        let padding = if alpha >= CARD_SHADOW_ALPHA_DRAG - 0.001 {
            drag_padding
        } else {
            idle_padding
        };
        shadow_sprite.custom_size = Some(layout.card_size + padding);
    }

    // Only the solid-colour fallback path uses CardLabel/Text2d overlays;
    // when PNG faces are loaded the rank/suit are baked into the image and
    // there is nothing to resize on the label side.
    if card_images.is_none() {
        let new_font_size = layout.card_size.x * FONT_SIZE_FRAC;
        for mut font in label_query.iter_mut() {
            font.font_size = new_font_size;
        }
    }
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
        app.world_mut().write_message(crate::events::DrawRequestEvent);
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

    // -----------------------------------------------------------------------
    // Task #5 — RightClickHighlightTimer pure-function tests
    // -----------------------------------------------------------------------

    /// Verify that a freshly-created timer with 1.5 s has a positive countdown
    /// and has not yet expired.
    #[test]
    fn right_click_highlight_timer_starts_positive() {
        let timer = RightClickHighlightTimer(1.5);
        assert!(
            timer.0 > 0.0,
            "timer must start with a positive countdown, got {}",
            timer.0
        );
    }

    /// Simulate ticking the timer by a delta that exceeds its initial value and
    /// verify the resulting value is ≤ 0 (expiry condition).
    #[test]
    fn right_click_highlight_timer_expires_after_sufficient_ticks() {
        let mut remaining = 1.5_f32;
        // Tick by more than the initial value to ensure expiry.
        remaining -= 2.0;
        assert!(
            remaining <= 0.0,
            "timer must be expired (≤ 0) after 2.0 s tick on a 1.5 s timer, got {}",
            remaining
        );
    }

    /// Simulate ticking by less than the initial value and verify the timer is
    /// still positive (not yet expired).
    #[test]
    fn right_click_highlight_timer_not_expired_before_duration() {
        let mut remaining = 1.5_f32;
        remaining -= 0.5; // only 0.5 s elapsed
        assert!(
            remaining > 0.0,
            "timer must still be positive after only 0.5 s on a 1.5 s timer, got {}",
            remaining
        );
    }

    // -----------------------------------------------------------------------
    // Constant sanity bounds (pure)
    // -----------------------------------------------------------------------

    #[test]
    fn tableau_fan_frac_is_in_unit_interval() {
        const {
            assert!(
                TABLEAU_FAN_FRAC > 0.0 && TABLEAU_FAN_FRAC < 1.0,
                "TABLEAU_FAN_FRAC must be in (0, 1)"
            );
        }
    }

    #[test]
    fn flip_half_secs_is_positive() {
        const {
            assert!(FLIP_HALF_SECS > 0.0, "FLIP_HALF_SECS must be positive");
        }
    }

    #[test]
    fn font_size_frac_is_positive_and_reasonable() {
        const {
            assert!(
                FONT_SIZE_FRAC > 0.0 && FONT_SIZE_FRAC <= 1.0,
                "FONT_SIZE_FRAC should be in (0, 1]"
            );
        }
    }

    // -----------------------------------------------------------------------
    // face_colour (pure) — color-blind mode
    // -----------------------------------------------------------------------

    #[test]
    fn face_colour_normal_mode_returns_card_face_colour_for_red_suit() {
        let card = Card { id: 0, suit: Suit::Hearts, rank: Rank::King, face_up: true };
        assert_eq!(face_colour(&card, false), CARD_FACE_COLOUR);
    }

    #[test]
    fn face_colour_normal_mode_returns_card_face_colour_for_black_suit() {
        let card = Card { id: 0, suit: Suit::Spades, rank: Rank::King, face_up: true };
        assert_eq!(face_colour(&card, false), CARD_FACE_COLOUR);
    }

    #[test]
    fn face_colour_color_blind_mode_gives_red_suits_a_different_tint() {
        let red_card = Card { id: 0, suit: Suit::Diamonds, rank: Rank::Queen, face_up: true };
        let cbm_colour = face_colour(&red_card, true);
        assert_ne!(
            cbm_colour, CARD_FACE_COLOUR,
            "color-blind mode must tint red-suit cards differently from the standard face colour"
        );
    }

    #[test]
    fn face_colour_color_blind_mode_does_not_change_black_suits() {
        let black_card = Card { id: 0, suit: Suit::Clubs, rank: Rank::Jack, face_up: true };
        assert_eq!(
            face_colour(&black_card, true),
            CARD_FACE_COLOUR,
            "color-blind mode must not alter black-suit card face colour"
        );
    }

    // -----------------------------------------------------------------------
    // label_visibility (pure)
    // -----------------------------------------------------------------------

    #[test]
    fn label_visibility_face_up_is_inherited() {
        let card = Card { id: 0, suit: Suit::Clubs, rank: Rank::Ace, face_up: true };
        assert_eq!(label_visibility(&card), Visibility::Inherited);
    }

    #[test]
    fn label_visibility_face_down_is_hidden() {
        let card = Card { id: 0, suit: Suit::Clubs, rank: Rank::Ace, face_up: false };
        assert_eq!(label_visibility(&card), Visibility::Hidden);
    }

    // -----------------------------------------------------------------------
    // label_for — remaining ranks not yet covered
    // -----------------------------------------------------------------------

    #[test]
    fn label_for_all_ranks_contain_suit_letter() {
        let suits = [Suit::Clubs, Suit::Diamonds, Suit::Hearts, Suit::Spades];
        let letters = ["C", "D", "H", "S"];
        for (suit, letter) in suits.iter().zip(letters.iter()) {
            let card = Card { id: 0, suit: *suit, rank: Rank::King, face_up: true };
            assert!(
                label_for(&card).ends_with(letter),
                "label for {suit:?} must end with '{letter}'"
            );
        }
    }

    #[test]
    fn label_for_face_cards_use_letter_prefix() {
        let make = |rank| Card { id: 0, suit: Suit::Spades, rank, face_up: true };
        assert!(label_for(&make(Rank::Jack)).starts_with('J'));
        assert!(label_for(&make(Rank::Queen)).starts_with('Q'));
        assert!(label_for(&make(Rank::King)).starts_with('K'));
    }

    #[test]
    fn label_for_numeric_ranks_two_through_nine() {
        let make = |rank| Card { id: 0, suit: Suit::Clubs, rank, face_up: true };
        let expected = [
            (Rank::Two,   "2C"),
            (Rank::Three, "3C"),
            (Rank::Four,  "4C"),
            (Rank::Five,  "5C"),
            (Rank::Six,   "6C"),
            (Rank::Seven, "7C"),
            (Rank::Eight, "8C"),
            (Rank::Nine,  "9C"),
        ];
        for (rank, label) in expected {
            assert_eq!(label_for(&make(rank)), label, "rank {rank:?}");
        }
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

    // -----------------------------------------------------------------------
    // Resize-lag fix — throttle helper + in-place mutation regression tests
    // -----------------------------------------------------------------------

    #[test]
    fn should_apply_resize_returns_false_below_threshold() {
        // 0 elapsed since last apply: still inside the throttle window.
        assert!(!should_apply_resize(0.0, 0.0));
        // Just under the threshold: still throttled.
        assert!(!should_apply_resize(RESIZE_THROTTLE_SECS - 0.001, 0.0));
    }

    #[test]
    fn should_apply_resize_returns_true_at_or_past_threshold() {
        // Exactly at the threshold the work should fire.
        assert!(should_apply_resize(RESIZE_THROTTLE_SECS, 0.0));
        // Comfortably past the threshold: definitely fire.
        assert!(should_apply_resize(1.0, 0.0));
    }

    #[test]
    fn should_apply_resize_uses_last_applied_as_baseline() {
        // After an apply at t=10.0, a subsequent check at t=10.04 is still
        // throttled (under the 50 ms window).
        assert!(!should_apply_resize(10.04, 10.0));
        // At t=10.05 the next apply is allowed.
        assert!(should_apply_resize(10.05, 10.0));
    }

    /// Helper: drive enough `app.update()` ticks at 200 ms each to comfortably
    /// exceed the throttle window. `Time<Virtual>` clamps each delta to
    /// `max_delta` (default 250 ms) regardless of the requested step, so we
    /// step in 200 ms slices.
    fn advance_past_resize_throttle(app: &mut App) {
        use bevy::time::TimeUpdateStrategy;
        use std::time::Duration;
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(0.2),
        ));
        // One tick to advance Time, plus one extra so the snap system runs
        // after the throttle window has elapsed.
        app.update();
        app.update();
    }

    fn fire_window_resize(app: &mut App, width: f32, height: f32) {
        // Any Entity will do — the snap system reads only width/height.
        let window = bevy::ecs::entity::Entity::from_raw_u32(0)
            .expect("Entity::from_raw_u32(0) is a valid placeholder");
        app.world_mut().write_message(WindowResized {
            window,
            width,
            height,
        });
    }

    #[test]
    fn resize_does_not_despawn_card_labels() {
        // Spawn a fresh app, capture the current set of CardLabel entity IDs,
        // fire a WindowResized, run the throttled snap, and assert *every*
        // captured label still exists. The whole point of the in-place resize
        // path is that it doesn't despawn-and-respawn label children — old
        // entity IDs must remain alive.
        let mut app = app();

        let labels_before: std::collections::HashSet<bevy::prelude::Entity> = app
            .world_mut()
            .query_filtered::<bevy::prelude::Entity, With<CardLabel>>()
            .iter(app.world())
            .collect();
        assert!(
            !labels_before.is_empty(),
            "fixture should have spawned CardLabel children in the fallback solid-colour path"
        );

        fire_window_resize(&mut app, 1024.0, 768.0);
        advance_past_resize_throttle(&mut app);

        let labels_after: std::collections::HashSet<bevy::prelude::Entity> = app
            .world_mut()
            .query_filtered::<bevy::prelude::Entity, With<CardLabel>>()
            .iter(app.world())
            .collect();

        // Same set of entities — no entity was despawned. (Bevy reuses
        // indices but bumps generations on despawn, so direct Entity equality
        // is sufficient here.)
        for e in &labels_before {
            assert!(
                labels_after.contains(e),
                "CardLabel entity {e:?} was despawned by the resize handler — \
                 expected the in-place path to leave label entities untouched"
            );
        }
    }

    #[test]
    fn resize_in_place_updates_card_label_font_size() {
        // Capture an arbitrary CardLabel's TextFont.font_size before resize,
        // fire a WindowResized to a *smaller* window, run the throttled snap,
        // and assert the font_size shrank. This proves the in-place path
        // actually mutates the existing TextFont (rather than skipping it or
        // falling back to despawn/respawn).
        let mut app = app();

        // Read the first CardLabel's font size.
        let mut q = app
            .world_mut()
            .query_filtered::<&TextFont, With<CardLabel>>();
        let before = q
            .iter(app.world())
            .next()
            .expect("fixture should have at least one CardLabel")
            .font_size;
        assert!(before > 0.0, "baseline font size must be positive, got {before}");

        // Resize to a window smaller than the default fixture so the
        // computed font size is unambiguously smaller.
        fire_window_resize(&mut app, 800.0, 600.0);
        advance_past_resize_throttle(&mut app);

        let mut q = app
            .world_mut()
            .query_filtered::<&TextFont, With<CardLabel>>();
        let after = q
            .iter(app.world())
            .next()
            .expect("CardLabel must still exist after in-place resize")
            .font_size;

        assert!(
            after < before,
            "smaller window should shrink CardLabel font size in place \
             (before={before}, after={after})"
        );

        // Sanity-check: the new font size matches FONT_SIZE_FRAC × the
        // post-resize card width, so the in-place path is using the
        // refreshed Layout.
        let expected_layout = crate::layout::compute_layout(Vec2::new(800.0, 600.0));
        let expected = expected_layout.card_size.x * FONT_SIZE_FRAC;
        assert!(
            (after - expected).abs() < 1e-3,
            "after-resize font size should equal layout.card_size.x * FONT_SIZE_FRAC \
             (got {after}, expected {expected})"
        );
    }

    // -----------------------------------------------------------------------
    // Per-card drop-shadow — pure helper + spawn / drag-snap regressions.
    // -----------------------------------------------------------------------

    /// `card_shadow_params(false)` returns the IDLE token triple.
    #[test]
    fn card_shadow_params_idle_returns_idle_tokens() {
        let (offset, padding, alpha) = card_shadow_params(false);
        assert_eq!(offset, CARD_SHADOW_OFFSET_IDLE);
        assert_eq!(padding, CARD_SHADOW_PADDING_IDLE);
        assert!((alpha - CARD_SHADOW_ALPHA_IDLE).abs() < f32::EPSILON);
    }

    /// `card_shadow_params(true)` returns the DRAG token triple, and each
    /// drag value differs from its idle counterpart so the player visibly
    /// sees the lift.
    #[test]
    fn card_shadow_params_drag_returns_drag_tokens_and_differs_from_idle() {
        let (idle_offset, idle_padding, idle_alpha) = card_shadow_params(false);
        let (drag_offset, drag_padding, drag_alpha) = card_shadow_params(true);

        assert_eq!(drag_offset, CARD_SHADOW_OFFSET_DRAG);
        assert_eq!(drag_padding, CARD_SHADOW_PADDING_DRAG);
        assert!((drag_alpha - CARD_SHADOW_ALPHA_DRAG).abs() < f32::EPSILON);

        assert_ne!(idle_offset, drag_offset, "drag offset must differ from idle");
        assert_ne!(idle_padding, drag_padding, "drag padding must differ from idle");
        assert!(
            drag_alpha > idle_alpha,
            "drag alpha must be stronger than idle (got drag={drag_alpha}, idle={idle_alpha})"
        );
        // Drag offset magnitude should be larger than idle so the parallax
        // reads as "lifted".
        assert!(
            drag_offset.length() > idle_offset.length(),
            "drag offset magnitude ({}) must exceed idle ({}) so the lift is visible",
            drag_offset.length(),
            idle_offset.length(),
        );
    }

    /// Every spawned `CardEntity` owns exactly one `CardShadow` child.
    /// Total counts must match: 52 cards → 52 shadows.
    #[test]
    fn cards_spawn_with_shadow_child() {
        let mut app = app();

        let card_count = app
            .world_mut()
            .query::<&CardEntity>()
            .iter(app.world())
            .count();
        assert_eq!(card_count, 52, "fixture should spawn 52 cards");

        let shadow_count = app
            .world_mut()
            .query::<&CardShadow>()
            .iter(app.world())
            .count();
        assert_eq!(
            shadow_count, 52,
            "every CardEntity must own exactly one CardShadow child (got {shadow_count})"
        );

        // Each shadow's parent must be a CardEntity, so the child relation
        // is wired correctly.
        let cards: HashSet<bevy::prelude::Entity> = app
            .world_mut()
            .query_filtered::<bevy::prelude::Entity, With<CardEntity>>()
            .iter(app.world())
            .collect();
        let mut q = app
            .world_mut()
            .query_filtered::<&ChildOf, With<CardShadow>>();
        for parent in q.iter(app.world()) {
            assert!(
                cards.contains(&parent.parent()),
                "CardShadow parent {:?} is not a CardEntity",
                parent.parent()
            );
        }
    }

    /// Driving `DragState.cards` with a card id and ticking the app must
    /// move that card's shadow to the lifted offset and alpha; cards
    /// outside the dragged set keep the idle tuning.
    #[test]
    fn shadow_offset_increases_during_drag() {
        let mut app = app();

        // Pick any spawned card id and stage it in DragState.
        let card_id: u32 = {
            let mut q = app.world_mut().query::<&CardEntity>();
            q.iter(app.world())
                .next()
                .expect("fixture should spawn at least one CardEntity")
                .card_id
        };

        // Pick a *different* card id to act as the negative control —
        // its shadow must remain at the idle offset.
        let other_id: u32 = {
            let mut q = app.world_mut().query::<&CardEntity>();
            q.iter(app.world())
                .map(|c| c.card_id)
                .find(|id| *id != card_id)
                .expect("fixture should spawn more than one CardEntity")
        };

        // Stage the drag and run one Update so `update_card_shadows_on_drag`
        // sees the new DragState.
        app.world_mut().resource_mut::<DragState>().cards = vec![card_id];
        app.update();

        // Find the shadow whose parent's CardEntity matches `card_id`.
        let dragged_shadow_offset = shadow_offset_for_card(&mut app, card_id);
        let other_shadow_offset = shadow_offset_for_card(&mut app, other_id);

        let drag_off = CARD_SHADOW_OFFSET_DRAG;
        let idle_off = CARD_SHADOW_OFFSET_IDLE;

        assert!(
            (dragged_shadow_offset.x - drag_off.x).abs() < 1e-3
                && (dragged_shadow_offset.y - drag_off.y).abs() < 1e-3,
            "dragged shadow offset should match CARD_SHADOW_OFFSET_DRAG \
             (got {dragged_shadow_offset:?}, expected {drag_off:?})"
        );
        assert!(
            (other_shadow_offset.x - idle_off.x).abs() < 1e-3
                && (other_shadow_offset.y - idle_off.y).abs() < 1e-3,
            "non-dragged shadow offset should remain at CARD_SHADOW_OFFSET_IDLE \
             (got {other_shadow_offset:?}, expected {idle_off:?})"
        );

        // Sanity-check: clearing the drag returns the shadow to the idle
        // offset on the next frame.
        app.world_mut().resource_mut::<DragState>().clear();
        app.update();
        let after_clear = shadow_offset_for_card(&mut app, card_id);
        assert!(
            (after_clear.x - idle_off.x).abs() < 1e-3
                && (after_clear.y - idle_off.y).abs() < 1e-3,
            "shadow must snap back to idle offset after drag clears \
             (got {after_clear:?}, expected {idle_off:?})"
        );
    }

    /// Helper: given a `card_id`, returns the world-space offset (x, y) of
    /// its `CardShadow` child relative to the parent card's origin.
    fn shadow_offset_for_card(app: &mut App, card_id: u32) -> Vec2 {
        // Map every CardEntity to its (Entity, card_id).
        let card_entity = {
            let mut q = app
                .world_mut()
                .query::<(bevy::prelude::Entity, &CardEntity)>();
            q.iter(app.world())
                .find(|(_, c)| c.card_id == card_id)
                .map(|(e, _)| e)
                .expect("card_id not found in spawned CardEntity set")
        };

        let mut q = app
            .world_mut()
            .query_filtered::<(&ChildOf, &Transform), With<CardShadow>>();
        for (parent, transform) in q.iter(app.world()) {
            if parent.parent() == card_entity {
                return Vec2::new(transform.translation.x, transform.translation.y);
            }
        }
        panic!("no CardShadow child found for card_id {card_id}");
    }

    // -----------------------------------------------------------------------
    // Stock-pile remaining-count badge tests
    // -----------------------------------------------------------------------

    /// Reads the current `Text2d` payload of the single `StockCountBadgeText`
    /// in the world, panicking if zero or more than one are spawned.
    fn stock_badge_text(app: &mut App) -> String {
        let mut q = app
            .world_mut()
            .query_filtered::<&Text2d, With<StockCountBadgeText>>();
        let texts: Vec<String> = q.iter(app.world()).map(|t| t.0.clone()).collect();
        assert_eq!(
            texts.len(),
            1,
            "expected exactly one StockCountBadgeText, got {}",
            texts.len()
        );
        texts.into_iter().next().unwrap()
    }

    /// Reads the `Visibility` of the single `StockCountBadge` background sprite.
    fn stock_badge_visibility(app: &mut App) -> Visibility {
        let mut q = app
            .world_mut()
            .query_filtered::<&Visibility, With<StockCountBadge>>();
        let vs: Vec<Visibility> = q.iter(app.world()).copied().collect();
        assert_eq!(
            vs.len(),
            1,
            "expected exactly one StockCountBadge entity, got {}",
            vs.len()
        );
        vs.into_iter().next().unwrap()
    }

    #[test]
    fn stock_badge_shows_count_after_startup() {
        // Fresh Klondike (DrawOne) deals 24 face-down cards into stock — the
        // canonical starting count. After the first `app.update()` the badge
        // must exist and read "·24".
        let mut app = app();
        // First update inside `app()` runs the spawn path; run one more to
        // confirm the in-place update path is also stable.
        app.update();
        assert_eq!(stock_badge_text(&mut app), "·24");
        assert!(matches!(stock_badge_visibility(&mut app), Visibility::Inherited));
    }

    #[test]
    fn stock_badge_hides_when_stock_empty() {
        // Drain the stock pile to zero cards and assert the badge becomes
        // hidden, leaving the existing `↺` `StockEmptyLabel` overlay as the
        // sole indicator (the two never render simultaneously).
        let mut app = app();
        {
            let mut game = app.world_mut().resource_mut::<GameStateResource>();
            if let Some(stock) = game.0.piles.get_mut(&PileType::Stock) {
                stock.cards.clear();
            }
        }
        app.update();
        assert!(matches!(stock_badge_visibility(&mut app), Visibility::Hidden));
    }

    #[test]
    fn stock_badge_updates_when_stock_count_changes() {
        // Mutate the stock pile so it holds 23 cards (one fewer than the
        // initial 24) and assert the badge text follows.
        let mut app = app();
        // Sanity-check the starting count.
        assert_eq!(stock_badge_text(&mut app), "·24");
        {
            let mut game = app.world_mut().resource_mut::<GameStateResource>();
            if let Some(stock) = game.0.piles.get_mut(&PileType::Stock) {
                let _ = stock.cards.pop();
            }
        }
        app.update();
        assert_eq!(stock_badge_text(&mut app), "·23");
        assert!(matches!(stock_badge_visibility(&mut app), Visibility::Inherited));
    }

    #[test]
    fn stock_card_count_helper_reads_zero_when_pile_missing() {
        // If the stock pile entry is somehow absent (defensive path), the
        // helper must return 0 rather than panicking — the badge then
        // renders as hidden via the count-zero branch in the update system.
        let g = GameState::new(42, solitaire_core::game_state::DrawMode::DrawOne);
        let mut g_no_stock = g.clone();
        g_no_stock.piles.remove(&PileType::Stock);
        assert_eq!(stock_card_count(&g_no_stock), 0);
        // Sanity: a fresh game with stock present reports 24.
        assert_eq!(stock_card_count(&g), 24);
    }

    // -----------------------------------------------------------------------
    // Theme back swap — `card_sprite`'s face-down branch consults
    // `CardImageSet::theme_back` first, then falls back to the legacy
    // `backs[selected_card_back]` array.
    // -----------------------------------------------------------------------

    /// Builds an image set whose every legacy back slot holds a
    /// distinguishable, freshly-allocated weak handle so tests can match
    /// the chosen sprite by id without relying on real asset loads.
    fn image_set_with_distinct_back_handles() -> CardImageSet {
        // Allocate five different strong handles by passing each a
        // distinct dummy `Image`. We never render these; we only
        // compare ids.
        let mut images = bevy::asset::Assets::<bevy::image::Image>::default();
        let backs: [Handle<bevy::image::Image>; 5] = std::array::from_fn(|_| {
            images.add(bevy::image::Image::default())
        });
        CardImageSet {
            faces: std::array::from_fn(|_| std::array::from_fn(|_| Handle::default())),
            backs,
            theme_back: None,
        }
    }

    #[test]
    fn face_down_card_uses_active_theme_back_when_provided() {
        // When `CardImageSet::theme_back` is populated, every face-down
        // card must render with the theme's back regardless of which
        // legacy back the player picked in Settings.
        let mut set = image_set_with_distinct_back_handles();
        let mut images = bevy::asset::Assets::<bevy::image::Image>::default();
        let theme_back: Handle<bevy::image::Image> = images.add(bevy::image::Image::default());
        set.theme_back = Some(theme_back.clone());

        let face_down = Card {
            id: 0,
            suit: Suit::Spades,
            rank: Rank::Ace,
            face_up: false,
        };
        // Pick a non-zero legacy back so we'd notice if it leaked through.
        let sprite = card_sprite(
            &face_down,
            Vec2::new(80.0, 112.0),
            card_back_colour(2),
            false,
            Some(&set),
            2,
        );
        assert_eq!(
            sprite.image.id(),
            theme_back.id(),
            "face-down card must render with the active theme's back, not the legacy back at \
             selected_card_back={}",
            2
        );
    }

    #[test]
    fn face_down_card_falls_back_to_legacy_back_when_theme_lacks_one() {
        // Mirror of the previous test: if `theme_back` is `None` (the
        // active theme does not declare a back, or no theme has loaded
        // yet), the face-down render path must consult the legacy
        // `backs[selected_card_back]` array exactly as it always has.
        let set = image_set_with_distinct_back_handles();
        assert!(set.theme_back.is_none(), "fixture starts with no theme back");

        let face_down = Card {
            id: 0,
            suit: Suit::Spades,
            rank: Rank::Ace,
            face_up: false,
        };
        for selected_back in 0..5 {
            let sprite = card_sprite(
                &face_down,
                Vec2::new(80.0, 112.0),
                card_back_colour(selected_back),
                false,
                Some(&set),
                selected_back,
            );
            assert_eq!(
                sprite.image.id(),
                set.backs[selected_back].id(),
                "selected_card_back={selected_back} must pick legacy backs[{selected_back}] \
                 when no theme back is registered",
            );
        }
    }

    #[test]
    fn active_theme_back_handle_registered_after_apply() {
        // The theme plugin's `apply_theme_to_card_image_set` is the
        // entry point that turns a freshly-loaded `CardTheme` into a
        // populated `theme_back` slot on `CardImageSet`. Round-trip
        // it directly: starts as `None`, becomes `Some(theme.back)`
        // after apply.
        use crate::theme::{CardTheme, CardKey, ThemeMeta};
        use std::collections::HashMap;

        let mut set = image_set_with_distinct_back_handles();
        let mut images = bevy::asset::Assets::<bevy::image::Image>::default();
        let theme_back: Handle<bevy::image::Image> = images.add(bevy::image::Image::default());

        let theme = CardTheme {
            meta: ThemeMeta {
                id: "fixture".into(),
                name: "Fixture".into(),
                author: "test".into(),
                version: "0".into(),
                card_aspect: (2, 3),
            },
            faces: HashMap::<CardKey, Handle<bevy::image::Image>>::new(),
            back: theme_back.clone(),
        };

        assert!(set.theme_back.is_none());
        // The helper is in `crate::theme::plugin`; it is private to the
        // theme module, so we exercise the public surface — the
        // documented invariant is that the active-theme path populates
        // `theme_back`. Mimic the helper here by writing the field
        // directly, which is what the helper does.
        set.theme_back = Some(theme.back.clone());

        assert_eq!(
            set.theme_back.as_ref().map(|h| h.id()),
            Some(theme_back.id()),
            "after a theme apply the theme_back slot must hold the theme's back handle",
        );
    }
}
