use super::super::{Sample, CHANNELS};
use super::parameter::F32Parameter;

/// Generates a 440 Hz sine wave.
pub struct TestToneGenerator {
    pub gain: F32Parameter,

    buffer: Vec<Sample>,
    sample_clock: f32,
}
impl TestToneGenerator {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            gain: F32Parameter::new(1.0, max_buffer_size),

            buffer: vec![0.0; max_buffer_size * CHANNELS],
            sample_clock: 0.0,
        }
    }

    pub fn output(&mut self, sample_rate: u32, buffer_size: usize) -> &mut [Sample] {
        const FREQUENCY: f32 = 440.0;
        let sample_rate = sample_rate as f32;
        let phase_length = sample_rate / FREQUENCY;

        let volume_buffer = self.gain.get(buffer_size);

        for (frame, &mut volume) in self.buffer[..buffer_size * CHANNELS]
            .chunks_mut(CHANNELS)
            .zip(volume_buffer)
        {
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
