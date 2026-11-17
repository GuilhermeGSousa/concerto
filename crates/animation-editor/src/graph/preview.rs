use concerto_animation::{
    graph::{AnimationGraph, NodePathStep},
    node::{NodeReport, state_machine::AnimationFSMTrigger},
    param::ReflectParam,
    player::{AnimationHandleComponent, AnimationPlayer},
    root::AnimationRootBone,
    target::animate_targets,
};
use concerto_app::{
    App,
    edit_time::AtPreviewTime,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Entity, Event, IntoSystemConfig, Query, Res, ResMut, Resource, With,
    command::CommandQueue,
    events::event_writer::EventWriter,
    reflect::AppTypeRegistry,
    signal::{On, Signal},
};
use concerto_foundation::{
    assets::{AssetId, asset_server::AssetServer, asset_store::AssetStore, handle::AssetHandle},
    time::Time,
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::skeleton::SkeletonComponent;
use concerto_name::Name;
use concerto_reflect::{Reflect, ReflectedValue, TypeRegistry, apply_value};

use super::{
    document::{StateValue, TransitionValue},
    params::GraphParameters,
    stage::GraphPreview,
};
use crate::preview::{ClipPreviewRoot, first_rooted, preview_stage};

/// One graph for a preview to play in place of its file: the asset's id and the graph as a save writes it.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphBytes {
    pub id: AssetId,
    pub bytes: Vec<u8>,
}

impl GraphBytes {
    /// `graph` as a save writes it, to stand in for the asset `id`.
    pub fn of(id: AssetId, graph: &AnimationGraph) -> anyhow::Result<Self> {
        Ok(Self {
            id,
            bytes: bincode::serialize(graph)?,
        })
    }
}

/// Stands each of `graphs` in for its asset, puts a graph shown before and left out now back to its file, and restarts every player on the graph `play`.
pub struct ShowGraphs {
    pub graphs: Vec<GraphBytes>,
    pub play: AssetId,
}
impl Signal for ShowGraphs {}

/// Sets the parameter `name` on every player's blackboard to `value`, the stored form of a value of the type `type_name`.
#[derive(Clone, Debug, PartialEq)]
pub struct SetGraphParam {
    pub name: Name,
    pub type_name: String,
    pub value: ReflectedValue,
}
impl Signal for SetGraphParam {}

/// Sets whether the players advance and how fast time passes for them.
pub struct SetGraphPlayback {
    pub playing: bool,
    pub speed: f32,
}
impl Signal for SetGraphPlayback {}

/// One playing node of a [`GraphPreviewState`]: its path from the graph the preview plays, its weight and what it reports.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewNode {
    pub path: Vec<NodePathStep>,
    pub weight: f32,
    pub report: NodeReport,
}

