//! Refreshing after an import must not disturb open documents.
use concerto_app::{App, schedule_groups::Update};
use concerto_editor::{
    asset_editor::{AssetEditorCommand, AssetEditorCommands},
    guard::UnsavedGuard,
    project::{EditorCommand, EditorCommands, ProjectPlugin, ProjectState, discover_project},
};
use concerto_foundation::assets::{
    AssetId,
    asset_server::AssetServer,
    content::{CONTENT_FORMAT_VERSION, ContentAssetHeader, ImportProvenance, write_content_asset},
};

fn asset(root: &std::path::Path, name: &str) {
    let header = ContentAssetHeader {
        format_version: CONTENT_FORMAT_VERSION,
        asset_id: AssetId::new(),
        references: vec![],
        kind: "Mesh".into(),
        provenance: Some(ImportProvenance {
            source: "assets/model.obj".into(),
            sub_asset: name.into(),
        }),
    };
    std::fs::create_dir_all(root.join("content")).expect("content");
    std::fs::write(
        root.join(format!("content/{name}.gasset")),
        write_content_asset(&header, &[]).expect("encode"),
    )
    .expect("write");
}

#[test]
fn refreshing_adds_assets_without_bumping_generation_or_closing_documents() {
    let project = tempfile::tempdir().expect("tempdir");
    asset(project.path(), "first");

    let mut app = App::new();
    app.insert_resource(EditorCommands::default());
    app.insert_resource(AssetEditorCommands::default());
    app.insert_resource(UnsavedGuard::default());
    app.insert_resource(AssetServer::new());
    let mut state = ProjectState::default();
    state.project = Some(discover_project(project.path()).expect("discover"));
    state.generation = 7;
    app.insert_resource(state);
    app.register_plugin(ProjectPlugin);
    app.finish_plugin_build();

    asset(project.path(), "second");
    app.get_resource_mut::<EditorCommands>()
        .expect("commands")
        .0
        .push_back(EditorCommand::RefreshCatalogue);
    app.main_mut().world_mut().run_schedule(Update);

    let state = app.get_resource::<ProjectState>().expect("state");
    assert_eq!(
        state.project.as_ref().expect("project").assets.len(),
        2,
        "the refresh picks up the newly written asset"
    );
    assert_eq!(
        state.generation, 7,
        "a refresh is the same project, so generation is untouched"
    );
    let documents = app
        .get_resource::<AssetEditorCommands>()
        .expect("documents");
    assert!(
        !documents
            .0
            .iter()
            .any(|c| matches!(c, AssetEditorCommand::CloseAll)),
        "a refresh must not close open documents"
    );
}
