//! Browser backend: the Web Audio API through `web-sys`.
//!
//! One `AudioContext`; each bus is a `GainNode` (`effects`/`ui` → `master` → destination);
//! each voice is `AudioBufferSourceNode → GainNode → StereoPannerNode → bus`. Mixing runs on
//! the browser's audio rendering thread, not the main thread.
//!
//! The context starts suspended until a user gesture calls [`WebAudioBackend::unlock`]. If
//! Web Audio is missing or blocked the backend stays silent and every command is a no-op.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use js_sys::{ArrayBuffer, Function};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AudioBuffer, AudioBufferSourceNode, AudioContext, AudioContextState, AudioNode,
    AudioScheduledSourceNode, GainNode, Response, StereoPannerNode,
};

use crate::backend::{Backend, Command};
use crate::manifest::ClipSource;
use crate::{Bus, ClipId, VoiceId};

/// What the page can currently hear.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioState {
    Running,
    /// Waiting for a user gesture, or suspended because the page is hidden.
    Suspended,
    /// No Web Audio, or the context could not be created: permanent silence.
    Unavailable,
}

impl AudioState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Suspended => "suspended",
            Self::Unavailable => "unavailable",
        }
    }
}

struct WebVoice {
    source: AudioBufferSourceNode,
    gain: GainNode,
    pan: StereoPannerNode,
    _on_ended: Closure<dyn FnMut()>,
}

struct Inner {
    context: AudioContext,
    master: GainNode,
    effects: GainNode,
    ui: GainNode,
    clips: Rc<RefCell<HashMap<ClipId, AudioBuffer>>>,
    voices: HashMap<VoiceId, WebVoice>,
    ended: Rc<RefCell<Vec<VoiceId>>>,
    finished: Vec<VoiceId>,
    errors: Rc<RefCell<Vec<String>>>,
    base_url: String,
    hidden: bool,
}

pub struct WebAudioBackend {
    inner: Option<Inner>,
    /// Starts swallowed while silent, reported finished so the director frees them.
    silent_finished: Vec<VoiceId>,
}

impl WebAudioBackend {
    /// Creates the context and bus graph. `base_url` prefixes relative clip URLs
    /// (e.g. `"./sounds/"`). Never fails: without Web Audio the backend is silent.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            inner: Inner::new(base_url.into()).ok(),
            silent_finished: Vec::new(),
        }
    }

    /// A backend that ignores every command.
    pub fn silent() -> Self {
        Self {
            inner: None,
            silent_finished: Vec::new(),
        }
    }

    /// Call from a user gesture (pointer/key) so the browser allows playback.
    pub fn unlock(&mut self) {
        if let Some(inner) = &self.inner
            && !inner.hidden
        {
            let _ = inner.context.resume();
        }
    }

    pub fn state(&self) -> AudioState {
        match &self.inner {
            None => AudioState::Unavailable,
            Some(inner) => match inner.context.state() {
                AudioContextState::Running => AudioState::Running,
                AudioContextState::Closed => AudioState::Unavailable,
                _ => AudioState::Suspended,
            },
        }
    }

    /// Clip fetch/decode failures since the last call.
    pub fn take_errors(&mut self) -> Vec<String> {
        self.inner
            .as_ref()
            .map(|inner| std::mem::take(&mut *inner.errors.borrow_mut()))
            .unwrap_or_default()
    }
}

impl Inner {
    fn new(base_url: String) -> Result<Self, JsValue> {
        let context = AudioContext::new()?;
        let master = context.create_gain()?;
        let effects = context.create_gain()?;
        let ui = context.create_gain()?;
        master.connect_with_audio_node(&context.destination())?;
        effects.connect_with_audio_node(&master)?;
        ui.connect_with_audio_node(&master)?;
        Ok(Self {
            context,
            master,
            effects,
            ui,
            clips: Rc::default(),
            voices: HashMap::new(),
            ended: Rc::default(),
            finished: Vec::new(),
            errors: Rc::default(),
            base_url,
            hidden: false,
        })
    }

    fn bus(&self, bus: Bus) -> &GainNode {
        match bus {
            Bus::Master => &self.master,
            Bus::Effects => &self.effects,
            Bus::Ui => &self.ui,
        }
    }

    fn load(&mut self, clip: ClipId, source: ClipSource) {
        match source {
            ClipSource::Tone(tone) => {
                let rate = self.context.sample_rate();
                let samples = tone.render(rate as u32);
                let buffer = self
                    .context
                    .create_buffer(1, samples.len() as u32, rate)
                    .and_then(|buffer| {
                        buffer.copy_to_channel(&samples, 0)?;
                        Ok(buffer)
                    });
                match buffer {
                    Ok(buffer) => {
                        self.clips.borrow_mut().insert(clip, buffer);
                    }
                    Err(error) => self.errors.borrow_mut().push(describe(&error)),
                }
            }
            ClipSource::Url(url) => {
                let url = if is_absolute(&url) {
                    url
                } else {
                    format!("{}{url}", self.base_url)
                };
                let context = self.context.clone();
                let clips = self.clips.clone();
                let errors = self.errors.clone();
                spawn_local(async move {
                    match fetch_and_decode(&context, &url).await {
                        Ok(buffer) => {
                            clips.borrow_mut().insert(clip, buffer);
                        }
                        Err(error) => errors
                            .borrow_mut()
                            .push(format!("{url}: {}", describe(&error))),
                    }
                });
            }
        }
    }

