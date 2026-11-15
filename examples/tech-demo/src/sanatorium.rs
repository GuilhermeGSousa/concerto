use concerto::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    asset_id,
    color::Color,
    ecs::{
        Added, CommandQueue, Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource,
        With, Without, component::name::Name,
    },
    foundation::{
        assets::{AssetId, asset_server::AssetServer},
        transform::Transform,
    },
    gameplay::third_person::update_third_person_cameras,
    mesh::MeshComponent,
    physics::{collider::Collider, shape::MeshCollider},
    render::MaterialComponent,
    scene::{scene::Scene, spawner::SceneSpawnerComponent},
    ui::frame_stats_overlay::FrameStatsOverlayPlugin,
    window::input::{Input, KeyCode, PhysicalKey, actions::ActionMap},
};
use concerto_debug_gizmos::DebugGizmos;
use concerto_motion_matching_gizmos::MotionMatchingGizmosPlugin;
use concerto_procedural_animation_gizmos::ProceduralAnimationGizmosPlugin;
use glam::Vec3;

use crate::body::install_body;
use crate::character::{
    CharacterConfig, PlayerSpawner, bind_controls, debug_keys, feed_foot_placement,
    setup_character_animations, spawn_character, update_camera_stance, update_movement,
};

const LEVEL: AssetId = asset_id!("content/sanatorium/level.gasset");
const COLLISION: AssetId = asset_id!("content/sanatorium/COL/scene.gasset");

const COLLISION_PREFIX: &str = "COL_";
const PLAYER_START: &str = "PlayerStart";

/// Builds the sanatorium: its level scene, the collision under it, and the character at `PlayerStart`.
pub fn sanatorium(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .register_plugin(MotionMatchingGizmosPlugin)
        .register_plugin(ProceduralAnimationGizmosPlugin)
        .register_plugin(FrameStatsOverlayPlugin);
    bind_controls(
        app.get_resource_mut::<ActionMap>()
            .expect("DefaultPlugins inserts the ActionMap"),
    );

    app.set_startup_scene(LEVEL)
        .register_type::<PlayerSpawner>()
        .insert_resource(CharacterConfig::sanatorium())
        .insert_resource(MarkerDebug::default());

    app.add_system(Startup, spawn_collision)
        .add_system(
            Update,
            (collide_collision_meshes, place_player_start).chain(),
        )
        .add_system(Update, classify_markers)
        .add_system(Update, spawn_character)
        .add_system(Update, setup_character_animations)
        .add_system(Update, feed_foot_placement)
        .add_system(Update, debug_keys)
        .add_system(Update, update_movement.after(update_camera_stance))
        .add_system(
            Update,
            update_camera_stance.before(update_third_person_cameras),
        )
        .add_system(Update, (toggle_marker_debug, draw_markers).chain());
    install_body(app);
}

fn spawn_collision(asset_server: Res<AssetServer>, mut cmd: CommandQueue) {
    cmd.spawn((
        SceneSpawnerComponent(asset_server.load::<Scene>(COLLISION)),
        Transform::default(),
    ));
}

fn is_collision(name: &Name) -> bool {
    name.as_str().starts_with(COLLISION_PREFIX)
}

/// Makes every spawned `COL_` mesh a static mesh collider that is never drawn.
fn collide_collision_meshes(
    spawned: Query<(Entity, &Name), (Added<Name>, With<MeshComponent>)>,
    mut cmd: CommandQueue,
) {
    for (entity, name) in spawned.iter() {
        if is_collision(name) {
            cmd.insert(MeshCollider, entity);
            cmd.remove::<MaterialComponent>(entity);
        }
    }
}

/// Marks `PlayerStart` as the player spawner once every area has spawned and every `COL_` mesh is a collider.
fn place_player_start(
    starts: Query<(Entity, &Name), Without<PlayerSpawner>>,
    loading: Query<Entity, With<SceneSpawnerComponent>>,
    unconverted: Query<
        &Name,
        (
            With<MeshComponent>,
            Without<MeshCollider>,
            Without<Collider>,
        ),
    >,
    mut cmd: CommandQueue,
) {
    if loading.iter().next().is_some() || unconverted.iter().any(is_collision) {
        return;
    }
    for (entity, name) in starts.iter() {
        if name.as_str() == PLAYER_START {
            cmd.insert(PlayerSpawner::ready(), entity);
        }
    }
}

/// What the debug view draws for a marker empty, read from its name.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
enum DebugMarker {
    WarmthZone,
    Route,
    StreamBoundary,
    Interact,
}

