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
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, With, Without, World,
    command::{CommandQueue, EntityCommandQueue},
    component::{name::Name, scene::SceneEntityRef},
    events::{event_reader::EventReader, event_writer::EventWriter},
    reflect::{ReflectComponent, ReflectSceneComponent},
    system::input::SystemLocal,
    world::FromWorld,
};
use concerto_editor::{
    asset_editor::{
        AssetEditor, AssetEditorAppExt, AssetEditorRegistry, EditorDocument, Unsaved,
        finish_asset_request,
    },
    guard::UnsavedGuard,
    hosted::{EditTarget, EditWorld, EditWorldSignals, Hosted, WarmUpLimit},
    import::ImportFinished,
    inspector::{
        DocumentValue, EditContext, EditError, EditableApp, InspectedValues, PropertyEditor,
        PropertyTarget,
    },
    project::{EditorCommand, EditorCommands, ProjectState},
    tools::{
        camera::FlyCamera, edits::PropertyCommitted, entity_ops::EntityEdit, scene::SceneRoot,
    },
};
use concerto_foundation::{
    assets::{
        Asset, AssetId, LoadableAsset,
        asset_server::{AssetLoadContext, AssetServer},
        asset_store::AssetStore,
        content::{
            ContentAssetHeader, ImportProvenance, decode_content_asset, read_content_asset_header,
            write_content_asset,
        },
        handle::AssetHandle,
        registry::AssetRegistry,
    },
    transform::{GlobalTransform, Transform, systems::propagate_global_transforms},
};
use concerto_import::ImportedSource as Imported;
use concerto_mcp::{AppMcpExt, McpTool, Needs, ToolCalls, host::run_with};
use concerto_mesh::Aabb;
use concerto_reflect::{Reflect, ReflectDefault, ReflectRef};
use concerto_render::{
    assets::texture::{Compression, MaxSize, Texture, TextureSettings},
    components::{camera::Camera, light::Light},
    gpu::Gpu,
};
use concerto_scene::{
    link::{LinkedNode, SceneLinkSpawned},
    node_id::NodeId,
    scene::{LinkChange, Scene, SceneLinkData, SceneNode},
};
use concerto_ui::theme::UITheme;
use concerto_window::input::{GamepadButton, Input, KeyCode, MouseButton, PhysicalKey};
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

static OPEN_THE_GATE: AtomicBool = AtomicBool::new(false);

const GATED_ADDRESS: &str = "content/gated.gasset";

#[derive(Serialize, Deserialize)]
struct GatedAsset;

impl Asset for GatedAsset {
    fn name() -> &'static str {
        "GatedAsset"
    }
}

impl LoadableAsset for GatedAsset {
    fn on_load(&mut self, _context: &AssetLoadContext) -> anyhow::Result<()> {
        while !OPEN_THE_GATE.load(Ordering::SeqCst) {
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
        .add_system(Startup, load_through_the_gate)
        .add_system(LateUpdate, note_the_hero);
}

static HERO_SEEN: Mutex<Option<Instant>> = Mutex::new(None);

fn note_the_hero(named: Query<&Name>) {
    let mut seen = HERO_SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    if seen.is_none() && named.iter().any(|name| name.as_str() == "Hero") {
        *seen = Some(Instant::now());
    }
}

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
struct Received {
    w_presses: u32,
    f1_presses: u32,
    clicks: u32,
    pad_presses: u32,
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
        if input.is_just_pressed(PhysicalKey::Code(KeyCode::F1)) {
            received.f1_presses += 1;
        }
        if input.is_mouse_button_just_pressed(MouseButton::Left) {
            received.clicks += 1;
        }
        if input
            .gamepads()
            .any(|pad| pad.is_just_pressed(GamepadButton::South))
        {
            received.pad_presses += 1;
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
struct Ids {}

#[derive(Deserialize, JsonSchema)]
struct CameraAt {}

#[derive(Deserialize, JsonSchema)]
struct DrawnCameraAt {}

#[derive(Deserialize, JsonSchema)]
struct ExtensionState {}

#[derive(Resource)]
struct ExtensionInstalled {
    after_the_editor: bool,
}

struct ExtensionTool {
    listed: bool,
    state: String,
}

fn extension(app: &mut App) {
    let after_the_editor = app.get_resource::<AssetEditorRegistry>().is_some();
    app.insert_resource(ExtensionInstalled { after_the_editor })
        .add_tool::<ExtensionState, _>(McpTool::read_only("extension_state"), extension_state);
}

fn extension_state(
    mut calls: ToolCalls<ExtensionState>,
    installed: Option<Res<ExtensionInstalled>>,
) {
    for (_, reply) in calls.read() {
        reply.ok(match &installed {
            Some(installed) if installed.after_the_editor => "after the editor",
            Some(_) => "before the editor",
            None => "not installed",
        });
    }
}

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

fn ids(
    mut calls: ToolCalls<Ids>,
    names: EditWorld<Query<(Entity, &Name)>>,
    roots: EditWorld<Query<Entity, With<SceneRoot>>>,
) {
    for (_, reply) in calls.read() {
        let mut ids = serde_json::Map::new();
        for (entity, name) in names.iter() {
            ids.insert(name.as_str().into(), entity.to_id_string().into());
        }
        if let Some(root) = roots.iter().next() {
            ids.insert("root".into(), root.to_id_string().into());
        }
        reply.ok(Value::Object(ids).to_string());
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

fn write_asset(root: &Path, address: &str, id: AssetId, kind: &str, payload: &[u8]) {
    write_imported(root, address, id, kind, 1, None, payload);
}

fn write_scene(root: &Path, address: &str, id: AssetId, scene: &Scene) {
    write_imported(
        root,
        address,
        id,
        Scene::name(),
        Scene::FORMAT_VERSION,
        None,
        &bincode::serialize(scene).unwrap(),
    );
}

fn write_imported(
    root: &Path,
    address: &str,
    id: AssetId,
    kind: &str,
    kind_version: u32,
    provenance: Option<ImportProvenance>,
    payload: &[u8],
) {
    let header = ContentAssetHeader {
        asset_id: id,
        references: vec![],
        kind: kind.into(),
        kind_version,
        provenance,
        import: None,
        uncooked: None,
        derived: None,
        embedded: Vec::new(),
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
    fn without_a_scene() -> Self {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("content")).unwrap();
        Self {
            directory,
            scene: AssetId::new(),
        }
    }

    fn new() -> Self {
        let fixture = Self::without_a_scene();
        let node = |name: &str, children: Vec<usize>| {
            let mut node = SceneNode {
                id: NodeId::from_name_path(name),
                name: name.into(),
                children,
                components: vec![],
                link: None,
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
        write_scene(fixture.directory.path(), "content/level.gasset", id, &scene);
        Self {
            scene: id,
            ..fixture
        }
    }

    fn add_scene(&self, address: &str) -> AssetId {
        let mut node = SceneNode {
            id: NodeId::from_name_path("Crate"),
            name: "Crate".into(),
            children: vec![],
            components: vec![],
            link: None,
        };
        node.push_component(&Transform::IDENTITY).unwrap();
        let scene = Scene {
            nodes: vec![node],
            referenced_assets: vec![],
        };
        let id = AssetId::new();
        write_scene(self.directory.path(), address, id, &scene);
        id
    }

    fn add(&self, address: &str, kind: &str) -> AssetId {
        let id = AssetId::new();
        write_asset(self.directory.path(), address, id, kind, &[]);
        id
    }

    fn add_part(&self, address: &str, kind: &str, source: &str, sub_asset: &str) -> AssetId {
        let id = AssetId::new();
        let provenance = ImportProvenance {
            source: source.into(),
            sub_asset: sub_asset.into(),
        };
        write_imported(
            self.directory.path(),
            address,
            id,
            kind,
            1,
            Some(provenance),
            &[],
        );
        id
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
    serve_sized(fixture, log, recipe, configure, SIZE, scenario)
}

fn serve_sized<T, F>(
    fixture: &Fixture,
    log: &Log,
    recipe: fn(&mut App),
    configure: impl FnOnce(&mut App),
    size: (u32, u32),
    scenario: impl FnOnce(Client) -> F + Send + 'static,
) -> Option<Served<T>>
where
    T: Send + 'static,
    F: Future<Output = T>,
{
    let gpu = Gpu::request_for_test()?;
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
    run_with(
        server_io,
        gpu,
        move |app| {
            concerto_editor::mcp::editor(recipe, root, vec![extension])(app);
            app.insert_resource(log)
                .insert_resource(FixtureScene(scene))
                .add_tool::<State, _>(McpTool::read_only("state"), state)
                .add_tool::<OpenScene, _>(McpTool::new("open_scene"), open_scene)
                .add_tool::<Rename, _>(McpTool::new("rename").needs(Needs::Scene), rename)
                .add_tool::<SeenFrames, _>(
                    McpTool::new("seen_frames").needs(Needs::Scene),
                    seen_frames,
                )
                .add_tool::<Ids, _>(McpTool::read_only("ids").needs(Needs::Scene), ids)
                .add_tool::<CameraAt, _>(
                    McpTool::read_only("camera_at").needs(Needs::Scene),
                    camera_at,
                )
                .add_tool::<DrawnCameraAt, _>(
                    McpTool::read_only("drawn_camera_at").needs(Needs::Scene),
                    drawn_camera_at,
                )
                .add_tool::<WantsProject, _>(
                    McpTool::read_only("wants_project").needs(Needs::Project),
                    wants_project,
                )
                .add_tool::<WantsPlaying, _>(
                    McpTool::read_only("wants_playing").needs(Needs::Playing),
                    wants_playing,
                );
            configure(app);
        },
        size,
    );
    let returned = Instant::now();
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
    until_with(client, tool, json!({}), done).await
}

async fn until_with(
    client: &Client,
    tool: &'static str,
    arguments: Value,
    done: impl Fn(&str) -> bool,
) -> String {
    let started = Instant::now();
    loop {
        let answer = ok_with(client, tool, arguments.clone()).await;
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

async fn close_the_project_scene(client: &Client) {
    until(client, "state", |state| state.contains("level target")).await;
    let document = document_id(&ok(client, "status").await);
    ok_with(client, "close_document", json!({ "document": document })).await;
}

async fn open(client: &Client) {
    until(client, "state", |state| state.starts_with("project")).await;
    assert_eq!(ok(client, "open_scene").await, "queued");
    until(client, "state", |state| state.contains("level target")).await;
}

#[test]
fn unmet_needs_answer_the_editor_codes() {
    let fixture = Fixture::without_a_scene();
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
}

#[test]
fn closing_the_client_discards_unsaved_edits_and_quits() {
    let fixture = Fixture::new();
    let saved = fixture.scene_file();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        dirty(&client).await;
        client.cancel().await.unwrap();
    }) else {
        return;
    };

    let closed_after = served.closed_after.expect("the client closed");
    assert!(closed_after < Duration::from_secs(5), "{closed_after:?}");
    assert_eq!(fixture.scene_file(), saved);
}

fn line<'a>(text: &'a str, prefix: &str) -> &'a str {
    text.lines()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("no `{prefix}` line in:\n{text}"))
}

struct StatusReads {
    before: String,
    hero: String,
    selected: String,
    after: String,
    cleared: String,
    none: String,
}

#[test]
fn status_reports_the_project_documents_and_the_selection_that_select_sets_and_clears() {
    let fixture = Fixture::new();
    let root = fixture.root().canonicalize().unwrap();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        let before = until(&client, "status", |status| status.ends_with("busy: no")).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let hero = scene_id(&ids, "Hero");
        let selected = ok_with(&client, "select", json!({ "entity": hero })).await;
        let after = ok(&client, "status").await;
        let cleared = ok(&client, "select").await;
        let none = ok(&client, "status").await;
        client.cancel().await.unwrap();
        StatusReads {
            before,
            hero,
            selected,
            after,
            cleared,
            none,
        }
    }) else {
        return;
    };

    let StatusReads {
        before,
        hero,
        selected,
        after,
        cleared,
        none,
    } = served.result;
    assert_eq!(line(&before, "project: "), root.display().to_string());
    let document = document_id(&before);
    assert!(
        documents(&before)[0]
            .starts_with(&format!("{document} Scene content/level.gasset · editing")),
        "{before}"
    );
    assert_eq!(line(&before, "active: "), document);
    assert_eq!(line(&before, "values: "), "none");
    let ViewportSize { width, height } = viewport_size(&before);
    assert!(width > 0.0 && height > 0.0, "{before}");
    assert_eq!(selected, format!("Selected {hero} Hero."));
    assert_eq!(line(&after, "selection: "), format!("{hero} Hero"));
    assert_eq!(cleared, "Selection cleared.");
    assert_eq!(line(&none, "selection: "), "none");
}

struct Resized {
    reply: String,
    status: String,
}

struct Shown {
    layout: &'static str,
    width: f32,
    height: f32,
}

struct PanelSizes {
    before: String,
    sized: [Resized; 4],
    filling: Value,
}

async fn resize(client: &Client, panel: &str, size: f32) -> Resized {
    let reply = ok_with(
        client,
        "set_panel_size",
        json!({ "panel": panel, "size": size }),
    )
    .await;
    let status = ok(client, "status").await;
    Resized { reply, status }
}

#[test]
fn set_panel_size_resizes_the_panels_around_the_viewport_and_status_reports_them() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve_sized(
        &fixture,
        &log,
        game,
        |_| {},
        (1280, 720),
        |client| async move {
            open(&client).await;
            let before = until(&client, "status", |status| {
                status.contains("\nviewport: ") && !status.contains("viewport: not shown")
            })
            .await;
            let sized = [
                resize(&client, "left", 300.0).await,
                resize(&client, "right", 400.0).await,
                resize(&client, "bottom", 300.0).await,
                resize(&client, "left", 5000.0).await,
            ];
            let filling = error_with(
                &client,
                "set_panel_size",
                json!({ "panel": "browser", "size": 300.0 }),
            )
            .await;
            client.cancel().await.unwrap();
            PanelSizes {
                before,
                sized,
                filling,
            }
        },
    ) else {
        return;
    };

    let PanelSizes {
        before,
        sized,
        filling,
    } = served.result;
    assert_eq!(
        line(&before, "layout: "),
        "left 260 · right 320 · bottom 220 · browser fill"
    );
    let ViewportSize { width, height } = viewport_size(&before);
    let expected = [
        Shown {
            layout: "left 300 · right 320 · bottom 220 · browser fill",
            width: width - 40.0,
            height,
        },
        Shown {
            layout: "left 300 · right 400 · bottom 220 · browser fill",
            width: width - 120.0,
            height,
        },
        Shown {
            layout: "left 300 · right 400 · bottom 300 · browser fill",
            width: width - 120.0,
            height: height - 80.0,
        },
        Shown {
            layout: "left 528 · right 400 · bottom 300 · browser fill",
            width: width - 348.0,
            height: height - 80.0,
        },
    ];
    for (resized, shown) in sized.iter().zip(expected) {
        for answer in [&resized.reply, &resized.status] {
            assert_eq!(line(answer, "layout: "), shown.layout);
            assert_eq!(
                viewport_size(answer),
                ViewportSize {
                    width: shown.width,
                    height: shown.height,
                },
                "{answer}"
            );
        }
    }
    assert_eq!(filling["code"], "refused", "{filling}");
}

#[test]
fn a_plugin_given_to_the_editor_is_installed_after_it_and_adds_its_tools_under_mcp() {
    extension(&mut App::new());

    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        let listed = client
            .list_all_tools()
            .await
            .unwrap()
            .iter()
            .any(|tool| tool.name == "extension_state");
        let state = ok(&client, "extension_state").await;
        client.cancel().await.unwrap();
        ExtensionTool { listed, state }
    }) else {
        return;
    };

    assert!(served.result.listed);
    assert_eq!(served.result.state, "after the editor");
}

struct AssetPages {
    pages: Vec<String>,
    malformed: CallToolResult,
}

#[test]
fn list_assets_filters_pages_and_folds_an_import_under_its_scene() {
    let fixture = Fixture::new();
    let rock = fixture.add("content/meshes/rock.gasset", "Mesh");
    let tree = fixture.add("content/meshes/tree.gasset", "Mesh");
    let bark = fixture.add("content/textures/bark.gasset", "Texture");
    let source = "assets/props/crate.glb";
    let scene = fixture.add_part("content/props/crate/scene.gasset", "Scene", source, "scene");
    let mesh = fixture.add_part(
        "content/props/crate/mesh_0.gasset",
        "Mesh",
        source,
        "mesh/0",
    );
    let material = fixture.add_part(
        "content/props/crate/material_0.gasset",
        "StandardMaterial",
        source,
        "material/0",
    );
    let level = fixture.scene;
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        open(&client).await;
        until(&client, "list_assets", |assets| {
            assets.contains("placeable")
        })
        .await;
        let pages = vec![
            ok(&client, "list_assets").await,
            ok_with(&client, "list_assets", json!({ "limit": 2 })).await,
            ok_with(&client, "list_assets", json!({ "kind": "Mesh" })).await,
            ok_with(
                &client,
                "list_assets",
                json!({ "parts_of": "content/props/crate/scene.gasset" }),
            )
            .await,
            ok_with(
                &client,
                "list_assets",
                json!({ "folder": "content/textures" }),
            )
            .await,
            ok_with(&client, "list_assets", json!({ "query": "TRE" })).await,
        ];
        let malformed = call_with(&client, "list_assets", json!({ "limit": "many" })).await;
        client.cancel().await.unwrap();
        AssetPages { pages, malformed }
    }) else {
        return;
    };

    let entry = |address: &str, kind: &str, id: AssetId| {
        format!("{address} · {kind} · {}", id.simple_hex())
    };
    let placeable = |line: String| format!("{line} · placeable");
    let level = placeable(entry("content/level.gasset", "Scene", level));
    let rock = entry("content/meshes/rock.gasset", "Mesh", rock);
    let tree = entry("content/meshes/tree.gasset", "Mesh", tree);
    let bark = entry("content/textures/bark.gasset", "Texture", bark);
    let face = placeable(entry("content/props/crate/scene.gasset", "Scene", scene))
        + " · import of assets/props/crate.glb with 2 parts";
    let part = |address: &str, kind: &str, id: AssetId| {
        entry(address, kind, id) + " · part of content/props/crate/scene.gasset"
    };
    let mesh = part("content/props/crate/mesh_0.gasset", "Mesh", mesh);
    let material = part(
        "content/props/crate/material_0.gasset",
        "StandardMaterial",
        material,
    );
    let AssetPages { pages, malformed } = served.result;
    assert_eq!(
        pages[0],
        format!("5 assets match; showing 1–5:\n{level}\n{rock}\n{tree}\n{face}\n{bark}")
    );
    assert_eq!(
        pages[1],
        format!(
            "5 assets match; showing 1–2:\n{level}\n{rock}\n… 3 more: list_assets {{\"limit\":2,\"offset\":2}}"
        )
    );
    assert_eq!(
        pages[2],
        format!("3 assets match; showing 1–3:\n{rock}\n{tree}\n{mesh}")
    );
    assert_eq!(
        pages[3],
        format!("2 assets match; showing 1–2:\n{material}\n{mesh}")
    );
    assert_eq!(pages[4], format!("1 asset matches; showing 1–1:\n{bark}"));
    assert_eq!(pages[5], format!("1 asset matches; showing 1–1:\n{tree}"));
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

