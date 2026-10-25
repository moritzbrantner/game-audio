//! Native backend: kira on cpal.
//!
//! Buses are kira sub-tracks: `master` is a sub-track of kira's main track, `effects` and
//! `ui` are children of `master`. `Suspend` pauses `master`.

use std::collections::HashMap;
use std::fmt;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use kira::sound::PlaybackState;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle, StaticSoundSettings};
use kira::track::{TrackBuilder, TrackHandle};
use kira::{
    AudioManager, AudioManagerSettings, Decibels, DefaultBackend, Frame, Panning, PlaybackRate,
    Tween,
};

use crate::backend::{Backend, Command};
use crate::manifest::ClipSource;
use crate::{Bus, ClipId, VoiceId, gain_to_db};

const TONE_SAMPLE_RATE: u32 = 48_000;

#[derive(Debug)]
pub enum KiraBackendError {
    /// The audio device could not be opened.
    Device(String),
    /// kira ran out of tracks.
    Tracks(String),
    /// A clip failed to load; its starts are reported finished immediately.
    Clip { clip: ClipId, error: String },
}

impl fmt::Display for KiraBackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(error) => write!(f, "audio device unavailable: {error}"),
            Self::Tracks(error) => write!(f, "could not create mixer tracks: {error}"),
            Self::Clip { clip, error } => write!(f, "clip {} failed to load: {error}", clip.0),
        }
    }
}

impl std::error::Error for KiraBackendError {}

pub struct KiraBackend {
    // Field order matters: tracks and voices drop before the manager.
    voices: HashMap<VoiceId, StaticSoundHandle>,
    effects: TrackHandle,
    ui: TrackHandle,
    master: TrackHandle,
    _manager: AudioManager<DefaultBackend>,
    clips: HashMap<ClipId, StaticSoundData>,
    finished: Vec<VoiceId>,
    asset_root: PathBuf,
    embedded: HashMap<String, &'static [u8]>,
    errors: Vec<KiraBackendError>,
}

impl KiraBackend {
    /// Opens the default output device. `Url` clip sources resolve relative to the current
    /// directory; see [`KiraBackend::with_asset_root`].
    pub fn new() -> Result<Self, KiraBackendError> {
        Self::with_settings(AudioManagerSettings::default())
    }

    pub fn with_settings(
        settings: AudioManagerSettings<DefaultBackend>,
    ) -> Result<Self, KiraBackendError> {
        let mut manager = AudioManager::<DefaultBackend>::new(settings)
            .map_err(|error| KiraBackendError::Device(error.to_string()))?;
        let tracks =
            |error: kira::ResourceLimitReached| KiraBackendError::Tracks(error.to_string());
        let mut master = manager.add_sub_track(TrackBuilder::new()).map_err(tracks)?;
        let effects = master.add_sub_track(TrackBuilder::new()).map_err(tracks)?;
        let ui = master.add_sub_track(TrackBuilder::new()).map_err(tracks)?;
        Ok(Self {
            voices: HashMap::new(),
            effects,
            ui,
            master,
            _manager: manager,
            clips: HashMap::new(),
            finished: Vec::new(),
            asset_root: PathBuf::new(),
            embedded: HashMap::new(),
            errors: Vec::new(),
        })
    }

    /// Resolves relative `Url` clip sources against `root`.
    pub fn with_asset_root(mut self, root: impl AsRef<Path>) -> Self {
        self.asset_root = root.as_ref().to_path_buf();
        self
    }

    /// Serves `Url` clip sources from bytes compiled into the binary (e.g. `include_bytes!`),
    /// keyed by the manifest URL exactly. Other URLs still load from the asset root.
    pub fn with_embedded_clips<S: Into<String>>(
        mut self,
        files: impl IntoIterator<Item = (S, &'static [u8])>,
    ) -> Self {
        self.embedded
            .extend(files.into_iter().map(|(url, bytes)| (url.into(), bytes)));
        self
    }

    /// Clip load failures since the last call. Playback continues without those clips.
    pub fn take_errors(&mut self) -> Vec<KiraBackendError> {
        std::mem::take(&mut self.errors)
    }

    fn track(&mut self, bus: Bus) -> &mut TrackHandle {
        match bus {
            Bus::Master => &mut self.master,
            Bus::Effects => &mut self.effects,
            Bus::Ui => &mut self.ui,
        }
    }

    fn load(&mut self, clip: ClipId, source: ClipSource) {
        let data = match source {
            ClipSource::Tone(tone) => {
                let frames: Arc<[Frame]> = tone
                    .render(TONE_SAMPLE_RATE)
                    .into_iter()
                    .map(|sample| Frame {
                        left: sample,
                        right: sample,
                    })
                    .collect();
                Ok(StaticSoundData {
                    sample_rate: TONE_SAMPLE_RATE,
                    frames,
                    settings: StaticSoundSettings::default(),
                    slice: None,
                })
            }
            ClipSource::Url(path) => match self.embedded.get(path.as_str()) {
                Some(bytes) => StaticSoundData::from_cursor(Cursor::new(*bytes)),
                None => StaticSoundData::from_file(self.asset_root.join(path)),
            }
            .map_err(|e| e.to_string()),
        };
        match data {
            Ok(data) => {
                self.clips.insert(clip, data);
            }
            Err(error) => self.errors.push(KiraBackendError::Clip { clip, error }),
        }
    }
}

fn decibels(gain: f32) -> Decibels {
    Decibels(gain_to_db(gain).max(Decibels::SILENCE.0))
}

fn fade(ms: u32) -> Tween {
    Tween {
        duration: Duration::from_millis(u64::from(ms)),
        ..Tween::default()
    }
}

impl Backend for KiraBackend {
    fn apply(&mut self, command: Command) {
        match command {
            Command::Load { clip, source } => self.load(clip, source),
            Command::Start {
                voice,
                clip,
                bus,
                gain,
                pan,
                rate,
            } => {
                let Some(data) = self.clips.get(&clip) else {
                    self.finished.push(voice);
                    return;
                };
                let data = data
                    .volume(decibels(gain))
                    .panning(Panning(pan))
                    .playback_rate(PlaybackRate(f64::from(rate)));
                match self.track(bus).play(data) {
                    Ok(handle) => {
                        self.voices.insert(voice, handle);
                    }
                    Err(_) => self.finished.push(voice),
                }
            }
            Command::Update { voice, gain, pan } => {
                if let Some(handle) = self.voices.get_mut(&voice) {
                    handle.set_volume(decibels(gain), fade(20));
                    handle.set_panning(Panning(pan), fade(20));
                }
            }
            Command::Stop { voice, fade_ms } => {
                if let Some(mut handle) = self.voices.remove(&voice) {
                    handle.stop(fade(fade_ms));
                }
            }
            Command::SetBusGain { bus, gain, fade_ms } => {
                self.track(bus).set_volume(decibels(gain), fade(fade_ms));
            }
            Command::Suspend => self.master.pause(fade(20)),
            Command::Resume => self.master.resume(fade(20)),
        }
    }

    fn finished_voices(&mut self) -> Vec<VoiceId> {
        self.voices.retain(|voice, handle| {
            let done = handle.state() == PlaybackState::Stopped;
            if done {
                self.finished.push(*voice);
            }
            !done
        });
        std::mem::take(&mut self.finished)
    }
}
