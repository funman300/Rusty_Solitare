//! Smooth animations: card slide (linear lerp), win cascade, achievement toast.
//!
//! `CardAnim` is the only animation component used by other plugins — import
//! it directly when adding animations outside this file.

use bevy::prelude::*;
use solitaire_data::AnimSpeed;

use crate::achievement_plugin::display_name_for;
use crate::auto_complete_plugin::AutoCompleteState;
use crate::card_plugin::CardEntity;
use crate::challenge_plugin::ChallengeAdvancedEvent;
use crate::daily_challenge_plugin::{DailyChallengeCompletedEvent, DailyGoalAnnouncementEvent};
use crate::events::{InfoToastEvent, NewGameConfirmEvent, XpAwardedEvent};
use crate::events::{AchievementUnlockedEvent, GameWonEvent};
use crate::game_plugin::GameMutation;
use crate::layout::LayoutResource;
use crate::progress_plugin::LevelUpEvent;
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource};
use crate::time_attack_plugin::TimeAttackEndedEvent;
use crate::weekly_goals_plugin::WeeklyGoalCompletedEvent;

/// Duration of a card slide (move) animation in seconds at Normal speed.
pub const SLIDE_SECS: f32 = 0.15;

/// The effective slide duration, updated whenever `Settings::animation_speed` changes.
#[derive(Resource, Debug, Clone, Copy)]
pub struct EffectiveSlideDuration {
    pub slide_secs: f32,
}

impl Default for EffectiveSlideDuration {
    fn default() -> Self {
        Self { slide_secs: SLIDE_SECS }
    }
}

fn anim_speed_to_secs(speed: &AnimSpeed) -> f32 {
    match speed {
        AnimSpeed::Normal => SLIDE_SECS,
        AnimSpeed::Fast => 0.07,
        AnimSpeed::Instant => 0.0,
    }
}

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
            .add_event::<DailyGoalAnnouncementEvent>()
            .add_event::<WeeklyGoalCompletedEvent>()
            .add_event::<TimeAttackEndedEvent>()
            .add_event::<ChallengeAdvancedEvent>()
            .add_event::<SettingsChangedEvent>()
            .add_event::<NewGameConfirmEvent>()
            .add_event::<InfoToastEvent>()
            .add_event::<XpAwardedEvent>()
            .init_resource::<EffectiveSlideDuration>()
            .add_systems(Startup, init_slide_duration)
            .add_systems(
                Update,
                (
                    advance_card_anims,
                    sync_slide_duration,
                    handle_win_cascade,
                    handle_achievement_toast,
                    handle_levelup_toast,
                    handle_daily_goal_announcement_toast,
                    handle_daily_toast,
                    handle_weekly_toast,
                    handle_time_attack_toast,
                    handle_challenge_toast,
                    handle_settings_toast,
                    handle_auto_complete_toast,
                    handle_new_game_confirm_toast,
                    handle_info_toast,
                    handle_xp_awarded_toast,
                    tick_toasts,
                )
                    .after(GameMutation),
            );
    }
}

fn init_slide_duration(
    settings: Option<Res<SettingsResource>>,
    mut dur: ResMut<EffectiveSlideDuration>,
) {
    if let Some(s) = settings {
        dur.slide_secs = anim_speed_to_secs(&s.0.animation_speed);
    }
}

