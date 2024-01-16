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
    use super::*;

    #[test]
    fn get_from_key() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();
        let k = at.mixer_track_key();
        let t = e.mixer_track(k).unwrap();
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

        e.mixer_track(at.mixer_track_key())
            .unwrap()
            .set_panning(0.42);

        let s = e.audio_track_state(&at).unwrap();
        e.delete_audio_track(at.clone()).unwrap();

        let at_new = e.reconstruct_audio_track(s).unwrap();

        assert_eq!(e.audio_tracks().count(), 1);
        assert_eq!(e.audio_tracks().next(), Some(&at_new));
        assert_eq!(at_new, at);
        assert_eq!(e.mixer_track(at.mixer_track_key()).unwrap().panning(), 0.42);
    }

    #[test]
    fn reconstruct_audio_tracks() {
        let mut e = Engine::dummy();
        let ats: Vec<_> = e.add_audio_tracks(42).unwrap().collect();

        for at in &ats {
            e.mixer_track(at.mixer_track_key())
                .unwrap()
                .set_panning(0.42);
        }

        let mut ss = Vec::new();
        for at in &ats {
            ss.push(e.audio_track_state(at).unwrap());
            e.delete_audio_track(at.clone()).unwrap();
        }

        for (at, s) in zip(ats, ss) {
            let at_new = e.reconstruct_audio_track(s).unwrap();
            assert_eq!(at_new, at);
            assert_eq!(e.mixer_track(at.mixer_track_key()).unwrap().panning(), 0.42);
        }

        assert_eq!(e.audio_tracks().count(), 42);
    }

    #[test]
    fn reconstruct_audio_track_with_clip() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);

        e.add_audio_clip(at.timeline_track_key(), ck, Timestamp::zero(), None)
            .unwrap();

        let s = e.audio_track_state(&at).unwrap();
        e.delete_audio_track(at.clone()).unwrap();

        let at_new = e.reconstruct_audio_track(s).unwrap();
        let acs_new = e.audio_clips(at_new.timeline_track_key()).unwrap();

        assert_eq!(acs_new.count(), 1);
    }
}

mod mixer_tracks {
    use super::*;

    #[test]
    fn set_panning() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();
        let mt = e.mixer_track_mut(at.mixer_track_key()).unwrap();

        mt.set_panning(0.123);

        assert_eq!(mt.panning(), 0.123);
    }

    #[test]
    fn set_volume() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();
        let mt = e.mixer_track_mut(at.mixer_track_key()).unwrap();

        mt.set_volume(0.123);

        assert_eq!(mt.volume(), 0.123);
    }
}

mod audio_clips {
    use adae::error::MoveAudioClipError;

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

        assert_eq!(e.audio_clips(at.timeline_track_key()).unwrap().count(), 0);

        let ck = import_audio_clip(&mut e);
        let r = e.add_audio_clip(at.timeline_track_key(), ck, Timestamp::from_beats(0), None);

