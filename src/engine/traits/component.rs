use std::fmt::Debug;

use super::Info;
use crate::engine::{components::event_queue::EventConsumer, Sample};

pub trait Component: Send + Debug {
    #[allow(unused_variables)]
    fn poll<'a, 'b>(&'a mut self, event_consumer: &mut EventConsumer<'a, 'b>) {}
}

pub trait Source: Component {
    fn output(&mut self, info: &Info) -> &mut [Sample];
}
