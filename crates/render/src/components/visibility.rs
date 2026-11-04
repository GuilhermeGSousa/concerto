use concerto_ecs::{component::Component, reflect::ReflectSceneComponent};
use concerto_reflect::Reflect;

use crate::components::render_entity::SyncWithRenderWorld;

/// Keeps a mesh drawn in every view, whatever its bounds.
#[derive(Component, Reflect, Default, Clone, Copy)]
#[require(SyncWithRenderWorld)]
#[reflect(SceneComponent)]
pub struct NoFrustumCulling;
