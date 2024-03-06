use adae::{Engine, Timestamp};

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
