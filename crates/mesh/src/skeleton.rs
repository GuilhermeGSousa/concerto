use concerto_ecs::{
    component::scene::SceneEntityRef, reflect::ReflectSceneComponent, Component, Entity,
};
use concerto_foundation::assets::{handle::AssetHandle, Asset, LoadableAsset};
use concerto_reflect::Reflect;
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

#[derive(Component, Clone, Reflect, Default)]
#[reflect(SceneComponent)]
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
