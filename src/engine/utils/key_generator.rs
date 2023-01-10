use std::{
    collections::HashSet,
    error::Error,
    fmt::{Debug, Display},
    hash::Hash,
};

use num_traits::{cast, PrimInt, Unsigned, WrappingAdd};

/// Construct for generating unique keys, via an incrementing counter.
///
/// Contains a set of all keys currently in use.
#[derive(Debug)]
pub struct KeyGenerator<K>
where
    K: PrimInt + Unsigned + WrappingAdd + Hash + Debug,
{
    last_key: K,
    used_keys: HashSet<K>,
}
impl<K> KeyGenerator<K>
where
    K: PrimInt + Unsigned + WrappingAdd + Hash + Debug,
{
    pub fn new() -> Self {
        KeyGenerator {
            last_key: K::max_value(),
            used_keys: HashSet::new(),
        }
    }

    /// Amount of unique keys that are left.
    ///
    /// This will be decremented after each call to [`Self::reserve_key()`] and [`Self::next_key()`],
    /// which will return an [`OverflowError`] if, and only if this returns 0.
    ///
    /// This will correspondingly be incremented after a succesful call to [`Self::free_key()`].
    pub fn remaining_keys(&self) -> K {
        // Size of used_keys should never be able to exceed number of values of K
        let used = cast(self.used_keys.len()).unwrap();
        K::max_value() - used
    }

    /// Return new unique key, registering it as occupied
    /// until [`Self::free_key()`] is called with this key as argument.
    pub fn next_key(&mut self) -> Result<K, OverflowError> {
        if self.remaining_keys() == K::zero() {
            return Err(OverflowError);
        }

        let mut key = self.last_key;

        loop {
            key = key.wrapping_add(&K::one());
            if !self.used_keys.contains(&key) {
                self.last_key = key;
                self.used_keys.insert(key);
                return Ok(key);
            }
        }
    }

    /// Free key, marking it as no longer occupied, being able to be used again.
    /// Reuse of the key will, however, only happen once the counter has wrapped around.
    pub fn free_key(&mut self, key: K) -> Result<(), InvalidKeyError<K>> {
        let succesful = self.used_keys.remove(&key);
        if succesful {
            Ok(())
        } else {
            Err(InvalidKeyError { key })
        }
    }

    /// Reserve a key, marking it as occupied.
    pub fn reserve_key(&mut self, key: K) -> Result<(), KeyCollisionError<K>> {
        let succesful = self.used_keys.insert(key);
        if succesful {
            Ok(())
        } else {
            Err(KeyCollisionError { key })
        }
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
pub struct InvalidKeyError<K: Debug> {
    key: K,
}
impl<K> Display for InvalidKeyError<K>
where
    K: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Key not present: {:?}", self.key)
    }
}
impl<K> Error for InvalidKeyError<K> where K: Debug {}

#[derive(Debug, PartialEq, Eq)]
pub struct KeyCollisionError<K: Debug> {
    key: K,
}
impl<K> Display for KeyCollisionError<K>
where
    K: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Key already present: {:?}", self.key)
    }
}
impl<K> Error for KeyCollisionError<K> where K: Debug {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_one() {
        let mut kg = KeyGenerator::<u32>::new();
        assert_eq!(kg.remaining_keys(), u32::MAX);
        kg.next_key().unwrap();
        assert_eq!(kg.remaining_keys(), u32::MAX - 1);
    }

    #[test]
    fn add_multiple() {
        let mut kg = KeyGenerator::<u32>::new();

        for i in 1..50 {
            kg.next_key().unwrap();
            assert_eq!(kg.remaining_keys(), u32::MAX - i);
        }
    }

    #[test]
    fn free_one() {
        let mut kg = KeyGenerator::<u32>::new();
        let k = kg.next_key().unwrap();
        kg.free_key(k).unwrap();
        assert_eq!(kg.remaining_keys(), u32::MAX);
    }

    #[test]
    fn free_multiple() {
        let mut kg = KeyGenerator::<u32>::new();

        let mut ks = Vec::new();
        for _ in 0..50 {
            ks.push(kg.next_key().unwrap());
        }

        for k in ks {
            kg.free_key(k).unwrap();
        }

        assert_eq!(kg.remaining_keys(), u32::MAX);
    }

    #[test]
    fn free_invalid() {
        let mut kg = KeyGenerator::<u32>::new();
        let r = kg.free_key(6);
        assert_eq!(r, Err(InvalidKeyError { key: 6 }));
        assert_eq!(kg.remaining_keys(), u32::MAX);
    }

    #[test]
    fn reserve_key() {
        let mut kg = KeyGenerator::<u32>::new();

        kg.reserve_key(0).unwrap();
        assert_eq!(kg.remaining_keys(), u32::MAX - 1);

        // Depends on key starting at 0
        let k = kg.next_key().unwrap();
        assert_eq!(k, 1);
    }

    #[test]
    fn reserve_invalid() {
        let mut kg = KeyGenerator::<u32>::new();
        let k = kg.next_key().unwrap();
        let r = kg.reserve_key(k);
        assert_eq!(r, Err(KeyCollisionError { key: k }));
    }

    #[test]
    fn free_reserve() {
        let mut kg = KeyGenerator::<u32>::new();
        let k = kg.next_key().unwrap();

        // When freeing a key it should not be reused immediately
        kg.free_key(k).unwrap();
        kg.next_key().unwrap();
        let r = kg.reserve_key(k);

        assert_eq!(r, Ok(()));
        assert_eq!(kg.remaining_keys(), u32::MAX - 2);
    }

    #[test]
    fn overflow() {
        let mut kg = KeyGenerator::<u8>::new();
        for i in 1..=255 {
            kg.next_key().unwrap();
            assert_eq!(kg.remaining_keys(), u8::MAX - i);
        }

        let r = kg.next_key();
        assert_eq!(r, Err(OverflowError));
        assert_eq!(kg.remaining_keys(), 0);
    }
}
