//! Contextual panel hosts and input transitions for asset editors.
use concerto_ecs::{
    Changed, Entity, Query, Res, ResMut, Without,
    command::CommandQueue,
    entity::hierarchy::{ChildOf, Children},
    query::change_detection::DetectChanges,
    signal::{On, lifecycle::Remove},
};
use concerto_foundation::assets::Asset;
use concerto_ui::{
    elements::prelude::*,
    focus::FocusedWidget,
    interaction::{HoveredNode, Interactable, UIInputState},
    node::UINode,
    theme::UITheme,
    transform::UIValue,
};
use concerto_window::input::actions::ActionMap;

use crate::{
    asset_editor::{ActiveEditor, AssetEditorRegistry, EditorDocument, EditorHosts, EditorOwned},
    dock::{PanelRegistry, PanelState, PanelStates, Region, card},
    hosted::ViewportTarget,
    inspector::InspectedValues,
};

/// Build contextual containers for new documents.
pub(crate) fn create_editor_hosts(
    documents: Query<(Entity, &EditorDocument), Without<EditorHosts>>,
    registry: Res<PanelRegistry>,
    editors: Res<AssetEditorRegistry>,
    theme: Res<UITheme>,
    nodes: Query<(&UINode, &ChildOf)>,
    mut commands: CommandQueue,
) {
    let bases = panel_bodies(&registry);
    for (editor, doc) in documents.iter() {
        let mut hosts = [None; 3];
        let mut bottom = None;
        if doc.asset_type != concerto_scene::scene::Scene::name() {
            let previewing = editors.preview(doc.asset_type).is_some();
            let form = editors.form(doc.asset_type);
            for (index, base) in bases.iter().copied().enumerate() {
                if (index == 0 && previewing) || (index < 2 && form) {
                    continue;
                }
                let Some((node, parent)) = base.and_then(|base| nodes.get_entity(base)) else {
                    continue;
                };
                let mut node = node.clone();
                node.visible = false;
                let mut region = commands.entity(parent.parent());
                let host = if form {
                    region.spawn_child((
                        UINode::default()
                            .with_position(taffy::Position::Absolute)
                            .with_visible(false),
                        EditorOwned(editor),
                        Interactable,
                    ))
                } else {
                    region.spawn_child((
                        card(&theme).node(|_| node),
                        EditorOwned(editor),
                        Interactable,
                    ))
                }
                .entity();
                hosts[index] = Some(host);
            }
            if editors.bottom_host(doc.asset_type)
                && let Some(strip) = registry.region(Region::Bottom)
            {
                let surface = card(&theme)
                    .width(UIValue::Px(0.0))
                    .min_width(UIValue::Px(0.0))
                    .grow()
                    .hidden();
                let host = commands
                    .entity(strip)
                    .spawn_child((surface, EditorOwned(editor), Interactable))
                    .entity();
                bottom = Some(host);
            }
        }
        commands.insert(
            EditorHosts {
                canvas: hosts[0],
                left: hosts[1],
                inspector: hosts[2],
                bottom,
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

/// Update visibility, and which regions collapse or fill, only when tab/panel resources or host components change.
pub(crate) fn sync_workspace(
    active: Res<ActiveEditor>,
    registry: Res<PanelRegistry>,
    mut states: ResMut<PanelStates>,
    editors: Res<AssetEditorRegistry>,
    documents: Query<&EditorDocument>,
    hosts: Query<(Entity, &EditorHosts)>,
    changed_hosts: Query<&EditorHosts, Changed<EditorHosts>>,
    values: Query<&InspectedValues>,
    changed_values: Query<&InspectedValues, Changed<InspectedValues>>,
    nodes: Query<&mut UINode>,
    parents: Query<&ChildOf>,
    children: Query<&mut Children>,
) {
    let hosts_changed = changed_hosts.iter().next().is_some();
    let values_changed = changed_values.iter().next().is_some();
    if !active.is_changed() && !registry.is_changed() && !hosts_changed && !values_changed {
        return;
    }
    let document = active.0.and_then(|entity| documents.get_entity(entity));
    let scene = document.is_none_or(|doc| doc.asset_type == concerto_scene::scene::Scene::name());
    let viewport = scene || document.is_some_and(|doc| editors.preview(doc.asset_type).is_some());
    let inspected = active
        .0
        .is_some_and(|entity| values.get_entity(entity).is_some());
    let form = document.is_some_and(|doc| editors.form(doc.asset_type));
    if let Some(centre) = registry.region(Region::Centre) {
        set_visible(&nodes, centre, !form);
    }
    let bases = panel_bodies(&registry);
    for (entity, visible) in bases.into_iter().zip([viewport, scene, scene || inspected]) {
        if let Some(entity) = entity {
            set_visible(&nodes, entity, visible);
        }
    }
    let front = active
        .0
        .and_then(|entity| hosts.get_entity(entity))
        .map(|(_, hosts)| *hosts)
        .unwrap_or_default();
    let side = |panel: bool, host: Option<Entity>| {
        if panel || host.is_some() {
            PanelState::Sized
        } else {
            PanelState::Hidden
        }
    };
    let right = side(bases[2].is_some() && (scene || inspected), front.inspector);
    let next = PanelStates {
        left: side(bases[1].is_some() && scene, front.left),
        right: if form && right == PanelState::Sized {
            PanelState::Fill
        } else {
            right
        },
        bottom: PanelState::Sized,
        browser: if front.bottom.is_some() {
            PanelState::Sized
        } else {
            PanelState::Fill
        },
    };
    if *states != next {
        *states = next;
    }
    for (entity, hosts) in hosts.iter() {
        for host in [hosts.canvas, hosts.left, hosts.inspector, hosts.bottom]
            .into_iter()
            .flatten()
        {
            set_visible(&nodes, host, active.0 == Some(entity));
        }
        if let Some(host) = hosts.inspector
            && values.get_entity(entity).is_some()
            && let Some(mut node) = nodes.get_entity(host)
            && (node.flex_grow != 0.0 || node.flex_shrink != 0.0)
        {
            node.flex_grow = 0.0;
            node.flex_shrink = 0.0;
        }
    }
    if hosts_changed || registry.is_changed() {
        if let Some(base) = bases[1] {
            let contextual: Vec<_> = hosts.iter().filter_map(|(_, h)| h.left).collect();
            sort_siblings(&parents, &children, base, |entity| {
                usize::from(entity != base && !contextual.contains(&entity))
            });
        }
        if let Some(base) = bases[2] {
            let contextual: Vec<_> = hosts.iter().filter_map(|(_, h)| h.inspector).collect();
            sort_siblings(&parents, &children, base, |entity| {
                if contextual.contains(&entity) {
                    0
                } else {
                    usize::from(entity != base) + 1
                }
            });
        }
    }
}

fn sort_siblings(
    parents: &Query<&ChildOf>,
    children: &Query<&mut Children>,
    entity: Entity,
    key: impl FnMut(Entity) -> usize,
) {
    if let Some(mut siblings) = parents
        .get_entity(entity)
        .and_then(|parent| children.get_entity(parent.parent()))
    {
        siblings.sort_by_key(key);
    }
}

fn set_visible(nodes: &Query<&mut UINode>, entity: Entity, visible: bool) {
    if let Some(mut node) = nodes.get_entity(entity)
        && node.visible != visible
    {
        node.visible = visible;
    }
}

pub(crate) fn reset_workspace_input(
    active: Res<ActiveEditor>,
    mut focus: ResMut<FocusedWidget>,
    mut hovered: ResMut<HoveredNode>,
    mut input: ResMut<UIInputState>,
    mut actions: ResMut<ActionMap>,
) {
    if active.is_changed() {
        clear_transient_input(&mut focus, &mut hovered, &mut input, &mut actions);
    }
}

pub(crate) fn reset_input_for_viewport_target(
    _: On<Remove<ViewportTarget>>,
    mut focus: ResMut<FocusedWidget>,
    mut hovered: ResMut<HoveredNode>,
    mut input: ResMut<UIInputState>,
    mut actions: ResMut<ActionMap>,
) {
    clear_transient_input(&mut focus, &mut hovered, &mut input, &mut actions);
}

fn clear_transient_input(
    focus: &mut FocusedWidget,
    hovered: &mut HoveredNode,
    input: &mut UIInputState,
    actions: &mut ActionMap,
) {
    **focus = None;
    **hovered = None;
    *input = UIInputState::default();
    actions.pop_context(crate::actions::TreeContext);
    actions.pop_context(crate::actions::ViewportContext);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_editor::{
        AssetEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry,
    };
    use concerto_ecs::{IntoSystem, System, World};
    use concerto_foundation::assets::{
        AssetId,
        registry::{AssetEntry, AssetRegistry},
    };
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct Notes;
    impl Asset for Notes {
        type EditorOnly = ();

        fn name() -> &'static str {
            "Notes"
        }
    }
    #[derive(Default)]
    struct NotesEditor;
    impl AssetEditor for NotesEditor {}

    fn run<M>(world: &mut World, system: impl IntoSystem<(), M>) {
        let mut system = system.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }

    #[test]
    fn a_documents_hosts_show_while_it_is_in_front_and_go_when_it_closes() {
        let mut world = World::new();
        let mut registry = AssetEditorRegistry::default();
        registry.register::<Notes, NotesEditor>().unwrap();
        world.insert_resource(registry);
        world.insert_resource(ActiveEditor::default());
        world.insert_resource(AssetEditorCommands::default());
        world.insert_resource(crate::guard::UnsavedGuard::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(PanelRegistry::default());
        world.insert_resource(PanelStates::default());
        world.insert_resource(HoveredNode::default());
        world.insert_resource(UIInputState::default());
        world.insert_resource(ActionMap::default());
        let asset = AssetEntry {
            embedded_in: None,
            id: AssetId::new(),
            address: "notes.gasset".into(),
            kind: "Notes".into(),
            kind_version: None,
            provenance: None,
        };
        let mut content = AssetRegistry::default();
        content.insert(asset.clone());
        world.insert_resource(content);
        let mut project = crate::project::ProjectState::default();
        project.project = Some(crate::project::Project {
            root: std::path::PathBuf::from("project"),
            content_root: "content".into(),
            default_scene: None,
        });
        world.insert_resource(project);
        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open(asset));
        run(&mut world, crate::asset_editor::process_editor_commands);
        let editor = world.get_resource::<ActiveEditor>().unwrap().0.unwrap();
        let canvas = world.spawn((UINode::default(), EditorOwned(editor)));
        let left = world.spawn((UINode::default(), EditorOwned(editor)));
        let inspector = world.spawn((UINode::default(), EditorOwned(editor)));
        world.insert(
            EditorHosts {
                canvas: Some(canvas),
                left: Some(left),
                inspector: Some(inspector),
                ..Default::default()
            },
            editor,
        );
        let mut sync = sync_workspace.into_system();
        sync.initialize(&mut world);
        let shown = |world: &World| {
            [canvas, left, inspector].map(|host| {
                world
                    .get_component_for_entity::<UINode>(host)
                    .unwrap()
                    .visible
            })
        };

        sync.run_and_apply((), &mut world);
        assert_eq!(shown(&world), [true; 3]);

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = None;
        sync.run_and_apply((), &mut world);
        assert_eq!(shown(&world), [false; 3]);

        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Close(editor));
        run(&mut world, crate::asset_editor::process_editor_commands);
        for host in [canvas, left, inspector] {
            assert!(!world.entity_is_valid(host));
        }
    }
}
