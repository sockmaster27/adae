use crate::zip;

use std::fmt::Debug;

use super::super::utils::format_truncate_list;
use super::super::{Sample, CHANNELS};
use super::parameter::{new_f32_parameter, F32Parameter, F32ParameterInterface};

pub fn new_test_tone(initial_volume: f32, max_buffer_size: usize) -> (TestToneInterface, TestTone) {
    let (volume_interface, volume) = new_f32_parameter(initial_volume, max_buffer_size);

    (
        TestToneInterface {
            volume: volume_interface,
        },
        TestTone {
            volume,

            buffer: vec![0.0; max_buffer_size * CHANNELS],
            sample_clock: 0.0,
        },
    )
}

/// Generates a 440 Hz sine wave.
pub struct TestTone {
    pub volume: F32Parameter,

    buffer: Vec<Sample>,
    sample_clock: f32,
}
impl TestTone {
    pub fn output(&mut self, sample_rate: u32, buffer_size: usize) -> &mut [Sample] {
        const FREQUENCY: f32 = 440.0;
        let sample_rate = sample_rate as f32;
        let phase_length = sample_rate / FREQUENCY;

        let volume_buffer = self.volume.get(buffer_size);

        for (frame, &mut volume) in zip!(
            self.buffer[..buffer_size * CHANNELS].chunks_mut(CHANNELS),
            volume_buffer
        ) {
            self.sample_clock += 1.0;
            self.sample_clock %= phase_length;

            let value = ((self.sample_clock * 2.0 * std::f32::consts::PI) / phase_length).sin();

            for sample in frame {
                *sample = volume * value;
            }
        }
        &mut self.buffer[..buffer_size * CHANNELS]
    }
}
impl Debug for TestTone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TestTone {{ volume: {:?}, buffer: {}, sample_clock: {:?} }}",
            self.volume,
            format_truncate_list(5, &self.buffer[..]),
            self.sample_clock,
        )
    }
}

pub struct TestToneInterface {
    pub volume: F32ParameterInterface,
}
