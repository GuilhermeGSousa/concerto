//! A downstream asset editor. Run with `cargo run -p concerto-editor --example custom_asset`.
use anyhow::Result;
use concerto_app::{
    App,
    edit_time::AtPreviewTime,
    main_schedule::MainSchedulePlugin,
    plugins::{AssetManagerPlugin, TimePlugin},
    schedule_groups::{Startup, Update},
};
use concerto_ecs::{
    Component, Entity, IntoSystem, Query, Res, System, World,
    command::{CommandQueue, EntityCommandQueue},
    entity::hierarchy::{ChildOf, Children},
    events::{event_channel::EventChannel, event_reader::EventReader},
    signal::{On, Signal},
};
use concerto_editor::{
    asset_editor::{
        ActiveEditor, AssetEditor, AssetEditorAppExt, AssetEditorCommand, AssetEditorCommands,
        AssetEditorRegistry, EditorDocument, PreviewSpec, Unsaved, asset_request_is_current,
        finish_asset_request, process_editor_commands,
    },
    dock::PanelRegistry,
    hosted::{
        HostRole, Hosted, HostedApp, HostedPlugin,
        input::{ViewportShown, ViewportView},
    },
    inspector::{
        DocumentValue, InspectedValues, InspectorPlugin, PropertyRow, RefreshInspector, spawn_panel,
    },
    preview::{PreviewHostPlugin, PreviewSignals},
    project::{Project, ProjectState},
    scene::SceneRecipe,
    tools::edits::PropertyCommitted,
};
use concerto_foundation::assets::{
    Asset, AssetId, CreatableAsset,
    asset_server::AssetServer,
    content::{
        CONTENT_FORMAT_VERSION, ContentAssetHeader, ImportProvenance, read_content_asset,
        write_content_asset,
    },
    registry::{AssetEntry, AssetRegistry},
};
use concerto_foundation::{
    time::Time,
    transform::{GlobalTransform, Transform},
};
use concerto_mesh::{
    Vertex,
    mesh::{Mesh, MeshComponent},
};
use concerto_reflect::Reflect;
use concerto_render::{MaterialComponent, assets::material::StandardMaterial};
use concerto_ui::{
    focus::{FocusedWidget, UIFocusGained, UIFocusLost},
    node::UINode,
    text_input::{UITextInput, UITextInputCancelled, UITextInputSubmitted},
    theme::UITheme,
};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec2, Vec3};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(serde::Serialize, serde::Deserialize, Reflect)]
pub struct Dialogue {
    pub text: String,
    pub spin_speed: f32,
}
impl Asset for Dialogue {
    fn name() -> &'static str {
        "ExampleDialogue"
    }
}
impl CreatableAsset for Dialogue {
    fn new_asset() -> Self {
        Dialogue {
            text: "Hello".into(),
            spin_speed: 2.0,
        }
    }
}

#[derive(Component)]
pub struct DialogueWidget {
    pub document: Entity,
    pub text: String,
}

#[derive(Component)]
pub struct DialogueDocument {
    pub widget: Entity,
    pub dialogue: Option<Dialogue>,
}

/// Turns its entity's `Transform` around world up, in radians per second.
#[derive(Component)]
pub struct Spin {
    pub speed: f32,
}

/// Sets every `Spin`'s speed in the preview.
pub struct SetSpin(pub f32);
impl Signal for SetSpin {}

#[derive(Default)]
pub struct DialogueEditor;
impl AssetEditor for DialogueEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        let document = editor.entity();
        let widget = editor
            .spawn_child(DialogueWidget {
                document,
                text: String::new(),
            })
            .entity();
        editor.insert((
            DialogueDocument {
                widget,
                dialogue: None,
            },
            dialogue_value(document),
        ));
    }

    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec {
            build: dialogue_preview,
        })
    }
}

fn dialogue_value(document: Entity) -> InspectedValues {
    InspectedValues(vec![Arc::new(DocumentValue::<DialogueDocument> {
        name: "Dialogue".into(),
        document,
        read: Box::new(|held| {
            held.dialogue
                .as_ref()
                .map(|dialogue| dialogue as &dyn Reflect)
        }),
        edit: Box::new(|held| {
            held.dialogue
                .as_mut()
                .map(|dialogue| dialogue as &mut dyn Reflect)
        }),
    })])
}

