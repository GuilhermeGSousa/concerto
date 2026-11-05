use crate::body::BodyId;

/// What a scene query skips.
#[derive(Clone, Copy, Debug, Default)]
pub struct QueryFilter<'a> {
    pub ignore: &'a [BodyId],
}
