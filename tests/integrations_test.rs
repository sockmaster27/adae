extern crate ardae;

use std::path::Path;

use ardae::{Engine, Timestamp};

#[test]
fn create_engine() {
    Engine::new();
}

#[test]
fn get_track_from_key() {
    let mut e = Engine::new();
    let k = e.add_track().unwrap();
    let t = e.track(k).unwrap();
    assert_eq!(t.key(), k);
}

#[test]
fn create_audio_track() {
    let mut e = Engine::new();

    let (tltk, tk) = e.add_audio_track().unwrap();

    let t = e.track(tk).unwrap();
    assert!(e.timeline().key_in_use(tltk));
    assert_eq!(t.key(), tk);
}

#[test]
fn add_audio_clip() {
    let mut e = Engine::new();
    let (tltk, _tk) = e.add_audio_track().unwrap();

    let ck = e
        .timeline_mut()
        .import_audio_clip(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test_files/44100 16-bit.wav"
        )))
        .unwrap();
    let r = e
        .timeline_mut()
        .add_clip(tltk, ck, Timestamp::from_beats(0), None);

    assert!(r.is_ok());
}
