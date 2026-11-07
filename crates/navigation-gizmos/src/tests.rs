use concerto_app::{
    App,
    schedule_groups::{First, LateUpdate},
};
use concerto_debug_gizmos::{DebugGizmosPlugin, GizmoStorage};
use concerto_ecs::{
    Entity, World,
    component::scene::SceneEntityRef,
    system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
};
use concerto_foundation::transform::{GlobalTransform, Transform};
use concerto_navigation::{
    NavGraph, NavLink, NavLinks, NavPoint, NavPointRecord, NavigationPlugin,
};
use glam::{Mat4, Quat, Vec3};

use crate::{
    NavigationGizmos, NavigationGizmosPlugin,
    gizmos::{
        DISABLED_COLOR, ENABLED_COLOR, LINK_COLOR, NavShape, ROUTE_COLOR, graph_shapes,
        link_shapes, route_shapes,
    },
};

const EPS: f32 = 1.0e-4;
const CIRCLE_SEGMENTS: usize = 32;

fn entity(index: u32) -> Entity {
    Entity::parse_id(&format!("{index}v1")).unwrap()
}

fn link(to: Entity, one_way: bool) -> NavLink {
    NavLink {
        to: SceneEntityRef::Entity(to),
        one_way,
    }
}

fn circles(shapes: &[NavShape]) -> Vec<NavShape> {
    shapes
        .iter()
        .copied()
        .filter(|shape| matches!(shape, NavShape::Circle { .. }))
        .collect()
}

fn lines_touching(shapes: &[NavShape], at: Vec3) -> usize {
    shapes
        .iter()
        .filter(|shape| match shape {
            NavShape::Line { start, end, color } => {
                *color == LINK_COLOR && (start.distance(at) < 0.5 || end.distance(at) < 0.5)
            }
            NavShape::Circle { .. } => false,
        })
        .count()
}

fn segments(shapes: &[NavShape]) -> usize {
    shapes
        .iter()
        .map(|shape| match shape {
            NavShape::Circle { .. } => CIRCLE_SEGMENTS,
            NavShape::Line { .. } => 1,
        })
        .sum()
}

#[test]
fn an_enabled_point_is_green_and_a_disabled_one_grey() {
    let [on, off] = [entity(1), entity(2)];
    let graph = NavGraph::build([
        NavPointRecord::new(on, Vec3::ZERO),
        NavPointRecord::new(off, Vec3::new(5.0, 0.0, 0.0)).with_enabled(false),
    ]);

    let shapes = graph_shapes(&graph);

    assert_eq!(
        circles(&shapes),
        vec![
            NavShape::Circle {
                center: Vec3::ZERO,
                color: ENABLED_COLOR,
            },
            NavShape::Circle {
                center: Vec3::new(5.0, 0.0, 0.0),
                color: DISABLED_COLOR,
            },
        ]
    );
}

#[test]
fn a_point_draws_a_facing_line_along_its_local_z() {
    let point = entity(1);
    let graph = NavGraph::build([NavPointRecord::new(point, Vec3::ZERO)
        .with_rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2))]);

    let shapes = graph_shapes(&graph);

    assert!(shapes.iter().any(|shape| matches!(
        shape,
        NavShape::Line { start, end, color }
            if *color == ENABLED_COLOR
                && start.distance(Vec3::ZERO) < EPS
                && end.x > 0.0
                && end.z.abs() < EPS
    )));
}

#[test]
fn only_a_one_way_link_gets_an_arrowhead_at_its_to_end() {
    let from = Vec3::ZERO;
    let to = Vec3::new(0.0, 0.0, 4.0);

    let two_way = link_shapes(from, to, false);
    let one_way = link_shapes(from, to, true);

    assert_eq!(
        two_way,
        vec![NavShape::Line {
            start: from,
            end: to,
            color: LINK_COLOR,
        }]
    );
    assert_eq!(one_way.len(), 3);
    assert_eq!(one_way[0], two_way[0]);
    for head in &one_way[1..] {
        let NavShape::Line { start, end, color } = *head else {
            panic!("an arrowhead is made of lines");
        };
        assert_eq!(color, LINK_COLOR);
        assert!(start.distance(to) < 0.5, "{start:?} is near the to end");
        assert!(end.z < start.z, "{end:?} points back towards the from end");
        assert!(end.x.abs() > EPS, "{end:?} opens to the side");
    }
}

