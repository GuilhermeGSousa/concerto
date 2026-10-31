use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup, Update},
};
use concerto_ecs::{
    Component, Entity, IntoSystemConfig, Query, Res, ResMut, Resource, World,
    command::CommandQueue,
    component::bundle::IntoBundle,
    entity::{EntityStructuralVersion, hierarchy::Children},
    query::change_detection::DetectChanges,
    signal::{On, lifecycle::Remove},
};
use std::any::TypeId;

use concerto_reflect::{PathSegment, PropertyPath};
use concerto_ui::elements::prelude::*;
use concerto_ui::{
    interaction::{Interactable, UIDisabled},
    node::{UILayout, UINode, UIRect},
    scroll::{SCROLLBAR_WIDTH, UIScrollArea},
    theme::UITheme,
    transform::UIValue,
};
use taffy::FlexDirection;

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::hosted::{EditTarget, EditWorld, EditWorldSignals, HostedEventsAppExt};
use crate::tools::{edits::PropertyCommitted, scene::SceneRoot, selection::Selection};

mod add_component;
mod boolean;
mod color;
mod component_menu;
mod list;
mod name_field;
mod numeric;
mod registry;
mod rows;
mod sync;
mod target;
mod values;
mod variants;

pub use crate::tools::edits::{ComponentEdit, ComponentEdits};
use add_component::{
    add_highlighted_component, clear_add_component_search, populate_add_component_menu,
};
pub use name_field::{FocusNameField, NameField};
use name_field::{name_field, update_name_field};
pub use sync::{InspectedComponent, standard_row_node};
use sync::{build_property_widgets, sync_inspected_components};

use boolean::refresh_bool_fields;
use color::refresh_color_swatches;
use numeric::{
    cancel_numeric_fields, commit_numeric_fields, refresh_numeric_fields, refresh_numeric_labels,
};
use variants::{refresh_option_fields, refresh_variant_labels};

use crate::tools::edits::EditComponent;
pub(crate) use list::{ListEdit, ListEditor};
use registry::send_property_commits;
pub use registry::{EditableApp, ErasedEditor, InspectorRegistry};
pub(crate) use registry::{InspectionSource, current_editor};
pub use rows::{
    EditContext, EditError, PlainRow, Property, PropertyCommit, PropertyCommits, PropertyEditor,
    PropertyKind, PropertyRow, PropertyRowValue,
};
pub use target::{ComponentTarget, DocumentValue, PropertyAddress, PropertyTarget, TargetWorld};
pub use values::{
    CommitValue, InspectedSnapshots, InspectedValues, RefreshInspector, ValueCard, ValueSnapshot,
};
use values::{
    build_value_widgets, commit_value, read_inspected_values, refresh_inspected_values,
    refresh_on_new_app, send_value_commits, sync_value_cards,
};

pub const PANEL_ID: &str = "concerto.ecs";

/// Panel metadata. Component cards and property rows own inspection state in ECS.
#[derive(Resource, Debug, Default, PartialEq)]
pub struct InspectorData {
    /// The entity being inspected, in the edit `App`'s world.
    pub scene_entity: Option<Entity>,
    /// The selected scene root, in the edit `App`'s world, when the root is what is selected.
    pub closable_scene_root: Option<Entity>,
}

const PROPERTY_LABEL_WIDTH: f32 = 72.0;

fn label_for(path: &PropertyPath) -> String {
    let name = path
        .segments()
        .iter()
        .rev()
        .find(|segment| match segment {
            PathSegment::Field(name) => name.parse::<usize>().is_err(),
            PathSegment::Index(_) => true,
        })
        .map_or(path.name().to_string(), ToString::to_string)
        .split('_')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = name.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[derive(Component)]
struct DetailsView;

#[derive(Component, Debug, Default)]
struct ComponentStack {
    scene_target: Option<Entity>,
    structural_version: Option<EntityStructuralVersion>,
    registry_tick: Option<concerto_ecs::component::Tick>,
}

pub struct InspectorPlugin;

impl Plugin for InspectorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(InspectorData::default());
        app.insert_resource(InspectorRegistry::default());
        app.insert_resource(PropertyCommits::default());
        app.insert_resource(ComponentEdits::default());
        app.insert_resource(FocusNameField::default());
        app.insert_resource(InspectedSnapshots::default());
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Looking Glass",
            region: Region::Side,
        });
        app.forward_hosted_event::<PropertyCommitted>();
        install_listeners(app.main_mut().world_mut());
        app.add_system(Startup, build_panel);
        app.add_system(Update, send_property_commits)
            .add_system(Update, send_value_commits)
            .add_system(Update, send_component_edits);
        app.add_system(LateUpdate, commit_numeric_fields)
            .add_system(LateUpdate, cancel_numeric_fields)
            .add_system(LateUpdate, collect_inspector_data)
            .add_system(LateUpdate, update_name_field)
            .add_system(LateUpdate, sync_inspected_components)
            .add_system(LateUpdate, build_property_widgets)
            .add_system(
                LateUpdate,
                (
                    refresh_inspected_values,
                    sync_value_cards,
                    build_value_widgets,
                )
                    .chain(),
            )
            .add_system(LateUpdate, add_highlighted_component)
            .add_system(LateUpdate, clear_add_component_search)
            .add_system(LateUpdate, populate_add_component_menu)
            .add_system(LateUpdate, refresh_numeric_fields)
            .add_system(LateUpdate, refresh_numeric_labels)
            .add_system(LateUpdate, refresh_color_swatches)
            .add_system(LateUpdate, refresh_bool_fields)
            .add_system(LateUpdate, refresh_variant_labels)
            .add_system(LateUpdate, refresh_option_fields)
            .add_system(LateUpdate, sync_inspector_scroll);
    }
}

