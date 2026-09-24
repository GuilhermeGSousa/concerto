use concerto_app::{
    App, Plugin,
    schedule_groups::{LateUpdate, Startup, Update},
};
use concerto_ecs::{
    Component, Entity, Query, Res, ResMut, Resource,
    command::CommandQueue,
    entity::{EntityStructuralVersion, hierarchy::Children},
};
use std::any::TypeId;

use concerto_editable::PropertyPath;
use concerto_ui::{
    interaction::{Interactable, UIDisabled},
    material::UIMaterial,
    node::{UILayout, UINode, UIRect},
    scroll::UIScrollArea,
    text::TextComponent,
    theme::UITheme,
    transform::UIValue,
};
use taffy::FlexDirection;

use crate::dock::{DockedApp, PanelDescriptor, PanelRegistry, Region};
use crate::scene::SceneRoot;
use crate::selection::Selection;

mod numeric;
mod registry;
mod rows;
mod sync;

pub use sync::InspectedComponent;
use sync::{build_property_widgets, sync_inspected_components};

use concerto_foundation::transform::Transform;
use numeric::{
    cancel_numeric_fields, commit_numeric_fields, refresh_numeric_fields,
    select_numeric_field_on_focus,
};

pub use registry::{EditableApp, InspectorRegistry, apply_property_commit, apply_property_commits};
pub use rows::{
    EditError, Property, PropertyCommit, PropertyCommits, PropertyEditor, PropertyRow,
    PropertyRowValue,
};

pub const PANEL_ID: &str = "concerto.ecs";

/// Panel metadata. Component cards and property rows own inspection state in ECS.
#[derive(Resource, Default, PartialEq)]
pub struct InspectorData {
    pub entity: Option<Entity>,
    pub closable_scene: Option<Entity>,
}

const PROPERTY_LABEL_WIDTH: f32 = 72.0;

fn label_for(path: &PropertyPath) -> String {
    let mut chars = path.name().chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[derive(Component)]
struct DetailsView;

/// The scrolling column that holds one card per component.
#[derive(Component, Debug, Default)]
struct ComponentStack {
    target: Option<Entity>,
    structural_version: Option<EntityStructuralVersion>,
    registry_tick: Option<concerto_ecs::component::Tick>,
}

pub struct InspectorPlugin;

impl Plugin for InspectorPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(InspectorData::default());
        app.insert_resource(InspectorScroll::default());
        app.insert_resource(InspectorRegistry::default());
        app.insert_resource(PropertyCommits::default());
        app.register_editable::<Transform>();
        app.add_panel(PanelDescriptor {
            id: PANEL_ID,
            title: "Looking Glass",
            region: Region::Side,
        });
        app.add_system(Startup, build_panel);
        app.add_system(Update, apply_property_commits);
        app.add_system(LateUpdate, select_numeric_field_on_focus)
            .add_system(LateUpdate, commit_numeric_fields)
            .add_system(LateUpdate, cancel_numeric_fields)
            .add_system(LateUpdate, collect_inspector_data)
            .add_system(LateUpdate, sync_inspected_components)
            .add_system(LateUpdate, build_property_widgets)
            .add_system(LateUpdate, refresh_numeric_fields)
            .add_system(LateUpdate, sync_inspector_scroll);
    }
}

fn collect_inspector_data(
    selection: Res<Selection>,
    selected: Query<Option<&SceneRoot>>,
    mut current: ResMut<InspectorData>,
) {
    let mut data = InspectorData::default();
    if let Some(entity) = selection.entity() {
        if let Some(root) = selected.get_entity(entity) {
            data.entity = Some(entity);
            data.closable_scene = root.map(|_| entity);
        }
    }
    if *current != data {
        *current = data;
    }
}

fn text(theme: &UITheme, value: &str) -> TextComponent {
    TextComponent {
        text: value.into(),
        font_size: theme.font_size_md,
        line_height: theme.line_height(theme.font_size_md),
        ..Default::default()
    }
}

fn build_panel(mut cmd: CommandQueue, registry: Res<PanelRegistry>, theme: Res<UITheme>) {
    if let Some(body) = registry.body(PANEL_ID) {
        spawn_panel(&mut cmd, body, &theme);
    }
}

pub fn spawn_panel(cmd: &mut CommandQueue, parent: Entity, theme: &UITheme) {
    let details = cmd
        .spawn(
            UINode {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                gap: glam::Vec2::new(0.0, theme.spacing_sm),
                ..Default::default()
            }
            .clipped(),
        )
        .entity();
    cmd.add_child(parent, details);

    // Components are unbounded, so the stack scrolls rather than pushing the
    // close button off the card.
    let view = cmd
        .spawn((
            UINode {
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                ..Default::default()
            }
            .clipped(),
            Interactable,
            DetailsView,
        ))
        .entity();
    cmd.add_child(details, view);

    let stack = cmd
        .spawn((
            UINode {
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Column,
                gap: glam::Vec2::new(0.0, theme.spacing_xs + 2.0),
                ..Default::default()
            },
            ComponentStack::default(),
        ))
        .entity();
    cmd.add_child(view, stack);
    cmd.insert(
        UIScrollArea {
            content: Some(stack),
            ..Default::default()
        },
        view,
    );
}

fn sync_inspector_scroll(
    stacks: Query<(&ComponentStack, &UILayout)>,
    views: Query<(&DetailsView, &mut UIScrollArea, &UILayout)>,
    data: Res<InspectorData>,
    mut shown: ResMut<InspectorScroll>,
) {
    let Some((_, mut area, view)) = views.iter().next() else {
        return;
    };

    if shown.changed_tick() != data.changed_tick() {
        shown.mark_changed();
        area.offset = 0.0;
    }
    let Some((_, stack)) = stacks.iter().next() else {
        return;
    };

    let extent = stack.rect.size.y;
    area.content_extent = extent;
    area.offset = area
        .offset
        .clamp(0.0, (extent - view.content_rect.size.y).max(0.0));
}

#[derive(Resource, Default)]
pub struct InspectorScroll;

#[cfg(test)]
mod tests;