fn unit_cube() -> Mesh {
    let mut mesh = Mesh {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    for normal in [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ] {
        let tangent = normal.any_orthonormal_vector();
        let bitangent = normal.cross(tangent);
        let first = mesh.vertices.len() as u32;
        for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            let offset = tangent * (u - 0.5) + bitangent * (v - 0.5);
            mesh.vertices.push(Vertex {
                pos_coords: (normal * 0.5 + offset).into(),
                uv_coords: [u, v],
                normal: normal.into(),
                tangent: tangent.into(),
                bitangent: bitangent.into(),
                ..Default::default()
            });
        }
        mesh.indices
            .extend([0, 1, 2, 0, 2, 3].map(|corner| first + corner));
    }
    mesh
}

fn dialogue_preview(app: &mut App) {
    app.register_type::<Dialogue>()
        .add_system(Startup, spawn_cube.at_preview_time())
        .add_system(Update, spin_cubes.at_preview_time())
        .add_listener(set_spin.at_preview_time());
}

fn spawn_cube(server: Option<Res<AssetServer>>, mut cmd: CommandQueue) {
    let cube = cmd
        .spawn((Transform::IDENTITY, Spin { speed: 0.0 }))
        .entity();
    if let Some(server) = server.filter(|server| server.is_registered::<Mesh>()) {
        cmd.insert(
            (
                MeshComponent {
                    handle: server.add(unit_cube()),
                },
                MaterialComponent::<StandardMaterial> {
                    handle: server.add(StandardMaterial::default()),
                },
            ),
            cube,
        );
    }
}

fn spin_cubes(time: Res<Time>, cubes: Query<(&Spin, &mut Transform)>) {
    let dt = time.delta().as_secs_f32();
    for (spin, mut transform) in cubes.iter() {
        if spin.speed != 0.0 {
            transform.rotation = Quat::from_rotation_y(spin.speed * dt) * transform.rotation;
        }
    }
}

fn set_spin(on: On<SetSpin>, spins: Query<&mut Spin>) {
    for mut spin in spins.iter() {
        spin.speed = on.signal().0;
    }
}

/// Normal custom-editor system: validate and deserialize before changing the
/// widget or completing the document request.
pub fn load_dialogue(
    documents: Query<(Entity, &mut EditorDocument, &mut DialogueDocument)>,
    project: Res<ProjectState>,
    mut previews: PreviewSignals,
    mut commands: CommandQueue,
) {
    let Some(root) = project.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    for (document_entity, mut document, mut state) in documents.iter() {
        let Some(asset) = document.pending.clone() else {
            continue;
        };
        let result = (|| -> Result<Dialogue> {
            let bytes = std::fs::read(root.join(&asset.address))?;
            let (header, payload) = read_content_asset(&bytes)?;
            anyhow::ensure!(
                header.asset_id == asset.id && header.kind == Dialogue::name(),
                "Asset identity or type mismatch"
            );
            let dialogue: Dialogue = bincode::deserialize(payload)?;
            anyhow::ensure!(!dialogue.text.trim().is_empty(), "Dialogue cannot be empty");
            Ok(dialogue)
        })();
        if !asset_request_is_current(&document, &asset) {
            continue;
        }
        match result {
            Ok(dialogue) => {
                commands.insert(
                    DialogueWidget {
                        document: document_entity,
                        text: dialogue.text.clone(),
                    },
                    state.widget,
                );
                previews.trigger(document_entity, SetSpin(dialogue.spin_speed));
                state.dialogue = Some(dialogue);
                commands.trigger(RefreshInspector);
                assert!(finish_asset_request(&mut document, &asset, Ok(())));
            }
            Err(error) => {
                assert!(finish_asset_request(&mut document, &asset, Err(error)));
            }
        }
    }
}

/// An edit of a document's `Dialogue` marks it unsaved and sends the speed to its preview.
pub fn follow_dialogue_edits(
    mut committed: EventReader<Hosted<PropertyCommitted>>,
    documents: Query<&DialogueDocument>,
    mut previews: PreviewSignals,
    mut commands: CommandQueue,
) {
    for Hosted { document, event } in committed.read() {
        if event.result.is_ok()
            && let Some(held) = documents.get_entity(*document)
            && let Some(dialogue) = &held.dialogue
        {
            commands.insert(Unsaved, *document);
            previews.trigger(*document, SetSpin(dialogue.spin_speed));
        }
    }
}

