use std::{collections::HashMap, fmt};

use concerto_animation::{
    graph::{AnimationGraph, AnimationNodeData, AnimationNodeIndex},
    node::{AnimationNode, ErasedAnimationNode, InputSlots, PlaceholderNode},
};
use concerto_reflect::{ReflectedValue, to_value};

/// Names one node of a graph tab for the life of its document; not in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GraphNodeId(pub u32);

impl fmt::Display for GraphNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A node as the working copy holds it: live, or the stored value of one that could not be built.
pub enum HeldValue {
    Live(Box<dyn ErasedAnimationNode>),
    Stored(ReflectedValue),
}

/// One node of a working copy, with its inputs in slot order.
pub struct HeldNode {
    pub id: GraphNodeId,
    pub type_name: String,
    pub value: HeldValue,
    pub inputs: Vec<GraphNodeId>,
    pub warning: Option<String>,
}

impl HeldNode {
    /// The live node, or none for one held as its stored value.
    pub fn live(&self) -> Option<&dyn ErasedAnimationNode> {
        match &self.value {
            HeldValue::Live(node) => Some(node.as_ref()),
            HeldValue::Stored(_) => None,
        }
    }

    /// The live node to edit, or none for one held as its stored value.
    pub fn live_mut(&mut self) -> Option<&mut dyn ErasedAnimationNode> {
        match &mut self.value {
            HeldValue::Live(node) => Some(node.as_mut()),
            HeldValue::Stored(_) => None,
        }
    }

    /// The inputs the node takes; a node held as its stored value takes what a placeholder takes.
    pub fn slots(&self) -> InputSlots {
        self.live().map_or_else(
            || AnimationNode::input_slots(&PlaceholderNode {}),
            ErasedAnimationNode::input_slots,
        )
    }

    /// The last segment of the node's type name.
    pub fn short_name(&self) -> &str {
        let name = self.type_name.split('<').next().unwrap_or(&self.type_name);
        name.rsplit("::").next().unwrap_or(name)
    }

    /// The node's short name with its article, as a sentence names it: `an AnimationClipNode`.
    pub fn described(&self) -> String {
        let name = self.short_name();
        let article = match name.chars().next() {
            Some('A' | 'E' | 'I' | 'O' | 'U') => "an",
            _ => "a",
        };
        format!("{article} {name}")
    }
}

/// Where a node is an input: its parent and the slot it fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphLink {
    pub parent: GraphNodeId,
    pub slot: usize,
}

/// What a connection did: the slot the child took, and the input it displaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Connected {
    pub slot: usize,
    pub loose: Option<GraphNodeId>,
}

/// Where a node of a working copy sits in a graph built from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltNode {
    pub id: GraphNodeId,
    pub index: AnimationNodeIndex,
}

/// A graph built from a working copy, with the index each node of the copy has in it.
pub struct BuiltGraph {
    pub graph: AnimationGraph,
    pub nodes: Vec<BuiltNode>,
}

struct Feed {
    node: GraphNodeId,
    parent: AnimationNodeIndex,
}

enum Place {
    Insert(usize),
    Replace(usize),
}

/// A graph as its tab edits it: every node under an id, loose ones included.
pub struct GraphCopy {
    nodes: Vec<HeldNode>,
    result: GraphNodeId,
    next: u32,
    editable: bool,
}

