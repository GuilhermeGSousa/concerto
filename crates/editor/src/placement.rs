//! Placing assets in the open scene: which kinds can be placed, the `place_asset` MCP tool, and dragging a content tile into the viewport.
use concerto_app::{
    App, PlaceableAssets, Plugin,
    schedule_groups::{LateUpdate, Update},
};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource,
    command::CommandQueue,
    component::name::Name,
    events::event_reader::EventReader,
    query::filter::With,
    signal::{On, lifecycle::Add, listener::IntoListener},
};
use concerto_foundation::assets::{AssetId, registry::AssetRegistry};
use concerto_mcp::{AppMcpExt, McpTool, Needs, PendingCall, ToolCalls, ToolError, ToolRegistry};
use concerto_ui::node::UILayout;
use glam::Vec3;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::{
    asset_editor::ActiveEditor,
    content::ContentAsset,
    drag_drop::{DragDropAppExt, DragSource, DropTarget, Dropped},
    entity_ids::{SceneEntities, resolve},
    hosted::{EditTarget, EditWorld, EditWorldSignals, HostRole, Hosted, HostedApp, PlayState},
    import_fold::shown_name,
    inspector::WriteRefusal,
    scene::SceneOpened,
    tools::placement::{AssetPlaced, PlaceAsset, PlaceAt},
    viewport::ViewportRegion,
};

/// Places assets in the open scene: keeps [`PlaceableKinds`] and, when the editor serves MCP, adds the `place_asset` tool.
pub struct AssetPlacementPlugin;

impl Plugin for AssetPlacementPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PlaceableKinds::default())
            .add_drag_payload::<AssetDrag>()
            .add_system(Update, note_placeable_kinds)
            .add_system(Update, offer_placeable_tiles.after(note_placeable_kinds))
            .add_listener(take_asset_drops);
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

/// What a content tile of a placeable asset carries while it is dragged; dropped on the viewport, the asset is placed under the pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct AssetDrag {
    /// The asset being dragged.
    pub id: AssetId,
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

fn offer_placeable_tiles(
    kinds: Res<PlaceableKinds>,
    registry: Res<AssetRegistry>,
    active: Res<ActiveEditor>,
    states: Query<&PlayState>,
    tiles: Query<(Entity, &ContentAsset, Option<&DragSource<AssetDrag>>)>,
    mut cmd: CommandQueue,
) {
    let playing = active
        .0
        .and_then(|document| states.get_entity(document))
        .is_some_and(|state| state.is_on());
    for (tile, asset, source) in tiles.iter() {
        let wanted = registry
            .get(asset.id)
            .filter(|entry| !playing && kinds.contains(&entry.kind))
            .map(|entry| DragSource::new(AssetDrag { id: entry.id }, shown_name(entry)));
        match (source, wanted) {
            (None, Some(wanted)) => cmd.insert(wanted, tile),
            (Some(source), wanted)
                if wanted.as_ref().is_none_or(|wanted| {
                    wanted.payload != source.payload || wanted.label != source.label
                }) =>
            {
                cmd.remove::<DragSource<AssetDrag>>(tile)
            }
            _ => {}
        }
    }
}

fn take_asset_drops(on: On<Add<ViewportRegion>>, mut cmd: CommandQueue) {
    cmd.insert(
        (
            DropTarget::<AssetDrag>::new(),
            place_dropped_asset.into_listener(),
        ),
        on.signal().entity,
    );
}

fn place_dropped_asset(
    on: On<Dropped<AssetDrag>>,
    regions: Query<&UILayout, With<ViewportRegion>>,
    mut scene: EditWorldSignals,
) {
    let Some(layout) = regions.get_entity(on.entity()) else {
        return;
    };
    let dropped = on.signal();
    scene.trigger(PlaceAsset {
        asset: dropped.payload.id,
        parent: None,
        at: PlaceAt::Pointer(dropped.position - layout.rect.min),
    });
}

