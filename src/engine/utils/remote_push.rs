use std::{
    collections::HashMap,
    fmt::Debug,
    hash::Hash,
    mem,
    ops::{Deref, DerefMut},
};

use crate::engine::{
    components::event_queue::{EventConsumer, EventProducer, EventProducerId, EventQueue},
    dropper::{DBox, Dropper},
    traits::Component,
};

pub trait RemotePushable<E: Send, K: Send>: Send + Debug + Sized {
    fn new_with_capacity(capacity: usize) -> Self;
    fn push(&mut self, element: E);
    fn remove(&mut self, key: K) -> bool;
    fn transplant(&mut self, other: &mut Self);

    fn new_remote_push_with_capacity(
        initial_capacity: usize,
        event_queue: &mut EventQueue,
        dropper: Dropper,
    ) -> (RemotePusher<E, K, Self>, RemotePushed<E, K, Self>) {
        let (event_producer, event_producer_id) = event_queue.add_component().unwrap();
        (
            RemotePusher {
                length: 0,
                capacity: initial_capacity,
                event_producer,
            },
            RemotePushed {
                inner: dropper.dbox(Box::new(Self::new_with_capacity(initial_capacity))),
                event_producer_id,
                dropper,
            },
        )
    }

    fn new_remote_push(
        event_queue: &mut EventQueue,
        dropper: Dropper,
    ) -> (RemotePusher<E, K, Self>, RemotePushed<E, K, Self>) {
        Self::new_remote_push_with_capacity(16, event_queue, dropper)
    }
}

pub type RemotePusherVec<E> = RemotePusher<E, E, Vec<E>>;
pub type RemotePushedVec<E> = RemotePushed<E, E, Vec<E>>;
impl<E> RemotePushable<E, E> for Vec<E>
where
    E: Send + Debug + PartialEq,
{
    fn new_with_capacity(capacity: usize) -> Self {
        Vec::with_capacity(capacity)
    }

    fn push(&mut self, element: E) {
        self.push(element)
    }

    fn remove(&mut self, element: E) -> bool {
        let pre_len = self.len();

        self.retain(|e| *e != element);

        let removed_one = self.len() == pre_len - 1;
        removed_one
    }

    fn transplant(&mut self, other: &mut Self) {
        for e in self.drain(..) {
            other.push(e)
        }
    }
}

