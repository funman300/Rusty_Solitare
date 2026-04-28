//! Sound-effect playback via `kira`.
//!
//! Loads five embedded WAVs (`include_bytes!`) at startup and plays them in
//! response to gameplay events:
//!
//! | Event | Sound |
//! |---|---|
//! | `DrawRequestEvent` | `card_flip.wav` (recycle: 0.5× volume) |
//! | `MoveRequestEvent` | `card_place.wav` |
//! | `MoveRejectedEvent` | `card_invalid.wav` |
//! | `NewGameRequestEvent` | `card_deal.wav` |
//! | `GameWonEvent` | `win_fanfare.wav` |
//!
//! An ambient loop is started at plugin startup using `card_flip.wav` at very
//! low volume (0.05 amplitude) routed through `music_track` as a placeholder
//! until a dedicated ambient track is available.
//!
//! If the audio device cannot be opened (e.g. a headless CI machine or a
//! Linux box without a running PulseAudio/Pipewire session), the plugin
//! logs a warning and degrades gracefully — gameplay continues, just
//! silently.

use std::io::Cursor;

use bevy::prelude::*;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::sound::Region;
use kira::track::{TrackBuilder, TrackHandle};
use kira::{AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Tween, Value};

use crate::events::{
    CardFaceRevealedEvent, CardFlippedEvent, DrawRequestEvent, GameWonEvent, MoveRejectedEvent,
    MoveRequestEvent, NewGameRequestEvent, UndoRequestEvent,
};
use crate::pause_plugin::PausedResource;
use crate::resources::GameStateResource;
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource};
use solitaire_core::pile::PileType;

/// Volume amplitude for the stock-recycle draw sound (half of normal 1.0).
const RECYCLE_VOLUME: f64 = 0.5;

/// Volume amplitude for the ambient music loop placeholder.
const AMBIENT_VOLUME: f64 = 0.05;

/// Converts a linear amplitude (0.0–1.0+) to the `Decibels` type used by
/// kira 0.12. Clamps to `Decibels::SILENCE` for non-positive amplitudes.
fn amplitude_to_decibels(amplitude: f32) -> Decibels {
    if amplitude <= 0.0 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * amplitude.log10())
    }
}

/// Returns `true` when a `DrawRequestEvent` will recycle the waste pile back
/// to stock rather than drawing a new card.
///
/// This is a pure function with no side effects — it can be called from tests
/// without an audio device or Bevy world.
fn is_recycle(stock_len: usize) -> bool {
    stock_len == 0
}

/// Pre-decoded sound effects. Cheap to clone (frames are an `Arc<[Frame]>`),
/// so we hand a fresh handle to `track.play()` on every event.
#[derive(Resource, Clone)]
pub struct SoundLibrary {
    pub deal: StaticSoundData,
    pub flip: StaticSoundData,
    pub place: StaticSoundData,
    pub invalid: StaticSoundData,
    pub fanfare: StaticSoundData,
}

/// Wraps the audio backend. `NonSend` because cpal streams are `!Send` on
/// some platforms.
pub struct AudioState {
    manager: Option<AudioManager<DefaultBackend>>,
    /// Dedicated sub-track for sound effects. Volume controlled by `sfx_volume`.
    sfx_track: Option<TrackHandle>,
    /// Dedicated sub-track for ambient music. Volume controlled by `music_volume`.
    music_track: Option<TrackHandle>,
    /// Handle to the looping ambient track so it can be paused or stopped later.
    #[allow(dead_code)]
    ambient_handle: Option<StaticSoundHandle>,
}

/// Tracks which audio channels the player has silenced via the M / Shift+M shortcuts.
///
/// These booleans override the `sfx_volume` / `music_volume` settings.  When
/// `true`, the corresponding track is forced to 0. When toggled back to `false`
/// the volume is restored from `SettingsResource`.
#[derive(Resource, Default)]
pub struct MuteState {
    pub sfx_muted: bool,
    pub music_muted: bool,
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        let mut manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()).ok();
        if manager.is_none() {
            warn!("audio device unavailable; SFX disabled");
        }

        let library = build_library();
        if library.is_none() {
            warn!("failed to decode embedded SFX assets; SFX disabled");
        }

        let (sfx_track, mut music_track) = match manager.as_mut() {
            Some(mgr) => {
                let sfx = mgr.add_sub_track(TrackBuilder::default()).ok();
                let music = mgr.add_sub_track(TrackBuilder::default()).ok();
                (sfx, music)
            }
            None => (None, None),
        };

        // Start the ambient loop placeholder (card_flip.wav looped at very low
        // volume through music_track).
        let ambient_handle =
            start_ambient_loop(manager.as_mut(), library.as_ref(), &mut music_track);

        app.insert_non_send_resource(AudioState {
            manager,
            sfx_track,
            music_track,
            ambient_handle,
        })
        .init_resource::<MuteState>();

        if let Some(lib) = library {
            app.insert_resource(lib);
        }

        app.add_message::<DrawRequestEvent>()
            .add_message::<MoveRequestEvent>()
            .add_message::<MoveRejectedEvent>()
            .add_message::<NewGameRequestEvent>()
            .add_message::<GameWonEvent>()
            .add_message::<CardFlippedEvent>()
            .add_message::<CardFaceRevealedEvent>()
            .add_message::<UndoRequestEvent>()
            .add_message::<SettingsChangedEvent>()
            .add_systems(Startup, apply_initial_volume)
            .add_systems(
                Update,
                (
                    play_on_draw,
                    play_on_move,
                    play_on_rejected,
                    play_on_new_game,
                    play_on_win,
                    play_on_face_revealed,
                    play_on_undo,
                    apply_volume_on_change,
                    handle_mute_keys,
                ),
            );
    }
}

