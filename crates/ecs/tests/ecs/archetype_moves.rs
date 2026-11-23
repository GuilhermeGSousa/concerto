use proptest::prelude::*;

use concerto_ecs::{Component, Entity, World, component::ComponentId};

#[derive(Component, Default)]
struct Armor(u32);

#[derive(Component)]
struct Speed(u32);

#[derive(Component)]
#[require(Armor)]
struct Knight;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Shape {
    armor: Option<u32>,
    speed: Option<u32>,
    knight: bool,
}

#[derive(Clone, Copy, Debug)]
enum Part {
    Armor,
    Speed,
    Knight,
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Spawn(Shape),
    Insert { target: usize, shape: Shape },
    Remove { target: usize, part: Part },
    Despawn { target: usize },
}

struct Live {
    entity: Entity,
    shape: Shape,
}

fn shapes() -> impl Strategy<Value = Shape> {
    (
        proptest::option::of(0u32..100),
        proptest::option::of(0u32..100),
        any::<bool>(),
    )
        .prop_map(|(armor, speed, knight)| Shape {
            armor,
            speed,
            knight,
        })
}

fn parts() -> impl Strategy<Value = Part> {
    prop_oneof![Just(Part::Armor), Just(Part::Speed), Just(Part::Knight)]
}

fn steps() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => shapes().prop_map(Step::Spawn),
        4 => (any::<usize>(), shapes()).prop_map(|(target, shape)| Step::Insert { target, shape }),
        4 => (any::<usize>(), parts()).prop_map(|(target, part)| Step::Remove { target, part }),
        1 => any::<usize>().prop_map(|target| Step::Despawn { target }),
    ]
}

fn spawn(world: &mut World, shape: Shape) -> Entity {
    match (shape.armor, shape.speed, shape.knight) {
        (None, None, false) => world.spawn(()),
        (Some(armor), None, false) => world.spawn(Armor(armor)),
        (None, Some(speed), false) => world.spawn(Speed(speed)),
        (Some(armor), Some(speed), false) => world.spawn((Armor(armor), Speed(speed))),
        (None, None, true) => world.spawn(Knight),
        (Some(armor), None, true) => world.spawn((Knight, Armor(armor))),
        (None, Some(speed), true) => world.spawn((Speed(speed), Knight)),
        (Some(armor), Some(speed), true) => world.spawn((Armor(armor), Speed(speed), Knight)),
    }
}

fn insert(world: &mut World, entity: Entity, shape: Shape) {
    match (shape.armor, shape.speed, shape.knight) {
        (None, None, false) => world.insert((), entity),
        (Some(armor), None, false) => world.insert(Armor(armor), entity),
        (None, Some(speed), false) => world.insert(Speed(speed), entity),
        (Some(armor), Some(speed), false) => world.insert((Armor(armor), Speed(speed)), entity),
        (None, None, true) => world.insert(Knight, entity),
        (Some(armor), None, true) => world.insert((Knight, Armor(armor)), entity),
        (None, Some(speed), true) => world.insert((Speed(speed), Knight), entity),
        (Some(armor), Some(speed), true) => {
            world.insert((Armor(armor), Speed(speed), Knight), entity)
        }
    }
}

fn remove(world: &mut World, entity: Entity, part: Part) {
    match part {
        Part::Armor => world.remove_component::<Armor>(entity),
        Part::Speed => world.remove_component::<Speed>(entity),
        Part::Knight => world.remove_component::<Knight>(entity),
    }
}

fn spawned(shape: Shape) -> Shape {
    Shape {
        armor: shape.armor.or(shape.knight.then_some(0)),
        ..shape
    }
}

fn inserted(held: Shape, shape: Shape) -> Shape {
    let knight = held.knight || shape.knight;
    Shape {
        armor: shape.armor.or(held.armor).or(shape.knight.then_some(0)),
        speed: shape.speed.or(held.speed),
        knight,
    }
}

fn removed(held: Shape, part: Part) -> Shape {
    match part {
        Part::Armor => Shape {
            armor: None,
            ..held
        },
        Part::Speed => Shape {
            speed: None,
            ..held
        },
        Part::Knight => Shape {
            knight: false,
            ..held
        },
    }
}

fn read(world: &World, entity: Entity) -> Shape {
    Shape {
        armor: world
            .get_component_for_entity::<Armor>(entity)
            .map(|armor| armor.0),
        speed: world
            .get_component_for_entity::<Speed>(entity)
            .map(|speed| speed.0),
        knight: world.get_component_for_entity::<Knight>(entity).is_some(),
    }
}

fn members(world: &World, entity: Entity) -> Vec<ComponentId> {
    let mut ids = world.component_ids(entity).to_vec();
    ids.sort();
    ids
}

fn populated(world: &World) -> usize {
    world
        .archetypes()
        .iter()
        .filter(|archetype| !archetype.is_empty())
        .count()
}

fn armor_total(world: &mut World) -> u32 {
    let mut state = world.query::<&Armor, ()>();
    state.iter(world).map(|armor| armor.0).sum()
}

fn play(world: &mut World, steps: &[Step]) -> Vec<Live> {
    let mut live: Vec<Live> = Vec::new();
    for step in steps {
        match *step {
            Step::Spawn(shape) => live.push(Live {
                entity: spawn(world, shape),
                shape: spawned(shape),
            }),
            Step::Insert { target, shape } if !live.is_empty() => {
                let slot = target % live.len();
                insert(world, live[slot].entity, shape);
                live[slot].shape = inserted(live[slot].shape, shape);
            }
            Step::Remove { target, part } if !live.is_empty() => {
                let slot = target % live.len();
                remove(world, live[slot].entity, part);
                live[slot].shape = removed(live[slot].shape, part);
            }
            Step::Despawn { target } if !live.is_empty() => {
                let gone = live.swap_remove(target % live.len());
                world.despawn(gone.entity);
            }
            _ => {}
        }
    }
    live
}

fn fresh(world: &mut World, shape: Shape) -> Entity {
    let entity = spawn(world, shape);
    if shape.knight && shape.armor.is_none() {
        world.remove_component::<Armor>(entity);
    }
    entity
}

proptest! {
    #[test]
    fn a_history_of_moves_ends_where_a_fresh_world_built_in_the_final_shape_does(
        steps in proptest::collection::vec(steps(), 0..80),
    ) {
        let mut moved = World::new();
        let live = play(&mut moved, &steps);

        let mut built = World::new();
        for held in &live {
            let twin = fresh(&mut built, held.shape);

            prop_assert_eq!(read(&moved, held.entity), held.shape);
            prop_assert_eq!(read(&built, twin), held.shape);
            prop_assert_eq!(members(&moved, held.entity), members(&built, twin));
        }
        prop_assert_eq!(populated(&moved), populated(&built));
        prop_assert_eq!(armor_total(&mut moved), armor_total(&mut built));
    }
}
