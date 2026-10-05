//! Engine-native editor. Panels submit commands; project systems own I/O.
//!
//! Asset editors allow temporary inspection and edits, but never save files.
//! Importing is the `import` CLI's job.
pub mod actions;
pub mod asset_editor;
pub mod content;
pub mod diagnostics;
pub mod dock;
pub mod fonts;
pub mod hierarchy;
pub mod inspector;
pub mod marks;
#[cfg(feature = "mcp")]
pub mod mcp;
pub mod project;
pub mod scene;
pub mod selection;
pub mod shell;
pub mod tabs;
pub mod viewport;
pub mod window_chrome;
mod workspace;

use concerto_app::{App, Plugin};
use concerto_ecs::IntoSystemConfig;
use std::path::PathBuf;

pub struct EditorPlugin {
    pub project: Option<PathBuf>,
    /// Let the window manager draw the window frame instead of the editor.
    pub decorated: bool,
    /// Run without a window: the project, asset editors, scene preview,
    /// selection, property model and editor camera, but no panels. Pair with
    /// an app that registers neither `WindowPlugin` nor `UIPlugin`.
    pub headless: bool,
    /// The viewport's initial render size. A windowed editor resizes it to its
    /// panel; a headless one keeps it.
    pub viewport_size: [u32; 2],
}

impl Default for EditorPlugin {
    fn default() -> Self {
        Self {
            project: None,
            decorated: false,
            headless: false,
            viewport_size: [1280, 720],
        }
    }
}

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        // The core: everything the editor knows and does without a window.
        app.insert_resource(selection::Selection::default());
        app.insert_resource(project::EditorCommands::default());
        app.insert_resource(project::ProjectState::default());
        app.register_plugin(project::ProjectPlugin);
        app.register_plugin(inspector::InspectorPlugin);
        app.insert_resource(asset_editor::AssetEditorRegistry::default());
        app.insert_resource(asset_editor::AssetEditorCommands::default());
        app.insert_resource(asset_editor::ActiveEditor::default());
        app.add_system(
            concerto_app::schedule_groups::Update,
            asset_editor::process_editor_commands,
        );
        app.register_plugin(scene::ScenePlugin);
        app.register_plugin(viewport::ViewportPlugin {
            size: self.viewport_size,
        });
        if let Some(path) = &self.project {
            app.get_resource_mut::<project::EditorCommands>()
                .expect("EditorCommands was just inserted")
                .0
                .push_back(project::EditorCommand::OpenProject(path.clone()));
        }
        if self.headless {
            return;
        }

        // The panels: everything that reads a window, input or the UI.
        app.insert_resource(dock::PanelRegistry::default());
        app.register_plugin(fonts::FontsPlugin);
        app.register_plugin(dock::DockPlugin);
        app.register_plugin(actions::ActionsPlugin);
        app.register_plugin(hierarchy::HierarchyPlugin);
        app.register_plugin(content::ContentPlugin);
        app.register_plugin(diagnostics::DiagnosticsPlugin);
        app.register_plugin(inspector::InspectorPanelPlugin);
        app.add_system(
            concerto_app::schedule_groups::Update,
            workspace::create_editor_hosts,
        );
        app.add_system(
            concerto_app::schedule_groups::Update,
            workspace::sync_workspace,
        );
        // Must see a scene replacement's `reset` before the viewport consumes it.
        app.add_system(
            concerto_app::schedule_groups::Update,
            workspace::reset_workspace_input.before(viewport::process_navigation_requests),
        );
        app.register_plugin(viewport::ViewportPanelPlugin);
        app.register_plugin(shell::ShellPlugin);
        app.register_plugin(tabs::TabsPlugin);
        app.register_plugin(window_chrome::WindowChromePlugin {
            decorated: self.decorated,
        });
    }
}
