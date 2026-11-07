use concerto_ecs::{Changed, Query};
use concerto_motion_matching::DesiredMotion;
use concerto_navigation::NavSteering;

/// Copies each agent's changed `NavSteering` into its `DesiredMotion`.
pub fn follow_nav_steering(
    agents: Query<(&NavSteering, &mut DesiredMotion), Changed<NavSteering>>,
) {
    for (steering, mut desired) in agents.iter() {
        desired.velocity = steering.velocity;
        desired.rotation = steering.facing;
    }
}
