use std::collections::{HashMap, VecDeque};

use concerto_ui::node::UIBox;
use glam::Vec2;

use super::{CanvasLink, CanvasMode, CanvasNode, CanvasNodeId, CanvasPart};

/// Width of every node box, in logical pixels.
pub const NODE_WIDTH: f32 = 224.0;
/// Height of a node's title row.
pub const HEADER_HEIGHT: f32 = 24.0;
/// Height of a node's subtitle row, when it has one.
pub const SUBTITLE_HEIGHT: f32 = 18.0;
/// Height of each input pin's row in `Flow`.
pub const PIN_ROW_HEIGHT: f32 = 20.0;
/// Space under a node's last row.
pub const NODE_PADDING: f32 = 4.0;
/// Space between two columns of nodes in `Flow`, which links cross.
pub const COLUMN_GAP: f32 = 72.0;
/// Space between two columns of nodes in `Diagram`, wide enough for a link's label.
pub const DIAGRAM_COLUMN_GAP: f32 = 200.0;
/// Space between two nodes stacked in a column.
pub const ROW_GAP: f32 = 16.0;
/// Space between the content's edge and its outermost nodes.
pub const MARGIN: f32 = 24.0;
/// How far from a pin's centre a pointer still hits it.
pub const PIN_REACH: f32 = 7.0;
/// How far from a link's line a pointer still hits it.
pub const LINK_REACH: f32 = 4.0;

const FORWARD_BEND: f32 = 110.0;
const BACKWARD_BEND: f32 = 110.0;
const LOOP_BEND: f32 = 40.0;
const LOOP_OFFSET: f32 = 8.0;

/// Which way a link's arrowhead points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowDirection {
    Left,
    Right,
}

/// A node's box and the centres of its pins, in content space.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeBox {
    pub id: CanvasNodeId,
    pub rect: UIBox,
    pub inputs: Vec<Vec2>,
    pub output: Option<Vec2>,
}

/// One straight run of a link.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub from: Vec2,
    pub to: Vec2,
}

impl Segment {
    /// The axis-aligned box that covers the run, `thickness` across.
    pub fn bounds(self, thickness: f32) -> UIBox {
        let half = Vec2::splat(thickness / 2.0);
        let min = self.from.min(self.to) - half;
        let max = self.from.max(self.to) + half;
        UIBox {
            min,
            size: max - min,
        }
    }
}

/// A link's path: out of `start`, along to `bend`, up or down, and along to `end`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinkRoute {
    /// The link's index in the canvas's `links`.
    pub link: usize,
    pub start: Vec2,
    /// The x of the vertical run.
    pub bend: f32,
    pub end: Vec2,
    pub arrow: Option<ArrowDirection>,
}

impl LinkRoute {
    /// A route between two points that bends halfway, as the connect drag draws.
    pub fn between(start: Vec2, end: Vec2) -> Self {
        Self {
            link: 0,
            start,
            bend: (start.x + end.x) / 2.0,
            end,
            arrow: None,
        }
    }

    /// The three runs, in order: horizontal, vertical, horizontal.
    pub fn segments(&self) -> [Segment; 3] {
        let first = Vec2::new(self.bend, self.start.y);
        let second = Vec2::new(self.bend, self.end.y);
        [
            Segment {
                from: self.start,
                to: first,
            },
            Segment {
                from: first,
                to: second,
            },
            Segment {
                from: second,
                to: self.end,
            },
        ]
    }
}

/// Every node's box and every link's route, in content space, whose origin is the content's top left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CanvasLayout {
    /// One box per node, in the order of the canvas's `nodes`.
    pub nodes: Vec<NodeBox>,
    /// One route per link whose two ends exist, in the order of the canvas's `links`.
    pub links: Vec<LinkRoute>,
    /// The extent of the content, margins included.
    pub size: Vec2,
}

impl CanvasLayout {
    /// The box of the first node with this id.
    pub fn node(&self, id: CanvasNodeId) -> Option<&NodeBox> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// What lies under `point`: a pin before the node it sits on, a node before a link behind it.
    pub fn part_at(&self, point: Vec2) -> CanvasPart {
        for node in &self.nodes {
            if node
                .output
                .is_some_and(|pin| pin.distance(point) <= PIN_REACH)
            {
                return CanvasPart::Output(node.id);
            }
            let slot = node
                .inputs
                .iter()
                .position(|pin| pin.distance(point) <= PIN_REACH);
            if let Some(slot) = slot {
                return CanvasPart::Input {
                    node: node.id,
                    slot,
                };
            }
        }
        if let Some(node) = self.nodes.iter().find(|node| node.rect.contains(point)) {
            return CanvasPart::Node(node.id);
        }
        self.links
            .iter()
            .find(|route| {
                route
                    .segments()
                    .iter()
                    .any(|segment| segment.bounds(LINK_REACH * 2.0).contains(point))
            })
            .map_or(CanvasPart::Empty, |route| CanvasPart::Link(route.link))
    }
}

