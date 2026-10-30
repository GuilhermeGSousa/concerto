use concerto_app::{plugins::Plugin, schedule_groups::LateUpdate};
use concerto_ecs::reflect::AppTypeRegistry;
use concerto_foundation::assets::handle::AssetHandle;
use concerto_mesh::skeleton::Skeleton;
use concerto_reflect::TypeRegistry;

use crate::{
    clip::AnimationClip,
    graph::AnimationGraph,
    node::{
        AnimationClipNode, BlendNode, PlaceholderNode, ResultNode, blend_space::BlendSpace2DNode,
        state_machine::AnimationStateMachine,
    },
    target::{
        animate_targets, initialize_animation_players, initialize_skeletons,
        update_animation_players,
    },
};

/// Registers the built-in animation nodes and the handle types a graph stores.
pub fn register_animation_types(registry: &mut TypeRegistry) {
    registry.register::<ResultNode>();
    registry.register::<BlendNode>();
    registry.register::<AnimationClipNode>();
    registry.register::<BlendSpace2DNode>();
    registry.register::<AnimationStateMachine>();
    registry.register::<PlaceholderNode>();
    registry.register::<AssetHandle<AnimationClip>>();
    registry.register::<AssetHandle<AnimationGraph>>();
}

pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_asset::<AnimationClip>();
        app.register_asset::<AnimationGraph>();
        app.register_asset::<Skeleton>();

        if let Some(registry) = app.get_resource::<AppTypeRegistry>() {
            register_animation_types(&mut registry.write());
        }

        app.add_system(LateUpdate, initialize_skeletons)
            .add_system(LateUpdate, animate_targets)
            .add_system(LateUpdate, initialize_animation_players)
            .add_system(LateUpdate, update_animation_players);
    }
}
