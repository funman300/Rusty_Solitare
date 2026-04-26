//! Smooth animations: card slide (linear lerp), win cascade, achievement toast.
//!
//! `CardAnim` is the only animation component used by other plugins — import
//! it directly when adding animations outside this file.

use bevy::prelude::*;

use crate::achievement_plugin::display_name_for;
use crate::card_plugin::CardEntity;
use crate::challenge_plugin::ChallengeAdvancedEvent;
use crate::daily_challenge_plugin::DailyChallengeCompletedEvent;
use crate::events::{AchievementUnlockedEvent, GameWonEvent};
use crate::game_plugin::GameMutation;
use crate::layout::LayoutResource;
use crate::progress_plugin::LevelUpEvent;
use crate::settings_plugin::SettingsChangedEvent;
use crate::time_attack_plugin::TimeAttackEndedEvent;
use crate::weekly_goals_plugin::WeeklyGoalCompletedEvent;

/// Duration of a card slide (move) animation in seconds.
pub const SLIDE_SECS: f32 = 0.15;

const WIN_TOAST_SECS: f32 = 4.0;
const ACHIEVEMENT_TOAST_SECS: f32 = 3.0;
const LEVELUP_TOAST_SECS: f32 = 3.0;
const DAILY_TOAST_SECS: f32 = 3.0;
const WEEKLY_TOAST_SECS: f32 = 3.0;
const TIME_ATTACK_TOAST_SECS: f32 = 5.0;
const CHALLENGE_TOAST_SECS: f32 = 3.0;
const VOLUME_TOAST_SECS: f32 = 1.4;
const CASCADE_STAGGER: f32 = 0.05;
const CASCADE_DURATION: f32 = 0.5;

/// Linear-lerp slide animation.
///
/// After `delay` seconds the card moves from `start` to `target` over
/// `duration` seconds. The component removes itself when the slide completes.
#[derive(Component, Debug, Clone)]
pub struct CardAnim {
    pub start: Vec3,
    pub target: Vec3,
    pub elapsed: f32,
    pub duration: f32,
    /// Additional wait before the slide begins.
    pub delay: f32,
}

/// Marker on a toast overlay UI node.
#[derive(Component, Debug)]
pub struct ToastOverlay;

/// Auto-dismiss countdown (seconds remaining). Attached to toast entities.
#[derive(Component, Debug)]
pub struct ToastTimer(pub f32);

pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut App) {
        // Register the events this plugin consumes so tests that don't include
        // GamePlugin can still run AnimationPlugin in isolation. Double-registration
        // is idempotent in Bevy.
        app.add_event::<GameWonEvent>()
            .add_event::<AchievementUnlockedEvent>()
            .add_event::<LevelUpEvent>()
            .add_event::<DailyChallengeCompletedEvent>()
            .add_event::<WeeklyGoalCompletedEvent>()
            .add_event::<TimeAttackEndedEvent>()
            .add_event::<ChallengeAdvancedEvent>()
            .add_event::<SettingsChangedEvent>()
            .add_systems(
                Update,
                (
                    advance_card_anims,
                    handle_win_cascade,
                    handle_achievement_toast,
                    handle_levelup_toast,
                    handle_daily_toast,
                    handle_weekly_toast,
                    handle_time_attack_toast,
                    handle_challenge_toast,
                    handle_settings_toast,
                    tick_toasts,
                )
                    .after(GameMutation),
            );
    }
}

fn advance_card_anims(
    mut commands: Commands,
    time: Res<Time>,
    mut anims: Query<(Entity, &mut Transform, &mut CardAnim)>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut anim) in &mut anims {
        if anim.delay > 0.0 {
            anim.delay = (anim.delay - dt).max(0.0);
            continue;
        }
        anim.elapsed += dt;
        let t = (anim.elapsed / anim.duration).min(1.0);
        transform.translation = anim.start.lerp(anim.target, t);
        if t >= 1.0 {
            commands.entity(entity).remove::<CardAnim>();
        }
    }
}

fn handle_win_cascade(
    mut commands: Commands,
    mut events: EventReader<GameWonEvent>,
    cards: Query<(Entity, &Transform), With<CardEntity>>,
    layout: Option<Res<LayoutResource>>,
) {
    if events.read().next().is_none() {
        return;
    }

    let margin = layout.as_ref().map_or(800.0, |l| l.0.card_size.x * 8.0);

    // Eight off-screen destinations spread around the window edges.
    let targets: [Vec3; 8] = [
        Vec3::new(margin, margin, 300.0),
        Vec3::new(-margin, margin, 300.0),
        Vec3::new(margin, -margin, 300.0),
        Vec3::new(-margin, -margin, 300.0),
        Vec3::new(0.0, margin, 300.0),
        Vec3::new(0.0, -margin, 300.0),
        Vec3::new(margin, 0.0, 300.0),
        Vec3::new(-margin, 0.0, 300.0),
    ];

    spawn_toast(&mut commands, "You Win!".to_string(), WIN_TOAST_SECS);

    for (i, (entity, transform)) in cards.iter().enumerate() {
        commands.entity(entity).insert(CardAnim {
            start: transform.translation,
            target: targets[i % 8],
            elapsed: 0.0,
            duration: CASCADE_DURATION,
            delay: i as f32 * CASCADE_STAGGER,
        });
    }
}

fn handle_achievement_toast(
    mut commands: Commands,
    mut events: EventReader<AchievementUnlockedEvent>,
) {
    for ev in events.read() {
        spawn_toast(
            &mut commands,
            format!("Achievement: {}", display_name_for(&ev.0)),
            ACHIEVEMENT_TOAST_SECS,
        );
    }
}

