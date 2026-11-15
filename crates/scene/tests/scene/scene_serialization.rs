use std::path::Path;

use concerto_foundation::assets::content::{
    check_encoding, decode_content_asset, write_content_asset, ContentAssetHeader,
};
use concerto_foundation::assets::AssetId;
use concerto_reflect::ReflectedValue;
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

fn version_1_nodes() -> Vec<SceneNode> {
    vec![
        SceneNode {
            name: "root".to_string(),
            children: vec![1],
            components: vec![],
            link: None,
        },
        SceneNode {
            name: "child".to_string(),
            children: vec![],
            components: vec![health(), team()],
            link: None,
        },
    ]
}

#[test]
fn scene_encoding_matches_its_fixture() {
    let mut nodes = version_1_nodes();
    nodes.push(SceneNode {
        name: "manor_art".to_string(),
        children: vec![],
        components: vec![],
        link: Some(SceneLinkData {
            scene: AssetId::from_path("content/manor_art/scene.gasset"),
            changes: vec![
                LinkChange::AddComponent {
                    path: "hall/door".to_string(),
                    component: health(),
                },
                LinkChange::RemoveComponent {
                    path: "hall/chair#1".to_string(),
                    type_name: "game::Team".to_string(),
                },
                LinkChange::RemoveNode {
                    path: "helpers".to_string(),
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

#[test]
fn a_version_1_scene_still_decodes() {
    let payload = std::fs::read(encoding_dir().join("Scene.v1.bin")).unwrap();
    let header = ContentAssetHeader {
        asset_id: AssetId::from_path("content/scenes/level.gasset"),
        references: Vec::new(),
        kind: "Scene".to_string(),
        kind_version: 1,
        provenance: None,
        derived: None,
        embedded: Vec::new(),
    };
    let bytes = write_content_asset(&header, &payload).unwrap();

    let (_, scene) = decode_content_asset::<Scene>(&bytes).expect("migrates");

    let expected = version_1_nodes();
    assert_eq!(scene.nodes.len(), expected.len());
    for (node, expected) in scene.nodes.iter().zip(&expected) {
        assert_eq!(node.name, expected.name);
        assert_eq!(node.children, expected.children);
        assert_eq!(node.components, expected.components);
        assert_eq!(node.link, None);
    }
    assert_eq!(
        scene.referenced_assets,
        vec![AssetId::from_path("models/character.gltf#mesh/0")]
    );
}
