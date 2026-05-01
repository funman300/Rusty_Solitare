use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowPosition};
use solitaire_data::{load_settings_from, provider_for_backend, settings_file_path, Settings};
use solitaire_engine::{
    register_theme_asset_sources, AchievementPlugin, AnimationPlugin, AssetSourcesPlugin,
    AudioPlugin, AutoCompletePlugin, CardAnimationPlugin, CardPlugin, ChallengePlugin,
    CursorPlugin, DailyChallengePlugin, FeedbackAnimPlugin, FontPlugin, GamePlugin, HelpPlugin,
    HomePlugin, HudPlugin, InputPlugin, LeaderboardPlugin, OnboardingPlugin, PausePlugin,
    ProfilePlugin, ProgressPlugin, SelectionPlugin, SettingsPlugin, SplashPlugin, StatsPlugin,
    SyncPlugin, TablePlugin, ThemePlugin, ThemeRegistryPlugin, TimeAttackPlugin, UiFocusPlugin,
    UiModalPlugin, UiTooltipPlugin, WeeklyGoalsPlugin, WinSummaryPlugin,
};

fn main() {
    // Install a panic hook that writes a crash log next to the save files
    // before re-running the default hook (so stderr still gets the message
    // and any debugger attached still sees the panic).
    install_crash_log_hook();

    // Initialise the platform keyring store before any token operations.
    // On Linux this uses the Secret Service (GNOME Keyring / KWallet); on
    // macOS it uses the Keychain; on Windows it uses the Credential store.
    // If the platform has no OS keyring (e.g. a headless CI box), keyring
    // operations will fail gracefully with TokenError::KeychainUnavailable.
    if let Err(e) = keyring::use_native_store(true) {
        eprintln!(
            "warn: could not initialise OS keyring ({e}); \
             server sync login will be unavailable"
        );
    }

    // Load settings before building the app so we can construct the right
    // sync provider. Falls back to defaults if no settings file exists yet.
    let settings: Settings = settings_file_path()
        .map(|p| load_settings_from(&p))
        .unwrap_or_default();
    let sync_provider = provider_for_backend(&settings.sync_backend);

    // Restore the previous window geometry if the player has one saved.
    // Otherwise open at the platform default (1280×800, centred on the
    // primary monitor). The window_geometry field is None on first run
    // and after upgrading from a build that didn't persist geometry.
    let (window_resolution, window_position) = match settings.window_geometry {
        Some(geom) => (
            (geom.width, geom.height).into(),
            WindowPosition::At(IVec2::new(geom.x, geom.y)),
        ),
        None => (
            (1280u32, 800u32).into(),
            WindowPosition::Centered(MonitorSelection::Primary),
        ),
    };

    let mut app = App::new();

    // The card-theme system's `themes://` asset source must be
    // registered *before* `DefaultPlugins` builds `AssetPlugin`,
    // because that plugin freezes the asset-source list at build
    // time. The matching `AssetSourcesPlugin` (added below) finishes
    // the wiring after `DefaultPlugins` by populating the embedded
    // default theme into Bevy's `EmbeddedAssetRegistry`.
    register_theme_asset_sources(&mut app);

    app
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Solitaire Quest".into(),
                        // X11/Wayland WM_CLASS so taskbar managers group
                        // multiple windows of this app correctly.
                        name: Some("solitaire-quest".into()),
                        resolution: window_resolution,
                        position: window_position,
                        // AutoNoVsync prefers Mailbox (triple-buffered) and
                        // falls back to Immediate, eliminating the vsync stall
                        // that AutoVsync produces during continuous window
                        // resize on X11 / Wayland. The game's frame budget is
                        // small enough that a few stray dropped frames from
                        // disabling vsync are imperceptible.
                        present_mode: PresentMode::AutoNoVsync,
                        resize_constraints: bevy::window::WindowResizeConstraints {
                            min_width: 800.0,
                            min_height: 600.0,
                            ..default()
                        },
                        ..default()
                    }),
                    ..default()
                })
                // The `assets/` directory lives at the workspace root, but
                // Bevy resolves `AssetPlugin::file_path` relative to the
                // binary package's `CARGO_MANIFEST_DIR` (`solitaire_app/`).
                // Point one level up so `cargo run -p solitaire_app` finds
                // card faces, backs, backgrounds, and the UI font.
                .set(bevy::asset::AssetPlugin {
                    file_path: "../assets".to_string(),
                    ..default()
                }),
        )
        .add_plugins(AssetSourcesPlugin)
        .add_plugins(ThemePlugin)
        .add_plugins(ThemeRegistryPlugin)
        .add_plugins(FontPlugin)
        .add_plugins(GamePlugin)
        .add_plugins(TablePlugin)
        .add_plugins(CardPlugin)
        .add_plugins(CursorPlugin)
        .add_plugins(InputPlugin)
        .add_plugins(SelectionPlugin)
        .add_plugins(AnimationPlugin)
        .add_plugins(FeedbackAnimPlugin)
        .add_plugins(CardAnimationPlugin)
        .add_plugins(AutoCompletePlugin)
        .add_plugins(StatsPlugin::default())
        .add_plugins(ProgressPlugin::default())
        .add_plugins(AchievementPlugin::default())
        .add_plugins(DailyChallengePlugin)
        .add_plugins(WeeklyGoalsPlugin)
        .add_plugins(ChallengePlugin)
        .add_plugins(TimeAttackPlugin)
        .add_plugins(HudPlugin)
        .add_plugins(HelpPlugin)
        .add_plugins(HomePlugin)
        .add_plugins(ProfilePlugin)
        .add_plugins(PausePlugin)
        .add_plugins(SettingsPlugin::default())
        .add_plugins(AudioPlugin)
        .add_plugins(OnboardingPlugin)
        .add_plugins(SyncPlugin::new(sync_provider))
        .add_plugins(LeaderboardPlugin)
        .add_plugins(WinSummaryPlugin)
        .add_plugins(UiModalPlugin)
        .add_plugins(UiFocusPlugin)
        .add_plugins(UiTooltipPlugin)
        .add_plugins(SplashPlugin)
        .run();
}

/// Wraps the default panic hook with one that also appends a crash log
/// to `<data_dir>/crash.log` (next to `settings.json`). The default hook
/// still runs afterwards, so stderr output and debugger integration are
/// unchanged. If the data directory is unavailable, the wrapper silently
/// falls through — the default hook handles output either way.
fn install_crash_log_hook() {
    let crash_log_path = settings_file_path().and_then(|p| {
        p.parent()
            .map(|parent| parent.join("crash.log"))
    });
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = crash_log_path.as_ref()
            && let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
        {
            // Plain unix-seconds timestamp keeps the format trivially
            // parseable and avoids pulling in chrono just for this.
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let _ = writeln!(file, "----- t={secs} -----\n{info}\n");
        }
        default_hook(info);
    }));
}
