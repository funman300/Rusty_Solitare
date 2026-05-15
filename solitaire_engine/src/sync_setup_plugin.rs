//! Sync-server configuration UI: login / register modal, provider hot-swap,
//! and disconnect handler.
//!
//! # Flow (connect)
//!
//! 1. Player clicks "Connect" in the Settings sync section.
//! 2. `SyncConfigureRequestEvent` → `open_sync_setup_modal` spawns the form.
//! 3. Player fills URL / Username / Password; Tab cycles fields.
//! 4. "Log In" or "Register" → `handle_auth_button` → async task on
//!    `AsyncComputeTaskPool` calling `SolitaireServerClient::login` or
//!    `::register`.
//! 5. `poll_auth_task` harvests the result:
//!    - **Ok**: store tokens → update `SettingsResource` → swap
//!      `SyncProviderResource` → fire `ManualSyncRequestEvent` → toast + close.
//!    - **Err**: display error inline; form stays open.
//!
//! # Flow (disconnect)
//!
//! `SyncLogoutRequestEvent` → `handle_logout` clears tokens, resets
//! `SyncBackend::Local`, swaps provider, closes settings, shows toast.
//!
//! # Flow (delete account)
//!
//! 1. Player clicks "Delete Account" in Settings.
//! 2. `DeleteAccountRequestEvent` → `open_delete_confirm_modal` spawns a
//!    two-button confirmation modal.
//! 3. "Cancel" → despawn modal.
//! 4. "Delete Forever" → `handle_delete_confirm` → async task on
//!    `AsyncComputeTaskPool` calling `SyncProvider::delete_account`.
//! 5. `poll_delete_task` harvests the result:
//!    - **Ok**: fire `SyncLogoutRequestEvent` (clears tokens + resets backend)
//!      + toast.
//!    - **Err**: display error in a toast; modal is already closed.

use std::sync::Arc;

use bevy::input::ButtonState;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, AsyncComputeTaskPool, Task};
use solitaire_data::{
    auth_tokens::{delete_tokens, store_tokens},
    settings::SyncBackend,
    save_settings_to,
    sync_client::{LocalOnlyProvider, SolitaireServerClient},
    SyncError,
};

use crate::events::{
    DeleteAccountRequestEvent, InfoToastEvent, ManualSyncRequestEvent, SyncConfigureRequestEvent,
    SyncLogoutRequestEvent,
};
use crate::font_plugin::FontResource;
use crate::settings_plugin::{SettingsResource, SettingsScreen, SettingsStoragePath};
use crate::sync_plugin::SyncProviderResource;
use crate::ui_modal::spawn_modal;
use crate::ui_theme::{
    ACCENT_PRIMARY, BG_ELEVATED, BG_ELEVATED_HI,
    BORDER_SUBTLE, HighContrastBorder, RADIUS_SM, STATE_DANGER, TEXT_DISABLED,
    TEXT_PRIMARY, TEXT_SECONDARY, TYPE_BODY, TYPE_BODY_LG, TYPE_CAPTION, VAL_SPACE_2, VAL_SPACE_3,
    VAL_SPACE_4, Z_MODAL_PANEL,
};

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Marker on the sync-setup modal scrim (despawn root).
#[derive(Component, Debug)]
pub struct SyncSetupScreen;

/// Discriminant attached to each input-field container and inner text entity.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum SyncFieldKind {
    Url,
    Username,
    Password,
}

/// Per-field raw-text buffer, stored on the inner text entity.
#[derive(Component, Default, Debug)]
struct SyncFieldBuffer(String);

/// Marker on the error-message text node.
#[derive(Component, Debug)]
struct SyncAuthError;

/// Marks the "Log In" button.
#[derive(Component, Debug)]
struct SyncLoginButton;

/// Marks the "Register" button.
#[derive(Component, Debug)]
struct SyncRegisterButton;

/// Marks the "Cancel" button.
#[derive(Component, Debug)]
struct SyncCancelButton;

