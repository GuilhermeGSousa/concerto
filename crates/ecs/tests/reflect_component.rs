use std::any::TypeId;

use concerto_ecs::{
    Component, Resource, World,
    reflect::{AppTypeRegistry, ReflectComponent},
    system::input::ComponentMetadata,
};
use concerto_reflect::{Reflect, ReflectDefault};

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
struct Health {
    points: f32,
}

#[derive(Component, Reflect)]
#[reflect(Component)]
struct Lens {
    length: f32,
}

fn component<T: 'static>(world: &World) -> ReflectComponent {
    *world
        .get_resource::<AppTypeRegistry>()
        .unwrap()
        .get_type_data::<ReflectComponent>(TypeId::of::<T>())
        .unwrap()
}

#[test]
fn a_new_world_registers_only_scene_entity_references() {
    let world = World::new();

    let registry = world.get_resource::<AppTypeRegistry>().unwrap();
    assert!(
        registry
            .get(TypeId::of::<concerto_ecs::component::scene::SceneEntityRef>())
            .is_some()
    );
    assert_eq!(registry.iter().count(), 1);
}

#[test]
fn a_reflected_component_is_read_edited_inserted_at_default_and_removed() {
    let mut world = World::new();
    world.register_type::<Health>();
    let reflect = component::<Health>(&world);
    let entity = world.spawn(());

    assert!((reflect.read)(&ComponentMetadata::new(&world), entity).is_none());
    assert!(!(reflect.edit)(&mut world, entity, &mut |_, _| {}));

    assert!(reflect.insert_default(TypeId::of::<Health>(), &mut world, entity));
    assert!((reflect.read)(&ComponentMetadata::new(&world), entity).is_some());

    let mut visited = false;
    assert!((reflect.edit)(&mut world, entity, &mut |_, _| visited = true));
    assert!(visited);

    (reflect.remove)(&mut world, entity);
    assert!((reflect.read)(&ComponentMetadata::new(&world), entity).is_none());
}

#[derive(Resource)]
struct Gravity(f32);

#[test]
fn an_edit_reads_the_worlds_resources_beside_the_component() {
    let mut world = World::new();
    world.register_type::<Health>();
    world.insert_resource(Gravity(9.8));
    let reflect = component::<Health>(&world);
    let entity = world.spawn(Health::default());

    let mut gravity = None;
    assert!((reflect.edit)(&mut world, entity, &mut |_, resources| {
        gravity = resources.get::<Gravity>().map(|gravity| gravity.0);
    }));

    assert_eq!(gravity, Some(9.8));
}

#[test]
fn a_component_without_default_cannot_be_inserted() {
    let mut world = World::new();
    world.register_type::<Lens>();
    let reflect = component::<Lens>(&world);
    let entity = world.spawn(());

    assert!(!reflect.insert_default(TypeId::of::<Lens>(), &mut world, entity));
    assert!(world.get_component_for_entity::<Lens>(entity).is_none());
    let _ = Lens { length: 1.0 }.length;
}
