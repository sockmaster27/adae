use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt::{Debug, Display},
    marker::PhantomData,
    mem,
    sync::{atomic::Ordering, Arc, Mutex},
};

use atomicbox::AtomicOptionBox;

use crate::engine::{
    dropper::{self, DBox},
    utils::{
        ringbuffer::{self, ringbuffer},
        smallest_pow2,
    },
};

// For use with arbitrary (void) pointers
type Unknown = u8;

type ComponentId = u32;

type ProcessorMap = HashMap<ComponentId, (*mut Unknown, fn(*mut Unknown, DBox<Unknown>))>;

#[repr(transparent)]
struct EmptyProcessorMap(ProcessorMap);

impl EmptyProcessorMap {
    fn with_capacity(capacity: usize) -> Self {
        EmptyProcessorMap(ProcessorMap::with_capacity(capacity))
    }

    fn from(mut processors: Box<ProcessorMap>) -> Box<Self> {
        processors.clear();

        // Should be safe as long as EmptyProcessorMap only contains one field and is repr(transparent)
        unsafe { Box::from_raw(Box::into_raw(processors).cast()) }
    }

    fn into(self: Box<Self>) -> Box<ProcessorMap> {
        unsafe { Box::from_raw(Box::into_raw(self).cast()) }
    }
}

// Since EmptyProcessorMap should never be able to contain any pointers, it should be safe to send it
unsafe impl Send for EmptyProcessorMap {}

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
    let (sender, receiver) = ringbuffer();
    let processors = Box::new(HashMap::new());

    let new_processor_map1 = Arc::new(AtomicOptionBox::none());
    let new_processor_map2 = Arc::clone(&new_processor_map1);

    (
        EventQueue {
            ids: HashSet::new(),
            last_id: 0,

            events: Arc::new(Mutex::new(sender)),

            capacity: processors.capacity(),
            len: 0,
            new_processor_map: new_processor_map1,
        },
        EventQueueProcessor {
            events: receiver,
            processors,

            new_processor_map: new_processor_map2,
        },
    )
}

pub struct EventQueue {
    ids: HashSet<ComponentId>,
    last_id: ComponentId,
    events: Arc<Mutex<ringbuffer::Sender<Event>>>,

    capacity: usize,
    len: usize,
    new_processor_map: Arc<AtomicOptionBox<EmptyProcessorMap>>,
}
impl EventQueue {
    pub fn add_component<E: Send>(
        &mut self,
    ) -> Result<(EventSender<E>, EventSenderId<E>), ComponentOverflowError> {
        let id = self.next_id()?;
        self.len += 1;
        self.ensure_capacity(self.len);

        Ok((
            EventSender {
                id,
                events: Arc::clone(&self.events),
                phantom: PhantomData,
            },
            EventSenderId {
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

    fn ensure_capacity(&self, len: usize) {
        if len >= self.capacity {
            let desired_capacity = smallest_pow2((len + 1) as f64);
            let new_processors = EmptyProcessorMap::with_capacity(desired_capacity);
            self.new_processor_map
                .store(Some(Box::new(new_processors)), Ordering::SeqCst);
        }
    }
}
unsafe impl Send for EventQueue {}

pub struct EventQueueProcessor {
    events: ringbuffer::Receiver<Event>,
    processors: Box<ProcessorMap>,
    new_processor_map: Arc<AtomicOptionBox<EmptyProcessorMap>>,
}
impl EventQueueProcessor {
    pub fn event_consumer<'a, 'b>(&'b mut self) -> EventReceiver<'a, 'b> {
        let new_processors = self.new_processor_map.take(Ordering::SeqCst);
        if let Some(new) = new_processors {
            let old = mem::replace(&mut self.processors, new.into());
            dropper::send(EmptyProcessorMap::from(old));
        }

        EventReceiver {
            parent: self,
            phantom: PhantomData,
        }
    }
}
unsafe impl Send for EventQueueProcessor {}

pub struct EventSender<E> {
    id: ComponentId,
    events: Arc<Mutex<ringbuffer::Sender<Event>>>,

    phantom: PhantomData<E>,
}
impl<'a, T> EventSender<T>
where
    T: Send + 'static,
{
    pub fn send_box(&self, event: Box<T>) {
        self.events.lock().unwrap().send(Event {
            id: self.id,
            inner: event,
        });
    }

    pub fn send(&self, event: T) {
        self.send_box(Box::new(event));
    }
}
pub struct EventSenderId<E> {
    id: ComponentId,
    phantom: PhantomData<E>,
}
impl<E> Clone for EventSenderId<E> {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            phantom: PhantomData,
        }
    }
}
impl<E> Copy for EventSenderId<E> {}
impl<E> Debug for EventSenderId<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EventProducerId({:?})", self.id)
    }
}