fn build_library() -> Option<SoundLibrary> {
    let deal = decode(include_bytes!("../../assets/audio/card_deal.wav"))?;
    let flip = decode(include_bytes!("../../assets/audio/card_flip.wav"))?;
    let place = decode(include_bytes!("../../assets/audio/card_place.wav"))?;
    let invalid = decode(include_bytes!("../../assets/audio/card_invalid.wav"))?;
    let fanfare = decode(include_bytes!("../../assets/audio/win_fanfare.wav"))?;
    Some(SoundLibrary {
        deal,
        flip,
        place,
        invalid,
        fanfare,
    })
}

fn decode(bytes: &'static [u8]) -> Option<StaticSoundData> {
    match StaticSoundData::from_cursor(Cursor::new(bytes.to_vec())) {
        Ok(data) => Some(data),
        Err(e) => {
            warn!("failed to decode SFX: {e}");
            None
        }
    }
}

/// Starts the ambient music loop placeholder (`card_flip.wav` looped at very
/// low volume) routed through `music_track`. Returns the handle so it can be
/// stored in `AudioState` for future pause/stop control.
///
/// Returns `None` when audio is unavailable or the library failed to load.
fn start_ambient_loop(
    manager: Option<&mut AudioManager<DefaultBackend>>,
    library: Option<&SoundLibrary>,
    music_track: &mut Option<TrackHandle>,
) -> Option<StaticSoundHandle> {
    let manager = manager?;
    let lib = library?;

    let mut data = lib.flip.clone();
    data.settings.loop_region = Some(Region::default());
    data.settings.volume = Value::Fixed(amplitude_to_decibels(AMBIENT_VOLUME as f32));

    let result = if let Some(track) = music_track.as_mut() {
        track.play(data)
    } else {
        manager.play(data)
    };

    match result {
        Ok(handle) => Some(handle),
        Err(e) => {
            warn!("failed to start ambient loop: {e}");
            None
        }
    }
}

fn play(audio: &mut AudioState, sound: &StaticSoundData) {
    let data = sound.clone();
    // Route SFX through the dedicated sfx_track so its volume is independent
    // of the music_track volume.
    let result = if let Some(track) = audio.sfx_track.as_mut() {
        track.play(data)
    } else if let Some(manager) = audio.manager.as_mut() {
        manager.play(data)
    } else {
        return;
    };
    if let Err(e) = result {
        warn!("failed to play SFX: {e}");
    }
}

impl AudioState {
    /// Plays `sound` through the SFX sub-track at `volume` amplitude (0.0–1.0+).
    ///
    /// Behaves identically to the crate-private `play()` function but accepts an
    /// explicit volume override so callers can play sounds at a fraction of their
    /// normal level. Silently does nothing when audio is unavailable.
    pub fn play_sfx_at_volume(&mut self, sound: &StaticSoundData, volume: f64) {
        let mut data = sound.clone();
        data.settings.volume = Value::Fixed(amplitude_to_decibels(volume as f32));

        let result = if let Some(track) = self.sfx_track.as_mut() {
            track.play(data)
        } else if let Some(manager) = self.manager.as_mut() {
            manager.play(data)
        } else {
            return;
        };
        if let Err(e) = result {
            warn!("failed to play SFX at volume {volume}: {e}");
        }
    }
}

fn set_sfx_volume(audio: &mut AudioState, volume: f32) {
    if let Some(track) = audio.sfx_track.as_mut() {
        track.set_volume(amplitude_to_decibels(volume.clamp(0.0, 1.0)), Tween::default());
    }
}

fn set_music_volume(audio: &mut AudioState, volume: f32) {
    if let Some(track) = audio.music_track.as_mut() {
        track.set_volume(amplitude_to_decibels(volume.clamp(0.0, 1.0)), Tween::default());
    }
}

