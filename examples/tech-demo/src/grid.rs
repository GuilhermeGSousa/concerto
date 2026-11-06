use concerto::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    color::Color,
    ecs::CommandQueue,
    foundation::transform::Transform,
    physics::collider::Collider,
    render::components::light::Light,
    ui::frame_stats_overlay::FrameStatsOverlayPlugin,
    world_grid::WorldGrid,
};
use concerto_motion_matching_gizmos::MotionMatchingGizmosPlugin;
use glam::{Quat, Vec3};

use crate::{
    character::{
        CharacterConfig, PlayerSpawner, cycle_displacement, debug_keys, feed_foot_placement,
        setup_character_animations, spawn_character, update_movement,
    },
    ui::{spawn_grounded_overlay, update_grounded_overlay},
};

const FLOOR_HALF_EXTENT: f32 = 500.0;
const FLOOR_HALF_THICKNESS: f32 = 0.5;

const GRID_LINES: Color = Color::srgba(0.247, 0.259, 0.322, 0.6);
const GRID_SURFACE: Color = Color::srgba(0.247, 0.259, 0.322, 0.0);

const SUN_INTENSITY: f32 = 2.0;
const SUN_YAW: f32 = 0.6;
const SUN_PITCH: f32 = -0.7;

/// Builds the grid level: the character and its gizmos on an empty floor under the world grid.
pub fn grid(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .register_plugin(MotionMatchingGizmosPlugin)
        .register_plugin(FrameStatsOverlayPlugin);

    app.insert_resource(CharacterConfig::forest());

    app.add_system(Startup, spawn_grid_level)
        .add_system(Update, spawn_character)
        .add_system(Startup, spawn_grounded_overlay)
        .add_system(Update, setup_character_animations)
        .add_system(Update, feed_foot_placement)
        .add_system(Update, update_movement)
        .add_system(Update, debug_keys)
        .add_system(Update, cycle_displacement)
        .add_system(Update, update_grounded_overlay);
}

fn spawn_grid_level(mut cmd: CommandQueue) {
    cmd.spawn(WorldGrid {
        line_color: GRID_LINES,
        surface_color: GRID_SURFACE,
        lit: false,
        ..Default::default()
    });
    cmd.spawn((
        Transform::from_translation(Vec3::NEG_Y * FLOOR_HALF_THICKNESS),
        Collider::cuboid(FLOOR_HALF_EXTENT, FLOOR_HALF_THICKNESS, FLOOR_HALF_EXTENT),
    ));
    cmd.spawn((
        Light::directional_light().with_intensity(SUN_INTENSITY),
        Transform::from_translation_rotation(
            Vec3::ZERO,
            Quat::from_rotation_y(SUN_YAW) * Quat::from_rotation_x(SUN_PITCH),
        ),
    ));
    cmd.spawn((Transform::default(), PlayerSpawner::ready()));
}

#[cfg(test)]
mod tests {
    use concerto::{
        ecs::{With, Without},
        foundation::transform::Transform,
        physics::{collider::Collider, rigid_body::RigidBody},
        render::components::light::Light,
        world_grid::WorldGrid,
    };
    use concerto_motion_matching_gizmos::MotionMatchingGizmos;
    use glam::Vec3;

    use crate::character::{CharacterConfig, Player};

    use super::grid;

    #[test]
    fn the_grid_level_spawns_the_character_with_every_gizmo_on_a_grid_floor_under_a_sun() {
        let Some(mut app) = concerto::headless(grid) else {
            return;
        };
        app.update();
        app.update();

        assert_eq!(
            app.get_resource::<CharacterConfig>(),
            Some(&CharacterConfig::forest())
        );
        let world = app.main_mut().world_mut();
        assert_eq!(world.query::<&WorldGrid, ()>().iter(world).count(), 1);
        assert_eq!(world.query::<&Light, ()>().iter(world).count(), 1);
        let floors: Vec<Vec3> = world
            .query::<&Transform, (With<Collider>, Without<RigidBody>)>()
            .iter(world)
            .map(|transform| transform.translation)
            .collect();
        assert_eq!(floors.len(), 1);
        assert!(floors[0].y < 0.0);
        let players: Vec<Vec3> = world
            .query::<&Transform, With<Player>>()
            .iter(world)
            .map(|transform| transform.translation)
            .collect();
        assert_eq!(players, vec![Vec3::ZERO]);
        let gizmos: Vec<[bool; 4]> = world
            .query::<&MotionMatchingGizmos, ()>()
            .iter(world)
            .map(|g| [g.simulation, g.query, g.matched, g.history])
            .collect();
        assert_eq!(gizmos, vec![[true; 4]]);
    }
}
