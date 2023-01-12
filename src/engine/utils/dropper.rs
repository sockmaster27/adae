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

use super::ringbuffer::{self, ringbuffer};

thread_local! {
    static DROPPER: RefCell<Option<Box<Dropper>>> = RefCell::new(None);
}

/// Send a box to another thread to be dropped
pub fn send(element: Box<dyn Send>) {
    DROPPER.with(|dropper| {
        let mut dropper_option = dropper.borrow_mut();
        let inner_dropper = dropper_option.get_or_insert_with(|| {
            // Initialized the first time it's used
            allow_heap! {{
                Box::new(Dropper::new())
            }}
        });
        inner_dropper.send(Event::Drop(element));
    })
}

enum Event {
    Drop(Box<dyn Send>),
    Done,
}
impl Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Event::Drop(_) => "Event::Drop(...)",
                Event::Done => "Event::Done",
            }
        )
    }
}

/// Construct for dropping things in another thread
struct Dropper {
    sender: ringbuffer::Sender<Event>,
    sleep: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
impl Dropper {
    fn new() -> Self {
        let (sender, mut receiver) = ringbuffer();

        let sleep1 = Arc::new(AtomicBool::new(true));
        let sleep2 = Arc::clone(&sleep1);

        let handle = thread::spawn(move || loop {
            for event in receiver.iter() {
                match event {
                    Event::Drop(e) => drop(e),
                    Event::Done => return,
                }
            }

            sleep1.store(true, Ordering::SeqCst);
            while sleep1.load(Ordering::SeqCst) {
                thread::park();
            }
        });

        Dropper {
            sender,
            sleep: sleep2,
            handle: Some(handle),
        }
    }

    fn send(&mut self, event: Event) {
        let handle = self.handle.as_ref().expect("No connected thread");
        self.sender.send(event);
        self.sleep.store(false, Ordering::SeqCst);
        handle.thread().unpark();
    }
}
impl Drop for Dropper {
    fn drop(&mut self) {
        // There should always be at least one slot left
        self.send(Event::Done);

        // handle.join() doesn't work reliably here, when this is stored in a thread_local,
        // so we'll just have to trust that the thread exists succesfully
    }
}
impl Debug for Dropper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Dropper {{ handle: {:?} }}", self.handle)
    }
}

/// A Box whose content will autoatically be dropped in another thread
#[derive(Debug)]
pub struct DBox<T>
where
    T: Send + 'static,
{
    inner: Option<Box<T>>,
}

impl<T> DBox<T>
where
    T: Send + 'static,
{
    pub fn new(element: T) -> Self {
        DBox {
            inner: Some(Box::new(element)),
        }
    }
}

impl<T> From<Box<T>> for DBox<T>
where
    T: Send + 'static,
{
    fn from(b: Box<T>) -> Self {
        DBox { inner: Some(b) }
    }
}

impl<T> Deref for DBox<T>
where
    T: Send + 'static,
{
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref().expect("DBox is empty")
    }
}
impl<T> DerefMut for DBox<T>
where
    T: Send + 'static,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut().expect("DBox is empty")
    }
}

impl<T> Drop for DBox<T>
where
    T: Send + 'static,
{
    fn drop(&mut self) {
        send(self.inner.take().expect("DBox is empty"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_one() {
        let e = Box::new(5);
        no_heap! {{
            send(e);
        }}
    }

    #[test]
    fn send_multiple() {
        for _ in 0..5 {
            let e = Box::new(5);
            no_heap! {{
                send(e);
            }}
        }
    }

    #[test]
    fn dbox_from() {
        let b = Box::new(5);
        no_heap! {{
            let d = DBox::from(b);
            drop(d);
        }}
    }

    #[test]
    fn multiple_dbox_from() {
        let b1 = Box::new(1);
        let b2 = Box::new(2);
        let b3 = Box::new(3);
        let b4 = Box::new(4);
        no_heap! {{
            let d1 = DBox::from(b1);
            let d2 = DBox::from(b2);
            let d3 = DBox::from(b3);
            let d4 = DBox::from(b4);
            drop(d1);
            drop(d2);
            drop(d3);
            drop(d4);
        }}
    }

    #[test]
    fn dbox_new() {
        let d = DBox::new(5);
        no_heap! {{
            drop(d);
        }}
    }
}
