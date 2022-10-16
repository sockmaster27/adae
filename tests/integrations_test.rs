extern crate ardae;
use ardae::Engine;

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

// #[test]
// fn create_timeline_track() {
//     let mut e = Engine::new();
//     let key = e.add_track().unwrap();
//     let track = e.track_mut(key).unwrap();
//     let timeline = track.add_timeline();
// }
