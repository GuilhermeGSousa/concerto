use std::{
    future::Future,
    path::Path,
    time::{Duration, Instant},
};

use concerto_animation::{
    graph::{AnimationGraph, AnimationNodeData},
    node::{
        AnimationClipNode, LayerNode,
        state_machine::{AnimationFSMTrigger, AnimationStateMachine},
    },
    retarget::RetargetNode,
};
use concerto_app::App;
use concerto_ecs::{
    IntoSystem, component::scene::SceneEntityRef, events::event_channel::EventChannel,
};
use concerto_editor::{
    asset_editor::{ActiveEditor, AssetEditorRegistry, SaveRequested},
    hosted::{
        HostedApp,
        testing::{SceneFixture, scene_recipe},
    },
    scene::SceneRecipe,
};
use concerto_foundation::{
    assets::{
        Asset, AssetId,
        content::{
            ContentAssetHeader, decode_content_asset, read_content_asset, save_content_asset_into,
            write_content_asset,
        },
        handle::AssetHandle,
        registry::AssetRegistry,
    },
    transform::Transform,
};
use concerto_mcp::host::run_with;
use concerto_mesh::skeleton::{JointPose, Skeleton, SkeletonComponent};
use concerto_name::Name;
use concerto_render::gpu::Gpu;
use concerto_scene::scene::{Scene, SceneNode};
use glam::Mat4;
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    AnimationGraphEditor, EditGraph, GraphDocument, GraphEdit, GraphEdited, GraphNodeId,
    document::{apply_graph_edit, load_graph},
    save::save_graphs,
};

const SIZE: (u32, u32) = (800, 600);
const PATIENCE: Duration = Duration::from_secs(120);
const GRAPH: &str = "content/Locomotion.gasset";
const BROKEN: &str = "content/Broken.gasset";

type Client = RunningService<RoleClient, ()>;

fn game(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins);
}

fn serve<T, F>(root: &Path, scenario: impl FnOnce(Client) -> F + Send + 'static) -> Option<T>
where
    T: Send + 'static,
    F: Future<Output = T>,
{
    let gpu = Gpu::request_for_test()?;
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let client = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let client = ().serve(client_io).await.ok()?;
                Some(scenario(client).await)
            })
    });
    run_with(
        server_io,
        gpu,
        concerto_editor::mcp::editor(game, root.to_path_buf(), vec![crate::install]),
        SIZE,
    );
    Some(
        client
            .join()
            .expect("the client's assertions hold")
            .expect("the client connects"),
    )
}

async fn call(client: &Client, tool: &'static str, arguments: Value) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(tool)
                .with_arguments(arguments.as_object().cloned().unwrap()),
        )
        .await
        .unwrap()
}

fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect()
}

async fn ok(client: &Client, tool: &'static str, arguments: Value) -> String {
    let result = call(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(false), "{tool}: {result:?}");
    text(&result)
}

async fn error(client: &Client, tool: &'static str, arguments: Value) -> Value {
    let result = call(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(true), "{tool}: {result:?}");
    serde_json::from_str(&text(&result)).unwrap()
}

