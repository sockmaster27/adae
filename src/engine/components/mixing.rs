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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_of_two_signals() {
        let mut mp = MixPoint::new(10);

        let signal1 = [3.0, 5.0, -2.0];
        let signal2 = [7.0, -2.0, -6.0];
        let result = mp.mix(&[&signal1, &signal2]);

        assert_eq!(result, [10.0, 3.0, -8.0]);
    }

    // This should only happen with debug assertions enabled.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn panics_with_different_lengths() {
        let mut mp = MixPoint::new(10);

        let signal1 = [3.0, 5.0, -2.0];
        let signal2 = [7.0, -2.0, -6.0, 8.0];
        mp.mix(&[&signal1, &signal2]);
    }

    // Tests whether there are significant rounding errors while mixing a large amount of signals.
    #[test]
    fn retains_precision() {
        let mut mp = MixPoint::new(10);

        let mut signals1 = [[0.0; 10]; 10000];

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

        let mut all_signals = vec![];
        for signal in signals1.iter().chain(signals2.iter()) {
            all_signals.push(&signal[..]);
        }

        let result = mp.mix(&all_signals[..]);
        assert_eq!(result, [0.0; 10])
    }
}