/// Marks the spinner / busy overlay node shown while the auth task is running.
#[derive(Component, Debug)]
struct SyncBusyOverlay;

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Which field in the sync-setup modal currently has keyboard focus.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq, Eq)]
enum SyncFocusedField {
    #[default]
    Url,
    Username,
    Password,
}

impl SyncFocusedField {
    fn next(self) -> Self {
        match self {
            Self::Url => Self::Username,
            Self::Username => Self::Password,
            Self::Password => Self::Url,
        }
    }

    fn kind(self) -> SyncFieldKind {
        match self {
            Self::Url => SyncFieldKind::Url,
            Self::Username => SyncFieldKind::Username,
            Self::Password => SyncFieldKind::Password,
        }
    }
}

/// In-flight login/register task. `url` and `username` are preserved so the
/// poll system can update settings and provider on success without re-reading
/// the (already-despawned or cleared) form fields.
#[derive(Resource, Default)]
struct PendingAuthTask {
    task: Option<Task<Result<(String, String), SyncError>>>,
    url: String,
    username: String,
}

/// Marker on the account-deletion confirmation modal root.
#[derive(Component, Debug)]
struct DeleteConfirmScreen;

/// Marks the "Delete Forever" confirmation button.
#[derive(Component, Debug)]
struct DeleteConfirmButton;

/// Marks the cancel button inside the delete-confirm modal.
#[derive(Component, Debug)]
struct DeleteCancelButton;

/// In-flight account-deletion task.
#[derive(Resource, Default)]
struct PendingDeleteTask(Option<Task<Result<(), SyncError>>>);

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers the sync configuration UI systems and resources.
pub struct SyncSetupPlugin;

impl Plugin for SyncSetupPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SyncFocusedField>()
            .init_resource::<PendingAuthTask>()
            .init_resource::<PendingDeleteTask>()
            .add_message::<SyncConfigureRequestEvent>()
            .add_message::<SyncLogoutRequestEvent>()
            .add_message::<DeleteAccountRequestEvent>()
            .add_message::<ManualSyncRequestEvent>()
            .add_message::<InfoToastEvent>()
            .add_systems(
                Update,
                (
                    open_sync_setup_modal,
                    handle_text_input,
                    update_field_borders,
                    handle_auth_button,
                    poll_auth_task,
                    handle_cancel,
                    handle_logout,
                    open_delete_confirm_modal,
                    handle_delete_cancel,
                    handle_delete_confirm,
                    poll_delete_task,
                )
                    .chain(),
            );
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Opens the sync-setup modal when `SyncConfigureRequestEvent` is received.
fn open_sync_setup_modal(
    mut events: MessageReader<SyncConfigureRequestEvent>,
    existing: Query<(), With<SyncSetupScreen>>,
    mut commands: Commands,
    mut focused: ResMut<SyncFocusedField>,
    font_res: Option<Res<FontResource>>,
) {
    if events.is_empty() {
        return;
    }
    events.clear();
    if !existing.is_empty() {
        return; // Already open.
    }
    *focused = SyncFocusedField::Url;
    spawn_sync_setup_modal(&mut commands, font_res.as_deref());
}