/// What a graph preview's first player plays, written each frame the preview plays; `nodes` is empty while no player plays the graph.
#[derive(Event, Clone, Debug, Default, PartialEq)]
pub struct GraphPreviewState {
    pub nodes: Vec<PreviewNode>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Restart {
    Settled,
    Loading,
    Starting,
}

#[derive(Resource)]
struct GraphStage {
    graphs: Vec<AssetHandle<AnimationGraph>>,
    play: Option<AssetHandle<AnimationGraph>>,
    restart: Restart,
    params: Vec<SetGraphParam>,
    playing: bool,
}

impl Default for GraphStage {
    fn default() -> Self {
        Self {
            graphs: Vec::new(),
            play: None,
            restart: Restart::Settled,
            params: Vec::new(),
            playing: true,
        }
    }
}

fn param_value(types: &TypeRegistry, param: &SetGraphParam) -> Option<Box<dyn Reflect>> {
    let Some(found) = types.iter().find_map(|registration| {
        registration
            .data::<ReflectParam>()
            .filter(|data| data.value_type_name() == param.type_name)
            .copied()
    }) else {
        log::warn!(
            "Parameter {} is a {}, which no registered Param names",
            param.name,
            param.type_name
        );
        return None;
    };
    let mut value = found.default_value();
    for warning in apply_value(value.as_mut(), &param.value) {
        log::warn!("Parameter {}: {warning}", param.name);
    }
    Some(value)
}

fn show_graphs(
    on: On<ShowGraphs>,
    server: Res<AssetServer>,
    mut stage: ResMut<GraphStage>,
    players: Query<&mut AnimationPlayer>,
) {
    let shown = on.signal();
    for mut player in players.iter() {
        player.set_paused(true);
    }
    let mut graphs = Vec::new();
    for graph in &shown.graphs {
        match bincode::deserialize::<AnimationGraph>(&graph.bytes) {
            Ok(decoded) => graphs.push(server.insert_loaded(graph.id, decoded)),
            Err(error) => log::warn!(
                "Graph {} cannot be previewed: {error}",
                graph.id.simple_hex()
            ),
        }
    }
    let files = server.registry();
    for dropped in &stage.graphs {
        let id = dropped.id();
        let filed = files.as_ref().is_some_and(|files| files.get(id).is_some());
        if filed && !shown.graphs.iter().any(|graph| graph.id == id) {
            server.reload::<AnimationGraph>(id);
        }
    }
    stage.play = Some(server.load(shown.play));
    stage.graphs = graphs;
    stage.restart = Restart::Loading;
}

fn set_graph_param(
    on: On<SetGraphParam>,
    types: Res<AppTypeRegistry>,
    mut stage: ResMut<GraphStage>,
    players: Query<&mut AnimationPlayer>,
) {
    let param = on.signal();
    let types = types.read();
    for mut player in players.iter() {
        if let Some(value) = param_value(&types, param) {
            player.set_boxed_param(param.name, value);
        }
    }
    stage
        .params
        .retain(|held| held.name != param.name || held.type_name != param.type_name);
    stage.params.push(param.clone());
}

fn set_graph_playback(
    on: On<SetGraphPlayback>,
    mut stage: ResMut<GraphStage>,
    mut time: ResMut<Time>,
    players: Query<&mut AnimationPlayer>,
) {
    let playback = on.signal();
    stage.playing = playback.playing;
    time.set_time_scale(playback.speed);
    if stage.restart != Restart::Settled {
        return;
    }
    for mut player in players.iter() {
        player.set_paused(!playback.playing);
    }
}

fn start_graph_players(
    mut stage: ResMut<GraphStage>,
    server: Res<AssetServer>,
    types: Res<AppTypeRegistry>,
    players: Query<(
        Entity,
        &mut AnimationPlayer,
        Option<&AnimationHandleComponent>,
    )>,
    mut cmd: CommandQueue,
) {
    let Some(play) = stage.play.clone() else {
        return;
    };
    if server.is_loading() {
        return;
    }
    let restart = stage.restart;
    for (entity, mut player, started) in players.iter() {
        if started.is_none() {
            let types = types.read();
            for param in &stage.params {
                if let Some(value) = param_value(&types, param) {
                    player.set_boxed_param(param.name, value);
                }
            }
            player.set_paused(!stage.playing);
            cmd.insert(AnimationHandleComponent::new(play.clone()), entity);
            continue;
        }
        match restart {
            Restart::Settled => {}
            Restart::Loading => {
                cmd.insert(AnimationHandleComponent::new(play.clone()), entity);
            }
            Restart::Starting => player.set_paused(!stage.playing),
        }
    }
    match restart {
        Restart::Settled => {}
        Restart::Loading => stage.restart = Restart::Starting,
        Restart::Starting => stage.restart = Restart::Settled,
    }
}

fn report_graph_state(
    stage: Res<GraphStage>,
    players: Query<(Entity, &AnimationPlayer), With<AnimationHandleComponent>>,
    graphs: Option<Res<AssetStore<AnimationGraph>>>,
    mut states: EventWriter<GraphPreviewState>,
) {
    if !stage.playing || stage.restart != Restart::Settled {
        return;
    }
    let mut state = GraphPreviewState::default();
    let first = players
        .iter()
        .filter(|(_, player)| !player.is_paused())
        .min_by_key(|(entity, _)| entity.index());
    if let (Some((_, player)), Some(graphs)) = (first, graphs) {
        player.for_each_node(&graphs, |visit| {
            state.nodes.push(PreviewNode {
                path: visit.path.to_vec(),
                weight: visit.weight,
                report: visit.report.clone(),
            });
        });
    }
    states.write(state);
}

fn carry_root_motion(
    skeletons: Query<(Entity, &SkeletonComponent)>,
    spaces: Query<&GlobalTransform>,
    root_bones: Query<&AnimationRootBone>,
    roots: Query<&mut Transform, With<ClipPreviewRoot>>,
) {
    let Some(skeleton) = first_rooted(skeletons.iter()) else {
        return;
    };
    let owner = skeletons
        .iter()
        .find(|(_, held)| std::ptr::eq(*held, skeleton))
        .map(|(owner, _)| owner);
    let (Some(bone), Some(space)) = (
        skeleton.root().and_then(|root| root_bones.get_entity(root)),
        owner.and_then(|owner| spaces.get_entity(owner)),
    ) else {
        return;
    };
    let rotation = space.rotation();
    let moved = rotation * (space.scale() * bone.root_motion.translation);
    let turned = rotation * bone.root_motion.rotation * rotation.inverse();
    if !moved.is_finite() || !turned.is_finite() {
        return;
    }
    for mut root in roots.iter() {
        root.translation += moved;
        root.rotation = (turned * root.rotation).normalize();
    }
}

/// What a graph's preview adds to the game's recipe: [`preview_stage`], a player on every rooted skeleton playing what [`ShowGraphs`] names, root motion moving the character, and the types of the tab's document values.
pub fn graph_preview(app: &mut App) {
    preview_stage(app);
    app.register_type::<StateValue>()
        .register_type::<TransitionValue>()
        .register_type::<AnimationFSMTrigger>()
        .register_type::<GraphPreview>()
        .register_type::<GraphParameters>()
        .insert_resource(GraphStage::default())
        .register_event::<GraphPreviewState>()
        .add_system(Update, start_graph_players.at_preview_time())
        .add_system(
            LateUpdate,
            (carry_root_motion, report_graph_state)
                .after(animate_targets)
                .at_preview_time(),
        )
        .add_listener(show_graphs.at_preview_time())
        .add_listener(set_graph_param.at_preview_time())
        .add_listener(set_graph_playback.at_preview_time());
}

#[cfg(test)]
mod tests {
    use super::*;
    use concerto_animation::{
        clip::{AnimationChanelOutput, AnimationChannel, AnimationClip},
        node::{AnimationClipNode, StateMachineReport, state_machine::AnimationStateMachine},
        plugin::AnimationPlugin,
    };
    use concerto_app::plugins::{AssetManagerPlugin, TransformPlugin};
    use concerto_ecs::{
        component::scene::SceneEntityRef,
        events::{event_channel::EventChannel, event_cursor::EventCursor},
    };
    use concerto_editor::{hosted::testing::plain_recipe, preview::PreviewPlugin};
    use concerto_foundation::assets::{
        ContentAssetRoot, content::save_content_asset_into, registry::AssetRegistry,
    };
    use concerto_mesh::skeleton::{JointPose, Skeleton};
    use concerto_name::name;
    use concerto_reflect::to_value;
    use glam::{Mat4, Vec2, Vec3};
    use uuid::Uuid;

