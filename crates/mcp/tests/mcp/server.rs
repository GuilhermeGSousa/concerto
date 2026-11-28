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
    AppMcpExt, FrameDemand, McpPlugin, McpRead, McpTool, Needs, PendingCall, ServerHandle,
    ToolCalls, ToolError,
};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult, Tool},
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
struct Wait {
    at_deadline: Option<String>,
}

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
    for (arguments, reply) in calls.read() {
        let mut pending = PendingCall::new(reply, "nothing").timeout(Duration::from_millis(50));
        pending.at_deadline = arguments.at_deadline;
        cmd.spawn(pending);
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

struct Served<T> {
    app: App,
    result: T,
}

fn serve<T, F>(
    mut step: impl FnMut(&mut App),
    scenario: impl FnOnce(Client) -> F + Send + 'static,
) -> Served<T>
where
    T: Send + 'static,
    F: Future<Output = T>,
{
    let _turn = ONE_APP_AT_A_TIME
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut app = app();
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let _server = ServerHandle::spawn(&app, server_io);
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
    Served { app, result }
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

struct Listed {
    tools: Vec<Tool>,
    unknown_refused: bool,
    calls: Vec<CallToolResult>,
}

#[test]
fn a_client_lists_each_tools_description_schema_and_hint_and_gets_its_calls_answered_or_refused() {
    let Listed {
        tools,
        unknown_refused,
        calls,
    } = serve(App::update, |client| async move {
        Listed {
            tools: client.list_all_tools().await.unwrap(),
            unknown_refused: client
                .call_tool(CallToolRequestParams::new("missing"))
                .await
                .is_err(),
            calls: vec![
                call(&client, "echo", json!({ "text": "hello" })).await,
                call(&client, "echo", json!({ "words": "hello" })).await,
                call(&client, "needs_scene", json!({})).await,
                call(&client, "silent", json!({})).await,
            ],
        }
    })
    .result;

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

    assert!(unknown_refused);
    assert_eq!(text(&calls[0]), "hello");
    assert_eq!(error(&calls[1])["code"], json!("invalid_arguments"));
    assert_eq!(
        error(&calls[2]),
        json!({ "code": "no_scene", "message": "open a scene first with open_asset" })
    );
    assert_eq!(error(&calls[3])["code"], json!("internal"));
}

#[test]
fn read_only_calls_are_answered_between_frames() {
    let Served { app, result } = serve(
        |app| app.serve(McpRead),
        |client| async move {
            vec![
                call(&client, "frames", json!({})).await,
                call(&client, "echo", json!({ "text": "between" })).await,
            ]
        },
    );

    assert_eq!(text(&result[0]), "0");
    assert_eq!(text(&result[1]), "between");
    assert_eq!(app.get_resource::<FrameCount>().unwrap().0, 0);
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
fn a_pending_call_past_its_deadline_is_answered_timeout_or_with_its_deadline_text() {
    let waited = serve(App::update, |client| async move {
        vec![
            call(&client, "wait", json!({})).await,
            call(&client, "wait", json!({ "at_deadline": "still waiting" })).await,
        ]
    })
    .result;

    assert_eq!(error(&waited[0])["code"], json!("timeout"));
    assert_eq!(text(&waited[1]), "still waiting");
}

struct Opens {
    first: CallToolResult,
    second: CallToolResult,
    seen_while_a_waits: String,
}

#[test]
fn a_change_waits_for_the_one_before_while_reads_go_through() {
    let Served { app, result } = serve(App::update, |client| async move {
        let watch = async {
            let seen = wait_for_progress(&client, "start a").await;
            DOOR.store(true, Ordering::SeqCst);
            wait_for_progress(&client, "start b").await;
            DOOR.store(true, Ordering::SeqCst);
            seen
        };
        let (first, second, seen_while_a_waits) = tokio::join!(
            call(&client, "open", json!({ "name": "a" })),
            call(&client, "open", json!({ "name": "b" })),
            watch
        );
        Opens {
            first,
            second,
            seen_while_a_waits,
        }
    });

    assert_eq!(result.seen_while_a_waits, "start a");
    assert_eq!(text(&result.first), "a is open");
    assert_eq!(text(&result.second), "b is open");
    assert_eq!(
        app.get_resource::<OpenLog>().unwrap().0,
        ["start a", "done a", "start b", "done b"]
    );
}

struct Frame {
    logged: usize,
    demanded: bool,
}

#[test]
fn frames_are_demanded_while_a_call_is_pending() {
    let mut frames = Vec::new();
    let opened = serve(
        |app| {
            app.update();
            let logged = app.get_resource::<OpenLog>().unwrap().0.len();
            frames.push(Frame {
                logged,
                demanded: app.get_resource::<FrameDemand>().unwrap().is_requested(),
            });
            if logged == 1 {
                DOOR.store(true, Ordering::SeqCst);
            }
        },
        |client| async move { call(&client, "open", json!({ "name": "a" })).await },
    )
    .result;

    assert_eq!(text(&opened), "a is open");
    let demanded = |entries: usize| -> Vec<bool> {
        frames
            .iter()
            .filter(|frame| frame.logged == entries)
            .map(|frame| frame.demanded)
            .collect()
    };
    assert!(demanded(0).iter().all(|demanded| !demanded));
    assert!(!demanded(1).is_empty() && demanded(1).iter().all(|demanded| *demanded));
    assert!(!demanded(2).is_empty() && demanded(2).iter().all(|demanded| !demanded));
}
