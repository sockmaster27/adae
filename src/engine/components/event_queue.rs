use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt::{Debug, Display},
    marker::PhantomData,
    mem,
    sync::{Arc, Mutex},
};

use ringbuf::RingBuffer;

use crate::engine::dropper::{DBox, Dropper};

type ComponentId = u32;

// For use with arbitrary (void) pointers
type Unknown = u8;

struct Event {
    id: ComponentId,
    inner: Box<dyn Send>,
}
impl Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Event {{ id: {:?}, inner: Box<dyn Send> }}", self.id)
    }
}

pub fn new_event_queue() -> (EventQueue, EventQueueProcessor) {
    let (producer, consumer) = RingBuffer::new(256).split();
    (
        EventQueue {
            // id 0 is used by the queue itself
            ids: HashSet::from([0]),
            last_id: 0,

            events: Arc::new(Mutex::new(producer)),
        },
        EventQueueProcessor {
            events: consumer,
            processors: HashMap::new(),
        },
    )
}

pub struct EventQueue {
    ids: HashSet<ComponentId>,
    last_id: ComponentId,
    events: Arc<Mutex<ringbuf::Producer<Event>>>,
}
impl EventQueue {
    pub fn add_component<E: Send>(
        &mut self,
    ) -> Result<(EventProducer<E>, EventProducerId<E>), ComponentOverflowError> {
        let id = self.next_id()?;

        Ok((
            EventProducer {
                id,
                events: Arc::clone(&self.events),
                phantom: PhantomData,
            },
            EventProducerId {
                id,
                phantom: PhantomData,
            },
        ))
    }

    fn next_id(&mut self) -> Result<ComponentId, ComponentOverflowError> {
        for i in 1..ComponentId::MAX {
            let id = self.last_id.wrapping_add(i);
            if !self.ids.contains(&id) {
                self.last_id = id;
                return Ok(id);
            }
        }

        Err(ComponentOverflowError)
    }
}
unsafe impl Send for EventQueue {}

pub struct EventQueueProcessor {
    events: ringbuf::Consumer<Event>,
    processors: HashMap<ComponentId, (*mut Unknown, fn(*mut Unknown, DBox<Unknown>))>,
}
impl EventQueueProcessor {
    pub fn event_consumer<'a, 'b>(&'b mut self) -> EventConsumer<'a, 'b> {
        EventConsumer {
            parent: self,
            phantom: PhantomData,
        }
    }
}
unsafe impl Send for EventQueueProcessor {}

pub struct EventProducer<E> {
    id: ComponentId,
    events: Arc<Mutex<ringbuf::Producer<Event>>>,

    phantom: PhantomData<E>,
}
impl<'a, T> EventProducer<T>
where
    T: Send + 'static,
{
    pub fn send_box(&self, event: Box<T>) {
        self.events
            .lock()
            .unwrap()
            .push(Event {
                id: self.id,
                inner: event,
            })
            .unwrap();
    }

    pub fn send(&self, event: T) {
        self.send_box(Box::new(event));
    }
}
pub struct EventProducerId<E> {
    id: ComponentId,
    phantom: PhantomData<E>,
}
impl<E> Clone for EventProducerId<E> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            phantom: PhantomData,
        }
    }
}
impl<E> Copy for EventProducerId<E> {}
impl<E> Debug for EventProducerId<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EventProducerId({:?})", self.id)
    }
}

pub struct EventConsumer<'a, 'b> {
    phantom: PhantomData<&'a mut Unknown>,
    parent: &'b mut EventQueueProcessor,
}
impl<'a, 'b> EventConsumer<'a, 'b> {
    pub fn register<C, E>(
        &mut self,
        id: EventProducerId<E>,
        component: &'a mut C,
        callback: fn(&mut C, DBox<E>),
    ) where
        E: Send + 'static,
    {
        // Safety:
        //
        // It should be safe to cast `component` from &mut C to *mut Unknown,
        // when `callback` is also transmuted into a function that takes *mut Unknown as its first parameter, since:
        // 1. The in-memory representation of &mut is assumed to be identical to that of *mut
        // 2. `callback` will only ever be called with `component` as its first argument
        // 3. The `component` pointer will be removed when the current EventConsumer is dropped,
        //    and it is guaranteed that the reference to `component` outlives this
        //
        // It should also be safe for the second parameter of `callback` to be transumted from &mut E to *mut Unknown, since:
        // 4. See point 1.
        // 5. An EventProducerId<E> should only ever be obtainable paired with an EventProducer<E> with a matching inner id,
        //    making it impossible - within reason - to push an event to the queue with this id, that has an inner type different from E
        self.parent.processors.insert(id.id, unsafe {
            (
                component as *mut C as *mut Unknown,
                mem::transmute(callback),
            )
        });
    }

    pub fn poll(self, dropper: &mut Dropper) {
        self.parent.events.pop_each(
            |mut event| {
                let (component, process) = self
                    .parent
                    .processors
                    .get(&event.id)
                    .expect("No processor registered for given id");

                let event_box =
                    unsafe { Box::from_raw(Box::into_raw(event.inner) as *mut Unknown) };
                let event_dbox = dropper.dbox(event_box);
                process(*component, event_dbox);

                true
            },
            None,
        );
    }
}
impl<'a, 'b> Drop for EventConsumer<'a, 'b> {
    fn drop(&mut self) {
        self.parent.processors.clear();
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ComponentOverflowError;
impl Display for ComponentOverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The max number of components has been exceeded")
    }
}
impl Error for ComponentOverflowError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test() {
        let (mut eq, mut eqp) = new_event_queue();
        let (ep, epi) = eq.add_component::<u32>().unwrap();
        let mut d = Dropper::dummy();

        ep.send(2);

        no_heap! {{
            let mut ec = eqp.event_consumer();
            let mut comp = 5;

            ec.register(epi, &mut comp, |&mut comp, number| {
                assert_eq!(comp, 5);
                assert_eq!(*number, 2);
            });
            ec.poll(&mut d);
        }}
    }
}