pub(crate) fn install_listeners(world: &mut World) {
    world.add_listener(forget_scene);
    world.add_listener(sync::forget_scene);
    world.add_listener(name_field::forget_scene);
    world.add_listener(add_component::forget_scene);
    world.add_listener(component_menu::forget_scene);
    world.add_listener(variants::forget_scene);
    world.add_listener(read_inspected_values);
    world.add_listener(refresh_on_new_app);
    world.add_listener(commit_value);
}

fn forget_scene(
    _: On<Remove<EditTarget>>,
    mut data: ResMut<InspectorData>,
    mut commits: ResMut<PropertyCommits>,
    mut edits: ResMut<ComponentEdits>,
) {
    if *data != InspectorData::default() {
        *data = InspectorData::default();
    }
    if commits.holds_for(TargetWorld::EditTarget) {
        commits.take_for(TargetWorld::EditTarget);
    }
    if !edits.0.is_empty() {
        edits.0.clear();
    }
}

fn send_component_edits(mut edits: ResMut<ComponentEdits>, mut scene: EditWorldSignals) {
    if edits.0.is_empty() {
        return;
    }
    for edit in std::mem::take(&mut edits.0) {
        scene.trigger(EditComponent(edit));
    }
}

pub(crate) fn collect_inspector_data(
    selection: EditWorld<Res<Selection>>,
    selected: EditWorld<Query<Option<&SceneRoot>>>,
    mut current: ResMut<InspectorData>,
) {
    let mut data = InspectorData::default();
    if let Some(entity) = selection.entity()
        && let Some(root) = selected.get_entity(entity)
    {
        data.scene_entity = root.is_none().then_some(entity);
        data.closable_scene_root = root.map(|_| entity);
    }
    if *current != data {
        *current = data;
    }
}

fn build_panel(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    if let Some(body) = registry.body(PANEL_ID) {
        spawn_panel(&mut cmd, body, &theme);
    }
}

pub fn spawn_panel(cmd: &mut CommandQueue, parent: Entity, theme: &UITheme) {
    cmd.entity(parent)
        .add_child_with(theme.column().grow().clipped(), |mut details| {
            details.add_child(name_field(theme)).add_child_with(
                (
                    UINode::default()
                        .with_flex_grow(1.0)
                        .with_flex_direction(FlexDirection::Column)
                        .clipped(),
                    Interactable,
                    DetailsView,
                ),
                |mut view| {
                    let stack = view
                        .spawn_child((
                            theme
                                .column()
                                .fixed()
                                .gap(theme.spacing_xs + 2.0)
                                .padding(UIRect {
                                    right: scrollbar_gutter(theme),
                                    ..UIRect::all(0.0)
                                }),
                            ComponentStack::default(),
                        ))
                        .entity();

                    view.insert(UIScrollArea {
                        content: Some(stack),
                        ..Default::default()
                    });
                },
            );
        });
}

fn scrollbar_gutter(theme: &UITheme) -> f32 {
    SCROLLBAR_WIDTH + theme.spacing_xs
}

fn sync_inspector_scroll(
    stacks: Query<(&ComponentStack, &UILayout)>,
    views: Query<(&DetailsView, &mut UIScrollArea, &UILayout)>,
    data: Res<InspectorData>,
) {
    let Some((_, mut area, view)) = views.iter().next() else {
        return;
    };

    if data.is_changed() && area.offset != 0.0 {
        area.offset = 0.0;
    }
    let Some((_, stack)) = stacks.iter().next() else {
        return;
    };

    let extent = stack.rect.size.y;
    if area.content_extent != extent {
        area.content_extent = extent;
    }
    let offset = area
        .offset
        .clamp(0.0, (extent - view.content_rect.size.y).max(0.0));
    if area.offset != offset {
        area.offset = offset;
    }
}

/// The systems that build the inspector's rows from the selection and send their edits.
#[cfg(test)]
pub(crate) fn row_systems() -> Vec<concerto_ecs::system::BoxedSystem> {
    use concerto_ecs::IntoSystem;
    vec![
        collect_inspector_data.into_boxed_system(),
        sync_inspected_components.into_boxed_system(),
        build_property_widgets.into_boxed_system(),
        send_property_commits.into_boxed_system(),
    ]
}

#[cfg(test)]
mod tests;
