//! Contextual panel hosts and input transitions for asset editors.
use ecs::{
    command::CommandQueue,
    entity::hierarchy::{ChildOf, Children},
    query::change_detection::DetectChanges,
    Changed, Entity, Query, Res, ResMut, Without,
};
use essential::assets::Asset;
use ui::{
    focus::FocusedWidget,
    interaction::{HoveredNode, Interactable, UIInputState},
    material::UIMaterial,
    node::UINode,
    theme::UITheme,
};
use window::input::actions::ActionMap;

use crate::{
    asset_editor::{ActiveEditor, EditorDocument, EditorHosts, EditorOwned},
    dock::PanelRegistry,
    hierarchy::HierarchyState,
    viewport::ViewportCommands,
};

/// Build contextual containers for new documents. Runs before workspace
/// visibility and custom UI systems, so deferred hosts are queryable there.
pub(crate) fn create_editor_hosts(
    documents: Query<(Entity, &EditorDocument), Without<EditorHosts>>,
    registry: Res<PanelRegistry>,
    theme: Res<UITheme>,
    nodes: Query<(&UINode, &ChildOf)>,
    mut commands: CommandQueue,
) {
    let bases = panel_bodies(&registry);
    for (editor, doc) in documents.iter() {
        let mut hosts = [None; 3];
        if doc.asset_type != scene::scene::Scene::name() {
            for (index, base) in bases.iter().copied().enumerate() {
                let Some((node, parent)) = base.and_then(|base| nodes.get_entity(base)) else {
                    continue;
                };
                let mut node = node.clone();
                node.visible = false;
                let host = commands
                    .spawn((
                        node,
                        EditorOwned(editor),
                        Interactable,
                        UIMaterial {
                            corner_radius: if index == 0 { 0.0 } else { theme.radius_lg },
                            ..UIMaterial::flat(if index == 0 {
                                theme.canvas
                            } else {
                                theme.surface
                            })
                        },
                    ))
                    .entity();
                commands.add_child(parent.parent(), host);
                hosts[index] = Some(host);
            }
        }
        commands.insert(
            EditorHosts {
                canvas: hosts[0],
                left: hosts[1],
                inspector: hosts[2],
            },
            editor,
        );
    }
}

fn panel_bodies(registry: &PanelRegistry) -> [Option<Entity>; 3] {
    [
        registry.body(crate::viewport::PANEL_ID),
        registry.body(crate::hierarchy::PANEL_ID),
        registry.body(crate::inspector::PANEL_ID),
    ]
}

/// Update visibility only when tab/panel resources or host components change.
/// Host changes matter even when the active editor stays the same.
pub(crate) fn sync_workspace(
    active: Res<ActiveEditor>,
    registry: Res<PanelRegistry>,
    documents: Query<&EditorDocument>,
    hosts: Query<(Entity, &EditorHosts)>,
    changed_hosts: Query<&EditorHosts, Changed<EditorHosts>>,
    nodes: Query<&mut UINode>,
    parents: Query<&ChildOf>,
    children: Query<&mut Children>,
) {
    let hosts_changed = changed_hosts.iter().next().is_some();
    if !active.has_changed() && !registry.has_changed() && !hosts_changed {
        return;
    }
    let scene = active
        .0
        .and_then(|entity| documents.get_entity(entity))
        .is_none_or(|doc| doc.asset_type == scene::scene::Scene::name());
    let bases = panel_bodies(&registry);
    for entity in bases.into_iter().flatten() {
        set_visible(&nodes, entity, scene);
    }
    for (entity, hosts) in hosts.iter() {
        for host in [hosts.canvas, hosts.left, hosts.inspector]
            .into_iter()
            .flatten()
        {
            set_visible(&nodes, host, active.0 == Some(entity));
        }
    }
    // Command application has attached new hosts by this point. Keep the
    // contextual left panel before the shared content browser in the rail.
    if hosts_changed || registry.has_changed() {
        if let Some(base) = bases[1] {
            if let Some(mut siblings) = parents
                .get_entity(base)
                .and_then(|parent| children.get_entity(parent.parent()))
            {
                let contextual: Vec<_> = hosts.iter().filter_map(|(_, h)| h.left).collect();
                siblings.sort_by_key(|entity| {
                    usize::from(entity != base && !contextual.contains(&entity))
                });
            }
        }
    }
}

fn set_visible(nodes: &Query<&mut UINode>, entity: Entity, visible: bool) {
    if let Some(mut node) = nodes.get_entity(entity) {
        if node.visible != visible {
            node.visible = visible;
        }
    }
}