    fn start(
        &mut self,
        voice: VoiceId,
        clip: ClipId,
        bus: Bus,
        gain: f32,
        pan: f32,
        rate: f32,
    ) -> Result<(), JsValue> {
        let Some(buffer) = self.clips.borrow().get(&clip).cloned() else {
            return Err(JsValue::from_str("clip not loaded"));
        };
        let context = &self.context;
        let source = context.create_buffer_source()?;
        source.set_buffer(Some(&buffer));
        source.playback_rate().set_value(rate);
        let gain_node = context.create_gain()?;
        gain_node.gain().set_value(gain);
        let pan_node = context.create_stereo_panner()?;
        pan_node.pan().set_value(pan);
        source.connect_with_audio_node(&gain_node)?;
        gain_node.connect_with_audio_node(&pan_node)?;
        pan_node.connect_with_audio_node(self.bus(bus))?;
        let ended = self.ended.clone();
        let on_ended = Closure::<dyn FnMut()>::new(move || ended.borrow_mut().push(voice));
        scheduled(&source).set_onended(Some(on_ended.as_ref().unchecked_ref::<Function>()));
        source.start()?;
        self.voices.insert(
            voice,
            WebVoice {
                source,
                gain: gain_node,
                pan: pan_node,
                _on_ended: on_ended,
            },
        );
        Ok(())
    }

    fn stop(&mut self, voice: VoiceId, fade_ms: u32) {
        // Keep the voice (and its onended closure) until the browser reports it ended.
        let Some(entry) = self.voices.get(&voice) else {
            return;
        };
        let now = self.context.current_time();
        let end = now + f64::from(fade_ms) / 1000.0;
        let gain = entry.gain.gain();
        let _ = gain.cancel_scheduled_values(now);
        let _ = gain.set_value_at_time(gain.value(), now);
        let _ = gain.linear_ramp_to_value_at_time(0.0, end);
        let _ = scheduled(&entry.source).stop_with_when(end);
    }

    fn ramp(&self, param: &web_sys::AudioParam, value: f32, fade_ms: u32) {
        let now = self.context.current_time();
        let _ = param.cancel_scheduled_values(now);
        if fade_ms == 0 {
            param.set_value(value);
        } else {
            let _ = param.set_target_at_time(value, now, f64::from(fade_ms) / 3000.0);
        }
    }
}

impl Backend for WebAudioBackend {
    fn apply(&mut self, command: Command) {
        let Some(inner) = &mut self.inner else {
            if let Command::Start { voice, .. } = command {
                self.silent_finished.push(voice);
            }
            return;
        };
        match command {
            Command::Load { clip, source } => inner.load(clip, source),
            Command::Start {
                voice,
                clip,
                bus,
                gain,
                pan,
                rate,
            } => {
                if inner.start(voice, clip, bus, gain, pan, rate).is_err() {
                    inner.finished.push(voice);
                }
            }
            Command::Update { voice, gain, pan } => {
                if let Some(entry) = inner.voices.get(&voice) {
                    inner.ramp(&entry.gain.gain(), gain, 20);
                    inner.ramp(&entry.pan.pan(), pan, 20);
                }
            }
            Command::Stop { voice, fade_ms } => inner.stop(voice, fade_ms),
            Command::SetBusGain { bus, gain, fade_ms } => {
                inner.ramp(&inner.bus(bus).gain(), gain, fade_ms);
            }
            Command::Suspend => {
                inner.hidden = true;
                let _ = inner.context.suspend();
            }
            Command::Resume => {
                inner.hidden = false;
                let _ = inner.context.resume();
            }
        }
    }

    fn finished_voices(&mut self) -> Vec<VoiceId> {
        let Some(inner) = &mut self.inner else {
            return std::mem::take(&mut self.silent_finished);
        };
        let ended: Vec<VoiceId> = inner.ended.borrow_mut().drain(..).collect();
        for voice in &ended {
            if let Some(entry) = inner.voices.remove(voice) {
                scheduled(&entry.source).set_onended(None);
                let _ = AsRef::<AudioNode>::as_ref(&entry.pan).disconnect();
            }
        }
        inner.finished.extend(ended);
        std::mem::take(&mut inner.finished)
    }
}

async fn fetch_and_decode(context: &AudioContext, url: &str) -> Result<AudioBuffer, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("no window"))?;
    let response: Response = JsFuture::from(window.fetch_with_str(url))
        .await?
        .dyn_into()?;
    if !response.ok() {
        return Err(JsValue::from_str(&format!("HTTP {}", response.status())));
    }
    let bytes: ArrayBuffer = JsFuture::from(response.array_buffer()?).await?.dyn_into()?;
    JsFuture::from(context.decode_audio_data(&bytes)?)
        .await?
        .dyn_into()
}

fn is_absolute(url: &str) -> bool {
    url.starts_with('/') || url.contains("://") || url.starts_with("data:")
}

fn describe(error: &JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            error
                .dyn_ref::<js_sys::Error>()
                .map(|e| String::from(e.message()))
        })
        .unwrap_or_else(|| format!("{error:?}"))
}

fn scheduled(source: &AudioBufferSourceNode) -> &AudioScheduledSourceNode {
    source.as_ref()
}
