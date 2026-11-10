use std::mem;

use concerto_ecs::{
    component::SystemTicks,
    system::input::{
        ForeignTicks, ReadOnlySystemInput, SystemInput, SystemInputData, ValidationError,
    },
    Resource, World,
};
use derive_more::{Deref, DerefMut};

use crate::schedule_groups::Extract;

pub type ExtractFn = Box<dyn FnMut(&mut World, &mut World) + Send + Sync>;

#[derive(Deref, Default, Resource)]
pub(crate) struct ScratchMainWorld(pub(crate) World);

#[derive(Deref, Resource)]
pub(crate) struct MainWorld(pub(crate) World);

impl MainWorld {
    pub(crate) fn new(world: World) -> Self {
        Self(world)
    }
}

/// Moves `main` into `render` as the [`MainWorld`] resource, runs the
/// [`Extract`] schedule, then swaps it back out.
pub fn extract(main: &mut World, render: &mut World) {
    let scratch = main
        .remove_resource::<ScratchMainWorld>()
        .unwrap_or_default();

    let moved_main = mem::replace(main, scratch.0);

    render.insert_resource(MainWorld::new(moved_main));

    render.run_schedule(Extract);

    let moved_main = render.remove_resource::<MainWorld>().unwrap();
    let scratch_world = mem::replace(main, moved_main.0);
    main.insert_resource(ScratchMainWorld(scratch_world));
}

/// Access data from the MainWorld
/// The MainWorld only exists during the [`Extract`] schedule
#[derive(Deref, DerefMut)]
pub struct Extracted<'world, 'state, T: ReadOnlySystemInput + 'static>(
    SystemInputData<'world, 'state, T>,
);

