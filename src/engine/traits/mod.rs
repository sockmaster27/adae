use super::Sample;
use std::fmt::Debug;

#[derive(Clone, Debug)]
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

pub trait Source: Send + Debug {
    fn poll(&mut self) {}
    fn output(&mut self, info: &Info) -> &mut [Sample];
}

pub trait Effect: Send + Debug {
    fn poll() {}
    fn process(&mut self, info: &Info, buffer: &mut [Sample]);
}
