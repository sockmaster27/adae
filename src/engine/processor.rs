use crate::zip;

use cpal::StreamConfig;

use super::{
    components::{
        event_queue::{new_event_queue, EventQueue, EventQueueProcessor},
        mixer::{new_mixer, Mixer, MixerProcessor},
        timeline::{new_timeline, Timeline, TimelineProcessor},
    },
    traits::{Component, Info, Source},
};
use super::{Sample, CHANNELS};
#[cfg(feature = "record_output")]
use crate::wav_recorder::WavRecorder;

/// Creates an corresponding pair of [`Processor`] and [`ProcessorInterface`].
///
/// The [`Processor`] should live on the audio thread, while the [`ProcessorInterface`] should not.
pub fn new_processor(
    stream_config: &StreamConfig,
    max_buffer_size: usize,
) -> (ProcessorInterface, Processor) {
    let sample_rate = stream_config.sample_rate.0;

    let (mut global_events, global_events_processor) = new_event_queue();
    let (timeline, timeline_processor) = new_timeline();
    let (mixer, mixer_processor) = new_mixer(&mut global_events, max_buffer_size);

    (
        ProcessorInterface {
            global_events,
            mixer,
            timeline,
        },
        Processor {
            output_channels: stream_config.channels,
            sample_rate,
            max_buffer_size,

            global_events: global_events_processor,
            mixer: mixer_processor,
            timeline: timeline_processor,

            #[cfg(feature = "record_output")]
            recorder: WavRecorder::new(
                CHANNELS
                    .try_into()
                    .expect("Too many channels to record to .wav"),
                sample_rate,
            ),
        },
    )
}

/// The interface to the processor, living outside of the audio thread.
/// Should somehwat mirror the [`Processor`].
pub struct ProcessorInterface {
    pub global_events: EventQueue,
    pub mixer: Mixer,
    pub timeline: Timeline,
}

/// Contatins all data that should persist from one buffer output to the next.
pub struct Processor {
    output_channels: u16,
    sample_rate: u32,
    max_buffer_size: usize,

    global_events: EventQueueProcessor,
    mixer: MixerProcessor,
    timeline: TimelineProcessor,

    #[cfg(feature = "record_output")]
    recorder: WavRecorder,
}
impl Processor {
    /// The function called to generate each audio buffer.
    pub fn output<T: cpal::Sample>(&mut self, data: &mut [T]) {
        // In some cases the buffer size can vary from one buffer to the next.
        let buffer_size = data.len() / usize::from(self.output_channels);

        let buffer = self.output_samples(buffer_size);

        // Convert to stream's sample type.
        for (in_sample, out_sample) in zip!(buffer, data) {
            *out_sample = T::from(in_sample);
        }
        // TODO: Scale channel counts.
        debug_assert_eq!(CHANNELS, self.output_channels.into());
    }

    fn output_samples(&mut self, buffer_size: usize) -> &mut [Sample] {
        #[cfg(debug_assertions)]
        if buffer_size > self.max_buffer_size {
            panic!("A buffer of size {} was requested, which exceeds the biggest producible size of {}.", buffer_size, self.max_buffer_size);
        }

        let mut event_consumer = self.global_events.event_consumer();
        self.mixer.poll(&mut event_consumer);
        event_consumer.poll();

        let buffer = self.mixer.output(&Info::new(self.sample_rate, buffer_size));

        Self::clip(buffer);

        #[cfg(feature = "record_output")]
        self.recorder.record(buffer);

        buffer
    }

    /// Clips the data, limiting its range from -1.0 to 1.0.
    /// This should only be done right before outputting.
    fn clip(buffer: &mut [Sample]) {
        for sample in buffer.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }
}
