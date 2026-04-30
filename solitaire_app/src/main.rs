use bevy::prelude::*;
use solitaire_data::{load_settings_from, provider_for_backend, settings_file_path, Settings};
use solitaire_engine::{
    AchievementPlugin, AnimationPlugin, AudioPlugin, AutoCompletePlugin, CardAnimationPlugin,
    CardPlugin, ChallengePlugin, CursorPlugin, DailyChallengePlugin, FeedbackAnimPlugin,
    FontPlugin, GamePlugin, HelpPlugin, HomePlugin, HudPlugin, InputPlugin, LeaderboardPlugin,
    OnboardingPlugin, PausePlugin, ProfilePlugin, ProgressPlugin, SelectionPlugin, SettingsPlugin,
    StatsPlugin, SyncPlugin, TablePlugin, TimeAttackPlugin, UiModalPlugin, WeeklyGoalsPlugin,
    WinSummaryPlugin,
};

fn main() {
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

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Solitaire Quest".into(),
                        resolution: (1280u32, 800u32).into(),
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
        .run();
}
