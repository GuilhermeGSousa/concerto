use concerto_app::{App, Plugin, edit_time::AtEditTime, schedule_groups::LateUpdate};
use concerto_ecs::{
    Changed, Entity, IntoSystemConfig, Or, Query, ResMut, With,
    component::{removed::RemovedComponents, scene::SceneEntityRef},
};
use concerto_foundation::transform::{
    GlobalTransform,
    systems::{propagate_global_transforms, update_simple_entities},
};

use crate::{
    graph::{NavGraph, NavPointRecord},
    point::{NavLink, NavLinks, NavPoint},
};

/// Registers the navigation components and keeps `NavGraph` in step with them.
pub struct NavigationPlugin;

impl Plugin for NavigationPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<NavPoint>()
            .register_type::<NavLinks>()
            .register_type::<NavLink>()
            .insert_resource(NavGraph::default())
            .track_removals::<NavPoint>()
            .track_removals::<NavLinks>();
        app.add_system(
            LateUpdate,
            rebuild_nav_graph
                .after(update_simple_entities)
                .after(propagate_global_transforms)
                .at_edit_time(),
        );
    }
}

/// Rebuilds `NavGraph` when a point or its links were added, changed, moved or removed.
pub fn rebuild_nav_graph(
    points: Query<(Entity, &GlobalTransform, &NavPoint, Option<&NavLinks>)>,
    changed: Query<
        Entity,
        (
            With<NavPoint>,
            Or<(
                Changed<NavPoint>,
                Changed<NavLinks>,
                Changed<GlobalTransform>,
            )>,
        ),
    >,
    mut removed_points: RemovedComponents<NavPoint>,
    mut removed_links: RemovedComponents<NavLinks>,
    mut graph: ResMut<NavGraph>,
) {
    let removed = removed_points.read().count() + removed_links.read().count() > 0;
    if !removed && changed.iter().next().is_none() {
        return;
    }
    *graph = NavGraph::build(points.iter().map(|(entity, transform, point, links)| {
        links
            .into_iter()
            .flat_map(|links| links.links.iter())
            .filter(|link| matches!(link.to, SceneEntityRef::Entity(_)))
            .fold(
                NavPointRecord::new(entity, transform.translation())
                    .with_rotation(transform.rotation())
                    .with_anchor(point.anchor)
                    .with_enabled(point.enabled),
                |record, link| record.with_link(*link),
            )
    }));
}
