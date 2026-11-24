use concerto_animation::{
    evaluation::AnimationGraphContext,
    graph::AnimationGraph,
    node::{AnimationNode, InputSlots, ReflectAnimationNode},
    player::{AnimationHandleComponent, AnimationPlayer},
    plugin::{AnimationAppExt, AnimationPlugin, register_animation_types},
    pose::{
        AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, JointPose, RootMotion,
        WeightedMotion,
    },
};
use concerto_app::{App, plugins::AssetManagerPlugin};
use concerto_ecs::{
    Entity,
    component::scene::SceneEntityRef,
    system::{executor::single_thread::SingleThreadedExecutor, schedule::Schedules},
    world::World,
};
use concerto_foundation::{
    assets::{Asset, AssetId, asset_store::AssetStore, handle::AssetHandle},
    time::Time,
    transform::Transform,
};
use concerto_mesh::skeleton::{Skeleton, SkeletonComponent};
use concerto_name::name;
use concerto_reflect::{GetTypeRegistration, Reflect, TypeRegistry};
use glam::{Mat4, Quat, Vec3};
use uuid::Uuid;

use crate::plugin::ProceduralAnimationPlugin;

pub(crate) struct Bone {
    pub(crate) name: &'static str,
    pub(crate) parent: Option<u16>,
    pub(crate) joint: JointPose,
}

pub(crate) fn bone(
    name: &'static str,
    parent: Option<u16>,
    translation: Vec3,
    rotation: Quat,
) -> Bone {
    Bone {
        name,
        parent,
        joint: JointPose {
            translation,
            rotation,
            scale: Vec3::ONE,
        },
    }
}

pub(crate) fn skeleton_of(bones: &[Bone]) -> Skeleton {
    Skeleton::new(
        vec![Mat4::IDENTITY; bones.len()],
        bones.iter().map(|bone| bone.parent).collect(),
        bones.iter().map(|bone| bone.name.into()).collect(),
        bones.iter().map(|bone| bone.joint.clone()).collect(),
        (0..bones.len() as u128).map(Uuid::from_u128).collect(),
    )
    .unwrap()
}

pub(crate) fn input_event() -> FiredEvent {
    FiredEvent {
        clip: AssetId::from_path("procedural_input"),
        key: 1,
        time: 0.3,
        weight: 0.5,
        mirrored: false,
    }
}

pub(crate) fn assert_near(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() < tolerance,
        "expected {expected}, got {actual}"
    );
}

pub(crate) fn angle(from: Quat, to: Quat) -> f32 {
    let between = from.inverse() * to;
    2.0 * between.xyz().length().atan2(between.w.abs())
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Frame {
    pub(crate) joints: Vec<JointPose>,
    pub(crate) root_motion: RootMotion,
    pub(crate) events: Vec<FiredEvent>,
}

#[derive(Default, Reflect)]
#[reflect(AnimationNode)]
pub(crate) struct Capture {}

impl AnimationNode for Capture {
    type State = Option<Frame>;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = ();

    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("pose")])
    }

    fn create_state(&self, _: &(), _context: &AnimationGraphContext<'_>) -> Option<Frame> {
        None
    }

    fn update(&self, _: &mut Option<Frame>, _: f32, _: &(), _: &AnimationGraphContext<'_>) {}

    fn root_motion(
        &self,
        state: &mut Option<Frame>,
        _: &(),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        inputs: &[WeightedMotion],
    ) -> RootMotion {
        let motion = inputs
            .first()
            .map_or(RootMotion::IDENTITY, |input| input.motion);
        *state = Some(Frame {
            joints: Vec::new(),
            root_motion: motion,
            events: Vec::new(),
        });
        motion
    }

    fn evaluate(
        &self,
        state: &mut Option<Frame>,
        _: &(),
        _: &AnimationGraphContext<'_>,
        _: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        _: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        let Some(input) = evaluated_inputs.first() else {
            return;
        };
        output.copy_from(&input.frame);
        *state = Some(Frame {
            joints: input.frame.joints().to_vec(),
            root_motion: state
                .as_ref()
                .map_or(RootMotion::IDENTITY, |frame| frame.root_motion),
            events: input.frame.events().to_vec(),
        });
    }

    fn reset(&self, state: &mut Option<Frame>) {
        *state = None;
    }
}

