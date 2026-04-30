//! Launch splash overlay.
//!
//! On app start the engine spawns a fullscreen, high-Z overlay that
//! reads "Solitaire Quest" in the project font for ~1.6 s
//! (300 ms fade-in, ~1 s hold, 300 ms fade-out), then despawns. The
//! existing deal animation plays *behind* the splash during the hold —
//! the user sees the dealt board appear as the splash dissolves.
//!
//! ## Why an overlay instead of an `AppState`
//!
//! Every existing plugin in this engine runs unconditionally on
//! `Startup`/`Update`; gating them with `run_if(in_state(...))` would be
//! a sweeping refactor for a one-off brand beat. The splash instead
//! sits on top of `Z_SPLASH` (above tooltips, focus ring, and toasts)
//! while the rest of the game runs normally beneath it. The handoff is
//! intentional: the user finishes the splash and the dealt board is
//! already there.
//!
//! ## Dismissal
//!
//! Any keypress, mouse click, or touch begin shortcuts the splash to its
//! fade-out window — never to an instant despawn, so the dissolve still
//! plays for visual continuity. The dismiss input is **not** consumed,
//! so a player who instinctively taps Space to "skip the intro" still
//! gets their stock draw the moment the splash clears (Space and most
//! other gameplay keys read `just_pressed`, which by the next tick is
//! already false — splash dismissal happens on the same tick as the
//! press, so downstream gameplay handlers see exactly the keystroke
//! they would have seen with no splash).
//!
//! ## Headless tests
//!
//! Under `MinimalPlugins + SplashPlugin`, the `Time<Virtual>` clock
//! clamps each tick to `max_delta` (default 250 ms) regardless of the
//! `TimeUpdateStrategy::ManualDuration` value, so tests advance time in
//! 200 ms ticks and call `app.update()` enough times to cross the
//! desired threshold (same approach used by `ui_tooltip::tests`).

use std::time::Duration;

use bevy::input::touch::Touches;
use bevy::prelude::*;

use crate::font_plugin::FontResource;
use crate::ui_theme::{
    ACCENT_PRIMARY, BG_BASE, MOTION_SPLASH_FADE_SECS, MOTION_SPLASH_TOTAL_SECS, TEXT_SECONDARY,
    TYPE_CAPTION, TYPE_DISPLAY, VAL_SPACE_2, Z_SPLASH,
};

// ---------------------------------------------------------------------------
// Public plugin
// ---------------------------------------------------------------------------

/// Drives the launch splash overlay. Add this plugin once at app start;
/// the splash spawns during `Startup`, fades in/out over
/// [`MOTION_SPLASH_TOTAL_SECS`], and despawns itself.
///
/// The overlay is a sibling of every other UI surface — it never
/// becomes a parent of game systems, and the deal animation runs
/// underneath it during the hold window. Dismissal on any keypress /
/// click / touch shortcuts the timeline into the fade-out phase rather
/// than despawning instantly, so the dissolve always plays.
pub struct SplashPlugin;

impl Plugin for SplashPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_splash).add_systems(
            Update,
            (dismiss_splash_on_input, advance_splash).chain(),
        );
    }
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Marker on the splash overlay scrim (root entity for the launch beat).
/// Despawned with descendants once [`MOTION_SPLASH_TOTAL_SECS`] elapses
/// or once a user-input dismissal advances the timeline past the hold.
#[derive(Component, Debug)]
pub struct SplashRoot;

/// Tracks the splash's elapsed visible duration. Stored as a component
/// on the splash root rather than a global resource so despawning the
/// splash root removes its state along with it — there's no second-run
/// concern (the splash is one-shot at app start) and a component keeps
/// the splash data co-located with its entity.
#[derive(Component, Debug, Default)]
pub struct SplashAge(pub Duration);

/// Marker on the splash title text. Used by [`advance_splash`] to write
/// the per-frame alpha into the text colour without walking arbitrary
/// children.
#[derive(Component, Debug)]
struct SplashTitle;

