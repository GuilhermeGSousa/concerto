use concerto_scene::link_path::{node_paths, strip_prefix};
use concerto_scene::scene::{Scene, SceneNode};

fn node(name: &str, children: Vec<usize>) -> SceneNode {
    SceneNode {
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
fn node_paths_name_every_node_distinctly_and_cross_links() {
    let scene = Scene {
        nodes: vec![
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
        ],
        referenced_assets: vec![],
    };

    assert_eq!(
        node_paths(&scene),
        vec![
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
        ]
    );

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
