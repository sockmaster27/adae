mod utils;
use adae::{
    error::{MoveAudioClipError, MoveAudioClipToTrackError},
    AudioClipKey, Engine, Timestamp,
};
use utils::import_audio_clip;

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

    let r = e.delete_audio_clips(acs);

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

    let s = e.delete_audio_clip(ac).unwrap();

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
    }

    let ss = e.delete_audio_clips(acs.iter().copied()).unwrap();

    let acs_new: Vec<AudioClipKey> = e
        .reconstruct_audio_clips(e.audio_timeline_track_key(at).unwrap(), ss)
        .unwrap()
        .collect();

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
    assert_eq!(ac.start(), Timestamp::from_beats(1));
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
    assert_eq!(ac.start(), Timestamp::from_beats(0));
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
    assert_eq!(ac.start(), Timestamp::from_beats(1));
    assert_eq!(ac.length(e.bpm_cents()), Timestamp::from_beats(1));
}

#[test]
fn crop_audio_clip_start_overlapping() {
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
    e.audio_clip_crop_start(ack, Timestamp::from_beats(1))
        .unwrap();

    e.add_audio_clip(
        e.audio_timeline_track_key(at).unwrap(),
        ck,
        Timestamp::from_beats(0),
        Some(Timestamp::from_beats(1)),
    )
    .unwrap();

    let r = e.audio_clip_crop_start(ack, Timestamp::from_beats(2));

    let ac = e.audio_clip(ack).unwrap();

    assert_eq!(r, Err(MoveAudioClipError::Overlapping));
    assert_eq!(ac.start(), Timestamp::from_beats(1));
    assert_eq!(ac.length(e.bpm_cents()), Timestamp::from_beats(1));
}

#[test]
fn crop_audio_clip_start_too_long() {
    let mut e = Engine::dummy();
    let at = e.add_audio_track().unwrap();

    let ck = import_audio_clip(&mut e);
    let ack = e
        .add_audio_clip(
            e.audio_timeline_track_key(at).unwrap(),
            ck,
            Timestamp::from_beats(1),
            Some(Timestamp::from_beats(2)),
        )
        .unwrap();

    // When stretched beyond its original length
    e.audio_clip_crop_start(ack, Timestamp::from_beats(1))
        .unwrap();
    e.audio_clip_crop_start(ack, Timestamp::from_beats(3))
        .unwrap();

    let ac = e.audio_clip(ack).unwrap();

    // Then it's capped at the original length
    assert_eq!(ac.start(), Timestamp::from_beats(1));
    assert_eq!(ac.length(e.bpm_cents()), Timestamp::from_beats(2));
}

#[test]
fn crop_audio_clip_start_originally_before_zero() {
    let mut e = Engine::dummy();
    let at = e.add_audio_track().unwrap();

    // Given a clip whose inner start is before zero
    let ck = import_audio_clip(&mut e);
    let ack = e
        .add_audio_clip(
            e.audio_timeline_track_key(at).unwrap(),
            ck,
            Timestamp::from_beats(0),
            Some(Timestamp::from_beats(3)),
        )
        .unwrap();
    e.audio_clip_crop_start(ack, Timestamp::from_beats(2))
        .unwrap();
    e.audio_clip_move(ack, Timestamp::from_beats(0)).unwrap();

    // When start is cropped it should not overflow
    e.audio_clip_crop_start(ack, Timestamp::from_beats(1))
        .unwrap();
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
    assert_eq!(ac.start(), Timestamp::from_beats(0));
    assert_eq!(ac.length(e.bpm_cents()), Timestamp::from_beats(1));
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

    assert_eq!(r, Err(MoveAudioClipError::Overlapping));
    assert_eq!(ac.start(), Timestamp::from_beats(0));
    assert_eq!(ac.length(e.bpm_cents()), Timestamp::from_beats(1));
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

    let r = e.audio_clip_move_to_track(
        ac,
        Timestamp::from_beats(42),
        e.audio_timeline_track_key(at2).unwrap(),
    );

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
    assert_eq!(e.audio_clip(ac).unwrap().start(), Timestamp::from_beats(42));
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
        Timestamp::from_beats(2),
        Some(Timestamp::from_beats(1)),
    )
    .unwrap();

    let r = e.audio_clip_move_to_track(
        ac,
        Timestamp::from_beats(2),
        e.audio_timeline_track_key(at2).unwrap(),
    );

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
    assert_eq!(e.audio_clip(ac).unwrap().start(), Timestamp::from_beats(0));
}

#[test]
fn waveform() {
    let mut e = Engine::dummy();
    let at = e.add_audio_track().unwrap();

    let ck = import_audio_clip(&mut e);
    let ac = e
        .add_audio_clip(
            e.audio_timeline_track_key(at).unwrap(),
            ck,
            Timestamp::from_beats(0),
            Some(Timestamp::from_beats(1)),
        )
        .unwrap();

    let bpm_cents = e.bpm_cents();

    let w = e.audio_clip_mut(ac).unwrap().waveform(bpm_cents);

    assert_eq!(w.len(), 2 * 2 * 42);
}
