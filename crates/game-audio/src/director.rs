use std::collections::{HashMap, HashSet, VecDeque};

use serde::{Deserialize, Serialize};

use crate::backend::{Backend, Command};
use crate::manifest::{SoundManifest, VariantMode};
use crate::settings::{AudioSettings, AudioSettingsCommand};
use crate::spatial::{Listener, spatialize};
use crate::{Bus, ClipId, EntityId, VoiceId, db_to_gain};

/// Tunables that are not per cue.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DirectorConfig {
    /// Global voice cap across all cues.
    pub max_voices: usize,
    /// Seed for random variants and pitch jitter.
    pub seed: u64,
    /// How many cue keys are remembered for dedup (oldest forgotten first).
    pub seen_key_capacity: usize,
    pub stop_fade_ms: u32,
    pub steal_fade_ms: u32,
    pub bus_fade_ms: u32,
}

impl Default for DirectorConfig {
    fn default() -> Self {
        Self {
            max_voices: 32,
            seed: 0x5EED_A0D1_0000_0001,
            seen_key_capacity: 4096,
            stop_fade_ms: 40,
            steal_fade_ms: 15,
            bus_fade_ms: 50,
        }
    }
}

/// A gameplay cue to sound, in the 67a presentation-cue shape.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CueEvent {
    /// Stable identity of the gameplay event. A key is sounded at most once; empty keys
    /// are never deduplicated.
    pub key: String,
    /// Cue kind from the manifest.
    pub kind: String,
    /// Entity the voice follows; also the position fallback.
    #[serde(default)]
    pub entity: Option<EntityId>,
    #[serde(default)]
    pub position: Option<[f32; 3]>,
}

impl CueEvent {
    pub fn new(key: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            kind: kind.into(),
            ..Self::default()
        }
    }

    pub fn at(mut self, position: [f32; 3]) -> Self {
        self.position = Some(position);
        self
    }

    pub fn on(mut self, entity: EntityId) -> Self {
        self.entity = Some(entity);
        self
    }
}

/// What [`Director::trigger`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerOutcome {
    Started {
        voice: VoiceId,
        stolen: Option<VoiceId>,
    },
    /// The cue key was already sounded (snapshot replay, resync).
    Duplicate,
    UnknownCue,
    Suspended,
    CoolingDown,
    /// Spatial source beyond `max_distance`.
    Inaudible,
    /// Pool full of voices with higher priority.
    Rejected,
}

/// A live voice, for diagnostics and tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoiceInfo {
    pub voice: VoiceId,
    pub priority: u8,
    pub bus: Bus,
    pub entity: Option<EntityId>,
    pub gain: f32,
    pub pan: f32,
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    id: VoiceId,
    cue: usize,
    priority: u8,
    seq: u64,
    bus: Bus,
    entity: Option<EntityId>,
    position: Option<[f32; 3]>,
    spatial: bool,
    base_gain: f32,
    gain: f32,
    pan: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct CueState {
    last_start_ms: Option<f64>,
    round_robin: usize,
}

struct CueRuntime {
    clips: Vec<ClipId>,
}

/// The audio director: all cue → voice decisions, no I/O. See the crate docs.
pub struct Director {
    manifest: SoundManifest,
    config: DirectorConfig,
    cue_index: HashMap<String, usize>,
    cue_runtime: Vec<CueRuntime>,
    cue_state: Vec<CueState>,
    voices: Vec<Voice>,
    next_voice: u64,
    next_seq: u64,
    listener: Listener,
    entity_positions: HashMap<EntityId, [f32; 3]>,
    settings: AudioSettings,
    suspended: bool,
    seen: HashSet<String>,
    seen_order: VecDeque<String>,
    rng: SplitMix64,
    commands: Vec<Command>,
}

const EPSILON: f32 = 1e-3;

