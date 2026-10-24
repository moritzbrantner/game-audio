use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Bus;
use crate::tone::Tone;

/// The sound manifest: clips, cues and the distance-fade curve. Usually loaded from JSON.
///
/// ```json
/// {
///   "attenuation": { "min_distance": 2, "max_distance": 40 },
///   "clips": [
///     { "id": "hit-1", "source": { "url": "sounds/hit-1.ogg" } },
///     { "id": "beep", "source": { "tone": { "frequency_hz": 880, "duration_ms": 80 } } }
///   ],
///   "cues": [
///     { "kind": "combat.hit", "clips": ["hit-1"], "priority": 80, "max_voices": 4,
///       "cooldown_ms": 40, "gain_db": -3, "pitch_jitter": 0.05, "spatial": true },
///     { "kind": "ui.click", "clips": ["beep"], "bus": "ui" }
///   ]
/// }
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundManifest {
    #[serde(default)]
    pub attenuation: Attenuation,
    pub clips: Vec<ClipSpec>,
    pub cues: Vec<CueSpec>,
}

/// One decodable or generated clip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipSpec {
    pub id: String,
    pub source: ClipSource,
}

/// Where a clip's samples come from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipSource {
    /// A file path (native) or URL (web), resolved by the backend.
    Url(String),
    /// A generated test tone; needs no file. Useful for placeholders and examples.
    Tone(Tone),
}

/// How a cue picks among its clip variants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantMode {
    #[default]
    RoundRobin,
    /// Seeded by [`crate::DirectorConfig::seed`], so runs are reproducible.
    Random,
}

/// A cue: what a gameplay event kind sounds like and how it competes for voices.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CueSpec {
    /// Gameplay cue kind, e.g. `combat.hit`.
    pub kind: String,
    /// Clip ids (variants).
    pub clips: Vec<String>,
    #[serde(default)]
    pub variants: VariantMode,
    #[serde(default)]
    pub bus: Bus,
    /// Higher wins when the global voice pool is full.
    #[serde(default = "default_priority")]
    pub priority: u8,
    /// Per-cue voice cap; the cue's oldest voice is stolen when it is reached.
    #[serde(default = "default_max_voices")]
    pub max_voices: u8,
    /// Re-triggers of this cue kind inside the window are dropped.
    #[serde(default)]
    pub cooldown_ms: u32,
    #[serde(default)]
    pub gain_db: f32,
    /// Playback rate is `1 ± pitch_jitter` (uniform, seeded).
    #[serde(default)]
    pub pitch_jitter: f32,
    /// Positional cues get distance fade and stereo pan; others play centred at full gain.
    #[serde(default)]
    pub spatial: bool,
}

fn default_priority() -> u8 {
    50
}

fn default_max_voices() -> u8 {
    4
}

impl CueSpec {
    /// A cue with default settings playing `clips` on the effects bus.
    pub fn new(kind: impl Into<String>, clips: &[&str]) -> Self {
        Self {
            kind: kind.into(),
            clips: clips.iter().map(|clip| (*clip).to_owned()).collect(),
            variants: VariantMode::default(),
            bus: Bus::default(),
            priority: default_priority(),
            max_voices: default_max_voices(),
            cooldown_ms: 0,
            gain_db: 0.0,
            pitch_jitter: 0.0,
            spatial: false,
        }
    }
}

/// Linear distance fade: full gain up to `min_distance`, silent from `max_distance`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attenuation {
    pub min_distance: f32,
    pub max_distance: f32,
}

impl Default for Attenuation {
    fn default() -> Self {
        Self {
            min_distance: 1.0,
            max_distance: 30.0,
        }
    }
}

#[derive(Debug)]
pub enum ManifestError {
    Json(serde_json::Error),
    DuplicateClip(String),
    DuplicateCue(String),
    UnknownClip { cue: String, clip: String },
    EmptyCue(String),
    ZeroVoices(String),
    InvalidAttenuation,
    InvalidTone(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "invalid manifest JSON: {error}"),
            Self::DuplicateClip(id) => write!(f, "duplicate clip id `{id}`"),
            Self::DuplicateCue(kind) => write!(f, "duplicate cue kind `{kind}`"),
            Self::UnknownClip { cue, clip } => {
                write!(f, "cue `{cue}` references unknown clip `{clip}`")
            }
            Self::EmptyCue(kind) => write!(f, "cue `{kind}` has no clips"),
            Self::ZeroVoices(kind) => write!(f, "cue `{kind}` has max_voices 0"),
            Self::InvalidAttenuation => {
                write!(f, "attenuation needs 0 <= min_distance < max_distance")
            }
            Self::InvalidTone(id) => write!(f, "clip `{id}` has an invalid tone"),
        }
    }
}

impl std::error::Error for ManifestError {}

impl SoundManifest {
    /// Parses and validates a JSON manifest.
    pub fn from_json(json: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_str(json).map_err(ManifestError::Json)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        let Attenuation {
            min_distance,
            max_distance,
        } = self.attenuation;
        if !(min_distance >= 0.0 && min_distance < max_distance && max_distance.is_finite()) {
            return Err(ManifestError::InvalidAttenuation);
        }
        let mut clips = HashSet::new();
        for clip in &self.clips {
            if !clips.insert(clip.id.as_str()) {
                return Err(ManifestError::DuplicateClip(clip.id.clone()));
            }
            if let ClipSource::Tone(tone) = &clip.source
                && !tone.is_valid()
            {
                return Err(ManifestError::InvalidTone(clip.id.clone()));
            }
        }
        let mut cues = HashSet::new();
        for cue in &self.cues {
            if !cues.insert(cue.kind.as_str()) {
                return Err(ManifestError::DuplicateCue(cue.kind.clone()));
            }
            if cue.clips.is_empty() {
                return Err(ManifestError::EmptyCue(cue.kind.clone()));
            }
            if cue.max_voices == 0 {
                return Err(ManifestError::ZeroVoices(cue.kind.clone()));
            }
            if let Some(clip) = cue.clips.iter().find(|clip| !clips.contains(clip.as_str())) {
                return Err(ManifestError::UnknownClip {
                    cue: cue.kind.clone(),
                    clip: clip.clone(),
                });
            }
        }
        Ok(())
    }
}
