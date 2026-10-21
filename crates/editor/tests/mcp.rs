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
use concerto_ecs::{Entity, Query, Res, ResMut, Resource, component::name::Name, signal::On};
use concerto_editor::{
    asset_editor::{EditorDocument, Unsaved},
    guard::UnsavedGuard,
    hosted::{EditTarget, EditWorld, EditWorldSignals},
    project::{EditorCommand, EditorCommands, ProjectState},
    tools::{entity_ops::EntityEdit, selection::Select},
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
use concerto_scene::scene::{Scene, SceneNode};
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

fn game(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins)
        .insert_resource(EditFrames::default())
        .add_system(Update, count_edit_frames.at_edit_time());
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

fn state(
    mut calls: ToolCalls<State>,
    project: Res<ProjectState>,
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

fn note_closing(_: On<HostClosing>, log: Res<Log>, mut cmd: concerto_ecs::command::CommandQueue) {
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
        let scene = Scene {
            nodes: vec![node("Hero", vec![1]), node("Lamp", vec![])],
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
    let result = call(client, tool).await;
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
