use cpal::StreamConfig;

use super::components::mixer::{new_mixer, Mixer, MixerInterface};
use super::{Sample, CHANNELS};
#[cfg(feature = "record_output")]
use crate::wav_recorder::WavRecorder;

/// Creates an corresponding pair of [`AudioThread`] and [`AudioThreadInterface`].
///
/// The [`AudioThreadInterface`] should not live on the actual audio thread.
pub fn new_audio_thread(
    stream_config: &StreamConfig,
    max_buffer_size: usize,
) -> (AudioThreadInterface, AudioThread) {
    let sample_rate = stream_config.sample_rate.0;

    let (mixer_interface, mixer) = new_mixer(max_buffer_size);

    (
        AudioThreadInterface {
            mixer: mixer_interface,
        },
        AudioThread {
            output_channels: stream_config.channels,
            sample_rate,
            max_buffer_size,

            mixer,

            #[cfg(feature = "record_output")]
            recorder: WavRecorder::new(CHANNELS as u16, sample_rate),
        },
    )
}

/// The interface to the audio thread, living elsewhere.
/// Should somehwat mirror the [`AudioThread`].
pub struct AudioThreadInterface {
    pub mixer: MixerInterface,
}

/// Contatins all data that should persist from one buffer output to the next.
pub struct AudioThread {
    output_channels: u16,
    sample_rate: u32,
    max_buffer_size: usize,

    mixer: Mixer,

    #[cfg(feature = "record_output")]
    recorder: WavRecorder,
}
impl AudioThread {
    /// The function called to generate each audio buffer.
    pub fn output<T: cpal::Sample>(&mut self, data: &mut [T]) {
        // In some cases the buffer size can vary from one buffer to the next.
        let buffer_size = data.len() / self.output_channels as usize;
        #[cfg(debug_assertions)]
        if buffer_size > self.max_buffer_size {
            panic!("A buffer of size {} was requested, which exceeds the biggest producible size of {}.", buffer_size, self.max_buffer_size);
        }

        self.mixer.poll();
        let buffer = self.mixer.output(self.sample_rate, buffer_size);

        Self::clip(buffer);

        #[cfg(feature = "record_output")]
        self.recorder.record(buffer);

        // TODO: Scale channel counts.
        debug_assert_eq!(CHANNELS, self.output_channels as usize);
        // Convert to stream's sample type.
        for (in_sample, out_sample) in buffer.iter().zip(data) {
            *out_sample = T::from(in_sample);
        }
    }

    /// Clips the data, limiting its range from -1.0 to 1.0.
    /// This should only be done right before outputting.
    fn clip(buffer: &mut [Sample]) {
        for sample in buffer.iter_mut() {
            *sample = if *sample > 1.0 {
                1.0
            } else if *sample < -1.0 {
                -1.0
            } else {
                *sample
            };
        }
    }
}