struct Looks {
    closed: Vec<Value>,
    ids: Value,
    trees: Vec<String>,
    found: Vec<String>,
}

#[test]
fn scene_tree_walks_the_open_scene_and_find_entities_searches_it() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        close_the_project_scene(&client).await;
        let closed = vec![
            error(&client, "scene_tree").await,
            error(&client, "find_entities").await,
        ];
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let trees = vec![
            ok(&client, "scene_tree").await,
            ok_with(
                &client,
                "scene_tree",
                json!({ "root": scene_id(&ids, "Lamp") }),
            )
            .await,
            ok_with(&client, "scene_tree", json!({ "depth": 1 })).await,
        ];
        let found = vec![
            ok_with(&client, "find_entities", json!({ "name": "l" })).await,
            ok_with(&client, "find_entities", json!({ "component": "light" })).await,
            ok_with(&client, "find_entities", json!({ "name": "l", "limit": 1 })).await,
        ];
        client.cancel().await.unwrap();
        Looks {
            closed,
            ids,
            trees,
            found,
        }
    }) else {
        return;
    };

    let Looks {
        closed,
        ids,
        trees,
        found,
    } = served.result;
    let id = |name: &str| scene_id(&ids, name);
    let root = id("root");
    let hero = id("Hero");
    let lamp = id("Lamp");
    let bulb = id("Bulb");
    let sword = id("Sword");
    let rig = id("Rig");
    for error in &closed {
        assert_eq!(error["code"], "no_scene", "{error}");
    }
    assert_eq!(
        without_components(&trees[0]),
        format!(
            "{root} content/level.gasset\n  {hero} Hero\n    {lamp} Lamp\n      … 1 more below: scene_tree {{\"depth\":3,\"limit\":200,\"root\":\"{lamp}\"}}\n    {sword} Sword\n  {rig} Rig"
        )
    );
    assert!(
        components(&trees[0], &hero).contains(&"Transform"),
        "{}",
        trees[0]
    );
    assert_eq!(
        without_components(&trees[1]),
        format!("{lamp} Lamp\n  {bulb} Bulb")
    );
    assert!(
        components(&trees[1], &bulb).contains(&"Light"),
        "{}",
        trees[1]
    );
    assert_eq!(
        without_components(&trees[2]),
        format!(
            "{root} content/level.gasset\n  … 5 more below: scene_tree {{\"depth\":2,\"limit\":200,\"root\":\"{root}\"}}"
        )
    );
    let lamp_line = format!("{lamp} content/level.gasset > Hero > Lamp");
    let bulb_line = format!("{bulb} content/level.gasset > Hero > Lamp > Bulb");
    assert_eq!(
        without_components(&found[0]),
        format!("2 entities match:\n{lamp_line}\n{bulb_line}")
    );
    assert_eq!(
        without_components(&found[1]),
        format!("1 entity matches:\n{bulb_line}")
    );
    assert_eq!(
        without_components(&found[2]),
        format!(
            "2 entities match:\n{lamp_line}\n… 1 more: find_entities {{\"limit\":2,\"name\":\"l\"}}"
        )
    );
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

struct Inspections {
    closed: Value,
    hero: String,
    tree: String,
    inspections: Vec<Value>,
    errors: Vec<Value>,
}

