//! Placing assets in the open scene: which kinds can be placed, and the `place_asset` MCP tool.
use concerto_app::{
    App, PlaceableAssets, Plugin,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, Resource, command::CommandQueue, component::name::Name,
    events::event_reader::EventReader, query::filter::With,
};
use concerto_foundation::assets::{AssetId, registry::AssetRegistry};
use concerto_mcp::{AppMcpExt, McpTool, Needs, PendingCall, ToolCalls, ToolError, ToolRegistry};
use glam::Vec3;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    hosted::{EditTarget, EditWorld, EditWorldSignals, HostRole, Hosted, HostedApp},
    inspector::refused,
    mcp::{SceneEntities, resolve},
    scene::SceneOpened,
    tools::placement::{AssetPlaced, PlaceAsset, PlaceAt},
};

/// Places assets in the open scene: keeps [`PlaceableKinds`] and, when the editor serves MCP, adds the `place_asset` tool.
pub struct AssetPlacementPlugin;

impl Plugin for AssetPlacementPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PlaceableKinds::default())
            .add_system(Update, note_placeable_kinds);
    }

    fn finish(&self, app: &mut App) {
        if app.get_resource::<ToolRegistry>().is_none() {
            return;
        }
        app.add_tool::<PlaceAssetArgs, _>(
            McpTool::new("place_asset").needs(Needs::Scene),
            place_asset,
        )
        .add_system(LateUpdate, answer_placements);
    }
}

