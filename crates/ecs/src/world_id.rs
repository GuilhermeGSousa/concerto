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

#[cfg(test)]
mod tests {
    use crate::World;

    #[test]
    fn two_worlds_have_different_ids_and_a_world_keeps_its_own() {
        let first = World::new();
        let second = World::new();

        assert_ne!(first.id(), second.id());
        assert_eq!(first.id(), first.id());
    }
}
