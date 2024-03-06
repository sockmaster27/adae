use adae::Engine;

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