#[test]
fn inspect_reads_components_as_json_with_their_paths_and_adapters() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        close_the_project_scene(&client).await;
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
        Inspections {
            closed,
            hero,
            tree,
            inspections,
            errors,
        }
    }) else {
        return;
    };

    let Inspections {
        closed,
        hero,
        tree,
        inspections,
        errors,
    } = served.result;
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
    let mut sample_paths = vec![json!({ "path": "values", "adapter": false })];
    sample_paths
        .extend((0..16).map(|index| json!({ "path": format!("values.{index}"), "adapter": true })));
    sample_paths.push(json!("… 84 more of values, 100 in all"));
    sample_paths.push(json!({ "path": "gain", "adapter": true }));
    assert_eq!(samples["paths"], Value::Array(sample_paths));
    let unreadable = inspected(&inspections[3], "Unreadable");
    assert_eq!(unreadable["value"], Value::Null);
    assert_eq!(unreadable["unavailable"], true);
    assert!(unreadable.get("paths").is_none(), "{unreadable}");

    assert_eq!(errors[0]["code"], "unknown_entity");
    assert_eq!(errors[1]["code"], "unknown_component");
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

struct ImportedSource {
    imported: String,
    listed: String,
    picture: String,
    walk: String,
    reimported: String,
    errors: Vec<Value>,
}

const WALK: &str = "HIERARCHY
ROOT Hips
{
\tOFFSET 0.0 90.0 0.0
\tCHANNELS 6 Xposition Yposition Zposition Zrotation Yrotation Xrotation
\tEnd Site
\t{
\t\tOFFSET 0.0 5.0 0.0
\t}
}
MOTION
Frames: 2
Frame Time: 0.0333333
0 90 0 0 0 0
1 90 0 10 0 0
";

#[test]
fn import_source_writes_the_source_and_its_assets_into_the_project() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let sources = tempfile::tempdir().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../obj/tests/fixtures");
    for name in ["square.obj", "square.mtl"] {
        std::fs::copy(fixtures.join(name), sources.path().join(name)).unwrap();
    }
    let square = sources.path().join("square.obj").display().to_string();
    let picture = sources.path().join("picture.png");
    RgbaImage::new(8, 8).save(&picture).unwrap();
    let picture = picture.display().to_string();
    let walk = sources.path().join("walk.bvh");
    std::fs::write(&walk, WALK).unwrap();
    let walk = walk.display().to_string();
    let placed_walk = root.join("assets/walk.bvh");
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, move |client| async move {
        until(&client, "state", |state| state.starts_with("project")).await;
        let imported = ok_with(
            &client,
            "import_source",
            json!({ "source": square, "destination": "assets/props/square.obj" }),
        )
        .await;
        let picture = ok_with(
            &client,
            "import_source",
            json!({
                "source": picture,
                "asset_settings": { "compression": "Bc7", "max_size": "S512" },
            }),
        )
        .await;
        let walked = ok_with(
            &client,
            "import_source",
            json!({ "source": walk, "settings": { "scale": 0.5 } }),
        )
        .await;
        let reimported = ok_with(
            &client,
            "reimport",
            json!({ "asset": "content/walk/scene.gasset" }),
        )
        .await;
        let face = ok_with(&client, "list_assets", json!({ "query": "square" })).await;
        let parts = ok_with(
            &client,
            "list_assets",
            json!({ "parts_of": "content/props/square/scene.gasset" }),
        )
        .await;
        let listed = format!("{face}\n{parts}");
        let mut errors = vec![
            error_with(
                &client,
                "import_source",
                json!({ "source": "nowhere/missing.obj" }),
            )
            .await,
            error_with(
                &client,
                "import_source",
                json!({ "source": square, "destination": "../square.obj" }),
            )
            .await,
            error_with(
                &client,
                "import_source",
                json!({ "source": square, "asset_settings": { "mips": false } }),
            )
            .await,
            error_with(
                &client,
                "import_source",
                json!({ "source": walk, "settings": { "speed": 2.0 } }),
            )
            .await,
            error_with(
                &client,
                "import_source",
                json!({ "source": walk, "settings": { "scale": 0.0 } }),
            )
            .await,
            error_with(
                &client,
                "reimport",
                json!({ "asset": "content/level.gasset" }),
            )
            .await,
        ];
        std::fs::remove_file(&placed_walk).unwrap();
        errors.push(
            error_with(
                &client,
                "reimport",
                json!({ "asset": "content/walk/scene.gasset#skeleton/0" }),
            )
            .await,
        );
        until(&client, "status", |status| status.contains("busy: no")).await;
        client.cancel().await.unwrap();
        ImportedSource {
            imported,
            listed,
            picture,
            walk: walked,
            reimported,
            errors,
        }
    }) else {
        return;
    };

    let ImportedSource {
        imported,
        listed,
        picture,
        walk,
        reimported,
        errors,
    } = served.result;
    let texture = picture
        .lines()
        .find(|line| line.contains(" · Texture · "))
        .unwrap_or_else(|| panic!("no texture in {picture}"));
    let file = std::fs::read(root.join(texture.split(" · ").next().unwrap())).unwrap();
    let (_, texture) = decode_content_asset::<Texture>(&file).unwrap();
    assert_eq!(
        texture.settings,
        TextureSettings::default()
            .with_compression(Compression::Bc7)
            .with_max_size(MaxSize::S512)
    );
    assert!(
        picture.contains("[asset_settings]") && picture.contains("compression = \"Bc7\""),
        "{picture}"
    );
    let record = read_content_asset_header(&root.join("content/walk/scene.gasset"))
        .unwrap()
        .import
        .expect("the walk's scene holds its import record");
    let recorded = concerto_import::importer_for("bvh")
        .unwrap()
        .settings_from_bytes(record.settings_version, &record.settings)
        .unwrap()
        .expect("the record is at the importer's settings version");
    let ReflectRef::Struct(fields) = recorded.reflect_ref() else {
        panic!("BVH settings are a struct");
    };
    assert_eq!(
        fields
            .field("scale")
            .and_then(|scale| scale.downcast_ref::<f32>()),
        Some(&0.5)
    );
    assert!(
        walk.contains("[settings]\nframe_range = \"None\"\nscale = 0.5"),
        "{walk}"
    );
    let (reimported, kept) = reimported.split_once("\n\n").unwrap();
    let mut reimported = reimported.lines();
    assert_eq!(
        reimported.next(),
        Some("Re-imported assets/walk.bvh: 3 assets, 0 sibling files."),
    );
    assert_eq!(
        reimported.collect::<Vec<_>>(),
        walk.split("\n\n")
            .next()
            .unwrap()
            .lines()
            .skip(1)
            .collect::<Vec<_>>(),
        "a re-import keeps every address and id"
    );
    assert!(
        kept.starts_with("[settings]\nframe_range = \"None\"\nscale = 0.5"),
        "{kept}"
    );
    let (imported, used) = imported.split_once("\n\n").unwrap();
    assert!(used.starts_with("[settings]"), "{used}");
    let mut lines = imported.lines();
    let summary = lines.next().unwrap();
    assert!(
        summary.starts_with("Imported assets/props/square.obj: ")
            && summary.ends_with(", 1 sibling file."),
        "{imported}"
    );
    assert!(root.join("assets/props/square.obj").is_file());
    assert!(root.join("assets/props/square.mtl").is_file());
    let assets: Vec<&str> = lines.collect();
    assert!(!assets.is_empty(), "{imported}");
    for asset in assets {
        let location = asset.split(" · ").next().unwrap();
        let file = location.split('#').next().unwrap();
        assert!(root.join(file).is_file(), "{asset}");
        assert!(listed.contains(asset), "{asset} is not in {listed}");
    }
    let codes: Vec<&str> = errors
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "unknown_source",
            "import_refused",
            "invalid_arguments",
            "invalid_arguments",
            "import_failed",
            "not_imported",
            "source_missing"
        ]
    );
    let message = |index: usize| errors[index]["message"].as_str().unwrap().to_owned();
    assert!(
        message(2).contains("has no asset_settings; leave out asset_settings"),
        "{}",
        message(2)
    );
    assert!(
        message(3).contains("speed") && message(3).contains("the fields are scale"),
        "{}",
        message(3)
    );
    assert!(
        message(3).contains("{\"frame_range\":\"None\",\"scale\":0.5}"),
        "{}",
        message(3)
    );
}

struct Opens {
    first: String,
    state: String,
    status: String,
    again: String,
    yard: String,
    tree: String,
    created: String,
    hall: String,
    errors: Vec<Value>,
}

#[test]
fn open_asset_waits_for_a_scene_and_create_asset_writes_one_it_opens() {
    let fixture = Fixture::new();
    let root = fixture.root();
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
        let created = ok_with(
            &client,
            "create_asset",
            json!({ "kind": "scene", "name": "Hall", "folder": "content/rooms" }),
        )
        .await;
        let hall = ok_with(
            &client,
            "open_asset",
            json!({ "asset": "content/rooms/Hall.gasset" }),
        )
        .await;
        let errors = vec![
            error_with(
                &client,
                "create_asset",
                json!({ "kind": "Mesh", "name": "Rock" }),
            )
            .await,
            error_with(
                &client,
                "create_asset",
                json!({ "kind": "Scene", "name": "Hall", "folder": "content/rooms" }),
            )
            .await,
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
        Opens {
            first,
            state,
            status,
            again,
            yard,
            tree,
            created,
            hall,
            errors,
        }
    }) else {
        return;
    };

    let Opens {
        first,
        state,
        status,
        again,
        yard,
        tree,
        created,
        hall,
        errors,
    } = served.result;
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
    assert!(
        created.starts_with("Created content/rooms/Hall.gasset · Scene · "),
        "{created}"
    );
    assert!(root.join("content/rooms/Hall.gasset").is_file());
    assert!(
        hall.starts_with("Opened content/rooms/Hall.gasset in document "),
        "{hall}"
    );
    let codes: Vec<&str> = errors
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "unknown_kind",
            "create_failed",
            "unknown_asset",
            "open_failed",
            "open_failed"
        ]
    );
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

struct Poses {
    closed: Vec<Value>,
    sword: String,
    before: String,
    framed: String,
    on_sword: [String; 2],
    status: String,
    all: String,
    on_all: [String; 2],
    turned: String,
    at: String,
    facing: String,
    invalid: Vec<Value>,
    unmoved: String,
}

#[test]
fn frame_and_set_camera_answer_the_pose_once_the_camera_shows_it() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        close_the_project_scene(&client).await;
        let angles = json!({ "position": [1.0, 2.0, 3.0], "yaw": 0.5, "pitch": 0.25 });
        let closed = vec![
            error(&client, "frame").await,
            error_with(&client, "set_camera", angles.clone()).await,
        ];
        open(&client).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        let sword = scene_id(&ids, "Sword");
        let before = ok(&client, "camera_at").await;
        let framed = ok_with(&client, "frame", json!({ "entity": sword })).await;
        let on_sword = [
            ok(&client, "camera_at").await,
            ok(&client, "drawn_camera_at").await,
        ];
        let status = ok(&client, "status").await;
        let all = ok(&client, "frame").await;
        let on_all = [
            ok(&client, "camera_at").await,
            ok(&client, "drawn_camera_at").await,
        ];
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
            json!({ "position": [1e39, 0.0, 0.0], "yaw": 0.0, "pitch": 0.0 }),
        ] {
            invalid.push(error_with(&client, "set_camera", arguments).await);
        }
        let unmoved = ok(&client, "camera_at").await;
        client.cancel().await.unwrap();
        Poses {
            closed,
            sword,
            before,
            framed,
            on_sword,
            status,
            all,
            on_all,
            turned,
            at,
            facing,
            invalid,
            unmoved,
        }
    }) else {
        return;
    };

    let Poses {
        closed,
        sword,
        before,
        framed,
        on_sword,
        status,
        all,
        on_all,
        turned,
        at,
        facing,
        invalid,
        unmoved,
    } = served.result;
    for error in &closed {
        assert_eq!(error["code"], "no_scene", "{error}");
    }
    assert_eq!(before, "0.00");
    let level = "yaw 0.000, pitch -0.150, looking along (0.000, -0.149, -0.989).";
    assert!(
        framed.starts_with(&format!("Framed {sword} Sword. Camera at (9.500, ")),
        "{framed}"
    );
    assert!(framed.ends_with(level), "{framed}");
    assert_eq!(on_sword, ["9.50", "9.50"]);
    assert_eq!(line(&status, "selection: "), format!("{sword} Sword"));
    assert!(
        all.starts_with("Framed the whole scene. Camera at (0.000, "),
        "{all}"
    );
    assert!(all.ends_with(level), "{all}");
    assert_eq!(on_all, ["0.00", "0.00"]);
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
    status: String,
    viewport: CallToolResult,
    small_viewport: CallToolResult,
}

