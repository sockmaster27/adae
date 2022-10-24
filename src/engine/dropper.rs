use core::fmt;
use std::{
    cell::RefCell,
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};

use ringbuf::RingBuffer;

/// A Box whose content will autoatically be dropped in another thread
///
/// Obtained though [`Dropper::dbox`]
#[derive(Debug)]
pub struct DBox<T>
where
    T: Send + 'static,
{
    inner: Option<Box<T>>,
    dropper: Dropper,
}
impl<T> Deref for DBox<T>
where
    T: Send + 'static,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.inner.expect("DBox is empty")
    }
}
impl<T> DerefMut for DBox<T>
where
    T: Send + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner.expect("DBox is empty")
    }
}
impl<T> Drop for DBox<T>
where
    T: Send + 'static,
{
    fn drop(&mut self) {
        self.dropper.send(self.inner.take().expect("DBox is empty"));
    }
}

enum Event {
    Drop(Box<dyn Send>),
    Reallocated(ringbuf::Consumer<Event>),
    Done,
}
impl Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Event::Drop(_) => "Event::Drop(...)",
                Event::Reallocated(_) => "Event::Reallocated(...)",
                Event::Done => "Event::Done",
            }
        )
    }
}

/// Channel for sending values to be dropped in another thread
pub struct Dropper {
    inner: Arc<RefCell<DropperInner>>,
}
impl Dropper {
    pub fn new() -> Self {
        let (producer, mut consumer) = RingBuffer::new(256).split();

        let sleep1 = Arc::new(AtomicBool::new(true));
        let sleep2 = Arc::clone(&sleep1);

        let handle = thread::spawn(move || {
            let mut done = false;
            while !done {
                while sleep1.load(Ordering::Acquire) {
                    thread::park();
                }

                let mut new_consumer = None;
                consumer.pop_each(
                    |event| {
                        match event {
                            Event::Drop(e) => drop(e),
                            Event::Reallocated(new) => new_consumer = Some(new),
                            Event::Done => done = true,
                        }

                        true
                    },
                    None,
                );

                if let Some(new) = new_consumer {
                    consumer = new;
                }

                sleep1.store(true, Ordering::Release);
            }
        });

        Dropper {
            inner: Arc::new(DropperInner {
                producer,
                sleep: sleep2,
                handle: Some(handle),
            }),
        }
    }

    pub fn dummy() -> Self {
        let (producer, _) = RingBuffer::new(0).split();

        Dropper {
            inner: Arc::new(DropperInner {
                producer,
                sleep: Arc::new(AtomicBool::new(true)),
                handle: None,
            }),
        }
    }

    pub fn send(&mut self, element: Box<dyn Send>) {
        self.inner.send(element)
    }

    pub fn dbox<T>(&self, element: Box<T>) -> DBox<T>
    where
        T: Send + 'static,
    {
        DBox {
            inner: Some(element),
            dropper: Dropper {
                inner: Arc::clone(&self.inner),
            },
        }
    }
}
impl Debug for Dropper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Dropper {{ inner: {:#16x} }}",
            Arc::as_ptr(&self.inner) as usize
        )
    }
}

struct DropperInner {
    producer: ringbuf::Producer<Event>,
    sleep: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
impl DropperInner {
    fn send(&mut self, element: Box<dyn Send>) {
        // In the case of missing space in the ringbuffer, it will be reallocated on the heap :(
        if self.producer.remaining() == 1 {
            let (new_producer, new_consumer) =
                RingBuffer::new(2 * self.producer.capacity()).split();

            self.send_event(Event::Reallocated(new_consumer));
            self.producer = new_producer;
        }

        self.send_event(Event::Drop(element));
    }

    fn send_event(&mut self, event: Event) {
        if let Some(handle) = self.handle.as_ref() {
            self.producer.push(event).unwrap();
            self.sleep.store(false, Ordering::Release);
            handle.thread().unpark();
        }
    }
}
impl Drop for DropperInner {
    fn drop(&mut self) {
        // There should always be at least one slot left
        self.send_event(Event::Done);

        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send() {
        let d = Dropper::new();
        let e = Box::new(5);
        no_heap! {{
            d.send(e);
        }}
    }
}
