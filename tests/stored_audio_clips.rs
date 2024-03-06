use adae::Engine;

mod utils;
use utils::import_audio_clip;

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
