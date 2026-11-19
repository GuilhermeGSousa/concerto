use concerto_app::{
    App,
    plugins::AssetManagerPlugin,
    schedule_groups::{First, LateUpdate, Update},
};
use concerto_debug_gizmos::{ComponentGizmo, DebugGizmos, DebugGizmosPlugin, GizmoStorage};
use concerto_ecs::{
    Entity, IntoSystem, Query, System, World,
    component::scene::SceneEntityRef,
    system::{
        executor::single_thread::SingleThreadedExecutor, input::StaticSystemInput,
        schedule::Schedules,
    },
};
use concerto_foundation::{
    assets::asset_server::AssetServer,
    transform::{GlobalTransform, Transform},
};
use concerto_navigation::{
    NavAgent, NavDestination, NavLink, NavLinks, NavMesh, NavMeshComponent, NavMeshData,
    NavMeshSettings, NavPoint, NavPolygon, NavigationPlugin,
};
use glam::{Mat4, Vec3};

use crate::{NavMeshGizmo, NavigationGizmos, NavigationGizmosPlugin};

fn two_quads() -> NavMeshData {
    NavMeshData {
        vertices: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(2.0, 0.0, 1.0),
        ],
        polygons: vec![
            NavPolygon {
                vertices: vec![0, 1, 4, 3],
            },
            NavPolygon {
                vertices: vec![1, 2, 5, 4],
            },
        ],
    }
}

struct Stage {
    app: App,
    destination: Entity,
}

impl Stage {
    fn new() -> Self {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.register_plugin(NavigationPlugin);
        app.register_plugin(DebugGizmosPlugin);
        app.register_plugin(NavigationGizmosPlugin);

        let world = app.main_mut().world_mut();
        world.register_component::<Transform>();
        let destination = world.spawn((
            NavPoint::default(),
            Transform::IDENTITY,
            GlobalTransform::new(Mat4::IDENTITY),
        ));
        world.spawn((
            NavPoint::default(),
            NavLinks {
                links: vec![NavLink {
                    to: SceneEntityRef::Entity(destination),
                    one_way: true,
                }],
            },
            Transform::from_translation(Vec3::new(4.0, 0.0, 0.0)),
            GlobalTransform::new(Mat4::from_translation(Vec3::new(4.0, 0.0, 0.0))),
        ));

        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);
        world.run_schedule(LateUpdate);

        Self { app, destination }
    }

    fn world(&mut self) -> &mut World {
        self.app.main_mut().world_mut()
    }

    fn tick(&mut self) {
        let world = self.world();
        world.run_schedule(First);
        world.run_schedule(Update);
        world.run_schedule(LateUpdate);
    }

    fn show(&mut self, shown: NavigationGizmos) {
        self.world().insert_resource(shown);
        self.tick();
    }

    fn draw_nav_mesh_gizmos(&mut self) {
        let world = self.world();
        let mut system = draw_nav_mesh_gizmos.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    fn segments(&mut self) -> usize {
        self.world()
            .get_resource::<GizmoStorage>()
            .unwrap()
            .segment_count()
    }
}

fn draw_nav_mesh_gizmos(
    navmeshes: Query<(Entity, &NavMeshComponent)>,
    input: StaticSystemInput<<NavMeshGizmo as ComponentGizmo>::Input>,
    mut gizmos: DebugGizmos,
) {
    for (entity, navmesh) in navmeshes.iter() {
        NavMeshGizmo::draw(
            entity,
            navmesh,
            &GlobalTransform::new(Mat4::IDENTITY),
            &input,
            &mut gizmos,
        );
    }
}

const NOTHING: NavigationGizmos = NavigationGizmos {
    graph: false,
    routes: false,
    mesh: false,
};

#[test]
fn each_layer_is_drawn_from_its_data_while_its_flag_is_on_and_not_when_off() {
    let mut stage = Stage::new();

    stage.show(NavigationGizmos {
        graph: true,
        ..NOTHING
    });
    assert!(stage.segments() > 0, "the graph");

    stage.show(NavigationGizmos {
        routes: true,
        ..NOTHING
    });
    assert_eq!(stage.segments(), 0, "no agent has a route yet");
    let destination = stage.destination;
    stage.world().spawn((
        NavAgent::new(),
        NavDestination::to_point(destination),
        GlobalTransform::new(Mat4::from_translation(Vec3::new(8.0, 0.0, 0.0))),
    ));
    stage.tick();
    assert!(stage.segments() > 0, "the agent's route");

    stage.show(NavigationGizmos {
        mesh: true,
        ..NOTHING
    });
    assert_eq!(stage.segments(), 0, "no navmesh has loaded yet");
    let server = stage.world().get_resource::<AssetServer>().unwrap().clone();
    let handle = server.add(NavMesh::baked(NavMeshSettings::new(), two_quads()));
    stage.world().spawn(NavMeshComponent { handle });
    stage.tick();
    stage.tick();
    assert!(stage.segments() > 0, "the navmesh");

    stage.show(NOTHING);
    assert_eq!(stage.segments(), 0);

    stage.draw_nav_mesh_gizmos();
    assert_eq!(
        stage.segments(),
        7,
        "the navmesh gizmo draws each edge, the shared one once"
    );
}