struct Edge {
    link: usize,
    from: usize,
    to: usize,
    slot: Option<usize>,
}

struct Placement {
    column: usize,
    top: f32,
}

fn node_height(node: &CanvasNode, mode: CanvasMode) -> f32 {
    let subtitle = if node.subtitle.is_some() {
        SUBTITLE_HEIGHT
    } else {
        0.0
    };
    let pins = match mode {
        CanvasMode::Flow => node.inputs.len() as f32 * PIN_ROW_HEIGHT,
        CanvasMode::Diagram => 0.0,
    };
    HEADER_HEIGHT + subtitle + pins + NODE_PADDING
}

fn edges_of(nodes: &[CanvasNode], links: &[CanvasLink]) -> Vec<Edge> {
    let mut index = HashMap::new();
    for (position, node) in nodes.iter().enumerate() {
        index.entry(node.id).or_insert(position);
    }
    links
        .iter()
        .enumerate()
        .filter_map(|(link, canvas_link)| {
            Some(Edge {
                link,
                from: *index.get(&canvas_link.from)?,
                to: *index.get(&canvas_link.to)?,
                slot: canvas_link.slot,
            })
        })
        .collect()
}

struct Tree<'a> {
    children: &'a [Vec<usize>],
    heights: &'a [f32],
    placed: Vec<Option<Placement>>,
}

impl Tree<'_> {
    fn place(&mut self, node: usize, column: usize, top: f32) -> f32 {
        self.placed[node] = Some(Placement { column, top });
        let mut cursor = top;
        let mut stacked = false;
        for &child in &self.children[node] {
            if self.placed[child].is_some() {
                continue;
            }
            if stacked {
                cursor += ROW_GAP;
            }
            cursor += self.place(child, column + 1, cursor);
            stacked = true;
        }
        let height = self.heights[node];
        let extent = (cursor - top).max(height);
        self.placed[node] = Some(Placement {
            column,
            top: top + (extent - height) / 2.0,
        });
        extent
    }
}

fn place_flow(nodes: &[CanvasNode], edges: &[Edge], heights: &[f32]) -> Vec<Placement> {
    let mut children = vec![Vec::new(); nodes.len()];
    let mut feeds = vec![false; nodes.len()];
    let mut by_slot: Vec<&Edge> = edges.iter().filter(|edge| edge.from != edge.to).collect();
    by_slot.sort_by_key(|edge| edge.slot.unwrap_or(usize::MAX));
    for edge in by_slot {
        children[edge.to].push(edge.from);
        feeds[edge.from] = true;
    }
    let mut tree = Tree {
        children: &children,
        heights,
        placed: nodes.iter().map(|_| None).collect(),
    };
    let roots = (0..nodes.len()).filter(|&node| !feeds[node]);
    let mut top = MARGIN;
    for node in roots.chain(0..nodes.len()) {
        if tree.placed[node].is_none() {
            top += tree.place(node, 0, top) + ROW_GAP;
        }
    }
    let placed: Vec<Placement> = tree.placed.into_iter().flatten().collect();
    let deepest = placed.iter().map(|at| at.column).max().unwrap_or(0);
    placed
        .into_iter()
        .map(|at| Placement {
            column: deepest - at.column,
            top: at.top,
        })
        .collect()
}

fn place_diagram(nodes: &[CanvasNode], edges: &[Edge], heights: &[f32]) -> Vec<Placement> {
    let mut layers: Vec<Option<usize>> = vec![None; nodes.len()];
    let mut queue = VecDeque::new();
    if !nodes.is_empty() {
        layers[0] = Some(0);
        queue.push_back(0);
    }
    while let Some(node) = queue.pop_front() {
        let next = layers[node].map_or(0, |layer| layer + 1);
        for edge in edges.iter().filter(|edge| edge.from == node) {
            if layers[edge.to].is_none() {
                layers[edge.to] = Some(next);
                queue.push_back(edge.to);
            }
        }
    }
    let last = layers.iter().flatten().max().map_or(0, |layer| layer + 1);
    let mut tops = vec![MARGIN; last + 1];
    layers
        .into_iter()
        .zip(heights)
        .map(|(layer, height)| {
            let column = layer.unwrap_or(last);
            let top = tops[column];
            tops[column] += height + ROW_GAP;
            Placement { column, top }
        })
        .collect()
}

