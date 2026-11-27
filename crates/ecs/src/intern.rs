use std::{
    collections::HashSet,
    fmt::Debug,
    hash::Hash,
    ops::Deref,
    sync::{PoisonError, RwLock},
};

#[doc(hidden)]
pub use std::boxed::Box;

use crate::utils::fixed_hasher::FixedHasher;

/// A value interned for the life of the process: copying it is free, and equality and hashing
/// compare the address rather than the value.
///
/// Labels such as [`ScheduleLabel`](crate::system::schedule::ScheduleLabel) and
/// [`SystemSet`](crate::SystemSet) are stored this way.
pub struct Interned<T: ?Sized + Internable + 'static>(&'static T);

impl<T: ?Sized + Internable> Deref for Interned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<T: ?Sized + Internable> Clone for Interned<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized + Internable> Copy for Interned<T> {}

impl<T: ?Sized + Internable + Debug> Debug for Interned<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A type that an [`Interner`] can store, comparing and hashing interned values by address.
pub trait Internable: Hash + Eq {
    /// Returns a copy of the value that lives for the rest of the process.
    fn leak(&self) -> &'static Self;

    /// Returns whether `other` is the same interned value as this one, by address.
    fn ref_eq(&self, other: &Self) -> bool;

    /// Hashes the interned value's address into `state`.
    fn ref_hash<H: core::hash::Hasher>(&self, state: &mut H);
}

impl<T: ?Sized + Internable> PartialEq for Interned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0.ref_eq(other.0)
    }
}

impl<T: ?Sized + Internable> Eq for Interned<T> {}

impl<T: ?Sized + Internable> Hash for Interned<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.0.ref_hash(state);
    }
}

impl<T: ?Sized + Internable> From<&Interned<T>> for Interned<T> {
    fn from(value: &Interned<T>) -> Self {
        *value
    }
}

/// A thread-safe set of interned values; interning an equal value twice returns the same
/// [`Interned`].
pub struct Interner<T: ?Sized + 'static>(RwLock<HashSet<&'static T, FixedHasher>>);

impl<T: ?Sized> Interner<T> {
    /// Creates an empty interner; usable in a `static`.
    pub const fn new() -> Self {
        Interner(RwLock::new(HashSet::with_hasher(FixedHasher)))
    }
}

impl<T: ?Sized> Default for Interner<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Internable + ?Sized> Interner<T> {
    /// Returns the interned copy of `value`, leaking a new one the first time an equal value is
    /// seen.
    pub fn intern(&self, value: &T) -> Interned<T> {
        {
            let set = self.0.read().unwrap_or_else(PoisonError::into_inner);

            if let Some(value) = set.get(value) {
                return Interned(*value);
            }
        }

        {
            let mut set = self.0.write().unwrap_or_else(PoisonError::into_inner);

            if let Some(value) = set.get(value) {
                Interned(*value)
            } else {
                let leaked = value.leak();
                set.insert(leaked);
                Interned(leaked)
            }
        }
    }
}
