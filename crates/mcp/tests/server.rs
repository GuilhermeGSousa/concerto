use std::{
    future::Future,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use concerto_app::{
    App, main_schedule::MainSchedulePlugin, plugins::TimePlugin, schedule_groups::Update,
};
use concerto_ecs::{
    CommandQueue, Component, Entity, Event, IntoSystemConfig, Query, Res, ResMut, Resource,
    events::{event_reader::EventReader, event_writer::EventWriter},
};
use concerto_mcp::{
    AppMcpExt, FrameDemand, McpPlugin, McpRead, McpTool, Needs, PendingCall, ToolCalls, ToolError,
    spawn_server,
};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Says the text back.
#[derive(Deserialize, JsonSchema)]
struct Echo {
    /// What to say back.
    text: String,
}

#[derive(Deserialize, JsonSchema)]
struct Twice {}

#[derive(Deserialize, JsonSchema)]
struct NeedsScene {}

#[derive(Deserialize, JsonSchema)]
struct Silent {}

#[derive(Deserialize, JsonSchema)]
struct Frames {}

#[derive(Resource)]
struct OpenScene;

#[derive(Resource, Default)]
struct FrameCount(u32);

fn count_frames(mut frames: ResMut<FrameCount>) {
    frames.0 += 1;
}

fn echo(mut calls: ToolCalls<Echo>) {
    for (arguments, reply) in calls.read() {
        reply.ok(arguments.text);
    }
}

fn twice(mut calls: ToolCalls<Twice>) {
    for (_, reply) in calls.read() {
        reply.ok("first");
        reply.ok("second");
    }
}

fn needs_scene(mut calls: ToolCalls<NeedsScene>, _: Res<OpenScene>) {
    for (_, reply) in calls.read() {
        reply.ok("a scene");
    }
}

fn silent(mut calls: ToolCalls<Silent>) {
    for _ in calls.read() {}
}

fn frames(mut calls: ToolCalls<Frames>, frames: Res<FrameCount>) {
    for (_, reply) in calls.read() {
        reply.ok(frames.0.to_string());
    }
}

/// Opens the named door and answers once it has swung open.
#[derive(Deserialize, JsonSchema)]
struct Open {
    name: String,
}

#[derive(Deserialize, JsonSchema)]
struct Progress {}

#[derive(Deserialize, JsonSchema)]
struct Wait {}

#[derive(Component)]
struct Opening(String);

#[derive(Event)]
struct Opened;

#[derive(Resource, Default)]
struct OpenLog(Vec<String>);

static DOOR: AtomicBool = AtomicBool::new(false);

fn open(mut calls: ToolCalls<Open>, mut log: ResMut<OpenLog>, mut cmd: CommandQueue) {
    for (arguments, reply) in calls.read() {
        log.0.push(format!("start {}", arguments.name));
        cmd.spawn((
            PendingCall::new(reply, "the door to open"),
            Opening(arguments.name),
        ));
    }
}

fn swing_door(mut opened: EventWriter<Opened>) {
    if DOOR.swap(false, Ordering::SeqCst) {
        opened.write(Opened);
    }
}

fn answer_opened(
    mut opened: EventReader<Opened>,
    waiting: Query<(Entity, &PendingCall, &Opening)>,
    mut log: ResMut<OpenLog>,
    mut cmd: CommandQueue,
) {
    for _ in opened.read() {
        for (entity, pending, opening) in waiting.iter() {
            pending.reply.ok(format!("{} is open", opening.0));
            log.0.push(format!("done {}", opening.0));
            cmd.despawn(entity);
        }
    }
}

fn progress(mut calls: ToolCalls<Progress>, log: Res<OpenLog>) {
    for (_, reply) in calls.read() {
        reply.ok(log.0.join(", "));
    }
}

fn wait(mut calls: ToolCalls<Wait>, mut cmd: CommandQueue) {
    for (_, reply) in calls.read() {
        cmd.spawn(PendingCall::new(reply, "nothing").timeout(Duration::from_millis(50)));
    }
}

fn app() -> App {
    DOOR.store(false, Ordering::SeqCst);
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(McpPlugin)
        .insert_resource(FrameCount::default())
        .insert_resource(OpenLog::default())
        .register_event::<Opened>()
        .add_system(Update, count_frames)
        .add_system(Update, (swing_door, answer_opened.after(swing_door)))
        .describe_need(
            Needs::Scene,
            ToolError::new("no_scene", "open a scene first with open_asset"),
        )
        .add_tool::<Echo, _>(McpTool::read_only("echo"), echo)
        .add_tool::<Twice, _>(McpTool::new("twice"), twice)
        .add_tool::<NeedsScene, _>(
            McpTool::read_only("needs_scene").needs(Needs::Scene),
            needs_scene,
        )
        .add_tool::<Silent, _>(McpTool::new("silent"), silent)
        .add_tool::<Frames, _>(McpTool::read_only("frames"), frames)
        .add_tool::<Open, _>(McpTool::new("open"), open)
        .add_tool::<Progress, _>(McpTool::read_only("progress"), progress)
        .add_tool::<Wait, _>(McpTool::new("wait"), wait);
    app.finish_plugin_build();
    app
}

static ONE_APP_AT_A_TIME: Mutex<()> = Mutex::new(());

type Client = RunningService<RoleClient, ()>;

fn serve<T, F>(
    mut step: impl FnMut(&mut App),
    scenario: impl FnOnce(Client) -> F + Send + 'static,
) -> (App, T)
where
    T: Send + 'static,
    F: Future<Output = T>,
{
    let _turn = ONE_APP_AT_A_TIME
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut app = app();
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let _server = spawn_server(&app, server_io);
    let client = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let client = ().serve(client_io).await.expect("the client connects");
                scenario(client).await
            })
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    while !client.is_finished() {
        assert!(Instant::now() < deadline, "the client did not finish");
        step(&mut app);
        std::thread::sleep(Duration::from_millis(1));
    }
    let result = client.join().expect("the client's assertions hold");
    (app, result)
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
    assert_eq!(result.is_error, Some(false), "{result:?}");
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect()
}

