use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use concerto_app::{
    App,
    edit_time::AtEditTime,
    schedule_groups::{LateUpdate, Startup, Update},
};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, Resource, With, Without,
    command::CommandQueue,
    component::name::Name,
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::On,
    signal::Signal,
    signal::listener::GlobalListener,
};
use concerto_editor::{
    asset_editor::{EditorDocument, Unsaved},
    guard::UnsavedGuard,
    hosted::{EditTarget, EditWorld, EditWorldSignals},
    project::{EditorCommand, EditorCommands, ProjectState},
    scene::Play,
    tools::{camera::FlyCamera, entity_ops::EntityEdit, scene::SceneRoot, selection::Select},
};
use concerto_foundation::{
    assets::{
        Asset, AssetId, LoadableAsset,
        asset_server::{AssetLoadContext, AssetServer},
        content::{
            CONTENT_FORMAT_VERSION, ContentAssetHeader, ImportProvenance, write_content_asset,
        },
        handle::AssetHandle,
    },
    transform::{GlobalTransform, Transform},
};
use concerto_mcp::{AppMcpExt, McpTool, Needs, ToolCalls, host::HostClosing, host::run_with};
use concerto_mesh::Aabb;
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_render::components::{camera::Camera, light::Light};
use concerto_scene::scene::{Scene, SceneNode};
use concerto_ui::{text::UIText, text_input::UITextInput};
use concerto_window::input::{Input, KeyCode, MouseButton, PhysicalKey};
use glam::{Vec2, Vec3};
use image::{ImageFormat, RgbaImage};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const SIZE: (u32, u32) = (800, 600);
const PATIENCE: Duration = Duration::from_secs(120);

static ONE_APP_AT_A_TIME: Mutex<()> = Mutex::new(());

#[derive(Resource, Default)]
struct EditFrames(u32);

fn count_edit_frames(mut frames: ResMut<EditFrames>) {
    frames.0 += 1;
}

fn bound_by_name(named: Query<(Entity, &Name), Without<Aabb>>, mut cmd: CommandQueue) {
    for (entity, name) in named.iter() {
        let x = match name.as_str() {
            "Sword" => 9.0,
            "Rig" => -10.0,
            _ => continue,
        };
        cmd.insert(
            Aabb {
                min: Vec3::new(x, 0.0, 0.0),
                max: Vec3::new(x + 1.0, 2.0, 2.0),
            },
            entity,
        );
    }
}

#[derive(Component, Reflect, Default)]
#[reflect(SceneComponent)]
struct Unreadable;

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default, SceneComponent)]
struct Samples {
    values: Vec<f32>,
    gain: f32,
}

fn sample_the_sword(named: Query<(Entity, &Name), Without<Samples>>, mut cmd: CommandQueue) {
    for (entity, name) in named.iter() {
        if name.as_str() == "Sword" {
            let values = (0..100).map(|value| value as f32).collect();
            cmd.insert((Samples { values, gain: 0.5 }, Unreadable), entity);
        }
    }
}

#[derive(Reflect, Default)]
struct Stride {
    length: f32,
    height: f32,
}

#[derive(Reflect)]
enum Pace {
    Still,
    Walk { stride: Stride },
    Run { stride: Stride, lean: f32 },
}

impl Default for Pace {
    fn default() -> Self {
        Self::Walk {
            stride: Stride {
                length: 1.0,
                height: 0.25,
            },
        }
    }
}

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
struct Gait {
    pace: Pace,
}

static RELEASE_THE_SLOW_ASSET: AtomicBool = AtomicBool::new(false);

const SLOW_ADDRESS: &str = "content/slow.gasset";

#[derive(Serialize, Deserialize)]
struct SlowAsset;

impl Asset for SlowAsset {
    fn name() -> &'static str {
        "SlowAsset"
    }
}

impl LoadableAsset for SlowAsset {
    fn on_load(&mut self, _context: &AssetLoadContext) -> anyhow::Result<()> {
        while !RELEASE_THE_SLOW_ASSET.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

struct LoadSlowly;

impl Signal for LoadSlowly {}

#[derive(Resource)]
struct SlowLoad {
    _handle: AssetHandle<SlowAsset>,
}

fn load_slowly(_: On<LoadSlowly>, server: Res<AssetServer>, mut cmd: CommandQueue) {
    let handle = server.load::<SlowAsset>(AssetId::from_path(SLOW_ADDRESS));
    cmd.insert_resource(SlowLoad { _handle: handle });
}

static OPEN_THE_GATE: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

const GATED_ADDRESS: &str = "content/gated.gasset";

#[derive(Serialize, Deserialize)]
struct GatedAsset {
    gate: usize,
}

impl Asset for GatedAsset {
    fn name() -> &'static str {
        "GatedAsset"
    }
}

impl LoadableAsset for GatedAsset {
    fn on_load(&mut self, _context: &AssetLoadContext) -> anyhow::Result<()> {
        while !OPEN_THE_GATE[self.gate].load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    }
}

#[derive(Resource)]
struct GatedLoad {
    _handle: AssetHandle<GatedAsset>,
}

fn load_through_the_gate(server: Res<AssetServer>, mut cmd: CommandQueue) {
    let handle = server.load::<GatedAsset>(AssetId::from_path(GATED_ADDRESS));
    cmd.insert_resource(GatedLoad { _handle: handle });
}

fn gated_game(app: &mut App) {
    game(app);
    app.register_asset::<GatedAsset>()
        .add_system(Startup, load_through_the_gate);
}

static HERO_SEEN: Mutex<Option<Instant>> = Mutex::new(None);

fn note_the_hero(named: Query<&Name>) {
    let mut seen = HERO_SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    if seen.is_none() && named.iter().any(|name| name.as_str() == "Hero") {
        *seen = Some(Instant::now());
    }
}

fn watched_game(app: &mut App) {
    game(app);
    app.add_system(LateUpdate, note_the_hero);
}

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
struct Received {
    w_presses: u32,
    clicks: u32,
    pointer: Vec2,
}

fn spawn_the_counter(mut cmd: CommandQueue) {
    cmd.spawn((Name::new("Counter"), Received::default()));
}

fn count_input(input: Res<Input>, counters: Query<&mut Received>) {
    for mut received in counters.iter() {
        if input.is_just_pressed(PhysicalKey::Code(KeyCode::KeyW)) {
            received.w_presses += 1;
        }
        if input.is_mouse_button_just_pressed(MouseButton::Left) {
            received.clicks += 1;
        }
        if received.pointer != input.mouse_position() {
            received.pointer = input.mouse_position();
        }
    }
}

fn counting_game(app: &mut App) {
    game(app);
    app.register_type::<Received>()
        .add_system(Startup, spawn_the_counter)
        .add_system(Update, count_input);
}

fn game(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins)
        .register_asset::<SlowAsset>()
        .add_listener(load_slowly.at_edit_time())
        .register_type::<Unreadable>()
        .register_type::<Samples>()
        .register_type::<other::Samples>()
        .register_type::<Gait>()
        .insert_resource(EditFrames::default())
        .add_system(Update, count_edit_frames.at_edit_time())
        .add_system(Update, bound_by_name.at_edit_time())
        .add_system(Update, sample_the_sword.at_edit_time());
}

#[derive(Resource, Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
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

#[derive(Resource)]
struct FixtureScene(AssetId);

#[derive(Resource)]
struct Closing;

#[derive(Resource)]
struct NeverInserted;

#[derive(Deserialize, JsonSchema)]
struct State {}

#[derive(Deserialize, JsonSchema)]
struct OpenScene {}

#[derive(Deserialize, JsonSchema)]
struct Rename {}

#[derive(Deserialize, JsonSchema)]
struct SeenFrames {}

#[derive(Deserialize, JsonSchema)]
struct WantsProject {}

#[derive(Deserialize, JsonSchema)]
struct WantsPlaying {}

#[derive(Deserialize, JsonSchema)]
struct SelectHero {}

#[derive(Deserialize, JsonSchema)]
struct Ids {}

#[derive(Deserialize, JsonSchema)]
struct PlayScene {}

#[derive(Deserialize, JsonSchema)]
struct CameraAt {}

#[derive(Deserialize, JsonSchema)]
struct DrawnCameraAt {}

#[derive(Deserialize, JsonSchema)]
struct StartSlowLoad {}

#[derive(Deserialize, JsonSchema)]
struct EditFramesSoFar {}

#[derive(Deserialize, JsonSchema)]
struct TextFields {}

fn state(
    mut calls: ToolCalls<State>,
    project: Res<ProjectState>,
    guard: Res<UnsavedGuard>,
    documents: Query<(&EditorDocument, Option<&EditTarget>, Option<&Unsaved>)>,
) {
    for (_, reply) in calls.read() {
        let mut text = if project.project.is_some() {
            "project".to_owned()
        } else if project.busy() {
            "opening".to_owned()
        } else {
            "none".to_owned()
        };
        for (document, target, unsaved) in documents.iter() {
            text.push_str(&format!(" {}", document.title));
            if target.is_some() {
                text.push_str(" target");
            }
            if unsaved.is_some() {
                text.push_str(" dirty");
            }
        }
        if guard.asking() {
            text.push_str(" asking");
        }
        reply.ok(text);
    }
}

fn open_scene(
    mut calls: ToolCalls<OpenScene>,
    scene: Res<FixtureScene>,
    mut commands: ResMut<EditorCommands>,
) {
    for (_, reply) in calls.read() {
        commands.0.push_back(EditorCommand::OpenAsset(scene.0));
        reply.ok("queued");
    }
}

fn rename(
    mut calls: ToolCalls<Rename>,
    names: EditWorld<Query<(Entity, &Name)>>,
    mut scene: EditWorldSignals,
) {
    for (_, reply) in calls.read() {
        let hero = names
            .iter()
            .find(|(_, name)| name.as_str() == "Hero")
            .map(|(entity, _)| entity);
        match hero {
            Some(entity) => {
                scene.trigger(EntityEdit::Rename {
                    entity,
                    name: "Villain".into(),
                });
                reply.ok("renamed");
            }
            None => reply.ok("no hero"),
        }
    }
}

fn select_hero(
    mut calls: ToolCalls<SelectHero>,
    names: EditWorld<Query<(Entity, &Name)>>,
    mut scene: EditWorldSignals,
) {
    for (_, reply) in calls.read() {
        let hero = names
            .iter()
            .find(|(_, name)| name.as_str() == "Hero")
            .map(|(entity, _)| entity)
            .unwrap();
        scene.trigger(Select(Some(hero)));
        reply.ok(hero.to_id_string());
    }
}

fn ids(
    mut calls: ToolCalls<Ids>,
    names: EditWorld<Query<(Entity, &Name)>>,
    roots: EditWorld<Query<Entity, With<SceneRoot>>>,
    listeners: EditWorld<Query<Entity, With<GlobalListener>>>,
) {
    for (_, reply) in calls.read() {
        let mut ids = serde_json::Map::new();
        for (entity, name) in names.iter() {
            ids.insert(name.as_str().into(), entity.to_id_string().into());
        }
        if let Some(root) = roots.iter().next() {
            ids.insert("root".into(), root.to_id_string().into());
        }
        ids.insert(
            "listeners".into(),
            listeners
                .iter()
                .map(|entity| Value::from(entity.to_id_string()))
                .collect(),
        );
        reply.ok(Value::Object(ids).to_string());
    }
}

fn play_scene(
    mut calls: ToolCalls<PlayScene>,
    targets: Query<Entity, With<EditTarget>>,
    mut cmd: CommandQueue,
) {
    for (_, reply) in calls.read() {
        match targets.iter().next() {
            Some(document) => {
                cmd.trigger(Play(document));
                reply.ok("playing");
            }
            None => reply.ok("no target"),
        }
    }
}

fn camera_at(mut calls: ToolCalls<CameraAt>, fly: EditWorld<Res<FlyCamera>>) {
    for (_, reply) in calls.read() {
        reply.ok(format!("{:.2}", fly.position.x));
    }
}

fn drawn_camera_at(
    mut calls: ToolCalls<DrawnCameraAt>,
    cameras: EditWorld<Query<&GlobalTransform, With<Camera>>>,
) {
    for (_, reply) in calls.read() {
        let x = cameras.iter().next().map(|global| global.translation().x);
        reply.ok(format!("{:.2}", x.unwrap_or(f32::NAN)));
    }
}

