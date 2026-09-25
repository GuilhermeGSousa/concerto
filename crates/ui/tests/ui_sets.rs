use concerto_ecs::{
    IntoSetConfigs, IntoSystemConfig, Resource, Schedule, World, resource::ResMut,
    system::executor::single_thread::SingleThreadedExecutor,
};
use concerto_ui::sets::UiSet;

#[derive(Resource, Default)]
struct Runs(Vec<&'static str>);

fn hit_test(mut runs: ResMut<Runs>) {
    runs.0.push("hit_test");
}

fn layout(mut runs: ResMut<Runs>) {
    runs.0.push("layout");
}

fn app_widget(mut runs: ResMut<Runs>) {
    runs.0.push("app_widget");
}

#[test]
fn a_consumer_can_slot_between_ui_sets_without_naming_a_ui_system() {
    let mut schedule = Schedule::new();
    schedule.add_system(app_widget.after(UiSet::Input).before(UiSet::Layout));
    schedule.configure_sets((UiSet::Input, UiSet::Widgets, UiSet::Layout).chain());
    schedule.add_system(layout.in_set(UiSet::Layout));
    schedule.add_system(hit_test.in_set(UiSet::Input));

    let mut world = World::new();
    world.insert_resource(Runs::default());
    schedule
        .compile::<SingleThreadedExecutor>(&mut world)
        .run(&mut world);

    assert_eq!(
        world.remove_resource::<Runs>().unwrap().0,
        vec!["hit_test", "app_widget", "layout"]
    );
}