impl DebugMarker {
    fn of(name: &str) -> Option<Self> {
        [
            ("WarmthZone_", Self::WarmthZone),
            ("Route_", Self::Route),
            ("StreamBoundary_", Self::StreamBoundary),
            ("Interact_", Self::Interact),
        ]
        .into_iter()
        .find(|(prefix, _)| name.starts_with(prefix))
        .map(|(_, marker)| marker)
    }

    fn color(self) -> Color {
        match self {
            Self::WarmthZone => Color::srgba(1.0, 0.55, 0.1, 1.0),
            Self::Route => Color::srgba(0.2, 0.9, 0.3, 1.0),
            Self::StreamBoundary => Color::srgba(0.3, 0.6, 1.0, 1.0),
            Self::Interact => Color::srgba(1.0, 0.2, 0.8, 1.0),
        }
    }
}

fn classify_markers(spawned: Query<(Entity, &Name), Added<Name>>, mut cmd: CommandQueue) {
    for (entity, name) in spawned.iter() {
        if let Some(marker) = DebugMarker::of(name.as_str()) {
            cmd.insert(marker, entity);
        }
    }
}

/// Whether the marker debug view is drawn; F3 toggles it.
#[derive(Resource, Default, Debug, PartialEq)]
struct MarkerDebug {
    visible: bool,
}

fn toggle_marker_debug(input: Res<Input>, mut debug: ResMut<MarkerDebug>) {
    if input.is_just_pressed(PhysicalKey::Code(KeyCode::F3)) {
        debug.visible = !debug.visible;
    }
}

const STREAM_BOUNDARY_SIZE: f32 = 1.0;
const INTERACT_SIZE: f32 = 0.3;

fn draw_markers(
    markers: Query<(&DebugMarker, &Transform)>,
    debug: Res<MarkerDebug>,
    mut gizmos: DebugGizmos,
) {
    if !debug.visible {
        return;
    }
    for (marker, transform) in markers.iter() {
        let color = marker.color();
        match marker {
            DebugMarker::WarmthZone | DebugMarker::Route => {
                let mut volume = transform.clone();
                volume.scale *= 2.0;
                gizmos.cuboid(&volume, color);
            }
            DebugMarker::StreamBoundary => {
                gizmos.cuboid_min_max(
                    transform.translation,
                    Vec3::splat(STREAM_BOUNDARY_SIZE),
                    color,
                );
            }
            DebugMarker::Interact => gizmos.cross(transform.translation, INTERACT_SIZE, color),
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto::{
        ecs::{IntoSystem, System, World, component::name::Name},
        foundation::assets::handle::AssetHandle,
        mesh::MeshComponent,
        physics::shape::MeshCollider,
        render::{MaterialComponent, assets::material::StandardMaterial},
        scene::{scene::Scene, spawner::SceneSpawnerComponent},
    };

    use super::{COLLISION, collide_collision_meshes, place_player_start};
    use crate::character::PlayerSpawner;

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn mesh(world: &mut World, name: &str) -> concerto::ecs::Entity {
        world.spawn((
            Name::new(name),
            MeshComponent::default(),
            MaterialComponent::<StandardMaterial>::default(),
        ))
    }

    #[test]
    fn collision_meshes_collide_unseen_and_the_player_starts_once_every_one_collides() {
        let mut world = World::new();
        let start = world.spawn(Name::new("PlayerStart"));
        let collision = mesh(&mut world, "COL_INT_Lodge_Wall_Solid_01");
        let visual = mesh(&mut world, "INT_Lodge_Wall_Solid_01");
        let area = world.spawn(SceneSpawnerComponent(AssetHandle::<Scene>::weak(COLLISION)));
        let is_spawner = |world: &World| {
            world
                .get_component_for_entity::<PlayerSpawner>(start)
                .is_some()
        };

        run(&mut world, collide_collision_meshes);
        run(&mut world, place_player_start);
        assert!(!is_spawner(&world), "an area is still spawning");

        world.despawn(area);
        mesh(&mut world, "COL_INT_Lodge_Wall_Solid_02");
        run(&mut world, place_player_start);
        assert!(!is_spawner(&world), "a collision mesh does not collide yet");

        run(&mut world, collide_collision_meshes);
        run(&mut world, place_player_start);

        assert!(
            world
                .get_component_for_entity::<MeshCollider>(collision)
                .is_some()
        );
        assert!(
            world
                .get_component_for_entity::<MaterialComponent>(collision)
                .is_none()
        );
        assert!(
            world
                .get_component_for_entity::<MeshCollider>(visual)
                .is_none()
        );
        assert!(
            world
                .get_component_for_entity::<MaterialComponent>(visual)
                .is_some()
        );
        assert_eq!(
            world.get_component_for_entity::<PlayerSpawner>(start),
            Some(&PlayerSpawner::ready())
        );
    }
}
