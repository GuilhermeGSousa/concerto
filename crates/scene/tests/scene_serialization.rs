//! Covers Scene/SceneNode round-tripping directly through bincode and
//! reporting the assets its component payloads reference for import-time
//! validation.
use concerto_ecs::component::Component;
use concerto_foundation::assets::{handle::AssetHandle, Asset, AssetId};
use concerto_mesh::mesh::MeshComponent;
use concerto_render::assets::material::StandardMaterial;
use concerto_render::components::material::MaterialComponent;
use concerto_scene::scene::{Scene, SceneNode, SerializedComponent};

fn component<T: serde::Serialize + Component>(value: &T) -> SerializedComponent {
    SerializedComponent {
        type_name: T::name().to_string(),
        data: serde_json::to_string(value).unwrap(),
    }
}

#[test]
fn round_trips_and_reports_references() {
    let mesh_id = AssetId::from_path("models/character.gltf#mesh/0");
    let material_id = AssetId::from_path("models/character.gltf#material/0");

    let sc = Scene {
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
                    component(&MeshComponent {
                        handle: AssetHandle::weak(mesh_id),
                    }),
                    component(&MaterialComponent::<StandardMaterial> {
                        handle: AssetHandle::weak(material_id),
                    }),
                ],
            },
        ],
        referenced_assets: vec![mesh_id, material_id],
    };

    let bytes = bincode::serialize(&sc).expect("Scene must serialize through bincode");
    let decoded: Scene =
        bincode::deserialize(&bytes).expect("Scene must deserialize through bincode");

    assert_eq!(
        decoded.nodes.len(),
        2,
        "both nodes must survive the round-trip"
    );
    assert_eq!(
        decoded.nodes[0].children,
        vec![1],
        "child indices are plain data and must round-trip verbatim"
    );

    let mesh_payload = decoded.nodes[1]
        .components
        .iter()
        .find(|c| c.type_name == MeshComponent::name())
        .expect("the child node must carry a MeshComponent payload");
    let decoded_mesh: MeshComponent = serde_json::from_str(&mesh_payload.data).unwrap();
    assert_eq!(
        decoded_mesh.handle.id(),
        mesh_id,
        "a node's mesh handle must deserialize back to the same AssetId"
    );

    assert_eq!(
        sc.referenced_sub_assets(),
        vec![mesh_id, material_id],
        "referenced_sub_assets must report every AssetId the scene's components use"
    );
}
