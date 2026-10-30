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
    dock::PanelRegistry,
    hosted::ViewportTarget,
    shell::CHATTER_ID,
};

const BOTTOM_HOST_HEIGHT: f32 = 132.0;

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
            for (index, base) in bases.iter().copied().enumerate() {
                if index == 0 && previewing {
                    continue;
                }
                let Some((node, parent)) = base.and_then(|base| nodes.get_entity(base)) else {
                    continue;
                };
                let mut node = node.clone();
                node.visible = false;
                let surface = if index == 0 {
                    theme.canvas()
                } else {
                    theme.panel()
                };
                let host = commands
                    .entity(parent.parent())
                    .spawn_child((surface.node(|_| node), EditorOwned(editor), Interactable))
                    .entity();
                hosts[index] = Some(host);
            }
            if editors.bottom_host(doc.asset_type)
                && let Some((_, chatter)) = registry
                    .body(CHATTER_ID)
                    .and_then(|chatter| nodes.get_entity(chatter))
            {
                let surface = theme
                    .panel()
                    .height(UIValue::Px(BOTTOM_HOST_HEIGHT))
                    .fixed()
                    .hidden();
                let host = commands
                    .entity(chatter.parent())
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

/// Update visibility only when tab/panel resources or host components change.
pub(crate) fn sync_workspace(
    active: Res<ActiveEditor>,
    registry: Res<PanelRegistry>,
    editors: Res<AssetEditorRegistry>,
    documents: Query<&EditorDocument>,
    hosts: Query<(Entity, &EditorHosts)>,
    changed_hosts: Query<&EditorHosts, Changed<EditorHosts>>,
    nodes: Query<&mut UINode>,
    parents: Query<&ChildOf>,
    children: Query<&mut Children>,
) {
    let hosts_changed = changed_hosts.iter().next().is_some();
    if !active.is_changed() && !registry.is_changed() && !hosts_changed {
        return;
    }
    let document = active.0.and_then(|entity| documents.get_entity(entity));
    let scene = document.is_none_or(|doc| doc.asset_type == concerto_scene::scene::Scene::name());
    let viewport = scene || document.is_some_and(|doc| editors.preview(doc.asset_type).is_some());
    let bases = panel_bodies(&registry);
    for (index, entity) in bases.into_iter().enumerate() {
        if let Some(entity) = entity {
            set_visible(&nodes, entity, if index == 0 { viewport } else { scene });
        }
    }
    for (entity, hosts) in hosts.iter() {
        for host in [hosts.canvas, hosts.left, hosts.inspector, hosts.bottom]
            .into_iter()
            .flatten()
        {
            set_visible(&nodes, host, active.0 == Some(entity));
        }
    }
    if hosts_changed || registry.is_changed() {
        if let Some(base) = bases[1] {
            let contextual: Vec<_> = hosts.iter().filter_map(|(_, h)| h.left).collect();
            sort_siblings(&parents, &children, base, |entity| {
                usize::from(entity != base && !contextual.contains(&entity))
            });
        }
        if let Some(chatter) = registry.body(CHATTER_ID) {
            let contextual: Vec<_> = hosts.iter().filter_map(|(_, h)| h.bottom).collect();
            sort_siblings(&parents, &children, chatter, |entity| {
                usize::from(!contextual.contains(&entity))
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
    use concerto_ecs::{IntoSystem, Resource, System, World};
    use concerto_window::input::MouseButton;
    fn process_editor_commands(world: &mut World) {
        let mut system = crate::asset_editor::process_editor_commands.into_system();
        system.initialize(world);
        system.run_and_apply((), world);
    }
    #[derive(Resource)]
    struct WorkspaceSystems([concerto_ecs::system::BoxedSystem; 2]);

    fn sync_workspace(world: &mut World) {
        let mut systems = world
            .remove_resource::<WorkspaceSystems>()
            .unwrap_or_else(|| {
                let mut systems = [
                    super::sync_workspace.into_boxed_system(),
                    super::reset_workspace_input.into_boxed_system(),
                ];
                for system in &mut systems {
                    system.initialize(world);
                }
                WorkspaceSystems(systems)
            });
        for system in &mut systems.0 {
            system.run_and_apply((), world);
        }
        world.insert_resource(systems);
    }
    use crate::asset_editor::{
        AssetEditor, AssetEditorCommand, AssetEditorCommands, AssetEditorRegistry,
    };
    use concerto_foundation::assets::{Asset, AssetId, content::ImportProvenance};
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct Notes;
    impl Asset for Notes {
        fn name() -> &'static str {
            "Notes"
        }
    }
    #[derive(Default)]
    struct NotesEditor;
    impl AssetEditor for NotesEditor {}

    #[derive(Serialize, Deserialize)]
    struct Take;
    impl Asset for Take {
        fn name() -> &'static str {
            "Take"
        }
    }
    #[derive(Default)]
    struct TakeEditor;
    impl AssetEditor for TakeEditor {
        fn bottom_host(&self) -> bool {
            true
        }
    }

    fn resources(world: &mut World) {
        world.insert_resource(ActiveEditor::default());
        world.insert_resource(AssetEditorCommands::default());
        world.insert_resource(crate::guard::UnsavedGuard::default());
        world.insert_resource(AssetEditorRegistry::default());
        world.insert_resource(crate::project::ProjectState::default());
        world.insert_resource(PanelRegistry::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(HoveredNode::default());
        world.insert_resource(UIInputState::default());
        world.insert_resource(ActionMap::default());
    }

    #[test]
    fn workspace_systems_do_not_request_exclusive_world_access() {
        for system in [
            super::create_editor_hosts.into_boxed_system(),
            super::sync_workspace.into_boxed_system(),
            super::reset_workspace_input.into_boxed_system(),
        ] {
            let mut meta = concerto_ecs::system::meta::SystemMetadata::default();
            let mut access = concerto_ecs::system::access::SystemAccess::default();
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
        world
            .get_resource_mut::<UIInputState>()
            .unwrap()
            .set_captured(MouseButton::Left, Some(first));
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
            world
                .get_resource::<UIInputState>()
                .unwrap()
                .captured(MouseButton::Left),
            Some(first)
        );

        let second = world.spawn(UINode::default().with_visible(false));
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
        deactivate.run_and_apply((), &mut world);
        sync_workspace(&mut world);
        assert!(
            !world
                .get_component_for_entity::<UINode>(second)
                .unwrap()
                .visible
        );
        assert!(world.get_resource::<FocusedWidget>().unwrap().is_none());
        assert!(
            world
                .get_resource::<UIInputState>()
                .unwrap()
                .captured(MouseButton::Left)
                .is_none()
        );
    }

    #[test]
    fn panel_changes_refresh_visibility_and_a_new_viewport_target_clears_input() {
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
        change_panels.run_and_apply((), &mut world);
        sync_workspace(&mut world);
        assert!(
            world
                .get_component_for_entity::<UINode>(host)
                .unwrap()
                .visible
        );

        world.tick();
        **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(host);
        **world.get_resource_mut::<HoveredNode>().unwrap() = Some(host);
        world
            .get_resource_mut::<UIInputState>()
            .unwrap()
            .set_captured(MouseButton::Left, Some(host));
        world
            .get_resource_mut::<ActionMap>()
            .unwrap()
            .push_context(crate::actions::ViewportContext);
        sync_workspace(&mut world);
        assert_eq!(**world.get_resource::<FocusedWidget>().unwrap(), Some(host));

        world.add_listener(reset_input_for_viewport_target);
        let target = world.spawn(ViewportTarget);
        world.despawn(target);

        assert!(world.get_resource::<FocusedWidget>().unwrap().is_none());
        assert!(world.get_resource::<HoveredNode>().unwrap().is_none());
        assert!(
            world
                .get_resource::<UIInputState>()
                .unwrap()
                .captured(MouseButton::Left)
                .is_none()
        );
        assert!(
            !world
                .get_resource::<ActionMap>()
                .unwrap()
                .is_active(crate::actions::ViewportContext)
        );
    }

    fn hold_input(world: &mut World, node: Entity) {
        **world.get_resource_mut::<FocusedWidget>().unwrap() = Some(node);
        world
            .get_resource_mut::<UIInputState>()
            .unwrap()
            .set_captured(MouseButton::Left, Some(node));
    }

    fn input_is_clear(world: &World) -> bool {
        world.get_resource::<FocusedWidget>().unwrap().is_none()
            && world
                .get_resource::<UIInputState>()
                .unwrap()
                .captured(MouseButton::Left)
                .is_none()
    }

    #[test]
    fn retargeting_a_front_preview_clears_input() {
        use crate::hosted::{SetEditTarget, testing::*};
        let mut world = World::new();
        resources(&mut world);
        world.insert_resource(crate::hosted::input::Routing::default());
        crate::hosted::install_listeners(&mut world);
        world.add_listener(reset_input_for_viewport_target);
        let preview = world.spawn(preview_now(plain_recipe));
        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(preview);
        world.trigger(SetEditTarget(Some(preview)));
        let node = world.spawn(UINode::default());
        hold_input(&mut world, node);

        world.trigger(SetEditTarget(Some(preview)));

        assert!(input_is_clear(&world));
    }

    fn document_of(world: &mut World, asset_type: &'static str) -> Entity {
        world.spawn(EditorDocument {
            asset_type,
            title: asset_type.into(),
            current: None,
            pending: None,
            order: 0,
            status: String::new(),
            failed: None,
        })
    }

    fn docked_world() -> World {
        use crate::dock::{DockPlugin, DockedApp, PanelDescriptor, Region};
        let mut app = concerto_app::App::new();
        let world = app.main_mut().world_mut();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        app.register_plugin(concerto_app::main_schedule::MainSchedulePlugin)
            .insert_resource(concerto_window::plugin::Window::new((1280, 720), 1.0))
            .insert_resource(UITheme::default())
            .insert_resource(PanelRegistry::default())
            .register_plugin(DockPlugin);
        for (id, region) in [
            (crate::viewport::PANEL_ID, Region::Scene),
            (crate::hierarchy::PANEL_ID, Region::Rail),
            (crate::inspector::PANEL_ID, Region::Side),
            (crate::shell::CHATTER_ID, Region::Foot),
        ] {
            app.add_panel(PanelDescriptor {
                id,
                title: id,
                region,
            });
        }
        app.finish_plugin_build();
        std::mem::take(app.main_mut().world_mut())
    }

    fn visible(world: &World, node: Option<Entity>) -> bool {
        world
            .get_component_for_entity::<UINode>(node.unwrap())
            .unwrap()
            .visible
    }

    fn foot_children(world: &World, chatter: Entity) -> Vec<Entity> {
        let region = world
            .get_component_for_entity::<ChildOf>(chatter)
            .unwrap()
            .parent();
        world
            .get_component_for_entity::<Children>(region)
            .unwrap()
            .iter()
            .copied()
            .collect()
    }

    #[test]
    fn a_bottom_host_sits_above_chatter_follows_its_document_and_closes_with_it() {
        let mut world = docked_world();
        world.insert_resource(ActiveEditor::default());
        world.insert_resource(AssetEditorCommands::default());
        world.insert_resource(crate::guard::UnsavedGuard::default());
        world.insert_resource(concerto_foundation::assets::registry::AssetRegistry::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(HoveredNode::default());
        world.insert_resource(UIInputState::default());
        world.insert_resource(ActionMap::default());
        let mut editors = AssetEditorRegistry::default();
        editors.register::<Take, TakeEditor>().unwrap();
        editors.register::<Notes, NotesEditor>().unwrap();
        world.insert_resource(editors);
        let chatter = world
            .get_resource::<PanelRegistry>()
            .unwrap()
            .body(crate::shell::CHATTER_ID)
            .unwrap();
        let take = document_of(&mut world, Take::name());
        let notes = document_of(&mut world, Notes::name());
        let scene = document_of(&mut world, concerto_scene::scene::Scene::name());
        let mut create = super::create_editor_hosts.into_system();
        create.initialize(&mut world);
        create.run_and_apply((), &mut world);

        let hosts = *world.get_component_for_entity::<EditorHosts>(take).unwrap();
        let bottom = hosts.bottom.unwrap();
        for document in [notes, scene] {
            assert!(
                world
                    .get_component_for_entity::<EditorHosts>(document)
                    .unwrap()
                    .bottom
                    .is_none()
            );
        }
        assert_eq!(
            world.get_component_for_entity::<EditorOwned>(bottom),
            Some(&EditorOwned(take))
        );
        assert!(
            world
                .get_component_for_entity::<Interactable>(bottom)
                .is_some()
        );
        assert_eq!(
            world
                .get_component_for_entity::<UINode>(bottom)
                .unwrap()
                .height,
            UIValue::Px(132.0)
        );
        assert!(!visible(&world, Some(bottom)));

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(take);
        sync_workspace(&mut world);
        assert_eq!(foot_children(&world, chatter), vec![bottom, chatter]);
        assert!(visible(&world, Some(bottom)) && visible(&world, Some(chatter)));

        for other in [notes, scene] {
            world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(other);
            sync_workspace(&mut world);
            assert!(!visible(&world, Some(bottom)) && visible(&world, Some(chatter)));
        }

        let second = document_of(&mut world, Take::name());
        create.run_and_apply((), &mut world);
        let second_bottom = world
            .get_component_for_entity::<EditorHosts>(second)
            .unwrap()
            .bottom
            .unwrap();
        sync_workspace(&mut world);
        assert_eq!(
            foot_children(&world, chatter),
            vec![bottom, second_bottom, chatter]
        );

        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Close(take));
        process_editor_commands(&mut world);
        sync_workspace(&mut world);
        assert!(!world.entity_is_valid(bottom));
        assert_eq!(foot_children(&world, chatter), vec![second_bottom, chatter]);
    }

    #[test]
    fn a_preview_tab_keeps_the_shared_viewport_in_place_of_a_canvas_host() {
        use crate::hosted::{
            EditTarget, SetEditTarget,
            input::{InViewport, Routing, ViewportShown, ViewportView, sync_in_viewport},
            testing::*,
        };
        let mut world = docked_world();
        world.insert_resource(ActiveEditor::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(HoveredNode::default());
        world.insert_resource(UIInputState::default());
        world.insert_resource(ActionMap::default());
        world.insert_resource(Routing::default());
        let mut editors = AssetEditorRegistry::default();
        editors
            .register::<PreviewedAsset, PreviewedEditor>()
            .unwrap();
        editors.register::<PlainAsset, PlainEditor>().unwrap();
        world.insert_resource(editors);
        crate::hosted::install_listeners(&mut world);
        let bases = panel_bodies(world.get_resource::<PanelRegistry>().unwrap());
        let preview = document_of(&mut world, PreviewedAsset::name());
        world.insert(preview_now(plain_recipe), preview);
        let plain = document_of(&mut world, PlainAsset::name());
        let mut create = super::create_editor_hosts.into_system();
        create.initialize(&mut world);
        create.run_and_apply((), &mut world);

        let hosts = *world
            .get_component_for_entity::<EditorHosts>(preview)
            .unwrap();
        assert!(hosts.canvas.is_none());
        assert!(hosts.left.is_some() && hosts.inspector.is_some());
        assert!(
            world
                .get_component_for_entity::<EditorHosts>(plain)
                .unwrap()
                .canvas
                .is_some()
        );

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(plain);
        sync_workspace(&mut world);
        assert!(!visible(&world, bases[0]));

        world.get_resource_mut::<ActiveEditor>().unwrap().0 = Some(preview);
        world.trigger(SetEditTarget(Some(preview)));
        sync_workspace(&mut world);
        assert!(visible(&world, bases[0]));
        assert!(!visible(&world, bases[1]) && !visible(&world, bases[2]));
        assert!(visible(&world, hosts.left) && visible(&world, hosts.inspector));

        world.insert_resource(ViewportShown(Some(ViewportView {
            origin: glam::Vec2::ZERO,
            size: (640, 480),
            over: false,
            typing: false,
        })));
        let mut sync = sync_in_viewport.into_system();
        sync.initialize(&mut world);
        sync.run_and_apply((), &mut world);
        assert!(
            world
                .get_component_for_entity::<InViewport>(preview)
                .is_some()
        );
        assert!(
            world
                .get_component_for_entity::<ViewportTarget>(preview)
                .is_some()
        );
        assert!(
            world
                .get_component_for_entity::<EditTarget>(preview)
                .is_none()
        );
    }

    #[test]
    fn contextual_hosts_follow_active_editor_and_are_removed_on_close() {
        let mut world = World::new();
        let mut registry = AssetEditorRegistry::default();
        registry.register::<Notes, NotesEditor>().unwrap();
        world.insert_resource(registry);
        world.insert_resource(crate::project::ProjectState::default());
        world.insert_resource(ActiveEditor::default());
        world.insert_resource(AssetEditorCommands::default());
        world.insert_resource(crate::guard::UnsavedGuard::default());
        world.insert_resource(FocusedWidget::default());
        world.insert_resource(PanelRegistry::default());
        world.insert_resource(HoveredNode::default());
        world.insert_resource(UIInputState::default());
        world.insert_resource(ActionMap::default());
        let asset = concerto_foundation::assets::registry::AssetEntry {
            id: AssetId::new(),
            address: "notes.gasset".into(),
            kind: "Notes".into(),
            provenance: Some(ImportProvenance {
                source: "fixture".into(),
                sub_asset: "notes".into(),
            }),
        };
        let mut content = concerto_foundation::assets::registry::AssetRegistry::default();
        content.insert(asset.clone());
        world.insert_resource(content);
        world
            .get_resource_mut::<crate::project::ProjectState>()
            .unwrap()
            .project = Some(crate::project::Project {
            root: std::path::PathBuf::from("project"),
            content_root: "content".into(),
        });
        world
            .get_resource_mut::<AssetEditorCommands>()
            .unwrap()
            .0
            .push_back(AssetEditorCommand::Open(asset));
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
                ..Default::default()
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