/// Places an asset in the open scene as a new entity named as list_assets and the Asset Browser show the asset, its file's name or for an import's scene its source's, selects it, and answers `placed <name> as entity <id>`.
#[derive(Deserialize, JsonSchema)]
pub struct PlaceAssetArgs {
    /// The asset, by id or by content address such as content/props/crate.gasset or content/props/crate/scene.gasset#mesh/0, one list_assets marks placeable.
    pub asset: String,
    /// The parent, such as 42v3, as scene_tree shows it; the scene root without one.
    pub parent: Option<String>,
    /// Where the entity goes, [x, y, z] in world space; the parent's origin without one.
    pub position: Option<[f32; 3]>,
}

#[derive(Component)]
struct Placing {
    document: Entity,
    request: PlaceAsset,
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
            .id_for_location(&arguments.asset)
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
        let request = PlaceAsset { asset, parent, at };
        scene.trigger(request.clone());
        cmd.spawn((
            PendingCall::new(reply, "the asset to be placed"),
            Placing { document, request },
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
            if placing.document != *document || placing.request != event.request {
                continue;
            }
            match &event.result {
                Ok(entity) => pending.reply.ok(format!(
                    "placed {} as entity {}",
                    names.get_entity(*entity).map_or("it", Name::as_str),
                    entity.to_id_string()
                )),
                Err(reason) => pending.reply.err(WriteRefusal::refused(reason).into()),
            }
            cmd.despawn(call);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use concerto_app::edit_time::AtEditTime;
    use concerto_app::main_schedule::MainSchedulePlugin;
    use concerto_app::plugins::TimePlugin;
    use concerto_ecs::{
        World,
        entity::hierarchy::{ChildOf, Children},
        events::event_channel::EventChannel,
        signal::{On, Signal},
    };
    use concerto_foundation::{
        assets::{
            asset_server::AssetServer,
            content::{ImportProvenance, save_content_asset_into},
            registry::AssetRegistry,
        },
        transform::{GlobalTransform, Transform},
    };
    use concerto_mcp::{McpPlugin, ServerHandle};
    use concerto_mesh::Mesh;
    use concerto_render::components::camera::Camera;
    use concerto_scene::node_id::NodeId;
    use concerto_scene::scene::{Scene, SceneNode};
    use concerto_ui::{
        interaction::UIInputState,
        node::{UIBox, UILayout},
        theme::UITheme,
    };
    use concerto_window::plugin::{Window, WindowPlugin};
    use glam::{Mat4, Quat, Vec2};
    use rmcp::{ServiceExt, model::CallToolRequestParams};
    use serde_json::{Value, json};

    use super::*;
    use crate::{
        asset_editor::ActiveEditor,
        content::ContentAsset,
        drag_drop::{DragDropPlugin, DragSource, Dropped},
        hosted::{
            HostedEventsAppExt, HostedPlugin, PlayState, SetEditTarget,
            input::{ViewportShown, ViewportView},
            testing::{hosted_now, scene_recipe},
            trigger_on,
        },
        mcp::EditorMcpPlugin,
        project::{Project, ProjectState},
        tools::{
            EditToolsCorePlugin,
            camera::{EditorCamera, ViewportPointer},
            picking::world_to_viewport,
            scene::SceneRoot,
        },
        viewport::ViewportRegion,
    };

    struct Opened {
        root: Entity,
        anchor: Entity,
    }

    const PICTURE: Vec2 = Vec2::new(640.0, 480.0);
    const PICTURE_AT: Vec2 = Vec2::new(300.0, 40.0);
    const EYE: Vec3 = Vec3::new(0.0, 2.0, 6.0);

    fn picture_camera() -> Camera {
        Camera {
            aspect: PICTURE.x / PICTURE.y,
            ..Default::default()
        }
    }

    struct ShowAPicture;
    impl Signal for ShowAPicture {}

    fn show_a_picture(_: On<ShowAPicture>, mut cmd: CommandQueue) {
        cmd.spawn((
            EditorCamera,
            picture_camera(),
            Transform::from_translation(EYE),
        ));
        cmd.insert_resource(ViewportPointer {
            size: PICTURE,
            ..Default::default()
        });
    }

    fn scene(app: &App, document: Entity) -> &World {
        app.main()
            .world()
            .get_component_for_entity::<HostedApp>(document)
            .unwrap()
            .world()
    }

    fn dragged(app: &App, tile: Entity) -> Option<DragSource<AssetDrag>> {
        app.main()
            .world()
            .get_component_for_entity::<DragSource<AssetDrag>>(tile)
            .cloned()
    }

    fn open_a_scene(app: &mut App) -> Opened {
        app.register_plugin(EditToolsCorePlugin)
            .add_listener(show_a_picture.at_edit_time());
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
        let _server = ServerHandle::spawn(app, server_io);
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

    fn place_the_same_asset_by_pointer(waiting: Query<&Placing>, mut scene: EditWorldSignals) {
        for placing in waiting.iter() {
            scene.trigger(PlaceAsset {
                asset: placing.request.asset,
                parent: None,
                at: PlaceAt::Pointer(glam::Vec2::ZERO),
            });
        }
    }

    #[test]
    fn place_asset_puts_an_asset_in_the_scene_and_list_assets_marks_what_it_takes() {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path();
        let mut registry = AssetRegistry::new();
        let node = |name: &str| SceneNode {
            id: NodeId::from_name_path(name),
            name: name.into(),
            children: vec![],
            components: vec![],
            link: None,
        };
        let pair = save_content_asset_into(
            &Scene {
                nodes: vec![node("left"), node("right")],
                referenced_assets: vec![],
            },
            dir,
            "content/props/pair/scene.gasset",
            &mut registry,
        )
        .unwrap();
        let mut face = registry.get(pair).unwrap().clone();
        face.provenance = Some(ImportProvenance {
            source: "assets/props/pair.glb".into(),
            sub_asset: "scene".into(),
        });
        registry.insert(face);
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

        let mut opened = None;
        let hosted = hosted_now(scene_recipe, |app| opened = Some(open_a_scene(app)));
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
            .insert_resource(UITheme::default())
            .insert_resource(UIInputState::default())
            .register_plugin(DragDropPlugin)
            .register_plugin(HostedPlugin)
            .register_event::<SceneOpened>()
            .forward_hosted_event::<AssetPlaced>()
            .register_plugin(AssetPlacementPlugin)
            .register_plugin(McpPlugin)
            .register_plugin(EditorMcpPlugin)
            .add_system(Update, place_the_same_asset_by_pointer);
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
        let shown = UIBox {
            min: PICTURE_AT,
            size: PICTURE,
        };
        let region = world.spawn((
            ViewportRegion,
            UILayout {
                rect: shown,
                content_rect: shown,
                clip_rect: shown,
                paint_order: 0,
            },
        ));
        let pair_tile = world.spawn(ContentAsset { id: pair });
        let mesh_tile = world.spawn(ContentAsset { id: mesh });
        app.update();
        app.update();
        for tile in [pair_tile, mesh_tile] {
            assert!(
                dragged(&app, tile).is_none(),
                "nothing is known to be placeable before a scene opens"
            );
        }
        app.main_mut()
            .world_mut()
            .get_resource_mut::<EventChannel<SceneOpened>>()
            .unwrap()
            .push_event(SceneOpened {
                document,
                asset: registry.get(pair).unwrap().clone(),
            });

        let answers = serve(
            &mut app,
            vec![
                place(json!({ "asset": "content/props/pair/scene.gasset" })),
                place(json!({
                    "asset": pair.simple_hex(),
                    "parent": anchor.to_id_string(),
                    "position": [12.0, 1.0, -3.0],
                })),
                place(json!({
                    "asset": "content/props/pair/scene.gasset",
                    "parent": anchor.to_id_string(),
                })),
                place(json!({ "asset": pair.simple_hex(), "position": [1.0, 0.0, 2.0] })),
                place(json!({ "asset": "content/props/missing.gasset" })),
                place(json!({ "asset": "content/props/crate.gasset" })),
                place(json!({ "asset": "content/props/pair/scene.gasset", "parent": "9999v1" })),
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
                format!("content/props/crate.gasset · Mesh · {}", mesh.simple_hex()),
                format!(
                    "content/props/pair/scene.gasset · Scene · {} · placeable · import of assets/props/pair.glb with no parts",
                    pair.simple_hex()
                ),
            ]
        );

        let world = scene(&app, document);
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
        let crated = placed(&answers[1], "pair");
        assert_eq!(parent(crated), Some(anchor));
        assert!(near(at(crated), Vec3::new(12.0, 1.0, -3.0)));
        let hung = placed(&answers[2], "pair");
        assert_eq!(parent(hung), Some(anchor));
        assert!(near(local(hung), Vec3::ZERO));
        assert!(near(at(hung), Vec3::new(10.0, 0.0, 0.0)));
        let second = placed(&answers[3], "pair");
        assert_eq!(parent(second), Some(root));
        assert!(near(local(second), Vec3::new(1.0, 0.0, 2.0)));

        assert_eq!(error(&answers[4])["code"], "unknown_asset");
        assert_eq!(error(&answers[5])["code"], "refused");
        assert_eq!(error(&answers[6])["code"], "unknown_entity");
        assert_eq!(
            world
                .get_component_for_entity::<Children>(root)
                .map(|children| children.iter().count()),
            Some(3)
        );

        let source = dragged(&app, pair_tile).expect("a tile of a placeable asset can be dragged");
        assert_eq!(source.payload, AssetDrag { id: pair });
        assert_eq!(source.label, "pair");
        assert!(dragged(&app, mesh_tile).is_none());

        app.main_mut()
            .world_mut()
            .insert(PlayState::Playing, document);
        app.update();
        app.update();
        assert!(
            dragged(&app, pair_tile).is_none(),
            "no tile can be dragged while the game plays"
        );
        app.main_mut()
            .world_mut()
            .insert(PlayState::Editing, document);
        app.update();
        app.update();
        assert!(dragged(&app, pair_tile).is_some());

        trigger_on(app.main_mut().world_mut(), document, ShowAPicture);
        let pointer = Vec2::new(400.0, 300.0);
        app.main_mut().world_mut().trigger_on(
            region,
            Dropped {
                payload: AssetDrag { id: pair },
                position: PICTURE_AT + pointer,
            },
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        let dropped = loop {
            app.update();
            let children: Vec<Entity> = scene(&app, document)
                .get_component_for_entity::<Children>(root)
                .map(|children| children.iter().copied().collect())
                .unwrap_or_default();
            if children.len() == 4 {
                break children[3];
            }
            assert!(
                Instant::now() < deadline,
                "the dropped asset was never placed"
            );
        };
        let world = scene(&app, document);
        assert_eq!(
            world
                .get_component_for_entity::<Name>(dropped)
                .map(Name::as_str),
            Some("pair")
        );
        let landed = world
            .get_component_for_entity::<Transform>(dropped)
            .unwrap()
            .translation;
        assert!(landed.y.abs() < 1e-3, "{landed:?} is not on the ground");
        let view_proj = picture_camera().build_projection_matrix()
            * Transform::from_translation(EYE).compute_matrix().inverse();
        let seen = world_to_viewport(landed, PICTURE, view_proj).unwrap();
        assert!(
            (seen - pointer).length() < 0.5,
            "{landed:?} shows at {seen:?}, not under the pointer at {pointer:?}"
        );
    }
}
