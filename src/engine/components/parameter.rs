use super::utils::MovingAverage;

/// Representes a numeric value, controlled by the user - by a knob or slider for example.
///
/// The value is smoothed (via simple moving average), to avoid distortion and clicking in the sound.
pub struct F32Parameter {
    buffer: Vec<f32>,

    desired: f32,
    moving_average: MovingAverage,
}
impl F32Parameter {
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
