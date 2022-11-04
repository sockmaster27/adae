use std::fmt::Debug;

use super::Info;
use crate::engine::{components::event_queue::EventReceiver, Sample};

pub trait Component: Send + Debug {
    #[allow(unused_variables)]
    fn poll<'a, 'b>(&'a mut self, event_receiver: &mut EventReceiver<'a, 'b>) {}
}

pub trait Source: Component {
    fn output(&mut self, info: &Info) -> &mut [Sample];
}
