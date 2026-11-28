use std::{
    path::Path,
    time::{Duration, Instant},
};

use concerto_app::App;
use concerto_foundation::assets::content::ContentAssetHeader;
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

type Client = RunningService<RoleClient, ()>;

fn game(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins);
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

async fn ok(client: &Client, tool: &'static str, arguments: Value) -> String {
    let result = call(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(false), "{tool}: {result:?}");
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect()
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

struct Picked {
    database: String,
    set: Value,
}

async fn pick_a_database(client: Client, project: &Path) -> Picked {
    until_idle(&client).await;
    for (kind, name) in [
        ("MotionDatabase", "Moves"),
        ("AnimationGraph", "Locomotion"),
    ] {
        ok(
            &client,
            "create_asset",
            json!({ "kind": kind, "name": name }),
        )
        .await;
    }
    let file = std::fs::read(project.join("content/Moves.gasset")).unwrap();
    let database = ContentAssetHeader::read(&file)
        .unwrap()
        .0
        .asset_id
        .simple_hex();
    ok(
        &client,
        "open_asset",
        json!({ "asset": "content/Locomotion.gasset" }),
    )
    .await;
    ok(
        &client,
        "graph_add_node",
        json!({ "type": "MotionMatchingNode", "parent": 0 }),
    )
    .await;
    ok(&client, "graph_select", json!({ "node": 1 })).await;
    let set = ok(
        &client,
        "set_property",
        json!({ "target": "Node", "path": "database", "value": database }),
    )
    .await;
    client.cancel().await.unwrap();
    Picked {
        database,
        set: serde_json::from_str(&set).unwrap(),
    }
}

#[test]
fn a_motion_matching_nodes_database_is_set_from_the_graph_tab_over_mcp() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    std::fs::create_dir(root.join("content")).unwrap();
    let project = root.clone();
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let client = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let client = ().serve(client_io).await.ok()?;
                Some(pick_a_database(client, &project).await)
            })
    });
    run_with(
        server_io,
        gpu,
        concerto_editor::mcp::editor(
            game,
            root,
            vec![concerto_animation_editor::install, crate::install],
        ),
        SIZE,
    );
    let picked = client
        .join()
        .expect("the client's assertions hold")
        .expect("the client connects");

    assert_eq!(
        picked.set["value"]["database"], picked.database,
        "{}",
        picked.set
    );
}
