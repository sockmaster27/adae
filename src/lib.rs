#[cfg(any(feature = "custom_debug_output"))]
#[macro_use]
extern crate lazy_static;

#[cfg(feature = "custom_debug_output")]
#[macro_use]
mod custom_output;
#[cfg(feature = "custom_debug_output")]
pub use custom_output::set_output;

#[cfg(any(feature = "test_alloc", test))]
#[macro_use]
mod test_alloc;
#[cfg(not(any(feature = "test_alloc", test)))]
macro_rules! no_heap {
    ($body:block) => {
        $body
    };
}

#[cfg(feature = "record_output")]
mod wav_recorder;

mod engine;
pub use engine::{inverse_meter_scale, meter_scale, Engine, Track, TrackData};
