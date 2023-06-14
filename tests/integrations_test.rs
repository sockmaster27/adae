extern crate adae;

use std::{iter::zip, path::Path};

use adae::{Engine, Timestamp};

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
fn delete_audio_track() {
    let mut e = Engine::dummy();
    let at = e.add_audio_track().unwrap();

    let r = e.delete_audio_track(at);

    assert!(r.is_ok());
    assert_eq!(e.audio_tracks().count(), 0);
}

#[test]
fn delete_audio_tracks() {
    let mut e = Engine::dummy();
    let ats = e.add_audio_tracks(42).unwrap();

    let r = e.delete_audio_tracks(ats.collect());

    assert!(r.is_ok());
    assert_eq!(e.audio_tracks().count(), 0);
}

#[test]
fn reconstruct_audio_track() {
    let mut e = Engine::dummy();
    let at = e.add_audio_track().unwrap();

    e.track(at.track_key()).unwrap().set_panning(0.42);

    let s = e.audio_track_state(&at).unwrap();
    e.delete_audio_track(at.clone()).unwrap();

    let at_new = e.reconstruct_audio_track(s).unwrap();

    assert_eq!(e.audio_tracks().count(), 1);
    assert_eq!(e.audio_tracks().next(), Some(&at_new));
    assert_eq!(at_new, at);
    assert_eq!(e.track(at.track_key()).unwrap().panning(), 0.42);
}

#[test]
fn reconstruct_audio_tracks() {
    let mut e = Engine::dummy();
    let ats: Vec<_> = e.add_audio_tracks(42).unwrap().collect();

    for at in &ats {
        e.track(at.track_key()).unwrap().set_panning(0.42);
    }

    let mut ss = Vec::new();
    for at in &ats {
        ss.push(e.audio_track_state(&at).unwrap());
        e.delete_audio_track(at.clone()).unwrap();
    }

    for (at, s) in zip(ats, ss) {
        let at_new = e.reconstruct_audio_track(s).unwrap();
        assert_eq!(at_new, at);
        assert_eq!(e.track(at.track_key()).unwrap().panning(), 0.42);
    }

    assert_eq!(e.audio_tracks().count(), 42);
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
