# game-audio

One shared Rust **audio director** for the owner's games, with two thin backends:

- **native**: [kira](https://crates.io/crates/kira) on cpal (feature `kira`)
- **browser**: the Web Audio API through `web-sys` (feature `web`, `wasm32`), shipped to
  JavaScript as the `@moritzbrantner/game-audio` package (wasm-bindgen facade + small glue)

The design is arpg ADR 0004 (`docs/adr/0004-cross-platform-game-audio.md`, arpg#67).

## What the director decides

The `Director` is pure and deterministic. It turns calls into `Command`s and never touches a
device:

| Concern | Behaviour |
| --- | --- |
| Manifest | JSON `SoundManifest`: clips (file/URL or generated tone), cues, distance curve |
| Buses | `master` ← `effects`, `ui`; mute silences `master` |
| Voice limits | per-cue `max_voices` (steals that cue's oldest voice), global `max_voices` (steals **lowest priority, then oldest**, and never steals a higher priority) |
| Cooldowns | per-cue `cooldown_ms`; re-triggers inside the window are dropped |
| Dedup | each cue `key` sounds at most once (bounded memory), so replays and resyncs stay silent |
| Spatial | linear distance fade (`min_distance`..`max_distance`) and stereo pan from the listener's right axis, computed in the director (no HRTF) |
| Variants | round-robin or seeded random clip choice, seeded pitch jitter |
| Lifecycle | `stop_entity`, `reset` (seek/reconnect; keeps seen keys), `set_suspended` (hidden tab) |
| Settings | `AudioSettings` → bus gains (slider² curve); `AudioSettingsCommand::from_setting` maps `audio.master_volume`, `audio.effects_volume`, `audio.ui_volume`, `audio.muted` |

Persistence of volume/mute stays in the [`settings`](https://github.com/moritzbrantner/settings)
repository: implement its `AudioSettingsAdapter` with `Command = game_audio::AudioSettingsCommand`
and pass the commands to `Director::apply_settings_command`. This crate stores nothing.

## Manifest

```json
{
  "attenuation": { "min_distance": 2, "max_distance": 40 },
  "clips": [
    { "id": "hit-1", "source": { "url": "hit-1.ogg" } },
    { "id": "beep", "source": { "tone": { "frequency_hz": 880, "duration_ms": 80, "waveform": "square" } } }
  ],
  "cues": [
    { "kind": "combat.hit", "clips": ["hit-1"], "priority": 80, "max_voices": 4,
      "cooldown_ms": 40, "gain_db": -3, "pitch_jitter": 0.05, "spatial": true },
    { "kind": "ui.click", "clips": ["beep"], "bus": "ui" }
  ]
}
```

Cue defaults: `bus: "effects"`, `priority: 50`, `max_voices: 4`, `cooldown_ms: 0`,
`gain_db: 0`, `pitch_jitter: 0`, `spatial: false`, `variants: "round_robin"`. Tone waveforms:
`sine`, `square`, `triangle`, `saw`, `noise`. Real CC0 sounds come from asset-tooling#297.

## Native usage (Rust)

```toml
[dependencies]
game-audio = { git = "https://github.com/moritzbrantner/game-audio", rev = "<commit>", features = ["kira"] }
```

```rust
use game_audio::{CueEvent, Director, DirectorConfig, EntityId, KiraBackend, SoundManifest};

let manifest = SoundManifest::from_json(&std::fs::read_to_string("assets/sounds.json")?)?;
let mut director = Director::new(manifest, DirectorConfig::default());
let mut backend = KiraBackend::new()?.with_asset_root("assets/sounds");

// each frame
director.set_listener(camera_pos, camera_right);
director.set_entity_position(EntityId(7), monster_pos);
director.trigger(&CueEvent::new("strike:player:1:42:monster:7", "combat.hit").on(EntityId(7)), now_ms);
director.pump(&mut backend); // frees finished voices, flushes commands
```

A game that compiles its sounds into the binary passes them with
`KiraBackend::with_embedded_clips([(url, bytes), ...])`; a manifest URL found there is decoded
from memory, any other URL still loads from the asset root.

Linux needs ALSA headers to build cpal (`libasound2-dev`). Hear it with generated tones and no
sound files:

```sh
cargo run -p game-audio --example tones --features kira
```

For tests or headless servers use `RecordingBackend`, which records commands and lets you mark
voices finished.

## Web usage (JavaScript)

The package is published as Git distribution commits on the `dist/game-audio` branch (built
by CI on every push to `main`). Pin an exact commit, never the branch:

```sh
bun add "@moritzbrantner/game-audio@github:moritzbrantner/game-audio#<dist commit>"
```

```js
import { createGameAudio } from "@moritzbrantner/game-audio";

const audio = await createGameAudio({
  manifest, // object or JSON text
  baseUrl: `${import.meta.env.BASE_URL}sounds/`,
});

// presentation cues: { key, kind, entity?, position?: [x, y, z] }; extra fields are ignored
for (const cue of cues) audio.trigger({ key: cue.key, kind: soundKind(cue), entity: cue.target?.id, position: cue.position });
audio.setListener(cameraPosition, cameraRight);
audio.applySettings({ master: 0.8, effects: 1, ui: 0.6, muted: false });
audio.pump(); // once per frame
```

The glue unlocks the `AudioContext` on the first pointer/key/touch gesture, suspends while
`document.hidden`, and never throws into the game loop. Without Web Audio, or if the wasm fails
to load, every call is a no-op and `audio.state()` is `"unavailable"`, so visual feedback is
unaffected. Mixing runs on the browser's audio thread (no `ScriptProcessorNode`, no
`SharedArrayBuffer`), so it works on GitHub Pages without COOP/COEP.

`@moritzbrantner/game-audio/wasm` exposes the raw wasm-bindgen `GameAudio` class.

## Development

```sh
cargo test --workspace                                  # director tests (recording backend)
cargo clippy --workspace --all-targets --features game-audio/kira -- -D warnings
cargo clippy --target wasm32-unknown-unknown -p game-audio-wasm -- -D warnings
scripts/build-web.sh                                    # web/pkg via wasm-bindgen-cli (version = Cargo.lock)
(cd web && bun test)                                    # JS glue tests
python3 -m http.server -d web 8000                      # then open /demo.html
scripts/publish-git-dist.sh                             # prepare the dist commit locally (no push)
```

## Licence

MIT OR Apache-2.0. Dependencies are free/OSS (kira, cpal, web-sys, wasm-bindgen, serde).
