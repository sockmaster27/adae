use std::sync::atomic::Ordering;
use std::sync::Arc;

use atomicbox::AtomicOptionBox;

use crate::zip;

use crate::engine::{Sample, CHANNELS};

use super::audio_meter::{new_audio_meter, AudioMeter, AudioMeterProcessor};
use super::event_queue::EventReceiver;
use super::mixer::TrackKey;
use super::parameter::{new_f32_parameter, F32Parameter, F32ParameterProcessor};
use super::test_tone::TestTone;
use super::timeline::{new_timeline_track, TimelineTrack};
use crate::engine::traits::{Component, Info, Source};

pub fn new_track(key: TrackKey, max_buffer_size: usize) -> (Track, TrackProcessor) {
    track_from_data(
        max_buffer_size,
        &TrackData {
            panning: 0.0,
            volume: 1.0,
            key,
        },
    )
}

pub fn track_from_data(max_buffer_size: usize, data: &TrackData) -> (Track, TrackProcessor) {
    let test_tone = TestTone::new(max_buffer_size);
    let source = Box::new(test_tone);

    let (panning, panning_processor) = new_f32_parameter(data.panning, max_buffer_size);
    let (volume, volume_processor) = new_f32_parameter(data.volume, max_buffer_size);
    let (meter, meter_processor) = new_audio_meter();

    let new_source1 = Arc::new(AtomicOptionBox::none());
    let new_source2 = Arc::clone(&new_source1);

    (
        Track {
            max_buffer_size,

            key: data.key,

            panning,
            volume,
            meter,

            new_source: new_source1,
        },
        TrackProcessor {
            source,

            panning: panning_processor,
            volume: volume_processor,
            meter: meter_processor,

            new_source: new_source2,
        },
    )
}

pub struct Track {
    max_buffer_size: usize,

    key: TrackKey,

    panning: F32Parameter,
    volume: F32Parameter,
    meter: AudioMeter,

    // Double-boxed because inner box holds a fat pointer, which can't be atomic
    new_source: Arc<AtomicOptionBox<Box<dyn Source>>>,
}
impl Track {
    pub fn key(&self) -> TrackKey {
        self.key
    }

    pub fn set_source(&self, source: Box<dyn Source>) {
        self.new_source
            .store(Some(Box::new(source)), Ordering::SeqCst);
    }

    pub fn panning(&self) -> Sample {
        self.panning.get()
    }
    pub fn set_panning(&self, value: Sample) {
        self.panning.set(value)
    }

    pub fn volume(&self) -> Sample {
        self.volume.get()
    }
    pub fn set_volume(&self, value: Sample) {
        self.volume.set(value)
    }

    pub fn add_timeline(&self) -> TimelineTrack {
        let (timeline_track, timeline_track_processor) = new_timeline_track(self.max_buffer_size);
        self.set_source(Box::new(timeline_track_processor));
        timeline_track
    }

    /// Returns an array of the signals current peak, long-term peak and RMS-level for each channel in the form:
    /// - `[peak: [left, right], long_peak: [left, right], rms: [left, right]]`
    ///
    /// Results are scaled and smoothed to avoid jittering, suitable for reading every frame.
    /// If this is not desirable see [`Self::read_meter_raw`].
    pub fn read_meter(&mut self) -> [[Sample; CHANNELS]; 3] {
        self.meter.read()
    }
    /// Same as [`Self::read_meter`], except results are not smoothed or scaled.
    ///
    /// Long peak stays in place for 1 second since it was last changed, before snapping to the current peak.
    pub fn read_meter_raw(&self) -> [[Sample; CHANNELS]; 3] {
        self.meter.read_raw()
    }
    /// Snap smoothed rms value to its current unsmoothed equivalent.
    ///
    /// Should be called before [`Self::read_meter`] is called the first time or after a long break,
    /// to avoid meter sliding in place from zero or a very old value.
    pub fn snap_rms(&mut self) {
        self.meter.snap_rms();
    }

    /// Takes a snapshot of the current state of the track
    pub fn data(&self) -> TrackData {
        TrackData {
            panning: self.panning.get(),
            volume: self.volume.get(),
            key: self.key(),
        }
    }
}

/// Contains all info about the tracks state,
/// that is relevant to reconstructing it
#[derive(Clone)]
pub struct TrackData {
    pub panning: f32,
    pub volume: f32,

    pub key: TrackKey,
}

#[derive(Debug)]
pub struct TrackProcessor {
    source: Box<dyn Source>,

    panning: F32ParameterProcessor,
    volume: F32ParameterProcessor,
    meter: AudioMeterProcessor,

    new_source: Arc<AtomicOptionBox<Box<dyn Source>>>,
}
impl TrackProcessor {
    fn pan(panning: f32, frame: &mut [Sample]) {
        // TODO: Pan laws
        let left_multiplier = (-panning + 1.0).clamp(0.0, 1.0);
        frame[0] *= left_multiplier;

        let right_multiplier = (panning + 1.0).clamp(0.0, 1.0);
        frame[1] *= right_multiplier;
    }
}
impl Component for TrackProcessor {
    fn poll<'a, 'b>(&'a mut self, event_receiver: &mut EventReceiver<'a, 'b>) {
        let source_option = self.new_source.take(Ordering::SeqCst);
        if let Some(source_box) = source_option {
            self.source = *source_box;
        }

        self.source.poll(event_receiver);
    }
}
impl Source for TrackProcessor {
    fn output(&mut self, info: &Info) -> &mut [Sample] {
        let Info {
            sample_rate,
            buffer_size,
        } = *info;

        let buffer = self.source.output(info);

        let volume_buffer = self.volume.get(buffer_size);
        let panning_buffer = self.panning.get(buffer_size);

        for ((frame, &mut volume), &mut panning) in
            zip!(buffer.chunks_mut(CHANNELS), volume_buffer, panning_buffer)
        {
            for sample in frame.iter_mut() {
                *sample *= volume;
            }

            Self::pan(panning, frame);
        }

        self.meter.report(&buffer, sample_rate as f32);
        buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_center() {
        let mut signal = [2.0, 3.0];

        TrackProcessor::pan(0.0, &mut signal);

        assert_eq!(signal, [2.0, 3.0]);
    }

    #[test]
    fn pan_left() {
        let mut signal = [2.0, 3.0];

        TrackProcessor::pan(-1.0, &mut signal);

        assert_eq!(signal, [2.0, 0.0]);
    }

    #[test]
    fn pan_right() {
        let mut signal = [2.0, 3.0];

        TrackProcessor::pan(1.0, &mut signal);

        assert_eq!(signal, [0.0, 3.0]);
    }
}
