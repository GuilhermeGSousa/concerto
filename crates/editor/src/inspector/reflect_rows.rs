use super::*;

use super::sync::{
    BuildPropertyWidget, CardRows, LabelColumn, body_node, build_row_widgets, show_properties,
};
use concerto_ecs::{Changed, entity::hierarchy::ChildOf, reflect::AppTypeRegistry};
use concerto_reflect::Reflect;
use std::sync::Arc;

const NAME: &str = "Value";
const EMPTY: &str = "No settings";

/// Rows for a reflected value on a UI entity.
///
/// Their edits change `value`, so `Changed<ReflectRows>` tells the owner.
#[derive(Component)]
pub struct ReflectRows {
    /// The value the rows show, written in place by their edits.
    pub value: Box<dyn Reflect>,
    /// The registry `value`'s type and the types of its fields are found in.
    pub types: AppTypeRegistry,
}

/// Shows each [`ReflectRows`] and applies its edits; register it after [`InspectorPlugin`].
pub struct ReflectRowsPlugin;

impl Plugin for ReflectRowsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(
            LateUpdate,
            (sync_reflect_rows, build_reflect_rows_widgets).chain(),
        );
    }
}

#[derive(Component)]
struct RowsBody {
    body: Entity,
    target: Arc<dyn PropertyTarget>,
}

struct RowsTarget {
    entity: Entity,
}

impl PropertyTarget for RowsTarget {
    fn name(&self) -> String {
        NAME.into()
    }

    fn world(&self) -> TargetWorld {
        TargetWorld::Editor
    }

    fn read(&self, world: &World, read: &mut dyn FnMut(&dyn Reflect)) -> bool {
        let Some(rows) = world.get_component_for_entity::<ReflectRows>(self.entity) else {
            return false;
        };
        read(rows.value.as_ref());
        true
    }

    fn edit(&self, world: &mut World, edit: &mut dyn FnMut(&mut dyn Reflect)) -> bool {
        let Some(rows) = world.get_component_for_entity_mut::<ReflectRows>(self.entity) else {
            return false;
        };
        edit(rows.value.as_mut());
        true
    }

    fn types(&self, world: &World) -> Option<AppTypeRegistry> {
        world
            .get_component_for_entity::<ReflectRows>(self.entity)
            .map(|rows| rows.types.clone())
    }

    fn saved(&self) -> bool {
        false
    }
}

fn sync_reflect_rows(
    widgets: Query<(Entity, &ReflectRows, Option<&RowsBody>)>,
    changed: Query<Entity, Changed<ReflectRows>>,
    registry: Res<InspectorRegistry>,
    theme: Res<UITheme>,
    bodies: Query<&Children>,
    rows: Query<(
        Option<&PropertyRow>,
        Option<&PlainRow>,
        Option<&PropertyRowValue>,
    )>,
    mut cmd: CommandQueue,
) {
    for (entity, widget, shown) in widgets.iter() {
        if shown.is_some() && !registry.is_changed() && !changed.contains_entity(entity) {
            continue;
        }
        let (body, target, read) = match shown {
            Some(shown) => (shown.body, shown.target.clone(), true),
            None => {
                let body = cmd.entity(entity).spawn_child(body_node(&theme)).entity();
                let target: Arc<dyn PropertyTarget> = Arc::new(RowsTarget { entity });
                cmd.entity(entity).insert((
                    RowsBody {
                        body,
                        target: target.clone(),
                    },
                    LabelColumn { body },
                ));
                (body, target, false)
            }
        };
        let types = widget.types.read();
        let properties = registry.collect(widget.value.as_ref(), &types);
        if properties.is_empty() {
            cmd.insert(body_node(&theme).with_visible(true), body);
            cmd.entity(body)
                .despawn_children()
                .add_child(theme.label(EMPTY).small().muted().single_line());
            continue;
        }
        show_properties(
            CardRows {
                body,
                name: NAME,
                target,
                read,
            },
            properties,
            &types,
            &registry,
            &mut cmd,
            &theme,
            &bodies,
            &rows,
        );
    }
}

fn build_reflect_rows_widgets(
    rows: Query<(
        Entity,
        &ChildOf,
        Option<&PropertyRow>,
        Option<&PropertyRowValue>,
        &BuildPropertyWidget,
    )>,
    widgets: Query<(&ReflectRows, &RowsBody)>,
    registry: Res<InspectorRegistry>,
    theme: Res<UITheme>,
    mut cmd: CommandQueue,
) {
    for (entity, parent, row, value, build) in rows.iter() {
        let Some((widget, _)) = widgets
            .iter()
            .find(|(_, shown)| shown.body == parent.parent())
        else {
            continue;
        };
        let types = widget.types.read();
        build_row_widgets(
            entity,
            row,
            value,
            build,
            &registry,
            Some(&types),
            &theme,
            &mut cmd,
        );
    }
}
