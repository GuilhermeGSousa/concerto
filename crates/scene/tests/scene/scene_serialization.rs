use std::path::Path;

use concerto_foundation::assets::content::check_encoding;
use concerto_foundation::assets::AssetId;
use concerto_reflect::ReflectedValue;
use concerto_scene::link_path::LinkPath;
use concerto_scene::node_id::NodeId;
use concerto_scene::scene::{LinkChange, Scene, SceneLinkData, SceneNode, SerializedComponent};

fn encoding_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding"))
}

fn health() -> SerializedComponent {
    SerializedComponent {
        type_name: "game::Health".to_string(),
        value: ReflectedValue::Struct(vec![
            ("current".to_string(), ReflectedValue::Leaf(vec![1, 2])),
            (
                "items".to_string(),
                ReflectedValue::List(vec![ReflectedValue::Leaf(vec![3])]),
            ),
        ]),
    }
}

fn team() -> SerializedComponent {
    SerializedComponent {
        type_name: "game::Team".to_string(),
        value: ReflectedValue::Variant {
            name: "Red".to_string(),
            fields: vec![("size".to_string(), ReflectedValue::Leaf(vec![4]))],
        },
    }
}

fn plain_nodes() -> Vec<SceneNode> {
    vec![
        SceneNode {
            id: NodeId::from_name_path("root"),
            name: "root".to_string(),
            children: vec![1],
            components: vec![],
            link: None,
        },
        SceneNode {
            id: NodeId::from_name_path("root/child"),
            name: "child".to_string(),
            children: vec![],
            components: vec![health(), team()],
            link: None,
        },
    ]
}

#[test]
fn scene_encoding_matches_its_fixture() {
    let mut nodes = plain_nodes();
    nodes.push(SceneNode {
        id: NodeId(7),
        name: "manor_art".to_string(),
        children: vec![],
        components: vec![],
        link: Some(SceneLinkData {
            scene: AssetId::from_path("content/manor_art/scene.gasset"),
            changes: vec![
                LinkChange::AddComponent {
                    path: LinkPath::new(vec![NodeId(11), NodeId(12)]),
                    component: health(),
                },
                LinkChange::RemoveComponent {
                    path: LinkPath::new(vec![NodeId(11), NodeId(13)]),
                    type_name: "game::Team".to_string(),
                },
                LinkChange::RemoveNode {
                    path: LinkPath::node(NodeId(14)),
                },
            ],
        }),
    });
    let scene = Scene {
        nodes,
        referenced_assets: vec![AssetId::from_path("models/character.gltf#mesh/0")],
    };

    check_encoding(&scene, encoding_dir());
}
