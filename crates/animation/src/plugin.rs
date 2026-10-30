use concerto_app::{edit_time::AtPreviewTime, plugins::Plugin, schedule_groups::LateUpdate};
use concerto_mesh::skeleton::Skeleton;

use crate::{
    clip::AnimationClip,
    graph::AnimationGraph,
    target::{
        animate_targets, initialize_animation_players, initialize_skeletons,
        update_animation_players,
    },
};

pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_asset::<AnimationClip>();
        app.register_asset::<AnimationGraph>();
        app.register_asset::<Skeleton>();

        app.add_system(LateUpdate, initialize_skeletons.at_preview_time())
            .add_system(LateUpdate, animate_targets.at_preview_time())
            .add_system(LateUpdate, initialize_animation_players.at_preview_time())
            .add_system(LateUpdate, update_animation_players.at_preview_time());
    }
}
