extern crate adae;

use std::{path::Path, thread::sleep, time::Duration};

use adae::{TimelineTrackKey, Timestamp};

#[test]
fn play_around() {
    // Start engine
    let mut engine = adae::Engine::dummy();

    // Add tracks
    let audio_track_keys = engine.add_audio_tracks(3).unwrap();
    assert!(audio_track_keys.len() == 3);
    let timeline_track_keys: Vec<TimelineTrackKey> = audio_track_keys
        .into_iter()
        .map(|k| engine.audio_timeline_track_key(k).unwrap())
        .collect();

    // Play and stop
    engine.play();
    sleep(Duration::from_secs(1));
    engine.pause();

    // Add clips
    let clip_key = engine
        .import_audio_clip(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test_files/44100 16-bit.wav"
        )))
        .unwrap();
    engine
        .add_audio_clip(
            timeline_track_keys[0],
            clip_key,
            Timestamp::from_beats(2),
            None,
        )
        .unwrap();
    engine
        .add_audio_clip(
            timeline_track_keys[1],
            clip_key,
            Timestamp::from_beats(1),
            Some(Timestamp::from_beats(2)),
        )
        .unwrap();

    // Jump to start and play
    engine.jump_to(Timestamp::zero());
    engine.play();
    sleep(Duration::from_secs(1));

    // Insert clip before playhead while playing
    engine
        .add_audio_clip(
            timeline_track_keys[2],
            clip_key,
            Timestamp::from_beats(0),
            None,
        )
        .unwrap();

    sleep(Duration::from_secs(1));
    engine.pause();

    // Close and load from state
    let state = engine.state();
    drop(engine);
    let (engine, import_erros) = adae::Engine::dummy_from_state(&state);
    assert!(import_erros.is_empty());
    assert_eq!(engine.state(), state);
}
