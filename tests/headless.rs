use concerto::DefaultPlugins;
use concerto_app::schedule_groups::Update;
use concerto_ecs::resource::{ResMut, Resource};
use concerto_window::plugin::Window;

#[derive(Resource, Default)]
struct Steps(u32);

fn count(mut steps: ResMut<Steps>) {
    steps.0 += 1;
}

#[test]
fn default_plugins_step_without_a_window() {
    let Some(mut app) = concerto::headless(|app| {
        app.register_plugin(DefaultPlugins)
            .insert_resource(Steps::default())
            .add_system(Update, count);
    }) else {
        return;
    };

    app.update();
    app.update();

    assert_eq!(app.get_resource::<Steps>().unwrap().0, 2);
    assert_eq!(app.get_resource::<Window>().unwrap().size(), (1280, 720));
}