#[test]
fn screenshot_answers_the_viewport_and_the_whole_editor_as_png() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, |client| async move {
        close_the_project_scene(&client).await;
        let editor = call_with(&client, "screenshot", json!({ "target": "editor" })).await;
        let small_editor = call_with(
            &client,
            "screenshot",
            json!({ "target": "editor", "max_edge": 200 }),
        )
        .await;
        let closed = error_with(&client, "screenshot", json!({ "target": "viewport" })).await;
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

fn stale(id: &str) -> String {
    let (index, generation) = id.split_once('v').unwrap();
    format!("{index}v{}", generation.parse::<u32>().unwrap() + 1)
}

fn linked_row(tree: &str, name: &str) -> bool {
    tree.lines()
        .any(|line| line.split_whitespace().nth(1) == Some(name) && line.ends_with(" (linked)"))
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
        close_the_project_scene(&client).await;
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

struct ComponentEdits {
    hero: String,
    added: Value,
    carried: String,
    removed: String,
    errors: Vec<Value>,
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
        ComponentEdits {
            hero,
            added,
            carried,
            removed,
            errors,
        }
    }) else {
        return;
    };

    let ComponentEdits {
        hero,
        added,
        carried,
        removed,
        errors,
    } = served.result;
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
    let ambiguous = errors[3]["message"].as_str().unwrap();
    assert!(ambiguous.contains("mcp::Samples"), "{ambiguous}");
    assert!(ambiguous.contains("mcp::other::Samples"), "{ambiguous}");
}

struct PropertySets {
    hero: String,
    chime: String,
    intensity: Value,
    color: Value,
    rotation: Value,
    values: Value,
    walk: Value,
    run: Value,
    spot: Value,
    directional: Value,
    voiced: String,
    merged: String,
    after: Value,
    errors: Vec<Value>,
}
#[test]
fn set_property_writes_through_the_adapter_or_set_value_and_answers_the_component() {
    let fixture = Fixture::new();
    let chime = fixture.add("content/chime.gasset", "Chime").simple_hex();
    let log = Log::default();
    let Some(served) = serve_with(
        &fixture,
        &log,
        voiced_game,
        |app| {
            app.register_property_editor::<Voice, VoiceEditor>()
                .add_tool::<VoiceHeld, _>(
                    McpTool::read_only("voice_held").needs(Needs::Scene),
                    voice_held,
                );
        },
        move |client| async move {
            open(&client).await;
            let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
            let (hero, bulb, sword, rig, root) = (
                scene_id(&ids, "Hero"),
                scene_id(&ids, "Bulb"),
                scene_id(&ids, "Sword"),
                scene_id(&ids, "Rig"),
                scene_id(&ids, "root"),
            );
            until(&client, "scene_tree", |tree| {
                components(tree, &sword).contains(&"Samples")
            })
            .await;
            let set = |entity: &str, component: &str, path: &str, value: Value| {
                let client = &client;
                let arguments = json!({ "entity": entity, "component": component, "path": path, "value": value });
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
            let walk: Value = serde_json::from_str(
                &ok_with(
                    &client,
                    "add_component",
                    json!({ "entity": rig, "component": "Gait" }),
                )
                .await,
            )
            .unwrap();
            let run = set(&rig, "Gait", "pace", json!({ "Run": { "lean": 0.5 } })).await;
            let spot = set(
                &bulb,
                "Light",
                "light_type",
                json!({ "Spot": { "cone_angle": 40.0 } }),
            )
            .await;
            let directional = set(&bulb, "Light", "light_type", json!("Directional")).await;
            ok_with(
                &client,
                "add_component",
                json!({ "entity": rig, "component": "Voiced" }),
            )
            .await;
            set(
                &rig,
                "Voiced",
                "voice",
                json!({ "note": chime, "target": hero, "level": 2 }),
            )
            .await;
            let voiced = ok(&client, "voice_held").await;
            set(&rig, "Voiced", "voice", json!({ "level": 3 })).await;
            let merged = ok(&client, "voice_held").await;
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
                refuse(json!({ "entity": sword, "component": "Samples", "path": "values", "value": "loud" })).await,
                refuse(json!({ "entity": rig, "component": "Gait", "path": "pace.stride", "value": { "length": 2.0 } })).await,
                refuse(json!({ "entity": rig, "component": "Gait", "path": "pace", "value": "Sprint" })).await,
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
                hero,
                chime,
                intensity,
                color,
                rotation,
                values,
                walk,
                run,
                spot,
                directional,
                voiced,
                merged,
                after,
                errors,
            }
        },
    ) else {
        return;
    };

    let PropertySets {
        hero,
        chime,
        intensity,
        color,
        rotation,
        values,
        walk,
        run,
        spot,
        directional,
        voiced,
        merged,
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
        spot["value"]["light_type"],
        json!({ "Spot": { "cone_angle": 40.0 } })
    );
    assert_eq!(directional["value"]["light_type"], "Directional");
    assert_eq!(voiced, format!("strong {chime} {hero} 2"));
    assert_eq!(merged, format!("strong {chime} {hero} 3"));
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
            "refused",
            "invalid_value",
            "unknown_path",
            "invalid_value"
        ]
    );
}

const TRIANGLE: &str = r#"{
  "asset": { "version": "2.0" },
  "scene": 0,
  "scenes": [{ "nodes": [0] }],
  "nodes": [{ "name": "Triangle", "mesh": 0 }],
  "meshes": [{
    "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1, "material": 0 }]
  }],
  "materials": [{
    "name": "Red",
    "pbrMetallicRoughness": { "baseColorFactor": [1.0, 0.0, 0.0, 1.0] }
  }],
  "accessors": [
    { "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "max": [1.0, 1.0, 0.0], "min": [0.0, 0.0, 0.0] },
    { "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }
  ],
  "bufferViews": [
    { "buffer": 0, "byteOffset": 0, "byteLength": 36 },
    { "buffer": 0, "byteOffset": 36, "byteLength": 6 }
  ],
  "buffers": [{
    "byteLength": 42,
    "uri": "data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAIA/AAAAAAAAAAABAAIA"
  }]
}"#;

struct ImportedScene {
    parts: String,
    part: Value,
    placed: String,
    saved: String,
    refused: Value,
    guarded: Value,
    untouched: bool,
    reopened: String,
}

#[test]
fn an_imports_scene_is_read_only_and_is_placed_in_an_authored_scene() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let sources = tempfile::tempdir().unwrap();
    let source = sources.path().join("triangle.gltf");
    std::fs::write(&source, TRIANGLE).unwrap();
    let source = source.display().to_string();
    let log = Log::default();
    let Some(served) = serve(&fixture, &log, move |client| async move {
        let face = "content/props/triangle/scene.gasset";
        let hall = json!({ "asset": "content/Hall.gasset" });
        open(&client).await;
        ok_with(
            &client,
            "import_source",
            json!({ "source": source, "destination": "assets/props/triangle.gltf" }),
        )
        .await;
        let import = std::fs::read(root.join(face)).unwrap();
        let parts = ok_with(&client, "list_assets", json!({ "parts_of": face })).await;
        let mesh = parts
            .lines()
            .find(|line| line.contains(" · Mesh · "))
            .and_then(|line| line.split(" · ").next())
            .unwrap_or_else(|| panic!("no mesh in {parts}"))
            .to_owned();
        ok_with(
            &client,
            "create_asset",
            json!({ "kind": "Scene", "name": "Hall" }),
        )
        .await;
        ok_with(&client, "open_asset", hall.clone()).await;
        let part = error_with(&client, "place_asset", json!({ "asset": mesh })).await;
        let placed = ok_with(&client, "place_asset", json!({ "asset": face })).await;
        until(&client, "status", |status| status.contains("unsaved")).await;
        let saved = ok(&client, "save").await;
        ok_with(&client, "open_asset", json!({ "asset": face })).await;
        let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
        ok_with(
            &client,
            "set_property",
            json!({ "entity": scene_id(&ids, "Triangle"), "component": "Transform", "path": "translation", "value": [1.0, 2.0, 3.0] }),
        )
        .await;
        until(&client, "status", |status| status.contains("unsaved")).await;
        let refused = error(&client, "save").await;
        let mut guarded = hall.clone();
        guarded["unsaved"] = "save".into();
        let guarded = error_with(&client, "open_asset", guarded).await;
        let untouched = std::fs::read(root.join(face)).unwrap() == import;
        let mut discarding = hall;
        discarding["unsaved"] = "discard".into();
        ok_with(&client, "open_asset", discarding).await;
        let reopened = until(&client, "scene_tree", |tree| tree.contains("[link → ")).await;
        client.cancel().await.unwrap();
        ImportedScene {
            parts,
            part,
            placed,
            saved,
            refused,
            guarded,
            untouched,
            reopened,
        }
    }) else {
        return;
    };

    let ImportedScene {
        parts,
        part,
        placed,
        saved,
        refused,
        guarded,
        untouched,
        reopened,
    } = served.result;
    assert!(
        parts.contains("content/props/triangle/scene.gasset#mesh/0 · Mesh · "),
        "{parts}"
    );
    assert_eq!(part["code"], "refused", "{part}");
    assert!(placed.starts_with("placed triangle as entity "), "{placed}");
    assert_eq!(saved, "Saved content/Hall.gasset.");
    assert_eq!(refused["code"], "save_failed", "{refused}");
    assert_eq!(guarded["code"], "save_failed", "{guarded}");
    assert!(untouched);
    assert!(reopened.contains(" triangle "), "{reopened}");
}

const PROPS: &str = "content/props.gasset";
const NOTE: &str = "content/note.gasset";
const WORLD: &str = "content/world.gasset";
const LOOP: &str = "content/loop.gasset";

fn plain_node(name: &str) -> SceneNode {
    let mut node = SceneNode {
        id: NodeId::from_name_path(name),
        name: name.into(),
        children: vec![],
        components: vec![],
        link: None,
    };
    node.push_component(&Transform::IDENTITY).unwrap();
    node
}

fn moved_props() -> Scene {
    let mut scene = props("Pillar");
    scene.nodes.swap(1, 2);
    scene.nodes[1].components.clear();
    scene.nodes[1]
        .push_component(&Transform::from_translation(Vec3::new(3.0, 0.0, 0.0)))
        .unwrap();
    scene
}

fn props(anchor: &str) -> Scene {
    let mut lantern = plain_node("Lantern");
    lantern.push_component(&Light::point_light()).unwrap();
    Scene {
        nodes: vec![plain_node(anchor), lantern, plain_node("Helper")],
        referenced_assets: vec![],
    }
}

#[derive(Serialize, Deserialize)]
struct Note(String);

impl Asset for Note {
    fn name() -> &'static str {
        "Note"
    }
}

impl LoadableAsset for Note {}

#[derive(Resource)]
struct NoteLoad(AssetHandle<Note>);