impl Director {
    /// Creates a director for a validated manifest and queues one `Load` per clip plus the
    /// initial bus gains.
    pub fn new(manifest: SoundManifest, config: DirectorConfig) -> Self {
        let clip_ids: HashMap<&str, ClipId> = manifest
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| (clip.id.as_str(), ClipId(index as u32)))
            .collect();
        let cue_runtime = manifest
            .cues
            .iter()
            .map(|cue| CueRuntime {
                clips: cue
                    .clips
                    .iter()
                    .filter_map(|clip| clip_ids.get(clip.as_str()).copied())
                    .collect(),
            })
            .collect();
        let cue_index = manifest
            .cues
            .iter()
            .enumerate()
            .map(|(index, cue)| (cue.kind.clone(), index))
            .collect();
        let mut commands: Vec<Command> = manifest
            .clips
            .iter()
            .enumerate()
            .map(|(index, clip)| Command::Load {
                clip: ClipId(index as u32),
                source: clip.source.clone(),
            })
            .collect();
        let settings = AudioSettings::default();
        commands.extend(Bus::ALL.map(|bus| Command::SetBusGain {
            bus,
            gain: settings.bus_gain(bus),
            fade_ms: 0,
        }));
        Self {
            cue_state: vec![CueState::default(); manifest.cues.len()],
            manifest,
            config,
            cue_index,
            cue_runtime,
            voices: Vec::new(),
            next_voice: 1,
            next_seq: 0,
            listener: Listener::default(),
            entity_positions: HashMap::new(),
            settings,
            suspended: false,
            seen: HashSet::new(),
            seen_order: VecDeque::new(),
            rng: SplitMix64(config.seed),
            commands,
        }
    }

    pub fn manifest(&self) -> &SoundManifest {
        &self.manifest
    }

    pub fn settings(&self) -> AudioSettings {
        self.settings
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended
    }

    pub fn voices(&self) -> impl Iterator<Item = VoiceInfo> + '_ {
        self.voices.iter().map(|voice| VoiceInfo {
            voice: voice.id,
            priority: voice.priority,
            bus: voice.bus,
            entity: voice.entity,
            gain: voice.gain,
            pan: voice.pan,
        })
    }

    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    /// Sounds a cue unless dedup, suspension, cooldown, distance or the voice pool say no.
    /// `now_ms` is any monotonic clock (e.g. `performance.now()`).
    pub fn trigger(&mut self, event: &CueEvent, now_ms: f64) -> TriggerOutcome {
        if !event.key.is_empty() && !self.remember(&event.key) {
            return TriggerOutcome::Duplicate;
        }
        let Some(&cue_index) = self.cue_index.get(&event.kind) else {
            return TriggerOutcome::UnknownCue;
        };
        if self.suspended {
            return TriggerOutcome::Suspended;
        }
        let cue = &self.manifest.cues[cue_index];
        let state = self.cue_state[cue_index];
        if let Some(last) = state.last_start_ms
            && now_ms - last < f64::from(cue.cooldown_ms)
        {
            return TriggerOutcome::CoolingDown;
        }

        if let (Some(entity), Some(position)) = (event.entity, event.position) {
            self.entity_positions.insert(entity, position);
        }
        let position = event.position.or_else(|| {
            event
                .entity
                .and_then(|e| self.entity_positions.get(&e).copied())
        });
        let (spatial_gain, pan) = match (cue.spatial, position) {
            (true, Some(position)) => {
                spatialize(&self.listener, position, &self.manifest.attenuation)
            }
            _ => (1.0, 0.0),
        };
        if spatial_gain <= 0.0 {
            return TriggerOutcome::Inaudible;
        }

        let (priority, max_voices) = (cue.priority, usize::from(cue.max_voices));
        let same_cue = self.voices.iter().filter(|v| v.cue == cue_index).count();
        let victim = if same_cue >= max_voices {
            self.oldest(|v| v.cue == cue_index)
        } else if self.voices.len() >= self.config.max_voices.max(1) {
            match self.oldest_lowest_priority() {
                Some(index) if self.voices[index].priority <= priority => Some(index),
                _ => return TriggerOutcome::Rejected,
            }
        } else {
            None
        };
        let stolen = victim.map(|index| {
            let voice = self.voices.remove(index);
            self.commands.push(Command::Stop {
                voice: voice.id,
                fade_ms: self.config.steal_fade_ms,
            });
            voice.id
        });

        let cue = &self.manifest.cues[cue_index];
        let clips = &self.cue_runtime[cue_index].clips;
        let clip = match cue.variants {
            VariantMode::RoundRobin => clips[state.round_robin % clips.len()],
            VariantMode::Random => clips[self.rng.below(clips.len())],
        };
        let rate = if cue.pitch_jitter > 0.0 {
            1.0 + cue.pitch_jitter * self.rng.signed_unit()
        } else {
            1.0
        };
        let base_gain = db_to_gain(cue.gain_db);
        let voice = Voice {
            id: VoiceId(self.next_voice),
            cue: cue_index,
            priority,
            seq: self.next_seq,
            bus: cue.bus,
            entity: event.entity,
            position,
            spatial: cue.spatial,
            base_gain,
            gain: base_gain * spatial_gain,
            pan,
        };
        self.next_voice += 1;
        self.next_seq += 1;
        self.cue_state[cue_index] = CueState {
            last_start_ms: Some(now_ms),
            round_robin: state.round_robin.wrapping_add(1),
        };
        self.commands.push(Command::Start {
            voice: voice.id,
            clip,
            bus: voice.bus,
            gain: voice.gain,
            pan: voice.pan,
            rate,
        });
        self.voices.push(voice);
        TriggerOutcome::Started {
            voice: voice.id,
            stolen,
        }
    }

    /// Moves the listener and re-spatialises every positional voice.
    pub fn set_listener(&mut self, position: [f32; 3], right: [f32; 3]) {
        self.listener = Listener { position, right };
        self.respatialize(|_| true);
    }

    pub fn listener(&self) -> Listener {
        self.listener
    }

    /// Moves an entity; voices following it are re-spatialised.
    pub fn set_entity_position(&mut self, entity: EntityId, position: [f32; 3]) {
        self.entity_positions.insert(entity, position);
        for voice in &mut self.voices {
            if voice.entity == Some(entity) {
                voice.position = Some(position);
            }
        }
        self.respatialize(|voice| voice.entity == Some(entity));
    }

    /// Entity removed: stop (with a short fade) everything it is playing.
    pub fn stop_entity(&mut self, entity: EntityId) {
        self.entity_positions.remove(&entity);
        let fade_ms = self.config.stop_fade_ms;
        let commands = &mut self.commands;
        self.voices.retain(|voice| {
            let keep = voice.entity != Some(entity);
            if !keep {
                commands.push(Command::Stop {
                    voice: voice.id,
                    fade_ms,
                });
            }
            keep
        });
    }

    /// Session replace, seek or reconnect: stops every voice and clears cooldowns and entity
    /// positions. Seen cue keys are kept so a replayed history stays silent; call
    /// [`Director::forget_cue_keys`] for a genuinely new session.
    pub fn reset(&mut self) {
        let fade_ms = self.config.stop_fade_ms;
        self.commands.extend(
            self.voices
                .drain(..)
                .map(|voice| Command::Stop {
                    voice: voice.id,
                    fade_ms,
                })
                .collect::<Vec<_>>(),
        );
        self.entity_positions.clear();
        for state in &mut self.cue_state {
            state.last_start_ms = None;
        }
    }

    pub fn forget_cue_keys(&mut self) {
        self.seen.clear();
        self.seen_order.clear();
    }

    /// Applies volume and mute; emits `SetBusGain` only for buses whose gain changed.
    pub fn apply_settings(&mut self, settings: AudioSettings) {
        let before = self.settings;
        self.settings = settings;
        for bus in Bus::ALL {
            let gain = settings.bus_gain(bus);
            if (gain - before.bus_gain(bus)).abs() > f32::EPSILON {
                self.commands.push(Command::SetBusGain {
                    bus,
                    gain,
                    fade_ms: self.config.bus_fade_ms,
                });
            }
        }
    }

    pub fn apply_settings_command(&mut self, command: AudioSettingsCommand) {
        let mut settings = self.settings;
        settings.apply(command);
        self.apply_settings(settings);
    }

    /// Hidden tab or minimised window. While suspended, triggers are dropped (their keys
    /// still count as seen, so stale cues never play late).
    pub fn set_suspended(&mut self, suspended: bool) {
        if self.suspended != suspended {
            self.suspended = suspended;
            self.commands.push(if suspended {
                Command::Suspend
            } else {
                Command::Resume
            });
        }
    }

    /// Frees the slots of voices the backend reported finished.
    pub fn voices_finished(&mut self, finished: impl IntoIterator<Item = VoiceId>) {
        let finished: HashSet<VoiceId> = finished.into_iter().collect();
        if !finished.is_empty() {
            self.voices.retain(|voice| !finished.contains(&voice.id));
        }
    }

    /// Takes the queued commands.
    pub fn drain(&mut self) -> std::vec::Drain<'_, Command> {
        self.commands.drain(..)
    }

    /// Frees finished voices, then sends queued commands to `backend`. Call once per frame
    /// (and after triggers if you want them to start this frame).
    pub fn pump(&mut self, backend: &mut impl Backend) {
        self.voices_finished(backend.finished_voices());
        for command in self.commands.drain(..) {
            backend.apply(command);
        }
    }

    fn remember(&mut self, key: &str) -> bool {
        if self.seen.contains(key) {
            return false;
        }
        let capacity = self.config.seen_key_capacity.max(1);
        while self.seen_order.len() >= capacity {
            if let Some(old) = self.seen_order.pop_front() {
                self.seen.remove(&old);
            }
        }
        self.seen.insert(key.to_owned());
        self.seen_order.push_back(key.to_owned());
        true
    }

    fn oldest(&self, filter: impl Fn(&Voice) -> bool) -> Option<usize> {
        self.voices
            .iter()
            .enumerate()
            .filter(|(_, voice)| filter(voice))
            .min_by_key(|(_, voice)| voice.seq)
            .map(|(index, _)| index)
    }

    fn oldest_lowest_priority(&self) -> Option<usize> {
        self.voices
            .iter()
            .enumerate()
            .min_by_key(|(_, voice)| (voice.priority, voice.seq))
            .map(|(index, _)| index)
    }

    fn respatialize(&mut self, filter: impl Fn(&Voice) -> bool) {
        let listener = self.listener;
        let attenuation = self.manifest.attenuation;
        for voice in &mut self.voices {
            let (true, Some(position)) = (voice.spatial, voice.position) else {
                continue;
            };
            if !filter(voice) {
                continue;
            }
            let (spatial_gain, pan) = spatialize(&listener, position, &attenuation);
            let gain = voice.base_gain * spatial_gain;
            if (gain - voice.gain).abs() > EPSILON || (pan - voice.pan).abs() > EPSILON {
                voice.gain = gain;
                voice.pan = pan;
                self.commands.push(Command::Update {
                    voice: voice.id,
                    gain,
                    pan,
                });
            }
        }
    }
}

/// Small deterministic PRNG (SplitMix64); avoids a `rand` dependency.
#[derive(Clone, Copy, Debug)]
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn signed_unit(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
