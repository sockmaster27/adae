use std::{
    collections::HashSet,
    error::Error,
    fmt::{Debug, Display},
    hash::Hash,
};

use num_traits::{cast, Bounded, One, PrimInt, Unsigned, WrappingAdd, Zero};

pub trait Key: Copy + Eq + Hash + Debug {
    type Id: PrimInt + Unsigned + WrappingAdd + Hash + Debug;
    fn new(id: Self::Id) -> Self;
    fn id(&self) -> Self::Id;
}

/// Macro for generating a new key type.
/// The resulting type will be a simple newtype wrapper around the given type.
macro_rules! key_type {
    ($name:ident, $id:ty) => {
        #[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub struct $name($id);
        impl Key for $name {
            type Id = $id;
            fn new(id: Self::Id) -> Self {
                Self(id)
            }
            fn id(&self) -> Self::Id {
                self.0
            }
        }
    };
}
pub(crate) use key_type;

/// Construct for generating unique keys, via an incrementing counter.
///
/// Contains a set of all keys currently in use.
#[derive(Debug)]
pub struct KeyGenerator<K>
where
    K: Key,
{
    last_id: K::Id,
    used_ids: HashSet<K::Id>,
}
impl<K> KeyGenerator<K>
where
    K: Key,

    // ↓ This should be implied by the above, but rustc doesn't seem to think so
    K::Id: Bounded + Zero + One + Ord,
{
    /// Create new `KeyGenerator` with no keys in use.
    pub fn new() -> Self {
        KeyGenerator {
            last_id: K::Id::max_value(),
            used_ids: HashSet::new(),
        }
    }

    /// Create new `KeyGenerator` with all keys in the given iterator already reserved.
    pub fn from_iter(iter: impl IntoIterator<Item = K>) -> Self {
        let mut kg = Self::new();
        let mut max = K::Id::zero();
        for key in iter {
            kg.reserve(key).expect("Duplicate key in iterator");
            max = max.max(key.id());
        }
        kg.last_id = max;
        kg
    }

    /// Amount of keys currently in use.
    ///
    /// This will be incremented after each call to [`Self::reserve()`] and [`Self::next()`].
    ///
    /// This will correspondingly be decremented after a succesful call to [`Self::free()`].
    pub fn used_keys(&self) -> K::Id {
        cast(self.used_ids.len()).unwrap()
    }

    /// Amount of unique keys that are left.
    ///
    /// This will be decremented after each call to [`Self::reserve()`] and [`Self::next()`],
    /// which will return an [`OverflowError`] if, and only if this returns 0.
    ///
    /// This will correspondingly be incremented after a succesful call to [`Self::free()`].
    pub fn remaining_keys(&self) -> K::Id {
        // Size of used_keys should never be able to exceed number of values of K
        K::Id::max_value() - self.used_keys()
    }

    /// Return new unique key, registering it as occupied
    /// until [`Self::free()`] is called with this key as argument.
    pub fn next(&mut self) -> Result<K, OverflowError> {
        let id = self.peek_next_id()?;
        let key = K::new(id);
        self.reserve(key).unwrap();
        self.last_id = id;
        Ok(key)
    }

    /// Checks what the next key will be, without actually reserving it.
    ///
    /// NOTE: Calling `self.next()` will redo this calculation,
    /// so if you need the key it is more efficient to call `self.reserve()`.
    pub fn peek_next(&self) -> Result<K, OverflowError> {
        self.peek_next_id().map(K::new)
    }

    /// Checks what the next ID will be, without actually reserving it.
    fn peek_next_id(&self) -> Result<K::Id, OverflowError> {
        if self.remaining_keys() == K::Id::zero() {
            return Err(OverflowError);
        }

        let mut id = self.last_id;

        loop {
            id = id.wrapping_add(&K::Id::one());
            if !self.used_ids.contains(&id) {
                return Ok(id);
            }
        }
    }

    /// Free key, marking it as no longer occupied, being able to be used again.
    /// Reuse of the key will, however, only happen once the counter has wrapped around.
    pub fn free(&mut self, key: K) -> Result<(), InvalidKeyError<K>> {
        let succesful = self.used_ids.remove(&key.id());
        if succesful {
            Ok(())
        } else {
            Err(InvalidKeyError { key })
        }
    }

    /// Reserve a key, marking it as occupied.
    pub fn reserve(&mut self, key: K) -> Result<(), KeyCollisionError<K>> {
        let succesful = self.used_ids.insert(key.id());
        if succesful {
            Ok(())
        } else {
            Err(KeyCollisionError { key })
        }
    }

    /// Check whether key is currently in use
    pub fn in_use(&self, key: K) -> bool {
        self.used_ids.contains(&key.id())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct OverflowError;
impl Display for OverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "The max number of keys has been exceeded")
    }
}
impl Error for OverflowError {}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidKeyError<K: Key> {
    key: K,
}
impl<K> Display for InvalidKeyError<K>
where
    K: Key,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Key not present: {:?}", self.key)
    }
}
impl<K> Error for InvalidKeyError<K> where K: Key {}