fn load_the_note(server: Res<AssetServer>, mut cmd: CommandQueue) {
    cmd.insert_resource(NoteLoad(server.load::<Note>(AssetId::from_path(NOTE))));
}

#[derive(Resource, Default)]
struct SpawnedAt(Option<f32>);

fn note_spawned_placement(
    mut spawned: EventReader<SceneLinkSpawned>,
    linked: Query<(&LinkedNode, &GlobalTransform)>,
    mut at: ResMut<SpawnedAt>,
) {
    for event in spawned.read() {
        if at.0.is_none() {
            at.0 = linked
                .iter()
                .find(|(node, _)| node.link == event.link)
                .map(|(_, global)| global.translation().x);
        }
    }
}

fn noted_game(app: &mut App) {
    game(app);
    app.register_asset::<Note>()
        .insert_resource(SpawnedAt::default())
        .add_system(Startup, load_the_note.at_edit_time())
        .add_system(
            LateUpdate,
            note_spawned_placement
                .after(propagate_global_transforms)
                .at_edit_time(),
        );
}

#[derive(Deserialize, JsonSchema)]
struct SpawnedPlacement {}

fn spawned_placement(mut calls: ToolCalls<SpawnedPlacement>, at: EditWorld<Res<SpawnedAt>>) {
    for (_, reply) in calls.read() {
        reply.ok(at
            .0
            .map_or_else(|| "none".to_owned(), |x| format!("{x:.2}")));
    }
}

fn write_note(root: &Path, text: &str) {
    write_asset(
        root,
        NOTE,
        AssetId::from_path(NOTE),
        Note::name(),
        &bincode::serialize(&Note(text.into())).unwrap(),
    );
}

#[derive(Deserialize, JsonSchema)]
struct NoteText {}

fn note_text(
    mut calls: ToolCalls<NoteText>,
    load: EditWorld<Option<Res<NoteLoad>>>,
    notes: EditWorld<Option<Res<AssetStore<Note>>>>,
) {
    for (_, reply) in calls.read() {
        let note = load
            .as_deref()
            .zip(notes.as_deref())
            .and_then(|(load, notes)| notes.get(&load.0));
        reply.ok(note.map_or("", |note| note.0.as_str()));
    }
}

#[derive(Deserialize, JsonSchema)]
struct Reimported {}

fn reimported(
    mut calls: ToolCalls<Reimported>,
    registry: Res<AssetRegistry>,
    mut finished: EventWriter<ImportFinished>,
) {
    for (_, reply) in calls.read() {
        let assets = registry
            .iter()
            .filter(|asset| asset.address == PROPS || asset.address == NOTE)
            .cloned()
            .collect();
        finished.write(ImportFinished {
            destination: "assets/props.gltf".into(),
            settings: String::new(),
            result: Ok(Imported {
                assets,
                siblings: vec![],
                removed: vec![],
            }),
        });
        reply.ok("finished");
    }
}

struct SavedLevel {
    names: Vec<String>,
    link: SceneLinkData,
}

fn saved_level(file: &[u8]) -> SavedLevel {
    let (_, level) = decode_content_asset::<Scene>(file).unwrap();
    let names = level.nodes.iter().map(|node| node.name.clone()).collect();
    let mut links = level.nodes.into_iter().filter_map(|node| node.link);
    let link = links.next().expect("one link node");
    assert!(links.next().is_none(), "one link node");
    SavedLevel { names, link }
}

fn added_gain(link: &SceneLinkData) -> f32 {
    let mut samples = Samples::default();
    for change in &link.changes {
        if let LinkChange::AddComponent { component, .. } = change {
            concerto_reflect::apply_value(&mut samples, &component.value);
        }
    }
    samples.gain
}

struct Links {
    placed: String,
    spawned_at: String,
    top: String,
    expanded: String,
    own: String,
    copied: String,
    refused: Vec<Value>,
    saved: String,
    saved_file: Vec<u8>,
    removed: String,
    kept: Value,
    resaved_file: Vec<u8>,
    reloaded: Value,
    after_reload: String,
    note: String,
    itself: Value,
    nested_tree: String,
    nested: Value,
    inner_removal: Value,
    world_file: Vec<u8>,
    refused_tree: String,
    refused_link: Value,
    reaching: Value,
}

