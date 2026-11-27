//! Gameplay building blocks over the engine: player input actions, first- and third-person
//! cameras, a first-person walker, and NPCs moved by navigation.

use concerto_app::{
    Plugin,
    schedule_groups::{FixedUpdate, Update},
};
use concerto_ecs::IntoSystemConfig;
use concerto_navigation::steer_nav_agents;
use concerto_physics::interpolation::interpolate_body_transforms;
use concerto_window::input::actions::ActionMap;

use crate::actions::bind_gameplay_actions;
use crate::camera::{
    CameraBoom, CameraSettings, move_camera_pivot, update_camera_booms, update_entity_follow,
};
use crate::first_person::{
    place_first_person_views, read_first_person_input, step_first_person_walkers,
};
use crate::navigation::follow_nav_steering;
use crate::third_person::{ThirdPersonCamera, update_third_person_cameras};

/// The input actions gameplay reads, and their default bindings.
pub mod actions;
/// Camera pivots turned by look input, entity following, and booms that keep the camera clear of
/// bodies.
pub mod camera;
/// A first-person walker driven by a player's input, and views placed at its eyes.
pub mod first_person;
/// Free flight for a [`Player`](crate::player::Player).
pub mod movement;
/// Moving navigation agents by what their navigation wants.
pub mod navigation;
/// The free-flying player and its camera.
pub mod player;
/// Third-person camera framings for follow, crouch and aim, eased between.
pub mod third_person;

/// Adds gameplay's actions, cameras, first-person walkers and nav-agent movement.
///
/// # Panics
///
/// Building it panics unless the `WindowPlugin` was added before it, since its actions are bound in
/// the window's `ActionMap`.
pub struct GameplayPlugin;

impl Plugin for GameplayPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        bind_gameplay_actions(
            app.get_resource_mut::<ActionMap>()
                .expect("WindowPlugin must be registered before GameplayPlugin"),
        );
        app.insert_resource(CameraSettings::default());
        app.register_type::<CameraBoom>();
        app.register_type::<ThirdPersonCamera>();

        app.add_system(Update, update_third_person_cameras)
            .add_system(Update, move_camera_pivot.after(update_third_person_cameras))
            .add_system(
                Update,
                update_entity_follow.after(interpolate_body_transforms),
            )
            .add_system(
                Update,
                update_camera_booms
                    .after(move_camera_pivot)
                    .after(update_entity_follow),
            )
            .add_system(Update, follow_nav_steering.after(steer_nav_agents))
            .add_system(Update, read_first_person_input)
            .add_system(FixedUpdate, step_first_person_walkers)
            .add_system(
                Update,
                place_first_person_views
                    .after(interpolate_body_transforms)
                    .after(read_first_person_input),
            );
    }
}
