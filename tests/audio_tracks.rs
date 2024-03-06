use std::iter::zip;

mod utils;
use adae::{AudioTrackKey, AudioTrackState, Engine, Timestamp};
use utils::import_audio_clip;

#[test]
fn get_from_key() {
    let mut e = Engine::dummy();
    let at = e.add_audio_track().unwrap();
    let k = e.audio_mixer_track_key(at).unwrap();
    let t = e.mixer_track(k).unwrap();
    assert_eq!(t.key(), k);
}

#[test]
fn add_audio_track() {
    let mut e = Engine::dummy();
    assert_eq!(e.audio_tracks().count(), 0);

    let at = e.add_audio_track().unwrap();

    assert_eq!(e.audio_tracks().count(), 1);
    assert_eq!(e.audio_tracks().next(), Some(at));
}

#[test]
fn add_audio_tracks() {
    let mut e = Engine::dummy();
    assert_eq!(e.audio_tracks().count(), 0);

    let mut ats: Vec<AudioTrackKey> = e.add_audio_tracks(42).unwrap().collect();

    assert_eq!(ats.len(), 42);
    assert_eq!(e.audio_tracks().count(), 42);
    for at1 in e.audio_tracks() {
        let pos = ats.iter().position(|&at2| at1 == at2).expect(
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

    let r = e.delete_audio_tracks(ats);

    assert!(r.is_ok());
    assert_eq!(e.audio_tracks().count(), 0);
}

#[test]
fn reconstruct_audio_track() {
    let mut e = Engine::dummy();
    let at = e.add_audio_track().unwrap();

    let mt = e.mixer_track(e.audio_mixer_track_key(at).unwrap()).unwrap();
    mt.set_panning(0.42);
    mt.set_volume(0.65);

    let s = e.delete_audio_track(at).unwrap();

    let at_new = e.reconstruct_audio_track(s).unwrap();

    assert_eq!(e.audio_tracks().count(), 1);
    assert_eq!(e.audio_tracks().next(), Some(at_new));
    assert_eq!(at_new, at);
    assert_eq!(
        e.audio_timeline_track_key(at_new),
        e.audio_timeline_track_key(at)
    );
    assert_eq!(e.audio_mixer_track_key(at_new), e.audio_mixer_track_key(at));
    assert_eq!(
        e.mixer_track(e.audio_mixer_track_key(at).unwrap())
            .unwrap()
            .panning(),
        0.42
    );
    assert_eq!(
        e.mixer_track(e.audio_mixer_track_key(at).unwrap())
            .unwrap()
            .volume(),
        0.65
    );
}

#[test]
fn reconstruct_audio_track_repeat() {
    let mut e = Engine::dummy();
    let ats: Vec<AudioTrackKey> = e.add_audio_tracks(42).unwrap().collect();

    for &at in &ats {
        let mt = e.mixer_track(e.audio_mixer_track_key(at).unwrap()).unwrap();
        mt.set_panning(0.42);
        mt.set_volume(0.65);
    }

    let ss: Vec<AudioTrackState> = e
        .delete_audio_tracks(ats.iter().copied())
        .unwrap()
        .collect();

    for (at, s) in zip(ats, ss) {
        let at_new = e.reconstruct_audio_track(s).unwrap();

        assert_eq!(at_new, at);
        assert_eq!(
            e.audio_timeline_track_key(at_new),
            e.audio_timeline_track_key(at)
        );
        assert_eq!(e.audio_mixer_track_key(at_new), e.audio_mixer_track_key(at));
        assert_eq!(
            e.mixer_track(e.audio_mixer_track_key(at).unwrap())
                .unwrap()
                .panning(),
            0.42
        );
        assert_eq!(
            e.mixer_track(e.audio_mixer_track_key(at).unwrap())
                .unwrap()
                .volume(),
            0.65
        );
    }

    assert_eq!(e.audio_tracks().count(), 42);
}

#[test]
fn reconstruct_audio_track_with_clip() {
    let mut e = Engine::dummy();
    let at = e.add_audio_track().unwrap();

    let ck = import_audio_clip(&mut e);

    e.add_audio_clip(
        e.audio_timeline_track_key(at).unwrap(),
        ck,
        Timestamp::zero(),
        None,
    )
    .unwrap();

    let s = e.delete_audio_track(at).unwrap();

    let at_new = e.reconstruct_audio_track(s).unwrap();
    let acs_new = e
        .audio_clips(e.audio_timeline_track_key(at_new).unwrap())
        .unwrap();

    assert_eq!(acs_new.count(), 1);
}

#[test]
fn reconstruct_audio_tracks() {
    let mut e = Engine::dummy();
    let ats: Vec<AudioTrackKey> = e.add_audio_tracks(42).unwrap().collect();

    for &at in &ats {
        let mt = e.mixer_track(e.audio_mixer_track_key(at).unwrap()).unwrap();
        mt.set_panning(0.42);
        mt.set_volume(0.65);
    }

    let ss = e.delete_audio_tracks(ats.iter().copied()).unwrap();

    let ats_new: Vec<AudioTrackKey> = e.reconstruct_audio_tracks(ss).unwrap().collect();

    for (at_new, at) in zip(ats_new, ats) {
        assert_eq!(at_new, at);
        assert_eq!(
            e.audio_timeline_track_key(at_new),
            e.audio_timeline_track_key(at)
        );
        assert_eq!(e.audio_mixer_track_key(at_new), e.audio_mixer_track_key(at));
        assert_eq!(
            e.mixer_track(e.audio_mixer_track_key(at).unwrap())
                .unwrap()
                .panning(),
            0.42
        );
        assert_eq!(
            e.mixer_track(e.audio_mixer_track_key(at).unwrap())
                .unwrap()
                .volume(),
            0.65
        );
    }

    assert_eq!(e.audio_tracks().count(), 42);
}
