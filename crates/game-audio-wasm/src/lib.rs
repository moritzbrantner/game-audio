//! wasm-bindgen facade: exposes [`game_audio::Director`] driving the Web Audio backend to
//! JavaScript as `GameAudio`. Use it through the `@moritzbrantner/game-audio` JS package,
//! which adds first-gesture unlock and hidden-tab suspension.

use game_audio::TriggerOutcome;

/// JS-facing name of a trigger outcome.
pub fn outcome_name(outcome: TriggerOutcome) -> &'static str {
    match outcome {
        TriggerOutcome::Started { stolen: None, .. } => "started",
        TriggerOutcome::Started {
            stolen: Some(_), ..
        } => "started-stealing",
        TriggerOutcome::Duplicate => "duplicate",
        TriggerOutcome::UnknownCue => "unknown-cue",
        TriggerOutcome::Suspended => "suspended",
        TriggerOutcome::CoolingDown => "cooling-down",
        TriggerOutcome::Inaudible => "inaudible",
        TriggerOutcome::Rejected => "rejected",
    }
}

#[cfg(target_arch = "wasm32")]
mod bindings {
    use game_audio::settings::SettingValue;
    use game_audio::{
        AudioSettings, AudioSettingsCommand, Backend, CueEvent, Director, DirectorConfig, EntityId,
        SoundManifest, WebAudioBackend,
    };
    use wasm_bindgen::prelude::*;

    use super::outcome_name;

    #[wasm_bindgen]
    pub struct GameAudio {
        director: Director,
        backend: WebAudioBackend,
    }

    fn vec3(values: &[f32]) -> Result<[f32; 3], JsError> {
        values
            .try_into()
            .map_err(|_| JsError::new("expected a 3-component vector"))
    }

    fn entity(id: f64) -> Result<EntityId, JsError> {
        if id.is_finite() && id >= 0.0 && id.fract() == 0.0 && id <= 9_007_199_254_740_991.0 {
            Ok(EntityId(id as u64))
        } else {
            Err(JsError::new(
                "entity ids must be non-negative safe integers",
            ))
        }
    }

    #[wasm_bindgen]
    impl GameAudio {
        /// `manifest` is the sound manifest JSON text; `config` is an optional
        /// `DirectorConfig`-shaped object; `base_url` prefixes relative clip URLs.
        #[wasm_bindgen(constructor)]
        pub fn new(
            manifest: &str,
            config: JsValue,
            base_url: Option<String>,
        ) -> Result<GameAudio, JsError> {
            let manifest = SoundManifest::from_json(manifest)?;
            let config: DirectorConfig = if config.is_undefined() || config.is_null() {
                DirectorConfig::default()
            } else {
                serde_wasm_bindgen::from_value(config)?
            };
            let mut audio = GameAudio {
                director: Director::new(manifest, config),
                backend: WebAudioBackend::new(base_url.unwrap_or_default()),
            };
            audio.pump();
            Ok(audio)
        }

        /// Sounds `{ key, kind, entity?, position?: [x, y, z] }` (extra fields ignored).
        /// Returns the outcome name, e.g. `"started"` or `"duplicate"`.
        pub fn trigger(&mut self, cue: JsValue, now_ms: f64) -> Result<String, JsError> {
            let cue: CueEvent = serde_wasm_bindgen::from_value(cue)?;
            let outcome = self.director.trigger(&cue, now_ms);
            self.pump();
            Ok(outcome_name(outcome).to_owned())
        }

        #[wasm_bindgen(js_name = setListener)]
        pub fn set_listener(&mut self, position: &[f32], right: &[f32]) -> Result<(), JsError> {
            self.director.set_listener(vec3(position)?, vec3(right)?);
            self.pump();
            Ok(())
        }

        #[wasm_bindgen(js_name = setEntityPosition)]
        pub fn set_entity_position(&mut self, id: f64, position: &[f32]) -> Result<(), JsError> {
            self.director
                .set_entity_position(entity(id)?, vec3(position)?);
            self.pump();
            Ok(())
        }

        #[wasm_bindgen(js_name = stopEntity)]
        pub fn stop_entity(&mut self, id: f64) -> Result<(), JsError> {
            self.director.stop_entity(entity(id)?);
            self.pump();
            Ok(())
        }

        /// Session replace / seek / reconnect. Seen cue keys are kept.
        pub fn reset(&mut self) {
            self.director.reset();
            self.pump();
        }

        #[wasm_bindgen(js_name = forgetCueKeys)]
        pub fn forget_cue_keys(&mut self) {
            self.director.forget_cue_keys();
        }

        /// `{ master, effects, ui, muted }`, volumes in 0..=1; missing fields default.
        #[wasm_bindgen(js_name = applySettings)]
        pub fn apply_settings(&mut self, settings: JsValue) -> Result<(), JsError> {
            let settings: AudioSettings = serde_wasm_bindgen::from_value(settings)?;
            self.director.apply_settings(settings);
            self.pump();
            Ok(())
        }

        /// Applies one `settings` value by id (`audio.master_volume`, `audio.muted`, ...).
        /// Numbers are 0..=1 unless `percent` is set. Returns false for ids it does not own.
        #[wasm_bindgen(js_name = applySetting)]
        pub fn apply_setting(&mut self, id: &str, value: JsValue, percent: bool) -> bool {
            let value = if let Some(flag) = value.as_bool() {
                SettingValue::Bool(flag)
            } else if let Some(number) = value.as_f64() {
                if percent {
                    SettingValue::Integer(number.round() as i64)
                } else {
                    SettingValue::Number(number)
                }
            } else {
                return false;
            };
            let Some(command) = AudioSettingsCommand::from_setting(id, value) else {
                return false;
            };
            self.director.apply_settings_command(command);
            self.pump();
            true
        }

        #[wasm_bindgen(js_name = setSuspended)]
        pub fn set_suspended(&mut self, suspended: bool) {
            self.director.set_suspended(suspended);
            self.pump();
        }

        /// Call from a user gesture.
        pub fn unlock(&mut self) {
            self.backend.unlock();
        }

        /// `"running"`, `"suspended"` (locked or hidden) or `"unavailable"`.
        pub fn state(&self) -> String {
            self.backend.state().as_str().to_owned()
        }

        #[wasm_bindgen(js_name = activeVoices)]
        pub fn active_voices(&self) -> usize {
            self.director.active_voices()
        }

        /// Clip load errors since the last call.
        #[wasm_bindgen(js_name = takeErrors)]
        pub fn take_errors(&mut self) -> Vec<String> {
            self.backend.take_errors()
        }

        /// Frees finished voices and flushes queued commands. Every mutating method already
        /// pumps; call this once per frame so ended voices free their slots promptly.
        pub fn pump(&mut self) {
            self.director
                .voices_finished(self.backend.finished_voices());
            for command in self.director.drain() {
                self.backend.apply(command);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_audio::VoiceId;

    #[test]
    fn outcome_names_are_stable() {
        let started = TriggerOutcome::Started {
            voice: VoiceId(1),
            stolen: None,
        };
        assert_eq!(outcome_name(started), "started");
        assert_eq!(outcome_name(TriggerOutcome::Duplicate), "duplicate");
        assert_eq!(outcome_name(TriggerOutcome::CoolingDown), "cooling-down");
    }
}