fn error(result: &CallToolResult) -> Value {
    assert_eq!(result.is_error, Some(true), "{result:?}");
    let body: String = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect();
    serde_json::from_str(&body).unwrap()
}

#[test]
fn the_tool_list_has_each_tools_description_schema_and_read_only_hint() {
    let (_, tools) = serve(App::update, |client| async move {
        client.list_all_tools().await.unwrap()
    });

    let names: Vec<_> = tools.iter().map(|tool| tool.name.to_string()).collect();
    assert_eq!(
        names,
        [
            "echo",
            "twice",
            "needs_scene",
            "silent",
            "frames",
            "open",
            "progress",
            "wait"
        ]
    );
    let echo = &tools[0];
    assert_eq!(echo.description.as_deref(), Some("Says the text back."));
    assert_eq!(
        echo.input_schema["properties"]["text"]["description"],
        json!("What to say back.")
    );
    assert_eq!(echo.input_schema["required"], json!(["text"]));
    let read_only = |index: usize| {
        tools[index]
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.read_only_hint)
    };
    assert_eq!(read_only(0), Some(true));
    assert_eq!(read_only(1), Some(false));
}

#[test]
fn a_call_is_answered_by_its_tools_system() {
    let (_, result) = serve(App::update, |client| async move {
        call(&client, "echo", json!({ "text": "hello" })).await
    });

    assert_eq!(text(&result), "hello");
}

#[test]
fn malformed_arguments_are_answered_invalid_arguments() {
    let (_, result) = serve(App::update, |client| async move {
        call(&client, "echo", json!({ "words": "hello" })).await
    });

    let body = error(&result);
    assert_eq!(body["code"], json!("invalid_arguments"));
    assert!(body["message"].as_str().unwrap().contains("text"), "{body}");
}

#[test]
fn a_call_no_system_answered_gets_its_unmet_need_or_internal() {
    let (_, (needs_scene, silent)) = serve(App::update, |client| async move {
        (
            call(&client, "needs_scene", json!({})).await,
            call(&client, "silent", json!({})).await,
        )
    });

    assert_eq!(
        error(&needs_scene),
        json!({ "code": "no_scene", "message": "open a scene first with open_asset" })
    );
    assert_eq!(error(&silent)["code"], json!("internal"));
}

#[test]
fn a_second_reply_is_ignored() {
    let (_, (first, after)) = serve(App::update, |client| async move {
        (
            call(&client, "twice", json!({})).await,
            call(&client, "echo", json!({ "text": "after" })).await,
        )
    });

    assert_eq!(text(&first), "first");
    assert_eq!(text(&after), "after");
}