    const ROOT: Uuid = Uuid::from_u128(1);
    const DRIVEN: Uuid = Uuid::from_u128(2);
    const IDLE_AT: Vec3 = Vec3::new(1.0, 0.0, 0.0);
    const NEAR_AT: Vec3 = Vec3::new(0.0, 2.0, 0.0);
    const SIDE_AT: Vec3 = Vec3::new(0.0, 3.0, 0.0);
    const UP_AT: Vec3 = Vec3::new(0.0, 4.0, 0.0);
    const FAR_AT: Vec3 = Vec3::new(0.0, 0.0, 5.0);
    const FAR: Vec2 = Vec2::new(5.0, 5.0);
    const FILED_AT: Vec3 = Vec3::new(6.0, 0.0, 0.0);

    fn held_clip(at: Vec3) -> AnimationClip {
        let mut clip = AnimationClip::default();
        clip.add_channel(
            DRIVEN,
            AnimationChannel::new(
                vec![0.0, 1.0],
                AnimationChanelOutput::from_translation([at.to_array(), at.to_array()].into_iter()),
            ),
        );
        clip
    }

    struct Sample {
        clip: AssetId,
        at: Vec2,
    }

    fn blend_graph(samples: &[Sample]) -> AnimationGraph {
        let mut graph = AnimationGraph::new();
        graph
            .result_node()
            .add_blend_space_2d_input_with(name!("direction"), |space| {
                for sample in samples {
                    space.animation_clip_input(AssetHandle::weak(sample.clip), sample.at);
                }
            });
        graph
    }

