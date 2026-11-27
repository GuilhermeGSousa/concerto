use concerto_ecs::World;
use concerto_foundation::time::Time;

use crate::{
    edit_time::AtEditTime,
    schedule_groups::{First, FixedUpdate, LateFixedUpdate, LateUpdate, Main, Update},
    Plugin,
};

/// Runs the main world's frame: [`First`], then [`FixedUpdate`] and [`LateFixedUpdate`] once per
/// fixed step the frame's time allows, then [`Update`] and [`LateUpdate`].
///
/// Add it to every game `App`; it needs [`Time`], which [`TimePlugin`](crate::plugins::TimePlugin)
/// inserts.
pub struct MainSchedulePlugin;

fn run_main(world: &mut World) {
    {
        profiling::scope!("schedule::event_update");
        world.run_schedule(First);
    }

    world
        .get_resource_mut::<Time>()
        .expect("Time resource not found")
        .accumulate_fixed_time();

    while world
        .get_resource_mut::<Time>()
        .unwrap()
        .expend_fixed_time()
    {
        profiling::scope!("fixed_update_step");

        {
            profiling::scope!("schedule::fixed_update");
            world.run_schedule(FixedUpdate);
        }

        {
            profiling::scope!("schedule::late_fixed_update");
            world.run_schedule(LateFixedUpdate);
        }
    }

    {
        profiling::scope!("schedule::update");
        world.run_schedule(Update);
    }

    {
        profiling::scope!("schedule::late_update");
        world.run_schedule(LateUpdate);
    }

    {
        profiling::scope!("world_tick");
        world.tick();
    }
}

impl Plugin for MainSchedulePlugin {
    fn build(&self, app: &mut crate::App) {
        app.main_mut().set_update_schedule(Main);
        app.add_system(Main, run_main.at_edit_time());

        #[cfg(all(feature = "multithreaded", not(target_arch = "wasm32")))]
        {
            use concerto_ecs::system::executor::multi_thread::MainThreadExecutor;
            app.insert_resource(MainThreadExecutor::default());
        }
    }
}
