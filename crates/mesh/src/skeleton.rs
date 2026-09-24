use concerto_ecs::{
    component::scene::{SceneComponent, SceneEntityRef, SceneSpawnContext},
    Component, Entity,
};
use concerto_foundation::assets::{
    asset_server::AssetServer, handle::AssetHandle, Asset, LoadableAsset,
};
use glam::Mat4;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Asset, Serialize, Deserialize)]
pub struct Skeleton {
    pub inverse_bindposes: Box<[Mat4]>,
}

impl From<Vec<Mat4>> for Skeleton {
    fn from(value: Vec<Mat4>) -> Self {
        Self {
            inverse_bindposes: value.into_boxed_slice(),
        }
    }
}

impl LoadableAsset for Skeleton {}

#[derive(Component, Clone, Serialize, Deserialize)]
pub struct SkeletonComponent {
    pub skeleton: AssetHandle<Skeleton>,
    pub bones: Vec<SceneEntityRef>,
    pub bone_ids: Vec<Uuid>,
    pub root: Option<SceneEntityRef>,
}

impl SkeletonComponent {
    pub fn new(
        skeleton: AssetHandle<Skeleton>,
        bones: Vec<SceneEntityRef>,
        bone_ids: Vec<Uuid>,
        root: Option<SceneEntityRef>,
    ) -> Self {
        Self {
            skeleton,
            bones,
            bone_ids,
            root,
        }
    }

    pub fn skeleton(&self) -> &AssetHandle<Skeleton> {
        &self.skeleton
    }

    pub fn bones(&self) -> &[SceneEntityRef] {
        &self.bones
    }

    pub fn bone_ids(&self) -> &[Uuid] {
        &self.bone_ids
    }

    pub fn root(&self) -> Option<Entity> {
        self.root.and_then(SceneEntityRef::entity)
    }
}

impl SceneComponent for SkeletonComponent {
    fn apply(mut self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        if let Some(server) = ctx.get_resource::<AssetServer>() {
            self.skeleton = server.load(self.skeleton.id());
        }
        for bone in &mut self.bones {
            ctx.resolve_entity(bone);
        }
        if let Some(root) = &mut self.root {
            ctx.resolve_entity(root);
        }
        ctx.insert(self, entity);
    }
}