fn apply_initial_volume(
    mut audio: NonSendMut<AudioState>,
    settings: Option<Res<SettingsResource>>,
) {
    let (sfx, music) = settings.map_or((1.0, 0.5), |s| (s.0.sfx_volume, s.0.music_volume));
    set_sfx_volume(&mut audio, sfx);
    set_music_volume(&mut audio, music);
}

fn play_on_undo(
    mut events: MessageReader<UndoRequestEvent>,
    mut audio: NonSendMut<AudioState>,
    lib: Option<Res<SoundLibrary>>,
) {
    let Some(lib) = lib else { return };
    for _ in events.read() {
        play(&mut audio, &lib.flip);
    }
}

fn apply_volume_on_change(
    mut events: MessageReader<SettingsChangedEvent>,
    mut audio: NonSendMut<AudioState>,
    mute: Option<Res<MuteState>>,
) {
    for ev in events.read() {
        let sfx_muted = mute.as_ref().is_some_and(|m| m.sfx_muted);
        let music_muted = mute.as_ref().is_some_and(|m| m.music_muted);
        set_sfx_volume(&mut audio, if sfx_muted { 0.0 } else { ev.0.sfx_volume });
        set_music_volume(&mut audio, if music_muted { 0.0 } else { ev.0.music_volume });
    }
}

/// `M` toggles mute for all audio; `Shift+M` toggles music only.
/// Volumes are restored from `SettingsResource` on unmute.
fn handle_mute_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut audio: NonSendMut<AudioState>,
    mut mute: ResMut<MuteState>,
    settings: Option<Res<SettingsResource>>,
    paused: Option<Res<PausedResource>>,
) {
    if paused.is_some_and(|p| p.0) || !keys.just_pressed(KeyCode::KeyM) {
        return;
    }
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let (sfx_vol, music_vol) = settings
        .as_ref()
        .map(|s| (s.0.sfx_volume, s.0.music_volume))
        .unwrap_or((1.0, 0.5));

    if shift {
        // Shift+M: toggle music mute only, SFX unaffected.
        mute.music_muted = !mute.music_muted;
    } else {
        // M: mute all if either channel is audible; unmute all otherwise.
        let new_state = !(mute.sfx_muted && mute.music_muted);
        mute.sfx_muted = new_state;
        mute.music_muted = new_state;
    }

    set_sfx_volume(&mut audio, if mute.sfx_muted { 0.0 } else { sfx_vol });
    set_music_volume(&mut audio, if mute.music_muted { 0.0 } else { music_vol });
}

fn play_on_draw(
    mut events: MessageReader<DrawRequestEvent>,
    mut audio: NonSendMut<AudioState>,
    lib: Option<Res<SoundLibrary>>,
    game: Option<Res<GameStateResource>>,
) {
    let Some(lib) = lib else {
        return;
    };
    for _ in events.read() {
        // When the stock pile is empty the draw action recycles the waste pile
        // back to stock. Play the flip sound at half volume to give audible
        // feedback that distinguishes a recycle from a normal draw.
        let stock_len = game
            .as_ref()
            .and_then(|g| g.0.piles.get(&PileType::Stock))
            .map_or(1, |p| p.cards.len()); // default > 0 → normal draw sound

        if is_recycle(stock_len) {
            let mut data = lib.flip.clone();
            data.settings.volume =
                Value::Fixed(amplitude_to_decibels(RECYCLE_VOLUME as f32));
            let result = if let Some(track) = audio.sfx_track.as_mut() {
                track.play(data)
            } else if let Some(manager) = audio.manager.as_mut() {
                manager.play(data)
            } else {
                continue;
            };
            if let Err(e) = result {
                warn!("failed to play recycle SFX: {e}");
            }
        } else {
            play(&mut audio, &lib.flip);
        }
    }
}

fn play_on_move(
    mut events: MessageReader<MoveRequestEvent>,
    mut audio: NonSendMut<AudioState>,
    lib: Option<Res<SoundLibrary>>,
) {
    let Some(lib) = lib else {
        return;
    };
    for _ in events.read() {
        play(&mut audio, &lib.place);
    }
}

fn play_on_rejected(
    mut events: MessageReader<MoveRejectedEvent>,
    mut audio: NonSendMut<AudioState>,
    lib: Option<Res<SoundLibrary>>,
) {
    let Some(lib) = lib else {
        return;
    };
    for _ in events.read() {
        play(&mut audio, &lib.invalid);
    }
}

fn play_on_new_game(
    mut events: MessageReader<NewGameRequestEvent>,
    mut audio: NonSendMut<AudioState>,
    lib: Option<Res<SoundLibrary>>,
) {
    let Some(lib) = lib else {
        return;
    };
    for _ in events.read() {
        play(&mut audio, &lib.deal);
    }
}