/// Routes keyboard input to the focused field while the modal is open.
fn handle_text_input(
    screen: Query<(), With<SyncSetupScreen>>,
    mut key_events: MessageReader<KeyboardInput>,
    mut focused: ResMut<SyncFocusedField>,
    mut fields: Query<(&SyncFieldKind, &mut SyncFieldBuffer, &mut Text, &mut TextColor)>,
    pending: Res<PendingAuthTask>,
) {
    if screen.is_empty() || pending.task.is_some() {
        // Swallow events while modal is closed or auth is in flight.
        key_events.clear();
        return;
    }

    for ev in key_events.read() {
        if ev.state != ButtonState::Pressed {
            continue;
        }

        // Tab / Shift-Tab cycle focus.
        if ev.key_code == KeyCode::Tab {
            let shift = ev.logical_key == bevy::input::keyboard::Key::Tab; // no-shift
            let _ = shift; // handled below via modifier check
            // Bevy doesn't give us the shift modifier state on KeyboardInput directly,
            // so we check key_code == Tab and trust that shift produces a separate event.
            // Use ButtonInput<KeyCode> alternative: we check Tab key here and rely on
            // the SyncFocusedField cycling being called per press.
            *focused = focused.next();
            continue;
        }

        if ev.key_code == KeyCode::Backspace {
            for (kind, mut buf, mut text, _) in &mut fields {
                if *kind == focused.kind() {
                    buf.0.pop();
                    text.0 = display_text(&buf.0, *kind);
                }
            }
            continue;
        }

        // Printable character — append to focused buffer.
        if let Some(ch) = ev.text.as_deref().and_then(printable_char) {
            for (kind, mut buf, mut text, mut color) in &mut fields {
                if *kind == focused.kind() {
                    if buf.0.len() < 256 {
                        buf.0.push(ch);
                    }
                    text.0 = display_text(&buf.0, *kind);
                    color.0 = TEXT_PRIMARY;
                }
            }
        }
    }
}

/// Updates the border colour of each input field based on which field is focused.
fn update_field_borders(
    screen: Query<(), With<SyncSetupScreen>>,
    focused: Res<SyncFocusedField>,
    mut borders: Query<(&SyncFieldKind, &mut BorderColor), Without<SyncFieldBuffer>>,
) {
    if screen.is_empty() || !focused.is_changed() {
        return;
    }
    for (kind, mut border) in &mut borders {
        *border = BorderColor::all(if *kind == focused.kind() {
            ACCENT_PRIMARY
        } else {
            BORDER_SUBTLE
        });
    }
}

/// Fires an async auth task when Login or Register is clicked.
fn handle_auth_button(
    login_q: Query<&Interaction, (Changed<Interaction>, With<SyncLoginButton>)>,
    register_q: Query<&Interaction, (Changed<Interaction>, With<SyncRegisterButton>)>,
    fields: Query<(&SyncFieldKind, &SyncFieldBuffer)>,
    mut pending: ResMut<PendingAuthTask>,
    mut error_nodes: Query<(&mut Text, &mut TextColor), With<SyncAuthError>>,
    mut busy_nodes: Query<&mut Visibility, With<SyncBusyOverlay>>,
) {
    let login_clicked = login_q
        .iter()
        .any(|i| *i == Interaction::Pressed);
    let register_clicked = register_q
        .iter()
        .any(|i| *i == Interaction::Pressed);

    if !login_clicked && !register_clicked {
        return;
    }
    if pending.task.is_some() {
        return; // Already in flight.
    }

    // Collect field values.
    let mut url = String::new();
    let mut username = String::new();
    let mut password = String::new();
    for (kind, buf) in &fields {
        match kind {
            SyncFieldKind::Url => url = buf.0.trim().to_string(),
            SyncFieldKind::Username => username = buf.0.trim().to_string(),
            SyncFieldKind::Password => password = buf.0.clone(),
        }
    }

    // Basic validation before hitting the network.
    let validation_error = if url.is_empty() {
        Some("Server URL is required")
    } else if !url.starts_with("http://") && !url.starts_with("https://") {
        Some("URL must start with http:// or https://")
    } else if username.is_empty() {
        Some("Username is required")
    } else if password.is_empty() {
        Some("Password is required")
    } else {
        None
    };

    if let Some(msg) = validation_error {
        for (mut text, mut color) in &mut error_nodes {
            text.0 = msg.to_string();
            color.0 = STATE_DANGER;
        }
        return;
    }

    // Clear error and show busy indicator.
    for (mut text, _) in &mut error_nodes {
        text.0 = "Connecting…".to_string();
    }
    for mut vis in &mut busy_nodes {
        *vis = Visibility::Visible;
    }

    let is_register = register_clicked;
    let client = SolitaireServerClient::new(url.clone(), username.clone());
    let pw = password.clone();

    let task = AsyncComputeTaskPool::get().spawn(async move {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| SyncError::Network(format!("tokio rt: {e}")))?
            .block_on(async {
                if is_register {
                    client.register(&pw).await
                } else {
                    client.login(&pw).await
                }
            })
    });

    pending.task = Some(task);
    pending.url = url;
    pending.username = username;
}

