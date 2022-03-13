use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::super::{Sample, CHANNELS};
use super::utils::{AtomicF32, RMS};

/// Creates a corresponding pair of [`AudioMeterInterface`] and [`AudioMeter`].
/// [`AudioMeter`] should live on the audio thread, while [`AudioMeterInterface`] can live wherever else.
pub fn new_audio_meter() -> (AudioMeterInterface, AudioMeter) {
    let peak1 = Arc::new([AtomicF32::new(0.0), AtomicF32::new(0.0)]);
    let peak2 = Arc::clone(&peak1);

    let long_peak1 = Arc::new([AtomicF32::new(0.0), AtomicF32::new(0.0)]);
    let long_peak2 = Arc::clone(&long_peak1);

    let rms1 = Arc::new([AtomicF32::new(0.0), AtomicF32::new(0.0)]);
    let rms2 = Arc::clone(&rms1);

    (
        AudioMeterInterface {
            peak: peak1,
            long_peak: long_peak1,
            rms: rms1,
        },
        AudioMeter {
            peak: peak2,

            long_peak: long_peak2,
            since_last_peak: [0.0; CHANNELS],

            rms: rms2,
            rms_history: [RMS::new(4800), RMS::new(48000)],
        },
    )
}

/// Acquired via the [`new_audio_meter`] function.
pub struct AudioMeter {
    peak: Arc<[AtomicF32; CHANNELS]>,

    long_peak: Arc<[AtomicF32; CHANNELS]>,
    since_last_peak: [f32; CHANNELS],

    rms: Arc<[AtomicF32; CHANNELS]>,
    rms_history: [RMS; CHANNELS],
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
                    *max = value;
                }
            }
        }
        for (peak, max) in self.peak.iter().zip(max_values) {
            peak.store(max, Ordering::Relaxed);
        }
    }

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
                let elapsed = sample_rate / (buffer.len() / CHANNELS) as f32;
                *since_last_peak += elapsed;
                if *since_last_peak > 5.0 {
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
        for frame in buffer.chunks(2) {
            for (&sample, rms_history) in frame.iter().zip(&mut self.rms_history) {
                rms_history.push(sample);
            }
        }

        // Output to atomics
        for (rms, rms_history) in self.rms.iter().zip(&self.rms_history) {
            let rms_value = rms_history.get_rms();
            rms.store(rms_value, Ordering::Relaxed);
        }
    }
}

/// Acquired via the [`new_audio_meter`] function.
pub struct AudioMeterInterface {
    peak: Arc<[AtomicF32; CHANNELS]>,
    long_peak: Arc<[AtomicF32; CHANNELS]>,
    rms: Arc<[AtomicF32; CHANNELS]>,
}
impl AudioMeterInterface {
    /// Return an array of the signals current peak, long-term peak and RMS-level for each channel in the form:
    /// - `[peak: [left, right], long_peak: [left, right], rms: [left, right]]`
    pub fn read(&self) -> [[Sample; CHANNELS]; 3] {
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