impl<T: ReadOnlySystemInput + 'static> SystemInput for Extracted<'_, '_, T> {
    type State = (T::State, ForeignTicks);

    type Data<'world, 'state> = Extracted<'world, 'state, T>;

    fn init_state(world: &mut World) -> Self::State {
        (T::init_state(world), ForeignTicks::default())
    }

    unsafe fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: concerto_ecs::world::UnsafeWorldCell<'world>,
        _ticks: SystemTicks,
    ) -> Result<Self::Data<'world, 'state>, ValidationError> {
        // SAFETY: `fill_access` declares read access to `MainWorld`.
        let main_world = unsafe { world.world() }
            .get_resource::<MainWorld>()
            .ok_or_else(|| {
                ValidationError::missing::<MainWorld>(
                    "`Extract` systems run only while the main world is lent to the render world",
                )
            })?;
        let (state, main_ticks) = state;
        let ticks = main_ticks.begin(main_world);
        // SAFETY: the main world is lent whole to the render world for `Extract`, and `T`
        // only reads it.
        unsafe { T::get_data(state, main_world.as_unsafe_world_cell(), ticks) }.map(Extracted)
    }

    fn ran(state: &mut Self::State) {
        T::ran(&mut state.0);
        state.1.commit();
    }

    fn fill_access(
        _meta: &mut concerto_ecs::system::meta::SystemMetadata,
        access: &mut concerto_ecs::system::access::SystemAccess,
    ) {
        access.read_resource::<MainWorld>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::App;
    use concerto_ecs::{
        component::removed::RemovedComponents, query::change_detection::DetectChanges,
        system::BoxedSystem, Component, Entity, IntoSystem, Res, ResMut, System,
    };

    #[derive(Resource)]
    struct Counter(usize);

    #[derive(Resource, Default)]
    struct Seen(Option<usize>);

    fn observe(counter: Extracted<Option<Res<Counter>>>, mut seen: ResMut<Seen>) {
        seen.0 = counter.0.as_ref().map(|counter| counter.0);
    }

    fn run_with(main: World) -> Option<usize> {
        let mut render = World::new();
        render.insert_resource(Seen::default());
        render.insert_resource(MainWorld::new(main));
        let mut system = observe.into_system();
        system.initialize(&mut render);
        system.run_and_apply((), &mut render);
        render.get_resource::<Seen>().unwrap().0
    }

    #[derive(Resource, Default)]
    struct Ran(bool);

    fn extracts(_: Extracted<Option<Res<Counter>>>, mut ran: ResMut<Ran>) {
        ran.0 = true;
    }

    #[test]
    fn an_extracting_system_is_skipped_without_a_main_world() {
        let mut render = World::new();
        render.insert_resource(Ran::default());
        let mut system = extracts.into_system();
        system.initialize(&mut render);

        system.run_and_apply((), &mut render);

        assert!(!render.get_resource::<Ran>().unwrap().0);
    }

    #[test]
    fn an_extracted_optional_resource_reads_the_main_world() {
        assert_eq!(run_with(World::new()), None);

        let mut main = World::new();
        main.insert_resource(Counter(3));
        assert_eq!(run_with(main), Some(3));
    }

    #[derive(Resource, Default)]
    struct Changes {
        changed: Option<bool>,
        added: Option<bool>,
    }

    #[derive(Resource)]
    struct Enabled;

    fn watch(counter: Extracted<Res<Counter>>, mut changes: ResMut<Changes>) {
        changes.changed = Some(counter.is_changed());
        changes.added = Some(counter.is_added());
    }

    fn watch_when_enabled(
        counter: Extracted<Res<Counter>>,
        _: Res<Enabled>,
        mut changes: ResMut<Changes>,
    ) {
        changes.changed = Some(counter.is_changed());
        changes.added = Some(counter.is_added());
    }

    fn bump(mut counter: ResMut<Counter>) {
        counter.0 += 1;
    }

    struct Frames {
        main: World,
        render: World,
        system: BoxedSystem,
    }

    impl Frames {
        fn new(system: BoxedSystem) -> Self {
            let mut main = World::new();
            main.insert_resource(Counter(0));
            let mut render = World::new();
            render.insert_resource(Changes::default());
            render.insert_resource(Enabled);
            let mut frames = Self {
                main,
                render,
                system,
            };
            frames.system.initialize(&mut frames.render);
            frames
        }

        fn extract(&mut self) -> Option<bool> {
            *self.render.get_resource_mut::<Changes>().unwrap() = Changes::default();
            self.render
                .insert_resource(MainWorld::new(mem::take(&mut self.main)));
            self.system.run_and_apply((), &mut self.render);
            self.main = self.render.remove_resource::<MainWorld>().unwrap().0;
            self.render.get_resource::<Changes>().unwrap().changed
        }

        fn added(&self) -> Option<bool> {
            self.render.get_resource::<Changes>().unwrap().added
        }
    }

    enum Step {
        Extract(Option<bool>),
        Added(bool),
        SystemWrite,
        DirectWrite,
        Tick,
        Disable,
        Enable,
    }

    struct Watch {
        name: &'static str,
        gated: bool,
        steps: &'static [Step],
    }

    #[test]
    fn an_extract_system_sees_each_main_world_change_once_even_across_a_skipped_run() {
        use Step::*;
        let cases = [
            Watch {
                name: "a main world system's write",
                gated: false,
                steps: &[
                    Extract(Some(true)),
                    Extract(Some(false)),
                    SystemWrite,
                    Tick,
                    Extract(Some(true)),
                    Extract(Some(false)),
                ],
            },
            Watch {
                name: "a direct write between extracts",
                gated: false,
                steps: &[
                    Extract(Some(true)),
                    Extract(Some(false)),
                    DirectWrite,
                    Extract(Some(true)),
                    Extract(Some(false)),
                ],
            },
            Watch {
                name: "a first run sees everything",
                gated: false,
                steps: &[
                    Tick,
                    Tick,
                    Extract(Some(true)),
                    Added(true),
                    Extract(Some(false)),
                    Added(false),
                ],
            },
            Watch {
                name: "a skipped extract keeps the change",
                gated: true,
                steps: &[
                    Extract(Some(true)),
                    Extract(Some(false)),
                    Disable,
                    DirectWrite,
                    Extract(None),
                    Tick,
                    Enable,
                    Extract(Some(true)),
                    Extract(Some(false)),
                ],
            },
        ];
        for case in cases {
            let system = if case.gated {
                watch_when_enabled.into_boxed_system()
            } else {
                watch.into_boxed_system()
            };
            let mut frames = Frames::new(system);
            let mut writer = bump.into_system();
            writer.initialize(&mut frames.main);
            for step in case.steps {
                match step {
                    Extract(changed) => {
                        assert_eq!(frames.extract(), *changed, "{}", case.name)
                    }
                    Added(added) => {
                        assert_eq!(frames.added(), Some(*added), "{}", case.name)
                    }
                    SystemWrite => writer.run_and_apply((), &mut frames.main),
                    DirectWrite => frames.main.get_resource_mut::<Counter>().unwrap().0 += 1,
                    Tick => frames.main.tick(),
                    Disable => {
                        frames.render.remove_resource::<Enabled>();
                    }
                    Enable => frames.render.insert_resource(Enabled),
                }
            }
        }
    }

    #[derive(Component)]
    struct Health;

    #[derive(Resource, Default)]
    struct Removed(Vec<Entity>);

    fn read_removals(mut removed: Extracted<RemovedComponents<Health>>, mut seen: ResMut<Removed>) {
        seen.0.extend(removed.read());
    }

    #[test]
    fn removals_in_the_main_world_reach_an_extract_system() {
        let mut app = App::new();
        app.track_removals::<Health>();
        let main = app.main_mut().world_mut();
        let entity = main.spawn(Health);
        main.remove_component::<Health>(entity);
        let mut render = World::new();
        render.insert_resource(Removed::default());
        render.insert_resource(MainWorld::new(mem::take(main)));
        let mut system = read_removals.into_system();
        system.initialize(&mut render);

        system.run_and_apply((), &mut render);

        assert_eq!(render.get_resource::<Removed>().unwrap().0, vec![entity]);
    }
}