    fn machine_graph(idle: AssetId, moving: AssetId) -> AnimationGraph {
        let mut machine = AnimationStateMachine::default();
        machine
            .add_state(name!("Idle"), AssetHandle::weak(idle))
            .unwrap();
        machine
            .add_state(name!("Moving"), AssetHandle::weak(moving))
            .unwrap();
        machine
            .add_transition(
                name!("Idle"),
                name!("Moving"),
                AnimationFSMTrigger::BoolEquals {
                    param: name!("moving").into(),
                    value: true,
                },
                0.05,
            )
            .unwrap();
        AnimationGraph::from_node(machine)
    }

    struct Stage {
        app: App,
        driven: Entity,
        _clips: Vec<AssetHandle<AnimationClip>>,
        read: EventCursor<GraphPreviewState>,
        states: Vec<GraphPreviewState>,
    }

    fn path(node: &PreviewNode) -> String {
        let steps: Vec<String> = node
            .path
            .iter()
            .map(|step| match step.state {
                Some(state) => format!("{}:{state}", step.node.index()),
                None => step.node.index().to_string(),
            })
            .collect();
        steps.join("/")
    }

    fn machine(state: &GraphPreviewState) -> Option<StateMachineReport> {
        state
            .nodes
            .iter()
            .find(|node| path(node) == "1")
            .and_then(|node| node.report.state_machine)
    }

    impl Stage {
        fn clip(&mut self, at: Vec3) -> AssetId {
            let id = AssetId::new();
            let server = self.app.get_resource::<AssetServer>().unwrap();
            self._clips.push(server.insert_loaded(id, held_clip(at)));
            id
        }

        fn show(&mut self, graphs: Vec<GraphBytes>, play: AssetId) {
            self.app
                .main_mut()
                .world_mut()
                .trigger(ShowGraphs { graphs, play });
        }

        fn set<T: Reflect>(&mut self, name: Name, value: T) {
            self.app.main_mut().world_mut().trigger(SetGraphParam {
                name,
                type_name: std::any::type_name::<T>().to_owned(),
                value: to_value(&value).unwrap(),
            });
        }

        fn frame(&mut self) {
            std::thread::sleep(std::time::Duration::from_millis(1));
            self.app.update();
            let written = self
                .app
                .get_resource::<EventChannel<GraphPreviewState>>()
                .unwrap()
                .read_from(&mut self.read)
                .cloned()
                .collect::<Vec<_>>();
            self.states.extend(written);
        }

        fn plays(&mut self, at: Vec3, what: &str) {
            let mut shown = Vec3::NAN;
            for _ in 0..200 {
                self.frame();
                shown = self
                    .app
                    .main()
                    .world()
                    .get_component_for_entity::<Transform>(self.driven)
                    .unwrap()
                    .translation;
                if shown.abs_diff_eq(at, 1e-4) {
                    return;
                }
            }
            panic!("{what}: the bone is at {shown}, not {at}");
        }
    }

    fn stage() -> Stage {
        let mut app = App::for_previewing();
        plain_recipe(&mut app);
        app.register_plugin(TransformPlugin)
            .register_plugin(AssetManagerPlugin)
            .register_plugin(AnimationPlugin)
            .register_plugin(PreviewPlugin);
        graph_preview(&mut app);
        app.finish_plugin_build();
        app.update();
        let world = app.main_mut().world_mut();
        let skeleton = AssetId::new();
        world
            .get_resource_mut::<AssetStore<Skeleton>>()
            .unwrap()
            .insert(
                skeleton,
                Skeleton::new(
                    vec![Mat4::IDENTITY; 2],
                    vec![None, Some(0)],
                    vec!["root".into(), "driven".into()],
                    vec![JointPose::IDENTITY; 2],
                    vec![ROOT, DRIVEN],
                )
                .unwrap(),
            );
        let root = world.spawn(Transform::IDENTITY);
        let driven = world.spawn(Transform::IDENTITY);
        world.entity_mut(root).add_child(driven);
        world.spawn(SkeletonComponent::new(
            AssetHandle::weak(skeleton),
            [root, driven].map(SceneEntityRef::Entity).to_vec(),
            Some(SceneEntityRef::Entity(root)),
        ));
        Stage {
            app,
            driven,
            _clips: Vec::new(),
            read: EventCursor::default(),
            states: Vec::new(),
        }
    }

