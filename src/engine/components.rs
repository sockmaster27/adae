use core::sync::atomic::Ordering;
use std::{
    sync::atomic::AtomicBool,
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

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
/// The value is smoothed (via simple moving average), to avoid distortion and clicking.
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

/// Component for the simple mixing together (addition) of two signals.
///
/// Mixing is done via 64-bit summing.
pub struct MixPoint {
    buffer: Vec<Sample>,
}
impl MixPoint {
    pub fn new(max_buffer_size: usize) -> Self {
        Self {
            buffer: vec![0.0; max_buffer_size * CHANNELS],
        }
    }

    /// Mix the two buffers into a new one via 64-bit summing.
    ///
    /// Result is not clipped.
    pub fn mix(&mut self, buffer1: &[Sample], buffer2: &[Sample]) -> &mut [Sample] {
        debug_assert_eq!(buffer1.len(), buffer2.len());

        for ((own_sample, sample1), sample2) in self.buffer.iter_mut().zip(buffer1).zip(buffer2) {
            *own_sample = (*sample1 as f64 + *sample2 as f64) as Sample;
        }

        &mut self.buffer[..buffer1.len()]
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

/// Creates a corresponding pair of `PeakMeterReporter` and `PeakMeterOutput`.
/// `PeakMeterReporter` should live on the audio thread, while `PeakMeterOutput` can live wherever else.
pub fn new_peak_meter(
    interval: f32,
    sample_rate: u32,
    max_buffer_size: usize,
) -> (PeakMeterOutput, PeakMeterReporter) {
    let (sender, receiver) = ringbuf::RingBuffer::new(max_buffer_size).split();
    (
        PeakMeterOutput::new(
            receiver,
            interval,
            max_buffer_size,
            Box::new(|peak| println!("Peak: {}", peak)),
        ),
        PeakMeterReporter {
            sender,
            sample_interval: (interval * sample_rate as f32) as usize,

            previous_max: 0.0,
            remainding: 0,
        },
    )
}

/// Acquired via the `new_peak_meter` function.
pub struct PeakMeterReporter {
    sender: ringbuf::Producer<Sample>,
    sample_interval: usize,

    // Carries over when buffer size isn't divisible by interval.
    previous_max: Sample,
    remainding: usize,
}
impl PeakMeterReporter {
    pub fn report(&mut self, buffer: &[Sample]) {
        // Handle previous remainder
        if self.remainding > buffer.len() {
            self.remainding -= buffer.len();
            self.previous_max = Self::max(self.previous_max, buffer);
        } else {
            let peak = Self::max(self.previous_max, &buffer[..self.remainding]);
            self.sender.push(peak).unwrap();

            // Handle main part
            let main_part = buffer[self.remainding..].chunks_exact(self.sample_interval);
            let remainder = main_part.remainder();
            for chunk in main_part {
                let peak = Self::max(0.0, chunk);
                self.sender.push(peak).unwrap();
            }

            // Handle next remainder
            self.previous_max = Self::max(0.0, remainder);
            self.remainding = self.sample_interval - remainder.len();
        }
    }

    fn max(start: Sample, buffer: &[Sample]) -> Sample {
        let mut max = start;
        for &value in buffer.iter() {
            if value > max {
                max = value;
            }
        }
        max
    }
}

/// Acquired via the `new_peak_meter` function.
pub struct PeakMeterOutput {
    stopped: Arc<AtomicBool>,
    join_handle: Option<JoinHandle<()>>,
}
impl PeakMeterOutput {
    fn new(
        receiver: ringbuf::Consumer<f32>,
        interval: f32,
        max_buffer_size: usize,
        callback: Box<dyn Fn(Sample) + Send>,
    ) -> Self {
        let stopped = Arc::new(AtomicBool::new(false));
        let stopped2 = Arc::clone(&stopped);

        let join_handle = thread::spawn(move || {
            Self::run(
                stopped2,
                interval,
                receiver,
                CircularArray::new(0.0, max_buffer_size),
                callback,
            )
        });

        let join_handle = Some(join_handle);

        Self {
            stopped,
            join_handle,
        }
    }

    fn run(
        stopped: Arc<AtomicBool>,
        interval: f32,
        mut receiver: ringbuf::Consumer<f32>,
        mut delay: CircularArray<Sample>,
        callback: Box<dyn Fn(Sample) + Send>,
    ) {
        while !stopped.load(Ordering::Relaxed) {
            spin_sleep::sleep(Duration::from_secs_f32(interval));
            while receiver.is_empty() {}
            let new_peak = receiver.pop().unwrap();
            let peak = delay.push_pop(new_peak);
            (callback)(peak);
        }
    }

    fn stop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.join_handle.take().unwrap().join().unwrap();
    }
}
impl Drop for PeakMeterOutput {
    fn drop(&mut self) {
        self.stop();
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