#[test]
fn links_are_placed_edited_saved_and_followed_over_mcp() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let props_id = AssetId::new();
    write_scene(&root, PROPS, props_id, &props("Anchor"));
    write_note(&root, "first");
    write_scene(
        &root,
        WORLD,
        AssetId::new(),
        &Scene {
            nodes: vec![plain_node("Ground")],
            referenced_assets: vec![],
        },
    );
    let loop_id = AssetId::new();
    let mut again = plain_node("again");
    again.link = Some(SceneLinkData {
        scene: loop_id,
        changes: vec![],
    });
    write_scene(
        &root,
        LOOP,
        loop_id,
        &Scene {
            nodes: vec![again],
            referenced_assets: vec![],
        },
    );
    let level_id = fixture.scene;
    let log = Log::default();
    let add_tools = |app: &mut App| {
        app.add_tool::<Reimported, _>(McpTool::new("reimported"), reimported)
            .add_tool::<NoteText, _>(
                McpTool::read_only("note_text").needs(Needs::Scene),
                note_text,
            )
            .add_tool::<SpawnedPlacement, _>(
                McpTool::read_only("spawned_placement").needs(Needs::Scene),
                spawned_placement,
            );
    };
    let Some(served) = serve_with(
        &fixture,
        &log,
        noted_game,
        add_tools,
        move |client| async move {
            open(&client).await;
            let placed = ok_with(
                &client,
                "place_asset",
                json!({ "asset": PROPS, "position": [5.0, 0.0, 0.0] }),
            )
            .await;
            let link = placed.rsplit(' ').next().unwrap().to_owned();
            let expanded = until_with(&client, "scene_tree", json!({ "root": link }), |tree| {
                linked_row(tree, "Helper")
            })
            .await;
            let at = |name: &str| tree_id(&expanded, name);
            let top = ok(&client, "scene_tree").await;
            let spawned_at = ok(&client, "spawned_placement").await;
            ok_with(
                &client,
                "add_component",
                json!({ "entity": at("Anchor"), "component": "editor::mcp::Samples" }),
            )
            .await;
            ok_with(
                &client,
                "set_property",
                json!({ "entity": at("Anchor"), "component": "editor::mcp::Samples", "path": "gain", "value": 2.5 }),
            )
            .await;
            ok_with(
                &client,
                "remove_component",
                json!({ "entity": at("Lantern"), "component": "Light" }),
            )
            .await;
            ok_with(&client, "delete_entity", json!({ "entity": at("Helper") })).await;
            let own = second_word(
                &ok_with(&client, "create_entity", json!({ "parent": at("Anchor") })).await,
            );
            let duplicated = ok_with(&client, "duplicate_entity", json!({ "entity": own })).await;
            let copy = duplicated
                .rsplit(" as ")
                .next()
                .and_then(|copy| copy.split_whitespace().next())
                .unwrap()
                .to_owned();
            let copied = ok_with(
                &client,
                "inspect",
                json!({ "entity": copy, "components": ["LinkParent"] }),
            )
            .await;
            let refused = vec![
                error_with(
                    &client,
                    "set_property",
                    json!({ "entity": at("Anchor"), "component": "Transform", "path": "translation", "value": [1.0, 2.0, 3.0] }),
                )
                .await,
                error_with(
                    &client,
                    "rename_entity",
                    json!({ "entity": at("Anchor"), "name": "Pillar" }),
                )
                .await,
            ];
            let saved = ok(&client, "save").await;
            let saved_file = std::fs::read(root.join("content/level.gasset")).unwrap();
            let removed = ok_with(
                &client,
                "remove_link_change",
                json!({ "entity": link, "index": 1 }),
            )
            .await;
            let respawned = until_with(&client, "scene_tree", json!({ "root": link }), |tree| {
                tree.lines().any(|line| line.contains(" Lantern [Light"))
            })
            .await;
            let kept = ok_with(
                &client,
                "inspect",
                json!({ "entity": tree_id(&respawned, "Lantern"), "components": ["Light"] }),
            )
            .await;
            ok(&client, "save").await;
            let resaved_file = std::fs::read(root.join("content/level.gasset")).unwrap();
            write_scene(&root, PROPS, props_id, &moved_props());
            write_note(&root, "second");
            ok(&client, "reimported").await;
            let reloaded = until_with(
                &client,
                "inspect",
                json!({ "entity": link }),
                |inspection| inspection.contains("unapplied"),
            )
            .await;
            let after_reload = ok_with(&client, "scene_tree", json!({ "root": link })).await;
            let note = until(&client, "note_text", |text| text == "second").await;
            let itself = error_with(
                &client,
                "place_asset",
                json!({ "asset": "content/level.gasset" }),
            )
            .await;
            ok_with(
                &client,
                "open_asset",
                json!({ "asset": WORLD, "unsaved": "discard" }),
            )
            .await;
            until(&client, "scene_tree", |tree| tree.contains(" Ground ")).await;
            let placed_level = ok_with(
                &client,
                "place_asset",
                json!({ "asset": "content/level.gasset" }),
            )
            .await;
            let level_link = placed_level.rsplit(' ').next().unwrap().to_owned();
            let level_tree = until_with(
                &client,
                "scene_tree",
                json!({ "root": level_link }),
                |tree| tree.contains(" props [link → "),
            )
            .await;
            let inner_link = tree_id(&level_tree, "props");
            let nested_tree = until_with(
                &client,
                "scene_tree",
                json!({ "root": inner_link }),
                |tree| linked_row(tree, "Lantern"),
            )
            .await;
            ok_with(
                &client,
                "add_component",
                json!({ "entity": tree_id(&nested_tree, "Lantern"), "component": "editor::mcp::Samples" }),
            )
            .await;
            let nested = ok_with(&client, "inspect", json!({ "entity": level_link })).await;
            let inner_removal = error_with(
                &client,
                "remove_link_change",
                json!({ "entity": inner_link, "index": 0 }),
            )
            .await;
            ok(&client, "save").await;
            let world_file = std::fs::read(root.join(WORLD)).unwrap();
            let placed_loop = ok_with(&client, "place_asset", json!({ "asset": LOOP })).await;
            let loop_link = placed_loop.rsplit(' ').next().unwrap().to_owned();
            let refused_tree = until_with(
                &client,
                "scene_tree",
                json!({ "root": loop_link }),
                |tree| tree.contains(" — refused: "),
            )
            .await;
            let refused_link = ok_with(
                &client,
                "inspect",
                json!({ "entity": tree_id(&refused_tree, "again") }),
            )
            .await;
            ok_with(
                &client,
                "open_asset",
                json!({ "asset": "content/level.gasset", "unsaved": "discard" }),
            )
            .await;
            until(&client, "scene_tree", |tree| tree.contains("[link → ")).await;
            let reaching = error_with(&client, "place_asset", json!({ "asset": WORLD })).await;
            client.cancel().await.unwrap();
            Links {
                placed,
                spawned_at,
                top,
                expanded,
                own,
                copied,
                refused,
                saved,
                saved_file,
                removed,
                kept: serde_json::from_str(&kept).unwrap(),
                resaved_file,
                reloaded: serde_json::from_str(&reloaded).unwrap(),
                after_reload,
                note,
                itself,
                nested_tree,
                nested: serde_json::from_str(&nested).unwrap(),
                inner_removal,
                world_file,
                refused_tree,
                refused_link: serde_json::from_str(&refused_link).unwrap(),
                reaching,
            }
        },
    ) else {
        return;
    };

    let Links {
        placed,
        spawned_at,
        top,
        expanded,
        own,
        copied,
        refused,
        saved,
        saved_file,
        removed,
        kept,
        resaved_file,
        reloaded,
        after_reload,
        note,
        itself,
        nested_tree,
        nested,
        inner_removal,
        world_file,
        refused_tree,
        refused_link,
        reaching,
    } = served.result;
    assert!(placed.starts_with("placed props as entity "), "{placed}");
    assert_eq!(spawned_at, "5.00");
    let link_rows: Vec<&str> = top
        .lines()
        .filter(|line| line.contains("[link → "))
        .collect();
    assert_eq!(link_rows.len(), 1, "{top}");
    assert!(
        link_rows[0].ends_with(" props [link → content/props.gasset] (0 changes)"),
        "{top}"
    );
    assert!(!top.contains("(linked)"), "{top}");
    for name in ["Anchor", "Lantern", "Helper"] {
        assert!(linked_row(&expanded, name), "{expanded}");
    }
    assert!(
        copied.contains(&NodeId::from_name_path("Anchor").0.to_string()),
        "{copied}"
    );
    let linked = "Anchor is a linked node of content/props.gasset; add a component to it, or edit content/props.gasset";
    for error in &refused {
        assert_eq!(error["code"], "refused", "{error}");
        assert!(
            error["message"].as_str().unwrap().contains(linked),
            "{error}"
        );
    }
    assert_eq!(saved, "Saved content/level.gasset.");
    let SavedLevel {
        names,
        link: saved_link,
    } = saved_level(&saved_file);
    assert_eq!(saved_link.scene, props_id);
    assert_eq!(saved_link.changes.len(), 3, "{:?}", saved_link.changes);
    assert_eq!(added_gain(&saved_link), 2.5);
    assert_eq!(names.iter().filter(|name| *name == "Entity").count(), 2);
    assert!(
        names
            .iter()
            .all(|name| !["Anchor", "Lantern", "Helper"].contains(&name.as_str()))
    );
    let (_, level) = decode_content_asset::<Scene>(&saved_file).unwrap();
    assert!(level.nodes.iter().any(|node| {
        node.name == "Entity"
            && node
                .components
                .iter()
                .any(|component| component.type_name.ends_with("LinkParent"))
    }));
    assert_eq!(removed, "Removed change 1 from props.");
    assert_eq!(kept["components"][0]["name"], "Light", "{kept}");
    let resaved_link = saved_level(&resaved_file).link;
    assert_eq!(resaved_link.changes.len(), 2, "{:?}", resaved_link.changes);
    assert_eq!(added_gain(&resaved_link), 2.5);
    let changes = reloaded["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 2, "{reloaded}");
    assert!(
        changes[0].as_str().unwrap().starts_with(&format!(
            "0: AddComponent {} [Samples] — unapplied: ",
            NodeId::from_name_path("Anchor")
        )),
        "{reloaded}"
    );
    assert_eq!(changes[1], "1: RemoveNode Helper — applied", "{reloaded}");
    assert!(
        after_reload.contains(&format!("{own} Entity ")),
        "{after_reload}"
    );
    assert!(!linked_row(&after_reload, "Helper"), "{after_reload}");
    assert_eq!(note, "second");
    assert_eq!(itself["code"], "refused", "{itself}");
    assert!(
        itself["message"]
            .as_str()
            .unwrap()
            .contains("a scene cannot link itself"),
        "{itself}"
    );
    assert!(linked_row(&nested_tree, "Lantern"), "{nested_tree}");
    assert_eq!(nested["state"], "spawned", "{nested}");
    assert_eq!(
        nested["changes"][0], "0: AddComponent Lantern [Samples] — applied",
        "{nested}"
    );
    assert_eq!(inner_removal["code"], "refused", "{inner_removal}");
    assert!(
        inner_removal["message"]
            .as_str()
            .unwrap()
            .contains("props is a linked node of content/level.gasset"),
        "{inner_removal}"
    );
    let props_node = level
        .nodes
        .iter()
        .find(|node| node.link.is_some())
        .unwrap()
        .id;
    let world_link = saved_level(&world_file).link;
    assert_eq!(world_link.scene, level_id);
    assert!(
        matches!(&world_link.changes[..], [LinkChange::AddComponent { path, .. }]
            if path.ids() == [props_node, NodeId::from_name_path("Lantern")]),
        "{:?}",
        world_link.changes
    );
    let refused_row = refused_tree
        .lines()
        .find(|line| line.contains(" again [link → content/loop.gasset]"))
        .unwrap_or_else(|| panic!("{refused_tree}"));
    assert!(
        refused_row.contains(" — refused: content/loop.gasset links itself through "),
        "{refused_tree}"
    );
    assert!(
        refused_link["state"]
            .as_str()
            .unwrap()
            .starts_with("refused: content/loop.gasset links itself"),
        "{refused_link}"
    );
    assert_eq!(reaching["code"], "refused", "{reaching}");
    assert!(
        reaching["message"]
            .as_str()
            .unwrap()
            .contains("a scene cannot link itself"),
        "{reaching}"
    );
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
        close_the_project_scene(&client).await;
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
    posed: Value,
    framed: Value,
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
        let posed = error_with(
            &client,
            "set_camera",
            json!({ "position": [0.0, 0.0, 0.0], "yaw": 0.0, "pitch": 0.0 }),
        )
        .await;
        let framed = error(&client, "frame").await;
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
            posed,
            framed,
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
        posed,
        framed,
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
    assert_eq!(save_while["code"], "save_failed", "{save_while}");
    assert_eq!(posed["code"], "refused", "{posed}");
    assert_eq!(framed["code"], "refused", "{framed}");
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

fn gated_fixture() -> Fixture {
    let fixture = Fixture::new();
    write_asset(
        &fixture.root(),
        GATED_ADDRESS,
        AssetId::from_path(GATED_ADDRESS),
        GatedAsset::name(),
        &bincode::serialize(&GatedAsset).unwrap(),
    );
    fixture
}

struct Played {
    document: String,
    played: String,
    answered: Instant,
    seen: Option<Instant>,
}

struct Gate {
    status: String,
    opened: Instant,
}

struct GatedPlay {
    played: Played,
    gate: Gate,
}

#[test]
fn play_answers_once_the_game_has_spawned_its_scene_and_loaded_what_it_loads_at_start() {
    let fixture = gated_fixture();
    let Some(served) = serve_with(
        &fixture,
        &Log::default(),
        gated_game,
        |app| {
            app.insert_resource(WarmUpLimit(Duration::from_secs(1)));
        },
        |client| async move {
            *HERO_SEEN.lock().unwrap_or_else(PoisonError::into_inner) = None;
            open(&client).await;
            let document = document_id(&ok(&client, "status").await);
            let play = async {
                let played = ok(&client, "play").await;
                let answered = Instant::now();
                let seen = *HERO_SEEN.lock().unwrap_or_else(PoisonError::into_inner);
                Played {
                    document,
                    played,
                    answered,
                    seen,
                }
            };
            let open_the_gate = async {
                let status = until(&client, "status", |status| status.contains(" · playing")).await;
                tokio::time::sleep(Duration::from_millis(500)).await;
                OPEN_THE_GATE.store(true, Ordering::SeqCst);
                Gate {
                    status,
                    opened: Instant::now(),
                }
            };
            let (played, gate) = tokio::join!(play, open_the_gate);
            client.cancel().await.unwrap();
            GatedPlay { played, gate }
        },
    ) else {
        OPEN_THE_GATE.store(true, Ordering::SeqCst);
        return;
    };

    let GatedPlay {
        played:
            Played {
                document,
                played,
                answered,
                seen,
            },
        gate: Gate { status, opened },
    } = served.result;
    assert!(status.contains(" · playing"), "{status}");
    assert!(answered >= opened, "play answered before the gate opened");
    let seen = seen.expect("the game spawned its scene before play answered");
    assert!(seen <= answered);
    let loaded = format!("Playing content/level.gasset in document {document}; it loaded in ");
    assert!(played.starts_with(&loaded), "{played}");
    assert!(played.ends_with(" s."), "{played}");
}

#[derive(Debug, PartialEq)]
struct ViewportSize {
    width: f32,
    height: f32,
}

fn viewport_size(status: &str) -> ViewportSize {
    let (width, height) = line(status, "viewport: ").split_once('x').unwrap();
    ViewportSize {
        width: width.parse().unwrap(),
        height: height.parse().unwrap(),
    }
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

struct Inputs {
    idle: Value,
    document: String,
    typed: String,
    after_keys: Value,
    after_move: Value,
    centre: [f32; 2],
    after_click: Value,
    unknown: Value,
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
            let idle = error_with(
                &client,
                "send_input",
                json!({ "events": [{ "key": "KeyW", "pressed": true }] }),
            )
            .await;
            ok(&client, "play").await;
            let status = ok(&client, "status").await;
            let ViewportSize { width, height } = viewport_size(&status);
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
            let unknown = error_with(
                &client,
                "send_input",
                json!({ "events": [{ "key": "KeyW", "pressed": true }, { "key": "Kew", "pressed": true }] }),
            )
            .await;
            client.cancel().await.unwrap();
            Inputs {
                idle,
                document,
                typed,
                after_keys,
                after_move,
                centre,
                after_click,
                unknown,
            }
        },
    ) else {
        return;
    };

    let Inputs {
        idle,
        document,
        typed,
        after_keys,
        after_move,
        centre,
        after_click,
        unknown,
    } = served.result;
    assert_eq!(idle["code"], "not_playing", "{idle}");
    assert_eq!(
        typed,
        format!("Sent 3 window events to the game in document {document}; held in the game: KeyW.")
    );
    assert_eq!(after_keys["w_presses"], 2, "{after_keys}");
    assert!(near(&after_move["pointer"], [10.0, 20.0]), "{after_move}");
    assert_eq!(after_click["clicks"], 1, "{after_click}");
    assert!(near(&after_click["pointer"], centre), "{after_click}");
    assert_eq!(unknown["code"], "invalid_arguments", "{unknown}");
}

struct Owners {
    unplugged: Value,
    pressed: String,
    under_editor: String,
    released: Value,
    back: String,
    after: Value,
    unplug: String,
}

