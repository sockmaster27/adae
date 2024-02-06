use std::{
    fmt::Debug,
    sync::{atomic::Ordering, Arc},
};

use crate::engine::CHANNELS;

use crate::engine::utils::{AtomicF32, MovingAverage};

pub fn f32_parameter(
    initial: f32,
    max_buffer_size: usize,
) -> (F32Parameter, F32ParameterProcessor) {
    let desired1 = Arc::new(AtomicF32::new(initial));
    let desired2 = Arc::clone(&desired1);
    (
        F32Parameter { desired: desired1 },
        F32ParameterProcessor {
            desired: desired2,
            moving_average: MovingAverage::new(initial, max_buffer_size),

            buffer: vec![0.0; max_buffer_size * CHANNELS],
        },
    )
}

/// Representes a numeric value, controlled by the user - by a knob or slider for example.
///
/// The value is smoothed (via simple moving average), to avoid distortion and clicking in the sound.
#[derive(Debug)]
pub struct F32Parameter {
    desired: Arc<AtomicF32>,
}
impl F32Parameter {
    pub fn set(&self, value: f32) {
        self.desired.store(value, Ordering::Relaxed);
    }

    /// Get last value passed to [`Self::set`]
    pub fn get(&self) -> f32 {
        self.desired.load(Ordering::Relaxed)
    }
}

pub struct F32ParameterProcessor {
    desired: Arc<AtomicF32>,
    moving_average: MovingAverage,

    buffer: Vec<f32>,
}
impl F32ParameterProcessor {
    pub fn get(&mut self, buffer_size: usize) -> &mut [f32] {
        let desired = self.desired.load(Ordering::Relaxed);

        for point in self.buffer[..buffer_size * CHANNELS].iter_mut() {
            self.moving_average.push(desired);
            *point = self.moving_average.average();
        }

        &mut self.buffer[..buffer_size * CHANNELS]
    }
}
impl Debug for F32ParameterProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("F32ParameterProcessor")
            .field("desired", &self.desired)
            .field("moving_average", &self.moving_average)
            .finish_non_exhaustive()
    }
}
