//! A downstream asset editor. Run with `cargo run -p editor --example custom_asset`.
use anyhow::Result;
use app::schedule_groups::Update;
use ecs::{
    Component, Entity, IntoSystem, Query, Res,
    command::{CommandQueue, EntityCommandQueue},
    entity::hierarchy::{ChildOf, Children},
};
use editor::{
    asset_editor::{
        ActiveEditor, AssetEditor, AssetEditorAppExt, AssetEditorCommand, AssetEditorCommands,
        AssetEditorRegistry, EditorDocument, asset_request_is_current, finish_asset_request,
        process_editor_commands,
    },
    project::{AssetEntry, Project, ProjectState},
};
use essential::assets::{
    Asset, AssetId,
    asset_server::AssetServer,
    content::{
        AssetRegistry, CONTENT_FORMAT_VERSION, ContentAssetHeader, ImportProvenance,
        read_content_asset, write_content_asset,
    },
};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Dialogue {
    pub text: String,
}
impl Asset for Dialogue {
    fn name() -> &'static str {
        "ExampleDialogue"
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

pub struct DialogueEditor;
impl AssetEditor for DialogueEditor {
    fn build(&self, editor: &mut EntityCommandQueue) {
        let document = editor.entity();
        let widget = editor
            .spawn_child_queue(DialogueWidget {
                document,
                text: String::new(),
            })
            .entity();
        editor.insert(DialogueDocument { widget });
    }
}

/// Normal custom-editor system: validate and deserialize before changing the
/// widget or completing the document request.
pub fn load_dialogue(
    documents: Query<(Entity, &mut EditorDocument, &DialogueDocument)>,
    project: Res<ProjectState>,
    mut commands: CommandQueue,
) {
    let Some(root) = project.project.as_ref().map(|project| project.root.clone()) else {
        return;
    };
    for (document_entity, mut document, state) in documents.iter() {
        let Some(asset) = document.pending.clone() else {
            continue;
        };
        let request = document.request_generation;
        let generation = project.generation;
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
        if !asset_request_is_current(&document, request, generation) {
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
                assert!(finish_asset_request(
                    &mut document,
                    request,
                    generation,
                    Ok(())
                ));
            }
            Err(error) => {
                assert!(finish_asset_request(
                    &mut document,
                    request,
                    generation,
                    Err(error)
                ));
            }
        }
    }
}

pub fn register(app: &mut app::App) -> Result<()> {
    app.register_asset::<Dialogue>();
    app.register_asset_editor::<Dialogue>(DialogueEditor)?;
    app.add_system(Update, load_dialogue);
    Ok(())
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
    let mut app = app::App::new();
    app.insert_resource(AssetServer::new());
    app.insert_resource(AssetEditorRegistry::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(ActiveEditor::default());
    register(&mut app)?;
    let mut project = ProjectState::default();
    project.project = Some(Project {
        root: root.into(),
        assets: vec![],
        registry: AssetRegistry::default(),
    });
    project.generation = 1;
    app.insert_resource(project);
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
                &bincode::serialize(&Dialogue { text: text.into() })?,
            )?,
        )?;
        let asset = AssetEntry {
            id,
            address,
            kind: header.kind,
            display_name: name.into(),
            folder: String::new(),
            provenance,
        };
        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open {
                asset,
                project_generation: 1,
            });
        process.run_and_apply(world);
        let mut load = load_dialogue.into_system();
        load.initialize(world);
        load.run_and_apply(world);
        // The first pass observes the document/widget archetype created by
        // the lifecycle system; the second pass processes that pending load.
        load.run_and_apply(world);
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
            doc.current.as_ref().unwrap().display_name,
            if text.is_empty() { "second" } else { name }
        );
        if text.is_empty() {
            assert!(doc.status.contains("empty"));
        }
    }
    world
        .get_resource_mut::<AssetEditorCommands>()
        .unwrap()
        .0
        .push_back(AssetEditorCommand::CloseAll);
    process.run_and_apply(world);
    assert_eq!(world.query::<&DialogueWidget, ()>().iter(world).count(), 0);
    Ok(())
}

#[allow(dead_code)]
fn main() -> Result<()> {
    smoke_test()?;
    println!("Custom asset editor smoke test passed");
    Ok(())
}
