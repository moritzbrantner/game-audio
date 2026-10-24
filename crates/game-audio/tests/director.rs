use game_audio::{
    Attenuation, AudioSettings, AudioSettingsCommand, Bus, ClipId, ClipSource, ClipSpec, Command,
    CueEvent, CueSpec, Director, DirectorConfig, EntityId, Listener, RecordingBackend,
    SoundManifest, Tone, TriggerOutcome, VariantMode, VoiceId, Waveform, distance_gain, stereo_pan,
};

fn tone_clip(id: &str, hz: f32) -> ClipSpec {
    ClipSpec {
        id: id.into(),
        source: ClipSource::Tone(Tone::new(hz, 100, Waveform::Sine)),
    }
}

fn manifest(cues: Vec<CueSpec>) -> SoundManifest {
    let manifest = SoundManifest {
        attenuation: Attenuation {
            min_distance: 1.0,
            max_distance: 11.0,
        },
        clips: vec![tone_clip("a", 440.0), tone_clip("b", 660.0)],
        cues,
    };
    manifest.validate().unwrap();
    manifest
}

fn cue(kind: &str, priority: u8, max_voices: u8) -> CueSpec {
    CueSpec {
        priority,
        max_voices,
        ..CueSpec::new(kind, &["a"])
    }
}

/// Director plus recording backend with the construction commands already flushed.
fn setup(cues: Vec<CueSpec>, max_voices: usize) -> (Director, RecordingBackend) {
    let mut director = Director::new(
        manifest(cues),
        DirectorConfig {
            max_voices,
            ..DirectorConfig::default()
        },
    );
    let mut backend = RecordingBackend::new();
    director.pump(&mut backend);
    backend.take();
    (director, backend)
}

fn started(outcome: TriggerOutcome) -> (VoiceId, Option<VoiceId>) {
    match outcome {
        TriggerOutcome::Started { voice, stolen } => (voice, stolen),
        other => panic!("expected a start, got {other:?}"),
    }
}

#[test]
fn construction_loads_clips_and_sets_bus_gains() {
    let mut director = Director::new(manifest(vec![cue("hit", 50, 4)]), DirectorConfig::default());
    let commands: Vec<_> = director.drain().collect();
    assert!(matches!(
        commands[0],
        Command::Load {
            clip: ClipId(0),
            ..
        }
    ));
    assert!(matches!(
        commands[1],
        Command::Load {
            clip: ClipId(1),
            ..
        }
    ));
    assert_eq!(
        commands[2..],
        Bus::ALL.map(|bus| Command::SetBusGain {
            bus,
            gain: 1.0,
            fade_ms: 0
        })
    );
}

#[test]
fn cue_keys_are_sounded_once() {
    let (mut director, mut backend) = setup(vec![cue("hit", 50, 4)], 8);
    started(director.trigger(&CueEvent::new("strike:1:10:2", "hit"), 0.0));
    assert_eq!(
        director.trigger(&CueEvent::new("strike:1:10:2", "hit"), 500.0),
        TriggerOutcome::Duplicate
    );
    // Reset (seek / reconnect) keeps the memory: a replayed history stays silent.
    director.reset();
    assert_eq!(
        director.trigger(&CueEvent::new("strike:1:10:2", "hit"), 900.0),
        TriggerOutcome::Duplicate
    );
    director.forget_cue_keys();
    started(director.trigger(&CueEvent::new("strike:1:10:2", "hit"), 1000.0));
    // Empty keys are never deduplicated.
    started(director.trigger(&CueEvent::new("", "hit"), 2000.0));
    started(director.trigger(&CueEvent::new("", "hit"), 3000.0));
    director.pump(&mut backend);
    assert_eq!(backend.started().len(), 4);
}

#[test]
fn seen_key_memory_is_bounded() {
    let mut director = Director::new(
        manifest(vec![cue("hit", 50, 8)]),
        DirectorConfig {
            seen_key_capacity: 2,
            ..DirectorConfig::default()
        },
    );
    for key in ["k1", "k2", "k3"] {
        started(director.trigger(&CueEvent::new(key, "hit"), 0.0));
    }
    assert_eq!(
        director.trigger(&CueEvent::new("k3", "hit"), 0.0),
        TriggerOutcome::Duplicate
    );
    started(director.trigger(&CueEvent::new("k1", "hit"), 0.0));
}

