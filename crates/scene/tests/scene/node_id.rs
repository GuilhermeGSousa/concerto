use std::collections::HashSet;

use concerto_scene::link_path::strip_prefix;
use concerto_scene::node_id::{derive_ids, NodeId};
use concerto_scene::scene::SceneNode;

fn node(name: &str, children: Vec<usize>) -> SceneNode {
    SceneNode {
        id: NodeId::default(),
        name: name.to_string(),
        children,
        components: vec![],
        link: None,
    }
}

struct StripRow {
    path: &'static str,
    prefix: &'static str,
    rest: Option<&'static str>,
}

#[test]
fn node_ids_follow_name_paths_tell_every_node_apart_and_cross_links() {
    let mut nodes = vec![
        node("chair", vec![]),
        node("chair", vec![2]),
        node("leg", vec![]),
        node("chair", vec![]),
        node("a/b", vec![]),
        node("c#d", vec![]),
        node("e\\f", vec![]),
        node("", vec![9]),
        node("", vec![]),
        node("x", vec![]),
        node("x", vec![]),
        node("a", vec![12]),
        node("b", vec![]),
        node("lamp", vec![14, 15]),
        node("lamp.primitive0", vec![]),
        node("lamp.primitive1", vec![]),
    ];

    derive_ids(&mut nodes);

    let paths = [
        "chair",
        "chair#1",
        "chair#1/leg",
        "chair#2",
        "a\\/b",
        "c\\#d",
        "e\\\\f",
        "#0",
        "#1",
        "#0/x",
        "x",
        "a",
        "a/b",
        "lamp",
        "lamp/lamp.primitive0",
        "lamp/lamp.primitive1",
    ];
    for (node, path) in nodes.iter().zip(paths) {
        assert_eq!(node.id, NodeId::from_name_path(path), "{path}");
    }
    assert_eq!(
        NodeId::from_name_path("hall/door"),
        NodeId(0x15d1_8ef5_d862_f9df)
    );
    let ids: Vec<NodeId> = nodes.iter().map(|node| node.id).collect();
    assert_eq!(ids.iter().collect::<HashSet<_>>().len(), ids.len());

    let rows = [
        StripRow {
            path: "manor_art/hall/door",
            prefix: "manor_art",
            rest: Some("hall/door"),
        },
        StripRow {
            path: "manor_artx/hall",
            prefix: "manor_art",
            rest: None,
        },
        StripRow {
            path: "a\\/b/c",
            prefix: "a\\/b",
            rest: Some("c"),
        },
    ];
    for row in rows {
        assert_eq!(
            strip_prefix(row.path, row.prefix).as_deref(),
            row.rest,
            "strip_prefix({:?}, {:?})",
            row.path,
            row.prefix
        );
    }
}