/// The asset kinds the recipe registered with `register_placeable_asset`, as of the last scene opened; empty before any has.
#[derive(Resource, Default)]
pub struct PlaceableKinds(Vec<&'static str>);

impl PlaceableKinds {
    /// Whether assets of the kind named `kind` can be placed.
    pub fn contains(&self, kind: &str) -> bool {
        self.0.contains(&kind)
    }

    /// The kinds, in the order the recipe registered them.
    pub fn iter(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.iter().copied()
    }
}

fn note_placeable_kinds(
    mut opened: EventReader<SceneOpened>,
    hosted: Query<&HostedApp>,
    mut kinds: ResMut<PlaceableKinds>,
) {
    for event in opened.read() {
        let Some(app) = hosted
            .get_entity(event.document)
            .filter(|app| app.role() == HostRole::Edit)
        else {
            continue;
        };
        let found: Vec<&'static str> = app
            .world()
            .get_resource::<PlaceableAssets>()
            .map(|placeable| placeable.iter().map(|kind| kind.kind).collect())
            .unwrap_or_default();
        if kinds.0 != found {
            kinds.0 = found;
        }
    }
}

/// Places an asset in the open scene as a new entity named after the asset's file, selects it, and answers `placed <name> as entity <id>`.
#[derive(Deserialize, JsonSchema)]
pub struct PlaceAssetArgs {
    /// The asset, by id or by content address such as content/props/crate.gasset, one list_assets marks placeable.
    pub asset: String,
    /// The parent, such as 42v3, as scene_tree shows it; the scene root without one.
    pub parent: Option<String>,
    /// Where the entity goes, [x, y, z] in world space; the parent's origin without one.
    pub position: Option<[f32; 3]>,
}

#[derive(Component)]
struct Placing {
    document: Entity,
    asset: AssetId,
}

fn place_asset(
    mut calls: ToolCalls<PlaceAssetArgs>,
    registry: Res<AssetRegistry>,
    entities: EditWorld<SceneEntities>,
    targets: Query<Entity, With<EditTarget>>,
    mut scene: EditWorldSignals,
    mut cmd: CommandQueue,
) {
    let Some(document) = targets.iter().next() else {
        return;
    };
    for (arguments, reply) in calls.read() {
        let Some(asset) = registry
            .id_for_address(&arguments.asset)
            .or_else(|| AssetId::from_simple_hex(&arguments.asset).ok())
        else {
            reply.err(ToolError::new(
                "unknown_asset",
                format!(
                    "no asset at {}; list_assets shows what exists",
                    arguments.asset
                ),
            ));
            continue;
        };
        let parent = match arguments.parent.as_deref().map(|id| resolve(id, &entities)) {
            Some(Ok(parent)) => Some(parent),
            Some(Err(error)) => {
                reply.err(error);
                continue;
            }
            None => None,
        };
        let at = match arguments.position.map(Vec3::from_array) {
            Some(position) if position.is_finite() => PlaceAt::Position(position),
            Some(_) => {
                reply.err(ToolError::new(
                    "invalid_arguments",
                    "position needs three finite numbers, [x, y, z] in world space.",
                ));
                continue;
            }
            None => PlaceAt::Origin,
        };
        scene.trigger(PlaceAsset { asset, parent, at });
        cmd.spawn((
            PendingCall::new(reply, "the asset to be placed"),
            Placing { document, asset },
        ));
    }
}

fn answer_placements(
    mut placed: EventReader<Hosted<AssetPlaced>>,
    waiting: Query<(Entity, &PendingCall, &Placing)>,
    names: EditWorld<Query<&Name>>,
    mut cmd: CommandQueue,
) {
    for Hosted { document, event } in placed.read() {
        for (call, pending, placing) in waiting.iter() {
            if placing.document != *document || placing.asset != event.asset {
                continue;
            }
            match &event.result {
                Ok(entity) => pending.reply.ok(format!(
                    "placed {} as entity {}",
                    names.get_entity(*entity).map_or("it", Name::as_str),
                    entity.to_id_string()
                )),
                Err(reason) => pending.reply.err(refused(reason).into()),
            }
            cmd.despawn(call);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use concerto_app::main_schedule::MainSchedulePlugin;
    use concerto_app::plugins::TimePlugin;
    use concerto_ecs::{
        entity::hierarchy::{ChildOf, Children},
        events::event_channel::EventChannel,
    };
    use concerto_foundation::{
        assets::{
            Asset, asset_server::AssetServer, content::save_content_asset_into,
            registry::AssetRegistry,
        },
        transform::{GlobalTransform, Transform},
    };
    use concerto_mcp::{McpPlugin, spawn_server};
    use concerto_mesh::Mesh;
    use concerto_scene::scene::{Scene, SceneNode};
    use concerto_window::plugin::{Window, WindowPlugin};
    use glam::{Mat4, Quat};
    use rmcp::{ServiceExt, model::CallToolRequestParams};
    use serde_json::{Value, json};

    use super::*;
    use crate::{
        asset_editor::ActiveEditor,
        hosted::{
            HostedEventsAppExt, HostedPlugin, SetEditTarget,
            input::{ViewportShown, ViewportView},
            testing::{hosted_now, scene_recipe},
        },
        mcp::EditorMcpPlugin,
        project::{Project, ProjectState},
        tools::{EditToolsCorePlugin, scene::SceneRoot},
    };

    #[derive(serde::Serialize, serde::Deserialize)]
    struct Note;
    impl Asset for Note {
        fn name() -> &'static str {
            "TestNote"
        }
    }

    fn game(app: &mut App) {
        scene_recipe(app);
        app.register_placeable_asset::<Mesh>();
    }

    struct Opened {
        root: Entity,
        anchor: Entity,
    }

    fn open_a_scene(app: &mut App) -> Opened {
        app.register_plugin(EditToolsCorePlugin);
        let world = app.main_mut().world_mut();
        let root = world.spawn((
            Name::new("root"),
            Transform::IDENTITY,
            GlobalTransform::new(Mat4::IDENTITY),
            SceneRoot,
        ));
        let anchor = world.spawn((
            Name::new("anchor"),
            Transform::from_translation_rotation(
                Vec3::new(10.0, 0.0, 0.0),
                Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ),
            GlobalTransform::new(Mat4::IDENTITY),
        ));
        world.entity_mut(root).add_child(anchor);
        Opened { root, anchor }
    }

    struct Answer {
        failed: bool,
        text: String,
    }

    struct Call {
        tool: &'static str,
        arguments: Value,
    }

    fn serve(app: &mut App, calls: Vec<Call>) -> Vec<Answer> {
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let _server = spawn_server(app, server_io);
        let client = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    let client = ().serve(client_io).await.expect("the client connects");
                    let mut answers = Vec::new();
                    for call in calls {
                        let result = client
                            .call_tool(
                                CallToolRequestParams::new(call.tool)
                                    .with_arguments(call.arguments.as_object().cloned().unwrap()),
                            )
                            .await
                            .unwrap();
                        answers.push(Answer {
                            failed: result.is_error == Some(true),
                            text: result
                                .content
                                .iter()
                                .filter_map(|block| block.as_text().map(|text| text.text.clone()))
                                .collect(),
                        });
                    }
                    client.cancel().await.unwrap();
                    answers
                })
        });
        let deadline = Instant::now() + Duration::from_secs(60);
        while !client.is_finished() {
            assert!(Instant::now() < deadline, "the client did not finish");
            app.update();
            std::thread::sleep(Duration::from_millis(1));
        }
        client.join().expect("the client's calls are answered")
    }

    fn place(arguments: Value) -> Call {
        Call {
            tool: "place_asset",
            arguments,
        }
    }

    fn placed(answer: &Answer, name: &str) -> Entity {
        assert!(!answer.failed, "{}", answer.text);
        let id = answer
            .text
            .strip_prefix(&format!("placed {name} as entity "))
            .unwrap_or_else(|| panic!("{}", answer.text));
        Entity::parse_id(id).unwrap()
    }

    fn error(answer: &Answer) -> Value {
        assert!(answer.failed, "{}", answer.text);
        serde_json::from_str(&answer.text).unwrap()
    }

    #[test]
    fn place_asset_puts_an_asset_in_the_scene_and_list_assets_marks_what_it_takes() {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path();
        let mut registry = AssetRegistry::new();
        let node = |name: &str| SceneNode {
            name: name.into(),
            children: vec![],
            components: vec![],
        };
        let pair = save_content_asset_into(
            &Scene {
                nodes: vec![node("left"), node("right")],
                referenced_assets: vec![],
            },
            dir,
            "content/props/pair.gasset",
            &mut registry,
        )
        .unwrap();
        let mesh = save_content_asset_into(
            &Mesh {
                vertices: vec![],
                indices: vec![],
            },
            dir,
            "content/props/crate.gasset",
            &mut registry,
        )
        .unwrap();
        let note = save_content_asset_into(&Note, dir, "content/props/note.gasset", &mut registry)
            .unwrap();

        let mut opened = None;
        let hosted = hosted_now(game, |app| opened = Some(open_a_scene(app)));
        let Opened { root, anchor } = opened.unwrap();
        hosted
            .world()
            .get_resource::<AssetServer>()
            .unwrap()
            .publish_project_content(dir, registry.clone())
            .unwrap();

        let mut project = ProjectState::default();
        project.project = Some(Project {
            root: dir.to_path_buf(),
            content_root: "content".into(),
            default_scene: None,
        });
        let mut app = App::new();
        app.register_plugin(MainSchedulePlugin)
            .register_plugin(TimePlugin)
            .register_plugin(WindowPlugin)
            .insert_resource(Window::new((1280, 720), 1.0))
            .insert_resource(ActiveEditor::default())
            .insert_resource(project)
            .insert_resource(registry.clone())
            .register_plugin(HostedPlugin)
            .register_event::<SceneOpened>()
            .forward_hosted_event::<AssetPlaced>()
            .register_plugin(AssetPlacementPlugin)
            .register_plugin(McpPlugin)
            .register_plugin(EditorMcpPlugin);
        app.finish_plugin_build();
        app.insert_resource(ViewportShown(Some(ViewportView {
            origin: glam::Vec2::ZERO,
            size: (640, 480),
            over: true,
            typing: false,
        })));
        let world = app.main_mut().world_mut();
        let document = world.spawn(hosted);
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(document);
        world.trigger(SetEditTarget(Some(document)));
        world
            .get_resource_mut::<EventChannel<SceneOpened>>()
            .unwrap()
            .push_event(SceneOpened {
                document,
                asset: registry.get(pair).unwrap().clone(),
            });

        let answers = serve(
            &mut app,
            vec![
                place(json!({ "asset": "content/props/pair.gasset" })),
                place(json!({
                    "asset": mesh.simple_hex(),
                    "parent": anchor.to_id_string(),
                    "position": [12.0, 1.0, -3.0],
                })),
                place(json!({
                    "asset": "content/props/crate.gasset",
                    "parent": anchor.to_id_string(),
                })),
                place(json!({ "asset": pair.simple_hex(), "position": [1.0, 0.0, 2.0] })),
                place(json!({ "asset": "content/props/missing.gasset" })),
                place(json!({ "asset": "content/props/note.gasset" })),
                place(json!({ "asset": "content/props/crate.gasset", "parent": "9999v1" })),
                Call {
                    tool: "list_assets",
                    arguments: json!({}),
                },
            ],
        );

        let mut listed: Vec<&str> = answers[7].text.lines().skip(1).collect();
        listed.sort_unstable();
        assert_eq!(
            listed,
            [
                format!(
                    "content/props/crate.gasset · Mesh · {} · placeable",
                    mesh.simple_hex()
                ),
                format!(
                    "content/props/note.gasset · TestNote · {}",
                    note.simple_hex()
                ),
                format!(
                    "content/props/pair.gasset · Scene · {} · placeable",
                    pair.simple_hex()
                ),
            ]
        );

        let world = app
            .main()
            .world()
            .get_component_for_entity::<HostedApp>(document)
            .unwrap()
            .world();
        let parent = |entity| {
            world
                .get_component_for_entity::<ChildOf>(entity)
                .map(ChildOf::parent)
        };
        let local = |entity| {
            world
                .get_component_for_entity::<Transform>(entity)
                .unwrap()
                .translation
        };
        let at = |entity| {
            world
                .get_component_for_entity::<GlobalTransform>(entity)
                .unwrap()
                .matrix()
                .transform_point3(Vec3::ZERO)
        };
        let near = |a: Vec3, b: Vec3| (a - b).length() < 1e-3;

        let first = placed(&answers[0], "pair");
        assert_eq!(parent(first), Some(root));
        assert!(near(local(first), Vec3::ZERO));
        assert_eq!(
            world
                .get_component_for_entity::<Children>(first)
                .map(|children| children.iter().count()),
            Some(2)
        );
        let crated = placed(&answers[1], "crate");
        assert_eq!(parent(crated), Some(anchor));
        assert!(near(at(crated), Vec3::new(12.0, 1.0, -3.0)));
        let hung = placed(&answers[2], "crate");
        assert_eq!(parent(hung), Some(anchor));
        assert!(near(local(hung), Vec3::ZERO));
        assert!(near(at(hung), Vec3::new(10.0, 0.0, 0.0)));
        let second = placed(&answers[3], "pair");
        assert_eq!(parent(second), Some(root));
        assert!(near(local(second), Vec3::new(1.0, 0.0, 2.0)));

        let missing = error(&answers[4]);
        assert_eq!(missing["code"], "unknown_asset");
        assert_eq!(
            missing["message"],
            "no asset at content/props/missing.gasset; list_assets shows what exists"
        );
        let unplaceable = error(&answers[5]);
        assert_eq!(unplaceable["code"], "refused");
        assert_eq!(
            unplaceable["message"],
            "TestNote assets cannot be placed; placeable kinds: Scene, Mesh"
        );
        assert_eq!(error(&answers[6])["code"], "unknown_entity");
        assert_eq!(
            world
                .get_component_for_entity::<Children>(root)
                .map(|children| children.iter().count()),
            Some(3)
        );
    }
}
