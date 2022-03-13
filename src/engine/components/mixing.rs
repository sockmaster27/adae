use super::super::{Sample, CHANNELS};

/// Component for the simple addition of signals.
///
/// Mixing is done via 64-bit summing.
pub struct MixPoint {
    sum_buffer: Vec<f64>,
    output_buffer: Vec<Sample>,
}
impl MixPoint {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            sum_buffer: vec![0.0; max_buffer_size * CHANNELS],
            output_buffer: vec![0.0; max_buffer_size * CHANNELS],
        }
    }

    /// Mix the buffers into a new one via 64-bit summing.
    /// If all buffers are not of equal size, the function will panic in debug mode.
    ///
    /// Result is not clipped.
    pub fn mix(&mut self, input_buffers: &[&[Sample]]) -> &mut [Sample] {
        let buffer_size = input_buffers[0].len();

        // Set sum buffer equal to the first input buffer.
        for (sum_sample, &input_sample) in self.sum_buffer.iter_mut().zip(input_buffers[0]) {
            *sum_sample = input_sample as f64;
        }

        for &input_buffer in input_buffers[1..].iter() {
            // Assert that all buffers are of equal size.
            #[cfg(debug_assertions)]
            if buffer_size != input_buffer.len() {
                panic!(
                    "At least two buffers were of different sizes: {}, {}.",
                    buffer_size,
                    input_buffer.len()
                );
            }

            // Sum
            for (sum_sample, &input_sample) in self.sum_buffer.iter_mut().zip(input_buffer) {
                *sum_sample += input_sample as f64;
            }
        }

        // Convert back to original sample format.
        for (output_sample, &sum_sample) in self.output_buffer[..buffer_size]
            .iter_mut()
            .zip(self.sum_buffer.iter())
        {
            *output_sample = sum_sample as Sample;
        }

        &mut self.output_buffer[..buffer_size]
    }
}
