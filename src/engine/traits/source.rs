use std::fmt::Debug;

use super::Info;
use crate::engine::Sample;

pub trait Source: Send + Debug {
    fn poll(&mut self);
    fn output(&mut self, info: Info) -> &mut [Sample];
}
