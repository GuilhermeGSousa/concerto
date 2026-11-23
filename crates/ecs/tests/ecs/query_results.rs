use proptest::prelude::*;

use concerto_ecs::{
    Added, Changed, Component, Entity, IntoSystem, Or, Query, Ref, Res, Resource, System, With,
    Without, World,
    query::{QueryData, change_detection::DetectChanges, filter::Not, filter::QueryFilter},
};

#[derive(Component)]
struct Health(u32);

#[derive(Component)]
struct Shield(u32);

#[derive(Component)]
struct Marked;

#[derive(Resource)]
struct Everyone(Vec<Entity>);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Cell {
    value: u32,
    added: bool,
    changed: bool,
}

impl Cell {
    fn fresh(value: u32) -> Cell {
        Cell {
            value,
            added: true,
            changed: true,
        }
    }

    fn written(self, value: u32) -> Cell {
        Cell {
            value,
            added: self.added,
            changed: true,
        }
    }

    fn settled(self) -> Cell {
        Cell {
            value: self.value,
            added: false,
            changed: false,
        }
    }
}

fn placed(held: Option<Cell>, value: u32) -> Cell {
    held.map_or(Cell::fresh(value), |cell| cell.written(value))
}

fn is_changed(cell: Option<Cell>) -> bool {
    cell.is_some_and(|cell| cell.changed)
}

fn is_added(cell: Option<Cell>) -> bool {
    cell.is_some_and(|cell| cell.added)
}

fn value(cell: Option<Cell>) -> Option<u32> {
    cell.map(|cell| cell.value)
}

#[derive(Clone, Copy, Debug)]
struct Body {
    entity: Entity,
    health: Option<Cell>,
    shield: Option<Cell>,
    marked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Seen {
    entity: Entity,
    health: Option<u32>,
    shield: Option<u32>,
    changed: Option<bool>,
    added: Option<bool>,
}

impl Seen {
    fn entity(entity: Entity) -> Seen {
        Seen {
            entity,
            health: None,
            shield: None,
            changed: None,
            added: None,
        }
    }

    fn values(entity: Entity, health: Option<u32>, shield: Option<u32>) -> Seen {
        Seen {
            health,
            shield,
            ..Seen::entity(entity)
        }
    }

    fn stamped(self, stamps: &impl DetectChanges) -> Seen {
        Seen {
            changed: Some(stamps.is_changed()),
            added: Some(stamps.is_added()),
            ..self
        }
    }

    fn of(body: &Body) -> Seen {
        Seen::values(body.entity, value(body.health), value(body.shield))
    }

