use cpal::StreamConfig;

use super::components::{
    new_peak_meter, DelayPoint, MixPoint, PeakMeter, PeakMeterInterface, TestToneGenerator,
};
use super::{Sample, CHANNELS};
#[cfg(feature = "record_output")]
use crate::wav_recorder::WavRecorder;

#[derive(Debug)]
pub enum Event {
    /// Sets the gain multiplier of the test tone.
    SetVolume(f32),
}

/// Creates an corresponding pair of `AudioThread` and `AudioThreadInterface`.
///
/// The `AudioThreadInterface` should not live on the actual audio thread.
pub fn new_audio_thread(
    stream_config: &StreamConfig,
    max_buffer_size: usize,
    event_receiver: ringbuf::Consumer<Event>,
) -> (AudioThreadInterface, AudioThread) {
    let sample_rate = stream_config.sample_rate.0;

    let (peak_meter_interface, peak_meter) = new_peak_meter();

    (
        AudioThreadInterface {
            peak_meter: peak_meter_interface,
        },
        AudioThread {
            output_channels: stream_config.channels,
            sample_rate,
            max_buffer_size,
            event_receiver,

            test_tone1: TestToneGenerator::new(max_buffer_size),
            test_tone2: TestToneGenerator::new(max_buffer_size),
            delay: DelayPoint::new(48000),
            mixer: MixPoint::new(max_buffer_size),
            peak_meter,

            #[cfg(feature = "record_output")]
            recorder: WavRecorder::new(CHANNELS as u16, sample_rate),
        },
    )
}

/// The interface to the audio thread, living elsewhere.
/// Should somehwat mirror the `AudioThread`.
pub struct AudioThreadInterface {
    pub peak_meter: PeakMeterInterface,
}

/// Contatins all data that should persist from one buffer output to the next.
pub struct AudioThread {
    output_channels: u16,
    sample_rate: u32,
    max_buffer_size: usize,
    event_receiver: ringbuf::Consumer<Event>,

    test_tone1: TestToneGenerator,
    test_tone2: TestToneGenerator,
    delay: DelayPoint,
    mixer: MixPoint,

    peak_meter: PeakMeter,

    #[cfg(feature = "record_output")]
    recorder: WavRecorder,
}
impl AudioThread {
    /// Goes through the event queue, and makes the necessary changes to the state.
    fn poll_events(&mut self) {
        self.event_receiver.pop_each(
            |event| {
                #[allow(unreachable_patterns)]
                match event {
                    Event::SetVolume(value) => self.test_tone1.gain.set(value),
                    _ => todo!("Add more events"),
                }

                // Return true to loop all the way through the iterable.
                true
            },
            None,
        );
    }

    /// The function called to generate each audio buffer.
    pub fn output<T: cpal::Sample>(&mut self, data: &mut [T]) {
        // In some cases the buffer size can vary from one buffer to the next.
        let buffer_size = data.len() / self.output_channels as usize;
        #[cfg(debug_assertions)]
        if buffer_size > self.max_buffer_size {
            panic!("A buffer of size {} was requested, which exceeds the biggest producible size of {}.", buffer_size, self.max_buffer_size);
        }

        self.poll_events();

        self.test_tone2.gain.set(0.5);
        let tone1 = self.test_tone1.output(self.sample_rate, buffer_size);
        let tone2 = self.test_tone2.output(self.sample_rate, buffer_size);
        self.delay.next(tone2);
        let buffer = self.mixer.mix(&[tone1, tone2]);
        debug_assert_eq!(buffer.len(), data.len());

        self.peak_meter.report(buffer);

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