async fn until_idle(client: &Client) {
    let started = Instant::now();
    loop {
        let status = ok(client, "status", json!({})).await;
        if !status.contains("project: none") && status.ends_with("busy: no") {
            return;
        }
        assert!(
            started.elapsed() < PATIENCE,
            "status still answers {status}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn write_broken_graph(root: &Path) {
    let header = ContentAssetHeader {
        asset_id: AssetId::new(),
        references: vec![],
        kind: AnimationGraph::name().into(),
        kind_version: AnimationGraph::FORMAT_VERSION,
        provenance: None,
        derived: None,
        embedded: Vec::new(),
    };
    std::fs::create_dir(root.join("content")).unwrap();
    std::fs::write(
        root.join(BROKEN),
        write_content_asset(&header, &[0xff]).unwrap(),
    )
    .unwrap();
}

fn described(graph: &AnimationGraph) -> Vec<String> {
    graph
        .node_indices()
        .map(|index| {
            let node = graph.node(index).unwrap();
            let inputs: Vec<usize> = graph
                .inputs(index)
                .iter()
                .map(|input| input.index())
                .collect();
            format!(
                "{} {:?} {inputs:?}",
                node.type_name(),
                node.value().unwrap()
            )
        })
        .collect()
}

fn expected_graph() -> AnimationGraph {
    let clip = || AnimationNodeData::of(&AnimationClipNode::default());
    let mut graph = AnimationGraph::new();
    let result = graph.result_index();
    let overlay = graph.insert_node(clip());
    let layer = graph.insert_node(AnimationNodeData::of(&LayerNode {
        weight: 0.5,
        ..Default::default()
    }));
    graph.insert_node(clip());
    let base = graph.insert_node(clip());
    graph.connect(result, layer).unwrap();
    graph.connect(layer, base).unwrap();
    graph.connect(layer, overlay).unwrap();
    graph
}

struct Walk {
    before_open: Value,
    created: String,
    added: Vec<String>,
    connected: Vec<String>,
    after_adding: String,
    refused: Value,
    values: String,
    inspected: String,
    saved: String,
    file: Vec<u8>,
    parented: String,
    childless: Value,
    removed: Vec<String>,
    edited: String,
    open_failed: Value,
    discarded: String,
}

#[test]
fn a_graph_is_created_wired_edited_saved_and_restored_by_a_discard_over_mcp() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    write_broken_graph(&root);
    let project = root.clone();
    let Some(walk) = serve(&root, move |client| async move {
        until_idle(&client).await;
        let before_open = error(&client, "graph_inspect", json!({})).await;
        let created = ok(
            &client,
            "create_asset",
            json!({ "kind": "AnimationGraph", "name": "Locomotion" }),
        )
        .await;
        ok(&client, "open_asset", json!({ "asset": GRAPH })).await;
        let clip = std::any::type_name::<AnimationClipNode>();
        let added = vec![
            ok(
                &client,
                "graph_add_node",
                json!({ "type": "AnimationClipNode" }),
            )
            .await,
            ok(
                &client,
                "graph_add_node",
                json!({ "type": "LayerNode", "parent": 0 }),
            )
            .await,
            ok(
                &client,
                "graph_add_node",
                json!({ "type": clip, "parent": 2 }),
            )
            .await,
            ok(
                &client,
                "graph_add_node",
                json!({ "type": "AnimationClipNode" }),
            )
            .await,
        ];
        let after_adding = ok(&client, "status", json!({})).await;
        let connected = vec![
            ok(&client, "graph_connect", json!({ "child": 1, "parent": 2 })).await,
            ok(
                &client,
                "graph_connect",
                json!({ "child": 4, "parent": 2, "slot": 0 }),
            )
            .await,
        ];
        let refused = error(
            &client,
            "graph_connect",
            json!({ "child": 3, "parent": 2, "slot": 2 }),
        )
        .await;
        ok(&client, "graph_select", json!({ "node": 2 })).await;
        ok(
            &client,
            "set_property",
            json!({ "target": "Node", "path": "weight", "value": 0.5 }),
        )
        .await;
        let values = ok(&client, "status", json!({})).await;
        let inspected = ok(&client, "graph_inspect", json!({})).await;
        let saved = ok(&client, "save", json!({})).await;
        let file = std::fs::read(project.join(GRAPH)).unwrap();
        let parented = ok(
            &client,
            "graph_add_node",
            json!({ "type": "LayerNode", "child": 3 }),
        )
        .await;
        let childless = error(
            &client,
            "graph_add_node",
            json!({ "type": "AnimationClipNode", "child": 3 }),
        )
        .await;
        let removed = vec![
            ok(&client, "graph_remove", json!({ "link": 1 })).await,
            ok(&client, "graph_remove", json!({ "node": 2 })).await,
        ];
        let edited = ok(&client, "graph_inspect", json!({})).await;
        let open_failed = error(
            &client,
            "open_asset",
            json!({ "asset": BROKEN, "unsaved": "discard" }),
        )
        .await;
        let discarded = ok(&client, "graph_inspect", json!({})).await;
        client.cancel().await.unwrap();
        Walk {
            before_open,
            created,
            added,
            connected,
            refused,
            after_adding,
            values,
            inspected,
            saved,
            file,
            parented,
            childless,
            removed,
            edited,
            open_failed,
            discarded,
        }
    }) else {
        return;
    };

    assert_eq!(walk.before_open["code"], "refused", "{}", walk.before_open);
    assert!(
        walk.created
            .starts_with("Created content/Locomotion.gasset · AnimationGraph · "),
        "{}",
        walk.created
    );
    assert_eq!(
        walk.added,
        [
            "Added node 1, an AnimationClipNode, loose.",
            "Added node 2, a LayerNode, as input 0 of node 0.",
            "Added node 3, an AnimationClipNode, as input 0 of node 2.",
            "Added node 4, an AnimationClipNode, loose.",
        ]
    );
    assert_eq!(
        walk.connected,
        [
            "Node 1 is input 1 of node 2.",
            "Node 4 is input 0 of node 2; node 3, which was there, is loose.",
        ]
    );
    assert_eq!(walk.refused["code"], "refused", "{}", walk.refused);
    assert!(
        walk.refused["message"]
            .as_str()
            .unwrap()
            .contains("2 input slots"),
        "{}",
        walk.refused
    );
    assert!(
        walk.after_adding.contains("values: Node"),
        "{}",
        walk.after_adding
    );
    assert!(walk.values.contains("values: Node"), "{}", walk.values);
    assert_eq!(
        walk.inspected,
        "Locomotion\n\
         0 · ResultNode · result\n\
         1 · AnimationClipNode · 2#1\n\
         2 · LayerNode · 0#0\n\
         3 · AnimationClipNode · loose\n\
         4 · AnimationClipNode · 2#0"
    );
    assert_eq!(walk.saved, format!("Saved {GRAPH}."));
    let saved = decode_content_asset::<AnimationGraph>(&walk.file)
        .unwrap()
        .1;
    assert_eq!(described(&saved), described(&expected_graph()));
    assert_eq!(
        walk.parented,
        "Added node 5, a LayerNode, with node 3 as input 0."
    );
    assert_eq!(walk.childless["code"], "refused", "{}", walk.childless);
    assert_eq!(walk.removed, ["Node 1 is loose.", "Removed node 2."]);
    assert_eq!(
        walk.edited,
        "Locomotion\n\
         0 · ResultNode · result\n\
         1 · AnimationClipNode · loose\n\
         3 · AnimationClipNode · 5#0\n\
         4 · AnimationClipNode · loose\n\
         5 · LayerNode · loose"
    );
    assert_eq!(walk.open_failed["code"], "open_failed");
    assert_eq!(walk.discarded, walk.inspected);
}

const IDLE: &str = "content/Idle.gasset";
const WALK: &str = "content/Walk.gasset";

fn id_of(file: &[u8]) -> AssetId {
    read_content_asset(file).unwrap().0.asset_id
}

struct Drill {
    unset: Value,
    held: Value,
    machine_selected: String,
    machine_values: String,
    opened: Vec<String>,
    states: Vec<String>,
    duplicate: Value,
    transitions: Vec<String>,
    selected: Vec<String>,
    taken: Value,
    diagram: String,
    nested_added: String,
    saved: String,
    root: Vec<u8>,
    idle: Vec<u8>,
    walk: Vec<u8>,
    nested_saved: String,
    nested_edited: String,
    state_of_graph: String,
    handed_over: Value,
    restored_root: String,
    restored_nested: String,
}

fn expected_root(root: AssetId, idle: AssetId, walk: AssetId) -> AnimationGraph {
    let idle_name = Name::new("Idle");
    let run = Name::new("Run");
    let mut machine = AnimationStateMachine::default();
    machine
        .add_state(idle_name, AssetHandle::weak(idle))
        .unwrap();
    machine.add_state(run, AssetHandle::weak(walk)).unwrap();
    machine
        .add_transition(idle_name, run, AnimationFSMTrigger::Instant, 0.0)
        .unwrap();
    machine
        .add_transition(run, run, AnimationFSMTrigger::Instant, 0.25)
        .unwrap();
    machine.set_initial(run).unwrap();
    let mut graph = AnimationGraph::new();
    let result = graph.result_index();
    let states = graph.insert_node(AnimationNodeData::of(&machine));
    graph.insert_node(AnimationNodeData::of(&RetargetNode {
        graph: AssetHandle::weak(root),
        ..Default::default()
    }));
    graph.connect(result, states).unwrap();
    graph
}

fn expected_nested() -> AnimationGraph {
    let mut graph = AnimationGraph::new();
    let result = graph.result_index();
    let clip = graph.insert_node(AnimationNodeData::of(&AnimationClipNode::default()));
    graph.connect(result, clip).unwrap();
    graph
}

#[test]
fn a_state_machine_is_drilled_into_edited_saved_with_its_states_graphs_and_restored_over_mcp() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    write_broken_graph(&root);
    let project = root.clone();
    let Some(drill) = serve(&root, move |client| async move {
        until_idle(&client).await;
        ok(
            &client,
            "create_asset",
            json!({ "kind": "AnimationGraph", "name": "Locomotion" }),
        )
        .await;
        ok(&client, "open_asset", json!({ "asset": GRAPH })).await;
        let read = |address: &str| std::fs::read(project.join(address)).unwrap();
        let own = id_of(&read(GRAPH)).simple_hex();
        ok(
            &client,
            "graph_add_node",
            json!({ "type": "AnimationStateMachine", "parent": 0 }),
        )
        .await;
        ok(&client, "graph_add_node", json!({ "type": "RetargetNode" })).await;
        let unset = error(&client, "graph_open", json!({ "node": 2 })).await;
        ok(&client, "graph_select", json!({ "node": 2 })).await;
        ok(
            &client,
            "set_property",
            json!({ "target": "Node", "path": "graph", "value": own }),
        )
        .await;
        let held = error(&client, "graph_open", json!({ "node": 2 })).await;
        let machine_selected = ok(&client, "graph_select", json!({ "node": 1 })).await;
        let machine_values = ok(&client, "status", json!({})).await;
        let mut opened = vec![ok(&client, "graph_open", json!({ "node": 1 })).await];
        let states = vec![
            ok(&client, "graph_add_state", json!({ "name": "Idle" })).await,
            ok(&client, "graph_add_state", json!({ "name": "Walk" })).await,
        ];
        let duplicate = error(&client, "graph_add_state", json!({ "name": "Idle" })).await;
        let transitions = vec![
            ok(
                &client,
                "graph_connect",
                json!({ "from": "Idle", "to": "Walk" }),
            )
            .await,
            ok(
                &client,
                "graph_connect",
                json!({ "from": "Walk", "to": "Walk" }),
            )
            .await,
        ];
        let mut selected = vec![
            ok(
                &client,
                "graph_select",
                json!({ "from": "Walk", "to": "Walk", "index": 0 }),
            )
            .await,
        ];
        ok(
            &client,
            "set_property",
            json!({ "target": "Transition", "path": "blend_time", "value": 0.25 }),
        )
        .await;
        selected.push(ok(&client, "graph_select", json!({ "state": "Walk" })).await);
        let taken = error(
            &client,
            "set_property",
            json!({ "target": "State", "path": "name", "value": "Idle" }),
        )
        .await;
        ok(
            &client,
            "set_property",
            json!({ "target": "State", "path": "name", "value": "Run" }),
        )
        .await;
        ok(
            &client,
            "set_property",
            json!({ "target": "State", "path": "initial", "value": true }),
        )
        .await;
        let diagram = ok(&client, "graph_inspect", json!({})).await;
        opened.push(ok(&client, "graph_open", json!({ "state": "Run" })).await);
        let nested_added = ok(
            &client,
            "graph_add_node",
            json!({ "type": "AnimationClipNode", "parent": 0 }),
        )
        .await;
        opened.push(ok(&client, "graph_up", json!({})).await);
        let saved = ok(&client, "save", json!({})).await;
        let (root, idle, walk) = (read(GRAPH), read(IDLE), read(WALK));
        ok(&client, "graph_open", json!({ "state": "Run" })).await;
        ok(
            &client,
            "graph_add_node",
            json!({ "type": "AnimationClipNode" }),
        )
        .await;
        let nested_saved = ok(&client, "save", json!({})).await;
        ok(&client, "graph_remove", json!({ "node": 1 })).await;
        let nested_edited = ok(&client, "graph_inspect", json!({})).await;
        ok(&client, "graph_up", json!({})).await;
        let state_of_graph = ok(
            &client,
            "graph_add_state",
            json!({ "name": "Rest", "graph": IDLE }),
        )
        .await;
        ok(&client, "graph_select", json!({ "state": "Idle" })).await;
        ok(&client, "graph_remove", json!({ "state": "Run" })).await;
        let handed_over = ok(&client, "inspect", json!({ "target": "State" })).await;
        error(
            &client,
            "open_asset",
            json!({ "asset": BROKEN, "unsaved": "discard" }),
        )
        .await;
        let restored_root = ok(&client, "graph_inspect", json!({})).await;
        ok(&client, "graph_open", json!({ "node": 1 })).await;
        ok(&client, "graph_open", json!({ "state": "Run" })).await;
        let restored_nested = ok(&client, "graph_inspect", json!({})).await;
        client.cancel().await.unwrap();
        Drill {
            unset,
            held,
            machine_selected,
            machine_values,
            opened,
            states,
            duplicate,
            transitions,
            selected,
            taken,
            diagram,
            nested_added,
            saved,
            root,
            idle,
            walk,
            nested_saved,
            nested_edited,
            state_of_graph,
            handed_over: serde_json::from_str(&handed_over).unwrap(),
            restored_root,
            restored_nested,
        }
    }) else {
        return;
    };

    let message = |refusal: &Value| refusal["message"].as_str().unwrap().to_owned();
    assert_eq!(drill.unset["code"], "refused", "{}", drill.unset);
    assert_eq!(
        message(&drill.unset),
        "Node 2 names no graph; select it and set its graph."
    );
    assert_eq!(
        message(&drill.held),
        "Locomotion is already open in this tab, above what is in view; a graph cannot be opened inside itself."
    );
    assert_eq!(
        drill.machine_selected,
        "Selected node 1, an AnimationStateMachine; its states and transitions are edited in its diagram, which graph_open node=1 shows, so it has no Node value."
    );
    assert!(
        drill.machine_values.contains("values: Preview, Parameters"),
        "{}",
        drill.machine_values
    );
    assert_eq!(
        drill.opened,
        [
            "Showing Locomotion › States.",
            "Showing Locomotion › States › Run (state).",
            "Showing Locomotion › States.",
        ]
    );
    assert_eq!(
        drill.states,
        [
            format!("Added state Idle, the initial state, playing {IDLE}."),
            format!("Added state Walk, playing {WALK}."),
        ]
    );
    assert_eq!(
        message(&drill.duplicate),
        "The state machine already has a state named Idle."
    );
    assert_eq!(
        drill.transitions,
        [
            "Added transition Idle → Walk #0.",
            "Added transition Walk → Walk #0.",
        ]
    );
    assert_eq!(
        drill.selected,
        [
            "Selected transition Walk → Walk #0; inspect and set_property take it as target Transition.",
            "Selected state Walk; inspect and set_property take it as target State.",
        ]
    );
    assert_eq!(
        message(&drill.taken),
        "Set State.name failed: the state machine already has a state named Idle"
    );
    assert_eq!(
        drill.diagram,
        format!(
            "Locomotion › States\n\
             Idle · {IDLE}\n\
             Run · initial · {WALK}\n\
             Idle → Run #0 · Instant · 0s\n\
             Run → Run #0 · Instant · 0.25s"
        )
    );
    assert_eq!(
        drill.nested_added,
        "Added node 1, an AnimationClipNode, as input 0 of node 0."
    );
    assert_eq!(drill.saved, format!("Saved {GRAPH}, {WALK}."));
    assert_eq!(drill.nested_saved, format!("Saved {WALK}."));
    let decoded = |file: &[u8]| decode_content_asset::<AnimationGraph>(file).unwrap().1;
    assert_eq!(
        described(&decoded(&drill.root)),
        described(&expected_root(
            id_of(&drill.root),
            id_of(&drill.idle),
            id_of(&drill.walk)
        ))
    );
    assert_eq!(
        described(&decoded(&drill.walk)),
        described(&expected_nested())
    );
    assert_eq!(
        described(&decoded(&drill.idle)),
        described(&AnimationGraph::new())
    );
    assert_eq!(
        drill.nested_edited,
        "Locomotion › States › Run (state)\n\
         0 · ResultNode · result\n\
         2 · AnimationClipNode · loose"
    );
    assert_eq!(
        drill.state_of_graph,
        format!("Added state Rest, playing {IDLE}.")
    );
    assert_eq!(
        drill.handed_over["value"]["initial"], true,
        "{}",
        drill.handed_over
    );
    assert_eq!(
        drill.restored_root,
        "Locomotion\n\
         0 · ResultNode · result\n\
         1 · AnimationStateMachine · 0#0\n\
         2 · RetargetNode · loose · Locomotion"
    );
    assert_eq!(
        drill.restored_nested,
        "Locomotion › States › Run (state)\n\
         0 · ResultNode · result\n\
         1 · AnimationClipNode · 0#0\n\
         2 · AnimationClipNode · loose"
    );
}

