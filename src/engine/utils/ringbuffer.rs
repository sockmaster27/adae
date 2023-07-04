///! Ringbuffer based channel, reallocated by the sender
use ringbuf::{HeapConsumer, HeapProducer, HeapRb};

use super::{dropper::DBox, smallest_pow2};

pub fn ringbuffer_with_capacity<T: Send>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    // There will be allocated enough room for capacity elements, plus one more slot for the reallocation
    let (producer, consumer) = HeapRb::new(capacity + 1).split();
    (
        Sender { inner: producer },
        Receiver {
            inner: DBox::new(consumer),
        },
    )
}

pub fn ringbuffer<T: Send>() -> (Sender<T>, Receiver<T>) {
    ringbuffer_with_capacity(64)
}

pub struct Sender<T: Send> {
    inner: HeapProducer<Event<T>>,
}
impl<T> Sender<T>
where
    T: Send,
{
    /// Might heap-allocate a new ringbuffer
    pub fn send(&mut self, element: T) {
        self.ensure_capacity();
        let result = self.inner.push(Event::Element(element));

        #[cfg(debug_assertions)]
        if result.is_err() {
            panic!("Sender::ensure_capacity failed to do its job")
        }
    }

    fn ensure_capacity(&mut self) {
        if self.inner.free_len() == 1 {
            let new_capacity = smallest_pow2((self.inner.capacity() + 1) as f64);
            let (producer, consumer) = HeapRb::new(new_capacity).split();
            let result = self.inner.push(Event::Reallocated(Box::new(consumer)));
            self.inner = producer;

            #[cfg(debug_assertions)]
            if result.is_err() {
                panic!("what")
            }
        }
    }
}

pub struct Receiver<T: 'static + Send> {
    inner: DBox<HeapConsumer<Event<T>>>,
}
impl<T> Receiver<T>
where
    T: Send,
{
    pub fn recv(&mut self) -> Option<T> {
        loop {
            match self.inner.pop() {
                None => return None,
                Some(event) => match event {
                    Event::Element(e) => return Some(e),
                    Event::Reallocated(new) => {
                        self.inner = DBox::from(new);
                    }
                },
            }
        }
    }

    /// Iterate through the elements in the ringbuffer.
    /// Pushing to the ringbuffer while loopinng through this,
    /// may cause this to run forever.
    ///
    /// See also [`Receiver::iter_bound`]
    pub fn iter(&mut self) -> impl Iterator<Item = T> + '_ {
        Iter { inner: self }
    }
}

pub struct Iter<'a, T: 'static + Send> {
    inner: &'a mut Receiver<T>,
}
impl<'a, T> Iterator for Iter<'a, T>
where
    T: Send,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.recv()
    }
}

enum Event<T> {
    Element(T),
    Reallocated(Box<HeapConsumer<Event<T>>>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capacity<T: Send>(sender: &Sender<T>) -> usize {
        // The spot for reallocation doesn't count
        // (consistent with ringbuffer_with_capacity)
        sender.inner.capacity() - 1
    }

    #[test]
    fn send_none() {
        let (_, mut receiver) = ringbuffer::<i32>();
        assert_eq!(receiver.recv(), None);
    }

    #[test]
    fn send_one() {
        let (mut sender, mut receiver) = ringbuffer();
        sender.send(5);
        assert_eq!(receiver.recv(), Some(5));
        assert_eq!(receiver.recv(), None);
    }

    #[test]
    fn send_multiple() {
        let (mut sender, mut receiver) = ringbuffer();

        sender.send(5);
        sender.send(4);
        sender.send(3);

        assert_eq!(receiver.recv(), Some(5));
        assert_eq!(receiver.recv(), Some(4));
        assert_eq!(receiver.recv(), Some(3));
        assert_eq!(receiver.recv(), None);
    }

    #[test]
    fn reallocate() {
        let (mut sender, mut receiver) = ringbuffer_with_capacity(1);
        assert_eq!(capacity(&sender), 2 - 1);

        sender.send(1);
        assert_eq!(capacity(&sender), 2 - 1);

        // Reallocate first time: 2-1 -> 4-1
        sender.send(2);
        assert_eq!(capacity(&sender), 4 - 1);
        sender.send(3);
        assert_eq!(capacity(&sender), 4 - 1);
        sender.send(4);
        assert_eq!(capacity(&sender), 4 - 1);

        // Reallocate second time: 4-1 -> 8-1
        sender.send(5);
        assert_eq!(capacity(&sender), 8 - 1);

        let r: Vec<i32> = receiver.iter().collect();
        assert_eq!(r, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn iter() {
        let (mut sender, mut receiver) = ringbuffer();

        sender.send(5);
        sender.send(4);
        sender.send(3);
        sender.send(7);

        let r: Vec<i32> = receiver.iter().collect();
        assert_eq!(r, vec![5, 4, 3, 7]);
    }
}
