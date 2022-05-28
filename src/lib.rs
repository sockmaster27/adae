#[cfg(feature = "custom_debug_output")]
#[macro_use]
extern crate lazy_static;

#[cfg(feature = "custom_debug_output")]
#[macro_use]
mod custom_output;
#[cfg(feature = "record_output")]
mod wav_recorder;

#[cfg(feature = "custom_debug_output")]
pub use custom_output::set_output;
mod engine;
pub use engine::{Engine, MixerTrack, MixerTrackData};
