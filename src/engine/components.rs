use core::sync::atomic::Ordering;
use std::{sync::atomic::AtomicU32, sync::Arc};

use super::{Sample, CHANNELS};

/// Generates a 440 Hz sine wave.
pub struct TestToneGenerator {
    pub gain: ValueParameter,

    buffer: Vec<Sample>,
    sample_clock: f32,
}
impl TestToneGenerator {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            gain: ValueParameter::new(1.0, max_buffer_size),

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

/// Representes a numeric value, controlled by the user - by a knob or slider for example.
///
/// The value is smoothed (via simple moving average), to avoid distortion and clicking in the sound.
pub struct ValueParameter {
    buffer: Vec<f32>,

    desired: f32,
    moving_average: MovingAverage,
}
impl ValueParameter {
    pub fn new(initial: f32, max_buffer_size: usize) -> Self {
        Self {
            buffer: vec![0.0; max_buffer_size],

            desired: initial,
            moving_average: MovingAverage::new(initial, max_buffer_size),
        }
    }

    pub fn set(&mut self, new_value: f32) {
        self.desired = new_value;
    }

    pub fn get(&mut self, buffer_size: usize) -> &mut [f32] {
        for point in self.buffer[..buffer_size].iter_mut() {
            self.moving_average.push(self.desired);
            *point = self.moving_average.get();
        }

        &mut self.buffer[..buffer_size]
    }
}

/// Calculates simple moving average with an internal history buffer.
struct MovingAverage {
    average: f32,
    history: CircularArray<f32>,
}
impl MovingAverage {
    fn new(initial: f32, window_size: usize) -> Self {
        Self {
            average: initial,
            history: CircularArray::new(initial, window_size),
        }
    }

    fn push(&mut self, new_value: f32) {
        let removed_value = self.history.push_pop(new_value);

        let window_size = self.history.len() as f32;
        self.average -= removed_value / window_size;
        self.average += new_value / window_size;
    }

    fn get(&self) -> f32 {
        self.average
    }
}

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

/// Circular sample delay.
pub struct DelayPoint {
    history: CircularArray<Sample>,
}
impl DelayPoint {
    pub fn new(sample_delay: usize) -> Self {
        Self {
            history: CircularArray::new(0.0, sample_delay * CHANNELS),
        }
    }

    pub fn next(&mut self, buffer: &mut [Sample]) {
        for sample in buffer {
            *sample = self.history.push_pop(*sample);
        }
    }
}

/// Creates a corresponding pair of `PeakMeterInterface` and `PeakMeter`.
/// `PeakMeter` should live on the audio thread, while `PeakMeterInterface` can live wherever else.
pub fn new_peak_meter() -> (PeakMeterInterface, PeakMeter) {
    let synced_peak = Arc::new(AtomicF32::new(0.0));
    let synced_peak2 = Arc::clone(&synced_peak);
    (
        PeakMeterInterface { peak: synced_peak },
        PeakMeter { peak: synced_peak2 },
    )
}

/// Acquired via the `new_peak_meter` function.
pub struct PeakMeter {
    peak: Arc<AtomicF32>,
}
impl PeakMeter {
    /// Locates the peak of the buffer and syncs it to the corresponding `PeakMeterInterface`.
    pub fn report(&mut self, buffer: &[Sample]) {
        let mut max = 0.0;
        for &value in buffer.iter() {
            if value.abs() > max {
                max = value;
            }
        }
        self.peak.store(max, Ordering::Release);
    }
}

/// Acquired via the `new_peak_meter` function.
pub struct PeakMeterInterface {
    peak: Arc<AtomicF32>,
}
impl PeakMeterInterface {
    pub fn get(&self) -> Sample {
        self.peak.load(Ordering::Acquire)
    }
}

/// Atomic storing and loading of an f32, via the raw bits of a u32.
struct AtomicF32 {
    inner: AtomicU32,
}
impl AtomicF32 {
    fn new(v: f32) -> Self {
        Self {
            inner: AtomicU32::new(v.to_bits()),
        }
    }

    fn store(&self, val: f32, order: Ordering) {
        self.inner.store(val.to_bits(), order);
    }

    fn load(&self, order: Ordering) -> f32 {
        f32::from_bits(self.inner.load(order))
    }
}

/// A ringbuffer-like queue, where the length is always the same, i.e. it only has one pointer.
// Please correct me if this structure has a better name.
struct CircularArray<T> {
    queue: Vec<T>,
    position: usize,
}
impl<T: Copy> CircularArray<T> {
    /// Create an array of the given size, filled with the `initial` value.
    fn new(initial: T, size: usize) -> Self {
        Self {
            queue: vec![initial; size],
            position: 0,
        }
    }

    /// Inserts the value at the back of the queue, and returns the value removed from the front.
    fn push_pop(&mut self, value: T) -> T {
        let removed = self.queue[self.position];
        self.queue[self.position] = value;

        self.position += 1;
        self.position %= self.len();

        removed
    }

    fn len(&self) -> usize {
        self.queue.len()
    }
}
