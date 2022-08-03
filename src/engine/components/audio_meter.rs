use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use crate::engine::utils::MovingAverage;
use crate::{meter_scale, non_copy_array, zip};

use crate::engine::utils::{rms, AtomicF32};
use crate::engine::{Sample, CHANNELS};

pub fn new_audio_meter() -> (AudioMeter, AudioMeterProcessor) {
    let peak1 = Arc::new(non_copy_array![AtomicF32::new(0.0); CHANNELS]);
    let peak2 = Arc::clone(&peak1);

    let long_peak1 = Arc::new(non_copy_array![AtomicF32::new(0.0); CHANNELS]);
    let long_peak2 = Arc::clone(&long_peak1);

    let rms1 = Arc::new(non_copy_array![AtomicF32::new(0.0); CHANNELS]);
    let rms2 = Arc::clone(&rms1);

    (
        AudioMeter {
            peak: peak1,
            last_peak_top: [0.0; CHANNELS],
            since_peak_top: [Instant::now(); CHANNELS],

            long_peak: long_peak1,
            last_long_peak_top: [0.0; CHANNELS],
            since_long_peak_top: [Instant::now(); CHANNELS],

            rms: rms1,
            rms_avg: non_copy_array![MovingAverage::new(0.0, 20); CHANNELS],
        },
        AudioMeterProcessor {
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
    last_peak_top: [f32; CHANNELS],
    since_peak_top: [Instant; CHANNELS],

    long_peak: Arc<[AtomicF32; CHANNELS]>,
    last_long_peak_top: [f32; CHANNELS],
    since_long_peak_top: [Instant; CHANNELS],

    rms: Arc<[AtomicF32; CHANNELS]>,
    rms_avg: [MovingAverage; CHANNELS],
}
impl AudioMeter {
    /// Returns an array of the signals current peak, long-term peak and RMS-level for each channel in the form:
    /// - `[peak: [left, right], long_peak: [left, right], rms: [left, right]]`
    ///
    /// Results are scaled and smoothed to avoid jittering, suitable for reading every frame.
    /// If this is not desirable see [`AudioMeter::read_raw`].
    pub fn read(&mut self) -> [[Sample; CHANNELS]; 3] {
        let mut peak = [0.0; CHANNELS];
        let mut long_peak = [0.0; CHANNELS];
        let mut rms = [0.0; CHANNELS];

        // Iterating through all channels of the first two:
        let channel_iter = zip!(
            self.peak.iter(),
            self.last_peak_top.iter_mut(),
            self.since_peak_top.iter_mut(),
            peak.iter_mut(),
        )
        .chain(zip!(
            self.long_peak.iter(),
            self.last_long_peak_top.iter_mut(),
            self.since_long_peak_top.iter_mut(),
            long_peak.iter_mut(),
        ));

        // Falling slowly
        for (((stat, last_top), since_last_top), result) in channel_iter {
            let stat = stat.load(Ordering::Relaxed);
            let scaled = meter_scale(stat);

            const DURATION: f32 = 0.3;
            let elapsed = since_last_top.elapsed().as_secs_f32();
            let progress = elapsed / DURATION;
            let fallen = *last_top * (-progress.powf(2.0) + 1.0);

            *result = if scaled >= fallen {
                *last_top = scaled;
                *since_last_top = Instant::now();
                scaled
            } else {
                fallen
            };
        }

        // Averaged
        for ((rms, avg), result) in zip!(self.rms.iter(), self.rms_avg.iter_mut(), rms.iter_mut()) {
            let rms = rms.load(Ordering::Relaxed);
            let scaled = meter_scale(rms);
            avg.push(scaled);
            *result = avg.average();
        }

        [peak, long_peak, rms]
    }

    /// Snap smoothed rms value to its current unsmoothed equivalent.
    ///
    /// Should be called before [`Self::read`] is called the first time or after a long break,
    /// to avoid meter sliding in place from zero or a very old value.
    pub fn snap_rms(&mut self) {
        for (rms, rms_avg) in zip!(self.rms.iter(), self.rms_avg.iter_mut()) {
            let rms = rms.load(Ordering::Relaxed);
            let scaled = meter_scale(rms);

            rms_avg.fill(scaled);
        }
    }

    /// Same as [`AudioMeter::read`], except results are not smoothed or scaled.
    ///
    /// Long peak stays in place for 1 second since it was last changed, before snapping to the current peak.
    pub fn read_raw(&self) -> [[Sample; CHANNELS]; 3] {
        let mut result = [[0.0; CHANNELS]; 3];
        for (result_frame, atomic_frame) in
            zip!(result.iter_mut(), [&self.peak, &self.long_peak, &self.rms])
        {
            for (result, atomic) in zip!(result_frame.iter_mut(), atomic_frame.iter()) {
                *result = atomic.load(Ordering::Relaxed);
            }
        }

        result
    }
}

/// Acquired via the [`new_audio_meter`] function.
#[derive(Debug)]
pub struct AudioMeterProcessor {
    peak: Arc<[AtomicF32; CHANNELS]>,

    /// Long peak simply holds the highest observed peak for 1 second.
    /// This guarantees that it's observed correctly,
    /// even if peak is checked less frequently than it is updated.
    long_peak: Arc<[AtomicF32; CHANNELS]>,
    since_last_peak: [f32; CHANNELS],

    rms: Arc<[AtomicF32; CHANNELS]>,
}
impl AudioMeterProcessor {
    pub fn report(&mut self, buffer: &[Sample], sample_rate: f32) {
        self.peak(buffer);
        self.long_peak(buffer.len() as f32, sample_rate);
        self.rms(buffer);
    }

    /// Locates the peak of the buffer and syncs it to the corresponding [`AudioMeter`].
    fn peak(&mut self, buffer: &[Sample]) {
        let mut max_values = [0.0, 0.0];
        for frame in buffer.chunks(2) {
            for (max, &value) in zip!(max_values.iter_mut(), frame) {
                if value.abs() > *max {
                    *max = value.abs();
                }
            }
        }

        for (peak, max) in zip!(self.peak.iter(), max_values) {
            peak.store(max, Ordering::Relaxed);
        }
    }

    /// Holds the peak for 1 second, before letting it fall.
    fn long_peak(&mut self, buffer_size: f32, sample_rate: f32) {
        // How long the peak is held in seconds
        const HOLD: f32 = 1.0;

        for ((a_long_peak, a_peak), since_last_peak) in zip!(
            self.long_peak.iter(),
            self.peak.iter(),
            &mut self.since_last_peak
        ) {
            let peak = a_peak.load(Ordering::Relaxed);
            let long_peak = a_long_peak.load(Ordering::Relaxed);

            *since_last_peak += buffer_size / sample_rate;

            if peak >= long_peak || *since_last_peak > HOLD {
                *since_last_peak = 0.0;
                a_long_peak.store(peak, Ordering::Relaxed);
            } else {
            }
        }
    }

    /// Calculates the root-mean-square of the buffer, and syncs it to the corresponding [`AudioMeter`].
    fn rms(&mut self, buffer: &[Sample]) {
        let rms_values = rms(buffer);

        // Output to atomics
        for (rms_atomic, rms_value) in zip!(self.rms.iter(), rms_values) {
            rms_atomic.store(rms_value, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak() {
        let sample_rate = 1.0;
        let (am, mut amp) = new_audio_meter();
        let input = [0.0, 3.5, -6.4, 0.2, -0.3, 0.4];

        amp.report(&input, sample_rate);

        let [peak, _, _] = am.read_raw();
        assert_eq!(peak, [6.4, 3.5]);
    }

    #[test]
    fn long_peak_reflects_peak() {
        let sample_rate = 3.0;
        let (am, mut amp) = new_audio_meter();
        let input = [0.0, 3.5, -6.4, 0.2, -0.3, 0.4];

        amp.report(&input, sample_rate);

        let [_, long_peak, _] = am.read_raw();
        assert_eq!(long_peak, [6.4, 3.5]);
    }

    #[test]
    fn long_peak_stays_for_a_bit() {
        let sample_rate = 4.0;
        let (am, mut amp) = new_audio_meter();
        let input1 = [-3.5, 1.2, 0.4, -1.1];
        let input2 = [0.0, 0.0, 0.0, 0.0];

        // Since the long_peak should stay in place for 1 second
        amp.report(&input1, sample_rate);
        amp.report(&input2, sample_rate);

        let [_, long_peak, _] = am.read_raw();
        assert_eq!(long_peak, [3.5, 1.2]);
    }

    #[test]
    fn long_peak_falls() {
        let sample_rate = 3.0;
        let (am, mut amp) = new_audio_meter();
        let input1 = [-3.5, 1.2, 0.4, -1.1];
        let input2 = [0.0, 0.0, 0.0, 0.0];

        amp.report(&input1, sample_rate);
        amp.report(&input2, sample_rate);

        let [_, long_peak, _] = am.read_raw();
        assert_eq!(long_peak, [0.0, 0.0]);
    }
}