    #[test]
    fn a_preview_plays_the_graphs_it_is_shown_follows_a_parameter_and_restarts_on_a_changed_graph()
    {
        let mut stage = stage();
        let idle_clip = stage.clip(IDLE_AT);
        let mut samples = vec![
            Sample {
                clip: stage.clip(NEAR_AT),
                at: Vec2::ZERO,
            },
            Sample {
                clip: stage.clip(SIDE_AT),
                at: Vec2::X,
            },
            Sample {
                clip: stage.clip(UP_AT),
                at: Vec2::Y,
            },
        ];
        let root = AssetId::new();
        let moving = AssetId::new();
        let directory = tempfile::tempdir().unwrap();
        let filed_clip = stage.clip(FILED_AT);
        let mut registry = AssetRegistry::new();
        let idle = save_content_asset_into(
            &AnimationGraph::from_node(AnimationClipNode::new(AssetHandle::weak(filed_clip))),
            None,
            directory.path(),
            "content/Idle.gasset",
            &mut registry,
        )
        .unwrap();
        stage
            .app
            .get_resource::<AssetServer>()
            .unwrap()
            .publish_content_source(
                ContentAssetRoot::Directory(directory.path().to_owned()),
                registry,
            );
        let fixed = [
            GraphBytes::of(root, &machine_graph(idle, moving)).unwrap(),
            GraphBytes::of(
                idle,
                &AnimationGraph::from_node(AnimationClipNode::new(AssetHandle::weak(idle_clip))),
            )
            .unwrap(),
        ];
        let shown = |samples: &[Sample]| {
            let mut graphs = fixed.to_vec();
            graphs.push(GraphBytes::of(moving, &blend_graph(samples)).unwrap());
            graphs
        };

        stage.show(shown(&samples), root);
        stage.plays(IDLE_AT, "the nested working copy of the initial state");
        let idling = stage.states.last().expect("a playing preview reports");
        assert_eq!(
            machine(idling),
            Some(StateMachineReport {
                current: Some(name!("Idle")),
                transition: None,
            })
        );
        let clip = idling
            .nodes
            .iter()
            .find(|node| path(node) == "1:Idle/1")
            .expect("the state's clip node is reported under its state");
        assert_eq!(clip.weight, 1.0);
        assert!(clip.report.clip_time.is_some_and(|time| time > 0.0));

        let before = stage.states.len();
        stage.set(name!("moving"), true);
        stage.plays(NEAR_AT, "the state the bool parameter leads to");
        stage.frame();
        let since = &stage.states[before..];
        assert!(
            since.iter().any(|state| {
                machine(state)
                    .and_then(|machine| machine.transition)
                    .is_some_and(|transition| {
                        transition.from == name!("Idle")
                            && transition.to == name!("Moving")
                            && transition.progress > 0.0
                            && transition.progress < 1.0
                    })
            }),
            "the transition is reported while it blends"
        );
        assert_eq!(
            since.last().and_then(machine),
            Some(StateMachineReport {
                current: Some(name!("Moving")),
                transition: None,
            })
        );

        stage.app.main_mut().world_mut().trigger(SetGraphPlayback {
            playing: false,
            speed: 1.0,
        });
        stage.frame();
        let paused = stage.states.len();
        stage.frame();
        assert_eq!(
            stage.states.len(),
            paused,
            "a paused preview reports nothing"
        );
        stage.app.main_mut().world_mut().trigger(SetGraphPlayback {
            playing: true,
            speed: 1.0,
        });

        samples.push(Sample {
            clip: stage.clip(FAR_AT),
            at: FAR,
        });
        stage.show(shown(&samples), root);
        stage.set(name!("direction"), FAR);
        stage.plays(FAR_AT, "the blend space's new input");

        stage.set(name!("moving"), false);
        stage.show(vec![fixed[0].clone()], root);
        stage.plays(
            FILED_AT,
            "the file of a graph the preview is no longer shown",
        );
    }
}
