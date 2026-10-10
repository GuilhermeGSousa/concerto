//! Covers `All<&dyn Trait>` queries: which entities match, what each yields, registration
//! timing, and how a system holding one is scheduled.

use concerto_ecs::{
    All, Component, Entity, IntoSystem, Query, ResMut, Resource, Schedule, System, With, World,
    queryable,
    system::{
        access::SystemAccess, executor::single_thread::SingleThreadedExecutor, meta::SystemMetadata,
    },
};

trait Named {
    fn name(&self) -> String;
}

#[derive(Component)]
struct Door;

#[derive(Component)]
struct Lever(u32);

#[derive(Component)]
struct Plain;

impl Named for Door {
    fn name(&self) -> String {
        "door".into()
    }
}

impl Named for Lever {
    fn name(&self) -> String {
        format!("lever {}", self.0)
    }
}

macro_rules! named_trait {
    ($trait:ident) => {
        #[queryable]
        trait $trait: Named {}
        impl $trait for Door {}
        impl $trait for Lever {}
    };
}

fn names<'a, T: Named + ?Sized + 'a>(items: impl Iterator<Item = &'a T>) -> Vec<String> {
    items.map(Named::name).collect()
}

#[test]
fn yields_every_implementor_on_an_entity() {
    named_trait!(Every);
    let mut world = World::new();
    world.register_component_as::<dyn Every, Lever>();
    world.register_component_as::<dyn Every, Door>();
    world.spawn((Door, Lever(1)));

    let mut query = world.query::<All<&dyn Every>, ()>();
    let mut rows: Vec<Vec<String>> = query.iter(&mut world).map(names).collect();
    rows.iter_mut().for_each(|row| row.sort());

    assert_eq!(rows, vec![vec!["door".to_string(), "lever 1".to_string()]]);
}

#[test]
fn skips_entities_without_an_implementor() {
    named_trait!(Skips);
    let mut world = World::new();
    world.register_component_as::<dyn Skips, Door>();
    let door = world.spawn(Door);
    let plain = world.spawn(Plain);

    let mut query = world.query::<(Entity, All<&dyn Skips>), ()>();
    let matched: Vec<Entity> = query.iter(&mut world).map(|(entity, _)| entity).collect();
    assert_eq!(matched, vec![door]);

    let mut query = (move |query: Query<All<&dyn Skips>>| {
        assert!(query.contains_entity(door));
        assert!(!query.contains_entity(plain));
    })
    .into_system();
    query.initialize(&mut world);
    query.run_and_apply((), &mut world);
}

#[test]
fn ignores_implementors_that_were_never_registered() {
    named_trait!(Unregistered);
    let mut world = World::new();
    world.register_component_as::<dyn Unregistered, Door>();
    world.spawn((Door, Lever(2)));
    world.spawn(Lever(3));

    let mut query = world.query::<All<&dyn Unregistered>, ()>();
    let rows: Vec<Vec<String>> = query.iter(&mut world).map(names).collect();

    assert_eq!(rows, vec![vec!["door".to_string()]]);
}

#[test]
fn registering_the_same_implementor_twice_yields_it_once() {
    named_trait!(Twice);
    let mut world = World::new();
    world.register_component_as::<dyn Twice, Door>();
    world.register_component_as::<dyn Twice, Door>();
    world.spawn(Door);

    let mut query = world.query::<All<&dyn Twice>, ()>();
    assert_eq!(query.iter(&mut world).next().unwrap().count(), 1);
}

#[test]
fn picks_up_implementors_registered_after_the_query_was_created() {
    named_trait!(Late);
    let mut world = World::new();
    world.register_component_as::<dyn Late, Door>();
    world.spawn(Lever(4));
    world.spawn(Door);

    let mut query = world.query::<All<&dyn Late>, ()>();
    assert_eq!(query.iter(&mut world).count(), 1);

    world.register_component_as::<dyn Late, Lever>();
    let mut rows: Vec<Vec<String>> = query.iter(&mut world).map(names).collect();
    rows.sort();

    assert_eq!(
        rows,
        vec![vec!["door".to_string()], vec!["lever 4".to_string()]]
    );
}

#[test]
fn implementors_are_registered_per_world() {
    named_trait!(PerWorld);
    let mut registered = World::new();
    let mut unregistered = World::new();
    registered.register_component_as::<dyn PerWorld, Door>();
    registered.spawn(Door);
    unregistered.spawn(Door);

    let mut query = registered.query::<All<&dyn PerWorld>, ()>();
    assert_eq!(query.iter(&mut registered).count(), 1);

    let mut query = unregistered.query::<All<&dyn PerWorld>, ()>();
    assert_eq!(query.iter(&mut unregistered).count(), 0);
}

