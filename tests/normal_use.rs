extern crate adae;

use std::{path::Path, thread::sleep, time::Duration};

use adae::Timestamp;

#[test]
fn play_around() {
    // Start engine
    let mut engine = adae::Engine::dummy();

    // Add tracks
    let track_keys: Vec<_> = engine.add_audio_tracks(3).unwrap().collect();
    assert!(track_keys.len() == 3);

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
        .add_clip(
            track_keys[0].timeline_track_key(),
            clip_key,
            Timestamp::from_beats(2),
            None,
        )
        .unwrap();
    engine
        .add_clip(
            track_keys[1].timeline_track_key(),
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
        .add_clip(
            track_keys[2].timeline_track_key(),
            clip_key,
            Timestamp::from_beats(0),
            None,
        )
        .unwrap();

    sleep(Duration::from_secs(1));
    engine.pause();
}