impl GraphCopy {
    /// Takes `graph` apart; its nodes are live when `editable`, after [`AnimationGraph::make_live`].
    pub fn read(mut graph: AnimationGraph, editable: bool) -> Self {
        let id_of = |index: usize| GraphNodeId(index as u32);
        let indices: Vec<_> = graph.node_indices().collect();
        let result = id_of(graph.result_index().index());
        let nodes: Vec<HeldNode> = indices
            .into_iter()
            .map(|index| {
                let number = index.index();
                let prefix = format!("node {number} ");
                let warnings: Vec<&str> = graph
                    .load_warnings()
                    .iter()
                    .filter(|warning| warning.reason.starts_with(&prefix))
                    .map(|warning| warning.reason.as_str())
                    .collect();
                let warning = (!warnings.is_empty()).then(|| warnings.join("; "));
                let inputs = graph
                    .inputs(index)
                    .into_iter()
                    .map(|input| id_of(input.index()))
                    .collect();
                let taken = graph.node_mut(index).map(|node| {
                    std::mem::replace(
                        node,
                        AnimationNodeData::Stored {
                            type_name: String::new(),
                            value: ReflectedValue::List(Vec::new()),
                        },
                    )
                });
                let (type_name, value) = match taken {
                    Some(AnimationNodeData::Live { type_name, node }) => {
                        (type_name, HeldValue::Live(node))
                    }
                    Some(
                        AnimationNodeData::Stored { type_name, value }
                        | AnimationNodeData::Placeholder { type_name, value },
                    ) => (type_name, HeldValue::Stored(value)),
                    None => (
                        String::new(),
                        HeldValue::Stored(ReflectedValue::List(Vec::new())),
                    ),
                };
                HeldNode {
                    id: id_of(number),
                    type_name,
                    value,
                    inputs,
                    warning,
                }
            })
            .collect();
        let next = nodes.iter().map(|node| node.id.0 + 1).max().unwrap_or(0);
        Self {
            nodes,
            result,
            next,
            editable,
        }
    }

    fn stored(node: &HeldNode) -> anyhow::Result<AnimationNodeData> {
        let value = match &node.value {
            HeldValue::Live(live) => to_value(live.as_reflect())?,
            HeldValue::Stored(value) => value.clone(),
        };
        Ok(AnimationNodeData::Stored {
            type_name: node.type_name.clone(),
            value,
        })
    }

    /// A fresh graph of what the copy holds: the result node, then every node in id order, connected in slot order.
    pub fn build(&self) -> anyhow::Result<BuiltGraph> {
        let mut graph = AnimationGraph::new();
        let mut indices = HashMap::from([(self.result, graph.result_index())]);
        for node in &self.nodes {
            if node.id == self.result {
                continue;
            }
            let index = graph.insert_node(Self::stored(node)?);
            indices.insert(node.id, index);
        }
        for node in &self.nodes {
            for input in &node.inputs {
                graph.connect(indices[&node.id], indices[input])?;
            }
        }
        let nodes = self
            .nodes
            .iter()
            .map(|node| BuiltNode {
                id: node.id,
                index: indices[&node.id],
            })
            .collect();
        Ok(BuiltGraph { graph, nodes })
    }

    /// A fresh graph whose result node is fed by the node `from` and everything that feeds it.
    pub fn build_from(&self, from: GraphNodeId) -> anyhow::Result<BuiltGraph> {
        let mut graph = AnimationGraph::new();
        let mut nodes = Vec::new();
        let mut open = vec![Feed {
            node: from,
            parent: graph.result_index(),
        }];
        while let Some(feed) = open.pop() {
            let node = self
                .node(feed.node)
                .ok_or_else(|| anyhow::anyhow!("There is no node {}", feed.node))?;
            let index = graph.insert_node(Self::stored(node)?);
            graph.connect(feed.parent, index)?;
            nodes.push(BuiltNode {
                id: feed.node,
                index,
            });
            open.extend(node.inputs.iter().rev().map(|input| Feed {
                node: *input,
                parent: index,
            }));
        }
        Ok(BuiltGraph { graph, nodes })
    }

    /// Whether the nodes are live, so the graph can be edited.
    pub fn editable(&self) -> bool {
        self.editable
    }

    /// Every node, in id order.
    pub fn nodes(&self) -> &[HeldNode] {
        &self.nodes
    }

    /// The node `id` names.
    pub fn node(&self, id: GraphNodeId) -> Option<&HeldNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// The node `id` names, to edit.
    pub fn node_mut(&mut self, id: GraphNodeId) -> Option<&mut HeldNode> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    /// The id of the result node.
    pub fn result(&self) -> GraphNodeId {
        self.result
    }

    /// Where `child` is an input, or none for the result node and a loose node.
    pub fn link_of(&self, child: GraphNodeId) -> Option<GraphLink> {
        self.nodes.iter().find_map(|node| {
            let slot = node.inputs.iter().position(|input| *input == child)?;
            Some(GraphLink {
                parent: node.id,
                slot,
            })
        })
    }

    /// Adds a loose node and answers its id.
    pub fn add(&mut self, type_name: String, node: Box<dyn ErasedAnimationNode>) -> GraphNodeId {
        let id = GraphNodeId(self.next);
        self.next += 1;
        self.nodes.push(HeldNode {
            id,
            type_name,
            value: HeldValue::Live(node),
            inputs: Vec::new(),
            warning: None,
        });
        id
    }