pub struct EventReceiver<'a, 'b> {
    phantom: PhantomData<&'a mut Unknown>,
    parent: &'b mut EventQueueProcessor,
}
impl<'a, 'b> EventReceiver<'a, 'b> {
    pub fn register<C, E>(
        &mut self,
        id: EventSenderId<E>,
        component: &'a mut C,
        callback: fn(&mut C, DBox<E>),
    ) where
        E: Send + 'static,
    {
        // Safety:
        //
        // It should be safe to cast `component` from &mut C to *mut Unknown,
        // when `callback` is also transmuted into a function that takes *mut Unknown as its first parameter, since:
        // 1. The in-memory representation of any &mut <Sized> is assumed to be identical to that of *mut Unknown
        // 2. `callback` will only ever be called with `component` as its first argument
        // 3. The `component` pointer will be removed when the current EventConsumer is dropped,
        //    and it is guaranteed by the struct lifetimes 'a that the reference to `component` outlives this
        //
        // It should also be safe for the second parameter of `callback` to be transmuted from DBox<E> to DBox<Unknown>, since:
        // 4. The in-memory representation of the two is assumed to be identical, with the contained value being behind a pointer
        // 5. An EventProducerId<E> should only ever be obtainable paired with an EventProducer<E> with a matching inner id,
        //    making it impossible to push an event to the queue with this id, that has an inner type different from E
        self.parent.processors.insert(id.id, unsafe {
            ((component as *mut C).cast(), mem::transmute(callback))
        });
    }

    pub fn poll(self) {
        for event in self.parent.events.iter_bound() {
            let (component, process) = self
                .parent
                .processors
                .get(&event.id)
                .expect("No processor registered for given id");

            let event_box = unsafe { Box::from_raw(Box::into_raw(event.inner).cast()) };
            let event_dbox = DBox::from(event_box);
            process(*component, event_dbox);
        }
    }
}
impl<'a, 'b> Drop for EventReceiver<'a, 'b> {
    fn drop(&mut self) {
        self.parent.processors.clear();
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ComponentOverflowError;
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
    fn empty_map_is_safe() {
        // Ensure that it is safe to cast between Box<EmptyProcessorMap> and Box<ProcessorMap>,
        // like it's done in EmptyProcessorMap::from and EmptyProcessorMap::into
        assert_eq!(
            mem::size_of::<ProcessorMap>(),
            mem::size_of::<EmptyProcessorMap>()
        );
        assert_eq!(
            mem::align_of::<ProcessorMap>(),
            mem::align_of::<EmptyProcessorMap>()
        );
    }

    #[test]
    fn send_one() {
        let (mut eq, mut eqp) = new_event_queue();
        let (ep, epi) = eq.add_component::<u32>().unwrap();

        ep.send(2);

        no_heap! {{
            let mut ec = eqp.event_consumer();
            let mut comp: u8 = 5;

            ec.register(epi, &mut comp, |&mut comp, number| {
                assert_eq!(comp, 5);
                assert_eq!(*number, 2);
            });
            ec.poll();
        }}
    }
}
