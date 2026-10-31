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
    Component, Entity, IntoSystem, Query, Res, System,
    command::{CommandQueue, EntityCommandQueue},
    entity::hierarchy::{ChildOf, Children},
    signal::{On, Signal},
};
use concerto_editor::{
    asset_editor::{
        ActiveEditor, AssetEditor, AssetEditorAppExt, AssetEditorCommand, AssetEditorCommands,
        AssetEditorRegistry, EditorDocument, PreviewSpec, asset_request_is_current,
        finish_asset_request, process_editor_commands,
    },
    hosted::{
        HostRole, HostedApp, HostedPlugin,
        input::{ViewportShown, ViewportView},
    },
    preview::{PreviewHostPlugin, PreviewSignals},
    project::{Project, ProjectState},
    scene::SceneRecipe,
};
use concerto_foundation::assets::{
    Asset, AssetId, CreatableAsset,
    asset_server::AssetServer,
    content::{ContentAssetHeader, ImportProvenance, read_content_asset, write_content_asset},
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
use concerto_render::{MaterialComponent, assets::material::StandardMaterial};
use concerto_window::plugin::{Window, WindowPlugin};
use glam::{Quat, Vec2, Vec3};
use std::time::{Duration, Instant};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Dialogue {
    pub text: String,
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
        editor.insert(DialogueDocument { widget });
    }

    fn preview(&self) -> Option<PreviewSpec> {
        Some(PreviewSpec {
            build: dialogue_preview,
        })
    }
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
    app.add_system(Startup, spawn_cube.at_preview_time())
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
    documents: Query<(Entity, &mut EditorDocument, &DialogueDocument)>,
    project: Res<ProjectState>,
    mut previews: PreviewSignals,
    mut commands: CommandQueue,
) {
    let Some(root) = project.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    for (document_entity, mut document, state) in documents.iter() {
        let Some(asset) = document.pending.clone() else {
            continue;
        };
        let result = (|| -> Result<String> {
            let bytes = std::fs::read(root.join(&asset.address))?;
            let (header, payload) = read_content_asset(&bytes)?;
            anyhow::ensure!(
                header.asset_id == asset.id && header.kind == Dialogue::name(),
                "Asset identity or type mismatch"
            );
            let dialogue: Dialogue = bincode::deserialize(payload)?;
            anyhow::ensure!(!dialogue.text.trim().is_empty(), "Dialogue cannot be empty");
            Ok(dialogue.text)
        })();
        if !asset_request_is_current(&document, &asset) {
            continue;
        }
        match result {
            Ok(text) => {
                commands.insert(
                    DialogueWidget {
                        document: document_entity,
                        text,
                    },
                    state.widget,
                );
                previews.trigger(document_entity, SetSpin(2.0));
                assert!(finish_asset_request(&mut document, &asset, Ok(())));
            }
            Err(error) => {
                assert!(finish_asset_request(&mut document, &asset, Err(error)));
            }
        }
    }
}

pub fn register(app: &mut App) -> Result<()> {
    app.register_asset::<Dialogue>();
    app.register_creatable_asset_editor::<Dialogue, DialogueEditor>()?;
    app.add_system(Update, load_dialogue);
    Ok(())
}

/// The game recipe of the smoke test: no renderer, so its previews draw nothing.
pub fn preview_recipe(app: &mut App) {
    app.register_plugin(MainSchedulePlugin)
        .register_plugin(AssetManagerPlugin)
        .register_plugin(TimePlugin)
        .register_plugin(WindowPlugin);
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
        .register_plugin(HostedPlugin)
        .register_plugin(PreviewHostPlugin);
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
            asset_id: id,
            references: vec![],
            kind: Dialogue::name().into(),
            kind_version: 1,
            provenance: Some(provenance.clone()),
        };
        let address = format!("{name}.gasset");
        std::fs::write(
            root.join(&address),
            write_content_asset(
                &header,
                &bincode::serialize(&Dialogue { text: text.into() })?,
            )?,
        )?;
        let asset = AssetEntry {
            id,
            address,
            kind: header.kind,
            kind_version: None,
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
    let deadline = Instant::now() + Duration::from_secs(20);
    while !preview_has_spun(&app, document) {
        anyhow::ensure!(Instant::now() < deadline, "The preview never spun");
        app.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    let world = app.main_mut().world_mut();
    world
        .get_resource_mut::<AssetEditorCommands>()
        .unwrap()
        .0
        .push_back(AssetEditorCommand::CloseAll);
    process.run_and_apply((), world);
    assert_eq!(world.query::<&DialogueWidget, ()>().iter(world).count(), 0);
    Ok(())
}

fn preview_has_spun(app: &App, document: Entity) -> bool {
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
    spin.speed == 2.0 && transform.rotation != Quat::IDENTITY
}

#[allow(dead_code)]
fn main() -> Result<()> {
    smoke_test()?;
    println!("Custom asset editor smoke test passed");
    Ok(())
}
