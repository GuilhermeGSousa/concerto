use crate::{define_label, intern::Interned};

pub use concerto_ecs_macros::SystemTag;

define_label!(
    /// A label a system can carry; tags never affect ordering, only a [`TagFilter`] reads them.
    SystemTag
);

pub type InternedSystemTag = Interned<dyn SystemTag>;

/// Chooses which systems run when a schedule is compiled with it.
#[derive(Default, Clone)]
pub struct TagFilter {
    pub include: Vec<InternedSystemTag>,
    pub exclude: Vec<InternedSystemTag>,
}

impl TagFilter {
    pub(crate) fn admits(&self, tags: &[InternedSystemTag]) -> bool {
        (self.include.is_empty() || tags.iter().any(|tag| self.include.contains(tag)))
            && !tags.iter().any(|tag| self.exclude.contains(tag))
    }
}
