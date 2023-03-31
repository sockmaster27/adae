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
fn add_audio_track() {
    let mut e = Engine::dummy();
    assert_eq!(e.audio_tracks().count(), 0);

    let at = e.add_audio_track().unwrap();

    assert_eq!(e.audio_tracks().count(), 1);
    assert_eq!(e.audio_tracks().next(), Some(&at));
}

#[test]
fn add_audio_tracks() {
    let mut e = Engine::dummy();
    assert_eq!(e.audio_tracks().count(), 0);

    let mut ats: Vec<_> = e.add_audio_tracks(42).unwrap().collect();

    assert_eq!(ats.len(), 42);
    assert_eq!(e.audio_tracks().count(), 42);
    for at1 in e.audio_tracks() {
        let pos = ats.iter().position(|at2| at1 == at2).expect(
            "add_audio_tracks() returned different tracks than the ones added to audio_tracks()",
        );
        ats.swap_remove(pos);
    }
    assert_eq!(ats.len(), 0);
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