pub fn register(app: &mut App) -> Result<()> {
    app.register_asset::<Dialogue>();
    app.register_type::<Dialogue>();
    app.register_creatable_asset_editor::<Dialogue, DialogueEditor>()?;
    app.add_system(Update, load_dialogue)
        .add_system(Update, follow_dialogue_edits);
    Ok(())
}

/// The game recipe of the smoke test: no renderer, so its previews draw nothing.
pub fn preview_recipe(app: &mut App) {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin);
}

fn build_inspector(mut cmd: CommandQueue, theme: Res<UITheme>) {
    let parent = cmd.spawn(UINode::default()).entity();
    spawn_panel(&mut cmd, parent, &theme);
}

pub fn smoke_test() -> Result<()> {
    let root = std::env::temp_dir().join(format!(
        "editor-custom-asset-{}",
        AssetId::new().simple_hex()
    ));
    std::fs::create_dir_all(&root)?;
    let result = smoke_test_at(&root);
    std::fs::remove_dir_all(&root)?;
    result
}

fn smoke_test_at(root: &std::path::Path) -> Result<()> {
    let mut app = App::new();
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin)
        .insert_resource(PanelRegistry::default())
        .insert_resource(UITheme::default())
        .insert_resource(FocusedWidget::default())
        .register_event::<UIFocusGained>()
        .register_event::<UIFocusLost>()
        .register_event::<UITextInputSubmitted>()
        .register_event::<UITextInputCancelled>()
        .register_plugin(HostedPlugin)
        .register_plugin(InspectorPlugin)
        .register_plugin(PreviewHostPlugin)
        .add_system(Startup, build_inspector);
    app.insert_resource(Window::new((64, 64), 1.0));
    app.insert_resource(SceneRecipe {
        recipe: preview_recipe,
        gpu: None,
        clipboard: None,
    });
    app.insert_resource(ViewportShown(Some(ViewportView {
        origin: Vec2::ZERO,
        size: (64, 64),
        over: false,
        typing: false,
    })));
    app.insert_resource(AssetServer::new());
    app.insert_resource(AssetEditorRegistry::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(ActiveEditor::default());
    app.insert_resource(concerto_editor::guard::UnsavedGuard::default());
    register(&mut app)?;
    let mut project = ProjectState::default();
    project.project = Some(Project {
        root: root.into(),
        content_root: "content".into(),
    });
    app.insert_resource(project);
    app.insert_resource(AssetRegistry::default());
    app.finish_plugin_build();
    let world = app.main_mut().world_mut();
    world.register_component::<ChildOf>();
    world.register_component::<Children>();
    let mut process = process_editor_commands.into_system();
    process.initialize(world);
    let mut document = None;
    for (name, text) in [("first", "Hello"), ("second", "Goodbye"), ("invalid", "")] {
        let id = AssetId::new();
        let provenance = ImportProvenance {
            source: "example".into(),
            sub_asset: name.into(),
        };
        let header = ContentAssetHeader {
            format_version: CONTENT_FORMAT_VERSION,
            asset_id: id,
            references: vec![],
            kind: Dialogue::name().into(),
            provenance: Some(provenance.clone()),
        };
        let address = format!("{name}.gasset");
        std::fs::write(
            root.join(&address),
            write_content_asset(
                &header,
                &bincode::serialize(&Dialogue {
                    text: text.into(),
                    spin_speed: 2.0,
                })?,
            )?,
        )?;
        let asset = AssetEntry {
            id,
            address,
            kind: header.kind,
            provenance: Some(provenance),
        };
        world
            .get_resource_mut::<AssetRegistry>()
            .unwrap()
            .insert(asset.clone());
        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open(asset));
        process.run_and_apply((), world);
        let mut load = load_dialogue.into_system();
        load.initialize(world);
        load.run_and_apply((), world);
        // The first pass observes the document/widget archetype created by
        // the lifecycle system; the second pass processes that pending load.
        load.run_and_apply((), world);
        let active = world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        assert_eq!(*document.get_or_insert(active), active);
        let widget = world
            .query::<&DialogueWidget, ()>()
            .iter(world)
            .next()
            .unwrap();
        assert_eq!(widget.document, active);
        let widget_text = widget.text.clone();
        assert_eq!(widget_text, if text.is_empty() { "Goodbye" } else { text });
        let held = world
            .get_component_for_entity::<DialogueDocument>(active)
            .unwrap();
        assert_eq!(held.dialogue.as_ref().unwrap().text, widget_text);
        let doc = world
            .get_component_for_entity::<EditorDocument>(active)
            .unwrap();
        assert_eq!(
            doc.current.as_ref().unwrap().name(),
            if text.is_empty() { "second" } else { name }
        );
        if text.is_empty() {
            assert!(doc.status.contains("empty"));
        }
    }
    let document = document.unwrap();
    wait_for_spin(&mut app, document, 2.0)?;
    app.update();
    let world = app.main_mut().world_mut();
    assert!(
        world
            .get_component_for_entity::<Unsaved>(document)
            .is_none()
    );
    let field = speed_field(world).expect("the inspector shows the speed");
    world
        .get_component_for_entity_mut::<UITextInput>(field)
        .unwrap()
        .value = "5".into();
    world
        .get_resource_mut::<EventChannel<UITextInputSubmitted>>()
        .unwrap()
        .push_event(UITextInputSubmitted {
            entity: field,
            value: "5".into(),
        });
    wait_for_spin(&mut app, document, 5.0)?;
    let world = app.main_mut().world_mut();
    let held = world
        .get_component_for_entity::<DialogueDocument>(document)
        .unwrap();
    assert_eq!(held.dialogue.as_ref().unwrap().spin_speed, 5.0);
    assert!(
        world
            .get_component_for_entity::<Unsaved>(document)
            .is_some()
    );
    world
        .get_resource_mut::<AssetEditorCommands>()
        .unwrap()
        .0
        .push_back(AssetEditorCommand::CloseAll);
    process.run_and_apply((), world);
    assert_eq!(world.query::<&DialogueWidget, ()>().iter(world).count(), 0);
    Ok(())
}

