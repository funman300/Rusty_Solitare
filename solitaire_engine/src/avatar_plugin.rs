//! Downloads and caches the player's server avatar for display in the
//! profile modal.
//!
//! # Flow
//!
//! 1. After a successful login/register, `sync_setup_plugin` fires
//!    [`AvatarFetchEvent`] with the server base URL and the relative
//!    avatar path (e.g. `/avatars/{uuid}.png`).
//! 2. [`handle_avatar_fetch`] spawns an async task on the
//!    [`AsyncComputeTaskPool`] that downloads the image bytes via
//!    `reqwest` (reusing the same HTTP client pattern as the sync client).
//! 3. [`poll_avatar_task`] harvests the result, decodes the bytes into a
//!    Bevy [`Image`] via `image::load_from_memory`, inserts it into
//!    [`Assets<Image>`], and stores the [`Handle<Image>`] in
//!    [`AvatarResource`].
//! 4. `profile_plugin` reads [`AvatarResource`] when the profile modal
//!    opens and renders the avatar image (or an initials fallback when
//!    `AvatarResource` is `None`).

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::tasks::{futures_lite::future, AsyncComputeTaskPool, Task};

use crate::resources::TokioRuntimeResource;

/// Stores the loaded avatar [`Handle<Image>`], or `None` when no avatar
/// has been fetched yet (new account, no internet, or fetch in progress).
#[derive(Resource, Default)]
pub struct AvatarResource(pub Option<Handle<Image>>);

/// Fired by `sync_setup_plugin` after a successful login or register when
/// the server reports that the user has a profile picture set.
#[derive(Debug, Clone)]
pub struct AvatarFetchEvent {
    /// Full HTTP(S) URL to the avatar image (base_url + avatar_url path).
    pub url: String,
}

impl bevy::prelude::Message for AvatarFetchEvent {}

/// In-flight avatar download task. Returns the raw image bytes on success,
/// or `None` on any network / decode error.
#[derive(Resource, Default)]
struct PendingAvatarTask(Option<Task<Option<Vec<u8>>>>);

pub struct AvatarPlugin;

impl Plugin for AvatarPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<AvatarFetchEvent>()
            .init_resource::<TokioRuntimeResource>()
            .init_resource::<AvatarResource>()
            .init_resource::<PendingAvatarTask>()
            .add_systems(Update, (handle_avatar_fetch, poll_avatar_task));
    }
}

fn handle_avatar_fetch(
    mut events: MessageReader<AvatarFetchEvent>,
    rt: Res<TokioRuntimeResource>,
    mut pending: ResMut<PendingAvatarTask>,
) {
    for ev in events.read() {
        // Cancel any in-flight task and restart with the new URL.
        let url = ev.url.clone();
        let rt = rt.0.clone();
        pending.0 = Some(AsyncComputeTaskPool::get().spawn(async move {
            rt.block_on(async move {
                let client = reqwest::Client::new();
                let bytes = client
                    .get(&url)
                    .send()
                    .await
                    .ok()?
                    .bytes()
                    .await
                    .ok()?;
                Some(bytes.to_vec())
            })
        }));
    }
}

fn poll_avatar_task(
    mut pending: ResMut<PendingAvatarTask>,
    mut avatar: ResMut<AvatarResource>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(task) = pending.0.as_mut() else {
        return;
    };
    let Some(result) = future::block_on(future::poll_once(task)) else {
        return;
    };
    pending.0 = None;

    let Some(bytes) = result else { return };

    match image::load_from_memory(&bytes) {
        Ok(dyn_img) => {
            let bevy_img = Image::from_dynamic(dyn_img, true, RenderAssetUsages::RENDER_WORLD);
            let handle = images.add(bevy_img);
            avatar.0 = Some(handle);
        }
        Err(e) => {
            warn!("avatar_plugin: failed to decode avatar image: {e}");
        }
    }
}
