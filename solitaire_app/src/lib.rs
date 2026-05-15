//! Library entry point for `solitaire_app`.
//!
//! The app is a `cdylib + bin` hybrid: desktop builds run through the
//! `bin` target's [`main`](crate::main_desktop) shim; Android builds
//! load this `cdylib` via NativeActivity / GameActivity, which calls
//! into the platform's own `main` glue. Both paths converge on
//! [`run`], so the ECS bootstrap is single-sourced.
//!
//! Why split this out: cargo-apk requires the package to expose a
//! `cdylib` library target — the Android activity dlopens
//! `libsolitaire_app.so` and calls into it. A bin-only crate panics
//! at build time with `Bin is not compatible with Cdylib`. The split
//! keeps the desktop `cargo run -p solitaire_app` flow unchanged
//! while making `cargo apk build -p solitaire_app` viable.

use std::fs::OpenOptions;
use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, WindowPosition};
#[cfg(not(target_os = "android"))]
use bevy::window::{Monitor, PrimaryMonitor, PrimaryWindow};
#[cfg(not(target_os = "android"))]
use bevy::winit::WinitWindows;
use solitaire_data::{load_settings_from, provider_for_backend, settings_file_path, Settings};
use solitaire_engine::{
    register_theme_asset_sources, AchievementPlugin, AnalyticsPlugin, AnimationPlugin, AssetSourcesPlugin,
    AudioPlugin, AutoCompletePlugin, AvatarPlugin, CardAnimationPlugin, CardPlugin, ChallengePlugin,
    CursorPlugin, DailyChallengePlugin, DiagnosticsHudPlugin, DifficultyPlugin, FeedbackAnimPlugin,
    FontPlugin, GamePlugin, HelpPlugin, HomePlugin, HudPlugin, InputPlugin, LeaderboardPlugin,
    OnboardingPlugin, PausePlugin, PlayBySeedPlugin, ProfilePlugin, ProgressPlugin,
    RadialMenuPlugin, ReplayOverlayPlugin, ReplayPlaybackPlugin, SafeAreaInsetsPlugin,
    SelectionPlugin, SettingsPlugin,
    SplashPlugin, StatsPlugin, SyncPlugin, SyncSetupPlugin, TablePlugin, ThemePlugin, ThemeRegistryPlugin,
    TimeAttackPlugin, UiFocusPlugin, UiModalPlugin, UiTooltipPlugin, WeeklyGoalsPlugin,
    WinSummaryPlugin,
};