#[test]
fn an_unknown_tool_is_a_protocol_error() {
    let (_, result) = serve(App::update, |client| async move {
        client
            .call_tool(CallToolRequestParams::new("missing"))
            .await
            .is_err()
    });

    assert!(result);
}

#[test]
fn read_only_calls_are_answered_between_frames() {
    let (app, (frames, echoed)) = serve(
        |app| app.serve(McpRead),
        |client| async move {
            (
                call(&client, "frames", json!({})).await,
                call(&client, "echo", json!({ "text": "between" })).await,
            )
        },
    );

    assert_eq!(text(&frames), "0");
    assert_eq!(text(&echoed), "between");
    assert_eq!(app.get_resource::<FrameCount>().unwrap().0, 0);
}

fn writes(mut calls: ToolCalls<Frames>, mut frames: ResMut<FrameCount>) {
    for (_, reply) in calls.read() {
        frames.0 += 1;
        reply.ok("written");
    }
}

#[test]
#[should_panic(expected = "read-only tool `frames`")]
fn a_read_only_tool_whose_system_writes_is_refused() {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(McpPlugin)
        .add_tool::<Frames, _>(McpTool::read_only("frames"), writes);
}

async fn wait_for_progress(client: &Client, entry: &str) -> String {
    loop {
        let seen = text(&call(client, "progress", json!({})).await);
        if seen.contains(entry) {
            return seen;
        }
    }
}

#[test]
fn a_pending_call_is_answered_by_a_later_event() {
    let (app, (opened, seen)) = serve(App::update, |client| async move {
        let watch = async {
            let seen = wait_for_progress(&client, "start a").await;
            DOOR.store(true, Ordering::SeqCst);
            seen
        };
        tokio::join!(call(&client, "open", json!({ "name": "a" })), watch)
    });

    assert_eq!(seen, "start a");
    assert_eq!(text(&opened), "a is open");
    assert_eq!(
        app.get_resource::<OpenLog>().unwrap().0,
        ["start a", "done a"]
    );
}

#[test]
fn a_pending_call_past_its_deadline_is_answered_timeout() {
    let (_, (waited, after)) = serve(App::update, |client| async move {
        (
            call(&client, "wait", json!({})).await,
            call(&client, "twice", json!({})).await,
        )
    });

    assert_eq!(
        error(&waited),
        json!({ "code": "timeout", "message": "Timed out waiting for nothing." })
    );
    assert_eq!(text(&after), "first");
}

#[test]
fn a_change_waits_for_the_one_before_while_reads_go_through() {
    let (app, (first, second, seen)) = serve(App::update, |client| async move {
        let watch = async {
            let seen = wait_for_progress(&client, "start a").await;
            DOOR.store(true, Ordering::SeqCst);
            wait_for_progress(&client, "start b").await;
            DOOR.store(true, Ordering::SeqCst);
            seen
        };
        tokio::join!(
            call(&client, "open", json!({ "name": "a" })),
            call(&client, "open", json!({ "name": "b" })),
            watch
        )
    });

    assert_eq!(seen, "start a");
    assert_eq!(text(&first), "a is open");
    assert_eq!(text(&second), "b is open");
    assert_eq!(
        app.get_resource::<OpenLog>().unwrap().0,
        ["start a", "done a", "start b", "done b"]
    );
}

#[test]
fn frames_are_demanded_while_a_call_is_pending() {
    let mut frames = Vec::new();
    let (_, opened) = serve(
        |app| {
            app.update();
            let logged = app.get_resource::<OpenLog>().unwrap().0.len();
            frames.push((
                logged,
                app.get_resource::<FrameDemand>().unwrap().is_requested(),
            ));
            if logged == 1 {
                DOOR.store(true, Ordering::SeqCst);
            }
        },
        |client| async move { call(&client, "open", json!({ "name": "a" })).await },
    );

    assert_eq!(text(&opened), "a is open");
    let demanded = |entries: usize| -> Vec<bool> {
        frames
            .iter()
            .filter(|(logged, _)| *logged == entries)
            .map(|(_, demanded)| *demanded)
            .collect()
    };
    assert!(demanded(0).iter().all(|demanded| !demanded), "{frames:?}");
    assert!(
        !demanded(1).is_empty() && demanded(1).iter().all(|demanded| *demanded),
        "{frames:?}"
    );
    assert!(
        !demanded(2).is_empty() && demanded(2).iter().all(|demanded| !demanded),
        "{frames:?}"
    );
}
