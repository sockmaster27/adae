use std::iter::zip;

use crate::engine::{Sample, CHANNELS};

/// Component for the simple addition of signals.
///
/// Mixing is done via 64-bit summing.
pub struct MixPoint {
    sum_buffer: Vec<f64>,
    output_buffer: Vec<Sample>,
    buffer_size_samples: Option<usize>,
}
impl MixPoint {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            sum_buffer: vec![0.0; max_buffer_size * CHANNELS],
            output_buffer: vec![0.0; max_buffer_size * CHANNELS],

            /// Measures samples in buffer instead of frames
            ///
            /// `buffer_size_samples == buffer_size * CHANNELS`
            buffer_size_samples: None,
        }
    }

    /// Resets sum and buffer size.
    pub fn reset(&mut self) {
        let dirty = self.buffer_size_samples.unwrap_or(0);
        self.sum_buffer[..dirty].fill(0.0);
        self.buffer_size_samples = None;
    }

    /// Add buffer to the 64-bit sum.
    /// With debug assertions enabled, this will panic if buffers of different sizes are added inbetween resets.
    pub fn add(&mut self, input_buffer: &[Sample]) {
        if self.buffer_size_samples.is_none() {
            self.buffer_size_samples = Some(input_buffer.len());
        } else {
            // Assert that all buffers added between resets are of equal size.
            #[cfg(debug_assertions)]
            {
                let buffer_size_samples = self.buffer_size_samples.unwrap();
                if buffer_size_samples != input_buffer.len() {
                    panic!(
                        "At least two buffers were of different sizes: {}, {}.",
                        buffer_size_samples,
                        input_buffer.len()
                    );
                }
            }
        }

        // Sum
        for (sum_sample, &input_sample) in zip(self.sum_buffer.iter_mut(), input_buffer) {
            *sum_sample += f64::from(input_sample);
        }
    }

    /// Get sum of all buffers added since last reset.
    /// Returns the full output buffer, so remember to only use the first `buffer_size * CHANNELS` samples.
    ///
    /// Result is not clipped.
    pub fn get(&mut self) -> &mut [Sample] {
        // Convert back to original sample format.
        for (output_sample, &sum_sample) in
            zip(self.output_buffer.iter_mut(), self.sum_buffer.iter())
        {
            *output_sample = sum_sample as Sample;
        }

        &mut self.output_buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_of_two_signals() {
        let mut mp = MixPoint::new(2);

        mp.add(&[3.0, 5.0, -2.0]);
        mp.add(&[7.0, -2.0, -6.0]);
        let result = mp.get();

        assert_eq!(result, &mut [10.0, 3.0, -8.0, 0.0][..]);
    }

    // This should only happen with debug assertions enabled.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn panics_with_different_lengths() {
        let mut mp = MixPoint::new(2);

        mp.add(&[3.0, 5.0, -2.0]);
        mp.add(&[7.0, -2.0, -6.0, 8.0]);
    }

    #[test]
    fn reset_resets() {
        let mut mp = MixPoint::new(2);

        let signal2 = [7.0, -2.0, -6.0, 8.0];
        mp.add(&signal2);
        assert_eq!(mp.get(), &[7.0, -2.0, -6.0, 8.0][..]);

        mp.reset();

        let signal1 = [3.0, 5.0, -2.0];
        mp.add(&signal1);
        assert_eq!(mp.get(), &[3.0, 5.0, -2.0, 0.0][..]);
    }

    #[test]
    fn no_additions() {
        let mut mp = MixPoint::new(2);
        mp.add(&[3.0, 5.0, -2.0]);
        mp.reset();
        assert_eq!(mp.get(), &[0.0, 0.0, 0.0, 0.0][..]);
    }

    // Tests whether there are significant rounding errors while mixing a large amount of signals.
    // Fails without 64-bit summing.
    #[test]
    fn retains_precision() {
        let mut mp = MixPoint::new(5);

        let mut signals1 = [[0.0; 10]; 10000];

        // Create some sines with different frequencies
        for (i, signal) in signals1.iter_mut().enumerate() {
            for (t, sample) in signal.iter_mut().enumerate() {
                *sample = ((t * i) as Sample).sin();
            }
        }

        let mut signals2 = signals1.clone();

        // Negate all
        for signal in &mut signals2 {
            for sample in signal {
                *sample *= -1.0;
            }
        }

        for signal in signals1.iter().chain(signals2.iter()) {
            mp.add(signal);
        }

        let result = mp.get();
        assert_eq!(result, &[0.0; 10][..]);
    }
}
