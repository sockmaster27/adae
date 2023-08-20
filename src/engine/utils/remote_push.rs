use std::{
    collections::HashMap,
    fmt::Debug,
    hash::Hash,
    mem,
    ops::{Deref, DerefMut},
};

use crate::engine::utils::dropper::DBox;

use super::{
    dropper,
    ringbuffer::{self, ringbuffer_with_capacity},
    smallest_pow2,
};

pub trait RemotePushable<E: Send, K: Send>: Send + Debug + Sized {
    fn len(&self) -> usize;
    fn capacity(&self) -> usize;

    fn with_capacity(capacity: usize) -> Self;
    fn push(&mut self, element: E);
    fn remove(&mut self, key: K) -> bool;
    fn transplant(&mut self, other: &mut Self);

    fn remote_push_with_capacity(
        initial_capacity: usize,
    ) -> (RemotePusher<E, K, Self>, RemotePushed<E, K, Self>) {
        let (event_sender, event_receiver) = ringbuffer_with_capacity(initial_capacity);

        (
            RemotePusher {
                length: 0,
                capacity: initial_capacity,
                event_sender,
            },
            RemotePushed {
                inner: DBox::new(Self::with_capacity(initial_capacity)),
                event_receiver,
            },
        )
    }

    fn remote_push() -> (RemotePusher<E, K, Self>, RemotePushed<E, K, Self>) {
        Self::remote_push_with_capacity(16)
    }

    fn into_remote_push(self) -> (RemotePusher<E, K, Self>, RemotePushed<E, K, Self>) {
        let (event_sender, event_receiver) = ringbuffer_with_capacity(16);

        let length = self.len();
        let capacity = self.capacity();

        (
            RemotePusher {
                length,
                capacity,
                event_sender,
            },
            RemotePushed {
                inner: DBox::new(self),
                event_receiver,
            },
        )
    }
}

pub type RemotePusherHashMap<K, V> = RemotePusher<(K, V), K, HashMap<K, V>>;
pub type RemotePushedHashMap<K, V> = RemotePushed<(K, V), K, HashMap<K, V>>;
impl<K, V> RemotePushable<(K, V), K> for HashMap<K, V>
where
    K: Send + Debug + Eq + Hash,
    V: Send + Debug,
{
    fn len(&self) -> usize {
        self.len()
    }

    fn capacity(&self) -> usize {
        self.capacity()
    }

    fn with_capacity(capacity: usize) -> Self {
        HashMap::with_capacity(2 * capacity)
    }

    fn push(&mut self, element: (K, V)) {
        let (key, value) = element;
        self.insert(key, value);
    }

    fn remove(&mut self, key: K) -> bool {
        self.remove(&key).is_some()
    }

    fn transplant(&mut self, other: &mut Self) {
        for (key, value) in self.drain() {
            other.insert(key, value);
        }
    }
}

enum Event<E, K, C>
where
    E: Send + 'static,
    K: Send + 'static,
    C: RemotePushable<E, K> + 'static,
{
    Push(E),
    Remove(K),

    // Slices are boxed for conversion to Box<dyn Send>
    #[allow(clippy::box_collection)]
    PushMultiple(Box<Vec<E>>),
    #[allow(clippy::box_collection)]
    RemoveMultiple(Box<Vec<K>>),

    Reallocated(Box<C>),
}

pub struct RemotePusher<E, K, C>
where
    E: Send + 'static,
    K: Send + 'static,
    C: RemotePushable<E, K> + 'static,
{
    /// Number of elements currently pushed to the collection
    length: usize,
    /// Number of elements there are room for in the latest allocation of the collection
    capacity: usize,

    event_sender: ringbuffer::Sender<Event<E, K, C>>,
}
impl<E, K, C> RemotePusher<E, K, C>
where
    E: Send + 'static,
    K: Send + 'static,
    C: RemotePushable<E, K> + 'static,
{
    pub fn push(&mut self, element: E) {
        self.length += 1;
        self.ensure_capacity(self.length);
        self.event_sender.send(Event::Push(element));
    }
    pub fn push_multiple(&mut self, elements: Vec<E>) {
        self.length += elements.len();
        self.ensure_capacity(self.length);

        self.event_sender
            .send(Event::PushMultiple(Box::new(elements)));
    }

    fn ensure_capacity(&mut self, needed_capacity: usize) {
        if self.capacity < needed_capacity {
            let new_capacity = smallest_pow2(needed_capacity as f64);

            let new_inner = C::with_capacity(new_capacity);
            self.event_sender
                .send(Event::Reallocated(Box::new(new_inner)));

            self.capacity = new_capacity;
        }
    }

    pub fn remove(&mut self, key: K) {
        if self.length == 0 {
            panic!("Attempted to remove element from empty collection");
        }

        self.event_sender.send(Event::Remove(key));
        self.length -= 1;
    }
    pub fn remove_multiple(&mut self, keys: Vec<K>) {
        if self.length < keys.len() {
            panic!("Number of keys to be removed exceeds length of collection");
        }
        self.length -= keys.len();
        self.event_sender
            .send(Event::RemoveMultiple(Box::new(keys)));
    }
}

pub struct RemotePushed<E, K, C>
where
    E: Send + 'static,
    K: Send + 'static,
    C: RemotePushable<E, K> + 'static,
{
    inner: DBox<C>,
    event_receiver: ringbuffer::Receiver<Event<E, K, C>>,
}
impl<E: Send, K: Send, C: RemotePushable<E, K>> RemotePushed<E, K, C> {
    pub fn push(&mut self, e: E) {
        self.inner.push(e);
    }

