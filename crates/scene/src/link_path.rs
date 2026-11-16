use std::fmt;

use concerto_reflect::Reflect;
use serde::{Deserialize, Serialize};

use crate::node_id::NodeId;

/// Where a linked node is from a link: the node's id at each link level, outermost first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct LinkPath(Vec<NodeId>);

impl LinkPath {
    /// The path through `ids`, outermost first.
    pub fn new(ids: Vec<NodeId>) -> Self {
        Self(ids)
    }

    /// The path of the node `id` of the link's own scene.
    pub fn node(id: NodeId) -> Self {
        Self(vec![id])
    }

    /// The node ids, outermost first.
    pub fn ids(&self) -> &[NodeId] {
        &self.0
    }

    /// The id this path ends at, in the innermost scene it reaches.
    pub fn last(&self) -> Option<NodeId> {
        self.0.last().copied()
    }

    /// This path continued into the node `id` of the scene its last node links.
    pub fn child(&self, id: NodeId) -> Self {
        let mut ids = self.0.clone();
        ids.push(id);
        Self(ids)
    }

    /// This path continued by `rest`, which starts in the scene its last node links.
    pub fn join(&self, rest: &LinkPath) -> Self {
        let mut ids = self.0.clone();
        ids.extend_from_slice(&rest.0);
        Self(ids)
    }

    /// Whether `prefix` names the leading ids of this path.
    pub fn starts_with(&self, prefix: &LinkPath) -> bool {
        self.0.starts_with(&prefix.0)
    }

    /// The rest of this path below `prefix`, when `prefix` names its leading ids and leaves at least one.
    pub fn strip_prefix(&self, prefix: &LinkPath) -> Option<LinkPath> {
        (self.0.len() > prefix.0.len() && self.starts_with(prefix))
            .then(|| Self(self.0[prefix.0.len()..].to_vec()))
    }
}

impl fmt::Display for LinkPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (at, id) in self.0.iter().enumerate() {
            if at > 0 {
                f.write_str("/")?;
            }
            write!(f, "{id}")?;
        }
        Ok(())
    }
}
