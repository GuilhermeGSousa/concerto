use concerto_app::{plugins::Plugin, schedule_groups::LateUpdate};

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

        app.add_system(LateUpdate, initialize_skeletons)
            .add_system(LateUpdate, animate_targets)
            .add_system(LateUpdate, initialize_animation_players)
            .add_system(LateUpdate, update_animation_players);
    }
}
