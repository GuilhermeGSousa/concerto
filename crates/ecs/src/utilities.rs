use std::{
    any::TypeId,
    cell::UnsafeCell,
    collections::HashMap,
    hash::{BuildHasher, Hasher},
    ptr,
};

/// [`BuildHasher`] for types that already contain a high-quality hash.
#[derive(Clone, Default)]
pub struct NoOpHash;

impl BuildHasher for NoOpHash {
    type Hasher = NoOpHasher;

    fn build_hasher(&self) -> Self::Hasher {
        NoOpHasher(0)
    }
}

pub type TypeIdMap<V> = HashMap<TypeId, V, NoOpHash>;

#[doc(hidden)]
pub struct NoOpHasher(u64);

// This is for types that already contain a high-quality hash and want to skip
// re-hashing that hash.
impl Hasher for NoOpHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // This should never be called by consumers. Prefer to call `write_u64` instead.
        // Don't break applications (slower fallback, just check in test):
        self.0 = bytes.iter().fold(self.0, |hash, b| {
            hash.rotate_left(8).wrapping_add(*b as u64)
        });
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = i;
    }
}

pub struct SyncUnsafeCell<T: ?Sized>(UnsafeCell<T>);

unsafe impl<T: ?Sized + Sync> Sync for SyncUnsafeCell<T> {}

impl<T> SyncUnsafeCell<T> {
    pub const fn new(value: T) -> Self {
        Self(UnsafeCell::new(value))
    }

    pub fn into_inner(self) -> T {
        self.0.into_inner()
    }

    pub const fn get(&self) -> *mut T {
        self.0.get()
    }

    pub const fn get_mut(&mut self) -> &mut T {
        self.0.get_mut()
    }
}

impl<T: ?Sized> SyncUnsafeCell<T> {
    pub fn from_mut(t: &mut T) -> &mut SyncUnsafeCell<T> {
        let ptr = ptr::from_mut(t) as *mut SyncUnsafeCell<T>;

        unsafe { &mut *ptr }
    }
}

impl<T> SyncUnsafeCell<[T]> {
    pub fn as_slice_of_cells(&self) -> &[SyncUnsafeCell<T>] {
        let self_ptr: *const SyncUnsafeCell<[T]> = ptr::from_ref(self);
        let slice_ptr = self_ptr as *const [SyncUnsafeCell<T>];
        unsafe { &*slice_ptr }
    }
}

impl<T: Default> Default for SyncUnsafeCell<T> {
    fn default() -> SyncUnsafeCell<T> {
        SyncUnsafeCell::new(T::default())
    }
}

impl<T> From<T> for SyncUnsafeCell<T> {
    fn from(t: T) -> SyncUnsafeCell<T> {
        SyncUnsafeCell::new(t)
    }
}

#[repr(transparent)]
pub struct SyncCell<T: ?Sized>(T);

impl<T: Sized> SyncCell<T> {
    /// Construct a new instance of a `SyncCell` from the given value.
    pub fn new(val: T) -> Self {
        Self(val)
    }

    /// Deconstruct this `SyncCell` into its inner value.
    pub fn to_inner(Self(val): Self) -> T {
        val
    }
}

impl<T: ?Sized> SyncCell<T> {
    pub fn get(&mut self) -> &mut T {
        &mut self.0
    }

    /// For types that implement [`Sync`], get shared access to this `SyncCell`'s inner value.
    pub fn read(&self) -> &T
    where
        T: Sync,
    {
        &self.0
    }

    /// Build a mutable reference to a `SyncCell` from a mutable reference
    /// to its inner value, to skip constructing with [`new()`](SyncCell::new()).
    pub fn from_mut(r: &'_ mut T) -> &'_ mut SyncCell<T> {
        // SAFETY: repr is transparent, so refs have the same layout; and `SyncCell` properties are `&mut`-agnostic
        unsafe { &mut *(ptr::from_mut(r) as *mut SyncCell<T>) }
    }
}

impl<T: Clone + Sync> Clone for SyncCell<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

unsafe impl<T: ?Sized> Sync for SyncCell<T> {}