fn write_stage(root: &Path) -> AssetId {
    let mut registry = AssetRegistry::new();
    let skeleton = Skeleton::new(
        vec![Mat4::IDENTITY; 2],
        vec![None, Some(0)],
        vec!["root".into(), "driven".into()],
        vec![JointPose::IDENTITY; 2],
        vec![Uuid::from_u128(1), Uuid::from_u128(2)],
    )
    .unwrap();
    let skeleton =
        save_content_asset_into(&skeleton, root, "content/skeleton.gasset", &mut registry).unwrap();
    let node = |name: &str, children: Vec<usize>| {
        let mut node = SceneNode {
            name: name.into(),
            children,
            components: vec![],
        };
        node.push_component(&Transform::IDENTITY).unwrap();
        node
    };
    let mut armature = node("armature", vec![1]);
    armature
        .push_component(&SkeletonComponent::new(
            AssetHandle::weak(skeleton),
            [1, 2].map(SceneEntityRef::Index).to_vec(),
            Some(SceneEntityRef::Index(1)),
        ))
        .unwrap();
    let scene = Scene {
        nodes: vec![armature, node("root", vec![2]), node("driven", vec![])],
        referenced_assets: vec![skeleton],
    };
    save_content_asset_into(&scene, root, "content/scene.gasset", &mut registry).unwrap();
    let bare = Scene {
        nodes: vec![node("prop", vec![])],
        referenced_assets: vec![],
    };
    save_content_asset_into(&bare, root, "content/bare.gasset", &mut registry).unwrap()
}

