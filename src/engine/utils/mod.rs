pub mod remote_push;
pub mod ringbuffer;

use std::fmt::Debug;
use std::sync::atomic::{AtomicU32, Ordering};

use super::{Sample, CHANNELS};

/// Macro for conveniently initializing a static array of a given size, of a type that is not [`Copy`].
///
/// The `initial` exoression is evaluated for each element in the array.
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
/// `zip!(a, b, c, d) -> (((a, b), c), d)`
#[macro_export(crate)]
macro_rules! zip {
    ($first:expr, $($rest:expr),+ $(,)?) => { {
        ($first.into_iter())$(.zip($rest.into_iter()))+
    }
    };
}

/// Find smallest power of 2 that is greater than or equal to `x`
pub fn smallest_pow2(x: f64) -> usize {
    2usize.pow(x.log2().ceil() as u32)
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
        for (sample, average) in zip!(frame, &mut averages) {
            *average += f64::from(sample.powi(2)) / buffer_size;
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

    pub fn iter<'a>(&'a self) -> impl Iterator<Item = &T> + 'a {
        let first_part = &self.buffer[self.position..];
        let last_part = &self.buffer[..self.position];
        first_part.iter().chain(last_part)
    }
}
impl<T: Clone + Debug> Debug for CircularArray<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ordered_list: Vec<&T> = self.iter().collect();
        write!(
            f,
            "CircularArray {}",
            format_truncate_list(10, &ordered_list[..])
        )
    }
}

/// Format list truncated like:
/// `"[0, 1, 2 ... 7, 8, 9]"`
pub fn format_truncate_list<T: Debug>(max_length: usize, list: &[T]) -> String {
    fn format_list<'a, T: 'a + Debug>(list: &[T]) -> String {
        let strings: Vec<String> = list.iter().map(|e| format!("{:?}", *e)).collect();
        strings.join(", ")
    }

    let truncated_iter = if list.len() <= max_length {
        format_list(list)
    } else {
        let half_length = max_length / 2;
        let first_five = format_list(&list[..half_length]);
        let last_five = format_list(&list[(list.len() - half_length)..]);
        format!("{} ... {}", first_five, last_five)
    };

    format!("[{}]", truncated_iter)
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

    #[test]
    fn format_small_list() {
        let range: Vec<_> = (0..5).collect();
        let output = format_truncate_list(5, &range[..]);
        assert_eq!(output, "[0, 1, 2, 3, 4]");
    }
    #[test]
    fn format_long_list() {
        let range: Vec<_> = (0..6).collect();
        let output = format_truncate_list(5, &range[..]);
        assert_eq!(output, "[0, 1 ... 4, 5]");
    }
}
