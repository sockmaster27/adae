use std::{
    fmt::Debug,
    sync::{atomic::Ordering, Arc},
};

use super::utils::{AtomicF32, MovingAverage};

pub fn new_f32_parameter(
    initial: f32,
    max_buffer_size: usize,
) -> (F32ParameterInterface, F32Parameter) {
    let desired1 = Arc::new(AtomicF32::new(initial));
    let desired2 = Arc::clone(&desired1);
    (
        F32ParameterInterface { desired: desired1 },
        F32Parameter {
            buffer: vec![0.0; max_buffer_size],

            desired: desired2,
            moving_average: MovingAverage::new(initial, max_buffer_size),
        },
    )
}

/// Representes a numeric value, controlled by the user - by a knob or slider for example.
///
/// The value is smoothed (via simple moving average), to avoid distortion and clicking in the sound.
#[derive(Debug)]
pub struct F32Parameter {
    buffer: Vec<f32>,

    desired: Arc<AtomicF32>,
    moving_average: MovingAverage,
}
impl F32Parameter {
    pub fn get(&mut self, buffer_size: usize) -> &mut [f32] {
        let desired = self.desired.load(Ordering::Relaxed);

        for point in self.buffer[..buffer_size].iter_mut() {
            self.moving_average.push(desired);
            *point = self.moving_average.average();
        }

        &mut self.buffer[..buffer_size]
    }
}

#[derive(Debug)]
pub struct F32ParameterInterface {
    desired: Arc<AtomicF32>,
}
impl F32ParameterInterface {
    pub fn set(&self, new_value: f32) {
        self.desired.store(new_value, Ordering::Relaxed);
    }
}