#[test]
fn the_graph_draws_an_arrowhead_only_where_a_link_is_one_way() {
    let [a, b, c] = [entity(1), entity(2), entity(3)];
    let [pa, pb, pc] = [
        Vec3::ZERO,
        Vec3::new(4.0, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 4.0),
    ];
    let graph = NavGraph::build([
        NavPointRecord::new(a, pa)
            .with_link(link(b, false))
            .with_link(link(c, true)),
        NavPointRecord::new(b, pb),
        NavPointRecord::new(c, pc),
    ]);

    let shapes = graph_shapes(&graph);

    assert_eq!(lines_touching(&shapes, pb), 1);
    assert_eq!(lines_touching(&shapes, pc), 3);
}

#[test]
fn a_route_is_a_line_from_the_agent_through_each_remaining_waypoint() {
    let agent = Vec3::new(1.0, 0.0, 1.0);
    let remaining = [Vec3::new(4.0, 0.0, 4.0), Vec3::new(4.0, 0.0, 12.0)];

    let shapes = route_shapes(agent, &remaining);

    assert_eq!(
        shapes,
        vec![
            NavShape::Line {
                start: agent,
                end: remaining[0],
                color: ROUTE_COLOR,
            },
            NavShape::Line {
                start: remaining[0],
                end: remaining[1],
                color: ROUTE_COLOR,
            },
        ]
    );
    assert_ne!(ROUTE_COLOR, ENABLED_COLOR);
    assert_ne!(ROUTE_COLOR, DISABLED_COLOR);
    assert_ne!(ROUTE_COLOR, LINK_COLOR);
}

#[test]
fn a_route_with_no_waypoints_left_draws_nothing() {
    assert!(route_shapes(Vec3::ZERO, &[]).is_empty());
}

struct Stage {
    app: App,
}

impl Stage {
    fn new() -> Self {
        let mut app = App::new();
        app.register_plugin(NavigationPlugin);
        app.register_plugin(DebugGizmosPlugin);
        app.register_plugin(NavigationGizmosPlugin);

        let world = app.main_mut().world_mut();
        world.register_component::<Transform>();
        let a = world.spawn((
            NavPoint::default(),
            Transform::IDENTITY,
            GlobalTransform::new(Mat4::IDENTITY),
        ));
        world.spawn((
            NavPoint::default(),
            NavLinks {
                links: vec![link(a, true)],
            },
            Transform::from_translation(Vec3::new(4.0, 0.0, 0.0)),
            GlobalTransform::new(Mat4::from_translation(Vec3::new(4.0, 0.0, 0.0))),
        ));

        let schedules = world.remove_resource::<Schedules>().unwrap();
        let compiled = schedules.compile::<SingleThreadedExecutor>(world);
        world.insert_resource(compiled);
        world.run_schedule(LateUpdate);

        Self { app }
    }

    fn world(&mut self) -> &mut World {
        self.app.main_mut().world_mut()
    }

    fn tick(&mut self) {
        let world = self.world();
        world.run_schedule(First);
        world.run_schedule(LateUpdate);
    }

    fn segments(&mut self) -> usize {
        self.world()
            .get_resource::<GizmoStorage>()
            .unwrap()
            .segment_count()
    }

    fn expected_segments(&mut self) -> usize {
        segments(&graph_shapes(
            self.world().get_resource::<NavGraph>().unwrap(),
        ))
    }
}

#[test]
fn the_graph_is_drawn_while_its_flag_is_on() {
    let mut stage = Stage::new();

    assert_eq!(
        stage.expected_segments(),
        2 * (CIRCLE_SEGMENTS + 1) + 3,
        "two points with facing lines, and a one-way link"
    );
    assert_eq!(stage.segments(), stage.expected_segments());
}

#[test]
fn with_the_graph_flag_off_nothing_is_drawn() {
    let mut stage = Stage::new();

    stage
        .world()
        .get_resource_mut::<NavigationGizmos>()
        .unwrap()
        .graph = false;
    stage.tick();

    assert_eq!(stage.segments(), 0);
}
