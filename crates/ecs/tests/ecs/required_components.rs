use concerto_ecs::{Component, World, reflect::ReflectSceneComponent};
use concerto_reflect::Reflect;

#[derive(Component, Default)]
struct Base;

#[derive(Component, Default)]
struct Other(#[allow(dead_code)] u32);

#[derive(Component)]
#[require(Base)]
struct Single;

#[derive(Component)]
#[require(Base, Other = Other(7))]
#[require(Marker<u8>)]
struct Many;

#[derive(Component, Default)]
struct Marker<T: Send + Sync + 'static>(std::marker::PhantomData<T>);

#[derive(Component)]
#[require(Marker<T>)]
struct Generic<T: Send + Sync + Default + 'static>(T);

#[derive(Component, Reflect, Default)]
#[require(Base)]
#[reflect(SceneComponent)]
struct Reflected {
    value: u32,
}

#[test]
fn derive_accepts_require_attributes() {
    let mut world = World::default();
    world.spawn(Single);
    world.spawn(Many);
    world.spawn(Generic(1u8));
    world.spawn(Reflected { value: 1 });
}
