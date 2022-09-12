use crate::zip;

use crate::engine::{Sample, CHANNELS};

/// Component for the simple addition of signals.
///
/// Mixing is done via 64-bit summing.
pub struct MixPoint {
    sum_buffer: Vec<f64>,
    output_buffer: Vec<Sample>,
    buffer_size: Option<usize>,
}
impl MixPoint {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            sum_buffer: vec![0.0; max_buffer_size * CHANNELS],
            output_buffer: vec![0.0; max_buffer_size * CHANNELS],
            buffer_size: None,
        }
    }

    /// Resets sum and buffer size.
    pub fn reset(&mut self) {
        let dirty = self.buffer_size.unwrap_or(0);
        for sample in &mut self.sum_buffer[..dirty] {
            *sample = 0.0;
        }
        self.buffer_size = None;
    }

    /// Add buffer to the 64-bit sum.
    /// With debug assertions enabled, this will panic if buffers of different sizes are added inbetween resets.
    pub fn add(&mut self, input_buffer: &[Sample]) {
        if self.buffer_size.is_none() {
            self.buffer_size = Some(input_buffer.len());
        } else {
            // Assert that all buffers added between resets are of equal size.
            #[cfg(debug_assertions)]
            {
                let buffer_size = self.buffer_size.unwrap();
                if buffer_size != input_buffer.len() {
                    panic!(
                        "At least two buffers were of different sizes: {}, {}.",
                        buffer_size,
                        input_buffer.len()
                    );
                }
            }
        }

        // Sum
        for (sum_sample, &input_sample) in zip!(self.sum_buffer.iter_mut(), input_buffer) {
            *sum_sample += f64::from(input_sample);
        }
    }

    /// Get sum of all buffers added since last reset.
    /// If no buffers have been added, this will return an `Error`, containing the full length zeroed buffer, as a suited buffer size isn't known.
    ///
    /// Result is not clipped.
    pub fn get(&mut self) -> Result<&mut [Sample], &mut [Sample]> {
        if let Some(buffer_size) = self.buffer_size {
            // Convert back to original sample format.
            for (output_sample, &sum_sample) in zip!(
                self.output_buffer[..buffer_size].iter_mut(),
                self.sum_buffer.iter()
            ) {
                *output_sample = sum_sample as Sample;
            }

            Ok(&mut self.output_buffer[..buffer_size])
        } else {
            // Buffer size is unknown.
            Err(&mut self.output_buffer)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_of_two_signals() {
        let mut mp = MixPoint::new(10);

        mp.add(&[3.0, 5.0, -2.0]);
        mp.add(&[7.0, -2.0, -6.0]);
        let result = mp.get();

        assert_eq!(result, Ok(&mut [10.0, 3.0, -8.0][..]));
    }

    // This should only happen with debug assertions enabled.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic]
    fn panics_with_different_lengths() {
        let mut mp = MixPoint::new(10);

        mp.add(&[3.0, 5.0, -2.0]);
        mp.add(&[7.0, -2.0, -6.0, 8.0]);
    }

    #[test]
    fn reset_resets() {
        let mut mp = MixPoint::new(10);

        let mut signal1 = [3.0, 5.0, -2.0];
        mp.add(&signal1);
        assert_eq!(mp.get(), Ok(&mut signal1[..]));

        mp.reset();

        let mut signal2 = [7.0, -2.0, -6.0, 8.0];
        mp.add(&signal2);
        assert_eq!(mp.get(), Ok(&mut signal2[..]));
    }

    #[test]
    fn no_additions() {
        let mut mp = MixPoint::new(2);
        mp.add(&[3.0, 5.0, -2.0]);
        mp.reset();
        assert_eq!(mp.get(), Err(&mut [0.0, 0.0, 0.0, 0.0][..]));
    }

    // Tests whether there are significant rounding errors while mixing a large amount of signals.
    // Fails without 64-bit summing.
    #[test]
    fn retains_precision() {
        let mut mp = MixPoint::new(10);

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
        assert_eq!(result, Ok(&mut [0.0; 10][..]));
    }
}