fn start_slow_load(mut calls: ToolCalls<StartSlowLoad>, mut scene: EditWorldSignals) {
    for (_, reply) in calls.read() {
        scene.trigger(LoadSlowly);
        reply.ok("loading");
    }
}

fn edit_frames_so_far(mut calls: ToolCalls<EditFramesSoFar>, frames: EditWorld<Res<EditFrames>>) {
    for (_, reply) in calls.read() {
        reply.ok(frames.0.to_string());
    }
}

fn text_fields(mut calls: ToolCalls<TextFields>, fields: Query<&UIText, With<UITextInput>>) {
    for (_, reply) in calls.read() {
        let mut values: Vec<&str> = fields.iter().map(|field| field.text.as_str()).collect();
        values.sort_unstable();
        reply.ok(values.join("|"));
    }
}

fn seen_frames(
    mut calls: ToolCalls<SeenFrames>,
    frames: EditWorld<Res<EditFrames>>,
    log: Res<Log>,
) {
    for (_, reply) in calls.read() {
        log.push(format!("tool {}", frames.0));
        reply.ok(frames.0.to_string());
    }
}

fn wants_project(mut calls: ToolCalls<WantsProject>, _: Res<NeverInserted>) {
    for (_, reply) in calls.read() {
        reply.ok("unreachable");
    }
}

fn wants_playing(mut calls: ToolCalls<WantsPlaying>, _: Res<NeverInserted>) {
    for (_, reply) in calls.read() {
        reply.ok("unreachable");
    }
}

fn note_update(frames: EditWorld<Res<EditFrames>>, log: Res<Log>) {
    log.push(format!("update {}", frames.0));
}

fn note_closing(_: On<HostClosing>, log: Res<Log>, mut cmd: CommandQueue) {
    log.push("host closing");
    cmd.insert_resource(Closing);
}

fn note_prompt(closing: Option<Res<Closing>>, guard: Res<UnsavedGuard>, log: Res<Log>) {
    if closing.is_some() {
        log.push(format!("asking {}", guard.asking()));
    }
}

fn write_asset(root: &Path, address: &str, id: AssetId, kind: &str, payload: &[u8]) {
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: id,
        references: vec![],
        kind: kind.into(),
        provenance: Some(ImportProvenance {
            source: "fixture.glb".into(),
            sub_asset: address.into(),
        }),
    };
    let path = root.join(address);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, write_content_asset(&header, payload).unwrap()).unwrap();
}

struct Fixture {
    directory: tempfile::TempDir,
    scene: AssetId,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("content")).unwrap();
        let node = |name: &str, children: Vec<usize>| {
            let mut node = SceneNode {
                name: name.into(),
                children,
                components: vec![],
            };
            node.push_component(&Transform::IDENTITY).unwrap();
            node
        };
        let mut bulb = node("Bulb", vec![]);
        bulb.push_component(&Light::point_light()).unwrap();
        let scene = Scene {
            nodes: vec![
                node("Hero", vec![1, 3]),
                node("Lamp", vec![2]),
                bulb,
                node("Sword", vec![]),
                node("Rig", vec![]),
            ],
            referenced_assets: vec![],
        };
        let id = AssetId::new();
        write_asset(
            directory.path(),
            "content/level.gasset",
            id,
            Scene::name(),
            &bincode::serialize(&scene).unwrap(),
        );
        Self {
            directory,
            scene: id,
        }
    }

    fn add_scene(&self, address: &str) -> AssetId {
        let mut node = SceneNode {
            name: "Crate".into(),
            children: vec![],
            components: vec![],
        };
        node.push_component(&Transform::IDENTITY).unwrap();
        let scene = Scene {
            nodes: vec![node],
            referenced_assets: vec![],
        };
        let id = AssetId::new();
        write_asset(
            self.directory.path(),
            address,
            id,
            Scene::name(),
            &bincode::serialize(&scene).unwrap(),
        );
        id
    }

    fn add(&self, address: &str, kind: &str) -> AssetId {
        let id = AssetId::new();
        write_asset(self.directory.path(), address, id, kind, &[]);
        id
    }

    fn break_catalogue(&self) {
        std::fs::write(self.directory.path().join("content.toml"), "root = \"\"\n").unwrap();
    }

    fn root(&self) -> PathBuf {
        self.directory.path().to_path_buf()
    }

    fn scene_file(&self) -> Vec<u8> {
        std::fs::read(self.directory.path().join("content/level.gasset")).unwrap()
    }
}

type Client = RunningService<RoleClient, ()>;

struct Served<T> {
    result: T,
    closed_after: Option<Duration>,
}

fn serve<T, F>(
    fixture: &Fixture,
    log: &Log,
    scenario: impl FnOnce(Client) -> F + Send + 'static,
) -> Option<Served<T>>
where
    T: Send + 'static,
    F: Future<Output = T>,
{
    serve_with(fixture, log, game, |_| {}, scenario)
}

fn serve_with<T, F>(
    fixture: &Fixture,
    log: &Log,
    recipe: fn(&mut App),
    configure: impl FnOnce(&mut App),
    scenario: impl FnOnce(Client) -> F + Send + 'static,
) -> Option<Served<T>>
where
    T: Send + 'static,
    F: Future<Output = T>,
{
    let _turn = ONE_APP_AT_A_TIME
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let cancelled = Arc::new(Mutex::new(None::<Instant>));
    let cancelled_by_client = cancelled.clone();
    let client = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let client = ().serve(client_io).await.ok()?;
                let result = scenario(client).await;
                *cancelled_by_client
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
                Some(result)
            })
    });
    let root = fixture.root();
    let scene = fixture.scene;
    let log = log.clone();
    let served = run_with(
        server_io,
        move |app| {
            concerto_editor::mcp::editor(recipe, root)(app);
            app.insert_resource(log)
                .insert_resource(FixtureScene(scene))
                .add_tool::<State, _>(McpTool::read_only("state"), state)
                .add_tool::<OpenScene, _>(McpTool::new("open_scene"), open_scene)
                .add_tool::<Rename, _>(McpTool::new("rename").needs(Needs::Scene), rename)
                .add_tool::<SeenFrames, _>(
                    McpTool::new("seen_frames").needs(Needs::Scene),
                    seen_frames,
                )
                .add_tool::<SelectHero, _>(
                    McpTool::new("select_hero").needs(Needs::Scene),
                    select_hero,
                )
                .add_tool::<Ids, _>(McpTool::read_only("ids").needs(Needs::Scene), ids)
                .add_tool::<PlayScene, _>(McpTool::new("play_scene"), play_scene)
                .add_tool::<CameraAt, _>(
                    McpTool::read_only("camera_at").needs(Needs::Scene),
                    camera_at,
                )
                .add_tool::<DrawnCameraAt, _>(
                    McpTool::read_only("drawn_camera_at").needs(Needs::Scene),
                    drawn_camera_at,
                )
                .add_tool::<StartSlowLoad, _>(
                    McpTool::new("start_slow_load").needs(Needs::Scene),
                    start_slow_load,
                )
                .add_tool::<EditFramesSoFar, _>(
                    McpTool::read_only("edit_frames_so_far").needs(Needs::Scene),
                    edit_frames_so_far,
                )
                .add_tool::<TextFields, _>(McpTool::read_only("text_fields"), text_fields)
                .add_tool::<WantsProject, _>(
                    McpTool::read_only("wants_project").needs(Needs::Project),
                    wants_project,
                )
                .add_tool::<WantsPlaying, _>(
                    McpTool::read_only("wants_playing").needs(Needs::Playing),
                    wants_playing,
                )
                .add_system(Update, note_update)
                .add_system(LateUpdate, note_prompt)
                .add_listener(note_closing);
            configure(app);
        },
        SIZE,
    );
    let returned = Instant::now();
    if served.is_err() {
        eprintln!("skipped: no GPU adapter");
        return None;
    }
    let result = client
        .join()
        .expect("the client's assertions hold")
        .expect("the client connects");
    let closed_after = cancelled
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .map(|cancelled| returned.saturating_duration_since(cancelled));
    Some(Served {
        result,
        closed_after,
    })
}

async fn call(client: &Client, tool: &'static str) -> CallToolResult {
    call_with(client, tool, json!({})).await
}

async fn call_with(client: &Client, tool: &'static str, arguments: Value) -> CallToolResult {
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

async fn ok(client: &Client, tool: &'static str) -> String {
    let result = call(client, tool).await;
    assert_eq!(result.is_error, Some(false), "{tool}: {result:?}");
    text(&result)
}

async fn ok_with(client: &Client, tool: &'static str, arguments: Value) -> String {
    let result = call_with(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(false), "{tool}: {result:?}");
    text(&result)
}

async fn error(client: &Client, tool: &'static str) -> Value {
    error_with(client, tool, json!({})).await
}

async fn error_with(client: &Client, tool: &'static str, arguments: Value) -> Value {
    let result = call_with(client, tool, arguments).await;
    assert_eq!(result.is_error, Some(true), "{tool}: {result:?}");
    serde_json::from_str(&text(&result)).unwrap()
}

async fn until(client: &Client, tool: &'static str, done: impl Fn(&str) -> bool) -> String {
    let started = Instant::now();
    loop {
        let answer = ok(client, tool).await;
        if done(&answer) {
            return answer;
        }
        assert!(
            started.elapsed() < PATIENCE,
            "{tool} still answers {answer}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn open(client: &Client) {
    until(client, "state", |state| state.starts_with("project")).await;
    assert_eq!(ok(client, "open_scene").await, "queued");
    until(client, "state", |state| state.contains("level target")).await;
}

#[test]
fn unmet_needs_answer_the_editor_codes() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        let errors = vec![
            error(&client, "wants_project").await,
            error(&client, "seen_frames").await,
            error(&client, "wants_playing").await,
        ];
        client.cancel().await.unwrap();
        errors
    }) else {
        return;
    };

    let codes: Vec<&str> = served
        .result
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["no_project", "no_scene", "not_playing"]);
    let messages: Vec<&str> = served
        .result
        .iter()
        .map(|error| error["message"].as_str().unwrap())
        .collect();
    assert_eq!(
        messages,
        [
            "Open a project first with open_project.",
            "Open a scene first with open_asset.",
            "Start the game with play.",
        ]
    );
}

#[test]
fn a_tool_sees_the_hosted_step_of_its_own_frame() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        let first: u32 = ok(&client, "seen_frames").await.parse().unwrap();
        let second: u32 = ok(&client, "seen_frames").await.parse().unwrap();
        client.cancel().await.unwrap();
        (first, second)
    }) else {
        return;
    };

    let (first, second) = served.result;
    assert!(second > first, "the edit App stepped: {first}, {second}");
    let entries = log.entries();
    for seen in [first, second] {
        let tool = entries
            .iter()
            .position(|entry| *entry == format!("tool {seen}"))
            .unwrap();
        assert_eq!(entries[tool + 1], format!("update {seen}"), "{entries:?}");
    }
}

#[test]
fn closing_the_client_discards_unsaved_edits_and_ends_with_no_prompt_open() {
    let fixture = Fixture::new();
    let saved = fixture.scene_file();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        assert_eq!(ok(&client, "rename").await, "renamed");
        let started = Instant::now();
        while !ok(&client, "state").await.contains("dirty") {
            assert!(
                started.elapsed() < PATIENCE,
                "the rename never marked the scene"
            );
            ok(&client, "seen_frames").await;
        }
        client.cancel().await.unwrap();
    }) else {
        return;
    };

    let closed_after = served.closed_after.expect("the client closed");
    assert!(closed_after < Duration::from_secs(5), "{closed_after:?}");
    let entries = log.entries();
    let closing = entries
        .iter()
        .position(|entry| entry == "host closing")
        .unwrap();
    let after: Vec<&str> = entries[closing + 1..]
        .iter()
        .map(String::as_str)
        .filter(|entry| entry.starts_with("asking"))
        .collect();
    assert!(after.contains(&"asking true"), "{entries:?}");
    assert_eq!(after.last(), Some(&"asking false"), "{entries:?}");
    assert_eq!(fixture.scene_file(), saved);
}

