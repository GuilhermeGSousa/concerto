use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};
use concerto_ecs::component::scene::SceneEntityRef;
use concerto_foundation::{
    assets::{
        Asset, AssetId, ContentAssetRoot, CreatableAsset,
        asset_server::AssetServer,
        asset_store::AssetStore,
        content::{check_encoding, save_content_asset_into},
        derived::{DerivedData, DerivedDataCache, DerivedInputs},
        handle::AssetHandle,
        registry::AssetRegistry,
    },
    transform::Transform,
};
use concerto_mesh::{MeshComponent, primitives::plane};
use concerto_navigation::{
    NavAgent, NavDestination, NavLink, NavLinks, NavMesh, NavMeshComponent, NavMeshData,
    NavMeshGeometry, NavMeshSettings, NavMeshStatus, NavPoint, NavRoute, NavSteering,
    NavigationPlugin, bake,
};
use concerto_physics::{collider::BoxCollider, sensor::Sensor, shape::MeshCollider};
use concerto_reflect::Reflect;
use concerto_scene::scene::{Scene, SceneNode};
use glam::{Affine3A, Quat, Vec2, Vec3};

struct Project {
    root: PathBuf,
    registry: AssetRegistry,
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Project {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "navmesh-asset-{tag}-{}",
            AssetId::new().simple_hex()
        ));
        std::fs::create_dir_all(root.join("content")).unwrap();
        Self {
            root,
            registry: AssetRegistry::new(),
        }
    }

    fn save<A: Asset>(&mut self, name: &str, value: &A) -> AssetId {
        save_content_asset_into(
            value,
            &self.root,
            &format!("content/{name}.gasset"),
            &mut self.registry,
        )
        .unwrap()
    }

    fn content_root(&self) -> ContentAssetRoot {
        ContentAssetRoot::Directory(self.root.clone())
    }

    fn inputs(&self, navmesh: &NavMesh) -> anyhow::Result<DerivedInputs> {
        let root = self.content_root();
        let mut inputs = pollster::block_on(DerivedInputs::read(
            &root,
            &self.registry,
            &navmesh.derived_inputs(),
        ))?;
        let further = navmesh.further_inputs(&inputs)?;
        pollster::block_on(inputs.read_more(&root, &self.registry, &further))?;
        Ok(inputs)
    }

    fn derive(&self, navmesh: &NavMesh) -> NavMesh {
        let inputs = self.inputs(navmesh).unwrap();
        let blob = navmesh.derive(&inputs, &mut |_, _| {}).unwrap();
        let mut derived = navmesh.clone();
        derived.apply_derived(&blob).unwrap();
        derived
    }

    fn blobs(&self) -> Vec<PathBuf> {
        let dir = self.root.join(format!(
            "derived-cache/NavMesh/v{}",
            NavMesh::DERIVED_VERSION
        ));
        std::fs::read_dir(dir)
            .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
            .unwrap_or_default()
    }

    fn load(&self, id: AssetId) -> Option<NavMesh> {
        let mut app = app();
        let server = app.get_resource::<AssetServer>().unwrap().clone();
        server.set_derived_cache(DerivedDataCache::in_project(&self.root));
        server.publish_content_source(self.content_root(), self.registry.clone());
        app.insert_resource(self.registry.clone());
        let handle = server.load::<NavMesh>(id);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            app.update();
            if let Some(navmesh) = app
                .get_resource::<AssetStore<NavMesh>>()
                .unwrap()
                .get(&handle)
            {
                return Some(navmesh.clone());
            }
            if !server.is_loading() {
                return None;
            }
            assert!(Instant::now() < deadline, "load timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

fn app() -> App {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(TransformPlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(NavigationPlugin);
    app.finish_plugin_build();
    app
}

fn placed(name: &str, transform: Transform, children: Vec<usize>) -> SceneNode {
    let mut node = SceneNode {
        name: name.to_owned(),
        children,
        components: Vec::new(),
    };
    node.push_component(&transform).unwrap();
    node
}

fn node(name: &str, translation: Vec3, children: Vec<usize>) -> SceneNode {
    placed(name, Transform::from_translation(translation), children)
}

fn with<T: Reflect>(mut node: SceneNode, component: &T) -> SceneNode {
    node.push_component(component).unwrap();
    node
}

fn scene(nodes: Vec<SceneNode>) -> Scene {
    Scene {
        nodes,
        referenced_assets: Vec::new(),
    }
}

fn box_collider(half_extents: Vec3) -> BoxCollider {
    BoxCollider { half_extents }
}

fn mesh_component(id: AssetId) -> MeshComponent {
    MeshComponent {
        handle: AssetHandle::weak(id),
    }
}

fn floor() -> SceneNode {
    with(
        node("floor", Vec3::new(0.0, -0.5, 0.0), Vec::new()),
        &box_collider(Vec3::new(5.0, 0.5, 5.0)),
    )
}

fn obstacle() -> SceneNode {
    with(
        node("obstacle", Vec3::new(0.0, 0.5, 0.0), Vec::new()),
        &box_collider(Vec3::splat(0.5)),
    )
}

fn navmesh_of(scenes: &[AssetId]) -> NavMesh {
    scenes.iter().fold(NavMesh::new(), |navmesh, id| {
        navmesh.with_scene(AssetHandle::weak(*id))
    })
}

fn corners(navmesh: &NavMesh, from: Vec3, to: Vec3) -> Vec<Vec3> {
    navmesh
        .query()
        .expect("a derived navmesh has a query")
        .path(from, to)
        .expect("a path joins the two points")
}

struct Bounds {
    min: Vec3,
    max: Vec3,
}

fn bounds(data: &NavMeshData) -> Bounds {
    Bounds {
        min: data
            .vertices
            .iter()
            .fold(Vec3::splat(f32::MAX), |min, vertex| min.min(*vertex)),
        max: data
            .vertices
            .iter()
            .fold(Vec3::splat(f32::MIN), |max, vertex| max.max(*vertex)),
    }
}

fn assert_near(edge: f32, expected: f32, cell_size: f32) {
    assert!(
        (edge - expected).abs() <= 2.0 * cell_size,
        "edge at {edge}, expected about {expected}"
    );
}

const WEST: Vec3 = Vec3::new(-3.0, 0.0, 0.0);
const EAST: Vec3 = Vec3::new(3.0, 0.0, 0.0);

#[test]
fn a_box_collider_in_a_scene_leaves_a_hole_the_path_goes_around() {
    let mut project = Project::new("hole");
    let level = project.save("level", &scene(vec![floor(), obstacle()]));

    let navmesh = project.derive(&navmesh_of(&[level]));

    let path = corners(&navmesh, WEST, EAST);
    assert!(path.len() >= 2, "{path:?}");
    assert!(path.iter().any(|corner| corner.z.abs() > 0.5), "{path:?}");
}

#[test]
fn a_sensor_box_is_not_baked() {
    let mut project = Project::new("sensor");
    let level = project.save("level", &scene(vec![floor(), with(obstacle(), &Sensor)]));

    let navmesh = project.derive(&navmesh_of(&[level]));

    assert_eq!(corners(&navmesh, WEST, EAST).len(), 1);
}

#[test]
fn a_sensor_mesh_collider_stays_solid_as_physics_keeps_it() {
    let mut project = Project::new("sensor-mesh");
    let ground = project.save("ground", &plane(Vec2::splat(10.0)));
    let level = project.save(
        "level",
        &scene(vec![with(
            with(
                with(node("ground", Vec3::ZERO, Vec::new()), &MeshCollider),
                &mesh_component(ground),
            ),
            &Sensor,
        )]),
    );

    let navmesh = project.derive(&navmesh_of(&[level]));

    assert!(!navmesh.data().unwrap().polygons.is_empty());
    assert_eq!(corners(&navmesh, WEST, EAST).len(), 1);
}

#[test]
fn nested_transforms_place_colliders_in_world_space() {
    let mut project = Project::new("nested");
    let parent = placed(
        "parent",
        Transform {
            translation: Vec3::new(10.0, 0.0, 0.0),
            rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            scale: Vec3::new(2.0, 1.0, 2.0),
        },
        vec![1],
    );
    let child = with(
        node("child", Vec3::new(1.0, -0.5, 0.0), Vec::new()),
        &box_collider(Vec3::new(2.0, 0.5, 2.0)),
    );
    let level = project.save("level", &scene(vec![parent, child]));

    let navmesh = project.derive(&navmesh_of(&[level]));

    let settings = NavMeshSettings::new();
    let Bounds { min, max } = bounds(navmesh.data().unwrap());
    let inset = settings.agent_radius;
    assert_near(min.x, 6.0 + inset, settings.cell_size);
    assert_near(max.x, 14.0 - inset, settings.cell_size);
    assert_near(min.z, -6.0 + inset, settings.cell_size);
    assert_near(max.z, 2.0 - inset, settings.cell_size);
    assert!(min.y.abs() <= 0.1 && max.y.abs() <= 0.1, "{min} {max}");
}

#[test]
fn further_inputs_name_the_meshes_of_colliding_nodes_and_nothing_else() {
    let mut project = Project::new("further");
    let collider = AssetId::new();
    let drawn = AssetId::new();
    let boxed = AssetId::new();
    let sensor = AssetId::new();
    let level = project.save(
        "level",
        &scene(vec![
            with(
                with(node("collider", Vec3::ZERO, Vec::new()), &MeshCollider),
                &mesh_component(collider),
            ),
            with(
                node("drawn", Vec3::ZERO, Vec::new()),
                &mesh_component(drawn),
            ),
            with(
                with(
                    with(node("boxed", Vec3::ZERO, Vec::new()), &MeshCollider),
                    &mesh_component(boxed),
                ),
                &box_collider(Vec3::ONE),
            ),
            with(
                with(
                    with(node("sensor", Vec3::ZERO, Vec::new()), &MeshCollider),
                    &mesh_component(sensor),
                ),
                &Sensor,
            ),
        ]),
    );
    let navmesh = navmesh_of(&[level]);
    let root = project.content_root();
    let inputs = pollster::block_on(DerivedInputs::read(
        &root,
        &project.registry,
        &navmesh.derived_inputs(),
    ))
    .unwrap();

    assert_eq!(
        navmesh.further_inputs(&inputs).unwrap(),
        vec![collider, sensor]
    );
}

#[test]
fn a_navmesh_loaded_through_the_server_derives_caches_and_keeps_its_scenes_weak() {
    let mut project = Project::new("round-trip");
    let level = project.save("level", &scene(vec![floor(), obstacle()]));
    let id = project.save(
        "navmesh",
        &navmesh_of(&[level]).with_settings(NavMeshSettings::new().with_agent_radius(0.4)),
    );

    let first = project.load(id).expect("the navmesh loads");
    assert!(!first.data().unwrap().polygons.is_empty());
    assert!(matches!(first.scenes[0], AssetHandle::Weak(..)));
    assert_eq!(project.blobs().len(), 1);

    let second = project.load(id).expect("the navmesh loads from the cache");
    assert_eq!(second.data(), first.data());
    assert_eq!(project.blobs().len(), 1);
    assert!(corners(&second, WEST, EAST).len() >= 2);
}

#[test]
fn changing_the_settings_derives_a_new_blob() {
    let mut project = Project::new("settings-key");
    let level = project.save("level", &scene(vec![floor()]));
    let navmesh = navmesh_of(&[level]);
    let wider = navmesh
        .clone()
        .with_settings(NavMeshSettings::new().with_agent_radius(0.5));

    let inputs = project.inputs(&navmesh).unwrap();
    assert_ne!(
        navmesh.derived_key(&inputs).unwrap(),
        wider.derived_key(&inputs).unwrap()
    );
}

#[test]
fn a_navmesh_with_no_scenes_does_not_derive() {
    assert!(!NavMesh::new_asset().is_derived());
    assert!(NavMesh::new_asset().data().is_none());
    assert!(navmesh_of(&[AssetId::new()]).is_derived());
}

#[test]
fn a_nil_scene_fails_naming_its_index() {
    let mut project = Project::new("nil");
    let level = project.save("level", &scene(vec![floor()]));
    let navmesh = navmesh_of(&[level, AssetId::nil()]);

    assert_eq!(navmesh.derived_inputs(), vec![level]);
    let error = project.inputs(&navmesh).unwrap_err();
    assert!(format!("{error:#}").contains("scene 1"), "got: {error:#}");
}

#[test]
fn the_navmesh_encoding_is_unchanged() {
    let navmesh =
        navmesh_of(&[AssetId::from_simple_hex("6a1f0c7e2b9d4e8f9a3c5d7e1f2a4b6c").unwrap()])
            .with_settings(
                NavMeshSettings::new()
                    .with_agent_radius(0.4)
                    .with_agent_height(1.7)
                    .with_max_slope(40.0)
                    .with_step_height(0.3)
                    .with_cell_size(0.15)
                    .with_cell_height(0.1),
            );
    check_encoding(
        &navmesh,
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding")),
    );
}

fn floor_data() -> NavMeshData {
    let mut geometry = NavMeshGeometry::new();
    geometry.add_box(
        Affine3A::from_translation(Vec3::new(0.0, -0.5, 0.0)),
        Vec3::new(5.0, 0.5, 5.0),
    );
    bake(&geometry, &NavMeshSettings::new()).unwrap()
}

#[test]
fn a_navmesh_component_gets_a_status_once_its_mesh_is_loaded() {
    let mut app = app();
    let data = floor_data();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let missing = app.main_mut().world_mut().spawn(NavMeshComponent {
        handle: AssetHandle::weak(AssetId::new()),
    });
    let handle = server.add(NavMesh::baked(NavMeshSettings::new(), data.clone()));
    let entity = app
        .main_mut()
        .world_mut()
        .spawn(NavMeshComponent { handle });

    app.update();
    app.update();

    let world = app.main_mut().world_mut();
    let Bounds { min, max } = bounds(&data);
    assert_eq!(
        world.get_component_for_entity::<NavMeshStatus>(entity),
        Some(&NavMeshStatus {
            polygons: data.polygons.len() as u32,
            min,
            max,
        })
    );
    assert!(
        world
            .get_component_for_entity::<NavMeshStatus>(missing)
            .is_none()
    );
}

#[test]
fn a_status_goes_away_when_the_component_points_at_a_mesh_that_is_not_loaded() {
    let mut app = app();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    let handle = server.add(NavMesh::baked(NavMeshSettings::new(), floor_data()));
    let entity = app
        .main_mut()
        .world_mut()
        .spawn(NavMeshComponent { handle });
    app.update();
    app.update();
    assert!(
        app.main_mut()
            .world_mut()
            .get_component_for_entity::<NavMeshStatus>(entity)
            .is_some()
    );

    app.main_mut().world_mut().insert(
        NavMeshComponent {
            handle: AssetHandle::weak(AssetId::new()),
        },
        entity,
    );
    app.update();
    app.update();

    assert!(
        app.main_mut()
            .world_mut()
            .get_component_for_entity::<NavMeshStatus>(entity)
            .is_none()
    );
}

#[test]
fn an_agent_sent_while_its_navmesh_loads_waits_and_then_routes_on_it() {
    let mut project = Project::new("agent-waits");
    let level = project.save("level", &scene(vec![floor(), obstacle()]));
    let id = project.save("navmesh", &navmesh_of(&[level]));
    let mut app = app();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    server.set_derived_cache(DerivedDataCache::in_project(&project.root));
    server.publish_content_source(project.content_root(), project.registry.clone());
    app.insert_resource(project.registry.clone());
    let world = app.main_mut().world_mut();
    world.spawn((Transform::from_translation(WEST), NavPoint::default()));
    let east = world.spawn((Transform::from_translation(EAST), NavPoint::default()));
    let agent = world.spawn((Transform::from_translation(WEST), NavAgent::new()));
    app.update();
    app.update();

    let handle = server.load::<NavMesh>(id);
    let world = app.main_mut().world_mut();
    world.spawn(NavMeshComponent { handle });
    world.insert(NavDestination::to_point(east), agent);
    let deadline = Instant::now() + Duration::from_secs(30);
    let route = loop {
        app.update();
        let world = app.main_mut().world_mut();
        if let Some(route) = world.get_component_for_entity::<NavRoute>(agent) {
            break route.clone();
        }
        assert!(
            world
                .get_component_for_entity::<NavDestination>(agent)
                .is_some(),
            "the agent gave up before its navmesh landed"
        );
        assert!(Instant::now() < deadline, "the navmesh never landed");
        std::thread::sleep(Duration::from_millis(1));
    };

    assert_eq!(route.planned_on(), Some(id));
    assert!(
        route.remaining().iter().any(|corner| corner.z.abs() > 0.5),
        "{route:?}"
    );
}

#[test]
fn an_agent_sent_elsewhere_while_its_navmesh_loads_stops_waits_and_then_routes_on_it() {
    let mut project = Project::new("agent-redirected");
    let level = project.save("level", &scene(vec![floor(), obstacle()]));
    let id = project.save("navmesh", &navmesh_of(&[level]));
    let mut app = app();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    server.set_derived_cache(DerivedDataCache::in_project(&project.root));
    server.publish_content_source(project.content_root(), project.registry.clone());
    app.insert_resource(project.registry.clone());
    let world = app.main_mut().world_mut();
    let west = world.spawn((Transform::from_translation(WEST), NavPoint::default()));
    let north = world.spawn((
        Transform::from_translation(Vec3::new(-3.0, 0.0, 3.0)),
        NavPoint::default(),
        NavLinks {
            links: vec![NavLink {
                to: SceneEntityRef::Entity(west),
                one_way: false,
            }],
        },
    ));
    let east = world.spawn((Transform::from_translation(EAST), NavPoint::default()));
    let agent = world.spawn((Transform::from_translation(WEST), NavAgent::new()));
    app.update();
    app.update();
    app.main_mut()
        .world_mut()
        .insert(NavDestination::to_point(north), agent);
    app.update();
    let world = app.main_mut().world_mut();
    assert_eq!(
        world
            .get_component_for_entity::<NavRoute>(agent)
            .map(NavRoute::planned_on),
        Some(None)
    );

    let handle = server.load::<NavMesh>(id);
    world.spawn(NavMeshComponent { handle });
    world.insert(NavDestination::to_point(east), agent);
    let deadline = Instant::now() + Duration::from_secs(30);
    let route = loop {
        app.update();
        let world = app.main_mut().world_mut();
        let route = world.get_component_for_entity::<NavRoute>(agent).cloned();
        if let Some(route) = route.filter(|route| route.planned_on().is_some()) {
            break route;
        }
        assert!(
            world
                .get_component_for_entity::<NavDestination>(agent)
                .is_some(),
            "the agent gave up before its navmesh landed"
        );
        assert!(
            world.get_component_for_entity::<NavRoute>(agent).is_none(),
            "the agent kept its stale route"
        );
        assert_eq!(
            world
                .get_component_for_entity::<NavSteering>(agent)
                .unwrap()
                .velocity,
            Vec3::ZERO
        );
        assert!(Instant::now() < deadline, "the navmesh never landed");
        std::thread::sleep(Duration::from_millis(1));
    };

    assert_eq!(route.planned_on(), Some(id));
    assert_eq!(route.destination(), east);
}

enum MeshSwap {
    OnTheSameHolder,
    OnANewHolder,
}

fn a_route_is_kept_while_the_next_mesh_loads(swap: MeshSwap) {
    let mut project = Project::new("agent-keeps-route");
    let level = project.save("level", &scene(vec![floor(), obstacle()]));
    let next = project.save("navmesh", &navmesh_of(&[level]));
    let mut app = app();
    let server = app.get_resource::<AssetServer>().unwrap().clone();
    server.set_derived_cache(DerivedDataCache::in_project(&project.root));
    server.publish_content_source(project.content_root(), project.registry.clone());
    app.insert_resource(project.registry.clone());
    let first = server.add(NavMesh::baked(NavMeshSettings::new(), floor_data()));
    let first_id = first.id();
    let world = app.main_mut().world_mut();
    world.spawn((Transform::from_translation(WEST), NavPoint::default()));
    let east = world.spawn((Transform::from_translation(EAST), NavPoint::default()));
    let agent = world.spawn((Transform::from_translation(WEST), NavAgent::new()));
    let holder = world.spawn(NavMeshComponent { handle: first });
    app.update();
    app.update();
    app.main_mut()
        .world_mut()
        .insert(NavDestination::to_point(east), agent);
    app.update();
    assert_eq!(
        app.main_mut()
            .world_mut()
            .get_component_for_entity::<NavRoute>(agent)
            .map(NavRoute::planned_on),
        Some(Some(first_id))
    );

    let handle = server.load::<NavMesh>(next);
    let world = app.main_mut().world_mut();
    match swap {
        MeshSwap::OnTheSameHolder => world.insert(NavMeshComponent { handle }, holder),
        MeshSwap::OnANewHolder => {
            world.despawn(holder);
            world.spawn(NavMeshComponent { handle });
        }
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    let route = loop {
        app.update();
        let world = app.main_mut().world_mut();
        assert!(
            world
                .get_component_for_entity::<NavDestination>(agent)
                .is_some(),
            "the agent gave up before the next navmesh landed"
        );
        let route = world
            .get_component_for_entity::<NavRoute>(agent)
            .cloned()
            .expect("the agent kept its route while the next navmesh loaded");
        if route.planned_on() == Some(next) {
            break route;
        }
        assert_eq!(route.planned_on(), Some(first_id));
        assert!(Instant::now() < deadline, "the navmesh never landed");
        std::thread::sleep(Duration::from_millis(1));
    };

    assert_eq!(route.destination(), east);
}

#[test]
fn a_route_is_kept_while_its_holders_next_mesh_loads() {
    a_route_is_kept_while_the_next_mesh_loads(MeshSwap::OnTheSameHolder);
}

#[test]
fn a_route_is_kept_while_a_new_holders_mesh_loads() {
    a_route_is_kept_while_the_next_mesh_loads(MeshSwap::OnANewHolder);
}
