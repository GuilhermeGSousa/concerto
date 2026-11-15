//! Covers that the engine's spawnable components go through reflection alone:
//! each spawns from a stored value and captures back to that same value, with
//! handles keeping the `AssetId` the scene referenced.
use std::collections::HashMap;

use concerto_color::{Color, LinearRgba};
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
        camera_environment::{CameraEnvironment, Fog, DEFAULT_AMBIENT_INTENSITY},
        light::Light,
        material::MaterialComponent,
        render_entity::SyncWithRenderWorld,
    },
};

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

struct RoundTrip {
    case: &'static str,
    stored: ReflectedValue,
    captured: ReflectedValue,
    referenced: Vec<AssetId>,
    expected_referenced: Vec<AssetId>,
}

fn round_trip<T: Component + Reflect>(
    case: &'static str,
    value: &T,
    expected_referenced: Vec<AssetId>,
) -> RoundTrip {
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
    RoundTrip {
        case,
        stored,
        captured,
        referenced: capture.into_referenced_assets(),
        expected_referenced,
    }
}

#[test]
fn a_component_spawned_from_its_stored_value_captures_back_to_it_with_its_asset_ids() {
    let mesh = AssetId::from_path("models/character.gltf#mesh/0");
    let material = AssetId::from_path("models/character.gltf#material/0");
    let texture = AssetId::from_path("targets/minimap#texture");
    let mut light = Light::spot_light(30.0);
    light.color = Color::RED;
    light.range = 12.0;
    let cases = [
        round_trip(
            "a mesh",
            &MeshComponent {
                handle: AssetHandle::<Mesh>::weak(mesh),
            },
            vec![mesh],
        ),
        round_trip(
            "a material",
            &MaterialComponent::<StandardMaterial> {
                handle: AssetHandle::weak(material),
            },
            vec![material],
        ),
        round_trip("a spot light", &light, vec![]),
        round_trip(
            "a camera with a texture target",
            &Camera {
                render_target: RenderTarget::texture(AssetHandle::weak(texture)),
                ..Camera::default()
            },
            vec![texture],
        ),
        round_trip(
            "a camera environment with fog and ambient",
            &CameraEnvironment::default()
                .with_fog(Fog {
                    color: Color::BLUE,
                    density: 0.2,
                    start: 4.0,
                })
                .with_ambient(Color::WHITE),
            vec![],
        ),
    ];
    for case in &cases {
        assert_eq!(case.captured, case.stored, "{}", case.case);
        assert_eq!(case.referenced, case.expected_referenced, "{}", case.case);
    }
}

#[test]
fn a_stored_camera_environment_without_ambient_spawns_with_the_default() {
    let ReflectedValue::Struct(mut fields) = to_value(&CameraEnvironment::default()).unwrap()
    else {
        panic!("a camera environment is a struct");
    };
    let ambient = fields
        .iter()
        .position(|(name, _)| name == "ambient")
        .expect("ambient is stored");
    let (_, ambient) = fields.remove(ambient);
    assert!(matches!(ambient, ReflectedValue::Leaf(_)));

    let mut world = world();
    let entity = world.spawn(());
    assert!(world.apply_scene_component(
        std::any::type_name::<CameraEnvironment>(),
        &ReflectedValue::Struct(fields),
        entity,
        &[entity]
    ));

    let spawned = world
        .get_component_for_entity::<CameraEnvironment>(entity)
        .unwrap();
    assert_eq!(
        spawned.ambient_light(),
        LinearRgba::new(
            DEFAULT_AMBIENT_INTENSITY,
            DEFAULT_AMBIENT_INTENSITY,
            DEFAULT_AMBIENT_INTENSITY,
            1.0
        )
    );
}
