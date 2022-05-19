// Module temporarily out of use
#![allow(dead_code)]

use super::super::{Sample, CHANNELS};

use super::super::utils::CircularArray;

/// Circular sample delay
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
