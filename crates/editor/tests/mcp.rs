use std::{
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use concerto_app::{
    App,
    edit_time::AtEditTime,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Entity, Query, Res, ResMut, Resource, With, Without, command::CommandQueue,
    component::name::Name, signal::On, signal::listener::GlobalListener,
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
        Asset, AssetId,
        content::{
            CONTENT_FORMAT_VERSION, ContentAssetHeader, ImportProvenance, write_content_asset,
        },
    },
    transform::Transform,
};
use concerto_mcp::{AppMcpExt, McpTool, Needs, ToolCalls, host::HostClosing, host::run_with};
use concerto_mesh::Aabb;
use concerto_render::components::light::Light;
use concerto_scene::scene::{Scene, SceneNode};
use glam::Vec3;
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
    service::RunningService,
};
use schemars::JsonSchema;
use serde::Deserialize;
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

fn game(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins)
        .insert_resource(EditFrames::default())
        .add_system(Update, count_edit_frames.at_edit_time())
        .add_system(Update, bound_by_name.at_edit_time());
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
            concerto_editor::mcp::editor(game, root)(app);
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
        let status = ok(&client, "status").await;
        let all = ok(&client, "frame").await;
        let on_all = ok(&client, "camera_at").await;
        client.cancel().await.unwrap();
        (closed, sword, before, framed, on_sword, status, all, on_all)
    }) else {
        return;
    };

    let (closed, sword, before, framed, on_sword, status, all, on_all) = served.result;
    assert_eq!(closed["code"], "no_scene");
    assert_eq!(before, "0.00");
    assert_eq!(framed, format!("Framed {sword} Sword."));
    assert_eq!(on_sword, "9.50");
    assert_eq!(line(&status, "selection: "), format!("{sword} Sword"));
    assert_eq!(all, "Framed the whole scene.");
    assert_eq!(on_all, "0.00");
}
