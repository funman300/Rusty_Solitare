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
use solitaire_core::game_state::GameState;
use solitaire_core::pile::PileType;

use crate::events::StateChangedEvent;
use crate::game_plugin::GameMutation;
use crate::layout::{Layout, LayoutResource};
use crate::resources::GameStateResource;

/// Fraction of card height used as vertical offset between stacked tableau cards.
const TABLEAU_FAN_FRAC: f32 = 0.25;

/// Fraction of card height used as a tiny offset between stacked cards in
/// non-tableau piles, so stacking is visible.
const STACK_FAN_FRAC: f32 = 0.003;

/// Font size as a fraction of card width.
const FONT_SIZE_FRAC: f32 = 0.28;

const CARD_FACE_COLOUR: Color = Color::srgb(0.98, 0.98, 0.95);
const CARD_BACK_COLOUR: Color = Color::srgb(0.15, 0.30, 0.55);
const RED_SUIT_COLOUR: Color = Color::srgb(0.78, 0.12, 0.15);
const BLACK_SUIT_COLOUR: Color = Color::srgb(0.08, 0.08, 0.08);

/// Marker component linking a Bevy entity to a `solitaire_core::Card::id`.
#[derive(Component, Debug, Clone, Copy)]
pub struct CardEntity {
    pub card_id: u32,
}

/// Marker for the text child inside a card entity.
#[derive(Component, Debug)]
pub struct CardLabel;

/// Renders cards by reading `GameStateResource` on `StateChangedEvent`.
pub struct CardPlugin;

impl Plugin for CardPlugin {
    fn build(&self, app: &mut App) {
        // PostStartup ensures TablePlugin's Startup system has inserted
        // LayoutResource before we try to read it.
        app.add_systems(PostStartup, sync_cards_startup)
            .add_systems(Update, sync_cards_on_change.after(GameMutation));
    }
}

/// Render the initial deal. Runs in `PostStartup`, so all `Startup` systems
/// (including `TablePlugin::setup_table` which inserts `LayoutResource`)
/// have already completed.
fn sync_cards_startup(
    commands: Commands,
    game: Res<GameStateResource>,
    layout: Option<Res<LayoutResource>>,
    entities: Query<(Entity, &CardEntity)>,
) {
    if let Some(layout) = layout {
        sync_cards(commands, &game.0, &layout.0, &entities);
    }
}

fn sync_cards_on_change(
    mut events: EventReader<StateChangedEvent>,
    commands: Commands,
    game: Res<GameStateResource>,
    layout: Option<Res<LayoutResource>>,
    entities: Query<(Entity, &CardEntity)>,
) {
    if events.read().next().is_none() {
        return;
    }
    if let Some(layout) = layout {
        sync_cards(commands, &game.0, &layout.0, &entities);
    }
}

fn sync_cards(
    mut commands: Commands,
    game: &GameState,
    layout: &Layout,
    entities: &Query<(Entity, &CardEntity)>,
) {
    let positions = card_positions(game, layout);

    // Map card_id -> Entity for in-place updates.
    let mut existing: HashMap<u32, Entity> = HashMap::new();
    for (entity, marker) in entities.iter() {
        existing.insert(marker.card_id, entity);
    }

    let live_ids: HashSet<u32> = positions.iter().map(|(c, _, _)| c.id).collect();

    // Despawn any entity whose card is no longer tracked.
    for (card_id, entity) in &existing {
        if !live_ids.contains(card_id) {
            commands.entity(*entity).despawn_recursive();
        }
    }

    // For each card in the current state: spawn or update its entity.
    for (card, position, z) in positions {
        match existing.get(&card.id) {
            Some(&entity) => update_card_entity(&mut commands, entity, &card, position, z, layout),
            None => spawn_card_entity(&mut commands, &card, position, z, layout),
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
        let fan_y = if is_tableau {
            -layout.card_size.y * TABLEAU_FAN_FRAC
        } else {
            0.0
        };

        for (i, card) in pile.cards.iter().enumerate() {
            let pos = Vec2::new(base.x, base.y + fan_y * i as f32);
            let z = 1.0 + (i as f32) * STACK_FAN_FRAC;
            out.push((card.clone(), pos, z));
        }
    }
    out
}

fn spawn_card_entity(commands: &mut Commands, card: &Card, pos: Vec2, z: f32, layout: &Layout) {
    let body_colour = if card.face_up {
        CARD_FACE_COLOUR
    } else {
        CARD_BACK_COLOUR
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

fn update_card_entity(
    commands: &mut Commands,
    entity: Entity,
    card: &Card,
    pos: Vec2,
    z: f32,
    layout: &Layout,
) {
    let body_colour = if card.face_up {
        CARD_FACE_COLOUR
    } else {
        CARD_BACK_COLOUR
    };

    commands.entity(entity).insert((
        Sprite {
            color: body_colour,
            custom_size: Some(layout.card_size),
            ..default()
        },
        Transform::from_xyz(pos.x, pos.y, z),
    ));

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
    fn card_positions_includes_all_52_cards() {
        let g = GameState::new(42, solitaire_core::game_state::DrawMode::DrawOne);
        let layout =
            crate::layout::compute_layout(Vec2::new(1280.0, 800.0));
        let positions = card_positions(&g, &layout);
        assert_eq!(positions.len(), 52);
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
}