/// Clear transient UI state on a tab transition or successful scene replacement.
/// Native pointer capture is released by the viewport's main-thread system.
pub(crate) fn reset_workspace_input(
    active: Res<ActiveEditor>,
    mut viewport: ResMut<ViewportCommands>,
    mut focus: ResMut<FocusedWidget>,
    mut hovered: ResMut<HoveredNode>,
    mut input: ResMut<UIInputState>,
    mut actions: ResMut<ActionMap>,
    mut hierarchy: ResMut<HierarchyState>,
) {
    let reset = viewport.has_changed() && viewport.reset;
    if !active.has_changed() && !reset {
        return;
    }
    **focus = None;
    **hovered = None;
    *input = UIInputState::default();
    actions.pop_context(crate::actions::TreeContext);
    actions.pop_context(crate::actions::ViewportContext);
    viewport.release_navigation = true;
    if reset {
        *hierarchy = HierarchyState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecs::{IntoSystem, System, World};
    fn process_editor_commands(world: &mut World) {
        let mut system = crate::asset_editor::process_editor_commands.into_system();
        system.initialize(world);
        system.run_and_apply(world);
    }
    fn sync_workspace(world: &mut World) {
        for mut system in [
            super::sync_workspace.into_system(),
            super::reset_workspace_input.into_system(),
        ] {
            system.initialize(world);
            system.run_and_apply(world);
        }
    }
    use crate::asset_editor::{
        AssetEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry,
    };
    use essential::assets::{content::ImportProvenance, Asset, AssetId};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct Notes;
    impl Asset for Notes {
        fn name() -> &'static str {
            "Notes"
        }
    }
    struct NotesEditor;
    impl AssetEditor for NotesEditor {}

    fn resources(world: &mut World) {
        world.insert_resource(ActiveEditor::default());
        world.insert_resource(AssetEditorCommands::default());
        world.insert_resource(AssetEditorRegistry::default());
        world.insert_resource(crate::project::ProjectState::default());
        world.insert_resource(PanelRegistry::default());
        world.insert_resource(ViewportCommands::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(HoveredNode::default());
        world.insert_resource(UIInputState::default());
        world.insert_resource(ActionMap::default());
        world.insert_resource(HierarchyState::default());
    }

    #[test]
    fn workspace_systems_do_not_request_exclusive_world_access() {
        for system in [
            super::create_editor_hosts.into_system(),
            super::sync_workspace.into_system(),
            super::reset_workspace_input.into_system(),
        ] {
            let mut meta = ecs::system::meta::SystemMetadata::default();
            let mut access = ecs::system::access::SystemAccess::default();
            system.fill_access(&mut meta, &mut access);
            assert!(!access.is_exclusive());
        }
    }

    #[test]
    fn unchanged_resources_preserve_focus_and_new_hosts_refresh_without_activation() {
        let mut world = World::new();
        resources(&mut world);
        let editor = world.spawn(());
        let first = world.spawn(UINode::default());
        world.insert(
            EditorHosts {
                canvas: Some(first),
                ..Default::default()
            },
            editor,
        );
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(editor);
        sync_workspace(&mut world);
        world.tick();
        **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(first);
        world.get_resource_mut::<UIInputState>().unwrap().captured = Some(first);
        world
            .get_resource_mut::<ViewportCommands>()
            .unwrap()
            .release_navigation = false;
        // Unrelated UI changes don't cause a full workspace refresh.
        world
            .get_component_for_entity_mut::<UINode>(first)
            .unwrap()
            .visible = false;
        sync_workspace(&mut world);
        assert!(
            !world
                .get_component_for_entity::<UINode>(first)
                .unwrap()
                .visible
        );
        assert_eq!(
            **world.get_resource::<FocusedWidget>().unwrap(),
            Some(first)
        );
        assert_eq!(
            world.get_resource::<UIInputState>().unwrap().captured,
            Some(first)
        );
        assert!(
            !world
                .get_resource::<ViewportCommands>()
                .unwrap()
                .release_navigation
        );

        let second = world.spawn(UINode {
            visible: false,
            ..Default::default()
        });
        world
            .get_component_for_entity_mut::<EditorHosts>(editor)
            .unwrap()
            .canvas = Some(second);
        sync_workspace(&mut world);
        assert!(
            world
                .get_component_for_entity::<UINode>(second)
                .unwrap()
                .visible
        );
        assert_eq!(
            **world.get_resource::<FocusedWidget>().unwrap(),
            Some(first)
        );

        world.tick();
        let mut deactivate = (|mut active: ResMut<ActiveEditor>| active.0 = None).into_system();
        deactivate.initialize(&mut world);
        deactivate.run_and_apply(&mut world);
        sync_workspace(&mut world);
        assert!(
            !world
                .get_component_for_entity::<UINode>(second)
                .unwrap()
                .visible
        );
        assert!(world.get_resource::<FocusedWidget>().unwrap().is_none());
        assert!(world
            .get_resource::<UIInputState>()
            .unwrap()
            .captured
            .is_none());
        assert!(
            world
                .get_resource::<ViewportCommands>()
                .unwrap()
                .release_navigation
        );
    }

    #[test]
    fn panel_changes_refresh_visibility_and_scene_resets_clear_input() {
        let mut world = World::new();
        resources(&mut world);
        let editor = world.spawn(());
        let host = world.spawn(UINode::default());
        world.insert(
            EditorHosts {
                canvas: Some(host),
                ..Default::default()
            },
            editor,
        );
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(editor);
        sync_workspace(&mut world);
        world.tick();
        world
            .get_component_for_entity_mut::<UINode>(host)
            .unwrap()
            .visible = false;
        let mut change_panels = (|mut panels: ResMut<PanelRegistry>| {
            panels.register(crate::dock::PanelDescriptor {
                id: "test",
                title: "Test",
                region: crate::dock::Region::Scene,
            })
        })
        .into_system();
        change_panels.initialize(&mut world);
        change_panels.run_and_apply(&mut world);
        sync_workspace(&mut world);
        assert!(
            world
                .get_component_for_entity::<UINode>(host)
                .unwrap()
                .visible
        );

        world.tick();
        **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(host);
        let mut reset =
            (|mut commands: ResMut<ViewportCommands>| commands.reset = true).into_system();
        reset.initialize(&mut world);
        reset.run_and_apply(&mut world);
        sync_workspace(&mut world);
        assert!(world.get_resource::<FocusedWidget>().unwrap().is_none());
        assert!(
            world
                .get_resource::<ViewportCommands>()
                .unwrap()
                .release_navigation
        );
    }

    #[test]
    fn contextual_hosts_follow_active_editor_and_are_removed_on_close() {
        let mut world = World::new();
        let mut registry = AssetEditorRegistry::default();
        registry.register::<Notes>(NotesEditor).unwrap();
        world.insert_resource(registry);
        world.insert_resource(crate::project::ProjectState::default());
        world.insert_resource(ActiveEditor::default());
        world.insert_resource(AssetEditorCommands::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(PanelRegistry::default());
        world.insert_resource(ViewportCommands::default());
        world.insert_resource(HoveredNode::default());
        world.insert_resource(UIInputState::default());
        world.insert_resource(ActionMap::default());
        world.insert_resource(HierarchyState::default());
        let asset = crate::project::AssetEntry {
            id: AssetId::new(),
            address: "notes.gasset".into(),
            kind: "Notes".into(),
            display_name: "notes".into(),
            folder: String::new(),
            provenance: ImportProvenance {
                source: "fixture".into(),
                sub_asset: "notes".into(),
            },
        };
        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open {
                asset,
                project_generation: 0,
            });
        process_editor_commands(&mut world);
        let editor = world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        let canvas = world.spawn((UINode::default(), EditorOwned(editor)));
        let left = world.spawn((UINode::default(), EditorOwned(editor)));
        let inspector = world.spawn((UINode::default(), EditorOwned(editor)));
        world.insert(
            EditorHosts {
                canvas: Some(canvas),
                left: Some(left),
                inspector: Some(inspector),
            },
            editor,
        );
        **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(left);
        sync_workspace(&mut world);
        assert!(world.get_resource::<FocusedWidget>().unwrap().is_none());
        for host in [canvas, left, inspector] {
            assert!(
                world
                    .get_component_for_entity::<UINode>(host)
                    .unwrap()
                    .visible
            );
        }
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = None;
        sync_workspace(&mut world);
        for host in [canvas, left, inspector] {
            assert!(
                !world
                    .get_component_for_entity::<UINode>(host)
                    .unwrap()
                    .visible
            );
        }
        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Close(editor));
        process_editor_commands(&mut world);
        for host in [canvas, left, inspector] {
            assert!(!world.entity_is_valid(host));
        }
    }
}
