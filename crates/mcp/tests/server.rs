//! A real rmcp client talking to the server over an in-memory pipe, with the
//! app driven by `run_until_closed` exactly as the headless runner drives it.
use std::time::Duration;

use concerto_app::{
    App, main_schedule::MainSchedulePlugin, plugins::TimePlugin, schedule_groups::Update,
};
use concerto_ecs::{ResMut, Resource, World};
use concerto_mcp::{
    Handled, McpApp, McpInbox, McpPlugin, McpTool, McpTools, NoArguments, Pending, ServerIdentity,
    ToolError, ToolOutput, call_tool, run_until_closed, spawn_server,
};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
struct Echo {
    /// What to say back.
    text: String,
}

#[derive(Deserialize, JsonSchema)]
struct Frames {
    frames: u32,
}

/// Frames run so far, so a test can tell which calls cost one.
#[derive(Resource, Default)]
struct FrameCount(u32);

fn count_frames(mut frames: ResMut<FrameCount>) {
    frames.0 += 1;
}

/// Apps in one process share the global compute pool, and as many apps
/// updating at once as the pool has threads deadlock it. Run one at a time.
static ONE_APP_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn app() -> App {
    let mut app = App::new();
    app.insert_resource(FrameCount::default())
        .add_system(Update, count_frames);
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(McpPlugin)
        .add_mcp_tool(
            McpTool::new("echo", "Says it back.", |_: &mut World, args: Echo| {
                ToolOutput::text(args.text).into()
            })
            .read_only(),
        )
        .add_mcp_tool(McpTool::new(
            "wait_frames",
            "Answers after some frames.",
            |_: &mut World, args: Frames| {
                let mut seen = 0;
                Pending::new(Duration::from_secs(10), move |_| {
                    seen += 1;
                    (seen >= args.frames).then(|| Ok(ToolOutput::text(format!("{seen} frames"))))
                })
                .into()
            },
        ))
        .add_mcp_tool(McpTool::new(
            "never",
            "Never answers.",
            |_: &mut World, _: NoArguments| {
                Pending::new(Duration::from_millis(50), |_| None)
                    .on_timeout("gave up")
                    .into()
            },
        ))
        .add_mcp_tool(McpTool::new(
            "boom",
            "Panics.",
            |_: &mut World, _: NoArguments| -> Handled { panic!("kaboom") },
        ))
        .add_mcp_tool(
            McpTool::new(
                "frames",
                "Frames so far.",
                |world: &mut World, _: NoArguments| {
                    let frames = world.get_resource::<FrameCount>().unwrap().0;
                    ToolOutput::text(frames.to_string()).into()
                },
            )
            .read_only(),
        )
        .add_mcp_tool(McpTool::new(
            "refuse",
            "Fails politely.",
            |_: &mut World, _: NoArguments| ToolError::new("nope", "Not today.").into(),
        ));
    app.finish_plugin_build();
    app
}

fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect()
}

fn arguments(value: Value) -> serde_json::Map<String, Value> {
    value.as_object().cloned().unwrap()
}

#[test]
fn a_client_lists_and_calls_tools_until_it_disconnects() {
    let _turn = ONE_APP_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut app = app();
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    {
        let world = app.main_mut().world_mut();
        spawn_server(
            ServerIdentity {
                name: "test".into(),
                version: "0".into(),
                instructions: "Testing.".into(),
            },
            world.get_resource::<McpTools>().unwrap(),
            world.get_resource::<McpInbox>().unwrap(),
            move || server_io,
        );
    }

    let client = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let client = ().serve(client_io).await.expect("initialise");
            let info = client.peer_info().unwrap();
            assert_eq!(info.server_info.as_ref().unwrap().name, "test");
            assert_eq!(info.instructions.as_deref(), Some("Testing."));

            let tools = client.list_all_tools().await.unwrap();
            let names: Vec<_> = tools.iter().map(|tool| tool.name.to_string()).collect();
            assert_eq!(
                names,
                ["echo", "wait_frames", "never", "boom", "frames", "refuse"]
            );
            let echo = &tools[0];
            assert!(
                echo.input_schema["properties"]["text"]["description"]
                    .as_str()
                    .is_some_and(|text| text.contains("say back")),
                "doc comments describe parameters: {:?}",
                echo.input_schema
            );
            assert_eq!(
                echo.annotations.as_ref().and_then(|a| a.read_only_hint),
                Some(true)
            );

            let call = |name: &'static str, args: Value| {
                let client = &client;
                async move {
                    client
                        .call_tool(CallToolRequestParams::new(name).with_arguments(arguments(args)))
                        .await
                        .unwrap()
                }
            };

            let result = call("echo", json!({"text": "hello"})).await;
            assert_eq!(
                (text(&result), result.is_error),
                ("hello".into(), Some(false))
            );

            let result = call("wait_frames", json!({"frames": 3})).await;
            assert_eq!(text(&result), "3 frames");

            let result = call("never", json!({})).await;
            assert_eq!(text(&result), "timeout: gave up");
            assert_eq!(result.is_error, Some(true));

            let result = call("refuse", json!({})).await;
            assert_eq!(text(&result), "nope: Not today.");

            let result = call("boom", json!({})).await;
            assert!(text(&result).starts_with("internal: `boom` panicked: kaboom"));

            let result = call("echo", json!({"words": "hello"})).await;
            assert!(
                text(&result).starts_with("invalid_arguments:"),
                "{}",
                text(&result)
            );

            let result = call("missing", json!({})).await;
            assert_eq!(text(&result), "unknown_tool: No tool named `missing`.");

            // Looking costs no frame; a call that may change something costs one.
            let frames = |text: String| text.parse::<u32>().unwrap();
            let before = frames(text(&call("frames", json!({})).await));
            call("echo", json!({"text": "look"})).await;
            assert_eq!(frames(text(&call("frames", json!({})).await)), before);
            call("refuse", json!({})).await;
            assert_eq!(frames(text(&call("frames", json!({})).await)), before + 1);

            // A panicking tool must not take the app down.
            let result = call("echo", json!({"text": "still here"})).await;
            assert_eq!(text(&result), "still here");

            client.cancel().await.unwrap();
        });
    });

    // Returns once the client disconnects; hangs (and the test times out) if
    // the runner misses the close.
    run_until_closed(&mut app);
    client.join().expect("client assertions");
}

#[test]
fn call_tool_runs_frames_until_the_tool_answers() {
    let _turn = ONE_APP_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut app = app();
    assert_eq!(
        call_tool(&mut app, "wait_frames", json!({"frames": 2}), 10),
        Ok(ToolOutput::text("2 frames"))
    );
    assert_eq!(
        call_tool(&mut app, "wait_frames", json!({"frames": 20}), 5)
            .unwrap_err()
            .code,
        "timeout"
    );
}