pub(crate) fn app_with<I: AnimationNode + GetTypeRegistration>() -> App {
    let mut app = App::new();
    app.register_plugin(AssetManagerPlugin);
    app.insert_resource(Time::new());
    app.register_plugin(AnimationPlugin);
    app.register_plugin(ProceduralAnimationPlugin);
    app.register_animation_node::<Capture>();
    app.register_animation_node::<I>();
    app.main_mut().world_mut().register_component::<Transform>();
    app
}

pub(crate) fn captured_graph<N, I>(node: N, input: I) -> AnimationGraph
where
    N: AnimationNode + GetTypeRegistration,
    I: AnimationNode + GetTypeRegistration,
{
    let mut registry = TypeRegistry::default();
    register_animation_types(&mut registry);
    registry.register::<N>();
    registry.register::<I>();
    registry.register::<Capture>();
    let mut graph = AnimationGraph::new();
    graph
        .result_node()
        .input(Capture {})
        .input(node)
        .add_input(input);
    graph.make_live(&registry, None).unwrap();
    graph
}

pub(crate) fn insert_asset<T: Asset>(world: &mut World, id: AssetId, asset: T) -> AssetHandle<T> {
    world
        .get_resource_mut::<AssetStore<T>>()
        .unwrap()
        .insert(id, asset);
    AssetHandle::weak(id)
}

pub(crate) struct Rigged {
    pub(crate) player: Entity,
    pub(crate) bones: Vec<Entity>,
}

pub(crate) fn spawn_rigged(
    world: &mut World,
    skeleton: &AssetHandle<Skeleton>,
    graph: &AssetHandle<AnimationGraph>,
    player: AnimationPlayer,
    bone_count: usize,
) -> Rigged {
    let bones: Vec<Entity> = (0..bone_count)
        .map(|_| world.spawn(Transform::IDENTITY))
        .collect();
    let player = world.spawn((
        player,
        AnimationHandleComponent::new(graph.clone()),
        SkeletonComponent::new(
            skeleton.clone(),
            bones
                .iter()
                .map(|bone| SceneEntityRef::Entity(*bone))
                .collect(),
            None,
        ),
    ));
    Rigged { player, bones }
}

pub(crate) fn compile(world: &mut World) {
    let schedules = world.remove_resource::<Schedules>().unwrap();
    let compiled = schedules.compile::<SingleThreadedExecutor>(world);
    world.insert_resource(compiled);
}

pub(crate) fn read_state<N: AnimationNode, R>(
    world: &mut World,
    player: Entity,
    read: impl FnOnce(&N, &N::State) -> R,
) -> Option<R> {
    let mut read = Some(read);
    let mut result = None;
    world.resource_scope::<AssetStore<AnimationGraph>>(|world, graphs| {
        world
            .get_component_for_entity::<AnimationPlayer>(player)
            .unwrap()
            .for_each_state::<N>(graphs, |node, state, _| {
                if let Some(read) = read.take() {
                    result = Some(read(node, state));
                }
            });
    });
    result
}

pub(crate) fn write_state<N: AnimationNode, R>(
    world: &mut World,
    player: Entity,
    write: impl FnOnce(&N, &mut N::State) -> R,
) -> Option<R> {
    let mut write = Some(write);
    let mut result = None;
    world.resource_scope::<AssetStore<AnimationGraph>>(|world, graphs| {
        world
            .get_component_for_entity_mut::<AnimationPlayer>(player)
            .unwrap()
            .for_each_state_mut::<N>(graphs, |node, state, _| {
                if let Some(write) = write.take() {
                    result = Some(write(node, state));
                }
            });
    });
    result
}

pub(crate) fn captured(world: &mut World, player: Entity) -> Frame {
    read_state::<Capture, _>(world, player, |_, frame| frame.clone())
        .flatten()
        .expect("the capture node evaluated its input")
}