/// Marker on the splash subtitle text (build version). Faded together
/// with the title so the brand beat dissolves as a single layer.
#[derive(Component, Debug)]
struct SplashSubtitle;

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Spawns the splash overlay at `Startup`. Builds a fullscreen scrim
/// at full alpha (the first `advance_splash` tick will overwrite the
/// alpha based on age), centres a "Solitaire Quest" title in
/// [`ACCENT_PRIMARY`], and pins a small build-version line below.
fn spawn_splash(mut commands: Commands, font_res: Option<Res<FontResource>>) {
    let font_handle = font_res.map(|f| f.0.clone()).unwrap_or_default();
    let title_font = TextFont {
        font: font_handle.clone(),
        font_size: TYPE_DISPLAY,
        ..default()
    };
    let subtitle_font = TextFont {
        font: font_handle,
        font_size: TYPE_CAPTION,
        ..default()
    };

    // Initial alpha is 0 (fade-in starts at 0 and grows). Without this
    // the first frame would flash full-opacity scrim before the
    // `advance_splash` tick lerped it down — visually a pop on slower
    // start-ups.
    let mut initial_bg = BG_BASE;
    initial_bg.set_alpha(0.0);
    let mut initial_title = ACCENT_PRIMARY;
    initial_title.set_alpha(0.0);
    let mut initial_subtitle = TEXT_SECONDARY;
    initial_subtitle.set_alpha(0.0);

    commands
        .spawn((
            SplashRoot,
            SplashAge(Duration::ZERO),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: VAL_SPACE_2,
                ..default()
            },
            BackgroundColor(initial_bg),
            GlobalZIndex(Z_SPLASH),
        ))
        .with_children(|root| {
            root.spawn((
                SplashTitle,
                Text::new("Solitaire Quest"),
                title_font,
                TextColor(initial_title),
            ));
            root.spawn((
                SplashSubtitle,
                Text::new(format!("v{}", env!("CARGO_PKG_VERSION"))),
                subtitle_font,
                TextColor(initial_subtitle),
            ));
        });
}

/// Computes the splash's per-frame alpha from its age. Three phases:
///
/// * `0..fade` — fade-in: `alpha = age / fade`.
/// * `fade..total - fade` — hold: `alpha = 1.0`.
/// * `total - fade..total` — fade-out: `alpha = (total - age) / fade`.
/// * `>= total` — splash is complete; caller despawns the root.
///
/// Returns `None` once the timeline is finished, signalling the splash
/// should be despawned.
fn splash_alpha(age: Duration) -> Option<f32> {
    let age_s = age.as_secs_f32();
    let total = MOTION_SPLASH_TOTAL_SECS;
    let fade = MOTION_SPLASH_FADE_SECS;

    if age_s >= total {
        return None;
    }
    if age_s < fade {
        // Fade-in.
        return Some((age_s / fade).clamp(0.0, 1.0));
    }
    if age_s < total - fade {
        // Hold.
        return Some(1.0);
    }
    // Fade-out.
    Some(((total - age_s) / fade).clamp(0.0, 1.0))
}

/// Advances every splash root's age by `time.delta()` and updates the
/// scrim + text alpha, despawning the splash once the timeline
/// finishes. Despawns with descendants so the title and subtitle leave
/// the world together.
fn advance_splash(
    mut commands: Commands,
    time: Res<Time>,
    mut roots: Query<(Entity, &mut SplashAge, &mut BackgroundColor, &Children), With<SplashRoot>>,
    mut titles: Query<&mut TextColor, (With<SplashTitle>, Without<SplashSubtitle>)>,
    mut subtitles: Query<&mut TextColor, (With<SplashSubtitle>, Without<SplashTitle>)>,
) {
    for (entity, mut age, mut bg, children) in &mut roots {
        age.0 = age.0.saturating_add(time.delta());
        let Some(alpha) = splash_alpha(age.0) else {
            commands.entity(entity).despawn();
            continue;
        };

        // Scrim alpha — keeps BG_BASE's RGB and just rewrites alpha.
        let mut scrim = BG_BASE;
        scrim.set_alpha(alpha);
        bg.0 = scrim;

        // Walk the splash root's direct children for the title /
        // subtitle markers and update their alpha. The hierarchy is
        // shallow (root → 2 text children) so a small loop is fine.
        for child in children.iter() {
            if let Ok(mut color) = titles.get_mut(child) {
                let mut c = ACCENT_PRIMARY;
                c.set_alpha(alpha);
                color.0 = c;
                continue;
            }
            if let Ok(mut color) = subtitles.get_mut(child) {
                let mut c = TEXT_SECONDARY;
                c.set_alpha(alpha);
                color.0 = c;
            }
        }
    }
}

