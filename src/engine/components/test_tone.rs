use std::fmt::Debug;

use crate::engine::traits::{Info, Source};
use crate::engine::utils::format_truncate_list;
use crate::engine::{Sample, CHANNELS};

/// Generates a 440 Hz sine wave.
pub struct TestTone {
    sample_clock: f32,
    buffer: Vec<Sample>,
}
impl TestTone {
    pub fn new(max_buffer_size: usize) -> Self {
        TestTone {
            buffer: vec![0.0; max_buffer_size * CHANNELS],
            sample_clock: 0.0,
        }
    }
}
impl Source for TestTone {
    fn output(&mut self, info: Info) -> &mut [Sample] {
        let Info {
            sample_rate,
            buffer_size,
        } = info;

        const FREQUENCY: f32 = 440.0;
        let sample_rate = sample_rate as f32;
        let phase_length = sample_rate / FREQUENCY;

        for frame in self.buffer[..buffer_size * CHANNELS].chunks_mut(CHANNELS) {
            self.sample_clock += 1.0;
            self.sample_clock %= phase_length;

            let value = ((self.sample_clock * 2.0 * std::f32::consts::PI) / phase_length).sin();

            for sample in frame {
                *sample = value;
            }
        }
        &mut self.buffer[..buffer_size * CHANNELS]
    }
}
impl Debug for TestTone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "TestTone {{ sample_clock: {:?}, buffer: {} }}",
            self.sample_clock,
            format_truncate_list(5, &self.buffer[..]),
        )
    }
}
