use concerto_ecs::{
    CommandQueue, Component, Entity, Query, Res,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::Vec3;

use crate::{asset::NavMesh, navmesh::NavMeshData};

/// The navmesh agents route on, held by an entity of the level.
#[derive(Component, Reflect, Clone, Debug, Default, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
pub struct NavMeshComponent {
    pub handle: AssetHandle<NavMesh>,
}

/// What the loaded navmesh of this entity's [`NavMeshComponent`] holds: its polygon count and the bounds of its vertices.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Component, Default)]
pub struct NavMeshStatus {
    pub polygons: u32,
    pub min: Vec3,
    pub max: Vec3,
}

fn status_of(data: &NavMeshData) -> NavMeshStatus {
    let polygons = data.polygons.len() as u32;
    if data.vertices.is_empty() {
        return NavMeshStatus {
            polygons,
            ..NavMeshStatus::default()
        };
    }
    NavMeshStatus {
        polygons,
        min: data
            .vertices
            .iter()
            .fold(Vec3::splat(f32::MAX), |min, vertex| min.min(*vertex)),
        max: data
            .vertices
            .iter()
            .fold(Vec3::splat(f32::MIN), |max, vertex| max.max(*vertex)),
    }
}

/// Writes a [`NavMeshStatus`] on each [`NavMeshComponent`] entity whose navmesh is loaded.
///
/// It removes the status from those whose navmesh is not.
pub fn update_nav_mesh_status(
    components: Query<(Entity, &NavMeshComponent, Option<&NavMeshStatus>)>,
    navmeshes: Res<AssetStore<NavMesh>>,
    mut cmd: CommandQueue,
) {
    for (entity, component, status) in components.iter() {
        let current = navmeshes
            .get(&component.handle)
            .and_then(NavMesh::data)
            .map(status_of);
        match (current, status) {
            (Some(current), Some(status)) if current == *status => {}
            (Some(current), _) => cmd.insert(current, entity),
            (None, Some(_)) => cmd.remove::<NavMeshStatus>(entity),
            (None, None) => {}
        }
    }
}
