use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::TimePlugin,
    schedule_groups::{First, LateUpdate, Update},
};
use concerto_ecs::{
    CommandQueue, Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, signal::On,
};
use concerto_mcp::{
    AppMcpExt, McpPlugin, McpRead, McpTool, PendingCall, ToolCalls,
    host::{HostClosing, HostPicture, run_with},
    receive_calls,
};
use concerto_window::plugin::{Window, WindowCommand};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize, JsonSchema)]
struct Updates {}

#[derive(Deserialize, JsonSchema)]
struct Change {}

#[derive(Deserialize, JsonSchema)]
struct Quit {}

#[derive(Deserialize, JsonSchema)]
struct Open {}

#[derive(Component)]
struct Opening(u32);

#[derive(Resource, Clone, Default)]
struct UpdateCount(Arc<AtomicU32>);

impl UpdateCount {
    fn get(&self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }
}

#[derive(Resource, Clone, Default)]
struct Observed(Arc<Mutex<Vec<String>>>);

impl Observed {
    fn push(&self, entry: impl Into<String>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry.into());
    }

    fn entries(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn count_updates(count: Res<UpdateCount>) {
    count.0.fetch_add(1, Ordering::SeqCst);
}

fn updates(mut calls: ToolCalls<Updates>, count: Res<UpdateCount>) {
    for (_, reply) in calls.read() {
        reply.ok(count.get().to_string());
    }
}

fn change(mut calls: ToolCalls<Change>, count: Res<UpdateCount>) {
    for (_, reply) in calls.read() {
        reply.ok(count.get().to_string());
    }
}

fn open(mut calls: ToolCalls<Open>, count: Res<UpdateCount>, mut cmd: CommandQueue) {
    for (_, reply) in calls.read() {
        cmd.spawn((
            PendingCall::new(reply, "the frame after next"),
            Opening(count.get()),
        ));
    }
}

fn answer_open(
    waiting: Query<(Entity, &PendingCall, &Opening)>,
    count: Res<UpdateCount>,
    mut cmd: CommandQueue,
) {
    for (entity, pending, opening) in waiting.iter() {
        if count.get() > opening.0 + 1 {
            pending.reply.ok(count.get().to_string());
            cmd.despawn(entity);
        }
    }
}

fn quit(mut calls: ToolCalls<Quit>, mut window: ResMut<Window>) {
    for (_, reply) in calls.read() {
        window.send(WindowCommand::Close);
        reply.ok("closing");
    }
}

fn close_on_host_closing(_: On<HostClosing>, mut window: ResMut<Window>, observed: Res<Observed>) {
    observed.push("host closing");
    window.send(WindowCommand::Close);
}

fn note_picture(picture: Option<Res<HostPicture>>, window: Res<Window>, observed: Res<Observed>) {
    if picture.is_some() {
        observed.push(format!("picture {:?}", window.size()));
    }
}

fn slow_first_frame(count: Res<UpdateCount>) {
    if count.get() == 0 {
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn recipe(count: UpdateCount, observed: Observed, closes: bool) -> impl FnOnce(&mut App) {
    move |app| {
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(McpPlugin)
            .insert_resource(count)
            .insert_resource(observed)
            .add_system(First, slow_first_frame.before(receive_calls))
            .add_system(Update, count_updates)
            .add_tool::<Updates, _>(McpTool::read_only("updates"), updates)
            .add_tool::<Change, _>(McpTool::new("change"), change)
            .add_tool::<Quit, _>(McpTool::new("quit"), quit)
            .add_tool::<Open, _>(McpTool::new("open"), open)
            .add_system(LateUpdate, answer_open)
            .add_system(McpRead, note_picture);
        if closes {
            app.add_listener(close_on_host_closing);
        }
    }
}

static ONE_APP_AT_A_TIME: Mutex<()> = Mutex::new(());

type Client = RunningService<RoleClient, ()>;

struct Hosted<T> {
    result: T,
    took: Duration,
}

fn host<T, F>(
    closes: bool,
    count: &UpdateCount,
    observed: &Observed,
    scenario: impl FnOnce(Client) -> F + Send + 'static,
) -> Option<Hosted<T>>
where
    T: Send + 'static,
    F: Future<Output = T>,
{
    let _turn = ONE_APP_AT_A_TIME
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
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
    let started = Instant::now();
    if run_with(
        server_io,
        recipe(count.clone(), observed.clone(), closes),
        (320, 180),
    )
    .is_err()
    {
        eprintln!("skipped: no GPU adapter");
        return None;
    }
    let took = started.elapsed();
    let result = client
        .join()
        .expect("the client's assertions hold")
        .expect("the client connects");
    Some(Hosted { result, took })
}

async fn call(client: &Client, tool: &'static str) -> String {
    let result: CallToolResult = client
        .call_tool(
            CallToolRequestParams::new(tool)
                .with_arguments(json!({}).as_object().cloned().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false), "{result:?}");
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect()
}

#[test]
fn reads_are_served_between_frames_and_a_change_costs_one_frame() {
    let count = UpdateCount::default();
    let observed = Observed::default();
    let Some(hosted) = host(true, &count, &observed, |client| async move {
        let seen = vec![
            call(&client, "updates").await,
            call(&client, "updates").await,
            call(&client, "change").await,
            call(&client, "updates").await,
        ];
        client.cancel().await.unwrap();
        seen
    }) else {
        return;
    };

    assert_eq!(hosted.result, ["1", "1", "1", "2"]);
    assert_eq!(count.get(), 2);
    assert_eq!(
        observed.entries(),
        [
            "picture (320, 180)",
            "picture (320, 180)",
            "picture (320, 180)",
            "host closing"
        ]
    );
}

#[test]
fn a_read_after_a_change_answered_on_a_later_frame_costs_no_frame() {
    let count = UpdateCount::default();
    let observed = Observed::default();
    let Some(hosted) = host(true, &count, &observed, |client| async move {
        let seen = vec![call(&client, "open").await, call(&client, "updates").await];
        client.cancel().await.unwrap();
        seen
    }) else {
        return;
    };

    assert_eq!(hosted.result, ["3", "3"]);
    assert_eq!(count.get(), 3);
}

#[test]
fn closing_the_client_ends_the_loop_once_the_app_closes() {
    let count = UpdateCount::default();
    let observed = Observed::default();
    let Some(hosted) = host(true, &count, &observed, |client| async move {
        client.cancel().await.unwrap();
    }) else {
        return;
    };

    assert!(hosted.took < Duration::from_secs(2), "{:?}", hosted.took);
    assert_eq!(observed.entries(), ["host closing"]);
}

#[test]
fn an_app_that_does_not_close_is_dropped_after_the_grace_period() {
    let count = UpdateCount::default();
    let observed = Observed::default();
    let Some(hosted) = host(false, &count, &observed, |client| async move {
        client.cancel().await.unwrap();
    }) else {
        return;
    };

    assert!(hosted.took >= Duration::from_secs(5), "{:?}", hosted.took);
    assert!(hosted.took < Duration::from_secs(8), "{:?}", hosted.took);
    assert!(count.get() <= 5 * 60 + 2, "{}", count.get());
}

#[test]
fn a_close_from_the_app_ends_the_loop() {
    let count = UpdateCount::default();
    let observed = Observed::default();
    let Some(hosted) = host(true, &count, &observed, |client| async move {
        call(&client, "quit").await
    }) else {
        return;
    };

    assert_eq!(hosted.result, "closing");
    assert!(observed.entries().is_empty(), "{:?}", observed.entries());
}