fn sync_slide_duration(
    mut events: EventReader<SettingsChangedEvent>,
    mut dur: ResMut<EffectiveSlideDuration>,
) {
    for ev in events.read() {
        dur.slide_secs = anim_speed_to_secs(&ev.0.animation_speed);
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
    let Some(ev) = events.read().next() else {
        return;
    };

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

    let m = ev.time_seconds / 60;
    let s = ev.time_seconds % 60;
    let win_msg = format!("You Win!  Score: {}  Time: {m}:{s:02}", ev.score);
    spawn_toast(&mut commands, win_msg, WIN_TOAST_SECS);

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
            format!("Achievement: {}", display_name_for(&ev.0.id)),
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

fn handle_daily_goal_announcement_toast(
    mut commands: Commands,
    mut events: EventReader<DailyGoalAnnouncementEvent>,
) {
    for ev in events.read() {
        spawn_toast(&mut commands, format!("Goal: {}", ev.0), DAILY_TOAST_SECS);
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
    mut last_sfx: Local<Option<f32>>,
    mut last_music: Local<Option<f32>>,
) {
    for ev in events.read() {
        let sfx = ev.0.sfx_volume;
        let music = ev.0.music_volume;
        let sfx_changed = last_sfx.is_none_or(|prev| (prev - sfx).abs() > f32::EPSILON);
        let music_changed = last_music.is_none_or(|prev| (prev - music).abs() > f32::EPSILON);
        *last_sfx = Some(sfx);
        *last_music = Some(music);
        if sfx_changed {
            let pct = (sfx * 100.0).round() as i32;
            spawn_toast(&mut commands, format!("SFX: {pct}%"), VOLUME_TOAST_SECS);
        }
        if music_changed {
            let pct = (music * 100.0).round() as i32;
            spawn_toast(&mut commands, format!("Music: {pct}%"), VOLUME_TOAST_SECS);
        }
    }
}

/// Shows a one-time "Auto-completing..." toast when auto-complete activates.
fn handle_auto_complete_toast(
    mut commands: Commands,
    state: Option<Res<AutoCompleteState>>,
    mut shown: Local<bool>,
) {
    let Some(s) = state else { return };
    if s.is_changed() {
        if s.active {
            if !*shown {
                *shown = true;
                spawn_toast(&mut commands, "Auto-completing…".to_string(), 2.0);
            }
        } else {
            *shown = false;
        }
    }
}

fn handle_new_game_confirm_toast(
    mut commands: Commands,
    mut events: EventReader<NewGameConfirmEvent>,
) {
    for _ in events.read() {
        spawn_toast(&mut commands, "Press N again to start a new game".to_string(), 3.0);
    }
}

fn handle_info_toast(mut commands: Commands, mut events: EventReader<InfoToastEvent>) {
    for ev in events.read() {
        spawn_toast(&mut commands, ev.0.clone(), 3.0);
    }
}

fn handle_xp_awarded_toast(mut commands: Commands, mut events: EventReader<XpAwardedEvent>) {
    for ev in events.read() {
        spawn_toast(&mut commands, format!("+{} XP", ev.amount), 3.0);
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
    fn anim_speed_fast_is_less_than_normal() {
        assert!(anim_speed_to_secs(&AnimSpeed::Fast) < anim_speed_to_secs(&AnimSpeed::Normal));
    }

    #[test]
    fn anim_speed_instant_is_zero() {
        assert_eq!(anim_speed_to_secs(&AnimSpeed::Instant), 0.0);
    }

    #[test]
    fn toast_dismissed_after_timer_reaches_zero() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AnimationPlugin);

        // Manually spawn a toast with a timer that's already expired.
        app.world_mut().spawn((ToastOverlay, ToastTimer(-0.001)));
        app.update();

        // The toast entity must have been despawned.
        let remaining = app
            .world_mut()
            .query::<&ToastTimer>()
            .iter(app.world())
            .count();
        assert_eq!(remaining, 0, "expired toast must be despawned");
    }

    #[test]
    fn toast_not_dismissed_before_timer_reaches_zero() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AnimationPlugin);

        // Large positive timer — should survive one update.
        app.world_mut().spawn((ToastOverlay, ToastTimer(100.0)));
        app.update();

        let remaining = app
            .world_mut()
            .query::<&ToastTimer>()
            .iter(app.world())
            .count();
        assert_eq!(remaining, 1, "unexpired toast must not be despawned");
    }

    #[test]
    fn info_toast_event_spawns_toast_overlay() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AnimationPlugin);

        app.world_mut().send_event(InfoToastEvent("hello".to_string()));
        app.update();

        let count = app
            .world_mut()
            .query::<&ToastOverlay>()
            .iter(app.world())
            .count();
        assert_eq!(count, 1, "InfoToastEvent must spawn exactly one ToastOverlay");
    }

    #[test]
    fn settings_changed_event_updates_slide_duration() {
        use solitaire_data::Settings;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AnimationPlugin);

        let mut fast_settings = Settings::default();
        fast_settings.animation_speed = AnimSpeed::Fast;
        app.world_mut().send_event(SettingsChangedEvent(fast_settings));
        app.update();

        let dur = app.world().resource::<EffectiveSlideDuration>().slide_secs;
        assert!((dur - anim_speed_to_secs(&AnimSpeed::Fast)).abs() < 1e-6);
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
