use super::audio_meter::{audio_meter, AudioMeter, AudioMeterProcessor};
use super::parameter::{f32_parameter, F32Parameter, F32ParameterProcessor};
use crate::engine::traits::{Effect, Info};
use crate::engine::{Sample, CHANNELS};
use crate::zip;

pub type TrackKey = u32;

pub fn track(key: TrackKey, max_buffer_size: usize) -> (Track, TrackProcessor) {
    track_from_state(
        max_buffer_size,
        &TrackState {
            panning: 0.0,
            volume: 1.0,
            key,
        },
    )
}

pub fn track_from_state(max_buffer_size: usize, state: &TrackState) -> (Track, TrackProcessor) {
    let (panning, panning_processor) = f32_parameter(state.panning, max_buffer_size);
    let (volume, volume_processor) = f32_parameter(state.volume, max_buffer_size);
    let (meter, meter_processor) = audio_meter();

    (
        Track {
            key: state.key,

            panning,
            volume,
            meter,
        },
        TrackProcessor {
            panning: panning_processor,
            volume: volume_processor,
            meter: meter_processor,
        },
    )
}

pub struct Track {
    key: TrackKey,

    panning: F32Parameter,
    volume: F32Parameter,
    meter: AudioMeter,
}
impl Track {
    pub fn key(&self) -> TrackKey {
        self.key
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
    pub(crate) fn state(&self) -> TrackState {
        TrackState {
            panning: self.panning.get(),
            volume: self.volume.get(),
            key: self.key(),
        }
    }
}

/// Contains all info about the tracks state,
/// that is relevant to reconstructing it
#[derive(Debug, Clone, PartialEq)]
pub struct TrackState {
    pub panning: f32,
    pub volume: f32,

    pub key: TrackKey,
}

#[derive(Debug)]
pub struct TrackProcessor {
    panning: F32ParameterProcessor,
    volume: F32ParameterProcessor,
    meter: AudioMeterProcessor,
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
impl Effect for TrackProcessor {
    fn process(&mut self, info: &Info, buffer: &mut [Sample]) {
        let Info {
            sample_rate,
            buffer_size,
        } = *info;

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