    fn with_stamps_of(self, cell: Option<Cell>) -> Seen {
        Seen {
            changed: Some(is_changed(cell)),
            added: Some(is_added(cell)),
            ..self
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Shape {
    health: Option<u32>,
    shield: Option<u32>,
    marked: bool,
}

#[derive(Clone, Copy, Debug)]
enum Part {
    Health,
    Shield,
    Marked,
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Spawn(Shape),
    Insert {
        target: usize,
        part: Part,
        value: u32,
    },
    Remove {
        target: usize,
        part: Part,
    },
    Write {
        target: usize,
        part: Part,
        value: u32,
    },
    Despawn {
        target: usize,
    },
    HealMarked,
    Tick,
    LookUp,
}

fn shapes() -> impl Strategy<Value = Shape> {
    (
        proptest::option::of(0u32..100),
        proptest::option::of(0u32..100),
        any::<bool>(),
    )
        .prop_map(|(health, shield, marked)| Shape {
            health,
            shield,
            marked,
        })
}

fn parts() -> impl Strategy<Value = Part> {
    prop_oneof![Just(Part::Health), Just(Part::Shield), Just(Part::Marked)]
}

fn steps() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => shapes().prop_map(Step::Spawn),
        3 => (any::<usize>(), parts(), 0u32..100)
            .prop_map(|(target, part, value)| Step::Insert { target, part, value }),
        2 => (any::<usize>(), parts()).prop_map(|(target, part)| Step::Remove { target, part }),
        3 => (any::<usize>(), parts(), 0u32..100)
            .prop_map(|(target, part, value)| Step::Write { target, part, value }),
        1 => any::<usize>().prop_map(|target| Step::Despawn { target }),
        1 => Just(Step::HealMarked),
        2 => Just(Step::Tick),
        1 => Just(Step::LookUp),
    ]
}

fn spawn(world: &mut World, shape: Shape) -> Body {
    let entity = world.spawn(());
    let mut body = Body {
        entity,
        health: None,
        shield: None,
        marked: false,
    };
    if let Some(health) = shape.health {
        insert(world, &mut body, Part::Health, health);
    }
    if let Some(shield) = shape.shield {
        insert(world, &mut body, Part::Shield, shield);
    }
    if shape.marked {
        insert(world, &mut body, Part::Marked, 0);
    }
    body
}

fn insert(world: &mut World, body: &mut Body, part: Part, value: u32) {
    match part {
        Part::Health => {
            world.insert(Health(value), body.entity);
            body.health = Some(placed(body.health, value));
        }
        Part::Shield => {
            world.insert(Shield(value), body.entity);
            body.shield = Some(placed(body.shield, value));
        }
        Part::Marked => {
            world.insert(Marked, body.entity);
            body.marked = true;
        }
    }
}

fn remove(world: &mut World, body: &mut Body, part: Part) {
    match part {
        Part::Health => {
            world.remove_component::<Health>(body.entity);
            body.health = None;
        }
        Part::Shield => {
            world.remove_component::<Shield>(body.entity);
            body.shield = None;
        }
        Part::Marked => {
            world.remove_component::<Marked>(body.entity);
            body.marked = false;
        }
    }
}

fn write(world: &mut World, body: &mut Body, part: Part, value: u32) {
    match part {
        Part::Health => {
            if let Some(health) = world.get_component_for_entity_mut::<Health>(body.entity) {
                health.0 = value;
            }
            body.health = body.health.map(|cell| cell.written(value));
        }
        Part::Shield => {
            if let Some(shield) = world.get_component_for_entity_mut::<Shield>(body.entity) {
                shield.0 = value;
            }
            body.shield = body.shield.map(|cell| cell.written(value));
        }
        Part::Marked => {}
    }
}

fn heal_marked(mut marked: Query<&mut Health, With<Marked>>) {
    for mut health in marked.iter_mut() {
        health.0 += 1;
    }
}

fn healed(body: Body) -> Body {
    Body {
        health: body
            .health
            .filter(|_| body.marked)
            .map(|cell| cell.written(cell.value + 1))
            .or(body.health),
        ..body
    }
}

fn settled(body: Body) -> Body {
    Body {
        health: body.health.map(Cell::settled),
        shield: body.shield.map(Cell::settled),
        ..body
    }
}

fn lookups_agree(
    everyone: &[Entity],
    iterated: &[Seen],
    looked_up: impl Fn(Entity) -> Option<Seen>,
    contains: impl Fn(Entity) -> bool,
) {
    for &entity in everyone {
        let expected = iterated.iter().copied().find(|seen| seen.entity == entity);
        assert_eq!(looked_up(entity), expected);
        assert_eq!(contains(entity), expected.is_some());
    }
}

fn look_up_reads(
    everyone: Res<Everyone>,
    marked: Query<(Entity, &Health, Option<&Shield>), With<Marked>>,
    recent: Query<(Entity, Ref<Shield>), Or<(With<Marked>, Changed<Shield>)>>,
    bare: Query<Entity, (With<Health>, Without<Shield>)>,
) {
    let iterated: Vec<Seen> = marked
        .iter()
        .map(|(entity, health, shield)| {
            Seen::values(entity, Some(health.0), shield.map(|shield| shield.0))
        })
        .collect();
    lookups_agree(
        &everyone.0,
        &iterated,
        |entity| {
            marked.get_entity(entity).map(|(entity, health, shield)| {
                Seen::values(entity, Some(health.0), shield.map(|shield| shield.0))
            })
        },
        |entity| marked.contains_entity(entity),
    );

    let iterated: Vec<Seen> = recent
        .iter()
        .map(|(entity, shield)| Seen::values(entity, None, Some(shield.0)).stamped(&shield))
        .collect();
    lookups_agree(
        &everyone.0,
        &iterated,
        |entity| {
            recent
                .get_entity(entity)
                .map(|(entity, shield)| Seen::values(entity, None, Some(shield.0)).stamped(&shield))
        },
        |entity| recent.contains_entity(entity),
    );

    let iterated: Vec<Seen> = bare.iter().map(Seen::entity).collect();
    lookups_agree(
        &everyone.0,
        &iterated,
        |entity| bare.get_entity(entity).map(Seen::entity),
        |entity| bare.contains_entity(entity),
    );
}

fn look_up_writes(
    everyone: Res<Everyone>,
    mut changed: Query<(Entity, &mut Health, Option<&mut Shield>), Changed<Health>>,
) {
    let iterated: Vec<Seen> = changed
        .iter_mut()
        .map(|(entity, health, shield)| {
            Seen::values(entity, Some(health.0), shield.map(|shield| shield.0)).stamped(&health)
        })
        .collect();
    for &entity in &everyone.0 {
        let expected = iterated.iter().copied().find(|seen| seen.entity == entity);
        let found = changed
            .get_entity_mut(entity)
            .map(|(entity, health, shield)| {
                Seen::values(entity, Some(health.0), shield.map(|shield| shield.0)).stamped(&health)
            });
        assert_eq!(found, expected);
        assert_eq!(changed.contains_entity(entity), expected.is_some());
    }
}

struct Lookups {
    reads: Box<dyn System<In = ()>>,
    writes: Box<dyn System<In = ()>>,
    heal: Box<dyn System<In = ()>>,
}

fn started<M>(world: &mut World, system: impl IntoSystem<(), M>) -> Box<dyn System<In = ()>> {
    let mut system = system.into_system();
    system.initialize(world);
    Box::new(system)
}

impl Lookups {
    fn new(world: &mut World) -> Lookups {
        world.insert_resource(Everyone(Vec::new()));
        let mut lookups = Lookups {
            reads: started(world, look_up_reads),
            writes: started(world, look_up_writes),
            heal: started(world, heal_marked),
        };
        lookups.run(world, &[]);
        lookups
    }

