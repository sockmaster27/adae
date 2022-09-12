mod source;
pub use source::Source;

#[derive(Clone)]
pub struct Info {
    pub sample_rate: u32,
    pub buffer_size: usize,
}
impl Copy for Info {}
