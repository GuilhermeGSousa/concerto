use std::path::Path;

use concerto_foundation::assets::content::check_encoding;
use concerto_foundation::assets::AssetId;
use concerto_reflect::ReflectedValue;
use concerto_scene::scene::{Scene, SceneNode, SerializedComponent};

#[test]
fn scene_encoding_matches_its_fixture() {
    let scene = Scene {
        nodes: vec![
            SceneNode {
                name: "root".to_string(),
                children: vec![1],
                components: vec![],
            },
            SceneNode {
                name: "child".to_string(),
                children: vec![],
                components: vec![
                    SerializedComponent {
                        type_name: "game::Health".to_string(),
                        value: ReflectedValue::Struct(vec![
                            ("current".to_string(), ReflectedValue::Leaf(vec![1, 2])),
                            (
                                "items".to_string(),
                                ReflectedValue::List(vec![ReflectedValue::Leaf(vec![3])]),
                            ),
                        ]),
                    },
                    SerializedComponent {
                        type_name: "game::Team".to_string(),
                        value: ReflectedValue::Variant {
                            name: "Red".to_string(),
                            fields: vec![("size".to_string(), ReflectedValue::Leaf(vec![4]))],
                        },
                    },
                ],
            },
        ],
        referenced_assets: vec![AssetId::from_path("models/character.gltf#mesh/0")],
    };

    check_encoding(
        &scene,
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding")),
    );
}