fn play_on_win(
    mut events: MessageReader<GameWonEvent>,
    mut audio: NonSendMut<AudioState>,
    lib: Option<Res<SoundLibrary>>,
) {
    let Some(lib) = lib else {
        return;
    };
    for _ in events.read() {
        play(&mut audio, &lib.fanfare);
    }
}

/// Plays the card-flip sound at the animation midpoint — the instant the face
/// is visually revealed — keeping audio and visuals in sync.
///
/// Driven by `CardFaceRevealedEvent`, which is fired by `tick_flip_anim` at
/// the phase transition (scale.x crosses 0), not by the move event itself.
fn play_on_face_revealed(
    mut events: MessageReader<CardFaceRevealedEvent>,
    mut audio: NonSendMut<AudioState>,
    lib: Option<Res<SoundLibrary>>,
) {
    let Some(lib) = lib else {
        return;
    };
    for _ in events.read() {
        play(&mut audio, &lib.flip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_wavs_decode_successfully() {
        // Verifies the include_bytes! paths resolve and the bytes are valid
        // WAV (so the gen_sfx output stays in sync with the loader).
        let lib = build_library();
        assert!(lib.is_some(), "embedded SFX failed to decode");
    }

    // -----------------------------------------------------------------------
    // MuteState toggle logic (pure, no AudioManager needed)
    // -----------------------------------------------------------------------

    /// Helper that mirrors the toggle logic inside `handle_mute_keys`
    /// for M (mute-all).
    fn toggle_all(mute: &mut MuteState) {
        let new_state = !(mute.sfx_muted && mute.music_muted);
        mute.sfx_muted = new_state;
        mute.music_muted = new_state;
    }

    /// Helper that mirrors the toggle logic for Shift+M (music-only).
    fn toggle_music(mute: &mut MuteState) {
        mute.music_muted = !mute.music_muted;
    }

    #[test]
    fn mute_all_toggles_both_channels() {
        let mut m = MuteState::default();
        toggle_all(&mut m);
        assert!(m.sfx_muted && m.music_muted, "M should mute both channels");
        toggle_all(&mut m);
        assert!(!m.sfx_muted && !m.music_muted, "second M should unmute both channels");
    }

    #[test]
    fn shift_m_toggles_music_only() {
        let mut m = MuteState::default();
        toggle_music(&mut m);
        assert!(m.music_muted, "Shift+M should mute music");
        assert!(!m.sfx_muted, "Shift+M must not mute SFX");
        toggle_music(&mut m);
        assert!(!m.music_muted, "second Shift+M should unmute music");
    }

    #[test]
    fn mute_all_while_music_already_muted_mutes_sfx_too() {
        let mut m = MuteState::default();
        // Music already muted via Shift+M.
        toggle_music(&mut m);
        assert!(m.music_muted && !m.sfx_muted);
        // M should mute sfx (not-all-muted → mute-all).
        toggle_all(&mut m);
        assert!(m.sfx_muted && m.music_muted, "M unmutes neither — it mutes all when sfx was audible");
    }

    #[test]
    fn mute_all_when_both_already_muted_unmutes_both() {
        let mut m = MuteState { sfx_muted: true, music_muted: true };
        toggle_all(&mut m);
        assert!(!m.sfx_muted && !m.music_muted, "M should unmute both when all were muted");
    }

    // -----------------------------------------------------------------------
    // Task #60 — stock-recycle detection (pure, no audio hardware needed)
    // -----------------------------------------------------------------------

    /// The recycle volume constant must be exactly half of normal (1.0).
    #[test]
    fn recycle_volume_is_half_normal() {
        assert!((RECYCLE_VOLUME - 0.5).abs() < f64::EPSILON);
    }

    /// `is_recycle` returns `true` only when the stock pile is empty.
    #[test]
    fn stock_empty_means_recycle() {
        assert!(is_recycle(0), "empty stock should trigger recycle");
        assert!(!is_recycle(1), "non-empty stock must not trigger recycle");
    }

    // -----------------------------------------------------------------------
    // Task #61 — AudioState has ambient_handle slot (compile-time check)
    // -----------------------------------------------------------------------

    /// Verifies that `AudioState` exposes an `ambient_handle` field of the
    /// correct type.  No real `AudioManager` is created; the field is set to
    /// `None` to avoid requiring audio hardware in CI.
    #[test]
    fn audio_state_has_music_track_slot() {
        let state = AudioState {
            manager: None,
            sfx_track: None,
            music_track: None,
            ambient_handle: None,
        };
        // The assertion is intentionally trivial — the real check is that this
        // code compiles, confirming the field exists with the expected type.
        assert!(state.ambient_handle.is_none());
    }
}