fn boxes(
    nodes: &[CanvasNode],
    placements: &[Placement],
    heights: &[f32],
    mode: CanvasMode,
) -> Vec<NodeBox> {
    nodes
        .iter()
        .zip(placements)
        .zip(heights)
        .map(|((node, at), &height)| {
            let gap = match mode {
                CanvasMode::Flow => COLUMN_GAP,
                CanvasMode::Diagram => DIAGRAM_COLUMN_GAP,
            };
            let min = Vec2::new(MARGIN + at.column as f32 * (NODE_WIDTH + gap), at.top);
            let subtitle = if node.subtitle.is_some() {
                SUBTITLE_HEIGHT
            } else {
                0.0
            };
            let pins = mode == CanvasMode::Flow;
            let inputs = if pins {
                (0..node.inputs.len())
                    .map(|slot| {
                        let row = HEADER_HEIGHT + subtitle + (slot as f32 + 0.5) * PIN_ROW_HEIGHT;
                        min + Vec2::new(0.0, row)
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let output =
                (pins && node.output).then(|| min + Vec2::new(NODE_WIDTH, HEADER_HEIGHT / 2.0));
            NodeBox {
                id: node.id,
                rect: UIBox {
                    min,
                    size: Vec2::new(NODE_WIDTH, height),
                },
                inputs,
                output,
            }
        })
        .collect()
}

fn flow_route(edge: &Edge, boxes: &[NodeBox]) -> LinkRoute {
    let from = &boxes[edge.from];
    let to = &boxes[edge.to];
    let start = from
        .output
        .unwrap_or(from.rect.min + Vec2::new(NODE_WIDTH, HEADER_HEIGHT / 2.0));
    let end = edge
        .slot
        .and_then(|slot| to.inputs.get(slot).copied())
        .unwrap_or(to.rect.min + Vec2::new(0.0, HEADER_HEIGHT / 2.0));
    let bend = if end.x > start.x {
        (start.x + end.x) / 2.0
    } else {
        start.x + COLUMN_GAP / 2.0
    };
    LinkRoute {
        link: edge.link,
        start,
        bend,
        end,
        arrow: None,
    }
}

fn diagram_route(edge: &Edge, boxes: &[NodeBox]) -> LinkRoute {
    let from = boxes[edge.from].rect;
    let to = boxes[edge.to].rect;
    let upper = |rect: UIBox| rect.min.y + rect.size.y / 3.0;
    let lower = |rect: UIBox| rect.min.y + rect.size.y * 2.0 / 3.0;
    if to.min.x > from.min.x {
        LinkRoute {
            link: edge.link,
            start: Vec2::new(from.max().x, upper(from)),
            bend: to.min.x - FORWARD_BEND,
            end: Vec2::new(to.min.x, upper(to)),
            arrow: Some(ArrowDirection::Right),
        }
    } else if to.min.x < from.min.x {
        LinkRoute {
            link: edge.link,
            start: Vec2::new(from.min.x, lower(from)),
            bend: to.max().x + BACKWARD_BEND,
            end: Vec2::new(to.max().x, lower(to)),
            arrow: Some(ArrowDirection::Left),
        }
    } else {
        let offset = if to.min.y < from.min.y {
            LOOP_OFFSET
        } else {
            -LOOP_OFFSET
        };
        LinkRoute {
            link: edge.link,
            start: Vec2::new(from.max().x, upper(from)),
            bend: from.max().x + LOOP_BEND + offset,
            end: Vec2::new(to.max().x, lower(to)),
            arrow: Some(ArrowDirection::Left),
        }
    }
}

/// Lays out `nodes` and `links`: a tree per root growing leftward in `Flow`, layers left to right in `Diagram`.
pub fn layout(nodes: &[CanvasNode], links: &[CanvasLink], mode: CanvasMode) -> CanvasLayout {
    let edges = edges_of(nodes, links);
    let heights: Vec<f32> = nodes.iter().map(|node| node_height(node, mode)).collect();
    let placements = match mode {
        CanvasMode::Flow => place_flow(nodes, &edges, &heights),
        CanvasMode::Diagram => place_diagram(nodes, &edges, &heights),
    };
    let boxes = boxes(nodes, &placements, &heights, mode);
    let links = edges
        .iter()
        .map(|edge| match mode {
            CanvasMode::Flow => flow_route(edge, &boxes),
            CanvasMode::Diagram => diagram_route(edge, &boxes),
        })
        .collect();
    let extent = boxes
        .iter()
        .fold(Vec2::ZERO, |extent, node| extent.max(node.rect.max()));
    CanvasLayout {
        nodes: boxes,
        links,
        size: extent + Vec2::splat(MARGIN),
    }
}
