use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::non_copy_array;

use super::super::{Sample, CHANNELS};
use super::utils::{rms, AtomicF32, MovingAverage};

/// Creates a corresponding pair of [`AudioMeterInterface`] and [`AudioMeter`].
/// [`AudioMeter`] should live on the audio thread, while [`AudioMeterInterface`] can live wherever else.
pub fn new_audio_meter() -> (AudioMeterInterface, AudioMeter) {
    let peak1 = Arc::new(non_copy_array![AtomicF32::new(0.0); CHANNELS]);
    let peak2 = Arc::clone(&peak1);

    let long_peak1 = Arc::new(non_copy_array![AtomicF32::new(0.0); CHANNELS]);
    let long_peak2 = Arc::clone(&long_peak1);

    let rms1 = Arc::new(non_copy_array![AtomicF32::new(0.0); CHANNELS]);
    let rms2 = Arc::clone(&rms1);

    (
        AudioMeterInterface {
            peak: peak1,
            long_peak: long_peak1,
            rms: rms1,

            smoothing: non_copy_array![non_copy_array![MovingAverage::new(0.0, 7); CHANNELS]; 3],
        },
        AudioMeter {
            peak: peak2,

            long_peak: long_peak2,
            since_last_peak: [0.0; CHANNELS],

            rms: rms2,
        },
    )
}

/// Acquired via the [`new_audio_meter`] function.
#[derive(Debug)]
pub struct AudioMeter {
    peak: Arc<[AtomicF32; CHANNELS]>,

    long_peak: Arc<[AtomicF32; CHANNELS]>,
    since_last_peak: [f32; CHANNELS],

    rms: Arc<[AtomicF32; CHANNELS]>,
}
impl AudioMeter {
    pub fn report(&mut self, buffer: &[Sample], sample_rate: f32) {
        self.peak(buffer);
        self.long_peak(buffer, sample_rate);
        self.rms(buffer);
    }

    /// Locates the peak of the buffer and syncs it to the corresponding [`AudioMeterInterface`].
    fn peak(&mut self, buffer: &[Sample]) {
        let mut max_values = [0.0, 0.0];
        for frame in buffer.chunks(2) {
            for (max, &value) in max_values.iter_mut().zip(frame) {
                if value.abs() > *max {
                    *max = value.abs();
                }
            }
        }
        for (peak, max) in self.peak.iter().zip(max_values) {
            peak.store(max, Ordering::Relaxed);
        }
    }

    /// Holds the peak for 1 second, before letting it fall.
    fn long_peak(&mut self, buffer: &[Sample], sample_rate: f32) {
        for ((a_long_peak, a_peak), since_last_peak) in self
            .long_peak
            .iter()
            .zip(self.peak.iter())
            .zip(&mut self.since_last_peak)
        {
            let peak = a_peak.load(Ordering::Relaxed);
            let long_peak = a_long_peak.load(Ordering::Relaxed);
            if peak >= long_peak {
                a_long_peak.store(peak, Ordering::Relaxed);
                *since_last_peak = 0.0;
            } else {
                let elapsed = (buffer.len() / CHANNELS) as f32 / sample_rate;
                *since_last_peak += elapsed;
                if *since_last_peak > 1.0 {
                    let mut new_long_peak = long_peak - elapsed;
                    if new_long_peak < 0.0 {
                        new_long_peak = 0.0;
                    }
                    a_long_peak.store(new_long_peak, Ordering::Relaxed);
                }
            }
        }
    }

    /// Calculates the root-mean-square of the buffer, and syncs it to the corresponding [`AudioMeterInterface`].
    fn rms(&mut self, buffer: &[Sample]) {
        let rms_values = rms(buffer);

        // Output to atomics
        for (rms_atomic, rms_value) in self.rms.iter().zip(rms_values) {
            rms_atomic.store(rms_value, Ordering::Relaxed);
        }
    }
}

/// Acquired via the [`new_audio_meter`] function.
#[derive(Debug)]
pub struct AudioMeterInterface {
    peak: Arc<[AtomicF32; CHANNELS]>,
    long_peak: Arc<[AtomicF32; CHANNELS]>,
    rms: Arc<[AtomicF32; CHANNELS]>,

    smoothing: [[MovingAverage; CHANNELS]; 3],
}
impl AudioMeterInterface {
    /// Return an array of the signals current peak, long-term peak and RMS-level for each channel in the form:
    /// - `[peak: [left, right], long_peak: [left, right], rms: [left, right]]`
    ///
    /// Results are smoothed to avoid jittering, suitable for reading every frame.
    /// If this is not desirable see [`AudioMeterInterface::read_raw`].
    pub fn read(&mut self) -> [[Sample; CHANNELS]; 3] {
        let mut result = self.read_raw();

        // Smooth
        for (stats, avgs) in result.iter_mut().zip(&mut self.smoothing) {
            for (stat, avg) in &mut stats.iter_mut().zip(avgs.iter_mut()) {
                avg.push(*stat);
                *stat = avg.average();
            }
        }

        result
    }

    /// Same as [`AudioMeterInterface::read`], except results are not smoothed.
    pub fn read_raw(&self) -> [[Sample; CHANNELS]; 3] {
        let mut result = [[0.0; CHANNELS]; 3];
        for (result_frame, atomic_frame) in
            result
                .iter_mut()
                .zip([&self.peak, &self.long_peak, &self.rms])
        {
            for (result, atomic) in result_frame.iter_mut().zip(atomic_frame.iter()) {
                *result = atomic.load(Ordering::Relaxed);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak() {
        let (am_interface, mut am) = new_audio_meter();
        let input = [0.0, 3.5, -6.4, 0.2, -0.3, 0.4];

        am.report(&input, 1.0);

        let [peak, _, _] = am_interface.read_raw();
        assert_eq!(peak, [6.4, 3.5]);
    }

    #[test]
    fn long_peak_reflects_peak() {
        let (am_interface, mut am) = new_audio_meter();
        let input = [0.0, 3.5, -6.4, 0.2, -0.3, 0.4];

        am.report(&input, 3.0);

        let [_, long_peak, _] = am_interface.read_raw();
        assert_eq!(long_peak, [6.4, 3.5]);
    }

    #[test]
    fn long_peak_stays_for_a_bit() {
        let (am_interface, mut am) = new_audio_meter();
        let input = [3.5, -1.2, 0.0, 0.0, 0.0, 0.0];

        // Since the long_peak should stay in place for 1 second
        am.report(&input, 3.0);

        let [_, long_peak, _] = am_interface.read_raw();
        assert_eq!(long_peak, [3.5, 1.2]);
    }

    #[test]
    fn long_peak_falls() {
        let (am_interface, mut am) = new_audio_meter();
        let input = [-3.5, 1.2, 0.0, 0.0, 0.0, 0.0];

        am.report(&input, 4.0);

        let [_, long_peak, _] = am_interface.read_raw();
        assert_eq!(long_peak, [3.5, 1.2]);
    }
}
