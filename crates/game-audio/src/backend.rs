//! The command stream between the director and an audio backend.

use std::collections::VecDeque;

use crate::manifest::ClipSource;
use crate::{Bus, ClipId, VoiceId};

#[cfg(all(feature = "kira", not(target_arch = "wasm32")))]
pub mod kira;
#[cfg(all(feature = "web", target_arch = "wasm32"))]
pub mod web;

/// One instruction for a backend. Gains are linear, pan is `-1.0..=1.0`, rate is a
/// playback-rate factor.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Decode or render a clip so later `Start`s can use it.
    Load {
        clip: ClipId,
        source: ClipSource,
    },
    Start {
        voice: VoiceId,
        clip: ClipId,
        bus: Bus,
        gain: f32,
        pan: f32,
        rate: f32,
    },
    /// A moving source or listener changed the voice's gain/pan.
    Update {
        voice: VoiceId,
        gain: f32,
        pan: f32,
    },
    Stop {
        voice: VoiceId,
        fade_ms: u32,
    },
    SetBusGain {
        bus: Bus,
        gain: f32,
        fade_ms: u32,
    },
    /// Hidden tab / minimised window: pause output.
    Suspend,
    Resume,
}

/// Executes director commands. Backends make no policy decisions.
pub trait Backend {
    fn apply(&mut self, command: Command);
    /// Voices that ended on their own (or failed to start) since the last call, so the
    /// director can free their slots.
    fn finished_voices(&mut self) -> Vec<VoiceId>;
}

/// Records every command; tests mark voices finished with [`RecordingBackend::finish`].
#[derive(Debug, Default)]
pub struct RecordingBackend {
    pub commands: Vec<Command>,
    finished: VecDeque<VoiceId>,
}

impl RecordingBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reports `voice` as finished on the next [`Backend::finished_voices`] call.
    pub fn finish(&mut self, voice: VoiceId) {
        self.finished.push_back(voice);
    }

    /// Returns and clears the recorded commands.
    pub fn take(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    /// Voice ids of recorded `Start` commands, in order.
    pub fn started(&self) -> Vec<VoiceId> {
        self.commands
            .iter()
            .filter_map(|command| match command {
                Command::Start { voice, .. } => Some(*voice),
                _ => None,
            })
            .collect()
    }

    /// Voice ids of recorded `Stop` commands, in order.
    pub fn stopped(&self) -> Vec<VoiceId> {
        self.commands
            .iter()
            .filter_map(|command| match command {
                Command::Stop { voice, .. } => Some(*voice),
                _ => None,
            })
            .collect()
    }
}

impl Backend for RecordingBackend {
    fn apply(&mut self, command: Command) {
        self.commands.push(command);
    }

    fn finished_voices(&mut self) -> Vec<VoiceId> {
        self.finished.drain(..).collect()
    }
}
