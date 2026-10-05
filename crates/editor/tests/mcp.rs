//! The editor's MCP tools against a headless editor and a fixture project,
//! called the way a client calls them. No window, GPU or transport.
#![cfg(feature = "mcp")]

use std::path::Path;

use concerto_app::{
    App,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin, TransformPlugin},
};
use concerto_editor::{EditorPlugin, mcp::EditorMcpPlugin};
use concerto_foundation::{
    assets::{
        Asset, AssetId,
        content::{
            CONTENT_FORMAT_VERSION, ContentAssetHeader, ImportProvenance, write_content_asset,
        },
    },
    transform::Transform,
};
use concerto_mcp::{ToolResult, call_tool};
use concerto_mesh::Mesh;
use concerto_render::assets::texture::Texture;
use concerto_scene::scene::{Scene, SceneNode};
use serde_json::{Value, json};

/// Frames a call may take; loads run on worker threads.
const FRAMES: usize = 2_000;

/// Apps in one process share the global compute pool, and as many apps
/// updating at once as the pool has threads deadlock it. Run one at a time.
static ONE_APP_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A headless editor, and the lock that keeps it the only app updating.
struct Editor {
    app: App,
    _turn: std::sync::MutexGuard<'static, ()>,
}

impl std::ops::Deref for Editor {
    type Target = App;
    fn deref(&self) -> &App {
        &self.app
    }
}

impl std::ops::DerefMut for Editor {
    fn deref_mut(&mut self) -> &mut App {
        &mut self.app
    }
}

fn write_asset(root: &Path, address: &str, kind: &str, payload: &[u8]) {
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: AssetId::new(),
        references: vec![],
        kind: kind.into(),
        provenance: Some(ImportProvenance {
            source: "fixture.glb".into(),
            sub_asset: address.into(),
        }),
    };
    std::fs::write(
        root.join(address),
        write_content_asset(&header, payload).unwrap(),
    )
    .unwrap();
}

/// `Hero` → `Body` → `Spine` → `Neck` → `Head`, and `Hero` → `Lamp`.
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("content")).unwrap();
    let node = |name: &str, children: Vec<usize>, translation: [f32; 3]| {
        let mut node = SceneNode {
            name: name.into(),
            children,
            components: vec![],
        };
        node.push_component(&Transform::from_translation(translation.into()))
            .unwrap();
        node
    };
    let scene = Scene {
        nodes: vec![
            node("Hero", vec![1, 5], [0.0; 3]),
            node("Body", vec![2], [1.0, 2.0, 3.0]),
            node("Spine", vec![3], [0.0; 3]),
            node("Neck", vec![4], [0.0; 3]),
            node("Head", vec![], [0.0; 3]),
            node("Lamp", vec![], [0.0; 3]),
        ],
        referenced_assets: vec![],
    };
    write_asset(
        dir.path(),
        "content/level.gasset",
        Scene::name(),
        &bincode::serialize(&scene).unwrap(),
    );
    write_asset(dir.path(), "content/rock.gasset", "Mesh", &[]);
    dir
}

fn editor(project: Option<&Path>) -> Editor {
    let turn = ONE_APP_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(TransformPlugin);
    // RenderPlugin needs a GPU adapter; register the asset types it would.
    app.register_asset::<Mesh>().register_asset::<Texture>();
    app.register_plugin(concerto_scene::plugin::ScenePlugin)
        .register_plugin(EditorPlugin {
            project: project.map(Path::to_path_buf),
            headless: true,
            ..Default::default()
        })
        .register_plugin(EditorMcpPlugin);
    app.finish_plugin_build();
    Editor { app, _turn: turn }
}

fn call(app: &mut App, tool: &str, arguments: Value) -> ToolResult {
    call_tool(app, tool, arguments, FRAMES)
}

fn ok(app: &mut App, tool: &str, arguments: Value) -> String {
    match call(app, tool, arguments.clone()) {
        Ok(output) => output.text,
        Err(error) => panic!("{tool} {arguments} failed: {error}"),
    }
}

fn error_code(app: &mut App, tool: &str, arguments: Value) -> &'static str {
    match call(app, tool, arguments.clone()) {
        Ok(output) => panic!("{tool} {arguments} succeeded: {}", output.text),
        Err(error) => error.code,
    }
}

/// The id at the start of the `scene_tree` or `find_entities` line naming `name`.
fn id_of(listing: &str, name: &str) -> String {
    listing
        .lines()
        .find(|line| {
            line.split_whitespace()
                .nth(1)
                .is_some_and(|word| word.ends_with(name))
        })
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or_else(|| panic!("no `{name}` in:\n{listing}"))
        .to_owned()
}

fn opened(project: &Path) -> Editor {
    let mut app = editor(None);
    let text = ok(
        &mut app,
        "open_project",
        json!({ "path": project.to_str().unwrap() }),
    );
    assert!(text.contains("2 assets: 1 Mesh, 1 Scene"), "{text}");
    ok(
        &mut app,
        "open_asset",
        json!({ "asset": "content/level.gasset" }),
    );
    app
}

#[test]
fn tools_that_need_a_project_say_so() {
    let mut app = editor(None);
    assert!(ok(&mut app, "status", json!({})).contains("project: none"));
    assert_eq!(error_code(&mut app, "list_assets", json!({})), "no_project");
    assert_eq!(
        error_code(&mut app, "open_asset", json!({ "asset": "level" })),
        "no_project"
    );
}

#[test]
fn opening_a_missing_project_fails_with_the_reason() {
    let mut app = editor(None);
    let error = call(
        &mut app,
        "open_project",
        json!({ "path": "/no/such/project" }),
    )
    .unwrap_err();
    assert_eq!(error.code, "open_failed");
    assert!(
        error.message.contains("Could not open project"),
        "{}",
        error.message
    );
}

