use serde::{Deserialize, Serialize};

/// A generated test tone, rendered identically by every backend.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tone {
    pub frequency_hz: f32,
    /// Optional linear sweep target (e.g. a falling "hurt" blip).
    #[serde(default)]
    pub end_frequency_hz: Option<f32>,
    pub duration_ms: u32,
    #[serde(default)]
    pub waveform: Waveform,
    /// Peak amplitude, 0..=1.
    #[serde(default = "default_amplitude")]
    pub amplitude: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Waveform {
    #[default]
    Sine,
    Square,
    Triangle,
    Saw,
    /// Deterministic white noise (frequency is ignored).
    Noise,
}

fn default_amplitude() -> f32 {
    0.5
}

const ATTACK_SECONDS: f32 = 0.004;

impl Tone {
    pub fn new(frequency_hz: f32, duration_ms: u32, waveform: Waveform) -> Self {
        Self {
            frequency_hz,
            end_frequency_hz: None,
            duration_ms,
            waveform,
            amplitude: default_amplitude(),
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        let positive = |hz: f32| hz.is_finite() && hz > 0.0;
        positive(self.frequency_hz)
            && self.end_frequency_hz.is_none_or(positive)
            && self.duration_ms > 0
            && (0.0..=1.0).contains(&self.amplitude)
    }

    /// Renders mono samples with a short attack and an exponential-ish release, so tones
    /// start and end without clicks.
    pub fn render(&self, sample_rate: u32) -> Vec<f32> {
        let rate = sample_rate as f32;
        let len = ((self.duration_ms as u64 * sample_rate as u64) / 1000).max(1) as usize;
        let duration = len as f32 / rate;
        let end_hz = self.end_frequency_hz.unwrap_or(self.frequency_hz);
        let mut phase = 0.0f32;
        let mut noise = 0x9E37_79B9_u32;
        (0..len)
            .map(|i| {
                let t = i as f32 / rate;
                let progress = t / duration;
                let hz = self.frequency_hz + (end_hz - self.frequency_hz) * progress;
                phase = (phase + hz / rate).fract();
                let raw = match self.waveform {
                    Waveform::Sine => (phase * std::f32::consts::TAU).sin(),
                    Waveform::Square => {
                        if phase < 0.5 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    Waveform::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
                    Waveform::Saw => 2.0 * phase - 1.0,
                    Waveform::Noise => {
                        noise ^= noise << 13;
                        noise ^= noise >> 17;
                        noise ^= noise << 5;
                        (noise as f32 / u32::MAX as f32) * 2.0 - 1.0
                    }
                };
                let attack = (t / ATTACK_SECONDS).min(1.0);
                let release = (1.0 - progress).max(0.0).powi(2);
                raw * self.amplitude * attack * release
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_expected_length_without_clicks() {
        let tone = Tone::new(440.0, 100, Waveform::Square);
        let samples = tone.render(48_000);
        assert_eq!(samples.len(), 4_800);
        assert!(samples[0].abs() < 1e-3);
        assert!(samples.last().unwrap().abs() < 1e-3);
        assert!(samples.iter().all(|s| s.abs() <= tone.amplitude + 1e-6));
    }

    #[test]
    fn noise_is_deterministic() {
        let tone = Tone::new(1.0, 20, Waveform::Noise);
        assert_eq!(tone.render(8_000), tone.render(8_000));
    }
}