    pub fn remove(&mut self, k: K) {
        let successful = self.inner.remove(k);

        if !successful {
            panic!("Attempted to remove key not present in collection");
        }
    }

    pub fn poll(&mut self) {
        for _ in 0..256 {
            let event_option = self.event_receiver.recv();
            match event_option {
                None => return,

                Some(event) => match event {
                    Event::Push(e) => {
                        self.push(e);
                    }
                    Event::Remove(k) => {
                        self.remove(k);
                    }

                    Event::PushMultiple(mut es) => {
                        for e in es.drain(..) {
                            self.push(e);
                        }
                        dropper::send(es);
                    }
                    Event::RemoveMultiple(mut ks) => {
                        for k in ks.drain(..) {
                            self.remove(k);
                        }
                        dropper::send(ks);
                    }

                    Event::Reallocated(new) => {
                        let mut new = DBox::from(new);
                        self.inner.transplant(&mut *new);
                        mem::swap(&mut new, &mut self.inner);
                    }
                },
            }
        }
    }
}
impl<E: Send, K: Send, C> Debug for RemotePushed<E, K, C>
where
    C: RemotePushable<E, K>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RemotePushed {{ inner: {:?}, ... }}", self.inner)
    }
}
impl<E: Send, K: Send, C: RemotePushable<E, K>> Deref for RemotePushed<E, K, C> {
    type Target = C;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl<E: Send, K: Send, C: RemotePushable<E, K>> DerefMut for RemotePushed<E, K, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod hash_map {
        use std::iter::zip;

        use super::*;

        #[test]
        fn push_one() {
            let (mut rper, mut rped) = HashMap::remote_push();

            rper.push(("mop", 5));

            no_heap! {{
                rped.poll();
            }}

            assert_eq!(rped.drain().collect::<Vec<(&str, i32)>>(), vec![("mop", 5)]);
        }

        #[test]
        fn push_repeatedly() {
            let (mut rper, mut rped) = HashMap::remote_push();

            rper.push(("mop", 2));
            rper.push(("stop", 5));
            rper.push(("flop", 1));

            no_heap! {{
                rped.poll();
            }}

            let mut result: Vec<(&str, i32)> = rped.drain().collect();
            result.sort();
            assert_eq!(result, vec![("flop", 1), ("mop", 2), ("stop", 5)]);
        }

        #[test]
        fn push_multiple() {
            let (mut rper, mut rped) = HashMap::remote_push();

            rper.push_multiple(vec![("mop", 2), ("stop", 5), ("flop", 1)]);

            no_heap! {{
                rped.poll();
            }}

            let mut result: Vec<(&str, i32)> = rped.drain().collect();
            result.sort();
            assert_eq!(result, vec![("flop", 1), ("mop", 2), ("stop", 5)]);
        }

        #[test]
        fn push_multiple_repeatedly() {
            let (mut rper, mut rped) = HashMap::remote_push();

            rper.push_multiple(vec![("mop", 2), ("stop", 5), ("flop", 1)]);
            rper.push_multiple(vec![("glop", 7), ("plop", 13), ("pop", 0)]);
            rper.push_multiple(vec![("blop", -1), ("slop", 8), ("tlop", 4)]);

            no_heap! {{
                rped.poll();
            }}

            let mut result: Vec<(&str, i32)> = rped.drain().collect();
            result.sort();
            assert_eq!(
                result,
                vec![
                    ("blop", -1),
                    ("flop", 1),
                    ("glop", 7),
                    ("mop", 2),
                    ("plop", 13),
                    ("pop", 0),
                    ("slop", 8),
                    ("stop", 5),
                    ("tlop", 4)
                ]
            );
        }

        #[test]
        fn reallocate() {
            let (mut rper, mut rped) = HashMap::remote_push_with_capacity(4);

            let pre_cap = rped.capacity();
            for (k, v) in zip("abcde".chars(), 0..5) {
                assert_eq!(pre_cap, rped.capacity());
                rper.push((k, v));

                no_heap! {{
                    rped.poll();
                }}
            }
            assert_ne!(pre_cap, rped.capacity());

            let mut result: Vec<(char, usize)> = rped.drain().collect();
            result.sort();
            assert_eq!(
                result,
                vec![('a', 0), ('b', 1), ('c', 2), ('d', 3), ('e', 4)]
            );
        }

        #[test]
        fn remove_immediately() {
            let (mut rper, mut rped) = HashMap::remote_push();

            rper.push(("mop", 5));
            rper.remove("mop");

            no_heap! {{
                rped.poll();
            }}

            assert!(rped.is_empty());
        }

        #[test]
        fn remove_delayed() {
            let (mut rper, mut rped) = HashMap::remote_push();

            let mut poll = || {
                no_heap! {{
                    rped.poll();
                }}
            };

            rper.push(("mop", 5));
            poll();

            rper.remove("mop");
            poll();

            assert!(rped.is_empty());
        }

        #[test]
        #[should_panic]
        fn remove_invalid() {
            let (mut rper, mut rped) = HashMap::remote_push();

            rper.push(("mop", 5));
            rper.remove("slop");

            no_heap! {{
                rped.poll();
            }}
        }
    }
}
