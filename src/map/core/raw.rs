#![allow(unsafe_code)]
//! This module encapsulates the `unsafe` access to `hashbrown::raw::RawTable`,
//! mostly in dealing with its bucket "pointers".

use super::{equivalent, Entry, HashValue, IndexMapCore, VacantEntry};
use core::fmt;
use core::mem::replace;
use hashbrown::raw::RawTable;

type RawBucket = hashbrown::raw::Bucket<usize>;

pub(super) struct DebugIndices<'a>(pub &'a RawTable<usize>);
impl fmt::Debug for DebugIndices<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // SAFETY: we're not letting any of the buckets escape this function
        let indices = unsafe { self.0.iter().map(|raw_bucket| raw_bucket.read()) };
        f.debug_list().entries(indices).finish()
    }
}

impl<K, V> IndexMapCore<K, V> {
    /// Sweep the whole table to erase indices start..end
    pub(super) fn erase_indices_sweep(&mut self, start: usize, end: usize) {
        // SAFETY: we're not letting any of the buckets escape this function
        unsafe {
            let offset = end - start;
            for bucket in self.indices.iter() {
                let i = bucket.read();
                if i >= end {
                    bucket.write(i - offset);
                } else if i >= start {
                    self.indices.erase(bucket);
                }
            }
        }
    }

    pub(crate) fn entry(&mut self, hash: HashValue, key: K) -> Entry<'_, K, V>
    where
        K: Eq,
    {
        let eq = equivalent(&key, &self.entries);
        match self.indices.find(hash.get(), eq) {
            // SAFETY: The entry is created with a live raw bucket, at the same time
            // we have a &mut reference to the map, so it can not be modified further.
            Some(raw_bucket) => Entry::Occupied(OccupiedEntry {
                map: self,
                raw_bucket,
                key,
            }),
            None => Entry::Vacant(VacantEntry {
                map: self,
                hash,
                key,
            }),
        }
    }

    pub(super) fn indices_mut(&mut self) -> impl Iterator<Item = &mut usize> {
        // SAFETY: we're not letting any of the buckets escape this function,
        // only the item references that are appropriately bound to `&mut self`.
        unsafe { self.indices.iter().map(|bucket| bucket.as_mut()) }
    }
}

/// A view into an occupied entry in a `IndexMap`.
/// It is part of the [`Entry`] enum.
///
/// [`Entry`]: enum.Entry.html
// SAFETY: The lifetime of the map reference also constrains the raw bucket,
// which is essentially a raw pointer into the map indices.
pub struct OccupiedEntry<'a, K, V> {
    map: &'a mut IndexMapCore<K, V>,
    raw_bucket: RawBucket,
    key: K,
}

// `hashbrown::raw::Bucket` is only `Send`, not `Sync`.
// SAFETY: `&self` only accesses the bucket to read it.
unsafe impl<K: Sync, V: Sync> Sync for OccupiedEntry<'_, K, V> {}

// The parent module also adds methods that don't threaten the unsafe encapsulation.
impl<'a, K, V> OccupiedEntry<'a, K, V> {
    pub fn key(&self) -> &K {
        &self.key
    }

    pub fn get(&self) -> &V {
        &self.map.entries[self.index()].value
    }

    pub fn get_mut(&mut self) -> &mut V {
        let index = self.index();
        &mut self.map.entries[index].value
    }

    /// Put the new key in the occupied entry's key slot
    pub(crate) fn replace_key(self) -> K {
        let index = self.index();
        let old_key = &mut self.map.entries[index].key;
        replace(old_key, self.key)
    }

    /// Return the index of the key-value pair
    #[inline]
    pub fn index(&self) -> usize {
        // SAFETY: we have &mut map keep keeping the bucket stable
        unsafe { self.raw_bucket.read() }
    }

    pub fn into_mut(self) -> &'a mut V {
        let index = self.index();
        &mut self.map.entries[index].value
    }

    /// Remove and return the key, value pair stored in the map for this entry
    ///
    /// Like `Vec::swap_remove`, the pair is removed by swapping it with the
    /// last element of the map and popping it off. **This perturbs
    /// the postion of what used to be the last element!**
    ///
    /// Computes in **O(1)** time (average).
    pub fn swap_remove_entry(self) -> (K, V) {
        // SAFETY: This is safe because it can only happen once (self is consumed)
        // and map.indices have not been modified since entry construction
        let index = unsafe { self.map.indices.remove(self.raw_bucket) };
        self.map.swap_remove_finish(index)
    }

    /// Remove and return the key, value pair stored in the map for this entry
    ///
    /// Like `Vec::remove`, the pair is removed by shifting all of the
    /// elements that follow it, preserving their relative order.
    /// **This perturbs the index of all of those elements!**
    ///
    /// Computes in **O(n)** time (average).
    pub fn shift_remove_entry(self) -> (K, V) {
        // SAFETY: This is safe because it can only happen once (self is consumed)
        // and map.indices have not been modified since entry construction
        let index = unsafe { self.map.indices.remove(self.raw_bucket) };
        self.map.shift_remove_finish(index)
    }
}