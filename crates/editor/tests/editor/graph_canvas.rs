use concerto_editor::graph_canvas::{
    CanvasLink, CanvasMode, CanvasNode, CanvasNodeId,
    layout::{CanvasLayout, layout},
};
use proptest::prelude::*;

struct Expected {
    id: u32,
    column: usize,
    row: usize,
}

struct Case {
    name: &'static str,
    mode: CanvasMode,
    nodes: Vec<CanvasNode>,
    links: Vec<CanvasLink>,
    expected: Vec<Expected>,
}

fn node(id: u32) -> CanvasNode {
    CanvasNode::new(CanvasNodeId(id), format!("node {id}")).with_output()
}

fn taking(id: u32, inputs: usize) -> CanvasNode {
    (0..inputs).fold(node(id), |node, slot| node.with_input(format!("in {slot}")))
}

fn link(from: u32, to: u32) -> CanvasLink {
    CanvasLink::new(CanvasNodeId(from), CanvasNodeId(to))
}

fn at(id: u32, column: usize, row: usize) -> Expected {
    Expected { id, column, row }
}

fn columns(placed: &CanvasLayout) -> Vec<f32> {
    let mut lefts: Vec<f32> = placed.nodes.iter().map(|node| node.rect.min.x).collect();
    lefts.sort_by(f32::total_cmp);
    lefts.dedup();
    lefts
}

fn found(placed: &CanvasLayout) -> Vec<Expected> {
    let columns = columns(placed);
    placed
        .nodes
        .iter()
        .map(|node| Expected {
            id: node.id.0,
            column: columns
                .iter()
                .position(|left| *left == node.rect.min.x)
                .unwrap(),
            row: placed
                .nodes
                .iter()
                .filter(|other| {
                    other.rect.min.x == node.rect.min.x && other.rect.min.y < node.rect.min.y
                })
                .count(),
        })
        .collect()
}

#[test]
fn a_canvas_lays_out_trees_leftward_and_diagrams_in_layers() {
    let cases = [
        Case {
            name: "a tree grows leftward with its inputs in slot order",
            mode: CanvasMode::Flow,
            nodes: vec![node(1), node(2), taking(3, 2), taking(4, 1)],
            links: vec![
                link(2, 3).with_slot(1),
                link(1, 3).with_slot(0),
                link(3, 4).with_slot(0),
            ],
            expected: vec![at(1, 0, 0), at(2, 0, 1), at(3, 1, 0), at(4, 2, 0)],
        },
        Case {
            name: "roots stack in node order with their trees",
            mode: CanvasMode::Flow,
            nodes: vec![taking(1, 1), taking(2, 1), node(3), node(4), node(5)],
            links: vec![link(4, 2).with_slot(0), link(3, 1).with_slot(0)],
            expected: vec![
                at(1, 1, 0),
                at(2, 1, 1),
                at(3, 0, 0),
                at(4, 0, 1),
                at(5, 1, 2),
            ],
        },
        Case {
            name: "a loop still places every node once",
            mode: CanvasMode::Flow,
            nodes: vec![taking(1, 1), taking(2, 1)],
            links: vec![link(1, 2).with_slot(0), link(2, 1).with_slot(0)],
            expected: vec![at(1, 1, 0), at(2, 0, 0)],
        },
        Case {
            name: "layers follow the distance from the first node",
            mode: CanvasMode::Diagram,
            nodes: vec![node(1), node(2), node(3), node(4), node(5), node(6)],
            links: vec![
                link(1, 3),
                link(1, 2),
                link(2, 4),
                link(3, 4),
                link(4, 1),
                link(6, 1),
                link(9, 1),
            ],
            expected: vec![
                at(1, 0, 0),
                at(2, 1, 0),
                at(3, 1, 1),
                at(4, 2, 0),
                at(5, 3, 0),
                at(6, 3, 1),
            ],
        },
    ];
    for case in cases {
        let placed = layout(&case.nodes, &case.links, case.mode);
        let found = found(&placed);
        assert_eq!(found.len(), case.expected.len(), "{}", case.name);
        for (found, expected) in found.iter().zip(&case.expected) {
            assert_eq!(found.id, expected.id, "{}", case.name);
            assert_eq!(
                found.column, expected.column,
                "{}: node {}",
                case.name, expected.id
            );
            assert_eq!(
                found.row, expected.row,
                "{}: node {}",
                case.name, expected.id
            );
        }
    }

    let there_and_back = layout(
        &[node(1), node(2)],
        &[link(1, 2), link(2, 1)],
        CanvasMode::Diagram,
    );
    let there = there_and_back.links[0];
    let back = there_and_back.links[1];
    assert_ne!(there.start.y, back.end.y);
    assert_ne!(there.end.y, back.start.y);
    assert_ne!(there.bend, back.bend);

    let beside = layout(
        &[node(1), node(2), node(3)],
        &[link(1, 2), link(1, 3), link(2, 3), link(3, 2)],
        CanvasMode::Diagram,
    );
    let down = beside.links[2];
    let up = beside.links[3];
    assert_ne!(down.start.y, up.end.y);
    assert_ne!(down.end.y, up.start.y);
    assert_ne!(down.bend, up.bend);
}

fn overlapping(placed: &CanvasLayout) -> Option<String> {
    placed.nodes.iter().enumerate().find_map(|(index, first)| {
        placed.nodes[index + 1..]
            .iter()
            .find(|second| first.rect.intersection(second.rect).has_area())
            .map(|second| format!("{:?} overlaps {:?}", first, second))
    })
}

proptest! {
    #[test]
    fn no_two_nodes_overlap_and_a_flow_node_is_left_of_its_parent(
        parents in prop::collection::vec(0usize..64, 1..14),
        subtitled in prop::collection::vec(any::<bool>(), 14),
        extra_from in prop::collection::vec(0usize..64, 0..10),
        extra_to in prop::collection::vec(0usize..64, 0..10),
    ) {
        let count = parents.len();
        let parent_of = |child: usize| Some(parents[child] % (child + 1)).filter(|parent| *parent != child);
        let mut inputs = vec![0usize; count];
        let mut links = Vec::new();
        for child in 0..count {
            if let Some(parent) = parent_of(child) {
                links.push(link(child as u32, parent as u32).with_slot(inputs[parent]));
                inputs[parent] += 1;
            }
        }
        let nodes: Vec<CanvasNode> = (0..count)
            .map(|index| {
                let node = taking(index as u32, inputs[index]);
                if subtitled[index] { node.with_subtitle("clip") } else { node }
            })
            .collect();

        let flow = layout(&nodes, &links, CanvasMode::Flow);
        prop_assert_eq!(overlapping(&flow), None);
        for child in 0..count {
            if let Some(parent) = parent_of(child) {
                let child_box = flow.node(CanvasNodeId(child as u32)).unwrap().rect;
                let parent_box = flow.node(CanvasNodeId(parent as u32)).unwrap().rect;
                prop_assert!(child_box.max().x < parent_box.min.x, "{child} is not left of {parent}");
            }
        }

        for (from, to) in extra_from.iter().zip(&extra_to) {
            links.push(link((from % count) as u32, (to % count) as u32));
        }
        prop_assert_eq!(overlapping(&layout(&nodes, &links, CanvasMode::Flow)), None);
        prop_assert_eq!(overlapping(&layout(&nodes, &links, CanvasMode::Diagram)), None);
    }
}
