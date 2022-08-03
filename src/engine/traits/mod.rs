mod source;
pub use source::Source;

pub struct Info {
    pub sample_rate: u32,
    pub buffer_size: usize,
}