/// Dismisses the splash on any user input. Accelerates each splash
/// root's age into the fade-out window so the dissolve still plays
/// (despawning instantly would feel abrupt). If the timeline is
/// already inside fade-out, the splash is left to finish on its own.
///
/// **Input is not consumed.** The splash neither calls
/// `clear_just_pressed` nor drains the touch / mouse buffers, so a
/// keystroke that dismissed the splash also reaches downstream
/// systems on the same tick (e.g. Space → `DrawRequestEvent`). This
/// matches what the user expects — the splash is a brand beat, not a
/// modal stop sign.
fn dismiss_splash_on_input(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Option<Res<Touches>>,
    mut roots: Query<&mut SplashAge, With<SplashRoot>>,
) {
    if roots.is_empty() {
        return;
    }

    let touch_pressed = touches
        .map(|t| t.iter_just_pressed().next().is_some())
        .unwrap_or(false);
    let dismissed = keys.get_just_pressed().next().is_some()
        || mouse.get_just_pressed().next().is_some()
        || touch_pressed;

    if !dismissed {
        return;
    }

    // Jump the age forward to the start of the fade-out so the
    // overlay dissolves cleanly. Saturating arithmetic on Duration
    // means an already-past-fade-out splash stays past fade-out.
    let fade_out_start = Duration::from_secs_f32(
        (MOTION_SPLASH_TOTAL_SECS - MOTION_SPLASH_FADE_SECS).max(0.0),
    );
    for mut age in &mut roots {
        if age.0 < fade_out_start {
            age.0 = fade_out_start;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;

    /// Builds a headless `App` with `MinimalPlugins + SplashPlugin` and
    /// runs one tick so `spawn_splash` (Startup) has executed before
    /// the first asserting `update`.
    fn headless_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(SplashPlugin);
        app.init_resource::<ButtonInput<KeyCode>>();
        app.init_resource::<ButtonInput<MouseButton>>();
        app.update();
        app
    }

    /// Tells `TimePlugin` to advance the virtual clock by `secs` on the
    /// next `app.update()`. Mirrors the helper in `ui_tooltip::tests`.
    fn set_manual_time_step(app: &mut App, secs: f32) {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            Duration::from_secs_f32(secs),
        ));
    }

    /// `Time<Virtual>` clamps per-tick deltas to `max_delta` (default
    /// 250 ms) regardless of the requested manual step, so we drive
    /// 200 ms ticks and call `update` enough times to exceed the target
    /// duration. Returns the splash root's recorded age after the
    /// stepping completes (or `None` if the splash was despawned).
    fn advance_by(app: &mut App, total_secs: f32) -> Option<Duration> {
        set_manual_time_step(app, 0.2);
        let ticks = (total_secs / 0.2).ceil() as usize + 1;
        for _ in 0..ticks {
            app.update();
        }
        let mut q = app
            .world_mut()
            .query_filtered::<&SplashAge, With<SplashRoot>>();
        q.iter(app.world()).next().map(|a| a.0)
    }

    fn count_splash_roots(app: &mut App) -> usize {
        app.world_mut()
            .query_filtered::<Entity, With<SplashRoot>>()
            .iter(app.world())
            .count()
    }

    fn press_key(app: &mut App, key: KeyCode) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        input.release_all();
        input.clear();
        input.press(key);
    }

    fn press_mouse(app: &mut App, button: MouseButton) {
        let mut input = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        input.release_all();
        input.clear();
        input.press(button);
    }

    /// Reads the splash scrim's `BackgroundColor` alpha. Panics if the
    /// splash root is missing — that's a regression in `spawn_splash`.
    fn scrim_alpha(app: &mut App) -> f32 {
        let mut q = app
            .world_mut()
            .query_filtered::<&BackgroundColor, With<SplashRoot>>();
        q.iter(app.world())
            .next()
            .expect("SplashRoot should exist")
            .0
            .alpha()
    }

    #[test]
    fn splash_spawns_on_startup() {
        let mut app = headless_app();
        assert_eq!(
            count_splash_roots(&mut app),
            1,
            "SplashRoot must exist after Startup"
        );
    }

    #[test]
    fn splash_despawns_after_total_duration() {
        let mut app = headless_app();
        // Comfortably past the total duration to absorb the
        // ManualDuration → Virtual-clock clamp + the despawn lag of
        // one extra tick.
        let _ = advance_by(&mut app, MOTION_SPLASH_TOTAL_SECS + 0.5);
        assert_eq!(
            count_splash_roots(&mut app),
            0,
            "SplashRoot must be despawned after MOTION_SPLASH_TOTAL_SECS"
        );
    }

    #[test]
    fn splash_alpha_curves_through_fade_hold_fade() {
        // Pure-function test on the curve so we don't need to wrangle
        // the virtual-clock clamp here. The integration assertion below
        // (`splash_dismisses_immediately_on_keypress`) covers the
        // wired-up version.
        // Start of fade-in.
        assert!(
            splash_alpha(Duration::ZERO).unwrap() < 0.05,
            "alpha at t=0 must be near 0 (fade-in start)"
        );
        // End of fade-in.
        let after_fade_in = Duration::from_secs_f32(MOTION_SPLASH_FADE_SECS);
        assert!(
            (splash_alpha(after_fade_in).unwrap() - 1.0).abs() < 0.001,
            "alpha at end of fade-in must be ~1.0"
        );
        // Mid-hold.
        let mid_hold = Duration::from_secs_f32(MOTION_SPLASH_TOTAL_SECS / 2.0);
        assert!(
            (splash_alpha(mid_hold).unwrap() - 1.0).abs() < f32::EPSILON,
            "alpha mid-hold must be exactly 1.0"
        );
        // Inside fade-out.
        let mid_fade_out = Duration::from_secs_f32(
            MOTION_SPLASH_TOTAL_SECS - MOTION_SPLASH_FADE_SECS / 2.0,
        );
        let mid_out_alpha = splash_alpha(mid_fade_out).unwrap();
        assert!(
            mid_out_alpha < 0.6 && mid_out_alpha > 0.4,
            "alpha mid-fade-out should be ~0.5, got {mid_out_alpha}"
        );
        // Past total.
        let past_total = Duration::from_secs_f32(MOTION_SPLASH_TOTAL_SECS + 0.1);
        assert!(
            splash_alpha(past_total).is_none(),
            "alpha past total duration must be None (signal: despawn)"
        );
    }

    #[test]
    fn splash_dismisses_immediately_on_keypress() {
        let mut app = headless_app();
        // Run one fast tick under the fade-in window so the splash is
        // unambiguously not yet in fade-out before the dismiss.
        set_manual_time_step(&mut app, 0.05);
        app.update();
        let pre_alpha = scrim_alpha(&mut app);
        assert!(
            pre_alpha < 1.0,
            "precondition: splash should be inside fade-in, not yet at full alpha (got {pre_alpha})"
        );

        // Press any key. The dismissal system should bump the age into
        // the fade-out window on this tick.
        press_key(&mut app, KeyCode::Space);
        app.update();

        // Either still alive in fade-out, or already despawned (the
        // 200 ms test-clock clamp can shave the fade-out window
        // depending on how many ticks `app.update()` has accrued).
        if count_splash_roots(&mut app) == 0 {
            return; // already past fade-out — that's fine.
        }
        let mut q = app
            .world_mut()
            .query_filtered::<&SplashAge, With<SplashRoot>>();
        let age = q
            .iter(app.world())
            .next()
            .expect("splash should exist after one post-dismiss tick")
            .0;
        let fade_out_start = Duration::from_secs_f32(
            MOTION_SPLASH_TOTAL_SECS - MOTION_SPLASH_FADE_SECS,
        );
        assert!(
            age >= fade_out_start,
            "after a keypress dismiss the splash must be in fade-out (age >= {fade_out_start:?}); got {age:?}"
        );
    }

    #[test]
    fn splash_dismisses_on_mouse_click() {
        let mut app = headless_app();
        set_manual_time_step(&mut app, 0.05);
        app.update();
        assert!(scrim_alpha(&mut app) < 1.0);

        press_mouse(&mut app, MouseButton::Left);
        app.update();

        if count_splash_roots(&mut app) == 0 {
            return;
        }
        let mut q = app
            .world_mut()
            .query_filtered::<&SplashAge, With<SplashRoot>>();
        let age = q
            .iter(app.world())
            .next()
            .expect("splash should exist after one post-dismiss tick")
            .0;
        let fade_out_start = Duration::from_secs_f32(
            MOTION_SPLASH_TOTAL_SECS - MOTION_SPLASH_FADE_SECS,
        );
        assert!(
            age >= fade_out_start,
            "after a left-click dismiss the splash must be in fade-out; got {age:?}"
        );
    }

    /// Bonus test: dismissing the splash with a keypress does NOT clear
    /// that key's `just_pressed` flag — downstream systems still see
    /// the keystroke that dismissed the splash. Important for parity
    /// with "no splash" behaviour where Space draws a card.
    #[test]
    fn dismissal_keypress_is_visible_to_other_systems() {
        let mut app = headless_app();
        press_key(&mut app, KeyCode::Space);
        app.update();
        let keys = app.world().resource::<ButtonInput<KeyCode>>();
        assert!(
            keys.just_pressed(KeyCode::Space),
            "Splash dismissal must NOT consume the input — downstream gameplay still needs it"
        );
    }
}
