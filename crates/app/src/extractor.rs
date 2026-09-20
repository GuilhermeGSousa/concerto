use std::mem;

use derive_more::{Deref, DerefMut};
use ecs::{
    system::input::{ReadOnlySystemInput, SystemInput, SystemInputData},
    Resource, World,
};

use crate::schedule_groups::Extract;

pub type ExtractFn = Box<dyn FnMut(&mut World, &mut World)>;

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

    fn get_data<'world, 'state>(
        state: &'state mut Self::State,
        world: ecs::world::UnsafeWorldCell<'world>,
    ) -> Self::Data<'world, 'state> {
        let main_world = world
            .world()
            .get_resource::<MainWorld>()
            .expect("MainWorld not found — `Extract` is only valid in the `Extract` update group");

        Extracted(T::get_data(state, main_world.as_unsafe_world_cell()))
    }

    fn fill_access(
        _meta: &mut ecs::system::meta::SystemMetadata,
        access: &mut ecs::system::access::SystemAccess,
    ) {
        access.read_resource::<MainWorld>();
    }
}
