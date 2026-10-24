//! Mapping from the `settings` repository's audio settings to bus gains.
//!
//! Persistence stays in `settings`: a consumer implements `settings_adapters::AudioSettingsAdapter`
//! with `Command = game_audio::AudioSettingsCommand`, maps changes with
//! [`AudioSettingsCommand::from_setting`], and hands the commands to
//! [`crate::Director::apply_settings_command`].

use serde::{Deserialize, Serialize};

use crate::Bus;

/// Setting ids this crate understands.
pub mod keys {
    /// Number in `0..=1`, or integer percent in `0..=100`.
    pub const MASTER_VOLUME: &str = "audio.master_volume";
    pub const EFFECTS_VOLUME: &str = "audio.effects_volume";
    pub const UI_VOLUME: &str = "audio.ui_volume";
    /// Bool.
    pub const MUTED: &str = "audio.muted";
}

/// Volume sliders (`0..=1`) and mute, as the player sees them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub master: f32,
    pub effects: f32,
    pub ui: f32,
    pub muted: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            master: 1.0,
            effects: 1.0,
            ui: 1.0,
            muted: false,
        }
    }
}

/// Perceptual slider curve: gain = volume².
pub fn volume_to_gain(volume: f32) -> f32 {
    let volume = if volume.is_finite() {
        volume.clamp(0.0, 1.0)
    } else {
        0.0
    };
    volume * volume
}

impl AudioSettings {
    /// Linear gain of one bus. Mute silences `Master` only, so un-muting restores the mix.
    pub fn bus_gain(&self, bus: Bus) -> f32 {
        match bus {
            Bus::Master if self.muted => 0.0,
            Bus::Master => volume_to_gain(self.master),
            Bus::Effects => volume_to_gain(self.effects),
            Bus::Ui => volume_to_gain(self.ui),
        }
    }

    pub fn apply(&mut self, command: AudioSettingsCommand) {
        match command {
            AudioSettingsCommand::SetVolume { bus, volume } => {
                let slot = match bus {
                    Bus::Master => &mut self.master,
                    Bus::Effects => &mut self.effects,
                    Bus::Ui => &mut self.ui,
                };
                *slot = volume.clamp(0.0, 1.0);
            }
            AudioSettingsCommand::SetMuted(muted) => self.muted = muted,
        }
    }
}

/// The mixer command a settings change maps to.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AudioSettingsCommand {
    SetVolume { bus: Bus, volume: f32 },
    SetMuted(bool),
}

/// A setting value in the shapes `settings-core` stores (`Bool`, `Integer`, `Number`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Integer(i64),
    Number(f64),
}

impl AudioSettingsCommand {
    /// Maps one setting id/value to a command; unknown ids or mismatched types give `None`.
    /// Integers are percent (`0..=100`), numbers are `0..=1`.
    pub fn from_setting(id: &str, value: SettingValue) -> Option<Self> {
        let bus = match id {
            keys::MASTER_VOLUME => Bus::Master,
            keys::EFFECTS_VOLUME => Bus::Effects,
            keys::UI_VOLUME => Bus::Ui,
            keys::MUTED => {
                return match value {
                    SettingValue::Bool(muted) => Some(Self::SetMuted(muted)),
                    _ => None,
                };
            }
            _ => return None,
        };
        let volume = match value {
            SettingValue::Integer(percent) => percent.clamp(0, 100) as f32 / 100.0,
            SettingValue::Number(volume) => volume.clamp(0.0, 1.0) as f32,
            SettingValue::Bool(_) => return None,
        };
        Some(Self::SetVolume { bus, volume })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_setting_ids_and_values() {
        assert_eq!(
            AudioSettingsCommand::from_setting(keys::MASTER_VOLUME, SettingValue::Integer(80)),
            Some(AudioSettingsCommand::SetVolume {
                bus: Bus::Master,
                volume: 0.8
            })
        );
        assert_eq!(
            AudioSettingsCommand::from_setting(keys::UI_VOLUME, SettingValue::Number(0.25)),
            Some(AudioSettingsCommand::SetVolume {
                bus: Bus::Ui,
                volume: 0.25
            })
        );
        assert_eq!(
            AudioSettingsCommand::from_setting(keys::MUTED, SettingValue::Bool(true)),
            Some(AudioSettingsCommand::SetMuted(true))
        );
        assert_eq!(
            AudioSettingsCommand::from_setting(keys::MUTED, SettingValue::Integer(1)),
            None
        );
        assert_eq!(
            AudioSettingsCommand::from_setting("video.vsync", SettingValue::Bool(true)),
            None
        );
    }

    #[test]
    fn bus_gain_uses_squared_curve_and_mute_on_master() {
        let mut settings = AudioSettings {
            master: 0.5,
            effects: 1.0,
            ui: 0.0,
            muted: false,
        };
        assert_eq!(settings.bus_gain(Bus::Master), 0.25);
        assert_eq!(settings.bus_gain(Bus::Effects), 1.0);
        assert_eq!(settings.bus_gain(Bus::Ui), 0.0);
        settings.apply(AudioSettingsCommand::SetMuted(true));
        assert_eq!(settings.bus_gain(Bus::Master), 0.0);
        assert_eq!(settings.bus_gain(Bus::Effects), 1.0);
    }
}
