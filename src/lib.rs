#![allow(clippy::inconsistent_digit_grouping)]

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
    error, inverse_meter_scale, meter_scale, AudioClip, AudioClipKey, AudioClipState, AudioTrack,
    AudioTrackState, Engine, EngineState, MixerTrack, MixerTrackKey, StoredAudioClip,
    StoredAudioClipKey, TimelineTrackKey, Timestamp,
};

pub mod config {
    use super::engine::config;

    // Expose everything except the error types, which are instead placed in the error module.
    pub use config::{
        Config, Host, OutputConfig, OutputConfigRange, OutputDevice, SampleFormat,
        SampleFormatFloat, SampleFormatInt, SampleFormatIntUnsigned,
    };
}