fn speed_field(world: &mut World) -> Option<Entity> {
    let (row, _) = world
        .query::<(Entity, &PropertyRow), ()>()
        .iter(world)
        .find(|(_, row)| row.address.path.to_string() == "spin_speed")?;
    text_field(world, row)
}

fn text_field(world: &World, entity: Entity) -> Option<Entity> {
    if world
        .get_component_for_entity::<UITextInput>(entity)
        .is_some()
    {
        return Some(entity);
    }
    world
        .get_component_for_entity::<Children>(entity)?
        .iter()
        .find_map(|child| text_field(world, *child))
}

fn wait_for_spin(app: &mut App, document: Entity, speed: f32) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !preview_has_spun(app, document, speed) {
        anyhow::ensure!(
            Instant::now() < deadline,
            "The preview never spun at {speed}"
        );
        app.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

fn preview_has_spun(app: &App, document: Entity, speed: f32) -> bool {
    let Some(hosted) = app
        .main()
        .world()
        .get_component_for_entity::<HostedApp>(document)
    else {
        return false;
    };
    assert_eq!(hosted.role(), HostRole::Preview);
    let world = hosted.world_for_test();
    let spins: Vec<_> = world
        .archetypes()
        .iter()
        .flat_map(|archetype| archetype.entities())
        .filter_map(|entity| {
            let spin = world.get_component_for_entity::<Spin>(*entity)?;
            let transform = world.get_component_for_entity::<Transform>(*entity)?;
            Some((*entity, spin, transform))
        })
        .collect();
    assert_eq!(spins.len(), 1);
    let (cube, spin, transform) = spins[0];
    assert!(
        world
            .get_component_for_entity::<GlobalTransform>(cube)
            .is_some()
    );
    spin.speed == speed && transform.rotation != Quat::IDENTITY
}

#[allow(dead_code)]
fn main() -> Result<()> {
    smoke_test()?;
    println!("Custom asset editor smoke test passed");
    Ok(())
}
