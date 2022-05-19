use crate::zip;

use super::super::{Sample, CHANNELS};

use super::audio_meter::{new_audio_meter, AudioMeter, AudioMeterInterface};
use super::parameter::{new_f32_parameter, F32Parameter, F32ParameterInterface};
use super::test_tone::{new_test_tone, TestTone};

pub fn new_mixer_track(key: u32, max_buffer_size: usize) -> (MixerTrackInterface, MixerTrack) {
    mixer_track_from_data(
        max_buffer_size,
        &MixerTrackData {
            panning: 0.0,
            volume: 1.0,
            key,
        },
    )
}

pub fn mixer_track_from_data(
    max_buffer_size: usize,
    data: &MixerTrackData,
) -> (MixerTrackInterface, MixerTrack) {
    let (_test_tone_interface, test_tone) = new_test_tone(1.0, max_buffer_size);
    let (panning_interface, panning) = new_f32_parameter(data.panning, max_buffer_size);
    let (volume_interface, volume) = new_f32_parameter(data.volume, max_buffer_size);
    let (meter_interface, meter) = new_audio_meter();
    (
        MixerTrackInterface {
            panning: panning_interface,
            volume: volume_interface,
            meter: meter_interface,

            key: data.key,
        },
        MixerTrack {
            test_tone,
            panning,
            volume,
            meter,
        },
    )
}

#[derive(Debug)]
pub struct MixerTrack {
    test_tone: TestTone,
    panning: F32Parameter,
    volume: F32Parameter,
    meter: AudioMeter,
}
impl MixerTrack {
    pub fn output(&mut self, sample_rate: u32, buffer_size: usize) -> &mut [Sample] {
        let buffer = self.test_tone.output(sample_rate, buffer_size);

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

    fn pan(panning: f32, frame: &mut [Sample]) {
        // TODO: Pan laws
        let mut left_multiplier = -panning + 1.0;
        left_multiplier = if left_multiplier > 1.0 {
            1.0
        } else {
            left_multiplier
        };
        frame[0] *= left_multiplier;

        let mut right_multiplier = panning + 1.0;
        right_multiplier = if right_multiplier > 1.0 {
            1.0
        } else {
            right_multiplier
        };
        frame[1] *= right_multiplier;
    }
}

#[derive(Debug)]
pub struct MixerTrackInterface {
    pub panning: F32ParameterInterface,
    pub volume: F32ParameterInterface,
    pub meter: AudioMeterInterface,

    key: u32,
}
impl MixerTrackInterface {
    pub fn key(&self) -> u32 {
        self.key
    }

    /// Takes a snapshot of the current state of the track
    pub fn data(&self) -> MixerTrackData {
        MixerTrackData {
            panning: self.panning.get(),
            volume: self.volume.get(),
            key: self.key(),
        }
    }
}

/// Contains all info about the tracks state,
/// that is relevant to reconstructing it
pub struct MixerTrackData {
    pub panning: f32,
    pub volume: f32,

    pub key: u32,
}

#[cfg(test)]
mod tests {
    use super::MixerTrack;

    #[test]
    fn pan_center() {
        let mut signal = [2.0, 3.0];

        MixerTrack::pan(0.0, &mut signal);

        assert_eq!(signal, [2.0, 3.0]);
    }

    #[test]
    fn pan_left() {
        let mut signal = [2.0, 3.0];

        MixerTrack::pan(-1.0, &mut signal);

        assert_eq!(signal, [2.0, 0.0]);
    }

    #[test]
    fn pan_right() {
        let mut signal = [2.0, 3.0];

        MixerTrack::pan(1.0, &mut signal);

        assert_eq!(signal, [0.0, 3.0]);
    }
}
