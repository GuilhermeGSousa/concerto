use concerto_editor::mcp::editor;
use concerto_mcp::host::run_with;
use concerto_ragdoll::RagdollShape;
use concerto_render::gpu::Gpu;
use rmcp::ServiceExt;
use serde_json::json;

use crate::create::{
    Client, RIG, SIZE, call, game, ok, saved_ragdoll, text, until_idle, write_project,
};

const RAGDOLL: &str = "content/hero/ragdoll.gasset";

async fn failure(client: &Client, tool: &'static str, arguments: serde_json::Value) -> String {
    let result = call(client, tool, arguments.clone()).await;
    assert_eq!(result.is_error, Some(true), "{tool} {arguments}");
    text(&result)
}

async fn tune(client: Client) {
    until_idle(&client).await;
    ok(&client, "create_ragdoll", json!({ "rig": RIG })).await;

    let before = ok(&client, "ragdoll_parts", json!({ "asset": RAGDOLL })).await;
    assert!(before.contains("17 parts"), "{before}");
    assert_eq!(before.lines().count(), 18, "{before}");
    assert!(
        before
            .lines()
            .any(|line| line.starts_with("pelvis") && line.contains("root")),
        "{before}"
    );
    assert!(
        before
            .lines()
            .any(|line| line.contains("thigh_l") && line.contains("under pelvis")),
        "{before}"
    );

    let closed = failure(
        &client,
        "set_ragdoll_part",
        json!({ "asset": RAGDOLL, "bone": "thigh_l", "radius": 0.3 }),
    )
    .await;
    assert!(closed.contains("open_asset"), "{closed}");

    ok(&client, "open_asset", json!({ "asset": RAGDOLL })).await;
    until_idle(&client).await;
    let fat = ok(
        &client,
        "set_ragdoll_part",
        json!({ "bone": "thigh_l", "shape": "capsule", "radius": 0.3 }),
    )
    .await;
    assert!(
        fat.contains("radius 0.300") && fat.contains("unsaved"),
        "{fat}"
    );
    let crowded = ok(
        &client,
        "test_ragdoll",
        json!({ "action": "drop", "height": 0.5 }),
    )
    .await;
    assert!(
        crowded
            .lines()
            .any(|line| line.contains("thigh_l and thigh_r") && line.contains("m into each other")),
        "{crowded}"
    );
    assert!(
        ok(&client, "ragdoll_parts", json!({}))
            .await
            .contains("the open tab's value"),
    );

    let slim = ok(
        &client,
        "set_ragdoll_part",
        json!({ "bone": "thigh_l", "radius": 0.06 }),
    )
    .await;
    assert!(slim.contains("radius 0.060"), "{slim}");
    let clean = ok(
        &client,
        "test_ragdoll",
        json!({ "action": "drop", "height": 0.5, "seconds": 8.0 }),
    )
    .await;
    assert!(clean.contains("interpenetration: none"), "{clean}");
    assert!(clean.contains("came to rest after"), "{clean}");
    let poked = ok(
        &client,
        "test_ragdoll",
        json!({ "action": "poke", "bone": "Head", "impulse": [0.0, 0.0, 30.0], "seconds": 8.0 }),
    )
    .await;
    assert!(poked.contains("Poke Head"), "{poked}");

    let added = ok(
        &client,
        "set_ragdoll_part",
        json!({ "bone": "index_l", "mass": 0.2 }),
    )
    .await;
    assert!(added.starts_with("Added index_l"), "{added}");
    let removed = ok(
        &client,
        "set_ragdoll_part",
        json!({ "bone": "index_l", "remove": true }),
    )
    .await;
    assert!(removed.contains("17 parts left"), "{removed}");

    for (tool, arguments, says) in [
        (
            "set_ragdoll_part",
            json!({ "bone": "tail", "radius": 0.1 }),
            "no bone `tail`",
        ),
        (
            "set_ragdoll_part",
            json!({ "bone": "thigh_l", "radius": -1.0 }),
            "above 0",
        ),
        (
            "ragdoll_parts",
            json!({ "asset": "content/hero/missing.gasset" }),
            "list_assets",
        ),
        (
            "test_ragdoll",
            json!({ "action": "poke", "bone": "tail" }),
            "no part on `tail`",
        ),
        ("test_ragdoll", json!({ "action": "poke" }), "needs `bone`"),
    ] {
        let answer = failure(&client, tool, arguments.clone()).await;
        assert!(answer.contains(says), "{tool} {arguments}: {answer}");
    }

    ok(&client, "save", json!({})).await;
    client.cancel().await.unwrap();
}

#[test]
fn an_agent_finds_an_interpenetration_in_the_drop_report_fixes_a_radius_and_saves() {
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
                tune(client).await;
                Some(())
            })
    });
    run_with(
        server_io,
        gpu,
        editor(game, root.clone(), vec![concerto_ragdoll_editor::install]),
        SIZE,
    );
    client
        .join()
        .expect("the client's assertions hold")
        .expect("the client connects");

    let saved = saved_ragdoll(&root, RAGDOLL);
    assert_eq!(saved.parts.len(), 17);
    let RagdollShape::Capsule { radius, .. } = saved.part("thigh_l").unwrap().shape else {
        panic!("the thigh is a capsule");
    };
    assert!((radius - 0.06).abs() < 1e-6, "{radius}");
}
