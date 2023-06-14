#[cfg(any(feature = "custom_debug_output"))]
#[macro_use]
extern crate lazy_static;

#[cfg(feature = "custom_debug_output")]
#[macro_use]
mod custom_output;
#[cfg(feature = "custom_debug_output")]
pub use custom_output::set_output;

#[cfg(any(debug_assertions, test))]
#[macro_use]
mod test_alloc;
#[cfg(not(any(debug_assertions, test)))]
macro_rules! no_heap {
    ($body:block) => {
        $body
    };
}
#[cfg(not(any(debug_assertions, test)))]
macro_rules! allow_heap {
    ($body:block) => {
        $body
    };
}

#[cfg(any(feature = "record_output", test))]
mod wav_recorder;

mod engine;
pub use engine::{
    inverse_meter_scale, meter_scale, AudioClipKey, AudioTrack, AudioTrackState, Engine, Timestamp,
    Track,
};
