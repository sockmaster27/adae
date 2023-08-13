use cpal::StreamConfig;
use serde::{Deserialize, Serialize};
use std::iter::zip;

use super::components::{
    audio_clip_store::ImportError,
    mixer::{mixer, Mixer, MixerProcessor, MixerState},
    timeline::{timeline, Timeline, TimelineProcessor, TimelineState},
};
use super::{info::Info, Sample, CHANNELS};
#[cfg(feature = "record_output")]
use crate::wav_recorder::WavRecorder;

/// Creates an corresponding pair of [`Processor`] and [`ProcessorInterface`].
///
/// The [`Processor`] should live on the audio thread, while the [`ProcessorInterface`] should not.
pub fn processor(
    state: &ProcessorState,
    stream_config: &StreamConfig,
    max_buffer_size: usize,
) -> (ProcessorInterface, Processor, Vec<ImportError>) {
    let output_channels = stream_config.channels;
    let sample_rate = stream_config.sample_rate.0;

    let (timeline, timeline_processor, import_errors) =
        timeline(&state.timeline, sample_rate, max_buffer_size);
    let (mixer, mixer_processor) = mixer(&state.mixer, max_buffer_size);

    (
        ProcessorInterface { mixer, timeline },
        Processor {
            output_channels,
            sample_rate,
            #[cfg(debug_assertions)]
            max_buffer_size,

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
        import_errors,
    )
}

/// The interface to the processor, living outside of the audio thread.
/// Should somehwat mirror the [`Processor`].
pub struct ProcessorInterface {
    pub mixer: Mixer,
    pub timeline: Timeline,
}
impl ProcessorInterface {
    pub fn state(&self) -> ProcessorState {
        ProcessorState {
            mixer: self.mixer.state(),
            timeline: self.timeline.state(),
        }
    }
}

/// Contatins all data that should persist from one buffer output to the next.
pub struct Processor {
    output_channels: u16,
    sample_rate: u32,
    #[cfg(debug_assertions)]
    max_buffer_size: usize,

    mixer: MixerProcessor,
    timeline: TimelineProcessor,

    #[cfg(feature = "record_output")]
    recorder: WavRecorder,
}
impl Processor {
    /// Synchronize with the [`ProcessorInterface`]
    pub fn poll(&mut self) {
        self.timeline.poll();
        self.mixer.poll();
    }

    /// The function called to generate each audio buffer.
    pub fn output<T: cpal::Sample + cpal::FromSample<Sample>>(&mut self, data: &mut [T]) {
        // In some cases the buffer size can vary from one buffer to the next.
        let buffer_size = data.len() / usize::from(self.output_channels);

        let buffer = self.output_samples(buffer_size);

        // Convert to stream's sample type.
        for (&mut in_sample, out_sample) in zip(buffer, data) {
            *out_sample = T::from_sample(in_sample);
        }
        // TODO: Scale channel counts.
        debug_assert_eq!(CHANNELS, self.output_channels.into());
    }

    fn output_samples(&mut self, buffer_size: usize) -> &mut [Sample] {
        #[cfg(debug_assertions)]
        if buffer_size > self.max_buffer_size {
            panic!("A buffer of size {} was requested, which exceeds the biggest producible size of {}.", buffer_size, self.max_buffer_size);
        }

        let info = Info {
            sample_rate: self.sample_rate,
            buffer_size,
        };
        let timeline_out = self.mixer.source_outs();
        self.timeline.output(timeline_out, &info);
        let buffer = self.mixer.output(&info);

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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default, Hash)]
pub struct ProcessorState {
    mixer: MixerState,
    timeline: TimelineState,
}
