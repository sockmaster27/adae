use std::fmt::Debug;

#[derive(Clone, Debug)]
pub struct Info {
    pub sample_rate: u32,
    pub buffer_size: usize,
}
