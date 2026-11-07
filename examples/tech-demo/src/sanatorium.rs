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
    render::{MaterialComponent, components::light::Light},
    scene::{scene::Scene, spawner::SceneSpawnerComponent},
    ui::frame_stats_overlay::FrameStatsOverlayPlugin,
    window::input::{Input, KeyCode, PhysicalKey},
};
use concerto_debug_gizmos::DebugGizmos;
use concerto_motion_matching_gizmos::MotionMatchingGizmosPlugin;
use glam::{Quat, Vec3};

use crate::character::{
    CharacterConfig, PlayerSpawner, debug_keys, feed_foot_placement, setup_character_animations,
    spawn_character, update_camera_stance, update_movement,
};

const MARKERS: AssetId = asset_id!("content/sanatorium/MARKERS/scene.gasset");
const AREAS: [AssetId; 10] = [
    asset_id!("content/sanatorium/EXT_Funicular/scene.gasset"),
    asset_id!("content/sanatorium/EXT_Approach/scene.gasset"),
    asset_id!("content/sanatorium/EXT_Balconies/scene.gasset"),
    asset_id!("content/sanatorium/INT_Lodge/scene.gasset"),
    asset_id!("content/sanatorium/INT_WardCorridor/scene.gasset"),
    asset_id!("content/sanatorium/INT_BrotherWing/scene.gasset"),
    asset_id!("content/sanatorium/INT_DirectorWing/scene.gasset"),
    asset_id!("content/sanatorium/INT_Boiler/scene.gasset"),
    asset_id!("content/sanatorium/INT_Service/scene.gasset"),
    asset_id!("content/sanatorium/COL/scene.gasset"),
];

const SUN_INTENSITY: f32 = 2.0;
const SUN_YAW: f32 = 0.6;
const SUN_PITCH: f32 = -0.7;

const COLLISION_PREFIX: &str = "COL_";
const PLAYER_START: &str = "PlayerStart";

/// Builds the sanatorium graybox: every area, its collision, and the character at `PlayerStart`.
pub fn sanatorium(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .register_plugin(MotionMatchingGizmosPlugin)
        .register_plugin(FrameStatsOverlayPlugin);

    app.set_startup_scene(MARKERS)
        .register_type::<PlayerSpawner>()
        .insert_resource(CharacterConfig::sanatorium())
        .insert_resource(MarkerDebug::default());

    app.add_system(Startup, spawn_areas)
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
}

