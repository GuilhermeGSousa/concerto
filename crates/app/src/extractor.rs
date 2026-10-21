use std::mem;

use concerto_ecs::{
    component::SystemTicks,
    system::input::{ReadOnlySystemInput, SystemInput, SystemInputData, ValidationError},
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
    type State = T::State;

    type Data<'world, 'state> = Extracted<'world, 'state, T>;

    fn init_state(world: &mut World) -> Self::State {
        T::init_state(world)
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
        // SAFETY: the main world is lent whole to the render world for `Extract`, and `T`
        // only reads it.
        unsafe {
            T::get_data(
                state,
                main_world.as_unsafe_world_cell(),
                main_world.out_of_system_ticks(),
            )
        }
        .map(Extracted)
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
    use concerto_ecs::{IntoSystem, Res, ResMut, System};

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
}
