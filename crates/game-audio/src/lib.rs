//! Shared game audio director with thin native and browser backends.
//!
//! The [`Director`] owns every audio *decision*: which cue plays, on which bus, with which
//! clip variant, gain, pan and playback rate, which voice is stolen when the pool is full,
//! which re-triggers fall inside a cooldown, and which cue keys were already heard (so a
//! replayed snapshot never replays audio). It is pure and deterministic: it turns calls into
//! [`Command`]s and never touches an audio device.
//!
//! A [`Backend`] only executes commands:
//!
//! - [`RecordingBackend`] records them (tests, headless servers).
//! - `KiraBackend` (feature `kira`, native targets) plays them through kira on cpal.
//! - `WebAudioBackend` (feature `web`, `wasm32`) plays them through the browser's Web Audio
//!   API, so mixing runs on the browser audio thread.
//!
//! Persisted volume and mute preferences belong to the `settings` repository; this crate
//! only maps them ([`AudioSettings`], [`AudioSettingsCommand`], [`settings::keys`]).

pub mod backend;
mod director;
mod manifest;
pub mod settings;
mod spatial;
mod tone;

pub use backend::{Backend, Command, RecordingBackend};
pub use director::{CueEvent, Director, DirectorConfig, TriggerOutcome, VoiceInfo};
pub use manifest::{
    Attenuation, ClipSource, ClipSpec, CueSpec, ManifestError, SoundManifest, VariantMode,
};
pub use settings::{AudioSettings, AudioSettingsCommand};
pub use spatial::{Listener, distance_gain, spatialize, stereo_pan};
pub use tone::{Tone, Waveform};

#[cfg(all(feature = "kira", not(target_arch = "wasm32")))]
pub use backend::kira::{KiraBackend, KiraBackendError};
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub use backend::web::{AudioState, WebAudioBackend};

use serde::{Deserialize, Serialize};

/// A mixer bus. `Effects` and `Ui` both feed `Master`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bus {
    Master,
    #[default]
    Effects,
    Ui,
}

impl Bus {
    pub const ALL: [Bus; 3] = [Bus::Master, Bus::Effects, Bus::Ui];
}

/// Index of a clip in the manifest's `clips` list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ClipId(pub u32);

/// A playing sound instance. Ids are never reused within one director.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct VoiceId(pub u64);

/// A game entity a voice follows (position updates) and can be stopped with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityId(pub u64);

/// Converts decibels to a linear amplitude factor.
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Converts a linear amplitude factor to decibels (`-inf` maps to `f32::NEG_INFINITY`).
pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * gain.log10()
    }
}