/// Polls the in-flight auth task. On success updates settings + provider.
#[allow(clippy::too_many_arguments)]
fn poll_auth_task(
    mut pending: ResMut<PendingAuthTask>,
    mut settings: ResMut<SettingsResource>,
    settings_path: Res<SettingsStoragePath>,
    mut provider: ResMut<SyncProviderResource>,
    mut error_nodes: Query<(&mut Text, &mut TextColor), With<SyncAuthError>>,
    mut busy_nodes: Query<&mut Visibility, With<SyncBusyOverlay>>,
    screen: Query<Entity, With<SyncSetupScreen>>,
    mut settings_screen: ResMut<SettingsScreen>,
    mut commands: Commands,
    mut manual_sync: MessageWriter<ManualSyncRequestEvent>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    let Some(task) = pending.task.as_mut() else {
        return;
    };
    let Some(result) = future::block_on(future::poll_once(task)) else {
        return;
    };
    pending.task = None;

    for mut vis in &mut busy_nodes {
        *vis = Visibility::Hidden;
    }

    match result {
        Ok((access_token, refresh_token)) => {
            let url = pending.url.clone();
            let username = pending.username.clone();

            // Persist tokens to the OS keychain / Android Keystore.
            if let Err(e) = store_tokens(&username, &access_token, &refresh_token) {
                for (mut text, mut color) in &mut error_nodes {
                    text.0 = format!("Token storage failed: {e}");
                    color.0 = STATE_DANGER;
                }
                return;
            }

            // Update settings and persist.
            settings.0.sync_backend = SyncBackend::SolitaireServer {
                url: url.clone(),
                username: username.clone(),
                avatar_url: None,
            };
            if let Some(path) = &settings_path.0
                && let Err(e) = save_settings_to(path, &settings.0)
            {
                warn!("sync setup: failed to persist settings: {e}");
            }

            // Hot-swap the provider so pull/push use the new credentials.
            provider.0 = Arc::new(SolitaireServerClient::new(url, username.clone()));

            // Kick off an immediate pull with the new provider.
            manual_sync.write(ManualSyncRequestEvent);

            // Close both the setup modal and the settings panel.
            for entity in &screen {
                commands.entity(entity).despawn();
            }
            settings_screen.0 = false;

            toast.write(InfoToastEvent(format!("Connected as {username}")));
        }
        Err(e) => {
            let msg = match e {
                SyncError::Auth(m) => m,
                SyncError::Network(m) => format!("Network error: {m}"),
                SyncError::Serialization(m) => format!("Unexpected response: {m}"),
                SyncError::UnsupportedPlatform => "Unsupported platform".into(),
            };
            for (mut text, mut color) in &mut error_nodes {
                text.0 = msg.clone();
                color.0 = STATE_DANGER;
            }
        }
    }
}

/// Dismisses the sync-setup modal on Cancel click or Escape.
fn handle_cancel(
    cancel_q: Query<&Interaction, (Changed<Interaction>, With<SyncCancelButton>)>,
    keys: Res<ButtonInput<KeyCode>>,
    screen: Query<Entity, With<SyncSetupScreen>>,
    mut commands: Commands,
) {
    let cancelled = cancel_q.iter().any(|i| *i == Interaction::Pressed)
        || keys.just_pressed(KeyCode::Escape);
    if !cancelled || screen.is_empty() {
        return;
    }
    for entity in &screen {
        commands.entity(entity).despawn();
    }
}