fn handle_levelup_toast(mut commands: Commands, mut events: EventReader<LevelUpEvent>) {
    for ev in events.read() {
        spawn_toast(
            &mut commands,
            format!("Level Up! → {}", ev.new_level),
            LEVELUP_TOAST_SECS,
        );
    }
}

fn handle_daily_toast(
    mut commands: Commands,
    mut events: EventReader<DailyChallengeCompletedEvent>,
) {
    for ev in events.read() {
        spawn_toast(
            &mut commands,
            format!("Daily Challenge Complete! (Streak: {})", ev.streak),
            DAILY_TOAST_SECS,
        );
    }
}

fn handle_weekly_toast(
    mut commands: Commands,
    mut events: EventReader<WeeklyGoalCompletedEvent>,
) {
    for ev in events.read() {
        spawn_toast(
            &mut commands,
            format!("Weekly Goal: {}", ev.description),
            WEEKLY_TOAST_SECS,
        );
    }
}

fn handle_time_attack_toast(
    mut commands: Commands,
    mut events: EventReader<TimeAttackEndedEvent>,
) {
    for ev in events.read() {
        spawn_toast(
            &mut commands,
            format!("Time Attack: {} win{}", ev.wins, if ev.wins == 1 { "" } else { "s" }),
            TIME_ATTACK_TOAST_SECS,
        );
    }
}

fn handle_challenge_toast(
    mut commands: Commands,
    mut events: EventReader<ChallengeAdvancedEvent>,
) {
    for ev in events.read() {
        spawn_toast(
            &mut commands,
            format!("Challenge {} cleared!", ev.previous_index.saturating_add(1)),
            CHALLENGE_TOAST_SECS,
        );
    }
}

fn handle_settings_toast(
    mut commands: Commands,
    mut events: EventReader<SettingsChangedEvent>,
) {
    for ev in events.read() {
        let pct = (ev.0.sfx_volume * 100.0).round() as i32;
        spawn_toast(
            &mut commands,
            format!("SFX: {pct}%"),
            VOLUME_TOAST_SECS,
        );
    }
}

fn tick_toasts(
    mut commands: Commands,
    time: Res<Time>,
    mut toasts: Query<(Entity, &mut ToastTimer)>,
) {
    let dt = time.delta_secs();
    for (entity, mut timer) in &mut toasts {
        timer.0 -= dt;
        if timer.0 <= 0.0 {
            commands.entity(entity).despawn_recursive();
        }
    }
}

fn spawn_toast(commands: &mut Commands, message: String, duration_secs: f32) {
    commands
        .spawn((
            ToastOverlay,
            ToastTimer(duration_secs),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(25.0),
                top: Val::Percent(42.0),
                width: Val::Percent(50.0),
                padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.72)),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(message),
                TextFont {
                    font_size: 32.0,
                    ..default()
                },
                TextColor(Color::srgb(1.0, 0.87, 0.0)),
            ));
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card_plugin::CardPlugin;
    use crate::game_plugin::GamePlugin;
    use crate::table_plugin::TablePlugin;

    fn app_with_anim() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(GamePlugin)
            .add_plugins(TablePlugin)
            .add_plugins(CardPlugin)
            .add_plugins(AnimationPlugin);
        app.update(); // PostStartup: spawns cards
        app
    }

    #[test]
    fn card_anim_at_half_elapsed_reaches_midpoint() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AnimationPlugin);

        let start = Vec3::ZERO;
        let target = Vec3::new(100.0, 0.0, 0.0);
        // elapsed = 0.5, duration = 1.0 → t = 0.5 even when dt=0
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(start),
                CardAnim { start, target, elapsed: 0.5, duration: 1.0, delay: 0.0 },
            ))
            .id();

        app.update();

        let pos = app.world().entity(entity).get::<Transform>().unwrap().translation;
        assert!((pos.x - 50.0).abs() < 1e-3, "expected midpoint x=50, got {}", pos.x);
        assert!(
            app.world().entity(entity).get::<CardAnim>().is_some(),
            "animation not yet complete"
        );
    }

    #[test]
    fn card_anim_removed_and_at_target_when_elapsed_equals_duration() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AnimationPlugin);

        let target = Vec3::new(10.0, 0.0, 0.0);
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(Vec3::ZERO),
                CardAnim { start: Vec3::ZERO, target, elapsed: 1.0, duration: 1.0, delay: 0.0 },
            ))
            .id();

        app.update();

        assert!(
            app.world().entity(entity).get::<CardAnim>().is_none(),
            "CardAnim should be removed when done"
        );
        let pos = app.world().entity(entity).get::<Transform>().unwrap().translation;
        assert!((pos.x - 10.0).abs() < 1e-3);
    }

    #[test]
    fn card_anim_does_not_move_during_delay() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AnimationPlugin);

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(Vec3::ZERO),
                CardAnim {
                    start: Vec3::ZERO,
                    target: Vec3::new(100.0, 0.0, 0.0),
                    elapsed: 0.0,
                    duration: 0.15,
                    delay: 100.0, // large delay — card must not move
                },
            ))
            .id();

        app.update();

        let pos = app.world().entity(entity).get::<Transform>().unwrap().translation;
        assert!(pos.x.abs() < 1e-3, "card must not move during delay period");
    }

    #[test]
    fn win_cascade_adds_anim_to_all_52_cards() {
        let mut app = app_with_anim();

        let before = app
            .world_mut()
            .query::<&CardAnim>()
            .iter(app.world())
            .count();
        assert_eq!(before, 0, "no animations before win");

        app.world_mut()
            .send_event(GameWonEvent { score: 500, time_seconds: 60 });
        app.update();

        let after = app
            .world_mut()
            .query::<&CardAnim>()
            .iter(app.world())
            .count();
        assert_eq!(after, 52, "all 52 cards should have cascade animations");
    }
}
