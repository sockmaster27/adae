use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::{atomic::Ordering, Arc},
};

use atomicbox::AtomicOptionBox;

pub trait RemotePushable<E, K>: IntoIterator<Item = E> {
    fn new_with_capacity(capacity: usize) -> Self;
    fn push(&mut self, element: E);
    fn remove(&mut self, key: K) -> bool;
    fn capacity(&self) -> usize;
}

pub fn new_remote_push<E, K, C: RemotePushable<E, K>>(
    initial_capacity: usize,
) -> (RemotePusher<E, K, C>, RemotePushed<E, K, C>) {
    (RemotePusher, RemotePushed)
}

pub struct RemotePusher<E, K, C: RemotePushable<E, K>> {
    /// Number of elements currently pushed to the collection
    length: usize,
    /// Number of elements there are space for in the latest allocation of the collection
    /// (possibly the one stored in `self.new_inner`)
    capacity: usize,

    new_inner: Arc<AtomicOptionBox<C>>,
    old_inner: Arc<AtomicOptionBox<C>>,

    pushed_elements: ringbuf::Producer<E>,
    removed_elements: ringbuf::Producer<K>,

    pushed_element_batches: ringbuf::Producer<Vec<E>>,
    removed_element_batches: ringbuf::Producer<Vec<K>>,

    old_pushed_batches: ringbuf::Consumer<Vec<E>>,
    old_removed_batches: ringbuf::Consumer<Vec<K>>,
}
impl<E, K, C: RemotePushable<E, K>> RemotePusher<E, K, C> {
    pub fn push(&self, element: E) -> Result<(), E> {
        self.ensure_capacity(self.length + 1);
        self.pushed_elements.push(element)?;
        self.length += 1;
        Ok(())
    }
    pub fn push_multiple(&self, elements: Vec<E>) -> Result<(), Vec<E>> {
        self.ensure_capacity(self.length + elements.len());
        self.pushed_element_batches.push(elements)?;
        self.length += elements.len();
        Ok(())
    }

    fn ensure_capacity(&self, needed_capacity: usize) {
        if self.capacity < needed_capacity {
            // Find next power of 2 fitting needed_capacity
            let new_capacity = 2usize.pow((needed_capacity as f64).log2().ceil() as u32);

            let new_inner = C::new_with_capacity(new_capacity);
            self.new_inner
                .store(Some(Box::new(new_inner)), Ordering::SeqCst);

            self.capacity = new_capacity;
        }
    }

    pub fn remove(&self, key: K) -> Result<(), K> {
        self.removed_elements.push(key)?;
        self.length -= 1;
        Ok(())
    }
    pub fn remove_multiple(&self, keys: Vec<K>) -> Result<(), Vec<K>> {
        if self.length < keys.len() {
            return Err(keys);
        }

        self.removed_element_batches.push(keys)?;
        self.length -= keys.len();
        Ok(())
    }
}

pub struct RemotePushed<E, K, C> {
    inner: Box<C>,
    length: usize,

    new_inner: Arc<AtomicOptionBox<C>>,
    old_inner: Arc<AtomicOptionBox<C>>,

    pushed_elements: ringbuf::Consumer<E>,
    removed_elements: ringbuf::Consumer<K>,

    pushed_element_batches: ringbuf::Consumer<Vec<E>>,
    removed_element_batches: ringbuf::Consumer<Vec<K>>,
}
impl<E, K: Debug, C: RemotePushable<E, K>> RemotePushed<E, K, C> {
    pub fn poll(&self) {
        self.swap_inner();
        let current_capacity = self.inner.capacity();

        let push = |element| {
            if self.length == current_capacity {
                self.swap_inner();

                #[cfg(debug_assertions)]
                if current_capacity >= self.inner.capacity() {
                    panic!();
                }

                let current_capacity = self.inner.capacity();
            }
            self.inner.push(element);
        };

        self.pushed_elements.pop_each(
            |element| {
                self.inner.push(element);
                true
            },
            None,
        );
        self.pushed_element_batches.pop_each(
            |elements| {
                for element in elements {
                    self.inner.push(element);
                }
                true
            },
            None,
        );
    }

    fn swap_inner(&mut self) {
        let new_inner = self.new_inner.take(Ordering::SeqCst);
        if let Some(new_inner) = new_inner {
            for element in *self.inner {
                new_inner.push(element);
            }

            let old_inner = self.old_inner.swap(Some(self.inner), Ordering::SeqCst);
            #[cfg(debug_assertions)]
            if let Some(_) = old_inner {
                panic!("");
            }
            self.inner = new_inner;
        }
    }

    fn remove(&self, key: K) {
        let found = self.inner.remove(key);

        #[cfg(debug_assertions)]
        if !found {
            panic!("Attempt to remove element, {key:?}, failed")
        }
    }
    fn remove_all(&self) {
        self.removed_elements.pop_each(
            |key| {
                self.remove(key);
                true
            },
            None,
        );
        self.removed_element_batches.pop_each(
            |keys| {
                for key in keys {
                    self.remove(key);
                }
                true
            },
            None,
        );
    }
}
impl<E, K, C: RemotePushable<E, K>> Deref for RemotePushed<E, K, C> {
    type Target = C;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl<E, K, C: RemotePushable<E, K>> DerefMut for RemotePushed<E, K, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