/// Clears stored tokens, resets the backend to `Local`, and hot-swaps the
/// provider. Triggered by "Disconnect" in the settings sync section.
fn handle_logout(
    mut events: MessageReader<SyncLogoutRequestEvent>,
    mut settings: ResMut<SettingsResource>,
    settings_path: Res<SettingsStoragePath>,
    mut provider: ResMut<SyncProviderResource>,
    mut settings_screen: ResMut<SettingsScreen>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    if events.is_empty() {
        return;
    }
    events.clear();

    // Extract username before resetting so we can clear the right keychain key.
    let username = match &settings.0.sync_backend {
        SyncBackend::SolitaireServer { username, .. } => Some(username.clone()),
        SyncBackend::Local => None,
    };

    if let Some(u) = username
        && let Err(e) = delete_tokens(&u)
    {
        warn!("sync logout: failed to clear tokens: {e}");
    }

    settings.0.sync_backend = SyncBackend::Local;
    if let Some(path) = &settings_path.0
        && let Err(e) = save_settings_to(path, &settings.0)
    {
        warn!("sync logout: failed to persist settings: {e}");
    }

    provider.0 = Arc::new(LocalOnlyProvider);
    settings_screen.0 = false;
    toast.write(InfoToastEvent("Disconnected from sync server".to_string()));
}

/// Opens the account-deletion confirmation modal when `DeleteAccountRequestEvent` fires.
fn open_delete_confirm_modal(
    mut events: MessageReader<DeleteAccountRequestEvent>,
    existing: Query<(), With<DeleteConfirmScreen>>,
    mut commands: Commands,
    font_res: Option<Res<FontResource>>,
) {
    if events.is_empty() {
        return;
    }
    events.clear();
    if !existing.is_empty() {
        return;
    }
    spawn_delete_confirm_modal(&mut commands, font_res.as_deref());
}

/// Despawns the delete-confirm modal on the cancel button or Escape.
fn handle_delete_cancel(
    cancel_q: Query<&Interaction, (Changed<Interaction>, With<DeleteCancelButton>)>,
    keys: Res<ButtonInput<KeyCode>>,
    screen: Query<Entity, With<DeleteConfirmScreen>>,
    mut commands: Commands,
) {
    let cancelled = cancel_q.iter().any(|i| *i == Interaction::Pressed)
        || keys.just_pressed(KeyCode::Escape);
    if !cancelled || screen.is_empty() {
        return;
    }
    for entity in &screen {
        commands.entity(entity).despawn();
    }
}

/// Spawns the async delete-account task when "Delete Forever" is clicked.
fn handle_delete_confirm(
    confirm_q: Query<&Interaction, (Changed<Interaction>, With<DeleteConfirmButton>)>,
    provider: Res<SyncProviderResource>,
    mut pending: ResMut<PendingDeleteTask>,
    screen: Query<Entity, With<DeleteConfirmScreen>>,
    mut commands: Commands,
) {
    if !confirm_q.iter().any(|i| *i == Interaction::Pressed) || pending.0.is_some() {
        return;
    }
    // Despawn the confirmation modal immediately so the player can't double-click.
    for entity in &screen {
        commands.entity(entity).despawn();
    }
    let provider = provider.0.clone();
    pending.0 = Some(AsyncComputeTaskPool::get().spawn(async move {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| SyncError::Network(format!("tokio rt: {e}")))?
            .block_on(provider.delete_account())
    }));
}

/// Polls the in-flight delete-account task. On success fires `SyncLogoutRequestEvent`.
fn poll_delete_task(
    mut pending: ResMut<PendingDeleteTask>,
    mut logout: MessageWriter<SyncLogoutRequestEvent>,
    mut toast: MessageWriter<InfoToastEvent>,
) {
    let Some(task) = pending.0.as_mut() else {
        return;
    };
    let Some(result) = future::block_on(future::poll_once(task)) else {
        return;
    };
    pending.0 = None;
    match result {
        Ok(()) => {
            logout.write(SyncLogoutRequestEvent);
            toast.write(InfoToastEvent("Account deleted".to_string()));
        }
        Err(e) => {
            let msg = match e {
                SyncError::Auth(_) => "Not authorised — try reconnecting first".to_string(),
                SyncError::Network(m) => format!("Network error: {m}"),
                other => format!("Delete failed: {other}"),
            };
            toast.write(InfoToastEvent(msg));
        }
    }
}

