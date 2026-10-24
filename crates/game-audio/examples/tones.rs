//! Plays generated test tones through the native kira backend; no sound files needed.
//!
//! `cargo run -p game-audio --example tones --features kira`
//!
//! You should hear: a UI click, a hit that pans left → right as its source circles the
//! listener and fades with distance, a rapid burst trimmed by the cooldown and voice cap,
//! a falling "hurt" sweep, then the same pickup at three volume settings and once muted.

use std::thread::sleep;
use std::time::{Duration, Instant};

use game_audio::{
    AudioSettings, CueEvent, Director, DirectorConfig, KiraBackend, SoundManifest, TriggerOutcome,
};

const MANIFEST: &str = r#"{
  "attenuation": { "min_distance": 1, "max_distance": 12 },
  "clips": [
    { "id": "click", "source": { "tone": { "frequency_hz": 1800, "duration_ms": 25, "waveform": "square", "amplitude": 0.25 } } },
    { "id": "hit-a", "source": { "tone": { "frequency_hz": 220, "end_frequency_hz": 140, "duration_ms": 140, "waveform": "saw" } } },
    { "id": "hit-b", "source": { "tone": { "frequency_hz": 247, "end_frequency_hz": 150, "duration_ms": 140, "waveform": "saw" } } },
    { "id": "hurt", "source": { "tone": { "frequency_hz": 660, "end_frequency_hz": 220, "duration_ms": 300, "waveform": "triangle" } } },
    { "id": "pickup", "source": { "tone": { "frequency_hz": 880, "end_frequency_hz": 1320, "duration_ms": 120 } } },
    { "id": "whoosh", "source": { "tone": { "frequency_hz": 1, "duration_ms": 90, "waveform": "noise", "amplitude": 0.2 } } }
  ],
  "cues": [
    { "kind": "ui.click", "clips": ["click"], "bus": "ui", "priority": 90 },
    { "kind": "combat.hit", "clips": ["hit-a", "hit-b"], "priority": 70, "max_voices": 3,
      "cooldown_ms": 60, "pitch_jitter": 0.06, "spatial": true },
    { "kind": "combat.whiff", "clips": ["whoosh"], "priority": 20, "max_voices": 2, "spatial": true },
    { "kind": "player.hurt", "clips": ["hurt"], "priority": 80, "max_voices": 1 },
    { "kind": "loot.pickup", "clips": ["pickup"], "bus": "ui" }
  ]
}"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = SoundManifest::from_json(MANIFEST)?;
    let mut director = Director::new(manifest, DirectorConfig::default());
    let mut backend = KiraBackend::new()?;
    let clock = Instant::now();
    let now = || clock.elapsed().as_secs_f64() * 1000.0;
    let mut key = 0u32;
    let mut cue = |director: &mut Director, kind: &str, at: Option<[f32; 3]>| {
        key += 1;
        let mut event = CueEvent::new(format!("demo:{key}"), kind);
        event.position = at;
        let outcome = director.trigger(&event, now());
        println!("{kind:<14} {at:?} -> {outcome:?}");
        outcome
    };
    let wait = |director: &mut Director, backend: &mut KiraBackend, ms: u64| {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            director.pump(backend);
            sleep(Duration::from_millis(10));
        }
    };
    director.pump(&mut backend);
    for error in backend.take_errors() {
        eprintln!("{error}");
    }

    cue(&mut director, "ui.click", None);
    wait(&mut director, &mut backend, 300);

    println!("-- a hit circling the listener at growing distance (left → front → right → behind)");
    for step in 0..12 {
        let angle = step as f32 / 12.0 * std::f32::consts::TAU + std::f32::consts::PI;
        let radius = 1.0 + step as f32 * 0.8;
        let at = [angle.cos() * radius, 0.0, angle.sin() * radius];
        cue(&mut director, "combat.hit", Some(at));
        wait(&mut director, &mut backend, 220);
    }

    println!("-- a burst of 10 hits 20 ms apart: the 60 ms cooldown drops most of them");
    for _ in 0..10 {
        cue(&mut director, "combat.hit", Some([0.0, 0.0, 2.0]));
        wait(&mut director, &mut backend, 20);
    }
    wait(&mut director, &mut backend, 300);

    println!("-- replaying an already-heard cue key is silent");
    let replay = CueEvent::new("demo:1", "ui.click");
    assert_eq!(director.trigger(&replay, now()), TriggerOutcome::Duplicate);

    cue(&mut director, "player.hurt", None);
    wait(&mut director, &mut backend, 500);

    println!("-- pickup at UI volume 1.0, 0.6, 0.3, then muted");
    for (ui, muted) in [(1.0, false), (0.6, false), (0.3, false), (1.0, true)] {
        director.apply_settings(AudioSettings {
            ui,
            muted,
            ..AudioSettings::default()
        });
        cue(&mut director, "loot.pickup", None);
        wait(&mut director, &mut backend, 400);
    }
    wait(&mut director, &mut backend, 200);
    Ok(())
}
