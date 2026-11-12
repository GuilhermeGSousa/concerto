use std::{
    future::Future,
    path::Path,
    time::{Duration, Instant},
};

use concerto_animation::{
    graph::{AnimationGraph, AnimationNodeData},
    node::{AnimationClipNode, LayerNode},
};
use concerto_app::App;
use concerto_foundation::assets::{
    Asset, AssetId,
    content::{ContentAssetHeader, decode_content_asset, write_content_asset},
};
use concerto_mcp::host::run_with;
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
