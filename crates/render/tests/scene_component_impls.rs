//! Covers that the engine's spawnable components go through reflection alone:
//! each spawns from a stored value and captures back to that same value, with
//! handles keeping the `AssetId` the scene referenced.
use std::collections::HashMap;

use concerto_color::Color;
use concerto_ecs::{component::Component, reflect::SceneCapture, World};
use concerto_foundation::{
    assets::{handle::AssetHandle, AssetId},
    transform::Transform,
};
use concerto_mesh::mesh::{Mesh, MeshComponent};
use concerto_reflect::{to_value, Reflect, ReflectedValue};
use concerto_render::{
    assets::material::StandardMaterial,
    components::{
        camera::{Camera, RenderTarget},
        camera_environment::{CameraEnvironment, Fog},
        light::{Light, LightType},
        material::MaterialComponent,
        render_entity::SyncWithRenderWorld,
    },
};
use glam::Vec3;

fn world() -> World {
    let mut world = World::new();
    world.register_type::<Transform>();
    world.register_type::<MeshComponent>();
    world.register_type::<MaterialComponent>();
    world.register_type::<Camera>();
    world.register_type::<CameraEnvironment>();
    world.register_type::<Light>();
    world.register_type::<SyncWithRenderWorld>();
    world
}

/// Spawns `value` from its stored form and captures it back, returning both.
fn round_trip<T: Component + Reflect>(value: &T) -> (ReflectedValue, ReflectedValue, Vec<AssetId>) {
    let mut world = world();
    let stored = to_value(value).unwrap();
    let entity = world.spawn(());
    let name = std::any::type_name::<T>();
    assert!(world.apply_scene_component(name, &stored, entity, &[entity]));

    let mut capture = SceneCapture::new(HashMap::from([(entity, 0)]));
    let captured = world
        .component_types(entity)
        .find(|component| component.name() == name)
        .expect("the spawned component is listed")
        .capture(&world, entity, &mut capture)
        .unwrap();
    (stored, captured, capture.into_referenced_assets())
}

#[test]
fn a_mesh_and_its_material_keep_their_asset_ids() {
    let mesh = AssetId::from_path("models/character.gltf#mesh/0");
    let material = AssetId::from_path("models/character.gltf#material/0");

    let (stored, captured, referenced) = round_trip(&MeshComponent {
        handle: AssetHandle::<Mesh>::weak(mesh),
    });
    assert_eq!(captured, stored);
    assert_eq!(referenced, [mesh]);

    let (stored, captured, referenced) = round_trip(&MaterialComponent::<StandardMaterial> {
        handle: AssetHandle::weak(material),
    });
    assert_eq!(captured, stored);
    assert_eq!(referenced, [material]);
}

#[test]
fn a_spot_light_keeps_its_cone() {
    let mut light = Light::spot_light(30.0);
    light.color = Color::RED;
    light.range = 12.0;
    let (stored, captured, _) = round_trip(&light);
    assert_eq!(captured, stored);
    let ReflectedValue::Struct(fields) = &captured else {
        panic!("a light is a struct");
    };
    let (_, light_type) = fields
        .iter()
        .find(|(name, _)| name == "light_type")
        .unwrap();
    assert!(matches!(light_type, ReflectedValue::Variant { name, .. } if name == "Spot"));
    assert!(matches!(LightType::default(), LightType::Point));
}

#[test]
fn a_camera_keeps_its_texture_target() {
    let texture = AssetId::from_path("targets/minimap#texture");
    let camera = Camera {
        render_target: RenderTarget::texture(AssetHandle::weak(texture)),
        ..Camera::default()
    };
    let (stored, captured, referenced) = round_trip(&camera);
    assert_eq!(captured, stored);
    assert_eq!(referenced, [texture]);
}

#[test]
fn a_camera_environment_keeps_its_fog_and_ambient() {
    let environment = CameraEnvironment::default()
        .with_fog(Fog {
            color: Color::BLUE,
            density: 0.2,
            start: 4.0,
        })
        .with_ambient(Color::WHITE);
    let (stored, captured, _) = round_trip(&environment);
    assert_eq!(captured, stored);
}

#[test]
fn a_transform_and_a_marker_round_trip() {
    let mut transform = Transform::IDENTITY;
    transform.translation = Vec3::new(1.0, 2.0, 3.0);
    let (stored, captured, _) = round_trip(&transform);
    assert_eq!(captured, stored);

    let (stored, captured, _) = round_trip(&SyncWithRenderWorld);
    assert_eq!(captured, stored);
}
