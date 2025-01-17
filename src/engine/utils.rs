pub mod dropper;
pub mod key_generator;
pub mod rbtree_node;
pub mod remote_push;
pub mod ringbuffer;

use std::any::Any;
use std::fmt::Debug;
use std::iter::zip;
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(test)]
use std::path::PathBuf;

use super::{Sample, CHANNELS};

/// Macro for conveniently initializing a static array of a given size, of a type that is not [`Copy`].
///
/// The `initial` expression is evaluated for each element in the array.
macro_rules! non_copy_array {
    ($initial:expr; $size:expr) => {
        [(); $size].map(|_| $initial)
    };
}
pub(crate) use non_copy_array;

/// Find smallest power of 2 that is greater than or equal to `x`
pub fn smallest_pow2(x: f64) -> usize {
    2usize.pow(x.log2().ceil() as u32)
}

/// Get the smallest and largest value from an iterator in the form `(min, max)`.
/// If the iterator is empty, the default value is returned for both.
pub fn min_max<I, T>(i: I, default: T) -> (T, T)
where
    I: IntoIterator<Item = T>,
    T: PartialOrd + Copy,
{
    let iter = i.into_iter();
    iter.fold((default, default), |(min, max), x| {
        (partial_min(min, x), partial_max(max, x))
    })
}

pub fn partial_min<T>(a: T, b: T) -> T
where
    T: PartialOrd,
{
    if a < b {
        a
    } else {
        b
    }
}

pub fn partial_max<T>(a: T, b: T) -> T
where
    T: PartialOrd,
{
    if a > b {
        a
    } else {
        b
    }
}

/// Get PathBuf to any file located in the `test_files` directory.
/// Should only be used for testing.
///
/// For example:
///
/// `test_file_path("44100 16-bit.wav")`
#[cfg(test)]
pub fn test_file_path(file_name: &str) -> PathBuf {
    PathBuf::from(format!(
        "{}{}",
        concat!(env!("CARGO_MANIFEST_DIR"), "/test_files/"),
        file_name,
    ))
}

/// Atomic supporting storing and loading of an f32, via the raw bits of a u32.
pub struct AtomicF32 {
    inner: AtomicU32,
}
impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self {
            inner: AtomicU32::new(v.to_bits()),
        }
    }

    pub fn store(&self, val: f32, order: Ordering) {
        self.inner.store(val.to_bits(), order);
    }

    pub fn load(&self, order: Ordering) -> f32 {
        f32::from_bits(self.inner.load(order))
    }
}
impl Debug for AtomicF32 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self.load(Ordering::SeqCst), f)
    }
}

/// Root Mean Square of a single buffer.
pub fn rms(buffer: &[Sample]) -> [f32; CHANNELS] {
    let buffer_size = (buffer.len() / CHANNELS) as f64;
    let mut averages = [0.0; CHANNELS];

    for frame in buffer.chunks(CHANNELS) {
        for (sample, average) in zip(frame, &mut averages) {
            *average += f64::from(sample.powi(2)) / buffer_size;
        }
    }

    averages.map(|x| (x as f32).sqrt())
}

/// Calculates simple moving average with an internal history buffer.
#[derive(Debug)]
pub struct MovingAverage {
    average: f64,
    history: CircularArray<f32>,
}
impl MovingAverage {
    pub fn new(initial: f32, window_size: usize) -> Self {
        Self {
            average: initial.into(),
            history: CircularArray::new(initial, window_size),
        }
    }

    pub fn push(&mut self, new_value: f32) {
        let removed_value = self.history.push_pop(new_value);

        // Storing the average as an f64 ensures far greater accuracy.
        let window_size = self.history.len() as f64;
        let delta = f64::from(new_value - removed_value) / window_size;
        self.average += delta;
    }

    /// Swap out entire contents with `value`
    pub fn fill(&mut self, value: f32) {
        self.history.fill(value);
        self.average = value.into();
    }

    pub fn average(&self) -> f32 {
        self.average as f32
    }
}

/// A ringbuffer-like queue, where the length is always the same, i.e. it only has one pointer.
// Please correct me if this has a better name.
pub struct CircularArray<T> {
    position: usize,
    buffer: Vec<T>,
}
impl<T: Clone> CircularArray<T> {
    /// Create an array of the given size, filled with the `initial` value.
    pub fn new(initial: T, size: usize) -> Self {
        Self {
            position: 0,
            buffer: vec![initial; size],
        }
    }

    /// Inserts the value at the back of the queue, and returns the value removed from the front.
    pub fn push_pop(&mut self, value: T) -> T {
        let removed = std::mem::replace(&mut self.buffer[self.position], value);

        self.position += 1;
        self.position %= self.len();

        removed
    }

    pub fn fill(&mut self, value: T) {
        self.buffer.fill(value)
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        let first_part = &self.buffer[self.position..];
        let last_part = &self.buffer[..self.position];
        first_part.iter().chain(last_part)
    }
}
impl<T: Clone + Debug> Debug for CircularArray<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// Get the the message from a `panic` cause.
pub fn panic_msg(e: Box<dyn Any + Send>) -> String {
    let msg = if let Some(s) = e.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.to_string()
    } else {
        "Unknown error".to_string()
    };

    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    // Just a surface-level test, no concurrency or anything.
    #[test]
    fn atomic_f32() {
        let a_f32 = AtomicF32::new(0.0);

        a_f32.store(3.0, Ordering::Relaxed);

        let result = a_f32.load(Ordering::Relaxed);
        assert_eq!(result, 3.0);
    }

    #[test]
    fn root_mean_square() {
        let result = rms(&[2.0, 5.4, 3.7, -3.0, 1.0, -15.0]);

        let expected = [6.23_f32.sqrt(), 87.72_f32.sqrt()];

        assert_eq!(result, expected)
    }

    #[test]
    fn moving_average() {
        let mut ma = MovingAverage::new(1.0, 10);

        for _ in 0..5 {
            ma.push(3.0);
        }

        assert_eq!(ma.average(), 2.0);
    }

    #[test]
    fn circular_array() {
        let mut ca = CircularArray::new(1, 5);
        let mut output = [0; 6];

        for number in &mut output {
            *number = ca.push_pop(2);
        }

        // Observe that all initial values are pushed through, plus a single of the supplied ones.
        let expected_output = [1, 1, 1, 1, 1, 2];
        assert_eq!(output, expected_output);
    }
}