async fn previewed_from(client: &Client, node: u32) -> String {
    ok(
        client,
        "set_property",
        json!({ "target": "Preview", "path": "from", "value": { "Some": { "0": node } } }),
    )
    .await;
    let mut live = String::new();
    for _ in 0..10 {
        live = ok(
            client,
            "graph_set_param",
            json!({ "name": "direction", "value": [0.0, 0.0] }),
        )
        .await;
        if live.contains(&format!("live:\n{node} ·")) {
            break;
        }
    }
    live
}

struct Previewed {
    optional: Value,
    ambiguous: Value,
    cleared: Value,
    unfit: Value,
    inspected: String,
    set: String,
    after_set: String,
    diagram: String,
    from: Value,
    subtree: Vec<String>,
    back: Value,
    refused_from: String,
    unknown: Value,
    status: String,
    unplayed: String,
    paused: String,
}

#[test]
fn the_preview_the_parameters_and_the_live_state_of_a_graph_are_read_and_set_over_mcp() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    write_broken_graph(&root);
    let bare = write_stage(&root).simple_hex();
    let Some(previewed) = serve(&root, move |client| async move {
        until_idle(&client).await;
        ok(
            &client,
            "create_asset",
            json!({ "kind": "AnimationGraph", "name": "Locomotion" }),
        )
        .await;
        ok(&client, "open_asset", json!({ "asset": GRAPH })).await;
        ok(
            &client,
            "graph_add_node",
            json!({ "type": "AnimationStateMachine", "parent": 0 }),
        )
        .await;
        ok(
            &client,
            "graph_add_node",
            json!({ "type": "BlendSpace2DNode" }),
        )
        .await;
        ok(&client, "graph_select", json!({ "node": 2 })).await;
        ok(
            &client,
            "set_property",
            json!({ "target": "Node", "path": "input.param", "value": "direction" }),
        )
        .await;
        ok(&client, "graph_open", json!({ "node": 1 })).await;
        ok(&client, "graph_add_state", json!({ "name": "Idle" })).await;
        ok(&client, "graph_add_state", json!({ "name": "Walk" })).await;
        ok(
            &client,
            "graph_connect",
            json!({ "from": "Idle", "to": "Walk" }),
        )
        .await;
        ok(
            &client,
            "set_property",
            json!({
                "target": "Transition",
                "path": "trigger",
                "value": { "BoolEquals": { "param": "moving", "value": true } }
            }),
        )
        .await;
        ok(&client, "graph_up", json!({})).await;
        ok(&client, "graph_add_node", json!({ "type": "LayerNode" })).await;
        ok(&client, "graph_select", json!({ "node": 3 })).await;
        let optional = ok(
            &client,
            "set_property",
            json!({
                "target": "Node",
                "path": "weight_param",
                "value": { "Some": { "0": "moving" } }
            }),
        )
        .await;
        let ambiguous = error(
            &client,
            "graph_set_param",
            json!({ "name": "moving", "value": true }),
        )
        .await;
        let cleared = ok(
            &client,
            "set_property",
            json!({ "target": "Node", "path": "weight_param", "value": "None" }),
        )
        .await;
        let unfit = error(
            &client,
            "graph_set_param",
            json!({ "name": "moving", "value": "yes" }),
        )
        .await;
        for _ in 0..2 {
            ok(
                &client,
                "graph_add_node",
                json!({ "type": "AnimationClipNode" }),
            )
            .await;
        }
        ok(&client, "save", json!({})).await;
        let inspected = ok(&client, "graph_inspect", json!({})).await;
        let mut set = String::new();
        for _ in 0..10 {
            set = ok(
                &client,
                "graph_set_param",
                json!({ "name": "moving", "value": true }),
            )
            .await;
            if set.contains("in Walk") {
                break;
            }
        }
        let after_set = ok(&client, "graph_inspect", json!({})).await;
        ok(&client, "graph_open", json!({ "node": 1 })).await;
        let diagram = ok(&client, "graph_inspect", json!({})).await;
        ok(&client, "graph_up", json!({})).await;
        let from = ok(
            &client,
            "set_property",
            json!({ "target": "Preview", "path": "from", "value": { "Some": { "0": 2 } } }),
        )
        .await;
        let subtree = vec![
            previewed_from(&client, 2).await,
            previewed_from(&client, 4).await,
            previewed_from(&client, 5).await,
        ];
        let back = ok(
            &client,
            "set_property",
            json!({ "target": "Preview", "path": "from", "value": "None" }),
        )
        .await;
        ok(
            &client,
            "set_property",
            json!({ "target": "Preview", "path": "from", "value": { "Some": { "0": 9 } } }),
        )
        .await;
        until_idle(&client).await;
        let refused_from = ok(&client, "status", json!({})).await;
        let unknown = error(
            &client,
            "graph_set_param",
            json!({ "name": "speed", "value": 1.0 }),
        )
        .await;
        ok(
            &client,
            "set_property",
            json!({ "target": "Preview", "path": "speed", "value": 0.5 }),
        )
        .await;
        let status = ok(&client, "status", json!({})).await;
        ok(
            &client,
            "set_property",
            json!({ "target": "Preview", "path": "scene", "value": bare }),
        )
        .await;
        let unplayed = ok(
            &client,
            "graph_set_param",
            json!({ "name": "moving", "value": true }),
        )
        .await;
        ok(
            &client,
            "set_property",
            json!({ "target": "Preview", "path": "playing", "value": false }),
        )
        .await;
        let paused = ok(
            &client,
            "graph_set_param",
            json!({ "name": "moving", "value": false }),
        )
        .await;
        client.cancel().await.unwrap();
        Previewed {
            optional: serde_json::from_str(&optional).unwrap(),
            ambiguous,
            cleared: serde_json::from_str(&cleared).unwrap(),
            unfit,
            inspected,
            set,
            after_set,
            diagram,
            from: serde_json::from_str(&from).unwrap(),
            subtree,
            back: serde_json::from_str(&back).unwrap(),
            refused_from,
            unknown,
            status,
            unplayed,
            paused,
        }
    }) else {
        return;
    };

    assert_eq!(
        previewed.optional["value"]["weight_param"],
        json!({ "Some": { "0": "moving" } }),
        "{}",
        previewed.optional
    );
    assert_eq!(
        previewed.ambiguous["message"],
        "moving is read as several types, bool and f32; give type."
    );
    assert_eq!(
        previewed.cleared["value"]["weight_param"], "None",
        "{}",
        previewed.cleared
    );
    assert_eq!(previewed.unfit["code"], "refused", "{}", previewed.unfit);
    assert!(
        previewed.unfit["message"]
            .as_str()
            .unwrap()
            .contains("\"yes\" does not fit"),
        "{}",
        previewed.unfit
    );
    let graph = "Locomotion\n\
         0 · ResultNode · result\n\
         1 · AnimationStateMachine · 0#0\n\
         2 · BlendSpace2DNode · loose\n\
         3 · LayerNode · loose\n\
         4 · AnimationClipNode · loose\n\
         5 · AnimationClipNode · loose\n\
         parameters:\n\
         direction · Vec2 · [0.0,0.0]\n";
    assert!(
        previewed
            .inspected
            .starts_with(&format!("{graph}moving · bool · false")),
        "{}",
        previewed.inspected
    );
    let live = "live:\n\
         0 · weight 1.00\n\
         1 · weight 1.00 · in Walk";
    assert_eq!(previewed.set, format!("Set moving to true.\n{live}"));
    assert_eq!(
        previewed.after_set,
        format!("{graph}moving · bool · true\n{live}")
    );
    assert!(
        previewed.diagram.ends_with("live:\nin Walk"),
        "{}",
        previewed.diagram
    );
    assert_eq!(
        previewed.from["value"]["from"],
        json!({ "Some": { "0": 2 } }),
        "{}",
        previewed.from
    );
    assert_eq!(
        previewed.subtree,
        [
            "Set direction to [0.0,0.0].\nlive:\n2 · weight 1.00",
            "Set direction to [0.0,0.0].\nlive:\n4 · weight 1.00 · time 0.00s",
            "Set direction to [0.0,0.0].\nlive:\n5 · weight 1.00 · time 0.00s",
        ]
    );
    assert_eq!(
        previewed.back["value"]["from"], "None",
        "{}",
        previewed.back
    );
    assert!(
        previewed
            .refused_from
            .contains("There is no node 9 to preview from"),
        "{}",
        previewed.refused_from
    );
    assert_eq!(
        previewed.unknown["code"], "refused",
        "{}",
        previewed.unknown
    );
    assert_eq!(
        previewed.unknown["message"],
        "There is no parameter speed; the parameters are direction (Vec2), moving (bool)."
    );
    assert!(
        previewed.status.contains("values: Preview, Parameters"),
        "{}",
        previewed.status
    );
    assert!(
        !previewed.status.contains("unsaved"),
        "{}",
        previewed.status
    );
    assert_eq!(
        previewed.unplayed,
        "Set moving to true. No preview is playing the graph, so no live state follows; set scene on the Preview value to a scene with a skeleton."
    );
    assert_eq!(
        previewed.paused,
        "Set moving to false. The preview is paused, so no live state follows; set playing on the Preview value to see the graph play."
    );
}

