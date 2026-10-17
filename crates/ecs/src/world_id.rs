use std::sync::atomic::{AtomicUsize, Ordering};

/// The identity of a [`World`](crate::World): unique in the process and fixed when the world is created.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WorldId(usize);

impl Default for WorldId {
    fn default() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}
