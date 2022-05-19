use std::sync::atomic::{AtomicU32, Ordering};

use super::{Sample, CHANNELS};

/// Macro for conveniently initializing a static array of a given size, of a type that is not [`Copy`].
#[macro_export(crate)]
macro_rules! non_copy_array {
    ($initial:expr; $size:expr) => {
        [(); $size].map(|_| $initial)
    };
}

/// Macro for more ergonomically zipping together multiple `IntoIterator`s.
///
/// Tuple isn't flattened though, so it's like:
///
/// `zip!(as, bs, cs, ds) -> (((a, b), c), d)`
#[macro_export(crate)]
macro_rules! zip {
    ($first:expr, $($rest:expr),+) => { {
        ($first.into_iter())$(.zip($rest.into_iter()))+
    }
    };
}

/// Atomic supporting storing and loading of an f32, via the raw bits of a u32.
#[derive(Debug)]
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

/// Root Mean Square of a single buffer.
pub fn rms(buffer: &[Sample]) -> [f32; CHANNELS] {
    let buffer_size = (buffer.len() / CHANNELS) as f64;
    let mut averages = [0.0; CHANNELS];

    for frame in buffer.chunks(CHANNELS) {
        for (sample, average) in zip!(frame, &mut averages) {
            *average += sample.powf(2.0) as f64 / buffer_size;
        }
    }

    let result = averages.map(|x| (x as f32).sqrt());
    result
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
            average: initial as f64,
            history: CircularArray::new(initial, window_size),
        }
    }

    pub fn push(&mut self, new_value: f32) {
        let removed_value = self.history.push_pop(new_value);

        // Storing the average as an f64 ensures far greater accuracy.
        let window_size = self.history.len() as f64;
        self.average -= removed_value as f64 / window_size;
        self.average += new_value as f64 / window_size;
    }

    pub fn average(&self) -> f32 {
        self.average as f32
    }
}

/// A ringbuffer-like queue, where the length is always the same, i.e. it only has one pointer.
// Please correct me if this has a better name.
#[derive(Debug)]
pub struct CircularArray<T> {
    queue: Vec<T>,
    position: usize,
}
impl<T: Copy> CircularArray<T> {
    /// Create an array of the given size, filled with the `initial` value.
    pub fn new(initial: T, size: usize) -> Self {
        Self {
            queue: vec![initial; size],
            position: 0,
        }
    }

    /// Inserts the value at the back of the queue, and returns the value removed from the front.
    pub fn push_pop(&mut self, value: T) -> T {
        let removed = self.queue[self.position];
        self.queue[self.position] = value;

        self.position += 1;
        self.position %= self.len();

        removed
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_macro() {
        let first = [1, 4, 7];
        let second = [2, 5, 8];
        let third = [3, 6, 9];

        let zipped: Vec<((i32, i32), i32)> = zip!(first, second, third).collect();

        assert_eq!(zipped, vec![((1, 2), 3), ((4, 5), 6), ((7, 8), 9)])
    }

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
