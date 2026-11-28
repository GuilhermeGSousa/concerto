use std::{
    path::Path,
    time::{Duration, Instant},
};

use concerto_app::App;
use concerto_foundation::assets::{
    content::{ContentAssetHeader, decode_content_asset, save_content_asset_into},
    handle::AssetHandle,
    registry::AssetRegistry,
};
use concerto_mcp::host::run_with;
use concerto_mesh::Skeleton;
use concerto_ragdoll::{
    Ragdoll,
    testing::{humanoid_rig, humanoid_skeleton},
};
use concerto_render::gpu::Gpu;
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
};
use serde_json::{Value, json};

pub(crate) const SIZE: (u32, u32) = (800, 600);
pub(crate) const PATIENCE: Duration = Duration::from_secs(120);
pub(crate) const RIG: &str = "content/hero/rig.gasset";
const BROKEN_RIG: &str = "content/hero/broken_rig.gasset";
pub(crate) const SKELETON: &str = "content/hero/skeleton.gasset";

pub(crate) type Client = RunningService<RoleClient, ()>;

pub(crate) fn game(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins);
}

pub(crate) fn write_project(root: &Path) {
    let mut registry = AssetRegistry::new();
    let skeleton =
        save_content_asset_into(&humanoid_skeleton(), root, SKELETON, &mut registry).unwrap();
    let rig = humanoid_rig().with_skeleton(AssetHandle::weak(skeleton));
    save_content_asset_into(&rig, root, RIG, &mut registry).unwrap();
    let mut broken = rig.clone();
    broken.chains.retain(|chain| chain.name != "arm_l");
    save_content_asset_into(&broken, root, BROKEN_RIG, &mut registry).unwrap();
}

pub(crate) async fn call(client: &Client, tool: &'static str, arguments: Value) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(tool)
                .with_arguments(arguments.as_object().cloned().unwrap()),
        )
        .await
        .unwrap()
}

pub(crate) fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect()
}

pub(crate) async fn ok(client: &Client, tool: &'static str, arguments: Value) -> String {
    let result = call(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(false), "{tool}: {result:?}");
    text(&result)
}

pub(crate) async fn until_idle(client: &Client) {
    let started = Instant::now();
    loop {
        let status = ok(client, "status", json!({})).await;
        if !status.contains("project: none") && status.ends_with("busy: no") {
            return;
        }
        assert!(started.elapsed() < PATIENCE, "still busy: {status}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

struct Refusal {
    arguments: Value,
    says: &'static str,
}

struct Generated {
    created: String,
    listed: String,
    heavy: String,
    refusals: Vec<(Refusal, String)>,
}

async fn generate_ragdolls(client: Client) -> Generated {
    until_idle(&client).await;
    let created = ok(&client, "create_ragdoll", json!({ "rig": RIG })).await;
    let listed = ok(&client, "list_assets", json!({ "kind": "Ragdoll" })).await;
    let heavy = ok(
        &client,
        "create_ragdoll",
        json!({ "rig": RIG, "address": "content/hero/heavy.gasset", "total_mass": 100.0 }),
    )
    .await;
    let mut refusals = Vec::new();
    for refusal in [
        Refusal {
            arguments: json!({ "rig": BROKEN_RIG, "address": "content/hero/other.gasset" }),
            says: "no chain called `arm_l`",
        },
        Refusal {
            arguments: json!({ "rig": SKELETON }),
            says: "create_asset kind=RetargetRig",
        },
        Refusal {
            arguments: json!({ "rig": RIG }),
            says: "already exists",
        },
        Refusal {
            arguments: json!({ "rig": "content/hero/missing.gasset" }),
            says: "list_assets",
        },
    ] {
        let result = call(&client, "create_ragdoll", refusal.arguments.clone()).await;
        assert_eq!(result.is_error, Some(true), "{:?}", refusal.arguments);
        refusals.push((refusal, text(&result)));
    }
    client.cancel().await.unwrap();
    Generated {
        created,
        listed,
        heavy,
        refusals,
    }
}

pub(crate) fn saved_ragdoll(root: &Path, address: &str) -> Ragdoll {
    let bytes = std::fs::read(root.join(address)).unwrap();
    ContentAssetHeader::read(&bytes).unwrap();
    decode_content_asset::<Ragdoll>(&bytes).unwrap().1
}

#[test]
fn create_ragdoll_generates_a_humanoid_beside_its_rig_and_refuses_what_it_cannot() {
    let Some(gpu) = Gpu::request_for_test() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_path_buf();
    write_project(&root);
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let client = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let client = ().serve(client_io).await.ok()?;
                Some(generate_ragdolls(client).await)
            })
    });
    run_with(
        server_io,
        gpu,
        concerto_editor::mcp::editor(game, root.clone(), vec![concerto_ragdoll_editor::install]),
        SIZE,
    );
    let generated = client
        .join()
        .expect("the client's assertions hold")
        .expect("the client connects");

    assert!(
        generated
            .created
            .starts_with("Created content/hero/ragdoll.gasset · Ragdoll"),
        "{}",
        generated.created
    );
    assert!(
        generated.created.contains("17 parts"),
        "{}",
        generated.created
    );
    assert_eq!(generated.created.lines().count(), 18);
    assert!(
        generated
            .created
            .lines()
            .any(|line| line.starts_with("pelvis · capsule") && line.ends_with(" kg")),
        "{}",
        generated.created
    );
    assert!(
        generated.listed.contains("content/hero/ragdoll.gasset"),
        "{}",
        generated.listed
    );
    assert!(generated.heavy.contains("100.0 kg"), "{}", generated.heavy);
    for (refusal, answer) in &generated.refusals {
        assert!(
            answer.contains(refusal.says),
            "{}: {answer}",
            refusal.arguments
        );
    }

    let skeleton: Skeleton = decode_content_asset(&std::fs::read(root.join(SKELETON)).unwrap())
        .unwrap()
        .1;
    let ragdoll = saved_ragdoll(&root, "content/hero/ragdoll.gasset");
    assert_eq!(ragdoll.parts.len(), 17);
    assert_eq!(ragdoll.resolve(&skeleton).unwrap().part_count(), 17);
    let mass: f32 = saved_ragdoll(&root, "content/hero/heavy.gasset")
        .parts
        .iter()
        .map(|part| part.mass)
        .sum();
    assert!((mass - 100.0).abs() < 0.01, "{mass}");
    assert!(!root.join("content/hero/other.gasset").exists());
}