#[test]
fn cooldown_drops_retriggers_inside_the_window() {
    let (mut director, _) = setup(
        vec![CueSpec {
            cooldown_ms: 100,
            ..cue("hit", 50, 8)
        }],
        8,
    );
    started(director.trigger(&CueEvent::new("1", "hit"), 1000.0));
    assert_eq!(
        director.trigger(&CueEvent::new("2", "hit"), 1099.0),
        TriggerOutcome::CoolingDown
    );
    started(director.trigger(&CueEvent::new("3", "hit"), 1100.0));
    director.reset();
    started(director.trigger(&CueEvent::new("4", "hit"), 1101.0));
}

#[test]
fn per_cue_cap_steals_that_cues_oldest_voice() {
    let (mut director, mut backend) = setup(vec![cue("hit", 50, 2), cue("ui", 10, 4)], 16);
    let (first, _) = started(director.trigger(&CueEvent::new("1", "hit"), 0.0));
    started(director.trigger(&CueEvent::new("u", "ui"), 1.0));
    started(director.trigger(&CueEvent::new("2", "hit"), 2.0));
    let (_, stolen) = started(director.trigger(&CueEvent::new("3", "hit"), 3.0));
    assert_eq!(stolen, Some(first));
    assert_eq!(director.active_voices(), 3);
    director.pump(&mut backend);
    assert_eq!(backend.stopped(), vec![first]);
}

#[test]
fn full_pool_steals_lowest_priority_then_oldest() {
    let (mut director, _) = setup(
        vec![cue("low", 10, 8), cue("mid", 50, 8), cue("high", 90, 8)],
        3,
    );
    let (mid_old, _) = started(director.trigger(&CueEvent::new("1", "mid"), 0.0));
    let (low, _) = started(director.trigger(&CueEvent::new("2", "low"), 1.0));
    let (mid_new, _) = started(director.trigger(&CueEvent::new("3", "mid"), 2.0));

    // Lowest priority goes first, even though it is newer than `mid_old`.
    let (_, stolen) = started(director.trigger(&CueEvent::new("4", "high"), 3.0));
    assert_eq!(stolen, Some(low));
    // Among equal priority, the oldest goes.
    let (_, stolen) = started(director.trigger(&CueEvent::new("5", "mid"), 4.0));
    assert_eq!(stolen, Some(mid_old));
    // An equal-priority newcomer replaces the oldest of its peers.
    let (_, stolen) = started(director.trigger(&CueEvent::new("6", "mid"), 5.0));
    assert_eq!(stolen, Some(mid_new));
    // A lower-priority cue never steals from higher ones.
    assert_eq!(
        director.trigger(&CueEvent::new("7", "low"), 6.0),
        TriggerOutcome::Rejected
    );
    assert_eq!(director.active_voices(), 3);
}

#[test]
fn finished_voices_free_their_slots() {
    let (mut director, mut backend) = setup(vec![cue("hit", 50, 1)], 1);
    let (voice, _) = started(director.trigger(&CueEvent::new("1", "hit"), 0.0));
    backend.finish(voice);
    director.pump(&mut backend);
    assert_eq!(director.active_voices(), 0);
    let (_, stolen) = started(director.trigger(&CueEvent::new("2", "hit"), 1.0));
    assert_eq!(stolen, None);
}

