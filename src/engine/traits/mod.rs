mod component;
pub use component::{Component, Source};

#[derive(Clone)]
pub struct Info {
    pub sample_rate: u32,
    pub buffer_size: usize,
}
impl Info {
    pub fn new(sample_rate: u32, buffer_size: usize) -> Self {
        Info {
            sample_rate,
            buffer_size,
        }
    }
}