#[test]
fn send_input_hands_input_between_editor_and_game_and_its_pads_reach_the_game_under_either() {
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
            let ViewportSize { width, height } = viewport_size(&status);
            let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
            let counter = scene_id(&ids, "Counter");
            let send = |events: Value| {
                let client = &client;
                async move { ok_with(client, "send_input", json!({ "events": events })).await }
            };
            let unplugged = error_with(
                &client,
                "send_input",
                json!({ "events": [{ "pad": 0, "button": "south", "value": 1.0 }] }),
            )
            .await;
            let pressed = send(json!([
                { "pad": 0, "connect": true, "name": "Test pad" },
                { "pad": 0, "button": "south", "value": 1.0 },
                { "pad": 0, "axis": "left_stick_x", "value": 0.5 },
            ]))
            .await;
            send(json!([
                { "key": "ShiftLeft", "pressed": true },
                { "key": "F1", "pressed": true },
            ]))
            .await;
            let under_editor = send(json!([
                { "pad": 0, "button": "south", "value": 0.0 },
                { "pad": 0, "button": "south", "value": 1.0 },
            ]))
            .await;
            let released = error_with(
                &client,
                "send_input",
                json!({ "events": [{ "key": "KeyW", "pressed": true }] }),
            )
            .await;
            let centre = [(width / 2.0).floor(), (height / 2.0).floor()];
            let back = send(json!([
                { "button": "left", "pressed": true, "at": centre },
                { "button": "left", "pressed": false },
                { "key": "KeyW", "pressed": true },
            ]))
            .await;
            let after = received(&ok_with(&client, "inspect", json!({ "entity": counter })).await);
            let unplug = send(json!([
                { "pad": 0, "button": "south", "value": 0.0 },
                { "pad": 0, "connect": false },
            ]))
            .await;
            client.cancel().await.unwrap();
            Owners {
                unplugged,
                pressed,
                under_editor,
                released,
                back,
                after,
                unplug,
            }
        },
    ) else {
        return;
    };

    let Owners {
        unplugged,
        pressed,
        under_editor,
        released,
        back,
        after,
        unplug,
    } = served.result;
    assert_eq!(unplugged["code"], "invalid_arguments", "{unplugged}");
    assert!(
        pressed
            .contains("pads connected: 0 (Test pad, from send_input); pad buttons held: 0 south."),
        "{pressed}"
    );
    assert!(
        under_editor.ends_with("The editor has input now."),
        "{under_editor}"
    );
    assert_eq!(released["code"], "input_released", "{released}");
    assert!(
        back.contains("held in the game: KeyW; held in the editor: F1, ShiftLeft"),
        "{back}"
    );
    assert_eq!(after["pad_presses"], 2, "{after}");
    assert_eq!(after["w_presses"], 1, "{after}");
    assert_eq!(after["clicks"], 0, "{after}");
    assert!(!unplug.contains("pads connected"), "{unplug}");
}

#[derive(Reflect, Default)]
enum Mood {
    #[default]
    Calm,
    Tense {
        level: f32,
    },
}

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
struct Roster {
    strides: Vec<Stride>,
    moods: Vec<Mood>,
    locked: Vec<u32>,
    gain: f32,
}

#[derive(Default)]
struct LockedEditor;

impl PropertyEditor<Vec<u32>> for LockedEditor {
    type Snapshot = usize;
    type Edit = u32;

    fn snapshot(&self, value: &Vec<u32>) -> usize {
        value.len()
    }

    fn build(&self, _: &mut EntityCommandQueue, _: &usize, _: &UITheme) {}

    fn apply(&self, value: &mut Vec<u32>, edit: &u32, _: &EditContext) -> Result<(), EditError> {
        value.push(*edit);
        Ok(())
    }
}

fn roster_game(app: &mut App) {
    game(app);
    app.register_type::<Roster>();
}

struct ListEdits {
    pushed_stride: String,
    roster: Value,
    written: Value,
    pushed_mood: String,
    moods: Value,
    pushed_sample: String,
    removed_sample: String,
    past_the_cut: Value,
    samples: Value,
    refused: Vec<Value>,
}

struct Refusal {
    tool: &'static str,
    arguments: Value,
}

#[test]
fn list_elements_are_listed_written_pushed_and_removed() {
    let fixture = Fixture::new();
    let log = Log::default();
    let Some(served) = serve_with(
        &fixture,
        &log,
        roster_game,
        |app| {
            app.register_property_editor::<Vec<u32>, LockedEditor>();
        },
        |client| async move {
            open(&client).await;
            let ids: Value = serde_json::from_str(&ok(&client, "ids").await).unwrap();
            let (rig, sword, root) = (
                scene_id(&ids, "Rig"),
                scene_id(&ids, "Sword"),
                scene_id(&ids, "root"),
            );
            until(&client, "scene_tree", |tree| {
                components(tree, &sword).contains(&"Samples")
            })
            .await;
            ok_with(
                &client,
                "add_component",
                json!({ "entity": rig, "component": "Roster" }),
            )
            .await;
            let pushed_stride = ok_with(
                &client,
                "list_push",
                json!({ "entity": rig, "component": "roster", "path": "strides" }),
            )
            .await;
            let set = |entity: &str, component: &str, path: &str, value: Value| {
                let client = &client;
                let arguments = json!({ "entity": entity, "component": component, "path": path, "value": value });
                async move {
                    serde_json::from_str::<Value>(&ok_with(client, "set_property", arguments).await)
                        .unwrap()
                }
            };
            let inspect = |entity: &str, component: &str| {
                let client = &client;
                let arguments = json!({ "entity": entity, "components": [component] });
                async move {
                    serde_json::from_str::<Value>(&ok_with(client, "inspect", arguments).await)
                        .unwrap()
                }
            };
            let written = set(&rig, "Roster", "strides.0.length", json!(2.5)).await;
            let roster = inspect(&rig, "Roster").await;
            let pushed_mood = ok_with(
                &client,
                "list_push",
                json!({ "entity": rig, "component": "Roster", "path": "moods" }),
            )
            .await;
            let moods = inspect(&rig, "Roster").await;
            set(&sword, "Samples", "values.3", json!(9.5)).await;
            let past_the_cut = set(&sword, "Samples", "values.50", json!(-1.0)).await;
            let pushed_sample = ok_with(
                &client,
                "list_push",
                json!({ "entity": sword, "component": "Samples", "path": "values" }),
            )
            .await;
            let removed_sample = ok_with(
                &client,
                "list_remove",
                json!({ "entity": sword, "component": "Samples", "path": "values", "index": 0 }),
            )
            .await;
            let samples = inspect(&sword, "Samples").await;
            let mut refused = Vec::new();
            for refusal in [
                Refusal {
                    tool: "set_property",
                    arguments: json!({ "entity": rig, "component": "Roster", "path": "strides.0", "value": { "length": 1.0 } }),
                },
                Refusal {
                    tool: "set_property",
                    arguments: json!({ "entity": rig, "component": "Roster", "path": "moods.0", "value": "Calm" }),
                },
                Refusal {
                    tool: "set_property",
                    arguments: json!({ "entity": sword, "component": "Samples", "path": "values.100", "value": 1.0 }),
                },
                Refusal {
                    tool: "list_push",
                    arguments: json!({ "entity": root, "component": "Transform", "path": "translation" }),
                },
                Refusal {
                    tool: "list_remove",
                    arguments: json!({ "entity": root, "component": "Transform", "path": "translation", "index": 0 }),
                },
                Refusal {
                    tool: "list_push",
                    arguments: json!({ "entity": rig, "component": "Roster", "path": "gain" }),
                },
                Refusal {
                    tool: "list_push",
                    arguments: json!({ "entity": rig, "component": "Roster", "path": "locked" }),
                },
                Refusal {
                    tool: "list_remove",
                    arguments: json!({ "entity": rig, "component": "Roster", "path": "locked", "index": 0 }),
                },
                Refusal {
                    tool: "list_remove",
                    arguments: json!({ "entity": rig, "component": "Roster", "path": "strides", "index": 1 }),
                },
                Refusal {
                    tool: "list_push",
                    arguments: json!({ "entity": rig, "component": "Roster", "path": "crew" }),
                },
            ] {
                refused.push(error_with(&client, refusal.tool, refusal.arguments).await);
            }
            client.cancel().await.unwrap();
            ListEdits {
                pushed_stride,
                roster,
                written,
                pushed_mood,
                moods,
                pushed_sample,
                removed_sample,
                past_the_cut,
                samples,
                refused,
            }
        },
    ) else {
        return;
    };

    let edits = served.result;
    assert_eq!(
        edits.pushed_stride,
        "Added an element to Roster.strides; its length is 1."
    );
    assert_eq!(edits.written["value"]["strides"][0]["length"], 2.5);
    assert_eq!(
        inspected(&edits.roster, "Roster")["paths"],
        json!([
            { "path": "strides", "adapter": false },
            { "path": "strides.0.length", "adapter": true },
            { "path": "strides.0.height", "adapter": true },
            { "path": "moods", "adapter": false },
            { "path": "locked", "adapter": true },
            { "path": "gain", "adapter": true },
        ])
    );
    assert_eq!(
        edits.pushed_mood,
        "Added an element to Roster.moods; its length is 1."
    );
    assert_eq!(
        inspected(&edits.moods, "Roster")["paths"][4],
        json!({ "path": "moods.0", "adapter": false, "enum": true })
    );
    assert_eq!(edits.past_the_cut["name"], "Samples");
    assert_eq!(
        edits.pushed_sample,
        "Added an element to Samples.values; its length is 101."
    );
    assert_eq!(
        edits.removed_sample,
        "Removed element 0 of Samples.values; its length is 100."
    );
    let values = inspected(&edits.samples, "Samples")["value"]["values"]
        .as_array()
        .unwrap();
    assert_eq!(values[0], 1.0);
    assert_eq!(values[2], 9.5);

    let refused = &edits.refused;
    let codes: Vec<&str> = refused
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(
        codes,
        [
            "unknown_path",
            "not_editable",
            "unknown_path",
            "refused",
            "refused",
            "not_a_list",
            "not_a_list",
            "not_a_list",
            "index_out_of_range",
            "unknown_path",
        ]
    );
}

#[derive(Serialize, Deserialize)]
struct Chime;

impl Asset for Chime {
    fn name() -> &'static str {
        "Chime"
    }
}

impl LoadableAsset for Chime {}

#[derive(Reflect, Default)]
#[reflect(Default)]
struct Voice {
    note: AssetHandle<Chime>,
    target: SceneEntityRef,
    level: u32,
}

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
struct Voiced {
    voice: Voice,
}

#[derive(Default)]
struct VoiceEditor;

struct VoiceEdit {
    note: AssetHandle<Chime>,
    target: SceneEntityRef,
    level: u32,
}

impl PropertyEditor<Voice> for VoiceEditor {
    type Snapshot = u32;
    type Edit = VoiceEdit;

    fn snapshot(&self, value: &Voice) -> u32 {
        value.level
    }

    fn build(&self, _: &mut EntityCommandQueue, _: &u32, _: &UITheme) {}

    fn apply(
        &self,
        value: &mut Voice,
        edit: &Self::Edit,
        _: &EditContext,
    ) -> Result<(), EditError> {
        value.note = edit.note.clone();
        value.target = edit.target;
        value.level = edit.level;
        Ok(())
    }

    fn edit_to(&self, value: &Voice) -> Option<Self::Edit> {
        Some(VoiceEdit {
            note: value.note.clone(),
            target: value.target,
            level: value.level,
        })
    }
}

fn voiced_game(app: &mut App) {
    game(app);
    app.register_asset::<Chime>()
        .register_type::<AssetHandle<Chime>>()
        .register_type::<Voice>()
        .register_type::<Voiced>();
}

#[derive(Deserialize, JsonSchema)]
struct VoiceHeld {}

fn voice_held(mut calls: ToolCalls<VoiceHeld>, voiced: EditWorld<Query<&Voiced>>) {
    for (_, reply) in calls.read() {
        let held = voiced.iter().next().map(|voiced| {
            let voice = &voiced.voice;
            let strength = match voice.note {
                AssetHandle::Strong(..) => "strong",
                _ => "weak",
            };
            let target = match voice.target {
                SceneEntityRef::Entity(entity) => entity.to_id_string(),
                SceneEntityRef::Index(index) => format!("index {index}"),
            };
            format!(
                "{strength} {} {target} {}",
                voice.note.id().simple_hex(),
                voice.level
            )
        });
        reply.ok(held.unwrap_or_default());
    }
}

#[derive(Resource)]
struct KeepOpening(Option<PathBuf>);

#[derive(Deserialize, JsonSchema)]
struct HoldOpening {
    hold: bool,
}

