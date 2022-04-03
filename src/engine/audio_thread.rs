use cpal::StreamConfig;

use super::components::audio_meter::{new_audio_meter, AudioMeter, AudioMeterInterface};
use super::components::test_tone::{new_test_tone, TestTone, TestToneInterface};
use super::components::{DelayPoint, MixPoint};
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

    let (test_tone_interface1, test_tone1) = new_test_tone(1.0, max_buffer_size);
    let (_, test_tone2) = new_test_tone(0.5, max_buffer_size);
    let (audio_meter_interface, audio_meter) = new_audio_meter();

    (
        AudioThreadInterface {
            tone: test_tone_interface1,
            audio_meter: audio_meter_interface,
        },
        AudioThread {
            output_channels: stream_config.channels,
            sample_rate,
            max_buffer_size,

            test_tone1,
            test_tone2,
            delay: DelayPoint::new(48000),
            mixer: MixPoint::new(max_buffer_size),
            audio_meter,

            #[cfg(feature = "record_output")]
            recorder: WavRecorder::new(CHANNELS as u16, sample_rate),
        },
    )
}

/// The interface to the audio thread, living elsewhere.
/// Should somehwat mirror the [`AudioThread`].
pub struct AudioThreadInterface {
    tone: TestToneInterface,
    pub audio_meter: AudioMeterInterface,
}
impl AudioThreadInterface {
    pub fn set_volume(&self, value: f32) {
        self.tone.volume.set(value);
    }
}

/// Contatins all data that should persist from one buffer output to the next.
pub struct AudioThread {
    output_channels: u16,
    sample_rate: u32,
    max_buffer_size: usize,

    test_tone1: TestTone,
    test_tone2: TestTone,
    delay: DelayPoint,
    mixer: MixPoint,

    audio_meter: AudioMeter,

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

        let tone1 = self.test_tone1.output(self.sample_rate, buffer_size);
        let tone2 = self.test_tone2.output(self.sample_rate, buffer_size);
        self.delay.next(tone2);

        self.mixer.reset();
        self.mixer.add(tone1);
        self.mixer.add(tone2);
        let buffer = self.mixer.get().unwrap();
        debug_assert_eq!(buffer.len(), data.len());

        self.audio_meter.report(buffer, self.sample_rate as f32);

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