fn animated_recipe(app: &mut App) {
    scene_recipe(app);
    app.register_plugin(concerto_animation::plugin::AnimationPlugin);
}

#[test]
fn nodes_added_and_connected_by_edit_graph_are_saved_into_the_graphs_file() {
    let mut f = SceneFixture::new();
    f.world.insert_resource(SceneRecipe {
        recipe: animated_recipe,
        gpu: None,
        clipboard: None,
    });
    f.world
        .get_resource_mut::<AssetEditorRegistry>()
        .unwrap()
        .register::<AnimationGraph, AnimationGraphEditor>()
        .unwrap();
    f.world.insert_resource(EventChannel::<GraphEdited>::new());
    f.world.add_listener(apply_graph_edit);
    f.add_delivery(load_graph.into_boxed_system());
    f.add_delivery(save_graphs.into_boxed_system());
    let root = f.project_root.clone();
    let registry = f.world.get_resource_mut::<AssetRegistry>().unwrap();
    let id = save_content_asset_into(&AnimationGraph::new(), &root, GRAPH, registry).unwrap();
    let asset = registry.get(id).unwrap().clone();
    f.request(&asset);
    let document = f.world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
    f.frames_until("the graph never opened", |f| {
        f.document(document).pending.is_none()
            && f.world
                .get_component_for_entity::<HostedApp>(document)
                .is_some()
    });
    let result = f
        .world
        .get_component_for_entity::<GraphDocument>(document)
        .unwrap()
        .graphs()[0]
        .copy
        .result();
    let add = |f: &mut SceneFixture, type_name: &str, parent: Option<GraphNodeId>| {
        f.world.trigger(EditGraph {
            document,
            edit: GraphEdit::AddNode {
                type_name: type_name.to_owned(),
                parent,
                slot: None,
            },
        });
        let edited = f.written::<GraphEdited>();
        edited
            .last()
            .and_then(|edited| edited.result.clone().ok())
            .and_then(|change| change.node)
            .expect("the node was added")
    };

    let layer = add(&mut f, "LayerNode", Some(result));
    add(&mut f, "AnimationClipNode", Some(layer));
    let loose = add(&mut f, "AnimationClipNode", None);
    f.world.trigger(EditGraph {
        document,
        edit: GraphEdit::Connect {
            child: loose,
            parent: layer,
            slot: None,
        },
    });
    f.frame();
    assert!(f.unsaved(document));
    f.world.insert(SaveRequested, document);
    f.frame();

    let bytes = std::fs::read(root.join(GRAPH)).unwrap();
    let (header, graph) = decode_content_asset::<AnimationGraph>(&bytes).unwrap();
    assert_eq!(header.asset_id, id);
    let layer = graph.inputs(graph.result_index())[0];
    assert!(
        graph
            .node(layer)
            .unwrap()
            .type_name()
            .ends_with("LayerNode")
    );
    let clips: Vec<String> = graph
        .inputs(layer)
        .iter()
        .map(|input| graph.node(*input).unwrap().type_name().to_owned())
        .collect();
    assert_eq!(clips.len(), 2);
    assert!(clips.iter().all(|clip| clip.ends_with("AnimationClipNode")));
    assert!(!f.unsaved(document));
}