        assert!(r.is_ok());
        assert_eq!(e.audio_clips(at.timeline_track_key()).unwrap().count(), 1);
    }

    #[test]
    fn delete_audio_clip() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ac = e
            .add_audio_clip(at.timeline_track_key(), ck, Timestamp::from_beats(0), None)
            .unwrap();

        assert_eq!(e.audio_clips(at.timeline_track_key()).unwrap().count(), 1);

        let r = e.delete_audio_clip(at.timeline_track_key(), ac);

        assert!(r.is_ok());
        assert_eq!(e.audio_clips(at.timeline_track_key()).unwrap().count(), 0);
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
                    at.timeline_track_key(),
                    ck,
                    Timestamp::from_beats(i),
                    Some(Timestamp::from_beats(1)),
                )
                .unwrap(),
            );
        }

        assert_eq!(e.audio_clips(at.timeline_track_key()).unwrap().count(), 42);

        let r = e.delete_audio_clips(at.timeline_track_key(), acs);

        assert!(r.is_ok());
        assert_eq!(e.audio_clips(at.timeline_track_key()).unwrap().count(), 0);
    }

    #[test]
    fn reconstruct_audio_clip() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ac = e
            .add_audio_clip(at.timeline_track_key(), ck, Timestamp::from_beats(0), None)
            .unwrap();

        let s = e.audio_clip(at.timeline_track_key(), ac).unwrap().state();

        e.delete_audio_clip(at.timeline_track_key(), ac).unwrap();

        let ac_new = e
            .reconstruct_audio_clip(at.timeline_track_key(), s)
            .unwrap();

        assert_eq!(e.audio_clips(at.timeline_track_key()).unwrap().count(), 1);
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
                    at.timeline_track_key(),
                    ck,
                    Timestamp::from_beats(i),
                    Some(Timestamp::from_beats(1)),
                )
                .unwrap();
            acs.push(ac);
            ss.push(e.audio_clip(at.timeline_track_key(), ac).unwrap().state())
        }

        e.delete_audio_clips(at.timeline_track_key(), acs.clone())
            .unwrap();

        let acs_new = e
            .reconstruct_audio_clips(at.timeline_track_key(), ss)
            .unwrap();

        assert_eq!(e.audio_clips(at.timeline_track_key()).unwrap().count(), 42);
        assert_eq!(acs, acs_new);
    }

    #[test]
    fn move_audo_clip() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ack = e
            .add_audio_clip(
                at.timeline_track_key(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(1)),
            )
            .unwrap();

        let r = e.audio_clip_move(at.timeline_track_key(), ack, Timestamp::from_beats(1));

        let ac = e.audio_clip(at.timeline_track_key(), ack).unwrap();

        assert_eq!(r, Ok(()));
        assert_eq!(ac.start, Timestamp::from_beats(1));
    }

    #[test]
    fn move_audo_clip_overlapping() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ac = e
            .add_audio_clip(
                at.timeline_track_key(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(1)),
            )
            .unwrap();
        e.add_audio_clip(
            at.timeline_track_key(),
            ck,
            Timestamp::from_beats(1),
            Some(Timestamp::from_beats(2)),
        )
        .unwrap();

        let r = e.audio_clip_move(at.timeline_track_key(), ac, Timestamp::from_beats(2));

        assert_eq!(r, Err(MoveAudioClipError::Overlapping));
    }

    #[test]
    fn crop_audio_clip_start() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ack = e
            .add_audio_clip(
                at.timeline_track_key(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(2)),
            )
            .unwrap();

        let r = e.audio_clip_crop_start(at.timeline_track_key(), ack, Timestamp::from_beats(1));

        let ac = e.audio_clip(at.timeline_track_key(), ack).unwrap();

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
            at.timeline_track_key(),
            ck,
            Timestamp::from_beats(0),
            Some(Timestamp::from_beats(1)),
        )
        .unwrap();
        let ac = e
            .add_audio_clip(
                at.timeline_track_key(),
                ck,
                Timestamp::from_beats(1),
                Some(Timestamp::from_beats(1)),
            )
            .unwrap();

        let r = e.audio_clip_crop_start(at.timeline_track_key(), ac, Timestamp::from_beats(2));

        assert_eq!(r, Err(CropAudioClipError::Overlapping));
    }

    #[test]
    fn crop_audio_clip_end() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ack = e
            .add_audio_clip(
                at.timeline_track_key(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(2)),
            )
            .unwrap();

        let r = e.audio_clip_crop_end(at.timeline_track_key(), ack, Timestamp::from_beats(1));

        let ac = e.audio_clip(at.timeline_track_key(), ack).unwrap();

        assert_eq!(r, Ok(()));
        assert_eq!(ac.start, Timestamp::from_beats(0));
        assert_eq!(ac.length, Some(Timestamp::from_beats(1)));
    }

    #[test]
    fn crop_audio_clip_end_overlapping() {
        let mut e = Engine::dummy();
        let at = e.add_audio_track().unwrap();

        let ck = import_audio_clip(&mut e);
        let ac = e
            .add_audio_clip(
                at.timeline_track_key(),
                ck,
                Timestamp::from_beats(0),
                Some(Timestamp::from_beats(1)),
            )
            .unwrap();
        e.add_audio_clip(
            at.timeline_track_key(),
            ck,
            Timestamp::from_beats(1),
            Some(Timestamp::from_beats(1)),
        )
        .unwrap();

        let r = e.audio_clip_crop_end(at.timeline_track_key(), ac, Timestamp::from_beats(2));

        assert_eq!(r, Err(CropAudioClipError::Overlapping));
    }
}