#[derive(Debug, PartialEq, Eq)]
pub struct KeyCollisionError<K: Key> {
    key: K,
}
impl<K> Display for KeyCollisionError<K>
where
    K: Key,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Key already present: {:?}", self.key)
    }
}
impl<K> Error for KeyCollisionError<K> where K: Key {}

#[cfg(test)]
mod tests {
    use super::*;

    key_type!(TestKey, u8);

    #[test]
    fn add_one() {
        let mut kg = KeyGenerator::<TestKey>::new();
        assert_eq!(kg.remaining_keys(), u8::MAX);
        kg.next().unwrap();
        assert_eq!(kg.remaining_keys(), u8::MAX - 1);
    }

    #[test]
    fn add_multiple() {
        let mut kg = KeyGenerator::<TestKey>::new();

        for i in 1..50 {
            kg.next().unwrap();
            assert_eq!(kg.remaining_keys(), u8::MAX - i);
        }
    }

    #[test]
    fn free_one() {
        let mut kg = KeyGenerator::<TestKey>::new();
        let k = kg.next().unwrap();
        kg.free(k).unwrap();
        assert_eq!(kg.remaining_keys(), u8::MAX);
    }

    #[test]
    fn free_multiple() {
        let mut kg = KeyGenerator::<TestKey>::new();

        let mut ks = Vec::new();
        for _ in 0..50 {
            ks.push(kg.next().unwrap());
        }

        for k in ks {
            kg.free(k).unwrap();
        }

        assert_eq!(kg.remaining_keys(), u8::MAX);
    }

    #[test]
    fn free_invalid() {
        let mut kg = KeyGenerator::<TestKey>::new();
        let r = kg.free(TestKey(6));
        assert_eq!(r, Err(InvalidKeyError { key: TestKey(6) }));
        assert_eq!(kg.remaining_keys(), u8::MAX);
    }

    #[test]
    fn reserve() {
        let mut kg = KeyGenerator::<TestKey>::new();

        kg.reserve(TestKey(0)).unwrap();
        assert_eq!(kg.remaining_keys(), u8::MAX - 1);

        // Depends on key starting at 0
        let k = kg.next().unwrap();
        assert_eq!(k, TestKey(1));
    }

    #[test]
    fn reserve_invalid() {
        let mut kg = KeyGenerator::<TestKey>::new();
        let k = kg.next().unwrap();
        let r = kg.reserve(k);
        assert_eq!(r, Err(KeyCollisionError { key: k }));
    }

    #[test]
    fn free_reserve() {
        let mut kg = KeyGenerator::<TestKey>::new();
        let k = kg.next().unwrap();

        // When freeing a key it should not be reused immediately
        kg.free(k).unwrap();
        kg.next().unwrap();
        let r = kg.reserve(k);

        assert_eq!(r, Ok(()));
        assert_eq!(kg.remaining_keys(), u8::MAX - 2);
    }

    #[test]
    fn overflow() {
        let mut kg = KeyGenerator::<TestKey>::new();
        for i in 1..=255 {
            kg.next().unwrap();
            assert_eq!(kg.remaining_keys(), u8::MAX - i);
        }

        let r = kg.next();
        assert_eq!(r, Err(OverflowError));
        assert_eq!(kg.remaining_keys(), 0);
    }

    #[test]
    fn in_use() {
        let mut kg = KeyGenerator::<TestKey>::new();

        assert!(!kg.in_use(TestKey(0)));
        kg.reserve(TestKey(0)).unwrap();
        assert!(kg.in_use(TestKey(0)));
    }
}
