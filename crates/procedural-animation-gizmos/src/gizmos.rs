use concerto_animation::{animate_targets, graph::AnimationGraph, player::AnimationPlayer};
use concerto_app::{plugins::Plugin, schedule_groups::LateUpdate};
use concerto_color::Color;
use concerto_debug_gizmos::{DebugGizmos, DebugGizmosPlugin};
use concerto_ecs::{Component, IntoSystemConfig, Query, Res, reflect::ReflectComponent};
use concerto_foundation::{
    assets::asset_store::AssetStore, transform::systems::propagate_global_transforms,
};
use concerto_procedural_animation::{FootLockNode, LegLock, LockPhase};
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_render::plugin::RenderPlugin;
use glam::Vec3;

pub(crate) const LOCKED_COLOR: Color = Color::rgba(0.2, 0.9, 0.3, 1.0);
pub(crate) const RELEASING_COLOR: Color = Color::rgba(1.0, 0.7, 0.1, 1.0);
pub(crate) const FREE_COLOR: Color = Color::rgba(0.6, 0.6, 0.6, 1.0);
pub(crate) const LOCK_RADIUS: f32 = 0.06;
pub(crate) const CROSS_SIZE: f32 = 0.04;

/// Whether a player's foot locks are drawn.
///
/// It goes on the entity that holds the `AnimationPlayer`.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
pub struct FootLockGizmos {
    pub locks: bool,
}

impl Default for FootLockGizmos {
    fn default() -> Self {
        Self { locks: true }
    }
}

/// Draws [`FootLockGizmos`] in game time.
///
/// Register it after `DefaultPlugins`.
pub struct ProceduralAnimationGizmosPlugin;

impl Plugin for ProceduralAnimationGizmosPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.register_type::<FootLockGizmos>();
        if app.is_plugin_added::<RenderPlugin>() && !app.is_plugin_added::<DebugGizmosPlugin>() {
            app.register_plugin(DebugGizmosPlugin);
        }
        if app.is_plugin_added::<DebugGizmosPlugin>() {
            app.add_system(
                LateUpdate,
                draw_foot_locks
                    .after(animate_targets)
                    .after(propagate_global_transforms),
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum LockShape {
    Circle {
        center: Vec3,
        color: Color,
    },
    Cross {
        center: Vec3,
        color: Color,
    },
    Line {
        start: Vec3,
        end: Vec3,
        color: Color,
    },
}

pub(crate) fn leg_shapes(leg: &LegLock) -> Vec<LockShape> {
    match leg.phase {
        LockPhase::Locked => vec![
            LockShape::Circle {
                center: leg.lock,
                color: LOCKED_COLOR,
            },
            LockShape::Cross {
                center: leg.lock,
                color: LOCKED_COLOR,
            },
        ],
        LockPhase::Releasing => vec![
            LockShape::Circle {
                center: leg.shown,
                color: RELEASING_COLOR,
            },
            LockShape::Line {
                start: leg.shown,
                end: leg.animated,
                color: RELEASING_COLOR,
            },
        ],
        LockPhase::Free => vec![LockShape::Cross {
            center: leg.animated,
            color: FREE_COLOR,
        }],
    }
}

fn draw_shape(gizmos: &mut DebugGizmos, shape: LockShape) {
    match shape {
        LockShape::Circle { center, color } => gizmos.circle(center, Vec3::Y, LOCK_RADIUS, color),
        LockShape::Cross { center, color } => gizmos.cross(center, CROSS_SIZE, color),
        LockShape::Line { start, end, color } => gizmos.line(start, end, color),
    }
}

/// Draws each foot lock of every player whose `FootLockGizmos` has `locks` on.
pub fn draw_foot_locks(
    players: Query<(&FootLockGizmos, &AnimationPlayer)>,
    graphs: Res<AssetStore<AnimationGraph>>,
    mut gizmos: DebugGizmos,
) {
    for (shown, player) in players.iter() {
        if !shown.locks {
            continue;
        }
        player.for_each_state::<FootLockNode>(&graphs, |_, state, _| {
            for leg in state.legs() {
                for shape in leg_shapes(leg) {
                    draw_shape(&mut gizmos, shape);
                }
            }
        });
    }
}
