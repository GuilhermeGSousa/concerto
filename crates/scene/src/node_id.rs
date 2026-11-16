use std::collections::{HashMap, HashSet};
use std::fmt;

use concerto_ecs::Component;
use concerto_reflect::Reflect;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::scene::SceneNode;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// A scene node's id, unique within its scene; an imported node's is a pure function of its name path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
pub struct NodeId(pub u64);

impl NodeId {
    /// The id of the node at `path`: names joined by `/`, with `#n` after the n-th same-named sibling.
    pub fn from_name_path(path: &str) -> NodeId {
        NodeId(fnv1a(FNV_OFFSET, path.as_bytes()))
    }

    /// A random id, for a node the editor creates.
    pub fn fresh() -> NodeId {
        NodeId(Uuid::new_v4().as_u64_pair().0)
    }

    fn rehashed(self) -> NodeId {
        NodeId(fnv1a(FNV_OFFSET, &self.0.to_le_bytes()))
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// The scene node an entity was spawned from; capture writes it back as the node's id.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct SceneNodeId(pub NodeId);

/// Gives every node of `nodes` the id derived from its name path, as an import does.
pub fn derive_ids(nodes: &mut [SceneNode]) {
    let kept = vec![None; nodes.len()];
    derive_missing_ids(nodes, &kept);
}

pub(crate) fn derive_missing_ids(nodes: &mut [SceneNode], kept: &[Option<NodeId>]) {
    let derived: Vec<NodeId> = name_paths(nodes)
        .iter()
        .map(|path| NodeId::from_name_path(path))
        .collect();
    let mut used = HashSet::new();
    for (node, kept) in nodes.iter_mut().zip(kept) {
        if let Some(id) = kept {
            node.id = unique(&mut used, *id);
        }
    }
    for ((node, kept), derived) in nodes.iter_mut().zip(kept).zip(derived) {
        if kept.is_none() {
            node.id = unique(&mut used, derived);
        }
    }
}

fn name_paths(nodes: &[SceneNode]) -> Vec<String> {
    let mut is_child = vec![false; nodes.len()];
    for node in nodes {
        for &child in &node.children {
            if let Some(flag) = is_child.get_mut(child) {
                *flag = true;
            }
        }
    }
    let roots: Vec<usize> = (0..nodes.len()).filter(|&i| !is_child[i]).collect();
    let mut paths: Vec<Option<String>> = vec![None; nodes.len()];
    let mut pending = vec![PendingSiblings {
        parent: String::new(),
        nodes: roots,
    }];
    while let Some(group) = pending.pop() {
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for index in group.nodes {
            let Some(node) = nodes.get(index) else {
                continue;
            };
            let count = seen.entry(node.name.as_str()).or_insert(0);
            let segment = match (node.name.is_empty(), *count) {
                (true, n) => format!("#{n}"),
                (false, 0) => escape(&node.name),
                (false, n) => format!("{}#{n}", escape(&node.name)),
            };
            *count += 1;
            if paths[index].is_some() {
                continue;
            }
            let path = match group.parent.is_empty() {
                true => segment,
                false => format!("{}/{segment}", group.parent),
            };
            pending.push(PendingSiblings {
                parent: path.clone(),
                nodes: node.children.clone(),
            });
            paths[index] = Some(path);
        }
    }
    paths.into_iter().map(Option::unwrap_or_default).collect()
}

struct PendingSiblings {
    parent: String,
    nodes: Vec<usize>,
}

fn escape(name: &str) -> String {
    let mut escaped = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '/' | '#' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}

fn unique(used: &mut HashSet<NodeId>, mut id: NodeId) -> NodeId {
    while !used.insert(id) {
        id = id.rehashed();
    }
    id
}

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}
