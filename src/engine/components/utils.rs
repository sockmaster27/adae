use std::sync::atomic::{AtomicU32, Ordering};

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

pub struct RMS {
    average: MovingAverage,
}
impl RMS {
    pub fn new(length: usize) -> Self {
        Self {
            average: MovingAverage::new(0.0, length),
        }
    }

    pub fn push(&mut self, new_value: f32) {
        self.average.push(new_value.powf(2.0));
    }

    pub fn get_rms(&self) -> f32 {
        self.average.get().sqrt()
    }
}

/// Calculates simple moving average with an internal history buffer.
pub struct MovingAverage {
    average: f32,
    history: CircularArray<f32>,
}
impl MovingAverage {
    pub fn new(initial: f32, window_size: usize) -> Self {
        Self {
            average: initial,
            history: CircularArray::new(initial, window_size),
        }
    }

    pub fn push(&mut self, new_value: f32) {
        let removed_value = self.history.push_pop(new_value);

        let window_size = self.history.len() as f32;
        self.average -= removed_value / window_size;
        self.average += new_value / window_size;
    }

    pub fn get(&self) -> f32 {
        self.average
    }
}

/// A ringbuffer-like queue, where the length is always the same, i.e. it only has one pointer.
// Please correct me if this has a better name.
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
