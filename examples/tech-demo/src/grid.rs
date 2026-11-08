use concerto::{
    DefaultPlugins,
    app::{
        App,
        schedule_groups::{Startup, Update},
    },
    color::Color,
    ecs::{
        CommandQueue, Component, Entity, Query, Ref, Res, component::scene::SceneEntityRef,
        events::event_reader::EventReader, query::change_detection::DetectChanges,
    },
    foundation::{assets::asset_server::AssetServer, time::Time, transform::Transform},
    mesh::{MeshComponent, primitives::cuboid},
    name::Name,
    navigation::{
        NavAgent, NavArrived, NavDestination, NavLink, NavLinks, NavMesh, NavMeshComponent,
        NavMeshGeometry, NavMeshSettings, NavPoint, bake,
    },
    physics::collider::Collider,
    render::{MaterialComponent, assets::material::StandardMaterial, components::light::Light},
    ui::frame_stats_overlay::FrameStatsOverlayPlugin,
    window::input::actions::ActionMap,
    world_grid::WorldGrid,
};
use concerto_motion_matching_gizmos::MotionMatchingGizmosPlugin;
use concerto_navigation_gizmos::NavigationGizmosPlugin;
use concerto_procedural_animation_gizmos::ProceduralAnimationGizmosPlugin;
use glam::{Affine3A, Quat, Vec3};