fn line<'a>(text: &'a str, prefix: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` line in:\n{text}"))
}

#[test]
fn status_reports_the_project_documents_and_the_edit_target_selection() {
    let fixture = Fixture::new();
    let root = fixture.root().canonicalize().unwrap();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        let before = until(&client, "status", |status| {
            !status.starts_with("project: none") && status.ends_with("busy: no")
        })
        .await;
        open(&client).await;
        let hero = ok(&client, "select_hero").await;
        let selected = format!("selection: {hero} Hero");
        let after = until(&client, "status", |status| {
            status.contains(&selected) && status.ends_with("busy: no")
        })
        .await;
        client.cancel().await.unwrap();
        (before, after, hero)
    }) else {
        return;
    };

    let (before, after, hero) = served.result;
    assert_eq!(line(&before, "project: "), root.display().to_string());
    line(&before, "project status: ");
    assert_eq!(line(&before, "documents: "), "none");
    assert_eq!(line(&before, "active: "), "none");
    assert_eq!(line(&before, "selection: "), "none");

    assert_eq!(line(&after, "project: "), root.display().to_string());
    let documents: Vec<&str> = after
        .lines()
        .skip_while(|line| *line != "documents:")
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .collect();
    assert_eq!(documents.len(), 1, "{after}");
    let (document, rest) = documents[0].trim_start().split_once(' ').unwrap();
    assert!(
        concerto_ecs::Entity::parse_id(document).is_some(),
        "{after}"
    );
    assert!(
        rest.starts_with("Scene content/level.gasset · editing"),
        "{after}"
    );
    assert_eq!(line(&after, "active: "), document);
    assert_eq!(line(&after, "selection: "), format!("{hero} Hero"));
    let (width, height) = line(&after, "viewport: ").split_once('x').unwrap();
    assert!(width.parse::<u32>().unwrap() > 0, "{after}");
    assert!(height.parse::<u32>().unwrap() > 0, "{after}");
}

async fn edit_frames(client: &Client) -> u32 {
    ok(client, "edit_frames_so_far").await.parse().unwrap()
}

#[test]
fn frames_run_while_the_edited_scene_loads_and_stop_once_it_has_loaded() {
    let fixture = Fixture::new();
    write_asset(
        &fixture.root(),
        SLOW_ADDRESS,
        AssetId::from_path(SLOW_ADDRESS),
        SlowAsset::name(),
        &[],
    );
    let Some(served) = serve(&fixture, &Log::default(), |client| async move {
        open(&client).await;
        until(&client, "status", |status| status.ends_with("busy: no")).await;
        ok(&client, "start_slow_load").await;
        let loading = ok(&client, "status").await;
        let first = edit_frames(&client).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let later = edit_frames(&client).await;
        RELEASE_THE_SLOW_ASSET.store(true, Ordering::SeqCst);
        until(&client, "status", |status| status.ends_with("busy: no")).await;
        let loaded = edit_frames(&client).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let idle = edit_frames(&client).await;
        client.cancel().await.unwrap();
        (loading, first, later, loaded, idle)
    }) else {
        return;
    };
    let (loading, first, later, loaded, idle) = served.result;
    assert!(loading.ends_with("busy: yes"), "{loading}");
    assert!(later > first, "{first} then {later} frames while loading");
    assert_eq!(idle, loaded, "frames ran once the scene had loaded");
}

#[test]
fn status_and_list_assets_without_a_project() {
    let fixture = Fixture::new();
    fixture.break_catalogue();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        let status = until(&client, "status", |status| {
            status.contains("Could not open project") && status.ends_with("busy: no")
        })
        .await;
        let error = error(&client, "list_assets").await;
        client.cancel().await.unwrap();
        (status, error)
    }) else {
        return;
    };

    let (status, error) = served.result;
    assert_eq!(line(&status, "project: "), "none");
    assert_eq!(line(&status, "selection: "), "none");
    assert_eq!(error["code"], "no_project");
}

#[test]
fn list_assets_filters_and_pages_the_catalogue() {
    let fixture = Fixture::new();
    let rock = fixture.add("content/meshes/rock.gasset", "Mesh");
    let tree = fixture.add("content/meshes/tree.gasset", "Mesh");
    let bark = fixture.add("content/textures/bark.gasset", "Texture");
    let level = fixture.scene;
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let pages = vec![
            ok(&client, "list_assets").await,
            ok_with(&client, "list_assets", json!({ "limit": 2 })).await,
            ok_with(&client, "list_assets", json!({ "limit": 2, "offset": 2 })).await,
            ok_with(
                &client,
                "list_assets",
                json!({ "kind": "Mesh", "limit": 1 }),
            )
            .await,
            ok_with(
                &client,
                "list_assets",
                json!({ "folder": "content/textures" }),
            )
            .await,
            ok_with(&client, "list_assets", json!({ "query": "TRE" })).await,
            ok_with(&client, "list_assets", json!({ "query": "nothing" })).await,
        ];
        let malformed = call_with(&client, "list_assets", json!({ "limit": "many" })).await;
        client.cancel().await.unwrap();
        (pages, malformed)
    }) else {
        return;
    };

    let entry = |address: &str, kind: &str, id: AssetId| {
        format!("{address} · {kind} · {}", id.simple_hex())
    };
    let level = entry("content/level.gasset", "Scene", level);
    let rock = entry("content/meshes/rock.gasset", "Mesh", rock);
    let tree = entry("content/meshes/tree.gasset", "Mesh", tree);
    let bark = entry("content/textures/bark.gasset", "Texture", bark);
    let (pages, malformed) = served.result;
    assert_eq!(
        pages[0],
        format!("4 assets match; showing 1–4:\n{level}\n{rock}\n{tree}\n{bark}")
    );
    assert_eq!(
        pages[1],
        format!(
            "4 assets match; showing 1–2:\n{level}\n{rock}\n… 2 more: list_assets {{\"limit\":2,\"offset\":2}}"
        )
    );
    assert_eq!(
        pages[2],
        format!("4 assets match; showing 3–4:\n{tree}\n{bark}")
    );
    assert_eq!(
        pages[3],
        format!(
            "2 assets match; showing 1–1:\n{rock}\n… 1 more: list_assets {{\"kind\":\"Mesh\",\"limit\":1,\"offset\":1}}"
        )
    );
    assert_eq!(pages[4], format!("1 asset matches; showing 1–1:\n{bark}"));
    assert_eq!(pages[5], format!("1 asset matches; showing 1–1:\n{tree}"));
    assert_eq!(pages[6], "No assets match. The project has 4 assets.");
    assert_eq!(malformed.is_error, Some(true), "{malformed:?}");
    let malformed: Value = serde_json::from_str(&text(&malformed)).unwrap();
    assert_eq!(malformed["code"], "invalid_arguments");
}

fn without_components(text: &str) -> String {
    text.lines()
        .map(|line| match line.rfind(" [") {
            Some(at) if line.ends_with(']') => &line[..at],
            _ => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn components<'a>(text: &'a str, id: &str) -> Vec<&'a str> {
    let line = text
        .lines()
        .find(|line| line.trim_start().starts_with(&format!("{id} ")))
        .unwrap_or_else(|| panic!("no line for {id} in:\n{text}"));
    let start = line.rfind(" [").unwrap() + 2;
    line[start..line.len() - 1]
        .split(", ")
        .filter(|name| !name.is_empty())
        .collect()
}

fn scene_id(ids: &Value, name: &str) -> String {
    ids[name]
        .as_str()
        .unwrap_or_else(|| panic!("no {name} in {ids}"))
        .to_owned()
}

#[test]
fn scene_tree_walks_the_open_scene_to_its_depth_and_limit() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let closed = error(&client, "scene_tree").await;
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let hero = Entity::parse_id(&scene_id(&ids, "Hero")).unwrap();
        let stale = format!("{}v{}", hero.index(), hero.generation().get() + 1);
        let trees = vec![
            ok(&client, "scene_tree").await,
            ok_with(
                &client,
                "scene_tree",
                json!({ "root": scene_id(&ids, "Lamp") }),
            )
            .await,
            ok_with(&client, "scene_tree", json!({ "depth": 1 })).await,
            ok_with(&client, "scene_tree", json!({ "limit": 3 })).await,
        ];
        let errors = vec![
            error_with(&client, "scene_tree", json!({ "root": stale })).await,
            error_with(&client, "scene_tree", json!({ "root": "999999v1" })).await,
            error_with(&client, "scene_tree", json!({ "root": "Hero" })).await,
        ];
        client.cancel().await.unwrap();
        (closed, ids, trees, errors)
    }) else {
        return;
    };

    let (closed, ids, trees, errors) = served.result;
    let id = |name: &str| scene_id(&ids, name);
    let (root, hero, lamp, bulb, sword, rig) = (
        id("root"),
        id("Hero"),
        id("Lamp"),
        id("Bulb"),
        id("Sword"),
        id("Rig"),
    );
    assert_eq!(closed["code"], "no_scene");
    assert_eq!(
        without_components(&trees[0]),
        format!(
            "{root} content/level.gasset\n  {hero} Hero\n    {lamp} Lamp\n      … 1 more below: scene_tree {{\"depth\":3,\"limit\":200,\"root\":\"{lamp}\"}}\n    {sword} Sword\n  {rig} Rig"
        )
    );
    assert!(
        components(&trees[0], &root).contains(&"Transform"),
        "{}",
        trees[0]
    );
    assert!(
        components(&trees[0], &hero).contains(&"Transform"),
        "{}",
        trees[0]
    );
    assert!(
        !components(&trees[0], &hero).contains(&"Light"),
        "{}",
        trees[0]
    );
    assert_eq!(
        without_components(&trees[1]),
        format!("{lamp} Lamp\n  {bulb} Bulb")
    );
    let bulb_components = components(&trees[1], &bulb);
    assert!(bulb_components.contains(&"Light"), "{}", trees[1]);
    assert!(bulb_components.contains(&"Transform"), "{}", trees[1]);
    let mut sorted = bulb_components.clone();
    sorted.sort_unstable();
    assert_eq!(bulb_components, sorted);
    assert_eq!(
        without_components(&trees[2]),
        format!(
            "{root} content/level.gasset\n  … 5 more below: scene_tree {{\"depth\":2,\"limit\":200,\"root\":\"{root}\"}}"
        )
    );
    assert_eq!(
        without_components(&trees[3]),
        format!(
            "{root} content/level.gasset\n  {hero} Hero\n    {lamp} Lamp\n      … 1 more below: scene_tree {{\"depth\":3,\"limit\":3,\"root\":\"{lamp}\"}}\n… 2 more within depth 3: scene_tree {{\"depth\":3,\"limit\":5}}"
        )
    );
    let codes: Vec<&str> = errors
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["stale_entity", "unknown_entity", "unknown_entity"]);
}

#[test]
fn find_entities_matches_names_and_components_with_their_paths() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let closed = error(&client, "find_entities").await;
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let found = vec![
            ok_with(&client, "find_entities", json!({ "name": "l" })).await,
            ok_with(&client, "find_entities", json!({ "name": "LAMP" })).await,
            ok_with(&client, "find_entities", json!({ "component": "light" })).await,
            ok_with(
                &client,
                "find_entities",
                json!({ "component": "concerto_render::components::light::Light", "name": "b" }),
            )
            .await,
            ok_with(&client, "find_entities", json!({ "name": "l", "limit": 1 })).await,
            ok_with(&client, "find_entities", json!({ "name": "nothing" })).await,
        ];
        client.cancel().await.unwrap();
        (closed, ids, found)
    }) else {
        return;
    };

    let (closed, ids, found) = served.result;
    let id = |name: &str| scene_id(&ids, name);
    let (lamp, bulb) = (id("Lamp"), id("Bulb"));
    assert_eq!(closed["code"], "no_scene");
    let lamp_line = format!("{lamp} content/level.gasset > Hero > Lamp");
    let bulb_line = format!("{bulb} content/level.gasset > Hero > Lamp > Bulb");
    assert_eq!(
        without_components(&found[0]),
        format!("2 entities match:\n{lamp_line}\n{bulb_line}")
    );
    assert!(
        components(&found[0], &bulb).contains(&"Light"),
        "{}",
        found[0]
    );
    assert_eq!(
        without_components(&found[1]),
        format!("1 entity matches:\n{lamp_line}")
    );
    assert_eq!(
        without_components(&found[2]),
        format!("1 entity matches:\n{bulb_line}")
    );
    assert_eq!(
        without_components(&found[3]),
        format!("1 entity matches:\n{bulb_line}")
    );
    assert_eq!(
        without_components(&found[4]),
        format!(
            "2 entities match:\n{lamp_line}\n… 1 more: find_entities {{\"limit\":2,\"name\":\"l\"}}"
        )
    );
    assert_eq!(found[5], "No entities match.");
}

fn inspected<'a>(inspection: &'a Value, name: &str) -> &'a Value {
    inspection["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|component| component["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in {inspection}"))
}

fn component_names(inspection: &Value) -> Vec<&str> {
    inspection["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|component| component["name"].as_str().unwrap())
        .collect()
}

#[test]
fn inspect_reads_components_as_json_with_their_paths_and_adapters() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let closed = error_with(&client, "inspect", json!({ "entity": "1v1" })).await;
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let tree = ok(&client, "scene_tree").await;
        let (hero, bulb, sword) = (
            scene_id(&ids, "Hero"),
            scene_id(&ids, "Bulb"),
            scene_id(&ids, "Sword"),
        );
        until(&client, "scene_tree", |tree| {
            components(tree, &sword).contains(&"Samples")
        })
        .await;
        let read = |arguments: Value| {
            let client = &client;
            async move {
                serde_json::from_str::<Value>(&ok_with(client, "inspect", arguments).await).unwrap()
            }
        };
        let inspections = vec![
            read(json!({ "entity": hero })).await,
            read(json!({ "entity": bulb, "components": ["light"] })).await,
            read(json!({
                "entity": bulb,
                "components": ["concerto_foundation::transform::Transform", "LIGHT"]
            }))
            .await,
            read(json!({ "entity": sword, "components": ["Samples", "Unreadable"] })).await,
        ];
        let errors = vec![
            error_with(&client, "inspect", json!({ "entity": "999999v1" })).await,
            error_with(
                &client,
                "inspect",
                json!({ "entity": hero, "components": ["Light"] }),
            )
            .await,
        ];
        client.cancel().await.unwrap();
        (closed, hero, tree, inspections, errors)
    }) else {
        return;
    };

    let (closed, hero, tree, inspections, errors) = served.result;
    assert_eq!(closed["code"], "no_scene");

    let whole = &inspections[0];
    assert_eq!(whole["entity"], hero.as_str());
    assert_eq!(whole["name"], "Hero");
    assert_eq!(component_names(whole), components(&tree, &hero));
    let transform = inspected(whole, "Transform");
    assert_eq!(
        transform["type"],
        "concerto_foundation::transform::Transform"
    );
    assert_eq!(
        transform["value"],
        json!({
            "rotation": [0.0, 0.0, 0.0, 1.0],
            "scale": [1.0, 1.0, 1.0],
            "translation": [0.0, 0.0, 0.0],
        })
    );
    assert_eq!(
        transform["paths"],
        json!([
            { "path": "translation", "adapter": true },
            { "path": "rotation", "adapter": true },
            { "path": "scale", "adapter": true },
        ])
    );

    assert_eq!(component_names(&inspections[1]), ["Light"]);
    let light = inspected(&inspections[1], "Light");
    assert_eq!(light["value"]["intensity"], 1.0);
    assert_eq!(light["value"]["light_type"], "Point");
    let paths = light["paths"].as_array().unwrap();
    let claimed = |path: &str| {
        paths
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("no {path} in {paths:?}"))["adapter"]
            .clone()
    };
    assert_eq!(claimed("intensity"), true);
    assert_eq!(claimed("shadowmaps_enabled"), true);
    assert_eq!(claimed("light_type"), false);
    assert_eq!(component_names(&inspections[2]), ["Light", "Transform"]);

    let samples = inspected(&inspections[3], "Samples");
    let values = samples["value"]["values"].as_array().unwrap();
    assert_eq!(values.len(), 17);
    assert_eq!(values[0], 0.0);
    assert_eq!(values[15], 15.0);
    assert_eq!(values[16], "… 84 more, 100 in all");
    assert_eq!(samples["value"]["gain"], 0.5);
    assert_eq!(
        samples["paths"],
        json!([
            { "path": "values", "adapter": false },
            { "path": "gain", "adapter": true },
        ])
    );
    let unreadable = inspected(&inspections[3], "Unreadable");
    assert_eq!(unreadable["value"], Value::Null);
    assert_eq!(unreadable["unavailable"], true);
    assert!(unreadable.get("paths").is_none(), "{unreadable}");

    assert_eq!(errors[0]["code"], "unknown_entity");
    assert_eq!(errors[1]["code"], "unknown_component");
    let message = errors[1]["message"].as_str().unwrap();
    assert!(message.contains("Transform"), "{message}");
}

#[test]
fn scene_tree_shows_the_running_game_without_its_listeners() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        assert_eq!(ok(&client, "play_scene").await, "playing");
        let started = Instant::now();
        let tree = loop {
            let result = call(&client, "scene_tree").await;
            let answer = text(&result);
            if result.is_error == Some(false) && answer.contains(" Hero") {
                break answer;
            }
            assert!(
                started.elapsed() < PATIENCE,
                "scene_tree still answers {answer}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let status = ok(&client, "status").await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let found = ok_with(&client, "find_entities", json!({ "name": "bulb" })).await;
        client.cancel().await.unwrap();
        (tree, status, ids, found)
    }) else {
        return;
    };

    let (tree, status, ids, found) = served.result;
    assert!(status.contains(" · playing"), "{status}");
    let listeners: Vec<&str> = ids["listeners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    assert!(!listeners.is_empty(), "{ids}");
    for line in tree.lines() {
        let first = line.split_whitespace().next().unwrap();
        assert!(
            !listeners.contains(&first),
            "{first} is a listener:\n{tree}"
        );
    }
    let hero = scene_id(&ids, "Hero");
    assert!(
        tree.lines()
            .any(|line| line.trim_start().starts_with(&format!("{hero} Hero "))),
        "{tree}"
    );
    assert!(found.starts_with("1 entity matches:\n"), "{found}");
    assert!(found.contains(" > Bulb ["), "{found}");
}

fn documents(status: &str) -> Vec<String> {
    status
        .lines()
        .skip_while(|line| *line != "documents:")
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .map(|line| line.trim_start().to_owned())
        .collect()
}

fn document_id(status: &str) -> String {
    let documents = documents(status);
    assert_eq!(documents.len(), 1, "{status}");
    documents[0].split_once(' ').unwrap().0.to_owned()
}

async fn dirty(client: &Client) {
    assert_eq!(ok(client, "rename").await, "renamed");
    let started = Instant::now();
    while !ok(client, "state").await.contains("dirty") {
        assert!(
            started.elapsed() < PATIENCE,
            "the rename never marked the scene"
        );
        ok(client, "seen_frames").await;
    }
}

#[test]
fn open_project_waits_for_the_project_and_reports_a_failure() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    other.add("content/meshes/rock.gasset", "Mesh");
    let other_root = other.root().canonicalize().unwrap();
    let missing = fixture.root().join("missing");
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, move |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let opened = ok_with(
            &client,
            "open_project",
            json!({ "path": other.root().display().to_string() }),
        )
        .await;
        let status = ok(&client, "status").await;
        let failed = error_with(
            &client,
            "open_project",
            json!({ "path": missing.display().to_string() }),
        )
        .await;
        let after = ok(&client, "status").await;
        client.cancel().await.unwrap();
        drop(other);
        (opened, status, failed, after)
    }) else {
        return;
    };

    let (opened, status, failed, after) = served.result;
    let root = other_root.display().to_string();
    assert_eq!(opened, format!("Opened {root}: 2 assets, 1 scene."));
    assert_eq!(line(&status, "project: "), root);
    assert_eq!(failed["code"], "open_failed");
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .starts_with("Could not open project"),
        "{failed}"
    );
    assert_eq!(line(&after, "project: "), root);
}

#[test]
fn open_asset_waits_for_a_scene_and_answers_at_once_when_it_is_shown() {
    let fixture = Fixture::new();
    let level = fixture.scene;
    let yard = fixture.add_scene("content/yard.gasset");
    fixture.add("content/meshes/rock.gasset", "Mesh");
    fixture.add("content/broken.gasset", "Scene");
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, move |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let first = ok_with(
            &client,
            "open_asset",
            json!({ "asset": "content/level.gasset" }),
        )
        .await;
        let state = ok(&client, "state").await;
        let status = ok(&client, "status").await;
        let again = ok_with(
            &client,
            "open_asset",
            json!({ "asset": level.simple_hex() }),
        )
        .await;
        let yard = ok_with(&client, "open_asset", json!({ "asset": yard.simple_hex() })).await;
        let tree = ok(&client, "scene_tree").await;
        let errors = vec![
            error_with(
                &client,
                "open_asset",
                json!({ "asset": "content/nothing.gasset" }),
            )
            .await,
            error_with(
                &client,
                "open_asset",
                json!({ "asset": "content/meshes/rock.gasset" }),
            )
            .await,
            error_with(
                &client,
                "open_asset",
                json!({ "asset": "content/broken.gasset" }),
            )
            .await,
        ];
        client.cancel().await.unwrap();
        (first, state, status, again, yard, tree, errors)
    }) else {
        return;
    };

    let (first, state, status, again, yard, tree, errors) = served.result;
    let document = document_id(&status);
    assert_eq!(
        first,
        format!("Opened content/level.gasset in document {document}.")
    );
    assert!(state.contains("level target"), "{state}");
    assert_eq!(again, first);
    assert_eq!(
        yard,
        format!("Opened content/yard.gasset in document {document}.")
    );
    assert!(tree.contains(" Crate "), "{tree}");
    let codes: Vec<&str> = errors
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["unknown_asset", "open_failed", "open_failed"]);
    assert!(
        errors[1]["message"].as_str().unwrap().contains("Mesh"),
        "{}",
        errors[1]
    );
}

#[test]
fn close_document_closes_the_document_status_names() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        let document = document_id(&ok(&client, "status").await);
        let errors = vec![
            error_with(&client, "close_document", json!({ "document": "level" })).await,
            error_with(&client, "close_document", json!({ "document": "999999v1" })).await,
        ];
        let closed = ok_with(&client, "close_document", json!({ "document": document })).await;
        let state = ok(&client, "state").await;
        client.cancel().await.unwrap();
        (document, errors, closed, state)
    }) else {
        return;
    };

    let (document, errors, closed, state) = served.result;
    let codes: Vec<&str> = errors
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["unknown_entity", "unknown_entity"]);
    assert_eq!(closed, format!("Closed document {document}."));
    assert_eq!(state, "project");
}

#[test]
fn a_call_that_would_discard_edits_cancels_the_prompt_and_fails() {
    let fixture = Fixture::new();
    fixture.add_scene("content/yard.gasset");
    let root = fixture.root().display().to_string();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, move |client| async move {
        open(&client).await;
        let document = document_id(&ok(&client, "status").await);
        dirty(&client).await;
        let mut answers = Vec::new();
        for (tool, arguments) in [
            ("open_asset", json!({ "asset": "content/yard.gasset" })),
            ("close_document", json!({ "document": document })),
            ("open_project", json!({ "path": root })),
        ] {
            let error = error_with(&client, tool, arguments).await;
            let state = ok(&client, "state").await;
            answers.push((error, state));
        }
        client.cancel().await.unwrap();
        (document, answers)
    }) else {
        return;
    };

    let (document, answers) = served.result;
    for (error, state) in answers {
        assert_eq!(error["code"], "unsaved_changes", "{error}");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains(&format!("{document} content/level.gasset")),
            "{error}"
        );
        assert_eq!(state, "project level target dirty");
    }
}

async fn rename_to(client: &Client, from: &str, to: &str) {
    let ids: Value = serde_json::from_str(&ok(client, "ids").await).unwrap();
    ok_with(
        client,
        "rename_entity",
        json!({ "entity": scene_id(&ids, from), "name": to }),
    )
    .await;
    let started = Instant::now();
    while !ok(client, "state").await.contains("dirty") {
        assert!(
            started.elapsed() < PATIENCE,
            "the rename never marked the scene"
        );
        ok(client, "seen_frames").await;
    }
}

fn holds(file: &[u8], name: &str) -> bool {
    file.windows(name.len())
        .any(|window| window == name.as_bytes())
}

struct OverriddenOpens {
    document: String,
    refused: Value,
    discarded: String,
    discarded_file: Vec<u8>,
    unused: String,
    after_unused: String,
    saved: String,
    saved_file: Vec<u8>,
    state: String,
}

#[test]
fn open_asset_discards_or_saves_unsaved_edits_when_told_to() {
    let fixture = Fixture::new();
    fixture.add_scene("content/yard.gasset");
    let original = fixture.scene_file();
    let level = fixture.root().join("content/level.gasset");
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, move |client| async move {
        open(&client).await;
        let document = document_id(&ok(&client, "status").await);
        let yard = |unsaved: Option<&str>| match unsaved {
            Some(unsaved) => json!({ "asset": "content/yard.gasset", "unsaved": unsaved }),
            None => json!({ "asset": "content/yard.gasset" }),
        };
        rename_to(&client, "Hero", "Villain").await;
        let refused = error_with(&client, "open_asset", yard(None)).await;
        let discarded = ok_with(&client, "open_asset", yard(Some("discard"))).await;
        let discarded_file = std::fs::read(&level).unwrap();
        let unused = ok_with(
            &client,
            "open_asset",
            json!({ "asset": "content/level.gasset", "unsaved": "save" }),
        )
        .await;
        let after_unused = ok(&client, "scene_tree").await;
        rename_to(&client, "Hero", "Knight").await;
        let saved = ok_with(&client, "open_asset", yard(Some("save"))).await;
        let saved_file = std::fs::read(&level).unwrap();
        let state = ok(&client, "state").await;
        client.cancel().await.unwrap();
        OverriddenOpens {
            document,
            refused,
            discarded,
            discarded_file,
            unused,
            after_unused,
            saved,
            saved_file,
            state,
        }
    }) else {
        return;
    };

    let OverriddenOpens {
        document,
        refused,
        discarded,
        discarded_file,
        unused,
        after_unused,
        saved,
        saved_file,
        state,
    } = served.result;
    let yard = format!("Opened content/yard.gasset in document {document}.");
    assert_eq!(refused["code"], "unsaved_changes", "{refused}");
    assert_eq!(discarded, yard);
    assert_eq!(discarded_file, original);
    assert_eq!(
        unused,
        format!("Opened content/level.gasset in document {document}.")
    );
    assert!(after_unused.contains(" Hero "), "{after_unused}");
    assert!(!after_unused.contains("Villain"), "{after_unused}");
    assert_eq!(saved, yard);
    assert!(holds(&saved_file, "Knight"));
    assert!(!holds(&saved_file, "Hero"));
    assert!(!state.contains("dirty"), "{state}");
    assert!(!state.contains("asking"), "{state}");
}

struct OverriddenCloses {
    closed: String,
    closed_state: String,
    closed_file: Vec<u8>,
    reopened: String,
    reopened_file: Vec<u8>,
    failed: Value,
    failed_state: String,
}

#[test]
fn close_document_and_open_project_take_the_override_and_a_failed_save_answers() {
    let fixture = Fixture::new();
    fixture.add_scene("content/yard.gasset");
    let original = fixture.scene_file();
    let root = fixture.root();
    let canonical = root.canonicalize().unwrap();
    let level = root.join("content/level.gasset");
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, move |client| async move {
        open(&client).await;
        let document = document_id(&ok(&client, "status").await);
        rename_to(&client, "Hero", "Villain").await;
        let closed = ok_with(
            &client,
            "close_document",
            json!({ "document": document, "unsaved": "discard" }),
        )
        .await;
        let closed_state = ok(&client, "state").await;
        let closed_file = std::fs::read(&level).unwrap();
        let level_asset = json!({ "asset": "content/level.gasset" });
        ok_with(&client, "open_asset", level_asset.clone()).await;
        rename_to(&client, "Hero", "Knight").await;
        let reopened = ok_with(
            &client,
            "open_project",
            json!({ "path": root.display().to_string(), "unsaved": "save" }),
        )
        .await;
        let reopened_file = std::fs::read(&level).unwrap();
        ok_with(&client, "open_asset", level_asset).await;
        rename_to(&client, "Knight", "Rogue").await;
        std::fs::remove_file(&level).unwrap();
        std::fs::create_dir(&level).unwrap();
        let failed = error_with(
            &client,
            "open_asset",
            json!({ "asset": "content/yard.gasset", "unsaved": "save" }),
        )
        .await;
        let failed_state = ok(&client, "state").await;
        client.cancel().await.unwrap();
        OverriddenCloses {
            closed,
            closed_state,
            closed_file,
            reopened,
            reopened_file,
            failed,
            failed_state,
        }
    }) else {
        return;
    };

    let OverriddenCloses {
        closed,
        closed_state,
        closed_file,
        reopened,
        reopened_file,
        failed,
        failed_state,
    } = served.result;
    assert!(closed.starts_with("Closed document "), "{closed}");
    assert_eq!(closed_state, "project");
    assert_eq!(closed_file, original);
    assert!(
        reopened.starts_with(&format!("Opened {}:", canonical.display())),
        "{reopened}"
    );
    assert!(holds(&reopened_file, "Knight"));
    assert_eq!(failed["code"], "save_failed", "{failed}");
    assert!(
        failed["message"]
            .as_str()
            .unwrap()
            .starts_with("Save failed"),
        "{failed}"
    );
    assert_eq!(failed_state, "project level target dirty");
}

#[test]
fn select_sets_and_clears_the_selection() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let closed = error_with(&client, "select", json!({ "entity": "1v1" })).await;
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let hero = scene_id(&ids, "Hero");
        let selected = ok_with(&client, "select", json!({ "entity": hero })).await;
        let status = ok(&client, "status").await;
        let unknown = error_with(&client, "select", json!({ "entity": "999999v1" })).await;
        let cleared = ok(&client, "select").await;
        let after = ok(&client, "status").await;
        client.cancel().await.unwrap();
        (closed, hero, selected, status, unknown, cleared, after)
    }) else {
        return;
    };

    let (closed, hero, selected, status, unknown, cleared, after) = served.result;
    assert_eq!(closed["code"], "no_scene");
    assert_eq!(selected, format!("Selected {hero} Hero."));
    assert_eq!(line(&status, "selection: "), format!("{hero} Hero"));
    assert_eq!(unknown["code"], "unknown_entity");
    assert_eq!(cleared, "Selection cleared.");
    assert_eq!(line(&after, "selection: "), "none");
}

#[test]
fn frame_answers_once_the_camera_has_framed() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let closed = error(&client, "frame").await;
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let sword = scene_id(&ids, "Sword");
        let before = ok(&client, "camera_at").await;
        let framed = ok_with(&client, "frame", json!({ "entity": sword })).await;
        let on_sword = ok(&client, "camera_at").await;
        let drawn_on_sword = ok(&client, "drawn_camera_at").await;
        let status = ok(&client, "status").await;
        let all = ok(&client, "frame").await;
        let on_all = ok(&client, "camera_at").await;
        let drawn_on_all = ok(&client, "drawn_camera_at").await;
        client.cancel().await.unwrap();
        (
            closed,
            sword,
            before,
            framed,
            (on_sword, drawn_on_sword),
            status,
            all,
            (on_all, drawn_on_all),
        )
    }) else {
        return;
    };

    let (closed, sword, before, framed, on_sword, status, all, on_all) = served.result;
    assert_eq!(closed["code"], "no_scene");
    assert_eq!(before, "0.00");
    let level = "yaw 0.000, pitch -0.150, looking along (0.000, -0.149, -0.989).";
    assert!(
        framed.starts_with(&format!("Framed {sword} Sword. Camera at (9.500, ")),
        "{framed}"
    );
    assert!(framed.ends_with(level), "{framed}");
    assert_eq!(on_sword, ("9.50".into(), "9.50".into()));
    assert_eq!(line(&status, "selection: "), format!("{sword} Sword"));
    assert!(
        all.starts_with("Framed the whole scene. Camera at (0.000, "),
        "{all}"
    );
    assert!(all.ends_with(level), "{all}");
    assert_eq!(on_all, ("0.00".into(), "0.00".into()));
}

#[test]
fn set_camera_places_the_camera_by_angles_or_by_a_point_to_face() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let angles = json!({ "position": [1.0, 2.0, 3.0], "yaw": 0.5, "pitch": 0.25 });
        let closed = error_with(&client, "set_camera", angles.clone()).await;
        open(&client).await;
        let turned = ok_with(&client, "set_camera", angles).await;
        let at = ok(&client, "camera_at").await;
        let facing = ok_with(
            &client,
            "set_camera",
            json!({ "position": [0.0, 0.0, 0.0], "look_at": [-1.0, std::f32::consts::SQRT_2, -1.0] }),
        )
        .await;
        let mut invalid = Vec::new();
        for arguments in [
            json!({ "position": [0.0, 0.0, 0.0], "look_at": [0.0, 0.0, -1.0], "yaw": 0.0, "pitch": 0.0 }),
            json!({ "position": [0.0, 0.0, 0.0] }),
            json!({ "position": [0.0, 0.0, 0.0], "yaw": 1.0 }),
            json!({ "position": [1.0, 1.0, 1.0], "look_at": [1.0, 1.0, 1.0] }),
        ] {
            invalid.push(error_with(&client, "set_camera", arguments).await);
        }
        let unmoved = ok(&client, "camera_at").await;
        client.cancel().await.unwrap();
        (closed, turned, at, facing, invalid, unmoved)
    }) else {
        return;
    };

    let (closed, turned, at, facing, invalid, unmoved) = served.result;
    assert_eq!(closed["code"], "no_scene");
    assert_eq!(
        turned,
        "Camera at (1.000, 2.000, 3.000), yaw 0.500, pitch 0.250, looking along (-0.465, 0.247, -0.850)."
    );
    assert_eq!(at, "1.00");
    assert_eq!(
        facing,
        "Camera at (0.000, 0.000, 0.000), yaw 0.785, pitch 0.785, looking along (-0.500, 0.707, -0.500)."
    );
    for error in &invalid {
        assert_eq!(error["code"], "invalid_arguments", "{error}");
    }
    assert_eq!(unmoved, "0.00");
}

#[test]
fn set_camera_and_frame_are_refused_while_the_game_plays() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        assert_eq!(ok(&client, "play_scene").await, "playing");
        until(&client, "status", |status| status.contains(" · playing")).await;
        let started = Instant::now();
        while call(&client, "scene_tree").await.is_error != Some(false) {
            assert!(
                started.elapsed() < PATIENCE,
                "the game never became the edit target"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let posed = error_with(
            &client,
            "set_camera",
            json!({ "position": [0.0, 0.0, 0.0], "yaw": 0.0, "pitch": 0.0 }),
        )
        .await;
        let framed = error(&client, "frame").await;
        client.cancel().await.unwrap();
        (posed, framed)
    }) else {
        return;
    };

    let (posed, framed) = served.result;
    assert_eq!(posed["code"], "refused", "{posed}");
    assert_eq!(framed["code"], "refused", "{framed}");
}

fn picture_of(result: &CallToolResult) -> RgbaImage {
    assert_eq!(result.is_error, Some(false), "{result:?}");
    let image = result
        .content
        .iter()
        .find_map(|block| block.as_image())
        .unwrap_or_else(|| panic!("no image in {result:?}"));
    assert_eq!(image.mime_type, "image/png");
    let png = STANDARD.decode(&image.data).unwrap();
    image::load_from_memory_with_format(&png, ImageFormat::Png)
        .unwrap()
        .to_rgba8()
}

fn colours(picture: &RgbaImage) -> usize {
    picture
        .pixels()
        .map(|pixel| pixel.0)
        .collect::<std::collections::HashSet<_>>()
        .len()
}

struct Shots {
    editor: CallToolResult,
    small_editor: CallToolResult,
    closed: Value,
    zero: Value,
    status: String,
    viewport: CallToolResult,
    small_viewport: CallToolResult,
}

#[test]
fn screenshot_answers_the_viewport_and_the_whole_editor_as_png() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let editor = call_with(&client, "screenshot", json!({ "target": "editor" })).await;
        let small_editor = call_with(
            &client,
            "screenshot",
            json!({ "target": "editor", "max_edge": 200 }),
        )
        .await;
        let closed = error_with(&client, "screenshot", json!({ "target": "viewport" })).await;
        let zero = error_with(
            &client,
            "screenshot",
            json!({ "target": "editor", "max_edge": 0 }),
        )
        .await;
        open(&client).await;
        let status = until(&client, "status", |status| status.ends_with("busy: no")).await;
        let viewport = call_with(&client, "screenshot", json!({ "target": "viewport" })).await;
        let small_viewport = call_with(
            &client,
            "screenshot",
            json!({ "target": "viewport", "max_edge": 64 }),
        )
        .await;
        client.cancel().await.unwrap();
        Shots {
            editor,
            small_editor,
            closed,
            zero,
            status,
            viewport,
            small_viewport,
        }
    }) else {
        return;
    };

    let Shots {
        editor,
        small_editor,
        closed,
        zero,
        status,
        viewport,
        small_viewport,
    } = served.result;
    let whole = picture_of(&editor);
    assert_eq!(whole.dimensions(), SIZE);
    assert!(colours(&whole) > 1);
    assert_eq!(text(&editor), "");
    assert_eq!(picture_of(&small_editor).dimensions(), (200, 150));
    assert_eq!(closed["code"], "no_scene", "{closed}");
    assert_eq!(zero["code"], "invalid_arguments", "{zero}");

    let (width, height) = line(&status, "viewport: ").split_once('x').unwrap();
    let shown = (
        width.parse::<u32>().unwrap(),
        height.parse::<u32>().unwrap(),
    );
    let picture = picture_of(&viewport);
    assert_eq!(picture.dimensions(), shown, "{status}");
    assert!(colours(&picture) > 1);
    assert!(
        text(&viewport).starts_with("Camera at (0.000, "),
        "{viewport:?}"
    );
    let small = picture_of(&small_viewport).dimensions();
    assert_eq!(small.0.max(small.1), 64, "{small:?}");
    let longest = shown.0.max(shown.1);
    assert_eq!(
        (small.0.min(small.1)),
        (shown.0.min(shown.1) * 64 + longest / 2) / longest,
        "{small:?} from {shown:?}"
    );
}

fn tree_id(tree: &str, name: &str) -> String {
    tree.lines()
        .find_map(|line| {
            let mut words = line.split_whitespace();
            let id = words.next()?;
            (words.next() == Some(name)).then(|| id.to_owned())
        })
        .unwrap_or_else(|| panic!("no {name} in:\n{tree}"))
}

struct Walk {
    project: String,
    scene: String,
    tree: String,
    found: String,
    hero: String,
    sword: String,
    selected: String,
    framed: String,
    status: String,
    document: String,
    closed: String,
    after: String,
}

#[test]
fn one_client_opens_looks_selects_frames_and_closes() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    let other_root = other.root().canonicalize().unwrap();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, move |client| async move {
        until(&client, "status", |status| {
            !status.starts_with("project: none") && status.ends_with("busy: no")
        })
        .await;
        let project = ok_with(
            &client,
            "open_project",
            json!({ "path": other.root().display().to_string() }),
        )
        .await;
        let scene = ok_with(
            &client,
            "open_asset",
            json!({ "asset": "content/level.gasset" }),
        )
        .await;
        let tree = ok(&client, "scene_tree").await;
        let found = ok_with(&client, "find_entities", json!({ "name": "sword" })).await;
        let hero = tree_id(&tree, "Hero");
        let sword = tree_id(&tree, "Sword");
        let selected = ok_with(&client, "select", json!({ "entity": hero })).await;
        let framed = ok_with(&client, "frame", json!({ "entity": sword })).await;
        let status = ok(&client, "status").await;
        let document = document_id(&status);
        let closed = ok_with(&client, "close_document", json!({ "document": document })).await;
        let after = ok(&client, "status").await;
        client.cancel().await.unwrap();
        drop(other);
        Walk {
            project,
            scene,
            tree,
            found,
            hero,
            sword,
            selected,
            framed,
            status,
            document,
            closed,
            after,
        }
    }) else {
        return;
    };

    let Walk {
        project,
        scene,
        tree,
        found,
        hero,
        sword,
        selected,
        framed,
        status,
        document,
        closed,
        after,
    } = served.result;
    let root = other_root.display().to_string();
    assert_eq!(project, format!("Opened {root}: 1 asset, 1 scene."));
    assert_eq!(
        scene,
        format!("Opened content/level.gasset in document {document}.")
    );
    let lamp = tree_id(&tree, "Lamp");
    let rig = tree_id(&tree, "Rig");
    let top = tree_id(&tree, "content/level.gasset");
    assert_eq!(
        without_components(&tree),
        format!(
            "{top} content/level.gasset\n  {hero} Hero\n    {lamp} Lamp\n      … 1 more below: scene_tree {{\"depth\":3,\"limit\":200,\"root\":\"{lamp}\"}}\n    {sword} Sword\n  {rig} Rig"
        )
    );
    assert_eq!(
        without_components(&found),
        format!("1 entity matches:\n{sword} content/level.gasset > Hero > Sword")
    );
    assert_eq!(selected, format!("Selected {hero} Hero."));
    assert!(
        framed.starts_with(&format!("Framed {sword} Sword. Camera at ")),
        "{framed}"
    );
    assert_eq!(line(&status, "project: "), root);
    assert_eq!(line(&status, "active: "), document);
    assert_eq!(line(&status, "selection: "), format!("{sword} Sword"));
    assert!(
        documents(&status)[0].contains("Scene content/level.gasset · editing"),
        "{status}"
    );
    assert_eq!(closed, format!("Closed document {document}."));
    assert_eq!(line(&after, "project: "), root);
    assert_eq!(line(&after, "documents: "), "none");
    assert_eq!(line(&after, "active: "), "none");
    assert_eq!(line(&after, "selection: "), "none");
}

fn stale(id: &str) -> String {
    let (index, generation) = id.split_once('v').unwrap();
    format!("{index}v{}", generation.parse::<u32>().unwrap() + 1)
}

fn second_word(answer: &str) -> String {
    answer.split_whitespace().nth(1).unwrap().to_owned()
}

struct EntityEdits {
    hero: String,
    root: String,
    created: String,
    renamed: String,
    duplicated: String,
    deleted: String,
    top: String,
    tree: String,
    errors: Vec<Value>,
}

#[test]
fn the_entity_tools_create_rename_duplicate_and_delete() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let closed = error_with(&client, "create_entity", json!({})).await;
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let (hero, root) = (scene_id(&ids, "Hero"), scene_id(&ids, "root"));
        let created = ok_with(&client, "create_entity", json!({ "parent": hero })).await;
        let torch = second_word(&created);
        let renamed = ok_with(
            &client,
            "rename_entity",
            json!({ "entity": torch, "name": " Torch " }),
        )
        .await;
        let duplicated = ok_with(&client, "duplicate_entity", json!({ "entity": torch })).await;
        let copy = duplicated.split_whitespace().nth(4).unwrap().to_owned();
        let deleted = ok_with(&client, "delete_entity", json!({ "entity": copy })).await;
        let top = ok(&client, "create_entity").await;
        let tree = ok(&client, "scene_tree").await;
        let errors = vec![
            closed,
            error_with(&client, "delete_entity", json!({ "entity": root })).await,
            error_with(
                &client,
                "rename_entity",
                json!({ "entity": torch, "name": " " }),
            )
            .await,
            error_with(
                &client,
                "duplicate_entity",
                json!({ "entity": stale(&hero) }),
            )
            .await,
            error_with(&client, "delete_entity", json!({ "entity": "999999v1" })).await,
        ];
        client.cancel().await.unwrap();
        EntityEdits {
            hero,
            root,
            created,
            renamed,
            duplicated,
            deleted,
            top,
            tree,
            errors,
        }
    }) else {
        return;
    };

    let EntityEdits {
        hero,
        root,
        created,
        renamed,
        duplicated,
        deleted,
        top,
        tree,
        errors,
    } = served.result;
    let torch = second_word(&created);
    assert_eq!(created, format!("Created {torch} Entity in {hero} Hero."));
    assert_eq!(renamed, format!("Renamed {torch} to Torch."));
    let copy = duplicated.split_whitespace().nth(4).unwrap().to_owned();
    assert_ne!(copy, torch);
    assert_eq!(
        duplicated,
        format!("Duplicated {torch} Torch as {copy} Torch.")
    );
    assert_eq!(
        deleted,
        format!("Deleted {copy} Torch; the selection is {hero} Hero.")
    );
    let new = second_word(&top);
    assert!(
        top.starts_with(&format!("Created {new} Entity in {root}")),
        "{top}"
    );
    assert_eq!(tree_id(&tree, "Torch"), torch);
    assert!(
        tree.lines()
            .any(|line| line.starts_with(&format!("  {new} Entity "))),
        "{tree}"
    );
    assert!(!tree.contains(&format!("{copy} ")), "{tree}");
    let codes: Vec<&str> = errors
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "no_scene",
            "refused",
            "refused",
            "stale_entity",
            "unknown_entity"
        ]
    );
    assert_eq!(
        errors[1]["message"],
        "Delete failed: the scene root cannot be changed"
    );
    assert_eq!(
        errors[2]["message"],
        "Rename failed: a name cannot be empty"
    );
}

mod other {
    use concerto_ecs::{Component, reflect::ReflectComponent};
    use concerto_reflect::{Reflect, ReflectDefault};

    #[derive(Component, Reflect, Default)]
    #[reflect(Component, Default)]
    pub struct Samples {
        count: u32,
    }
}

#[test]
fn the_component_tools_add_and_remove_by_name() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let (hero, root) = (scene_id(&ids, "Hero"), scene_id(&ids, "root"));
        let added: Value = serde_json::from_str(
            &ok_with(
                &client,
                "add_component",
                json!({ "entity": hero, "component": "light" }),
            )
            .await,
        )
        .unwrap();
        let carried = ok_with(
            &client,
            "inspect",
            json!({ "entity": hero, "components": ["Light"] }),
        )
        .await;
        let twice = error_with(
            &client,
            "add_component",
            json!({ "entity": hero, "component": "Light" }),
        )
        .await;
        let removed = ok_with(
            &client,
            "remove_component",
            json!({ "entity": hero, "component": "concerto_render::components::light::Light" }),
        )
        .await;
        let errors = vec![
            twice,
            error_with(
                &client,
                "remove_component",
                json!({ "entity": hero, "component": "Light" }),
            )
            .await,
            error_with(
                &client,
                "add_component",
                json!({ "entity": hero, "component": "Lantern" }),
            )
            .await,
            error_with(
                &client,
                "add_component",
                json!({ "entity": hero, "component": "samples" }),
            )
            .await,
            error_with(
                &client,
                "remove_component",
                json!({ "entity": stale(&hero), "component": "Transform" }),
            )
            .await,
            error_with(
                &client,
                "add_component",
                json!({ "entity": root, "component": "Light" }),
            )
            .await,
        ];
        client.cancel().await.unwrap();
        (hero, added, carried, removed, errors)
    }) else {
        return;
    };

    let (hero, added, carried, removed, errors) = served.result;
    assert_eq!(added["name"], "Light");
    assert_eq!(added["type"], "concerto_render::components::light::Light");
    assert_eq!(added["value"]["intensity"], 1.0);
    assert!(added["paths"].as_array().unwrap().len() > 1, "{added}");
    let carried: Value = serde_json::from_str(&carried).unwrap();
    assert_eq!(component_names(&carried), ["Light"]);
    assert_eq!(removed, format!("Removed Light from {hero} Hero."));
    let codes: Vec<&str> = errors
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "refused",
            "unknown_component",
            "unknown_component",
            "ambiguous_component",
            "stale_entity",
            "refused"
        ]
    );
    assert_eq!(
        errors[0]["message"],
        "Add Light failed: it is already there, or has no default value"
    );
    assert_eq!(
        errors[5]["message"],
        "Add Light failed: the scene root cannot be changed"
    );
    let ambiguous = errors[3]["message"].as_str().unwrap();
    assert!(ambiguous.contains("mcp::Samples"), "{ambiguous}");
    assert!(ambiguous.contains("mcp::other::Samples"), "{ambiguous}");
}

struct PropertySets {
    intensity: Value,
    color: Value,
    rotation: Value,
    values: Value,
    after: Value,
    errors: Vec<Value>,
}

#[test]
fn set_property_writes_through_the_adapter_or_set_value_and_answers_the_component() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let (hero, bulb, sword, root) = (
            scene_id(&ids, "Hero"),
            scene_id(&ids, "Bulb"),
            scene_id(&ids, "Sword"),
            scene_id(&ids, "root"),
        );
        until(&client, "scene_tree", |tree| {
            components(tree, &sword).contains(&"Samples")
        })
        .await;
        let set = |entity: &str, component: &str, path: &str, value: Value| {
            let client = &client;
            let arguments =
                json!({ "entity": entity, "component": component, "path": path, "value": value });
            async move {
                serde_json::from_str::<Value>(&ok_with(client, "set_property", arguments).await)
                    .unwrap()
            }
        };
        let intensity = set(&bulb, "Light", "intensity", json!(3.5)).await;
        let color = set(&bulb, "light", "color", json!([1.0, 0.5, 0.25, 1.0])).await;
        let rotation = set(
            &hero,
            "Transform",
            "rotation",
            json!([0.0, 0.70710677, 0.0, 0.70710677]),
        )
        .await;
        let values = set(&sword, "Samples", "values", json!([1.0, 2.0, 3.0])).await;
        let refuse = |arguments: Value| {
            let client = &client;
            async move { error_with(client, "set_property", arguments).await }
        };
        let errors = vec![
            refuse(json!({ "entity": bulb, "component": "Light", "path": "intensity", "value": "NaN" })).await,
            refuse(json!({ "entity": bulb, "component": "Light", "path": "intensity", "value": "bright" })).await,
            refuse(json!({ "entity": bulb, "component": "Light", "path": "brightness", "value": 1.0 })).await,
            refuse(json!({ "entity": bulb, "component": "Camera", "path": "fov", "value": 1.0 })).await,
            refuse(json!({ "entity": stale(&bulb), "component": "Light", "path": "intensity", "value": 1.0 })).await,
            refuse(json!({ "entity": root, "component": "Transform", "path": "translation", "value": [1.0, 0.0, 0.0] })).await,
        ];
        let after: Value = serde_json::from_str(
            &ok_with(
                &client,
                "inspect",
                json!({ "entity": bulb, "components": ["Light"] }),
            )
            .await,
        )
        .unwrap();
        client.cancel().await.unwrap();
        PropertySets {
            intensity,
            color,
            rotation,
            values,
            after,
            errors,
        }
    }) else {
        return;
    };

    let PropertySets {
        intensity,
        color,
        rotation,
        values,
        after,
        errors,
    } = served.result;
    assert_eq!(intensity["name"], "Light");
    assert_eq!(intensity["value"]["intensity"], 3.5);
    assert!(intensity["paths"].is_array(), "{intensity}");
    assert_eq!(color["value"]["color"], json!([1.0, 0.5, 0.25, 1.0]));
    assert_eq!(color["value"]["intensity"], 3.5);
    let quaternion: Vec<f64> = rotation["value"]["rotation"]
        .as_array()
        .unwrap()
        .iter()
        .map(|component| component.as_f64().unwrap())
        .collect();
    let expected = [
        0.0,
        std::f64::consts::FRAC_1_SQRT_2,
        0.0,
        std::f64::consts::FRAC_1_SQRT_2,
    ];
    let dot: f64 = quaternion.iter().zip(expected).map(|(a, b)| a * b).sum();
    assert!(dot.abs() > 0.9999, "{rotation}");
    assert_eq!(rotation["value"]["translation"], json!([0.0, 0.0, 0.0]));
    assert_eq!(values["name"], "Samples");
    assert_eq!(values["value"]["values"], json!([1.0, 2.0, 3.0]));
    assert_eq!(values["value"]["gain"], 0.5);
    assert_eq!(inspected(&after, "Light")["value"]["intensity"], 3.5);
    let codes: Vec<&str> = errors
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "refused",
            "invalid_value",
            "unknown_path",
            "unknown_component",
            "stale_entity",
            "refused"
        ]
    );
    assert_eq!(
        errors[0]["message"],
        "Set Light.intensity failed: the value was refused"
    );
    assert_eq!(
        errors[5]["message"],
        "Set Transform.translation failed: the scene root cannot be changed"
    );
    let unknown_path = errors[2]["message"].as_str().unwrap();
    assert!(unknown_path.contains("intensity"), "{unknown_path}");
}

struct EnumSets {
    walk: Value,
    run: Value,
    spot: Value,
    directional: Value,
    after: Value,
    errors: Vec<Value>,
}

#[test]
fn set_property_writes_a_whole_enum_at_a_path_its_rows_sit_under() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let (bulb, rig) = (scene_id(&ids, "Bulb"), scene_id(&ids, "Rig"));
        let walk: Value = serde_json::from_str(
            &ok_with(
                &client,
                "add_component",
                json!({ "entity": rig, "component": "Gait" }),
            )
            .await,
        )
        .unwrap();
        let set = |entity: &str, component: &str, path: &str, value: Value| {
            let client = &client;
            let arguments =
                json!({ "entity": entity, "component": component, "path": path, "value": value });
            async move {
                serde_json::from_str::<Value>(&ok_with(client, "set_property", arguments).await)
                    .unwrap()
            }
        };
        let run = set(&rig, "Gait", "pace", json!({ "Run": { "lean": 0.5 } })).await;
        let spot = set(
            &bulb,
            "Light",
            "light_type",
            json!({ "Spot": { "cone_angle": 40.0 } }),
        )
        .await;
        let directional = set(&bulb, "Light", "light_type", json!("Directional")).await;
        let refuse = |arguments: Value| {
            let client = &client;
            async move { error_with(client, "set_property", arguments).await }
        };
        let errors = vec![
            refuse(json!({ "entity": rig, "component": "Gait", "path": "pace.stride", "value": { "length": 2.0 } })).await,
            refuse(json!({ "entity": rig, "component": "Gait", "path": "pace", "value": "Sprint" })).await,
        ];
        let after: Value = serde_json::from_str(
            &ok_with(
                &client,
                "inspect",
                json!({ "entity": rig, "components": ["Gait"] }),
            )
            .await,
        )
        .unwrap();
        client.cancel().await.unwrap();
        EnumSets {
            walk,
            run,
            spot,
            directional,
            after,
            errors,
        }
    }) else {
        return;
    };

    let EnumSets {
        walk,
        run,
        spot,
        directional,
        after,
        errors,
    } = served.result;
    assert_eq!(
        walk["value"],
        json!({ "pace": { "Walk": { "stride": { "height": 0.25, "length": 1.0 } } } })
    );
    assert_eq!(
        walk["paths"],
        json!([
            { "path": "pace", "adapter": false, "enum": true },
            { "path": "pace.stride.length", "adapter": true },
            { "path": "pace.stride.height", "adapter": true },
        ])
    );
    assert_eq!(
        run["value"],
        json!({ "pace": { "Run": { "lean": 0.5, "stride": { "height": 0.0, "length": 0.0 } } } })
    );
    assert_eq!(
        run["paths"],
        json!([
            { "path": "pace", "adapter": false, "enum": true },
            { "path": "pace.stride.length", "adapter": true },
            { "path": "pace.stride.height", "adapter": true },
            { "path": "pace.lean", "adapter": true },
        ])
    );
    assert_eq!(
        spot["value"]["light_type"],
        json!({ "Spot": { "cone_angle": 40.0 } })
    );
    let paths = |light: &Value| -> Vec<Value> {
        light["paths"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["path"].as_str().unwrap().starts_with("light_type"))
            .cloned()
            .collect()
    };
    assert_eq!(
        paths(&spot),
        [
            json!({ "path": "light_type", "adapter": false, "enum": true }),
            json!({ "path": "light_type.cone_angle", "adapter": true }),
        ]
    );
    assert_eq!(directional["value"]["light_type"], "Directional");
    assert_eq!(
        paths(&directional),
        [json!({ "path": "light_type", "adapter": false, "enum": true })]
    );
    let codes: Vec<&str> = errors
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["unknown_path", "refused"]);
    assert_eq!(
        errors[0]["message"],
        "Gait has no path `pace.stride`; its paths are pace, pace.stride.length, pace.stride.height, pace.lean."
    );
    assert_eq!(
        errors[1]["message"],
        "Set Gait.pace failed: pace: no variant 'Sprint'; the variants are Still, Walk, Run"
    );
    assert_eq!(inspected(&after, "Gait")["value"], run["value"]);
}

struct Saves {
    closed: Value,
    document: String,
    clean: String,
    saved: String,
    saved_file: Vec<u8>,
    state: String,
    again: String,
    unknown: Value,
    reread: Value,
}

#[test]
fn save_writes_the_scene_and_answers_when_there_is_nothing_to_save() {
    let fixture = Fixture::new();
    let level = fixture.root().join("content/level.gasset");
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, move |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let closed = error(&client, "save").await;
        open(&client).await;
        let document = document_id(&ok(&client, "status").await);
        let clean = ok(&client, "save").await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let sword = scene_id(&ids, "Sword");
        until(&client, "scene_tree", |tree| {
            components(tree, &sword).contains(&"Samples")
        })
        .await;
        ok_with(
            &client,
            "set_property",
            json!({ "entity": sword, "component": "Samples", "path": "gain", "value": 2.0 }),
        )
        .await;
        rename_to(&client, "Hero", "Villain").await;
        let saved = ok_with(&client, "save", json!({ "document": document })).await;
        let saved_file = std::fs::read(&level).unwrap();
        let state = ok(&client, "state").await;
        let again = ok(&client, "save").await;
        let unknown = error_with(&client, "save", json!({ "document": "999999v1" })).await;
        ok_with(&client, "close_document", json!({ "document": document })).await;
        ok_with(
            &client,
            "open_asset",
            json!({ "asset": "content/level.gasset" }),
        )
        .await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let reread = serde_json::from_str(
            &ok_with(
                &client,
                "inspect",
                json!({ "entity": scene_id(&ids, "Sword"), "components": ["Samples", "Unreadable"] }),
            )
            .await,
        )
        .unwrap();
        client.cancel().await.unwrap();
        Saves {
            closed,
            document,
            clean,
            saved,
            saved_file,
            state,
            again,
            unknown,
            reread,
        }
    }) else {
        return;
    };

    let Saves {
        closed,
        document,
        clean,
        saved,
        saved_file,
        state,
        again,
        unknown,
        reread,
    } = served.result;
    assert_eq!(closed["code"], "no_scene", "{closed}");
    let nothing = format!("Nothing to save in document {document} content/level.gasset.");
    assert_eq!(clean, nothing);
    assert_eq!(saved, "Saved content/level.gasset.");
    assert!(holds(&saved_file, "Villain"));
    assert!(!holds(&saved_file, "Hero"));
    assert!(holds(&saved_file, "Samples"));
    assert!(holds(&saved_file, "Unreadable"));
    assert_eq!(state, "project level target");
    assert_eq!(again, nothing);
    assert_eq!(unknown["code"], "unknown_entity", "{unknown}");
    assert_eq!(inspected(&reread, "Samples")["value"]["gain"], 2.0);
    assert_eq!(inspected(&reread, "Unreadable")["unavailable"], true);
}

struct Plays {
    document: String,
    idle: Value,
    played: String,
    played_file: Vec<u8>,
    status: String,
    again: Value,
    save_while: Value,
    game_tree: String,
    game_ids: Value,
    stopped: String,
    tree: String,
    ids: Value,
    state: String,
    idle_again: Value,
}

#[test]
fn play_runs_the_game_and_stop_brings_back_the_edited_scene() {
    let fixture = Fixture::new();
    let level = fixture.root().join("content/level.gasset");
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, move |client| async move {
        open(&client).await;
        let document = document_id(&ok(&client, "status").await);
        let idle = error(&client, "stop").await;
        rename_to(&client, "Hero", "Villain").await;
        let played = ok(&client, "play").await;
        let played_file = std::fs::read(&level).unwrap();
        let status = ok(&client, "status").await;
        let again = error(&client, "play").await;
        let save_while = error(&client, "save").await;
        let game_tree = until(&client, "scene_tree", |tree| tree.contains(" Villain ")).await;
        let game_ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        ok_with(
            &client,
            "rename_entity",
            json!({ "entity": scene_id(&game_ids, "Villain"), "name": "Knight" }),
        )
        .await;
        let stopped = ok(&client, "stop").await;
        let tree = ok(&client, "scene_tree").await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let state = ok(&client, "state").await;
        let idle_again = error(&client, "stop").await;
        client.cancel().await.unwrap();
        Plays {
            document,
            idle,
            played,
            played_file,
            status,
            again,
            save_while,
            game_tree,
            game_ids,
            stopped,
            tree,
            ids,
            state,
            idle_again,
        }
    }) else {
        return;
    };

    let Plays {
        document,
        idle,
        played,
        played_file,
        status,
        again,
        save_while,
        game_tree,
        game_ids,
        stopped,
        tree,
        ids,
        state,
        idle_again,
    } = served.result;
    assert_eq!(idle["code"], "not_playing", "{idle}");
    let saved = format!(
        "Playing content/level.gasset in document {document}; its unsaved changes were saved first; it loaded in "
    );
    assert!(played.starts_with(&saved), "{played}");
    assert!(played.ends_with(" s."), "{played}");
    assert!(holds(&played_file, "Villain"));
    assert!(status.contains(" · playing"), "{status}");
    assert_eq!(again["code"], "play_failed", "{again}");
    assert_eq!(again["message"], "The game is already playing");
    assert_eq!(save_while["code"], "save_failed", "{save_while}");
    assert_eq!(save_while["message"], "Stop to save");
    assert!(game_ids.get("root").is_none(), "{game_ids}");
    assert!(!game_tree.contains("Knight"), "{game_tree}");
    assert_eq!(
        stopped,
        format!(
            "Stopped the game in document {document}; the scene is back as it was before play."
        )
    );
    assert!(tree.contains(" Villain "), "{tree}");
    assert!(!tree.contains("Knight"), "{tree}");
    assert!(ids.get("root").is_some(), "{ids}");
    assert_eq!(state, "project level target");
    assert_eq!(idle_again["code"], "not_playing", "{idle_again}");
}

#[test]
fn the_panels_show_the_edited_scene_again_once_the_stopped_editor_is_quiet() {
    let fixture = Fixture::new();
    let Some(served) = serve(&fixture, &Log::default(), |client| async move {
        open(&client).await;
        ok(&client, "select_hero").await;
        until(&client, "status", |status| status.ends_with("busy: no")).await;
        let before = ok(&client, "text_fields").await;
        ok(&client, "play").await;
        ok(&client, "stop").await;
        until(&client, "status", |status| status.ends_with("busy: no")).await;
        let after = ok(&client, "text_fields").await;
        client.cancel().await.unwrap();
        (before, after)
    }) else {
        return;
    };

    let (before, after) = served.result;
    let fields: Vec<&str> = before.split('|').collect();
    assert!(fields.contains(&"Hero"), "{before}");
    assert!(fields.contains(&"1.000"), "{before}");
    assert_eq!(after, before);
}

fn gated_fixture(gate: usize) -> Fixture {
    let fixture = Fixture::new();
    write_asset(
        &fixture.root(),
        GATED_ADDRESS,
        AssetId::from_path(GATED_ADDRESS),
        GatedAsset::name(),
        &bincode::serialize(&GatedAsset { gate }).unwrap(),
    );
    fixture
}

#[test]
fn play_answers_once_what_the_game_loads_at_start_has_loaded() {
    let fixture = gated_fixture(0);
    let Some(served) = serve_with(
        &fixture,
        &Log::default(),
        gated_game,
        |_| {},
        |client| async move {
            open(&client).await;
            let document = document_id(&ok(&client, "status").await);
            let play = async {
                let played = ok(&client, "play").await;
                (played, Instant::now())
            };
            let open_the_gate = async {
                let status = until(&client, "status", |status| status.contains(" · playing")).await;
                tokio::time::sleep(Duration::from_millis(500)).await;
                OPEN_THE_GATE[0].store(true, Ordering::SeqCst);
                (status, Instant::now())
            };
            let ((played, answered), (status, opened)) = tokio::join!(play, open_the_gate);
            client.cancel().await.unwrap();
            (document, played, answered, status, opened)
        },
    ) else {
        OPEN_THE_GATE[0].store(true, Ordering::SeqCst);
        return;
    };

    let (document, played, answered, status, opened) = served.result;
    assert!(status.contains(" · playing"), "{status}");
    assert!(answered >= opened, "play answered before the gate opened");
    let loaded = format!("Playing content/level.gasset in document {document}; it loaded in ");
    assert!(played.starts_with(&loaded), "{played}");
    assert!(played.ends_with(" s."), "{played}");
}

#[test]
fn play_answers_that_the_game_is_still_loading_when_its_wait_runs_out() {
    let fixture = gated_fixture(1);
    let Some(served) = serve_with(
        &fixture,
        &Log::default(),
        gated_game,
        |app| {
            app.insert_resource(concerto_editor::mcp::PlayTimeout(Duration::from_secs(8)));
        },
        |client| async move {
            open(&client).await;
            let document = document_id(&ok(&client, "status").await);
            let played = call(&client, "play").await;
            let status = ok(&client, "status").await;
            OPEN_THE_GATE[1].store(true, Ordering::SeqCst);
            let stopped = call(&client, "stop").await;
            client.cancel().await.unwrap();
            (document, played, status, stopped)
        },
    ) else {
        OPEN_THE_GATE[1].store(true, Ordering::SeqCst);
        return;
    };

    let (document, played, status, stopped) = served.result;
    assert_eq!(played.is_error, Some(false), "{played:?}");
    assert_eq!(
        text(&played),
        format!(
            "Playing content/level.gasset in document {document}; it is still loading, so scene_tree and screenshot show it as it arrives."
        )
    );
    assert!(status.contains(" · playing"), "{status}");
    assert_eq!(stopped.is_error, Some(false), "{stopped:?}");
}

#[test]
fn play_answers_once_the_game_has_spawned_its_scene() {
    let fixture = Fixture::new();
    let Some(served) = serve_with(
        &fixture,
        &Log::default(),
        watched_game,
        |_| {},
        |client| async move {
            *HERO_SEEN.lock().unwrap_or_else(PoisonError::into_inner) = None;
            open(&client).await;
            ok(&client, "play").await;
            let answered = Instant::now();
            let seen = *HERO_SEEN.lock().unwrap_or_else(PoisonError::into_inner);
            client.cancel().await.unwrap();
            (seen, answered)
        },
    ) else {
        return;
    };

    let (seen, answered) = served.result;
    let seen = seen.expect("the game spawned its scene before play answered");
    assert!(seen <= answered);
}

fn viewport_size(status: &str) -> (f32, f32) {
    let (width, height) = line(status, "viewport: ").split_once('x').unwrap();
    (width.parse().unwrap(), height.parse().unwrap())
}

fn received(inspection: &str) -> Value {
    let inspection: Value = serde_json::from_str(inspection).unwrap();
    inspected(&inspection, "Received")["value"].clone()
}

fn near(value: &Value, expected: [f32; 2]) -> bool {
    value.as_array().is_some_and(|pair| {
        pair.len() == 2
            && pair.iter().zip(expected).all(|(got, wanted)| {
                got.as_f64()
                    .is_some_and(|got| (got - f64::from(wanted)).abs() < 0.01)
            })
    })
}

#[test]
fn send_input_reaches_the_game_as_the_editor_window_would_pass_it_on() {
    let fixture = Fixture::new();
    let Some(served) = serve_with(
        &fixture,
        &Log::default(),
        counting_game,
        |_| {},
        |client| async move {
            open(&client).await;
            ok(&client, "play").await;
            let status = ok(&client, "status").await;
            let (width, height) = viewport_size(&status);
            let document = document_id(&status);
            let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
            let counter = scene_id(&ids, "Counter");
            let read = || {
                let client = &client;
                let counter = counter.clone();
                async move { received(&ok_with(client, "inspect", json!({ "entity": counter })).await) }
            };
            let typed = ok_with(
                &client,
                "send_input",
                json!({ "events": [
                    { "key": "KeyW", "pressed": true },
                    { "key": "KeyW", "pressed": false },
                    { "key": "KeyW", "pressed": true, "text": "w" },
                ] }),
            )
            .await;
            let after_keys = read().await;
            ok_with(
                &client,
                "send_input",
                json!({ "events": [{ "key": "KeyW", "pressed": false }, { "move": [10.0, 20.0] }] }),
            )
            .await;
            let after_move = read().await;
            let centre = [(width / 2.0).floor(), (height / 2.0).floor()];
            ok_with(
                &client,
                "send_input",
                json!({ "events": [
                    { "button": "left", "pressed": true, "at": centre },
                    { "button": "left", "pressed": false },
                    { "wheel": [0.0, 1.0] },
                ] }),
            )
            .await;
            let after_click = read().await;
            client.cancel().await.unwrap();
            (document, typed, after_keys, after_move, centre, after_click)
        },
    ) else {
        return;
    };

    let (document, typed, after_keys, after_move, centre, after_click) = served.result;
    assert_eq!(
        typed,
        format!("Sent 3 window events to the game in document {document}; held in the game: KeyW.")
    );
    assert_eq!(after_keys["w_presses"], 2, "{after_keys}");
    assert!(near(&after_move["pointer"], [10.0, 20.0]), "{after_move}");
    assert_eq!(after_click["clicks"], 1, "{after_click}");
    assert!(near(&after_click["pointer"], centre), "{after_click}");
}

#[test]
fn send_input_refuses_unknown_keys_points_off_the_picture_and_a_game_without_input() {
    let fixture = Fixture::new();
    let Some(served) = serve_with(
        &fixture,
        &Log::default(),
        counting_game,
        |_| {},
        |client| async move {
            open(&client).await;
            let idle = error_with(
                &client,
                "send_input",
                json!({ "events": [{ "key": "KeyW", "pressed": true }] }),
            )
            .await;
            ok(&client, "play").await;
            let (width, _) = viewport_size(&ok(&client, "status").await);
            let unknown = error_with(
                &client,
                "send_input",
                json!({ "events": [{ "key": "KeyW", "pressed": true }, { "key": "Kew", "pressed": true }] }),
            )
            .await;
            let off = error_with(
                &client,
                "send_input",
                json!({ "events": [{ "move": [width, 0.0] }] }),
            )
            .await;
            let malformed = error_with(
                &client,
                "send_input",
                json!({ "events": [{ "key": "KeyW" }] }),
            )
            .await;
            let released = ok_with(
                &client,
                "send_input",
                json!({ "events": [{ "key": "ShiftLeft", "pressed": true }, { "key": "F1", "pressed": true }] }),
            )
            .await;
            let refused = error_with(
                &client,
                "send_input",
                json!({ "events": [{ "key": "KeyW", "pressed": true }] }),
            )
            .await;
            client.cancel().await.unwrap();
            (idle, unknown, off, malformed, released, refused, width)
        },
    ) else {
        return;
    };

    let (idle, unknown, off, malformed, released, refused, width) = served.result;
    assert_eq!(idle["code"], "not_playing", "{idle}");
    assert_eq!(unknown["code"], "invalid_arguments", "{unknown}");
    let message = unknown["message"].as_str().unwrap();
    assert!(
        message.starts_with("events[1]: no key named Kew"),
        "{message}"
    );
    assert!(message.contains("KeyW"), "{message}");
    assert_eq!(off["code"], "invalid_arguments", "{off}");
    assert!(
        off["message"].as_str().unwrap().contains(&format!(
            "outside the viewport's picture, which is {width}x"
        )),
        "{off}"
    );
    assert_eq!(malformed["code"], "invalid_arguments", "{malformed}");
    assert!(
        released.ends_with(" The editor has input now."),
        "{released}"
    );
    assert_eq!(refused["code"], "input_released", "{refused}");
}