// ---------------------------------------------------------------------------
// UI construction
// ---------------------------------------------------------------------------

fn spawn_sync_setup_modal(commands: &mut Commands, font_res: Option<&FontResource>) {
    spawn_modal(commands, SyncSetupScreen, Z_MODAL_PANEL + 1, |card| {
        // Header.
        card.spawn(Node {
            padding: UiRect::new(VAL_SPACE_4, VAL_SPACE_4, VAL_SPACE_3, VAL_SPACE_2),
            ..default()
        })
        .with_children(|h| {
            h.spawn((
                Text::new("Connect to Server"),
                make_font(font_res, TYPE_BODY_LG),
                TextColor(TEXT_PRIMARY),
            ));
        });

        // Scrollable body — three labeled input fields + error line.
        card.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: VAL_SPACE_3,
            padding: UiRect::axes(VAL_SPACE_4, VAL_SPACE_2),
            flex_grow: 1.0,
            ..default()
        })
        .with_children(|body| {
            spawn_field(
                body,
                SyncFieldKind::Url,
                "Server URL",
                "https://your-server.example.com",
                true,   // focused initially
                font_res,
            );
            spawn_field(
                body,
                SyncFieldKind::Username,
                "Username",
                "your-username",
                false,
                font_res,
            );
            spawn_field(
                body,
                SyncFieldKind::Password,
                "Password",
                "••••••••",
                false,
                font_res,
            );

            // Error / status line.
            body.spawn(Node {
                min_height: Val::Px(18.0),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    SyncAuthError,
                    SyncBusyOverlay,
                    Text::new(String::new()),
                    make_font(font_res, TYPE_CAPTION),
                    TextColor(TEXT_SECONDARY),
                    Visibility::Hidden,
                ));
            });

            // Tab hint — desktop only; no Tab key on Android.
            #[cfg(not(target_os = "android"))]
            body.spawn((
                Text::new("Tab  =  next field"),
                make_font(font_res, TYPE_CAPTION),
                TextColor(TEXT_DISABLED),
            ));
        });

        // Action row.
        card.spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::FlexEnd,
            column_gap: VAL_SPACE_2,
            padding: UiRect::new(VAL_SPACE_4, VAL_SPACE_4, VAL_SPACE_2, VAL_SPACE_3),
            ..default()
        })
        .with_children(|actions| {
            spawn_action_button(actions, SyncCancelButton, "Cancel", false, font_res);
            spawn_action_button(actions, SyncRegisterButton, "Register", false, font_res);
            spawn_action_button(actions, SyncLoginButton, "Log In", true, font_res);
        });
    });
}

fn spawn_field(
    parent: &mut ChildSpawnerCommands,
    kind: SyncFieldKind,
    label: &str,
    placeholder: &str,
    focused: bool,
    font_res: Option<&FontResource>,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|col| {
            // Label.
            col.spawn((
                Text::new(label.to_string()),
                make_font(font_res, TYPE_CAPTION),
                TextColor(TEXT_SECONDARY),
            ));

            // Input border container — carries kind for the border-update system.
            col.spawn((
                kind,
                Node {
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                    padding: UiRect::axes(VAL_SPACE_2, Val::Px(6.0)),
                    min_height: Val::Px(32.0),
                    ..default()
                },
                BackgroundColor(BG_ELEVATED),
                BorderColor::all(if focused { ACCENT_PRIMARY } else { BORDER_SUBTLE }),
                HighContrastBorder::with_default(BORDER_SUBTLE),
            ))
            .with_children(|border| {
                // Inner text / buffer entity.
                border.spawn((
                    kind,
                    SyncFieldBuffer(String::new()),
                    Text::new(placeholder.to_string()),
                    make_font(font_res, TYPE_BODY),
                    TextColor(TEXT_DISABLED),
                ));
            });
        });
}

