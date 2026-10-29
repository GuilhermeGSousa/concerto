use concerto_ecs::{Component, Entity, World};

#[derive(Component, Default)]
struct Base;

#[derive(Component, Default)]
struct Extra;

#[derive(Component, Default)]
struct Leaf;

#[derive(Component, Default)]
struct Value(u32);

#[derive(Component, Default)]
#[require(Leaf)]
struct Own;

#[derive(Component, Default)]
#[require(Base)]
struct Requirer;

#[derive(Component, Default)]
#[require(Requirer)]
struct Deeper;

#[derive(Component, Default)]
#[require(Value = Value(1))]
struct Declares;

fn has<T: Component>(world: &World, entity: Entity) -> bool {
    world.get_component_for_entity::<T>(entity).is_some()
}

fn value(world: &World, entity: Entity) -> Option<u32> {
    world
        .get_component_for_entity::<Value>(entity)
        .map(|value| value.0)
}

#[test]
fn external_registration_adds_a_requirement() {
    let mut world = World::new();
    world.register_required_components::<Base, Extra>();

    let entity = world.spawn(Base);

    assert!(has::<Extra>(&world, entity));
}

#[test]
fn external_registration_merges_with_the_types_own() {
    let mut world = World::new();
    world.register_required_components::<Own, Extra>();

    let entity = world.spawn(Own);

    assert!(has::<Leaf>(&world, entity));
    assert!(has::<Extra>(&world, entity));
}

#[test]
fn external_registration_uses_the_given_constructor() {
    let mut world = World::new();
    world.register_required_components_with::<Base, Value>(|| Value(4));

    let entity = world.spawn(Base);

    assert_eq!(value(&world, entity), Some(4));
}

#[test]
fn external_registration_reaches_types_built_before_it() {
    let mut world = World::new();
    world.register_component::<Deeper>();
    world.register_required_components::<Base, Extra>();

    let entity = world.spawn(Deeper);

    assert!(has::<Extra>(&world, entity));
    assert!(has::<Base>(&world, entity));
    assert!(has::<Requirer>(&world, entity));
}

#[test]
fn external_registration_reaches_types_built_after_it() {
    let mut world = World::new();
    world.register_required_components::<Base, Extra>();

    let entity = world.spawn(Deeper);

    assert!(has::<Extra>(&world, entity));
}

#[test]
fn external_registration_is_allowed_while_the_archetype_is_empty() {
    let mut world = World::new();
    let entity = world.spawn(Base);
    world.despawn(entity);

    world.register_required_components::<Base, Extra>();

    let entity = world.spawn(Base);
    assert!(has::<Extra>(&world, entity));
}

#[test]
#[should_panic(expected = "Base")]
fn external_registration_panics_when_an_entity_holds_the_type() {
    let mut world = World::new();
    world.spawn(Base);

    world.register_required_components::<Base, Extra>();
}

#[test]
#[should_panic(expected = "Base")]
fn external_registration_panics_when_an_entity_holds_a_requirer() {
    let mut world = World::new();
    world.spawn(Requirer);

    world.register_required_components::<Base, Extra>();
}

#[test]
#[should_panic(expected = "Base")]
fn external_registration_panics_on_a_cycle() {
    let mut world = World::new();
    world.register_required_components::<Base, Extra>();

    world.register_required_components::<Extra, Base>();
}

#[test]
#[should_panic(expected = "Base")]
fn external_registration_panics_on_a_self_requirement() {
    let mut world = World::new();

    world.register_required_components::<Base, Base>();
}

#[test]
fn a_declared_requirement_wins_over_an_external_one() {
    let mut world = World::new();
    world.register_component::<Declares>();
    world.register_required_components_with::<Declares, Value>(|| Value(5));

    let entity = world.spawn(Declares);

    assert_eq!(value(&world, entity), Some(1));
}

#[test]
fn the_later_external_registration_wins() {
    let mut world = World::new();
    world.register_required_components_with::<Base, Value>(|| Value(2));
    world.register_required_components_with::<Base, Value>(|| Value(3));

    let entity = world.spawn(Base);

    assert_eq!(value(&world, entity), Some(3));
}

fn make_value() -> Value {
    Value(7)
}

#[test]
fn identical_registration_is_a_no_op_while_an_entity_holds_the_type() {
    let mut world = World::new();
    world.register_required_components_with::<Base, Value>(make_value);
    let entity = world.spawn(Base);

    world.register_required_components_with::<Base, Value>(make_value);

    assert_eq!(value(&world, entity), Some(7));
}

#[test]
#[should_panic(expected = "Base")]
fn different_constructor_still_panics_while_an_entity_holds_the_type() {
    let mut world = World::new();
    world.register_required_components_with::<Base, Value>(make_value);
    world.spawn(Base);

    world.register_required_components_with::<Base, Value>(|| Value(8));
}