pub type RemotePusherHashMap<K, V> = RemotePusher<(K, V), K, HashMap<K, V>>;
pub type RemotePushedHashMap<K, V> = RemotePushed<(K, V), K, HashMap<K, V>>;
impl<K, V> RemotePushable<(K, V), K> for HashMap<K, V>
where
    K: Send + Debug + Eq + Hash,
    V: Send + Debug,
{
    fn new_with_capacity(capacity: usize) -> Self {
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
    Push(Option<E>),
    PushMultiple(Box<Vec<Option<E>>>),
    Remove(Option<K>),
    RemoveMultiple(Box<Vec<Option<K>>>),
    Reallocated(Option<Box<C>>),
}

pub struct RemotePusher<E, K, C>
where
    E: Send + 'static,
    K: Send + 'static,
    C: RemotePushable<E, K> + 'static,
{
    /// Number of elements currently pushed to the collection
    length: usize,
    /// Number of elements there are space for in the latest allocation of the collection
    /// (possibly the one stored in `self.new_inner`)
    capacity: usize,

    event_producer: EventProducer<Event<E, K, C>>,
}
impl<'a, E, K, C> RemotePusher<E, K, C>
where
    E: Send + 'static,
    K: Send + 'static,
    C: RemotePushable<E, K> + 'static,
{
    pub fn push(&mut self, element: E) {
        self.length += 1;
        self.ensure_capacity(self.length);
        self.event_producer.send(Event::Push(Some(element)));
    }
    pub fn push_multiple(&mut self, elements: Vec<E>) {
        self.length += elements.len();
        self.ensure_capacity(self.length);

        let option_elements = elements.into_iter().map(|e| Some(e)).collect();

        self.event_producer
            .send(Event::PushMultiple(Box::new(option_elements)));
    }

    fn ensure_capacity(&mut self, needed_capacity: usize) {
        if self.capacity < needed_capacity {
            // Find next power of 2 fitting needed_capacity
            let new_capacity = 2usize.pow((needed_capacity as f64).log2().ceil() as u32);

            let new_inner = C::new_with_capacity(new_capacity);
            self.event_producer
                .send(Event::Reallocated(Some(Box::new(new_inner))));

            self.capacity = new_capacity;
        }
    }

    pub fn remove(&mut self, key: K) {
        if self.length == 0 {
            panic!("Attempted to remove element from empty collection");
        }

        self.event_producer.send(Event::Remove(Some(key)));
        self.length -= 1;
    }
    pub fn remove_multiple(&mut self, keys: Vec<K>) {
        if self.length < keys.len() {
            panic!("Number of keys to be removed exceeds length of collection");
        }
        self.length -= keys.len();
        let option_keys = keys.into_iter().map(|k| Some(k)).collect();
        self.event_producer
            .send(Event::RemoveMultiple(Box::new(option_keys)));
    }
}

pub struct RemotePushed<E, K, C>
where
    E: Send + 'static,
    K: Send + 'static,
    C: RemotePushable<E, K> + 'static,
{
    inner: DBox<C>,
    event_producer_id: EventProducerId<Event<E, K, C>>,

    dropper: Dropper,
}
impl<E: Send, K: Send, C: RemotePushable<E, K>> RemotePushed<E, K, C> {
    fn process_event(&mut self, mut event: DBox<Event<E, K, C>>) {
        match &mut *event {
            Event::Push(e) => {
                self.push(e);
            }
            Event::PushMultiple(es) => {
                for e in es.iter_mut() {
                    self.push(e);
                }
            }
            Event::Remove(k) => {
                self.remove(k);
            }
            Event::RemoveMultiple(ks) => {
                for k in ks.iter_mut() {
                    self.remove(k);
                }
            }
            Event::Reallocated(ref mut new) => {
                let mut new = self.dropper.dbox(new.take().unwrap());
                self.inner.transplant(&mut *new);
                mem::swap(&mut new, &mut self.inner);
            }
        }
    }

    fn push(&mut self, e: &mut Option<E>) {
        self.inner.push(e.take().unwrap());
    }

    fn remove(&mut self, k: &mut Option<K>) {
        self.inner.remove(k.take().unwrap());
    }
}
impl<E: Send, K: Send, C: RemotePushable<E, K>> Component for RemotePushed<E, K, C> {
    fn poll<'a, 'b>(&'a mut self, event_consumer: &mut EventConsumer<'a, 'b>) {
        event_consumer.register(self.event_producer_id, self, Self::process_event);
    }
}
impl<E: Send, K: Send, C> Debug for RemotePushed<E, K, C>
where
    C: RemotePushable<E, K>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RemotePushed {{ event_producer_id: {:?}, inner: {:?} }}",
            self.event_producer_id, self.inner
        )
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
    use crate::{
        engine::{components::event_queue::new_event_queue, dropper::Dropper},
        zip,
    };

    use super::*;

    mod vec {
        use super::*;

        #[test]
        fn push_one() {
            let mut d = Dropper::dummy();
            let (mut eq, mut eqp) = new_event_queue();
            let (mut rper, mut rped) = Vec::new_remote_push(&mut eq, d);

            rper.push(5);
            let mut ec = eqp.event_consumer();
            rped.poll(&mut ec);
            ec.poll(&mut d);

            assert_eq!(*rped, vec![5]);
        }

        #[test]
        fn push_repeatedly() {
            let mut d = Dropper::dummy();
            let (mut eq, mut eqp) = new_event_queue();
            let (mut rper, mut rped) = Vec::new_remote_push(&mut eq, d);

            rper.push(2);
            rper.push(7);
            rper.push(5);
            let mut ec = eqp.event_consumer();
            rped.poll(&mut ec);
            ec.poll(&mut d);

            assert_eq!(*rped, vec![2, 7, 5]);
        }

        #[test]
        fn push_multiple() {
            let mut d = Dropper::dummy();
            let (mut eq, mut eqp) = new_event_queue();
            let (mut rper, mut rped) = Vec::new_remote_push(&mut eq, d);

            rper.push_multiple(vec![2, 7, 5]);
            let mut ec = eqp.event_consumer();
            rped.poll(&mut ec);
            ec.poll(&mut d);

            assert_eq!(*rped, vec![2, 7, 5]);
        }

        #[test]
        fn push_multiple_repeatedly() {
            let mut d = Dropper::dummy();
            let (mut eq, mut eqp) = new_event_queue();
            let (mut rper, mut rped) = Vec::new_remote_push(&mut eq, d);

            rper.push_multiple(vec![2, 7, 5]);
            rper.push_multiple(vec![8, 16, 1]);
            rper.push_multiple(vec![3, 14, 4]);
            let mut ec = eqp.event_consumer();
            rped.poll(&mut ec);
            ec.poll(&mut d);

            assert_eq!(*rped, vec![2, 7, 5, 8, 16, 1, 3, 14, 4]);
        }

        #[test]
        fn reallocate() {
            let mut d = Dropper::dummy();
            let (mut eq, mut eqp) = new_event_queue();
            let (mut rper, mut rped) = Vec::new_remote_push_with_capacity(4, &mut eq, d);

            let pre_cap = rped.capacity();
            for i in 0..5 {
                assert_eq!(pre_cap, rped.capacity());
                rper.push(i);
                let mut ec = eqp.event_consumer();
                rped.poll(&mut ec);
                ec.poll(&mut d);
            }
            assert_ne!(pre_cap, rped.capacity());
            assert_eq!(*rped, vec![0, 1, 2, 3, 4]);
        }
    }

    mod hash_map {
        use super::*;

        #[test]
        fn push_one() {
            let mut d = Dropper::dummy();
            let (mut eq, mut eqp) = new_event_queue();
            let (mut rper, mut rped) = HashMap::new_remote_push(&mut eq, d);

            rper.push(("mop", 5));
            let mut ec = eqp.event_consumer();
            rped.poll(&mut ec);
            ec.poll(&mut d);

            assert_eq!(rped.drain().collect::<Vec<(&str, i32)>>(), vec![("mop", 5)]);
        }

        #[test]
        fn push_repeatedly() {
            let mut d = Dropper::dummy();
            let (mut eq, mut eqp) = new_event_queue();
            let (mut rper, mut rped) = HashMap::new_remote_push(&mut eq, d);

            rper.push(("mop", 2));
            rper.push(("stop", 5));
            rper.push(("flop", 1));
            let mut ec = eqp.event_consumer();
            rped.poll(&mut ec);
            ec.poll(&mut d);

            let mut result: Vec<(&str, i32)> = rped.drain().collect();
            result.sort();
            assert_eq!(result, vec![("flop", 1), ("mop", 2), ("stop", 5)]);
        }

        #[test]
        fn push_multiple() {
            let mut d = Dropper::dummy();
            let (mut eq, mut eqp) = new_event_queue();
            let (mut rper, mut rped) = HashMap::new_remote_push(&mut eq, d);

            rper.push_multiple(vec![("mop", 2), ("stop", 5), ("flop", 1)]);
            let mut ec = eqp.event_consumer();
            rped.poll(&mut ec);
            ec.poll(&mut d);

            let mut result: Vec<(&str, i32)> = rped.drain().collect();
            result.sort();
            assert_eq!(result, vec![("flop", 1), ("mop", 2), ("stop", 5)]);
        }

        #[test]
        fn push_multiple_repeatedly() {
            let mut d = Dropper::dummy();
            let (mut eq, mut eqp) = new_event_queue();
            let (mut rper, mut rped) = HashMap::new_remote_push(&mut eq, d);

            rper.push_multiple(vec![("mop", 2), ("stop", 5), ("flop", 1)]);
            rper.push_multiple(vec![("glop", 7), ("plop", 13), ("pop", 0)]);
            rper.push_multiple(vec![("blop", -1), ("slop", 8), ("tlop", 4)]);
            let mut ec = eqp.event_consumer();
            rped.poll(&mut ec);
            ec.poll(&mut d);

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
            let mut d = Dropper::dummy();
            let (mut eq, mut eqp) = new_event_queue();
            let (mut rper, mut rped) = HashMap::new_remote_push_with_capacity(4, &mut eq, d);

            let pre_cap = rped.capacity();
            for (k, v) in zip!("abcde".chars(), 0..5) {
                assert_eq!(pre_cap, rped.capacity());
                rper.push((k, v));
                let mut ec = eqp.event_consumer();
                rped.poll(&mut ec);
                ec.poll(&mut d);
            }
            assert_ne!(pre_cap, rped.capacity());

            let mut result: Vec<(char, usize)> = rped.drain().collect();
            result.sort();
            assert_eq!(
                result,
                vec![('a', 0), ('b', 1), ('c', 2), ('d', 3), ('e', 4)]
            );
        }
    }
}
