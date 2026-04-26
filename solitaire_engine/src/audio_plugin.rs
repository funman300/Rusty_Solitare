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

use crate::events::{
    DrawRequestEvent, GameWonEvent, MoveRejectedEvent, MoveRequestEvent, NewGameRequestEvent,
};

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
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()).ok();
        if manager.is_none() {
            warn!("audio device unavailable; SFX disabled");
        }
        app.insert_non_send_resource(AudioState { manager });

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
            .add_systems(
                Update,
                (
                    play_on_draw,
                    play_on_move,
                    play_on_rejected,
                    play_on_new_game,
                    play_on_win,
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
    if let Err(e) = manager.play(sound.clone()) {
        warn!("failed to play SFX: {e}");
    }
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
}