fn hold_opening(
    mut calls: ToolCalls<HoldOpening>,
    state: Res<ProjectState>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        match (arguments.hold, &state.project) {
            (true, Some(project)) => cmd.insert_resource(KeepOpening(Some(project.root.clone()))),
            (true, None) => {
                reply.ok("no project");
                continue;
            }
            (false, _) => cmd.insert_resource(KeepOpening(None)),
        }
        reply.ok("held");
    }
}

fn keep_opening(keep: Option<Res<KeepOpening>>, mut commands: ResMut<EditorCommands>) {
    let Some(root) = keep.and_then(|keep| keep.0.clone()) else {
        return;
    };
    if !commands
        .0
        .iter()
        .any(|command| matches!(command, EditorCommand::OpenProject(_)))
    {
        commands.0.push_back(EditorCommand::OpenProject(root));
    }
}

struct ProjectOpens {
    busy: Vec<Value>,
    opened: String,
    status: String,
    failed: Value,
    after: String,
}

#[test]
fn open_project_answers_busy_while_one_opens_waits_for_the_project_and_reports_a_failure() {
    let fixture = Fixture::new();
    let root = fixture.root();
    let other = Fixture::new();
    other.add("content/meshes/rock.gasset", "Mesh");
    let other_root = other.root().canonicalize().unwrap();
    let missing = fixture.root().join("missing");
    let log = Log::default();
    let Some(served) = serve_with(
        &fixture,
        &log,
        game,
        |app| {
            app.add_tool::<HoldOpening, _>(McpTool::new("hold_opening"), hold_opening)
                .add_system(LateUpdate, keep_opening);
        },
        move |client| async move {
            until(&client, "state", |state| state.starts_with("project")).await;
            assert_eq!(
                ok_with(&client, "hold_opening", json!({ "hold": true })).await,
                "held"
            );
            let busy = vec![
                error_with(
                    &client,
                    "open_project",
                    json!({ "path": root.display().to_string() }),
                )
                .await,
                error_with(
                    &client,
                    "open_asset",
                    json!({ "asset": "content/level.gasset" }),
                )
                .await,
            ];
            ok_with(&client, "hold_opening", json!({ "hold": false })).await;
            until(&client, "status", |status| status.ends_with("busy: no")).await;
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
            ProjectOpens {
                busy,
                opened,
                status,
                failed,
                after,
            }
        },
    ) else {
        return;
    };

    let ProjectOpens {
        busy,
        opened,
        status,
        failed,
        after,
    } = served.result;
    for error in &busy {
        assert_eq!(error["code"], "busy", "{error}");
    }
    let root = other_root.display().to_string();
    assert_eq!(opened, format!("Opened {root}: 2 assets, 1 scene."));
    assert_eq!(line(&status, "project: "), root);
    assert_eq!(failed["code"], "open_failed", "{failed}");
    assert_eq!(line(&after, "project: "), root);
}

#[derive(Serialize, Deserialize)]
struct Tune;

impl Asset for Tune {
    fn name() -> &'static str {
        "Tune"
    }
}

#[derive(Reflect, Default)]
#[reflect(Default)]
enum Temper {
    #[default]
    Calm,
    Loud,
}

#[derive(Reflect, Default)]
#[reflect(Default)]
struct Tuning {
    gain: f32,
    temper: Temper,
    samples: Vec<f32>,
}

#[derive(Reflect, Default)]
struct Loose {
    gain: f32,
    samples: Vec<f32>,
}

#[derive(Component, Default)]
struct HeldTune {
    tuning: Tuning,
    loose: Loose,
}

fn tune_values(document: Entity) -> Vec<Arc<dyn PropertyTarget>> {
    vec![
        Arc::new(DocumentValue::<HeldTune> {
            name: "Tuning".into(),
            document,
            read: Box::new(|held| Some(&held.tuning)),
            edit: Box::new(|held| Some(&mut held.tuning)),
        }),
        Arc::new(DocumentValue::<HeldTune> {
            name: "Loose".into(),
            document,
            read: Box::new(|held| Some(&held.loose)),
            edit: Box::new(|held| Some(&mut held.loose)),
        }),
        Arc::new(DocumentValue::<HeldTune> {
            name: "Absent".into(),
            document,
            read: Box::new(|_| None),
            edit: Box::new(|_| None),
        }),
        Arc::new(DocumentValue::<HeldTune> {
            name: "Twin".into(),
            document,
            read: Box::new(|held| Some(&held.tuning)),
            edit: Box::new(|held| Some(&mut held.tuning)),
        }),
        Arc::new(DocumentValue::<HeldTune> {
            name: "Twin".into(),
            document,
            read: Box::new(|held| Some(&held.loose)),
            edit: Box::new(|held| Some(&mut held.loose)),
        }),
    ]
}

#[derive(Default)]
struct TuneEditor;

impl AssetEditor for TuneEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        let targets = tune_values(editor.entity());
        editor.insert((HeldTune::default(), InspectedValues(targets)));
    }
}

struct TunesSeenLoading {
    assets: Vec<AssetId>,
}

impl FromWorld for TunesSeenLoading {
    fn from_world(_world: &World) -> Self {
        Self { assets: Vec::new() }
    }
}

fn load_tunes(
    documents: Query<&mut EditorDocument, With<HeldTune>>,
    mut seen: SystemLocal<TunesSeenLoading>,
) {
    let mut loading = Vec::new();
    for mut document in documents.iter() {
        let Some(asset) = document.pending.clone() else {
            continue;
        };
        if !seen.assets.contains(&asset.id) {
            loading.push(asset.id);
            continue;
        }
        let result = if asset.address.contains("broken") {
            Err(anyhow::anyhow!("{} is out of tune", asset.address))
        } else {
            Ok(())
        };
        finish_asset_request(&mut document, &asset, result);
    }
    seen.assets = loading;
}

fn note_value_edits(mut committed: EventReader<Hosted<PropertyCommitted>>, log: Res<Log>) {
    for Hosted { document, event } in committed.read() {
        log.push(format!(
            "{} {} {}",
            document.to_id_string(),
            event.address.path,
            event.result.is_ok()
        ));
    }
}

fn tune_editor(app: &mut App) {
    app.register_asset_editor::<Tune, TuneEditor>().unwrap();
    app.register_type::<Tuning>()
        .add_system(Update, load_tunes)
        .add_system(LateUpdate, note_value_edits);
}

async fn open_tune(client: &Client) -> String {
    until(client, "state", |state| state.starts_with("project")).await;
    let opened = ok_with(
        client,
        "open_asset",
        json!({ "asset": "content/tune.gasset" }),
    )
    .await;
    opened
        .trim_end_matches('.')
        .rsplit(' ')
        .next()
        .unwrap()
        .to_owned()
}

async fn inspected_target(client: &Client, name: &str) -> Value {
    serde_json::from_str(&ok_with(client, "inspect", json!({ "target": name })).await).unwrap()
}

async fn set_on_target(client: &Client, target: &str, path: &str, value: Value) -> Value {
    serde_json::from_str(
        &ok_with(
            client,
            "set_property",
            json!({ "target": target, "path": path, "value": value }),
        )
        .await,
    )
    .unwrap()
}

struct ValueCalls {
    document: String,
    status: String,
    shown: Value,
    through_an_editor: Value,
    an_enum: Value,
    a_list: Value,
    pushed: String,
    removed: String,
    after: Value,
    gone: Value,
    invalid: Vec<Value>,
    errors: Vec<Value>,
}

#[test]
fn a_documents_values_are_read_and_written_by_name_with_no_scene_open() {
    let fixture = Fixture::without_a_scene();
    fixture.add("content/tune.gasset", "Tune");
    let log = Log::default();
    let Some(served) = serve_with(&fixture, &log, game, tune_editor, |client| async move {
        let document = open_tune(&client).await;
        let status = until(&client, "status", |status| status.ends_with("busy: no")).await;
        let shown = inspected_target(&client, "Tuning").await;
        let through_an_editor = set_on_target(&client, "Tuning", "gain", json!(0.75)).await;
        let an_enum = set_on_target(&client, "Tuning", "temper", json!("Loud")).await;
        let a_list = set_on_target(&client, "Tuning", "samples", json!([1.0, 2.0])).await;
        let pushed = ok_with(
            &client,
            "list_push",
            json!({ "target": "Tuning", "path": "samples" }),
        )
        .await;
        let removed = ok_with(
            &client,
            "list_remove",
            json!({ "target": "Tuning", "path": "samples", "index": 0 }),
        )
        .await;
        let after = inspected_target(&client, "Tuning").await;
        let gone = inspected_target(&client, "Absent").await;
        let invalid = vec![
            error_with(
                &client,
                "inspect",
                json!({ "target": "Tuning", "entity": "1v1" }),
            )
            .await,
            error_with(
                &client,
                "set_property",
                json!({ "path": "gain", "value": 1.0 }),
            )
            .await,
            error_with(
                &client,
                "list_push",
                json!({ "entity": "1v1", "path": "samples" }),
            )
            .await,
        ];
        let errors = vec![
            error_with(&client, "inspect", json!({ "target": "Pitch" })).await,
            error_with(
                &client,
                "set_property",
                json!({ "target": "Twin", "path": "gain", "value": 1.0 }),
            )
            .await,
            error_with(
                &client,
                "set_property",
                json!({ "target": "Loose", "path": "samples", "value": [1.0] }),
            )
            .await,
        ];
        client.cancel().await.unwrap();
        ValueCalls {
            document,
            status,
            shown,
            through_an_editor,
            an_enum,
            a_list,
            pushed,
            removed,
            after,
            gone,
            invalid,
            errors,
        }
    }) else {
        return;
    };

    let answers = served.result;
    assert_eq!(
        line(&answers.status, "values: "),
        "Tuning, Loose, Absent, Twin, Twin"
    );
    assert_eq!(line(&answers.status, "active: "), answers.document);
    let shown = &answers.shown;
    assert_eq!(shown["name"], "Tuning");
    assert_eq!(shown["type"], "editor::mcp::Tuning");
    assert_eq!(
        shown["value"],
        json!({ "gain": 0.0, "temper": "Calm", "samples": [] })
    );
    assert!(shown.get("entity").is_none(), "{shown}");
    let adapters: Vec<String> = shown["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| format!("{} {}", path["path"].as_str().unwrap(), path["adapter"]))
        .collect();
    assert_eq!(adapters, ["gain true", "temper false", "samples false"]);
    assert_eq!(answers.through_an_editor["value"]["gain"], 0.75);
    assert_eq!(answers.an_enum["value"]["temper"], "Loud");
    assert_eq!(answers.a_list["value"]["samples"], json!([1.0, 2.0]));
    assert_eq!(
        answers.pushed,
        "Added an element to Tuning.samples; its length is 3."
    );
    assert_eq!(
        answers.removed,
        "Removed element 0 of Tuning.samples; its length is 2."
    );
    assert_eq!(
        answers.after["value"],
        json!({ "gain": 0.75, "temper": "Loud", "samples": [2.0, 0.0] })
    );
    assert_eq!(answers.gone["value"], Value::Null);
    assert_eq!(answers.gone["unavailable"], true);
    for error in &answers.invalid {
        assert_eq!(error["code"], "invalid_arguments", "{error}");
    }
    let codes: Vec<&str> = answers
        .errors
        .iter()
        .map(|error| error["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["unknown_target", "ambiguous_target", "refused"]);
    let reports: Vec<String> = log
        .entries()
        .into_iter()
        .filter(|entry| entry.starts_with(&answers.document))
        .take(5)
        .collect();
    assert_eq!(
        reports,
        [
            "gain true",
            "temper true",
            "samples true",
            "samples true",
            "samples true"
        ]
        .map(|report| format!("{} {report}", answers.document))
    );
}