#[test]
fn a_call_made_while_the_launch_project_opens_waits_for_it() {
    let project = fixture();
    let mut app = editor(Some(project.path()));
    let text = ok(&mut app, "open_asset", json!({ "asset": "level" }));
    assert!(text.starts_with("Opened content/level.gasset"), "{text}");
}

#[test]
fn listing_filters_and_pages_the_catalogue() {
    let project = fixture();
    let mut app = opened(project.path());
    let text = ok(&mut app, "list_assets", json!({ "kind": "Scene" }));
    assert!(text.starts_with("1 asset match"), "{text}");
    assert!(text.contains("content/level.gasset · Scene · "), "{text}");
    let text = ok(&mut app, "list_assets", json!({ "limit": 1 }));
    assert!(text.ends_with("… 1 more (list_assets offset=1)"), "{text}");
    assert_eq!(
        error_code(
            &mut app,
            "open_asset",
            json!({ "asset": "content/rock.gasset" })
        ),
        "no_editor"
    );
    assert_eq!(
        error_code(&mut app, "open_asset", json!({ "asset": "missing" })),
        "unknown_asset"
    );
}

#[test]
fn the_tree_truncates_with_the_call_that_continues_it() {
    let project = fixture();
    let mut app = opened(project.path());
    let tree = ok(&mut app, "scene_tree", json!({ "depth": 3 }));
    // Scene root, Hero, then Body and Lamp; Spine is below depth 3.
    assert!(tree.contains("(scene content/level.gasset)"), "{tree}");
    assert!(!tree.contains("Spine"), "{tree}");
    let body = id_of(&tree, "Body");
    assert!(
        tree.contains(&format!("… 3 more below (scene_tree root={body})")),
        "{tree}"
    );

    let deeper = ok(&mut app, "scene_tree", json!({ "root": body }));
    assert!(
        deeper.contains("Spine") && deeper.contains("Neck"),
        "{deeper}"
    );

    let limited = ok(&mut app, "scene_tree", json!({ "depth": 10, "limit": 2 }));
    assert!(
        limited.ends_with("… 5 more within depth 10 not shown (scene_tree limit=7, or pass a deeper entity as root)"),
        "{limited}"
    );
}

#[test]
fn find_and_inspect_read_live_values() {
    let project = fixture();
    let mut app = opened(project.path());
    let found = ok(&mut app, "find_entities", json!({ "name": "head" }));
    assert!(
        found.contains("(unnamed)/Hero/Body/Spine/Neck/Head [Transform]"),
        "{found}"
    );

    let found = ok(&mut app, "find_entities", json!({ "name": "body" }));
    let body = id_of(&found, "Body");
    let report: Value =
        serde_json::from_str(&ok(&mut app, "inspect", json!({ "entity": body }))).unwrap();
    assert_eq!(report["name"], "Body");
    assert_eq!(
        report["components"]["Transform"]["translation"],
        json!([1.0, 2.0, 3.0])
    );
    assert_eq!(report["children"].as_array().unwrap().len(), 1);

    assert_eq!(
        error_code(
            &mut app,
            "inspect",
            json!({ "entity": body, "components": ["Light"] })
        ),
        "unknown_component"
    );
    assert_eq!(
        error_code(&mut app, "inspect", json!({ "entity": "nonsense" })),
        "invalid_entity"
    );
    assert!(
        ok(&mut app, "find_entities", json!({ "component": "Light" }))
            .starts_with("No entities match")
    );
}

#[test]
fn ids_go_stale_when_their_scene_closes() {
    let project = fixture();
    let mut app = opened(project.path());
    let tree = ok(&mut app, "scene_tree", json!({}));
    let hero = id_of(&tree, "Hero");
    let status = ok(&mut app, "status", json!({}));
    let editor = status
        .lines()
        .find(|line| line.contains("content/level.gasset [active]"))
        .and_then(|line| line.split_whitespace().next())
        .unwrap_or_else(|| panic!("no open editor in:\n{status}"))
        .to_owned();

    ok(&mut app, "close_editor", json!({ "editor": editor }));
    assert!(ok(&mut app, "status", json!({})).contains("editors: none open"));
    assert_eq!(
        error_code(&mut app, "inspect", json!({ "entity": hero })),
        "stale_entity"
    );
}

#[test]
fn selection_and_camera_are_the_editors_own() {
    let project = fixture();
    let mut app = opened(project.path());
    let tree = ok(&mut app, "scene_tree", json!({}));
    let lamp = id_of(&tree, "Lamp");

    ok(&mut app, "select", json!({ "entity": lamp }));
    assert!(
        ok(&mut app, "status", json!({})).contains(&format!("selection: entity {lamp} \"Lamp\""))
    );
    ok(&mut app, "select", json!({ "asset": "rock" }));
    assert!(ok(&mut app, "status", json!({})).contains("selection: asset content/rock.gasset"));
    ok(&mut app, "select", json!({}));
    assert!(ok(&mut app, "status", json!({})).contains("selection: none"));

    let text = ok(
        &mut app,
        "set_camera",
        json!({ "position": [0, 2, 10], "look_at": [-10, 2, 10] }),
    );
    // Looking down -X is a quarter turn of yaw from -Z, level.
    assert_eq!(
        text,
        "Camera at [0.000, 2.000, 10.000], yaw 1.571, pitch 0.000."
    );
    assert_eq!(
        error_code(
            &mut app,
            "set_camera",
            json!({ "position": [0, 2, 10], "look_at": [0, 2, 10] })
        ),
        "invalid_arguments"
    );
    // The fixture has no meshes, so framing has nothing to fit.
    assert_eq!(error_code(&mut app, "frame", json!({})), "nothing_to_frame");
}