/// App entry point — builds and runs the Bevy app.
///
/// Called from both the desktop `bin` target's `main` shim and (on
/// Android) the platform's NativeActivity / GameActivity glue.
pub fn run() {
    // Install a panic hook that writes a crash log next to the save files
    // before re-running the default hook (so stderr still gets the message
    // and any debugger attached still sees the panic).
    install_crash_log_hook();

    // Initialise the platform keyring store before any token operations.
    // On Linux this uses the Secret Service (GNOME Keyring / KWallet); on
    // macOS it uses the Keychain; on Windows it uses the Credential store.
    // If the platform has no OS keyring (e.g. a headless CI box), keyring
    // operations will fail gracefully with TokenError::KeychainUnavailable.
    //
    // Android: `keyring` isn't compiled in (its `rpassword` transitive
    // pulls a libc symbol Android's bionic doesn't expose). `auth_tokens`
    // ships an Android stub that returns KeychainUnavailable for every
    // call — the runtime behaviour is "session login required each launch"
    // until we wire Android Keystore via JNI in the Phase-Android round.
    #[cfg(not(target_os = "android"))]
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
    // primary monitor) — `apply_smart_default_window_size` will resize
    // up to a monitor-relative target on the first frame so HiDPI / 4K
    // sessions don't end up with a comparatively tiny window.
    #[cfg(not(target_os = "android"))]
    let had_saved_geometry = settings.window_geometry.is_some();
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
                        title: "Ferrous Solitaire".into(),
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
                        // Android windows always fill the screen; max_width/max_height
                        // default to 0.0, which panics Bevy's clamp when min > max.
                        #[cfg(not(target_os = "android"))]
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
                // on desktop Bevy resolves `AssetPlugin::file_path` relative
                // to the binary package's `CARGO_MANIFEST_DIR`
                // (`solitaire_app/`), so `cargo run -p solitaire_app` would
                // miss the workspace-root `assets/` without a `../` prefix.
                //
                // On Android cargo-apk packages the same directory into the
                // APK at `assets/` (via `[package.metadata.android].assets`
                // in solitaire_app/Cargo.toml). Bevy's `AndroidAssetReader`
                // is already rooted there, so any `file_path` other than the
                // default makes it walk *out* of the APK's assets root and
                // all loads fail silently — which is what produced the
                // solid-red card-back fallback in the v0.22.3 screenshot.
                .set(bevy::asset::AssetPlugin {
                    #[cfg(not(target_os = "android"))]
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
        // Cursor-icon feedback is desktop-only; Android has no pointer cursor.
        // The drop-target highlight systems (update_drop_highlights,
        // update_drop_target_overlays) live in CursorPlugin but ARE useful
        // on Android — they've been left running because their Bevy system
        // params compile and function on Android; only the CursorIcon insert
        // is inert. Gate the whole plugin if the cursor APIs ever cause
        // Android linker issues; for now it's harmless to leave it registered.
        .add_plugins(CursorPlugin)
        .add_plugins(InputPlugin)
        .add_plugins(RadialMenuPlugin)
        .add_plugins(SelectionPlugin)
        .add_plugins(AnimationPlugin)
        .add_plugins(FeedbackAnimPlugin)
        .add_plugins(CardAnimationPlugin)
        .add_plugins(AutoCompletePlugin)
        .add_plugins(ReplayPlaybackPlugin)
        .add_plugins(ReplayOverlayPlugin)
        .add_plugins(StatsPlugin::default())
        .add_plugins(ProgressPlugin::default())
        .add_plugins(AchievementPlugin::default())
        .add_plugins(DailyChallengePlugin)
        .add_plugins(WeeklyGoalsPlugin)
        .add_plugins(ChallengePlugin)
        .add_plugins(PlayBySeedPlugin)
        .add_plugins(DifficultyPlugin)
        .add_plugins(TimeAttackPlugin)
        .add_plugins(SafeAreaInsetsPlugin)
        .add_plugins(HudPlugin)
        .add_plugins(HelpPlugin)
        .add_plugins(HomePlugin::default())
        .add_plugins(AvatarPlugin)
        .add_plugins(ProfilePlugin)
        .add_plugins(PausePlugin)
        .add_plugins(SettingsPlugin::default())
        .add_plugins(AudioPlugin)
        .add_plugins(OnboardingPlugin)
        .add_plugins(SyncPlugin::new(sync_provider))
        .add_plugins(SyncSetupPlugin)
        .add_plugins(AnalyticsPlugin)
        .add_plugins(LeaderboardPlugin)
        .add_plugins(WinSummaryPlugin)
        .add_plugins(UiModalPlugin)
        .add_plugins(UiFocusPlugin)
        .add_plugins(UiTooltipPlugin)
        .add_plugins(SplashPlugin)
        .add_plugins(DiagnosticsHudPlugin);

    // Wire the runtime window icon. Bevy 0.18 has no first-class
    // `Window::icon` field; the icon is set through the underlying
    // `winit::window::Window` via `WinitWindows`. Android draws its
    // launcher icon from the APK manifest, so the system is desktop-
    // only — same target-gate as the `winit` dep itself.
    #[cfg(not(target_os = "android"))]
    app.add_systems(Update, set_window_icon);

    // Smart default window sizing: when no saved geometry was loaded,
    // resize the freshly-opened 1280×800 window to ~70 % of the primary
    // monitor's logical size on the first frame. Without this, a 4K
    // monitor opens the same 1280×800 window that a 1080p monitor
    // does — visually tiny relative to screen. Skipped entirely when
    // saved geometry was applied; the player's preference always wins.
    //
    // Players who specifically want the literal 1280×800 baseline on
    // every fresh launch can flip `disable_smart_default_size` in
    // Settings to opt out. The flag is checked once at startup; a
    // mid-session change applies on the next launch.
    // Android windows are always full-screen; the OS controls sizing.
    #[cfg(not(target_os = "android"))]
    if !had_saved_geometry && !settings.disable_smart_default_size {
        app.add_systems(Update, apply_smart_default_window_size);
    }

    app.run();
}

/// One-shot Update system that runs only on launches without saved
/// window geometry. Resizes the primary window to a fraction of the
/// primary monitor's *logical* size — bigger monitors get bigger
/// windows automatically. Logical size already accounts for the OS's
/// HiDPI scale factor, so a 2880×1800 Retina display reporting
/// scale_factor 2.0 yields a 1440×900 logical size and a 1008×630
/// target window — same physical inches as a 1920×1080 monitor with
/// scale_factor 1.0 yielding 1344×756.
///
/// Uses `Local<bool>` to make itself one-shot rather than introducing
/// a dedicated resource. The Update tick is necessary because Bevy
/// populates the `Monitor` entities asynchronously after winit's
/// Resumed event fires; they may not exist on the first Startup pass.
#[cfg(not(target_os = "android"))]
fn apply_smart_default_window_size(
    mut applied: Local<bool>,
    monitors: Query<&Monitor, With<PrimaryMonitor>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if *applied {
        return;
    }
    let Ok(monitor) = monitors.single() else {
        // Primary monitor not yet spawned by bevy_winit. Try again
        // next frame; the cost is one early-exit per tick until
        // monitors arrive (typically frame 1 or 2).
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };

    let scale = monitor.scale_factor as f32;
    if scale <= 0.0 {
        // Defensive: a zero or negative scale factor would NaN the
        // arithmetic below. Bail and accept the default size.
        *applied = true;
        return;
    }
    let logical_w = monitor.physical_width as f32 / scale;
    let logical_h = monitor.physical_height as f32 / scale;

    // Target 70 % of monitor in each dimension, clamped to the
    // existing 800×600 minimum and the monitor's own logical size
    // (so we never request a window larger than the screen).
    let target_w = (logical_w * 0.7).clamp(800.0, logical_w);
    let target_h = (logical_h * 0.7).clamp(600.0, logical_h);

    // Resize only when the change is meaningful — at exactly 1280×800
    // on a 1920×1080 monitor the new target is 1344×756 (only ~5 %
    // wider), worth the resize; at the same default on an 800×600
    // monitor the clamp pins us at 800×600 and we shouldn't resize.
    let curr_w = window.resolution.width();
    let curr_h = window.resolution.height();
    if (curr_w - target_w).abs() > 8.0 || (curr_h - target_h).abs() > 8.0 {
        window.resolution.set(target_w, target_h);
    }
    *applied = true;
}

/// One-shot Update system that sets the primary window's taskbar /
/// title-bar icon to the embedded 256 px Terminal-aesthetic mark
/// generated by `solitaire_engine/examples/icon_generator.rs`.
///
/// Bevy 0.18 has no `Window::icon` field — the icon is set through
/// the underlying `winit::window::Window` via the `WinitWindows`
/// resource. The system is desktop-only (Android draws its launcher
/// icon from the APK manifest, not from any runtime call). Returns
/// silently and tries again next frame until both the primary
/// window and `WinitWindows` are populated, then sets the icon
/// once and self-disables via `Local<bool>`.
///
/// Icon bytes are `include_bytes!()`-embedded at compile time, same
/// shape as the audio assets and default-theme SVGs — no runtime
/// asset-path resolution, no `cargo run` working-directory
/// assumptions. PNG → RGBA decode runs through `tiny_skia` (already
/// in the build for SVG rasterisation), so this system adds zero
/// new dependencies on top of the direct `winit` dep that's
/// already required for `Icon` construction.
#[cfg(not(target_os = "android"))]
fn set_window_icon(
    mut applied: Local<bool>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    // `Option<NonSend<...>>` rather than `NonSend<...>` because Bevy
    // 0.18's stricter system-param validation panics on the first
    // few frames before `WinitWindows` is inserted (the resource is
    // populated after winit's `Resumed` event, which fires after
    // the first system-tick batch). The early-return below handles
    // the `None` window-wrapper case for the same lifecycle reason.
    winit_windows: Option<NonSend<WinitWindows>>,
) {
    if *applied {
        return;
    }
    let Some(winit_windows) = winit_windows else {
        return;
    };
    let Ok(primary_entity) = primary_window.single() else {
        return;
    };
    let Some(window_wrapper) = winit_windows.get_window(primary_entity) else {
        // Primary window's underlying winit handle not yet
        // populated — `WinitWindows` fills in after the first
        // `Resumed` event. Try again next frame.
        return;
    };

    // The 256 × 256 PNG is sufficient for `set_window_icon`; winit
    // scales it for the actual rendered size. Smaller PNGs in
    // `assets/icon/` exist for downstream Linux hicolor / Windows
    // `.ico` / macOS `.icns` packaging — they're not used here.
    const ICON_BYTES: &[u8] = include_bytes!("../../assets/icon/icon_256.png");

    let pixmap = match tiny_skia::Pixmap::decode_png(ICON_BYTES) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("warn: could not decode embedded window icon PNG: {e}");
            *applied = true; // don't retry every frame
            return;
        }
    };
    let rgba = pixmap.data().to_vec();
    let icon = match winit::window::Icon::from_rgba(rgba, pixmap.width(), pixmap.height()) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("warn: could not construct window icon: {e}");
            *applied = true;
            return;
        }
    };
    window_wrapper.set_window_icon(Some(icon));
    *applied = true;
}

/// Android entry point called by NativeActivity after dlopen-ing the `.so`.
/// Sets the `AndroidApp` handle that Bevy's winit backend reads before
/// constructing the event loop, then delegates to [`run`].
///
/// The `#[bevy_main]` proc-macro would generate the same code but only
/// works on a function named `main`; our shared entry point is `run`, so
/// we emit the equivalent expansion manually.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: bevy::android::android_activity::AndroidApp) {
    let _ = bevy::android::ANDROID_APP.set(android_app);
    run();
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
