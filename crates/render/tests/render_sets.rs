use concerto_ecs::{
    resource::ResMut, system::executor::single_thread::SingleThreadedExecutor, IntoSetConfigs,
    IntoSystemConfig, Resource, Schedule, World,
};
use concerto_render::sets::RenderSet;

#[derive(Resource, Default)]
struct Runs(Vec<&'static str>);

fn lights(mut runs: ResMut<Runs>) {
    runs.0.push("lights");
}

fn shadow_view_proj(mut runs: ResMut<Runs>) {
    runs.0.push("shadow_view_proj");
}

fn shadow_maps(mut runs: ResMut<Runs>) {
    runs.0.push("shadow_maps");
}

#[test]
fn the_shadow_chain_runs_each_system_once_in_order() {
    let mut schedule = Schedule::new();
    schedule.configure_sets((RenderSet::Lights, RenderSet::Shadows).chain());
    schedule.add_system(
        shadow_maps
            .in_set(RenderSet::Shadows)
            .after(shadow_view_proj),
    );
    schedule.add_system(shadow_view_proj.in_set(RenderSet::Shadows));
    schedule.add_system(lights.in_set(RenderSet::Lights));

    let mut world = World::new();
    world.insert_resource(Runs::default());
    schedule
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);

    assert_eq!(
        world.remove_resource::<Runs>().unwrap().0,
        vec!["lights", "shadow_view_proj", "shadow_maps"]
    );
}
