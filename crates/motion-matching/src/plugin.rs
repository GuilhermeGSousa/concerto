use concerto_app::plugins::Plugin;

use crate::simulation::{CharacterSimulation, DesiredMotion, TrajectorySample};

pub struct MotionMatchingPlugin;

impl Plugin for MotionMatchingPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_type::<DesiredMotion>()
            .register_type::<TrajectorySample>()
            .register_type::<CharacterSimulation>();
    }
}