#[test]
fn distance_fade_and_pan_math() {
    let attenuation = Attenuation {
        min_distance: 2.0,
        max_distance: 12.0,
    };
    assert_eq!(distance_gain(0.0, &attenuation), 1.0);
    assert_eq!(distance_gain(2.0, &attenuation), 1.0);
    assert!((distance_gain(7.0, &attenuation) - 0.5).abs() < 1e-6);
    assert_eq!(distance_gain(12.0, &attenuation), 0.0);
    assert_eq!(distance_gain(50.0, &attenuation), 0.0);

    let listener = Listener {
        position: [10.0, 0.0, 0.0],
        right: [0.0, 0.0, 2.0], // need not be normalised
    };
    let pan = |p| stereo_pan(&listener, p, &attenuation);
    assert!((pan([10.0, 0.0, 5.0]) - 1.0).abs() < 1e-6);
    assert!((pan([10.0, 0.0, -5.0]) + 1.0).abs() < 1e-6);
    assert!(
        pan([15.0, 0.0, 0.0]).abs() < 1e-6,
        "straight ahead is centred"
    );
    let diagonal = pan([14.0, 0.0, 4.0]);
    assert!((diagonal - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-5);
    // Inside min_distance the pan narrows towards centre.
    assert!((pan([10.0, 0.0, 1.0]) - 0.5).abs() < 1e-6);
    assert_eq!(pan([10.0, 0.0, 0.0]), 0.0);
}

#[test]
fn spatial_cues_fade_pan_and_follow_listener_and_entities() {
    let (mut director, mut backend) = setup(
        vec![
            CueSpec {
                spatial: true,
                gain_db: -6.0,
                ..cue("hit", 50, 4)
            },
            cue("ui", 50, 4),
        ],
        8,
    );
    let entity = EntityId(7);
    started(director.trigger(
        &CueEvent::new("1", "hit").on(entity).at([6.0, 0.0, 0.0]),
        0.0,
    ));
    // Non-spatial cues ignore positions.
    started(director.trigger(&CueEvent::new("2", "ui").at([100.0, 0.0, 0.0]), 0.0));
    assert_eq!(
        director.trigger(&CueEvent::new("3", "hit").at([-20.0, 0.0, 0.0]), 0.0),
        TriggerOutcome::Inaudible
    );
    director.pump(&mut backend);
    let starts: Vec<_> = backend
        .take()
        .into_iter()
        .filter_map(|c| match c {
            Command::Start { gain, pan, bus, .. } => Some((gain, pan, bus)),
            _ => None,
        })
        .collect();
    let half_db6 = 0.5 * 10f32.powf(-6.0 / 20.0);
    assert!((starts[0].0 - half_db6).abs() < 1e-5);
    assert!((starts[0].1 - 1.0).abs() < 1e-6);
    assert_eq!(starts[1], (1.0, 0.0, Bus::Effects));

    // Entity moves left of the listener: one Update for the following voice only.
    director.set_entity_position(entity, [-1.0, 0.0, 0.0]);
    director.pump(&mut backend);
    let updates = backend.take();
    assert_eq!(updates.len(), 1);
    assert!(matches!(updates[0], Command::Update { pan, .. } if (pan + 1.0).abs() < 1e-6));

    // Listener turns around: right ear now faces -x.
    director.set_listener([0.0; 3], [-1.0, 0.0, 0.0]);
    director.pump(&mut backend);
    assert!(
        matches!(backend.take()[..], [Command::Update { pan, .. }] if (pan - 1.0).abs() < 1e-6)
    );

    // Unchanged pose: no redundant updates.
    director.set_listener([0.0; 3], [-1.0, 0.0, 0.0]);
    director.pump(&mut backend);
    assert!(backend.take().is_empty());

    // Entity removal stops its voices with a fade, leaves the rest.
    director.stop_entity(entity);
    director.pump(&mut backend);
    assert!(matches!(backend.take()[..], [Command::Stop { fade_ms, .. }] if fade_ms > 0));
    assert_eq!(director.active_voices(), 1);
}

#[test]
fn entity_position_is_used_when_cue_has_none() {
    let (mut director, mut backend) = setup(
        vec![CueSpec {
            spatial: true,
            ..cue("hit", 50, 4)
        }],
        8,
    );
    director.set_entity_position(EntityId(1), [0.0, 0.0, 0.0]);
    director.set_listener([3.5, 0.0, 0.0], [0.0, 1.0, 0.0]);
    started(director.trigger(&CueEvent::new("1", "hit").on(EntityId(1)), 0.0));
    director.pump(&mut backend);
    assert!(matches!(
        backend.take()[..],
        [Command::Start { gain, .. }] if (gain - 0.75).abs() < 1e-6
    ));
}

#[test]
fn bus_volumes_and_mute_map_to_bus_gains() {
    let (mut director, mut backend) = setup(vec![cue("hit", 50, 4)], 8);
    director.apply_settings(AudioSettings {
        master: 0.5,
        effects: 1.0,
        ui: 0.2,
        muted: false,
    });
    director.pump(&mut backend);
    let gains: Vec<_> = backend
        .take()
        .into_iter()
        .map(|c| match c {
            Command::SetBusGain { bus, gain, .. } => (bus, gain),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(gains.len(), 2, "effects unchanged, so not re-sent");
    assert_eq!(gains[0], (Bus::Master, 0.25));
    assert_eq!(gains[1].0, Bus::Ui);
    assert!((gains[1].1 - 0.04).abs() < 1e-6);

    director.apply_settings_command(AudioSettingsCommand::SetMuted(true));
    director.pump(&mut backend);
    assert!(matches!(
        backend.take()[..],
        [Command::SetBusGain {
            bus: Bus::Master,
            gain: 0.0,
            ..
        }]
    ));
    director.apply_settings_command(AudioSettingsCommand::SetMuted(false));
    director.pump(&mut backend);
    assert!(matches!(
        backend.take()[..],
        [Command::SetBusGain {
            bus: Bus::Master,
            gain: 0.25,
            ..
        }]
    ));
}

#[test]
fn suspended_director_drops_cues_and_marks_them_seen() {
    let (mut director, mut backend) = setup(vec![cue("hit", 50, 4)], 8);
    director.set_suspended(true);
    director.set_suspended(true);
    assert_eq!(
        director.trigger(&CueEvent::new("1", "hit"), 0.0),
        TriggerOutcome::Suspended
    );
    director.set_suspended(false);
    assert_eq!(
        director.trigger(&CueEvent::new("1", "hit"), 1.0),
        TriggerOutcome::Duplicate
    );
    director.pump(&mut backend);
    assert_eq!(backend.take(), vec![Command::Suspend, Command::Resume]);
}

#[test]
fn unknown_cues_are_ignored() {
    let (mut director, _) = setup(vec![cue("hit", 50, 4)], 8);
    assert_eq!(
        director.trigger(&CueEvent::new("1", "nope"), 0.0),
        TriggerOutcome::UnknownCue
    );
}

#[test]
fn variants_round_robin_and_seeded_random_is_reproducible() {
    let round_robin = CueSpec::new("rr", &["a", "b"]);
    let random = CueSpec {
        variants: VariantMode::Random,
        pitch_jitter: 0.1,
        ..CueSpec::new("rand", &["a", "b"])
    };
    let run = || {
        let (mut director, mut backend) = setup(vec![round_robin.clone(), random.clone()], 64);
        for i in 0..3 {
            director.trigger(&CueEvent::new(format!("rr{i}"), "rr"), 0.0);
        }
        for i in 0..8 {
            director.trigger(&CueEvent::new(format!("x{i}"), "rand"), 0.0);
        }
        director.pump(&mut backend);
        backend
            .take()
            .into_iter()
            .filter_map(|c| match c {
                Command::Start { clip, rate, .. } => Some((clip, rate)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let first = run();
    assert_eq!(first, run());
    assert_eq!(
        first[..3].iter().map(|(c, _)| *c).collect::<Vec<_>>(),
        [ClipId(0), ClipId(1), ClipId(0)]
    );
    assert!(first[..3].iter().all(|(_, rate)| *rate == 1.0));
    assert!(
        first[3..]
            .iter()
            .all(|(_, rate)| (0.9..=1.1).contains(rate))
    );
    assert!(first[3..].iter().any(|(_, rate)| *rate != 1.0));
}

#[test]
fn manifest_json_round_trip_and_validation() {
    let json = r#"{
        "attenuation": { "min_distance": 2, "max_distance": 40 },
        "clips": [
            { "id": "hit-1", "source": { "url": "sounds/hit-1.ogg" } },
            { "id": "beep", "source": { "tone": { "frequency_hz": 880, "duration_ms": 80, "waveform": "square" } } }
        ],
        "cues": [
            { "kind": "combat.hit", "clips": ["hit-1"], "priority": 80, "cooldown_ms": 40, "spatial": true },
            { "kind": "ui.click", "clips": ["beep"], "bus": "ui" }
        ]
    }"#;
    let manifest = SoundManifest::from_json(json).unwrap();
    assert_eq!(manifest.cues[1].bus, Bus::Ui);
    assert_eq!(manifest.cues[1].priority, 50);
    assert_eq!(manifest.cues[0].max_voices, 4);
    let bad = json.replace(r#"["beep"]"#, r#"["missing"]"#);
    assert!(SoundManifest::from_json(&bad).is_err());
    let dup = json.replace("ui.click", "combat.hit");
    assert!(SoundManifest::from_json(&dup).is_err());
}

#[test]
fn cue_event_deserializes_from_presentation_cue_shape() {
    let event: CueEvent = serde_json::from_str(
        r#"{ "key": "strike:player:1:42:monster:3", "kind": "hit", "entity": 3, "position": [100, 0, 0] }"#,
    )
    .unwrap();
    assert_eq!(event.entity, Some(EntityId(3)));
    assert_eq!(event.position, Some([100.0, 0.0, 0.0]));
}
