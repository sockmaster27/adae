extern crate adae;

use std::{iter::zip, path::Path};

use adae::error::CropAudioClipError;
use adae::{Engine, StoredAudioClipKey, Timestamp};

fn import_audio_clip(e: &mut Engine) -> StoredAudioClipKey {
    e.import_audio_clip(Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test_files/44100 16-bit.wav"
    )))
    .unwrap()
}

#[test]
fn create_dummy_engine() {
    Engine::dummy();
}

#[test]
fn play() {
    let mut e = Engine::dummy();
    e.play();
}

#[test]
fn pause() {
    let mut e = Engine::dummy();
    e.pause();
}

#[test]
fn jump_to() {
    let mut e = Engine::dummy();
    e.jump_to(Timestamp::from_beats(42));
}

#[test]
fn get_playhead_position() {
    let mut e = Engine::dummy();
    let p = e.playhead_position();
    assert_eq!(p, Timestamp::from_beats(0));
}

mod audio_tracks {
    use adae::{AudioTrackKey, AudioTrackState};

    use super::*;

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

        let mut ats = e.add_audio_tracks(42).unwrap();

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

        let r = e.delete_audio_tracks(&ats);

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

        let s = e.audio_track_state(at).unwrap();
        e.delete_audio_track(at).unwrap();

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
        let ats = e.add_audio_tracks(42).unwrap();

        for &at in &ats {
            let mt = e.mixer_track(e.audio_mixer_track_key(at).unwrap()).unwrap();
            mt.set_panning(0.42);
            mt.set_volume(0.65);
        }

        let ss: Vec<AudioTrackState> = ats
            .iter()
            .map(|&at| e.audio_track_state(at).unwrap())
            .collect();
        e.delete_audio_tracks(&ats).unwrap();

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

        let s = e.audio_track_state(at).unwrap();
        e.delete_audio_track(at).unwrap();

        let at_new = e.reconstruct_audio_track(s).unwrap();
        let acs_new = e
            .audio_clips(e.audio_timeline_track_key(at_new).unwrap())
            .unwrap();

        assert_eq!(acs_new.count(), 1);
    }

    #[test]
    fn reconstruct_audio_tracks() {
        let mut e = Engine::dummy();
        let ats = e.add_audio_tracks(42).unwrap();

        for &at in &ats {
            let mt = e.mixer_track(e.audio_mixer_track_key(at).unwrap()).unwrap();
            mt.set_panning(0.42);
            mt.set_volume(0.65);
        }

        let ss: Vec<AudioTrackState> = ats
            .iter()
            .map(|&at| e.audio_track_state(at).unwrap())
            .collect();
        e.delete_audio_tracks(&ats).unwrap();

        let ats_new: Vec<AudioTrackKey> = e.reconstruct_audio_tracks(&ss).unwrap().collect();

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
}

mod mixer_tracks {
    use super::*;

    #[test]
    fn set_panning() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();
        let mt = e
            .mixer_track_mut(e.audio_mixer_track_key(at).unwrap())
            .unwrap();

        mt.set_panning(0.123);

        assert_eq!(mt.panning(), 0.123);
    }

    #[test]
    fn set_volume() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();
        let mt = e
            .mixer_track_mut(e.audio_mixer_track_key(at).unwrap())
            .unwrap();

        mt.set_volume(0.123);

        assert_eq!(mt.volume(), 0.123);
    }
}

mod audio_clips {
    use adae::error::{MoveAudioClipError, MoveAudioClipToTrackError};

    use super::*;

    mod stored {
        use super::*;

        #[test]
        fn import_new_audio_clip() {
            let mut e = Engine::dummy();
            assert_eq!(e.stored_audio_clips().count(), 0);
            import_audio_clip(&mut e);
            assert_eq!(e.stored_audio_clips().count(), 1);
        }

        #[test]
        fn get_from_key() {
            let mut e = Engine::dummy();
            let ck = import_audio_clip(&mut e);

            let ac = e.stored_audio_clip(ck).unwrap();

            assert_eq!(ac.key(), ck);
        }

