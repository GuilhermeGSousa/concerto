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
use concerto_foundation::assets::{
    Asset, AssetId,
    content::{ContentAssetHeader, decode_content_asset, read_content_asset, write_content_asset},
    handle::AssetHandle,
};
use concerto_mcp::host::run_with;
use concerto_name::Name;
use concerto_render::gpu::Gpu;
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
};
use serde_json::{Value, json};

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
    refused: Value,
    values: String,
    inspected: String,
    saved: String,
    file: Vec<u8>,
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
            values,
            inspected,
            saved,
            file,
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
    assert_eq!(walk.removed, ["Node 1 is loose.", "Removed node 2."]);
    assert_eq!(
        walk.edited,
        "Locomotion\n\
         0 · ResultNode · result\n\
         1 · AnimationClipNode · loose\n\
         3 · AnimationClipNode · loose\n\
         4 · AnimationClipNode · loose"
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
    opened: Vec<String>,
    states: Vec<String>,
    duplicate: Value,
    transitions: Vec<String>,
    selected: Vec<String>,
    diagram: String,
    nested_added: String,
    saved: String,
    root: Vec<u8>,
    idle: Vec<u8>,
    walk: Vec<u8>,
    nested_edited: String,
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
        ok(&client, "graph_remove", json!({ "node": 1 })).await;
        let nested_edited = ok(&client, "graph_inspect", json!({})).await;
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
            opened,
            states,
            duplicate,
            transitions,
            selected,
            diagram,
            nested_added,
            saved,
            root,
            idle,
            walk,
            nested_edited,
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
    assert_eq!(drill.saved, format!("Saved {GRAPH}."));
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
        "Locomotion › States › Run (state)\n0 · ResultNode · result"
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
         1 · AnimationClipNode · 0#0"
    );
}