    fn label(&self, id: GraphNodeId) -> String {
        match self.node(id) {
            Some(node) => format!("Node {id}, {},", node.described()),
            None => format!("Node {id}"),
        }
    }

    fn known(&self, id: GraphNodeId) -> Result<&HeldNode, String> {
        self.node(id)
            .ok_or_else(|| format!("There is no node {id}; graph_inspect lists the nodes"))
    }

    fn reaches(&self, from: GraphNodeId, to: GraphNodeId) -> bool {
        let mut open = vec![from];
        while let Some(id) = open.pop() {
            if id == to {
                return true;
            }
            if let Some(node) = self.node(id) {
                open.extend(node.inputs.iter().copied());
            }
        }
        false
    }

    fn detach(&mut self, link: GraphLink) {
        if let Some(parent) = self.node_mut(link.parent) {
            parent.inputs.remove(link.slot);
            if let Some(live) = parent.live_mut() {
                live.input_removed(link.slot);
            }
        }
    }

    /// Makes `child` an input of `parent`: at `slot`, replacing what a fixed slot holds, or at the first empty slot.
    pub fn connect(
        &mut self,
        child: GraphNodeId,
        parent: GraphNodeId,
        slot: Option<usize>,
    ) -> Result<Connected, String> {
        self.known(child)?;
        let onto = self.known(parent)?;
        if child == self.result {
            return Err("The result node cannot be an input of another node".to_owned());
        }
        if self.reaches(child, parent) {
            return Err(format!(
                "Making node {child} an input of node {parent} would close a cycle"
            ));
        }
        let from = self.link_of(child);
        if let Some(link) = from
            && link.parent == parent
            && slot.is_none_or(|slot| slot == link.slot)
        {
            return Ok(Connected {
                slot: link.slot,
                loose: None,
            });
        }
        let moved_within = from.is_some_and(|link| link.parent == parent);
        let held = onto.inputs.len() - usize::from(moved_within);
        let place = match onto.slots() {
            InputSlots::None => {
                return Err(format!("{} takes no inputs", self.label(parent)));
            }
            InputSlots::Fixed(names) => match slot {
                Some(slot) if slot >= names.len() => {
                    return Err(format!(
                        "{} has {} input slots, numbered from 0",
                        self.label(parent),
                        names.len()
                    ));
                }
                Some(slot) if slot < held => Place::Replace(slot),
                _ if held < names.len() => Place::Insert(held),
                _ => {
                    return Err(format!(
                        "{} has every input slot taken; name the slot to replace",
                        self.label(parent)
                    ));
                }
            },
            InputSlots::Variable => Place::Insert(slot.map_or(held, |slot| slot.min(held))),
        };
        if let Some(link) = from {
            self.detach(link);
        }
        let Some(onto) = self.node_mut(parent) else {
            return Err(format!("There is no node {parent}"));
        };
        match place {
            Place::Insert(slot) => {
                onto.inputs.insert(slot, child);
                if let Some(live) = onto.live_mut() {
                    live.input_added(slot);
                }
                Ok(Connected { slot, loose: None })
            }
            Place::Replace(slot) => {
                let loose = std::mem::replace(&mut onto.inputs[slot], child);
                if let Some(live) = onto.live_mut() {
                    live.input_removed(slot);
                    live.input_added(slot);
                }
                Ok(Connected {
                    slot,
                    loose: Some(loose),
                })
            }
        }
    }

    /// Takes `child` out of its parent's inputs, leaving it loose.
    pub fn remove_link(&mut self, child: GraphNodeId) -> Result<(), String> {
        self.known(child)?;
        let link = self
            .link_of(child)
            .ok_or_else(|| format!("Node {child} is not an input of any node"))?;
        self.detach(link);
        Ok(())
    }

    /// Removes a node, leaving its inputs loose.
    pub fn remove_node(&mut self, id: GraphNodeId) -> Result<(), String> {
        self.known(id)?;
        if id == self.result {
            return Err("The result node cannot be removed".to_owned());
        }
        if let Some(link) = self.link_of(id) {
            self.detach(link);
        }
        self.nodes.retain(|node| node.id != id);
        Ok(())
    }
}