fn spawn_action_button<M: Component>(
    parent: &mut ChildSpawnerCommands,
    marker: M,
    label: &str,
    primary: bool,
    font_res: Option<&FontResource>,
) {
    let bg = if primary { ACCENT_PRIMARY } else { BG_ELEVATED_HI };
    let fg = TEXT_PRIMARY;
    parent
        .spawn((
            marker,
            Button,
            Node {
                padding: UiRect::axes(VAL_SPACE_3, VAL_SPACE_2),
                justify_content: JustifyContent::Center,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                ..default()
            },
            BackgroundColor(bg),
            BorderColor::all(if primary { ACCENT_PRIMARY } else { BORDER_SUBTLE }),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(label.to_string()),
                make_font(font_res, TYPE_BODY),
                TextColor(fg),
            ));
        });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_font(font_res: Option<&FontResource>, size: f32) -> TextFont {
    TextFont {
        font: font_res.map(|f| f.0.clone()).unwrap_or_default(),
        font_size: size,
        ..default()
    }
}

fn spawn_delete_confirm_modal(commands: &mut Commands, font_res: Option<&FontResource>) {
    spawn_modal(commands, DeleteConfirmScreen, Z_MODAL_PANEL + 2, |card| {
        // Header.
        card.spawn(Node {
            padding: UiRect::new(VAL_SPACE_4, VAL_SPACE_4, VAL_SPACE_3, VAL_SPACE_2),
            ..default()
        })
        .with_children(|h| {
            h.spawn((
                Text::new("Delete Account"),
                make_font(font_res, TYPE_BODY_LG),
                TextColor(STATE_DANGER),
            ));
        });

        // Body.
        card.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: VAL_SPACE_2,
            padding: UiRect::axes(VAL_SPACE_4, VAL_SPACE_2),
            ..default()
        })
        .with_children(|body| {
            body.spawn((
                Text::new(
                    "This permanently deletes your account and all server data.\n\
                     Local progress is kept. This cannot be undone.",
                ),
                make_font(font_res, TYPE_BODY),
                TextColor(TEXT_SECONDARY),
            ));
        });

        // Actions.
        card.spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::FlexEnd,
            column_gap: VAL_SPACE_2,
            padding: UiRect::new(VAL_SPACE_4, VAL_SPACE_4, VAL_SPACE_2, VAL_SPACE_3),
            ..default()
        })
        .with_children(|actions| {
            spawn_action_button(actions, DeleteCancelButton, "Cancel", false, font_res);
            // "Delete Forever" button — danger styling (STATE_DANGER background).
            actions
                .spawn((
                    DeleteConfirmButton,
                    Button,
                    Node {
                        padding: UiRect::axes(VAL_SPACE_3, VAL_SPACE_2),
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(RADIUS_SM)),
                        ..default()
                    },
                    BackgroundColor(STATE_DANGER),
                    BorderColor::all(STATE_DANGER),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Delete Forever"),
                        make_font(font_res, TYPE_BODY),
                        TextColor(TEXT_PRIMARY),
                    ));
                });
        });
    });
}

/// Returns the display string for a field — password fields show bullets.
fn display_text(raw: &str, kind: SyncFieldKind) -> String {
    if kind == SyncFieldKind::Password {
        "•".repeat(raw.len())
    } else {
        raw.to_string()
    }
}

/// Extracts a printable ASCII character from a SmolStr keypress text.
fn printable_char(text: &str) -> Option<char> {
    let ch = text.chars().next()?;
    // Accept printable ASCII: 0x20 (space) through 0x7e (~).
    (' '..='~').contains(&ch).then_some(ch)
}
