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
    let stored_clip_key = engine
        .import_audio_clip(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test_files/44100 16-bit.wav"
        )))
        .unwrap();
    engine
        .add_audio_clip(
            timeline_track_keys[0],
            stored_clip_key,
            Timestamp::from_beats(2),
            None,
        )
        .unwrap();
    engine
        .add_audio_clip(
            timeline_track_keys[1],
            stored_clip_key,
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
            stored_clip_key,
            Timestamp::zero(),
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

#[test]
fn stretch_around() {
    // Start engine
    let mut engine = adae::Engine::dummy();

    // Add tracks
    let audio_track_key = engine.add_audio_track().unwrap();
    let timeline_track_key = engine.audio_timeline_track_key(audio_track_key).unwrap();

    // Add clip
    let stored_clip_key = engine
        .import_audio_clip(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test_files/44100 16-bit.wav"
        )))
        .unwrap();
    let audio_clip_key = engine
        .add_audio_clip(
            timeline_track_key,
            stored_clip_key,
            Timestamp::zero(),
            Some(Timestamp::from_beats(30)),
        )
        .unwrap();

    let new_length = Timestamp::from_beats(20) + Timestamp::from_beat_units(1);
    engine
        .audio_clip_crop_start(audio_clip_key, new_length)
        .unwrap();
    engine
        .audio_clip_crop_start(audio_clip_key, Timestamp::from_beats(30))
        .unwrap();

    // No rounding error should result in an overflow
}