use crate::{
    character::{
        CharacterConfig, NpcSpawner, PlayerSpawner, bind_controls, cycle_displacement, debug_keys,
        feed_foot_placement, setup_character_animations, spawn_character, spawn_npc,
        update_movement,
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

const BOX_HALF_EXTENT: f32 = 0.5;
const BOXES: [Vec3; 4] = [
    Vec3::new(0.0, BOX_HALF_EXTENT, 4.0),
    Vec3::new(4.0, BOX_HALF_EXTENT, 8.0),
    Vec3::new(0.0, BOX_HALF_EXTENT, 12.0),
    Vec3::new(-4.0, BOX_HALF_EXTENT, 8.0),
];
const BOX_COLOR: Color = Color::srgba(0.45, 0.45, 0.5, 1.0);

const NAV_FLOOR_CENTRE: Vec3 = Vec3::new(0.0, -0.5, 8.0);
const NAV_FLOOR_HALF_EXTENTS: Vec3 = Vec3::new(8.0, 0.5, 8.0);

const NPC_SPAWN: Vec3 = Vec3::new(-2.0, 0.0, 6.0);
const PATROL_WAIT: f32 = 1.0;

/// A patrol point: the anchor that names it and where it stands.
pub(crate) struct PatrolPoint {
    pub(crate) anchor: &'static str,
    pub(crate) position: Vec3,
}

pub(crate) const PATROL: [PatrolPoint; 4] = [
    PatrolPoint {
        anchor: "patrol_a",
        position: Vec3::new(-4.0, 0.0, 4.0),
    },
    PatrolPoint {
        anchor: "patrol_b",
        position: Vec3::new(4.0, 0.0, 4.0),
    },
    PatrolPoint {
        anchor: "patrol_c",
        position: Vec3::new(4.0, 0.0, 12.0),
    },
    PatrolPoint {
        anchor: "patrol_d",
        position: Vec3::new(-4.0, 0.0, 12.0),
    },
];

/// An NPC walking the patrol loop: the index of the point it heads to next, and the seconds it still waits before setting off.
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub(crate) struct Patrol {
    pub(crate) next: usize,
    pub(crate) wait: f32,
}

/// Builds the grid level: the character and its gizmos on a floor under the world grid, with four boxes the patrolling NPC routes around on a navmesh baked at startup.
pub fn grid(app: &mut App) {
    app.register_plugin(DefaultPlugins)
        .register_plugin(MotionMatchingGizmosPlugin)
        .register_plugin(ProceduralAnimationGizmosPlugin)
        .register_plugin(NavigationGizmosPlugin)
        .register_plugin(FrameStatsOverlayPlugin);
    bind_controls(
        app.get_resource_mut::<ActionMap>()
            .expect("DefaultPlugins inserts the ActionMap"),
    );

    app.insert_resource(CharacterConfig::forest());

    app.add_system(Startup, spawn_grid_level)
        .add_system(Update, spawn_character)
        .add_system(Update, spawn_npc)
        .add_system(Update, patrol)
        .add_system(Startup, spawn_grounded_overlay)
        .add_system(Update, setup_character_animations)
        .add_system(Update, feed_foot_placement)
        .add_system(Update, update_movement)
        .add_system(Update, debug_keys)
        .add_system(Update, cycle_displacement)
        .add_system(Update, update_grounded_overlay);
}

fn spawn_grid_level(asset_server: Res<AssetServer>, mut cmd: CommandQueue) {
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
    spawn_boxes(&asset_server, &mut cmd);
    spawn_navmesh(&asset_server, &mut cmd);
    cmd.spawn((Transform::default(), PlayerSpawner::ready()));
    spawn_patrol_points(&mut cmd);
    cmd.spawn((Transform::from_translation(NPC_SPAWN), NpcSpawner::ready()));
}

fn spawn_boxes(asset_server: &AssetServer, cmd: &mut CommandQueue) {
    let mesh = asset_server.add(cuboid(Vec3::splat(BOX_HALF_EXTENT * 2.0)));
    let material = asset_server.add(StandardMaterial::default().with_base_color_factor(BOX_COLOR));
    for centre in BOXES {
        cmd.spawn((
            Transform::from_translation(centre),
            Collider::cuboid(BOX_HALF_EXTENT, BOX_HALF_EXTENT, BOX_HALF_EXTENT),
            MeshComponent {
                handle: mesh.clone(),
            },
            MaterialComponent {
                handle: material.clone(),
            },
        ));
    }
}

fn grid_nav_geometry() -> NavMeshGeometry {
    let mut geometry = NavMeshGeometry::new();
    geometry.add_box(
        Affine3A::from_translation(NAV_FLOOR_CENTRE),
        NAV_FLOOR_HALF_EXTENTS,
    );
    for centre in BOXES {
        geometry.add_box(
            Affine3A::from_translation(centre),
            Vec3::splat(BOX_HALF_EXTENT),
        );
    }
    geometry
}

fn spawn_navmesh(asset_server: &AssetServer, cmd: &mut CommandQueue) {
    let settings = NavMeshSettings::new();
    match bake(&grid_nav_geometry(), &settings) {
        Ok(data) => {
            cmd.spawn(NavMeshComponent {
                handle: asset_server.add(NavMesh::baked(settings, data)),
            });
        }
        Err(error) => {
            log::warn!("the grid navmesh did not bake, so the NPC routes on the nav graph: {error}")
        }
    }
}

fn spawn_patrol_points(cmd: &mut CommandQueue) {
    let points: Vec<Entity> = PATROL
        .iter()
        .enumerate()
        .map(|(index, corner)| {
            let toward_next = PATROL[(index + 1) % PATROL.len()].position - corner.position;
            cmd.spawn((
                NavPoint {
                    anchor: Name::new(corner.anchor),
                    enabled: true,
                },
                Transform::from_translation_rotation(
                    corner.position,
                    Quat::from_rotation_y(toward_next.x.atan2(toward_next.z)),
                ),
            ))
            .entity()
        })
        .collect();
    for (index, point) in points.iter().enumerate() {
        let mut links = vec![NavLink {
            to: SceneEntityRef::Entity(points[(index + 1) % points.len()]),
            one_way: false,
        }];
        if index == 0 {
            links.push(NavLink {
                to: SceneEntityRef::Entity(points[2]),
                one_way: true,
            });
        }
        cmd.insert(NavLinks { links }, *point);
    }
}

fn patrol_destination(patrol: &Patrol) -> NavDestination {
    NavDestination::to_anchor(Name::new(PATROL[patrol.next].anchor))
}

/// Sends each new NPC to the first patrol point, and on each arrival waits, then sends it on to the next point in the loop.
pub(crate) fn patrol(
    npcs: Query<(Entity, Ref<NavAgent>, &mut Patrol, Option<&NavDestination>)>,
    mut arrived: EventReader<NavArrived>,
    time: Res<Time>,
    mut cmd: CommandQueue,
) {
    let arrivals: Vec<Entity> = arrived.read().map(|arrival| arrival.agent).collect();
    let delta = time.delta().as_secs_f32();
    for (npc, agent, mut patrol, destination) in npcs.iter() {
        if agent.is_added() && destination.is_none() {
            cmd.insert(patrol_destination(&patrol), npc);
            continue;
        }
        if arrivals.contains(&npc) {
            patrol.next = (patrol.next + 1) % PATROL.len();
            patrol.wait = PATROL_WAIT;
        }
        if patrol.wait <= 0.0 {
            continue;
        }
        patrol.wait -= delta;
        if patrol.wait <= 0.0 {
            patrol.wait = 0.0;
            cmd.insert(patrol_destination(&patrol), npc);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    use concerto::{
        app::{
            App,
            main_schedule::MainSchedulePlugin,
            plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
            schedule_groups::Startup,
        },
        ecs::{
            CommandQueue, Entity, IntoSystem, System, With, Without, World,
            events::{event_channel::EventChannel, event_cursor::EventCursor},
        },
        foundation::{time::Time, transform::Transform},
        name::Name,
        navigation::{
            NavAgent, NavArrived, NavDestination, NavGraph, NavMeshComponent, NavMeshData,
            NavMeshQuery, NavMeshSettings, NavPoint, NavigationPlugin, bake,
        },
        physics::{collider::Collider, rigid_body::RigidBody},
        render::components::light::Light,
        world_grid::WorldGrid,
    };
    use concerto_motion_matching_gizmos::MotionMatchingGizmos;
    use glam::Vec3;

    use crate::character::{CharacterConfig, Player};

    use super::{
        BOX_HALF_EXTENT, BOXES, NPC_SPAWN, PATROL, Patrol, grid, grid_nav_geometry, patrol,
        spawn_patrol_points,
    };

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
        assert_eq!(floors.len(), 1 + BOXES.len());
        assert_eq!(floors.iter().filter(|floor| floor.y < 0.0).count(), 1);
        for centre in BOXES {
            assert!(floors.contains(&centre));
        }
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
        assert_eq!(gizmos, vec![[true; 4]; 2]);
        let npcs: Vec<Vec3> = world
            .query::<&Transform, (With<NavAgent>, With<Patrol>, Without<Player>)>()
            .iter(world)
            .map(|transform| transform.translation)
            .collect();
        assert_eq!(npcs, vec![NPC_SPAWN]);
        assert_eq!(world.query::<&NavPoint, ()>().iter(world).count(), 4);
        assert_eq!(
            world.query::<&NavMeshComponent, ()>().iter(world).count(),
            1
        );
    }

    const CLEARANCE_TOLERANCE: f32 = 0.1;
    const SAMPLE_STEP: f32 = 0.05;

    fn inside_inflated_box(point: Vec3, agent_radius: f32) -> bool {
        let reach = BOX_HALF_EXTENT + agent_radius - CLEARANCE_TOLERANCE;
        BOXES
            .iter()
            .any(|centre| (point.x - centre.x).abs() < reach && (point.z - centre.z).abs() < reach)
    }

    fn on_mesh(data: &NavMeshData, point: Vec3) -> bool {
        data.polygons.iter().any(|polygon| {
            let corners: Vec<Vec3> = polygon
                .vertices
                .iter()
                .map(|index| data.vertices[*index as usize])
                .collect();
            let sides: Vec<f32> = corners
                .iter()
                .zip(corners.iter().cycle().skip(1))
                .map(|(from, to)| {
                    (to.x - from.x) * (point.z - from.z) - (to.z - from.z) * (point.x - from.x)
                })
                .collect();
            sides.iter().all(|side| *side >= 0.0) || sides.iter().all(|side| *side <= 0.0)
        })
    }

    #[test]
    fn every_patrol_leg_routes_around_the_boxes_on_the_baked_navmesh() {
        let settings = NavMeshSettings::new();
        let data = bake(&grid_nav_geometry(), &settings).unwrap();

        for point in PATROL
            .iter()
            .map(|corner| corner.position)
            .chain([NPC_SPAWN])
        {
            assert!(!inside_inflated_box(point, settings.agent_radius));
            assert!(on_mesh(&data, point), "{point} is off the navmesh");
        }

        let query = NavMeshQuery::new(&data);
        for (index, corner) in PATROL.iter().enumerate() {
            let next = PATROL[(index + 1) % PATROL.len()].position;
            let path = query
                .path(corner.position, next)
                .unwrap_or_else(|| panic!("no path from {} to {next}", corner.position));
            assert!((*path.last().unwrap() - next).length() < CLEARANCE_TOLERANCE);
            let mut from = corner.position;
            for to in path {
                let steps = ((to - from).length() / SAMPLE_STEP).ceil().max(1.0) as u32;
                for step in 0..=steps {
                    let point = from.lerp(to, step as f32 / steps as f32);
                    assert!(
                        !inside_inflated_box(point, settings.agent_radius),
                        "the leg from {} to {next} passes through a box at {point}",
                        corner.position
                    );
                }
                from = to;
            }
        }
    }

    fn anchor(graph: &NavGraph, index: usize) -> Entity {
        graph
            .anchor(Name::new(PATROL[index].anchor))
            .expect("the patrol point is in the graph")
    }

    fn spawn_patrol(mut cmd: CommandQueue) {
        spawn_patrol_points(&mut cmd);
    }

    #[test]
    fn the_patrol_points_make_a_square_with_a_one_way_diagonal_each_facing_the_next_corner() {
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(TransformPlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(NavigationPlugin);
        app.add_system(Startup, spawn_patrol);
        app.finish_plugin_build();
        app.update();
        app.update();

        let graph = app.get_resource::<NavGraph>().unwrap();
        assert_eq!(graph.points().count(), PATROL.len());
        for (index, corner) in PATROL.iter().enumerate() {
            let point = graph.point(anchor(graph, index)).unwrap();
            assert_eq!(point.position, corner.position);
            let next = PATROL[(index + 1) % PATROL.len()].position;
            let toward_next = (next - corner.position).normalize();
            assert!((point.facing * Vec3::Z - toward_next).length() < 1e-4);
        }
        let across = graph.path(anchor(graph, 0), anchor(graph, 2)).unwrap();
        assert!((across.length - 8.0 * 2.0_f32.sqrt()).abs() < 1e-3);
        let back = graph.path(anchor(graph, 2), anchor(graph, 0)).unwrap();
        assert!((back.length - 16.0).abs() < 1e-3);
    }

    #[test]
    fn the_npc_walks_to_the_first_patrol_point() {
        let Some(mut app) = concerto::headless(grid) else {
            return;
        };
        let started = Instant::now();
        let mut cursor = EventCursor::<NavArrived>::default();
        loop {
            assert!(
                started.elapsed() < Duration::from_secs(30),
                "the NPC did not reach patrol_a"
            );
            app.update();
            let first = app
                .get_resource::<NavGraph>()
                .and_then(|graph| graph.anchor(Name::new(PATROL[0].anchor)));
            let reached = app
                .get_resource::<EventChannel<NavArrived>>()
                .unwrap()
                .read_from(&mut cursor)
                .any(|arrival| first == Some(arrival.point));
            if reached {
                break;
            }
        }
    }

    fn tick(world: &mut World) {
        thread::sleep(Duration::from_millis(5));
        world.get_resource_mut::<Time>().unwrap().update();
    }

    fn destination(world: &World, npc: Entity) -> Option<NavDestination> {
        world
            .get_component_for_entity::<NavDestination>(npc)
            .cloned()
    }

    #[test]
    fn the_patrol_starts_at_patrol_a_and_moves_on_to_the_next_corner_after_a_wait() {
        let mut world = World::new();
        world.insert_resource(Time::new());
        world.insert_resource(EventChannel::<NavArrived>::new());
        let npc = world.spawn((NavAgent::new(), Patrol::default()));
        let mut system = patrol.into_system();
        system.initialize(&mut world);

        tick(&mut world);
        system.run_and_apply((), &mut world);
        assert_eq!(
            destination(&world, npc),
            Some(NavDestination::to_anchor(Name::new("patrol_a")))
        );

        world.remove_component::<NavDestination>(npc);
        world
            .get_resource_mut::<EventChannel<NavArrived>>()
            .unwrap()
            .push_event(NavArrived {
                agent: npc,
                point: npc,
            });
        tick(&mut world);
        system.run_and_apply((), &mut world);
        assert_eq!(destination(&world, npc), None);
        assert_eq!(
            world.get_component_for_entity::<Patrol>(npc).unwrap().next,
            1
        );

        world.insert(
            Patrol {
                next: 1,
                wait: 0.001,
            },
            npc,
        );
        tick(&mut world);
        system.run_and_apply((), &mut world);
        assert_eq!(
            destination(&world, npc),
            Some(NavDestination::to_anchor(Name::new("patrol_b")))
        );
    }
}
