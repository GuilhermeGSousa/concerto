use std::collections::HashMap;

use concerto_ecs::{
    command::CommandQueue, system::input::SystemLocal, world::FromWorld, Component, Entity, Query,
    Res, Without, World,
};
use concerto_foundation::assets::{asset_store::AssetStore, AssetId};

use crate::mesh::{Aabb, Mesh, MeshComponent};

/// The mesh asset an entity's [`Aabb`] was computed from. An `Aabb` without one was set by hand.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct AabbSource(AssetId);

#[derive(Default)]
pub struct MeshBoundsCache {
    boxes: HashMap<AssetId, Option<Aabb>>,
}

impl FromWorld for MeshBoundsCache {
    fn from_world(_: &World) -> Self {
        Self::default()
    }
}

/// Keeps a local-space [`Aabb`] on every entity with a loaded [`MeshComponent`].
pub fn update_mesh_bounds(
    entities: Query<(Entity, &MeshComponent, Option<&AabbSource>, Option<&Aabb>)>,
    orphans: Query<(Entity, &AabbSource), Without<MeshComponent>>,
    meshes: Res<AssetStore<Mesh>>,
    mut cache: SystemLocal<MeshBoundsCache>,
    mut cmd: CommandQueue,
) {
    for (entity, mesh, source, aabb) in entities.iter() {
        let id = mesh.handle.id();
        match source {
            Some(source) if source.0 == id => continue,
            None if aabb.is_some() => continue,
            _ => {}
        }
        let bounds = match cache.boxes.get(&id) {
            Some(bounds) => *bounds,
            None => {
                let Some(mesh) = meshes.get(&mesh.handle) else {
                    continue;
                };
                let bounds = mesh.local_aabb();
                cache.boxes.insert(id, bounds);
                bounds
            }
        };
        match bounds {
            Some(bounds) => cmd.insert((bounds, AabbSource(id)), entity),
            None => {
                if aabb.is_some() {
                    cmd.remove::<Aabb>(entity);
                }
                cmd.insert(AabbSource(id), entity);
            }
        }
    }
    for (entity, _) in orphans.iter() {
        cmd.remove::<Aabb>(entity);
        cmd.remove::<AabbSource>(entity);
    }
}