fn spawn_areas(asset_server: Res<AssetServer>, mut cmd: CommandQueue) {
    cmd.spawn((
        Light::directional_light().with_intensity(SUN_INTENSITY),
        Transform::from_translation_rotation(
            Vec3::ZERO,
            Quat::from_rotation_y(SUN_YAW) * Quat::from_rotation_x(SUN_PITCH),
        ),
    ));
    for area in AREAS {
        cmd.spawn((
            SceneSpawnerComponent(asset_server.load::<Scene>(area)),
            Transform::default(),
        ));
    }
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
        app::StartupScene,
        ecs::{IntoSystem, System, World, component::name::Name},
        foundation::assets::{content::decode_content_asset, handle::AssetHandle},
        mesh::MeshComponent,
        physics::{collider::Collider, shape::MeshCollider},
        render::{MaterialComponent, assets::material::StandardMaterial},
        scene::{scene::Scene, spawner::SceneSpawnerComponent},
        window::input::{Input, KeyCode, PhysicalKey},
    };

    use super::{
        AREAS, DebugMarker, MARKERS, MarkerDebug, classify_markers, collide_collision_meshes,
        place_player_start, sanatorium, toggle_marker_debug,
    };
    use crate::character::{CharacterConfig, PlayerSpawner};

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
    fn collision_meshes_become_undrawn_colliders_and_the_rest_stay_drawn() {
        let mut world = World::new();
        let collision = mesh(&mut world, "COL_INT_Lodge_Wall_Solid_01");
        let visual = mesh(&mut world, "INT_Lodge_Wall_Solid_01");

        run(&mut world, collide_collision_meshes);

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
    }

    fn is_spawner(world: &World, entity: concerto::ecs::Entity) -> bool {
        world
            .get_component_for_entity::<PlayerSpawner>(entity)
            .is_some()
    }

    #[test]
    fn player_start_waits_for_every_area_and_every_collision_mesh() {
        let mut world = World::new();
        let start = world.spawn(Name::new("PlayerStart"));
        let area = world.spawn(SceneSpawnerComponent(AssetHandle::<Scene>::weak(AREAS[0])));

        run(&mut world, place_player_start);
        assert!(!is_spawner(&world, start));

        world.despawn(area);
        let collision = mesh(&mut world, "COL_EXT_Approach_Ground_Slope_01");
        run(&mut world, place_player_start);
        assert!(!is_spawner(&world, start));

        world.insert(MeshCollider, collision);
        run(&mut world, place_player_start);
        assert_eq!(
            world.get_component_for_entity::<PlayerSpawner>(start),
            Some(&PlayerSpawner::ready())
        );
    }

    #[test]
    fn a_built_collider_counts_as_converted() {
        let mut world = World::new();
        let start = world.spawn(Name::new("PlayerStart"));
        let collision = mesh(&mut world, "COL_INT_Boiler_Floor_Tile_01");
        world.insert(Collider::cuboid(1.0, 1.0, 1.0), collision);

        run(&mut world, place_player_start);

        assert!(is_spawner(&world, start));
    }

    #[test]
    fn marker_names_choose_what_the_debug_view_draws() {
        let mut world = World::new();
        let names = [
            "WarmthZone_Lodge",
            "Route_Heavy_ServiceDoor",
            "StreamBoundary_Lodge_WardCorridor",
            "Interact_Door_Ward",
            "NPC_Nurse",
        ];
        let entities: Vec<_> = names
            .iter()
            .map(|name| world.spawn(Name::new(*name)))
            .collect();

        run(&mut world, classify_markers);

        let kinds: Vec<Option<DebugMarker>> = entities
            .iter()
            .map(|entity| {
                world
                    .get_component_for_entity::<DebugMarker>(*entity)
                    .copied()
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                Some(DebugMarker::WarmthZone),
                Some(DebugMarker::Route),
                Some(DebugMarker::StreamBoundary),
                Some(DebugMarker::Interact),
                None,
            ]
        );
    }

    #[test]
    fn f3_toggles_the_marker_debug_view() {
        let mut world = World::new();
        world.insert_resource(MarkerDebug::default());
        let mut input = Input::new();
        input.update_key_input(PhysicalKey::Code(KeyCode::F3), true);
        world.insert_resource(input);

        run(&mut world, toggle_marker_debug);

        assert_eq!(
            world.get_resource::<MarkerDebug>(),
            Some(&MarkerDebug { visible: true })
        );
    }

    #[test]
    fn the_recipe_starts_from_the_markers_with_the_sanatorium_character() {
        let Some(app) = concerto::headless(sanatorium) else {
            return;
        };

        assert_eq!(
            app.get_resource::<StartupScene>(),
            Some(&StartupScene(MARKERS))
        );
        assert_eq!(
            app.get_resource::<CharacterConfig>(),
            Some(&CharacterConfig::sanatorium())
        );
    }

    fn scene(path: &str) -> Scene {
        let bytes =
            std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).unwrap();
        decode_content_asset::<Scene>(&bytes).unwrap().1
    }

    #[test]
    fn the_collision_scene_holds_only_col_meshes_and_the_markers_hold_the_player_start() {
        let collision = scene("content/sanatorium/COL/scene.gasset");
        assert!(collision.nodes.len() > 1000);
        assert!(
            collision
                .nodes
                .iter()
                .filter(|node| !node.name.contains(".primitive"))
                .all(|node| node.name.starts_with("COL_"))
        );

        let markers = scene("content/sanatorium/MARKERS/scene.gasset");
        assert_eq!(
            markers
                .nodes
                .iter()
                .filter(|node| node.name == "PlayerStart")
                .count(),
            1
        );
    }
}
