//! Sound-effect playback via `kira`.
//!
//! Loads five embedded WAVs (`include_bytes!`) at startup and plays them in
//! response to gameplay events:
//!
//! | Event | Sound |
//! |---|---|
//! | `DrawRequestEvent` | `card_flip.wav` |
//! | `MoveRequestEvent` | `card_place.wav` |
//! | `MoveRejectedEvent` | `card_invalid.wav` |
//! | `NewGameRequestEvent` | `card_deal.wav` |
//! | `GameWonEvent` | `win_fanfare.wav` |
//!
//! If the audio device cannot be opened (e.g. a headless CI machine or a
//! Linux box without a running PulseAudio/Pipewire session), the plugin
//! logs a warning and degrades gracefully — gameplay continues, just
//! silently.

use std::io::Cursor;

use bevy::prelude::*;
use kira::manager::backend::DefaultBackend;
use kira::manager::{AudioManager, AudioManagerSettings};
use kira::sound::static_sound::StaticSoundData;
use kira::track::{TrackBuilder, TrackHandle};
use kira::tween::Tween;

use crate::events::{
    CardFlippedEvent, DrawRequestEvent, GameWonEvent, MoveRejectedEvent, MoveRequestEvent,
    NewGameRequestEvent, UndoRequestEvent,
};
use crate::pause_plugin::PausedResource;
use crate::settings_plugin::{SettingsChangedEvent, SettingsResource};

/// Pre-decoded sound effects. Cheap to clone (frames are an `Arc<[Frame]>`),
/// so we hand a fresh handle to `manager.play()` on every event.
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
    /// No sounds are currently routed here; the track exists so future ambient
    /// music can be added without changing the volume architecture.
    music_track: Option<TrackHandle>,
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

        let (sfx_track, music_track) = match manager.as_mut() {
            Some(mgr) => {
                let sfx = mgr.add_sub_track(TrackBuilder::default()).ok();
                let music = mgr.add_sub_track(TrackBuilder::default()).ok();
                (sfx, music)
            }
            None => (None, None),
        };

        app.insert_non_send_resource(AudioState { manager, sfx_track, music_track })
            .init_resource::<MuteState>();

        let library = build_library();
        if let Some(lib) = library {
            app.insert_resource(lib);
        } else {
            warn!("failed to decode embedded SFX assets; SFX disabled");
        }

        app.add_event::<DrawRequestEvent>()
            .add_event::<MoveRequestEvent>()
            .add_event::<MoveRejectedEvent>()
            .add_event::<NewGameRequestEvent>()
            .add_event::<GameWonEvent>()
            .add_event::<CardFlippedEvent>()
            .add_event::<UndoRequestEvent>()
            .add_event::<SettingsChangedEvent>()
            .add_systems(
                Startup,
                apply_initial_volume,
            )
            .add_systems(
                Update,
                (
                    play_on_draw,
                    play_on_move,
                    play_on_rejected,
                    play_on_new_game,
                    play_on_win,
                    play_on_card_flip,
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

fn play(audio: &mut AudioState, sound: &StaticSoundData) {
    let Some(manager) = audio.manager.as_mut() else {
        return;
    };
    // Route SFX through the dedicated sfx_track so its volume is independent
    // of the music_track volume.
    let mut data = sound.clone();
    if let Some(track) = &audio.sfx_track {
        data.settings.output_destination = track.id().into();
    }
    if let Err(e) = manager.play(data) {
        warn!("failed to play SFX: {e}");
    }
}

fn set_sfx_volume(audio: &mut AudioState, volume: f32) {
    if let Some(track) = audio.sfx_track.as_mut() {
        track.set_volume(volume.clamp(0.0, 1.0) as f64, Tween::default());
    }
}

fn set_music_volume(audio: &mut AudioState, volume: f32) {
    if let Some(track) = audio.music_track.as_mut() {
        track.set_volume(volume.clamp(0.0, 1.0) as f64, Tween::default());
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
    mut events: EventReader<UndoRequestEvent>,
    mut audio: NonSendMut<AudioState>,
    lib: Option<Res<SoundLibrary>>,
) {
    let Some(lib) = lib else { return };
    for _ in events.read() {
        play(&mut audio, &lib.flip);
    }
}

fn apply_volume_on_change(
    mut events: EventReader<SettingsChangedEvent>,
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
    mut events: EventReader<DrawRequestEvent>,
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

fn play_on_move(
    mut events: EventReader<MoveRequestEvent>,
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
    mut events: EventReader<MoveRejectedEvent>,
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
    mut events: EventReader<NewGameRequestEvent>,
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
    mut events: EventReader<GameWonEvent>,
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

fn play_on_card_flip(
    mut events: EventReader<CardFlippedEvent>,
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
}
