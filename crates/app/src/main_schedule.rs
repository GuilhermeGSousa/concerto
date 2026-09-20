use ecs::World;
use essential::time::Time;

use crate::{
    schedule_groups::{First, FixedUpdate, LateFixedUpdate, LateUpdate, Main, Update},
    Plugin,
};

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
        app.add_system(Main, run_main);

        #[cfg(all(feature = "multithreaded", not(target_arch = "wasm32")))]
        {
            use ecs::system::executor::multi_thread::MainThreadExecutor;
            app.insert_resource(MainThreadExecutor::default());
        }
    }
}
