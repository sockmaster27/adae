extern crate ardae;

use std::path::Path;

use ardae::{Engine, Timestamp};

#[test]
fn create_engine() {
    Engine::new();
}

#[test]
fn create_dummy_engine() {
    Engine::dummy();
}

#[test]
fn get_track_from_key() {
    let mut e = Engine::dummy();
    let at = e.add_audio_track().unwrap();
    let k = at.track_key();
    let t = e.track(k).unwrap();
    assert_eq!(t.key(), k);
}

#[test]
fn add_audio_clip() {
    let mut e = Engine::dummy();
    let at = e.add_audio_track().unwrap();

    let ck = e
        .import_audio_clip(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test_files/44100 16-bit.wav"
        )))
        .unwrap();
    let r = e.add_clip(at.timeline_track_key(), ck, Timestamp::from_beats(0), None);

    assert!(r.is_ok());
}