        #[test]
        fn length() {
            let mut e = Engine::dummy();
            let ck = import_audio_clip(&mut e);

            let ac = e.stored_audio_clip(ck).unwrap();

            assert_eq!(ac.length(), 1_322_978);
        }
    }

    #[test]
    fn add_audio_clip() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at).unwrap())
                .unwrap()
                .count(),
            0
        );

        let ck = import_audio_clip(&mut e);
        let r = e.add_audio_clip(
            e.audio_timeline_track_key(at).unwrap(),
            ck,
            Timestamp::from_beats(0),
            None,
        );

        assert!(r.is_ok());
        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at).unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn delete_audio_clip() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ac = e
            .add_audio_clip(
                e.audio_timeline_track_key(at).unwrap(),
                ck,
                Timestamp::from_beats(0),
                None,
            )
            .unwrap();

        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at).unwrap())
                .unwrap()
                .count(),
            1
        );

        let r = e.delete_audio_clip(ac);

        assert!(r.is_ok());
        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at).unwrap())
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn delete_audio_clips() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();
        let ck = import_audio_clip(&mut e);

        let mut acs = Vec::new();
        for i in 0..42 {
            acs.push(
                e.add_audio_clip(
                    e.audio_timeline_track_key(at).unwrap(),
                    ck,
                    Timestamp::from_beats(i),
                    Some(Timestamp::from_beats(1)),
                )
                .unwrap(),
            );
        }

        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at).unwrap())
                .unwrap()
                .count(),
            42
        );

        let r = e.delete_audio_clips(&acs);

        assert!(r.is_ok());
        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at).unwrap())
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn reconstruct_audio_clip() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ac = e
            .add_audio_clip(
                e.audio_timeline_track_key(at).unwrap(),
                ck,
                Timestamp::from_beats(0),
                None,
            )
            .unwrap();

        let s = e.audio_clip(ac).unwrap().state();

        e.delete_audio_clip(ac).unwrap();

        let ac_new = e
            .reconstruct_audio_clip(e.audio_timeline_track_key(at).unwrap(), s)
            .unwrap();

        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at).unwrap())
                .unwrap()
                .count(),
            1
        );
        assert_eq!(ac, ac_new);
    }

    #[test]
    fn reconstruct_audio_clips() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);

        let mut acs = Vec::new();
        let mut ss = Vec::new();
        for i in 0..42 {
            let ac = e
                .add_audio_clip(
                    e.audio_timeline_track_key(at).unwrap(),
                    ck,
                    Timestamp::from_beats(i),
                    Some(Timestamp::from_beats(1)),
                )
                .unwrap();
            acs.push(ac);
            ss.push(e.audio_clip(ac).unwrap().state())
        }

        e.delete_audio_clips(&acs).unwrap();

        let acs_new = e
            .reconstruct_audio_clips(e.audio_timeline_track_key(at).unwrap(), ss)
            .unwrap();

        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at).unwrap())
                .unwrap()
                .count(),
            42
        );
        assert_eq!(acs, acs_new);
    }

    #[test]
    fn move_audo_clip() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ack = e
            .add_audio_clip(
                e.audio_timeline_track_key(at).unwrap(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(1)),
            )
            .unwrap();

        let r = e.audio_clip_move(ack, Timestamp::from_beats(1));

        let ac = e.audio_clip(ack).unwrap();

        assert_eq!(r, Ok(()));
        assert_eq!(ac.start, Timestamp::from_beats(1));
    }

    #[test]
    fn move_audo_clip_overlapping() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ack = e
            .add_audio_clip(
                e.audio_timeline_track_key(at).unwrap(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(1)),
            )
            .unwrap();
        e.add_audio_clip(
            e.audio_timeline_track_key(at).unwrap(),
            ck,
            Timestamp::from_beats(1),
            Some(Timestamp::from_beats(2)),
        )
        .unwrap();

        let r = e.audio_clip_move(ack, Timestamp::from_beats(2));

        let ac = e.audio_clip(ack).unwrap();

        assert_eq!(r, Err(MoveAudioClipError::Overlapping));
        assert_eq!(ac.start, Timestamp::from_beats(0));
    }

    #[test]
    fn crop_audio_clip_start() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ack = e
            .add_audio_clip(
                e.audio_timeline_track_key(at).unwrap(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(2)),
            )
            .unwrap();

        let r = e.audio_clip_crop_start(ack, Timestamp::from_beats(1));

        let ac = e.audio_clip(ack).unwrap();

        assert_eq!(r, Ok(()));
        assert_eq!(ac.start, Timestamp::from_beats(1));
        assert_eq!(ac.length, Some(Timestamp::from_beats(1)));
    }

    #[test]
    fn crop_audio_clip_start_overlapping() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        e.add_audio_clip(
            e.audio_timeline_track_key(at).unwrap(),
            ck,
            Timestamp::from_beats(0),
            Some(Timestamp::from_beats(1)),
        )
        .unwrap();
        let ack = e
            .add_audio_clip(
                e.audio_timeline_track_key(at).unwrap(),
                ck,
                Timestamp::from_beats(1),
                Some(Timestamp::from_beats(1)),
            )
            .unwrap();

        let r = e.audio_clip_crop_start(ack, Timestamp::from_beats(2));

        let ac = e.audio_clip(ack).unwrap();

        assert_eq!(r, Err(CropAudioClipError::Overlapping));
        assert_eq!(ac.start, Timestamp::from_beats(1));
        assert_eq!(ac.length, Some(Timestamp::from_beats(1)));
    }

    #[test]
    fn crop_audio_clip_end() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ack = e
            .add_audio_clip(
                e.audio_timeline_track_key(at).unwrap(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(2)),
            )
            .unwrap();

        let r = e.audio_clip_crop_end(ack, Timestamp::from_beats(1));

        let ac = e.audio_clip(ack).unwrap();

        assert_eq!(r, Ok(()));
        assert_eq!(ac.start, Timestamp::from_beats(0));
        assert_eq!(ac.length, Some(Timestamp::from_beats(1)));
    }

    #[test]
    fn crop_audio_clip_end_overlapping() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ack = e
            .add_audio_clip(
                e.audio_timeline_track_key(at).unwrap(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(1)),
            )
            .unwrap();
        e.add_audio_clip(
            e.audio_timeline_track_key(at).unwrap(),
            ck,
            Timestamp::from_beats(1),
            Some(Timestamp::from_beats(1)),
        )
        .unwrap();

        let r = e.audio_clip_crop_end(ack, Timestamp::from_beats(2));

        let ac = e.audio_clip(ack).unwrap();

        assert_eq!(r, Err(CropAudioClipError::Overlapping));
        assert_eq!(ac.start, Timestamp::from_beats(0));
        assert_eq!(ac.length, Some(Timestamp::from_beats(1)));
    }

    #[test]
    fn move_audio_clip_to_another_track() {
        let mut e = Engine::dummy();
        let at1 = e.add_audio_track().unwrap();
        let at2 = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ac = e
            .add_audio_clip(
                e.audio_timeline_track_key(at1).unwrap(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(1)),
            )
            .unwrap();

        let r = e.audio_clip_move_to_track(ac, e.audio_timeline_track_key(at2).unwrap());

        assert!(r.is_ok());
        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at1).unwrap())
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at2).unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn move_audio_clip_to_another_track_overlapping() {
        let mut e = Engine::dummy();
        let at1 = e.add_audio_track().unwrap();
        let at2 = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ac = e
            .add_audio_clip(
                e.audio_timeline_track_key(at1).unwrap(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(1)),
            )
            .unwrap();
        e.add_audio_clip(
            e.audio_timeline_track_key(at2).unwrap(),
            ck,
            Timestamp::from_beats(0),
            Some(Timestamp::from_beats(1)),
        )
        .unwrap();

        let r = e.audio_clip_move_to_track(ac, e.audio_timeline_track_key(at2).unwrap());

        assert_eq!(r, Err(MoveAudioClipToTrackError::Overlapping));
        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at1).unwrap())
                .unwrap()
                .count(),
            1
        );
        assert_eq!(
            e.audio_clips(e.audio_timeline_track_key(at2).unwrap())
                .unwrap()
                .count(),
            1
        );
    }
}