    fn run(&mut self, world: &mut World, everyone: &[Entity]) {
        world.insert_resource(Everyone(everyone.to_vec()));
        self.reads.run_and_apply((), world);
        self.writes.run_and_apply((), world);
    }
}

struct Played {
    live: Vec<Body>,
    everyone: Vec<Entity>,
}

fn play(world: &mut World, lookups: &mut Lookups, steps: &[Step]) -> Played {
    let mut live: Vec<Body> = Vec::new();
    let mut everyone = Vec::new();
    for step in steps {
        match *step {
            Step::Spawn(shape) => {
                let body = spawn(world, shape);
                everyone.push(body.entity);
                live.push(body);
            }
            Step::Insert {
                target,
                part,
                value,
            } if !live.is_empty() => {
                let slot = target % live.len();
                insert(world, &mut live[slot], part, value);
            }
            Step::Remove { target, part } if !live.is_empty() => {
                let slot = target % live.len();
                remove(world, &mut live[slot], part);
            }
            Step::Write {
                target,
                part,
                value,
            } if !live.is_empty() => {
                let slot = target % live.len();
                write(world, &mut live[slot], part, value);
            }
            Step::Despawn { target } if !live.is_empty() => {
                let gone = live.swap_remove(target % live.len());
                world.despawn(gone.entity);
            }
            Step::HealMarked => {
                lookups.heal.run_and_apply((), world);
                live = live.into_iter().map(healed).collect();
            }
            Step::Tick => {
                world.tick();
                live = live.into_iter().map(settled).collect();
            }
            Step::LookUp => lookups.run(world, &everyone),
            _ => {}
        }
    }
    Played { live, everyone }
}

fn expected(
    world: &World,
    live: &[Body],
    passes: impl Fn(&Body) -> bool,
    shown: impl Fn(&Body) -> Seen,
) -> Vec<Seen> {
    world
        .archetypes()
        .iter()
        .flat_map(|archetype| archetype.entities())
        .filter_map(|entity| live.iter().find(|body| body.entity == *entity))
        .filter(|body| passes(body))
        .map(shown)
        .collect()
}

fn yielded<D: QueryData, F: QueryFilter>(
    world: &mut World,
    shown: impl for<'a> Fn(D::Item<'a>) -> Seen,
) -> Vec<Seen> {
    world.query::<D, F>().iter(world).map(shown).collect()
}

fn only_entity(body: &Body) -> Seen {
    Seen::entity(body.entity)
}

proptest! {
    #[test]
    fn a_query_yields_what_the_entities_hold_in_archetype_and_row_order(
        steps in proptest::collection::vec(steps(), 0..60),
    ) {
        let mut world = World::new();
        let mut lookups = Lookups::new(&mut world);
        let Played { live, everyone } = play(&mut world, &mut lookups, &steps);
        lookups.run(&mut world, &everyone);
        let live = &live[..];

        prop_assert_eq!(
            yielded::<(Entity, &Health, Option<&Shield>), With<Marked>>(
                &mut world,
                |(entity, health, shield)| {
                    Seen::values(entity, Some(health.0), shield.map(|shield| shield.0))
                },
            ),
            expected(&world, live, |body| body.health.is_some() && body.marked, Seen::of)
        );
        prop_assert_eq!(
            yielded::<(Entity, Option<&Health>, Ref<Shield>), Without<Marked>>(
                &mut world,
                |(entity, health, shield)| {
                    Seen::values(entity, health.map(|health| health.0), Some(shield.0))
                        .stamped(&shield)
                },
            ),
            expected(
                &world,
                live,
                |body| body.shield.is_some() && !body.marked,
                |body| Seen::of(body).with_stamps_of(body.shield),
            )
        );
        prop_assert_eq!(
            yielded::<(Entity, &mut Health, Option<&mut Shield>), ()>(
                &mut world,
                |(entity, health, shield)| {
                    Seen::values(entity, Some(health.0), shield.map(|shield| shield.0))
                        .stamped(&health)
                },
            ),
            expected(
                &world,
                live,
                |body| body.health.is_some(),
                |body| Seen::of(body).with_stamps_of(body.health),
            )
        );
        prop_assert_eq!(
            yielded::<(Entity, &Health), Changed<Health>>(&mut world, |(entity, health)| {
                Seen::values(entity, Some(health.0), None)
            }),
            expected(
                &world,
                live,
                |body| is_changed(body.health),
                |body| Seen::values(body.entity, value(body.health), None),
            )
        );
        prop_assert_eq!(
            yielded::<Entity, Added<Shield>>(&mut world, Seen::entity),
            expected(&world, live, |body| is_added(body.shield), only_entity)
        );
        prop_assert_eq!(
            yielded::<Entity, Or<(With<Marked>, Changed<Shield>)>>(&mut world, Seen::entity),
            expected(
                &world,
                live,
                |body| body.marked || is_changed(body.shield),
                only_entity,
            )
        );
        prop_assert_eq!(
            yielded::<Entity, (With<Health>, Without<Shield>)>(&mut world, Seen::entity),
            expected(
                &world,
                live,
                |body| body.health.is_some() && body.shield.is_none(),
                only_entity,
            )
        );
        prop_assert_eq!(
            yielded::<Entity, Not<Or<(With<Health>, With<Marked>)>>>(&mut world, Seen::entity),
            expected(
                &world,
                live,
                |body| body.health.is_none() && !body.marked,
                only_entity,
            )
        );
        prop_assert_eq!(
            yielded::<Entity, Or<(Without<Health>, Added<Shield>)>>(&mut world, Seen::entity),
            expected(
                &world,
                live,
                |body| body.health.is_none() || is_added(body.shield),
                only_entity,
            )
        );
    }
}
