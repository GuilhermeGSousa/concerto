use std::hash::BuildHasher;

use foldhash::fast::{FixedState, FoldHasher};

const FIXED_HASHER: FixedState = FixedState::with_seed(5);

/// A [`BuildHasher`] with a fixed seed, so a map using it can be built in a `const` context.
#[derive(Copy, Clone, Default, Debug)]
pub struct FixedHasher;

impl BuildHasher for FixedHasher {
    type Hasher = FoldHasher<'static>;

    fn build_hasher(&self) -> Self::Hasher {
        FIXED_HASHER.build_hasher()
    }
}