#[test]
fn combines_with_other_query_data_and_filters() {
    named_trait!(Combined);
    let mut world = World::new();
    world.register_component_as::<dyn Combined, Door>();
    world.register_component_as::<dyn Combined, Lever>();
    world.spawn((Door, Plain));
    world.spawn(Door);
    world.spawn((Lever(5), Plain));

    let mut query = world.query::<(Option<&Lever>, All<&dyn Combined>), With<Plain>>();
    let mut rows: Vec<(Option<u32>, Vec<String>)> = query
        .iter(&mut world)
        .map(|(lever, items)| (lever.map(|lever| lever.0), names(items)))
        .collect();
    rows.sort();

    assert_eq!(
        rows,
        vec![
            (None, vec!["door".to_string()]),
            (Some(5), vec!["lever 5".to_string()]),
        ]
    );
}

#[test]
fn zero_sized_implementors_are_yielded() {
    #[queryable]
    trait Marker {
        fn id(&self) -> u32;
    }

    #[derive(Component)]
    struct First;
    #[derive(Component)]
    struct Second;

    impl Marker for First {
        fn id(&self) -> u32 {
            1
        }
    }
    impl Marker for Second {
        fn id(&self) -> u32 {
            2
        }
    }

    let mut world = World::new();
    world.register_component_as::<dyn Marker, First>();
    world.register_component_as::<dyn Marker, Second>();
    world.spawn((First, Second));

    let mut query = world.query::<All<&dyn Marker>, ()>();
    let ids: Vec<Vec<u32>> = query
        .iter(&mut world)
        .map(|items| items.map(Marker::id).collect())
        .collect();

    assert_eq!(ids, vec![vec![1, 2]]);
}

#[test]
fn generic_traits_are_queryable_per_instantiation() {
    #[queryable]
    trait Scale<T> {
        fn scale(&self, value: T) -> T;
    }

    impl Scale<u32> for Lever {
        fn scale(&self, value: u32) -> u32 {
            self.0 * value
        }
    }
    impl Scale<f32> for Door {
        fn scale(&self, value: f32) -> f32 {
            value * 0.5
        }
    }

    let mut world = World::new();
    world.register_component_as::<dyn Scale<u32>, Lever>();
    world.register_component_as::<dyn Scale<f32>, Door>();
    world.spawn((Door, Lever(3)));

    let mut ints = world.query::<All<&dyn Scale<u32>>, ()>();
    let scaled: Vec<u32> = ints
        .iter(&mut world)
        .flat_map(|items| items.map(|item| item.scale(2)).collect::<Vec<_>>())
        .collect();
    assert_eq!(scaled, vec![6]);

    let mut floats = world.query::<All<&dyn Scale<f32>>, ()>();
    let scaled: Vec<f32> = floats
        .iter(&mut world)
        .flat_map(|items| items.map(|item| item.scale(2.0)).collect::<Vec<_>>())
        .collect();
    assert_eq!(scaled, vec![1.0]);
}

#[derive(Resource, Default)]
struct Seen(Vec<String>);

named_trait!(InSystem);

fn collect_names(query: Query<All<&dyn InSystem>>, mut seen: ResMut<Seen>) {
    for items in query.iter() {
        seen.0.extend(names(items));
    }
}

#[test]
fn runs_inside_a_scheduled_system() {
    let mut world = World::new();
    world.init_resource::<Seen>();
    world.register_component_as::<dyn InSystem, Door>();
    world.spawn(Door);

    let mut schedule = Schedule::new();
    schedule.add_system(collect_names);
    schedule
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);

    assert_eq!(world.get_resource::<Seen>().unwrap().0, vec!["door"]);
}

#[test]
fn is_scheduled_as_reading_every_component() {
    named_trait!(Access);
    let mut trait_reader = SystemAccess::default();
    (|_: Query<All<&dyn Access>>| {})
        .into_system()
        .fill_access(&mut SystemMetadata::default(), &mut trait_reader);

    let mut reader = SystemAccess::default();
    reader.read_component::<Door>();
    let mut writer = SystemAccess::default();
    writer.write_component::<Plain>();

    assert!(SystemAccess::are_disjoint(&trait_reader, &reader));
    assert!(!SystemAccess::are_disjoint(&trait_reader, &writer));
}
