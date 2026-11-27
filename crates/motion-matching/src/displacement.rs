use std::f32::consts::FRAC_PI_2;

use concerto_animation::pose::{twist_about, RootMotion};
use concerto_ecs::{
    component::scene::SceneEntityRef,
    reflect::{ReflectComponent, ReflectSceneComponent},
    Component,
};
use concerto_foundation::math::spring_damper::{
    damp_adjustment_exact, from_scaled_angle_axis, quat_abs, to_scaled_angle_axis,
};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Quat, Vec3};

const ADJUSTMENT_EPS: f32 = 1.0e-8;

/// How a visible character is reconciled with its capsule each frame.
///
/// The methods are mutually exclusive.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
pub enum CharacterDisplacement {
    /// Averages the character and the capsule; see [`Synchronization`].
    Synchronization(Synchronization),
    /// Damps the character toward the capsule; see [`Adjustment`].
    Adjustment(Adjustment),
    /// Keeps the character near the capsule; see [`Clamp`].
    Clamp(Clamp),
}

impl Default for CharacterDisplacement {
    fn default() -> Self {
        Self::Synchronization(Synchronization::default())
    }
}

/// Places the character and the capsule at a weighted average of the two each frame.
///
/// `data_factor` is clamped to `0..=1`: 0 is code-driven, 1 is data-driven,
/// and how fast values in between converge depends on the frame rate.
#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(Default)]
pub struct Synchronization {
    /// How far the result leans toward the character's animated placement, from 0 to 1.
    pub data_factor: f32,
}

impl Default for Synchronization {
    fn default() -> Self {
        Self { data_factor: 1.0 }
    }
}

/// Damps the character towards the capsule, by no more than a ratio of this frame's root motion when `by_velocity` is on.
#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(Default)]
pub struct Adjustment {
    /// Whether each frame's correction is capped by a ratio of that frame's root motion.
    pub by_velocity: bool,
    /// The halflife, in seconds, the position gap closes with.
    pub position_halflife: f32,
    /// The halflife, in seconds, the facing gap closes with.
    pub rotation_halflife: f32,
    /// The largest position correction per frame, as a fraction of the frame's root translation.
    pub position_max_ratio: f32,
    /// The largest facing correction per frame, as a fraction of the frame's root turn.
    pub rotation_max_ratio: f32,
}

impl Default for Adjustment {
    fn default() -> Self {
        Self {
            by_velocity: true,
            position_halflife: 0.1,
            rotation_halflife: 0.2,
            position_max_ratio: 0.5,
            rotation_max_ratio: 0.5,
        }
    }
}

/// Keeps the character within `max_distance` metres and `max_angle` radians of the capsule.
#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(Default)]
pub struct Clamp {
    /// The farthest the character may be from the capsule, in metres.
    pub max_distance: f32,
    /// The widest the character's facing may differ from the capsule's, in radians.
    pub max_angle: f32,
}

impl Default for Clamp {
    fn default() -> Self {
        Self {
            max_distance: 0.15,
            max_angle: FRAC_PI_2,
        }
    }
}

/// Names a character's capsule and its skeleton.
///
/// The capsule holds the character's
/// [`CharacterSimulation`](crate::simulation::CharacterSimulation), and the
/// skeleton its `SkeletonComponent` and `AnimationPlayer`. The link goes on the
/// visible character, set in the editor or inserted by the game.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Component, SceneComponent, Default)]
pub struct CharacterLink {
    /// The entity holding the character's
    /// [`CharacterSimulation`](crate::simulation::CharacterSimulation).
    pub capsule: SceneEntityRef,
    /// The entity holding the skeleton and its animation player.
    pub skeleton: SceneEntityRef,
}

/// Where a body stands and how it faces, in world space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    pub(crate) position: Vec3,
    pub(crate) rotation: Quat,
}

/// A character's placement for this frame, and the world-space offset on the plane that takes the capsule to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Displaced {
    pub(crate) character: Placement,
    pub(crate) pull: Vec3,
}

pub(crate) fn yaw_of(rotation: Quat) -> Quat {
    twist_about(rotation, Vec3::Y)
}

fn yaw_from(from: Quat, to: Quat) -> Vec3 {
    to_scaled_angle_axis(quat_abs(yaw_of(to) * yaw_of(from).inverse()))
}

impl Synchronization {
    fn apply(&self, capsule: Placement, moved: Placement) -> Placement {
        let data_factor = self.data_factor.clamp(0.0, 1.0);
        let moved_yaw = yaw_of(moved.rotation);
        let yaw = yaw_of(capsule.rotation).slerp(moved_yaw, data_factor);
        Placement {
            position: capsule.position.lerp(moved.position, data_factor),
            rotation: yaw * moved_yaw.inverse() * moved.rotation,
        }
    }
}

impl Adjustment {
    fn apply(&self, dt: f32, capsule: Placement, moved: Placement, delta: RootMotion) -> Placement {
        let gap = (capsule.position - moved.position).with_y(0.0);
        let halflife = self.position_halflife.max(0.0);
        let mut shift = damp_adjustment_exact(gap, halflife, dt, ADJUSTMENT_EPS);
        let yaw_gap = yaw_from(moved.rotation, capsule.rotation);
        let halflife = self.rotation_halflife.max(0.0);
        let mut turn = damp_adjustment_exact(yaw_gap, halflife, dt, ADJUSTMENT_EPS);
        if self.by_velocity {
            let turned = to_scaled_angle_axis(quat_abs(delta.rotation)).length();
            let max_shift = self.position_max_ratio * delta.translation.length();
            shift = shift.clamp_length_max(max_shift.max(0.0));
            turn = turn.clamp_length_max((self.rotation_max_ratio * turned).max(0.0));
        }
        Placement {
            position: moved.position + shift,
            rotation: from_scaled_angle_axis(turn) * moved.rotation,
        }
    }
}

impl Clamp {
    fn apply(&self, capsule: Placement, moved: Placement) -> Placement {
        let offset = (moved.position - capsule.position).with_y(0.0);
        let yaw_offset = yaw_from(capsule.rotation, moved.rotation);
        Placement {
            position: moved.position
                + (offset.clamp_length_max(self.max_distance.max(0.0)) - offset),
            rotation: from_scaled_angle_axis(
                yaw_offset.clamp_length_max(self.max_angle.max(0.0)) - yaw_offset,
            ) * moved.rotation,
        }
    }
}

impl CharacterDisplacement {
    /// Moves `character` by `delta`, this frame's world-space root motion, and reconciles it with `capsule` by this method.
    pub(crate) fn displace(
        &self,
        dt: f32,
        capsule: Placement,
        character: Placement,
        delta: RootMotion,
    ) -> Displaced {
        let delta = RootMotion {
            translation: delta.translation.with_y(0.0),
            rotation: yaw_of(delta.rotation),
        };
        let moved = Placement {
            position: character.position + delta.translation,
            rotation: delta.rotation * character.rotation,
        };
        let placed = match self {
            Self::Synchronization(method) => method.apply(capsule, moved),
            Self::Adjustment(method) => method.apply(dt, capsule, moved, delta),
            Self::Clamp(method) => method.apply(capsule, moved),
        };
        let pull = if matches!(self, Self::Synchronization(_)) {
            (placed.position - capsule.position).with_y(0.0)
        } else {
            Vec3::ZERO
        };
        Displaced {
            character: Placement {
                position: placed.position.with_y(capsule.position.y),
                rotation: placed.rotation.normalize(),
            },
            pull,
        }
    }
}

#[cfg(test)]
mod tests {

    use concerto_app::{plugins::AssetManagerPlugin, App};
    use concerto_ecs::entity::hierarchy::{ChildOf, Children};
    use concerto_reflect::{apply_value, to_value};
    use concerto_scene::{
        capture::capture_scene,
        spawner::{spawn_scene_in_world, SpawnFilter},
    };

    use super::*;
    use crate::plugin::MotionMatchingPlugin;

    const EPS: f32 = 1.0e-4;

    fn at(x: f32, z: f32, yaw: f32) -> Placement {
        Placement {
            position: Vec3::new(x, 0.0, z),
            rotation: Quat::from_rotation_y(yaw),
        }
    }

    fn moved_by(translation: Vec3, yaw: f32) -> RootMotion {
        RootMotion {
            translation,
            rotation: Quat::from_rotation_y(yaw),
        }
    }

    fn synchronization(data_factor: f32) -> CharacterDisplacement {
        CharacterDisplacement::Synchronization(Synchronization { data_factor })
    }

    fn adjustment(by_velocity: bool) -> CharacterDisplacement {
        CharacterDisplacement::Adjustment(Adjustment {
            by_velocity,
            position_halflife: 0.1,
            rotation_halflife: 0.1,
            ..Adjustment::default()
        })
    }

    fn clamp() -> CharacterDisplacement {
        CharacterDisplacement::Clamp(Clamp {
            max_distance: 0.5,
            max_angle: 0.5,
        })
    }

    struct Displaced {
        name: &'static str,
        method: CharacterDisplacement,
        delta_time: f32,
        capsule: Placement,
        character: Placement,
        motion: RootMotion,
        position: Vec3,
        yaw: f32,
        pull: Option<Vec3>,
    }

    #[test]
    fn each_method_places_the_character_and_pulls_the_capsule_as_it_promises() {
        let synchronized = |name, data_factor, position, yaw, pull| Displaced {
            name,
            method: synchronization(data_factor),
            delta_time: 0.016,
            capsule: at(1.0, 2.0, 0.4),
            character: at(3.0, 2.0, 0.9),
            motion: moved_by(Vec3::X, 0.1),
            position,
            yaw,
            pull: Some(pull),
        };
        let clamped = |name, capsule, character, position, yaw| Displaced {
            name,
            method: clamp(),
            delta_time: 0.016,
            capsule,
            character,
            motion: RootMotion::IDENTITY,
            position,
            yaw,
            pull: None,
        };
        let rows = [
            synchronized(
                "synchronization at 0 sits on the capsule",
                0.0,
                Vec3::new(1.0, 0.0, 2.0),
                0.4,
                Vec3::ZERO,
            ),
            synchronized(
                "synchronization at a half meets halfway",
                0.5,
                Vec3::new(2.5, 0.0, 2.0),
                0.7,
                Vec3::new(1.5, 0.0, 0.0),
            ),
            synchronized(
                "synchronization at 1 follows root motion",
                1.0,
                Vec3::new(4.0, 0.0, 2.0),
                1.0,
                Vec3::new(3.0, 0.0, 0.0),
            ),
            Displaced {
                name: "only the yaw of a turn is applied",
                capsule: at(0.0, 0.0, 0.0),
                character: at(0.0, 0.0, 0.2),
                motion: RootMotion {
                    translation: Vec3::ZERO,
                    rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(0.3),
                },
                position: Vec3::ZERO,
                yaw: 0.7,
                pull: None,
                ..synchronized("", 1.0, Vec3::ZERO, 0.0, Vec3::ZERO)
            },
            Displaced {
                name: "adjustment closes half the gap left after root motion in one halflife",
                method: adjustment(false),
                delta_time: 0.1,
                capsule: at(2.0, 0.0, 0.8),
                character: at(0.0, 0.0, 0.0),
                motion: moved_by(Vec3::X, 0.4),
                position: Vec3::new(1.5, 0.0, 0.0),
                yaw: 0.6,
                pull: Some(Vec3::ZERO),
            },
            Displaced {
                name: "adjustment by velocity moves at most the ratio of this frame's delta",
                method: adjustment(true),
                delta_time: 0.1,
                capsule: at(0.2, 2.0, 2.0),
                character: at(0.0, 0.0, 0.0),
                motion: moved_by(Vec3::new(0.2, 0.0, 0.0), 0.1),
                position: Vec3::new(0.2, 0.0, 0.1),
                yaw: 0.15,
                pull: None,
            },
            clamped(
                "clamp pulls a far character to max distance along the line",
                at(1.0, 1.0, 0.0),
                at(4.0, 5.0, 0.0),
                Vec3::new(1.3, 0.0, 1.4),
                0.0,
            ),
            clamped(
                "clamp limits yaw to max angle",
                at(0.0, 0.0, 0.2),
                at(0.0, 0.0, -1.5),
                Vec3::ZERO,
                -0.3,
            ),
            clamped(
                "clamp measures yaw the short way round",
                at(0.0, 0.0, 3.0),
                at(0.0, 0.0, -3.0),
                Vec3::ZERO,
                -3.0,
            ),
        ];

        for row in rows {
            let result =
                row.method
                    .displace(row.delta_time, row.capsule, row.character, row.motion);

            assert!(
                result.character.position.distance(row.position) < EPS,
                "{}: {:?}",
                row.name,
                result.character.position
            );
            let forward = result.character.rotation * Vec3::Z;
            assert!(
                forward.distance(Quat::from_rotation_y(row.yaw) * Vec3::Z) < EPS,
                "{}: {forward:?}",
                row.name
            );
            if let Some(pull) = row.pull {
                assert!(result.pull.distance(pull) < EPS, "{}", row.name);
            }
        }
    }

    #[test]
    fn height_is_the_capsules_in_every_method() {
        let capsule = Placement {
            position: Vec3::new(0.0, 1.5, 0.0),
            rotation: Quat::IDENTITY,
        };
        let character = Placement {
            position: Vec3::new(0.2, 0.3, 0.0),
            rotation: Quat::IDENTITY,
        };
        let delta = moved_by(Vec3::new(0.1, 0.7, 0.0), 0.0);
        for method in [synchronization(1.0), adjustment(true), clamp()] {
            let result = method.displace(0.016, capsule, character, delta);
            assert_eq!(result.character.position.y, 1.5, "{method:?}");
            assert_eq!(result.pull.y, 0.0, "{method:?}");
        }
    }

    fn variants() -> [CharacterDisplacement; 3] {
        [
            CharacterDisplacement::Synchronization(Synchronization { data_factor: 0.25 }),
            CharacterDisplacement::Adjustment(Adjustment {
                by_velocity: false,
                position_halflife: 0.3,
                rotation_halflife: 0.4,
                position_max_ratio: 0.6,
                rotation_max_ratio: 0.7,
            }),
            CharacterDisplacement::Clamp(Clamp {
                max_distance: 0.8,
                max_angle: 0.9,
            }),
        ]
    }

    #[test]
    fn each_variant_round_trips_through_reflection() {
        for value in variants() {
            let stored = to_value(&value).unwrap();
            let mut loaded = CharacterDisplacement::Clamp(Clamp::default());
            assert!(apply_value(&mut loaded, &stored).is_empty());
            assert_eq!(loaded, value);
        }
    }

    #[test]
    fn a_link_captured_into_a_scene_spawns_back_naming_the_spawned_capsule_and_skeleton() {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.register_plugin(MotionMatchingPlugin);
        let world = app.main_mut().world_mut();
        world.register_component::<ChildOf>();
        world.register_component::<Children>();
        let root = world.spawn(());
        let capsule = world.spawn(());
        let skeleton = world.spawn(());
        let character = world.spawn(CharacterLink {
            capsule: SceneEntityRef::Entity(capsule),
            skeleton: SceneEntityRef::Entity(skeleton),
        });
        for child in [capsule, character, skeleton] {
            world.entity_mut(root).add_child(child);
        }

        let scene = capture_scene(world, root).unwrap();
        let parent = world.spawn(());
        let spawned =
            spawn_scene_in_world(world, &scene, parent, &SpawnFilter::default()).node_entities;

        let link = world
            .get_component_for_entity::<CharacterLink>(spawned[1])
            .unwrap();
        assert_eq!(
            *link,
            CharacterLink {
                capsule: SceneEntityRef::Entity(spawned[0]),
                skeleton: SceneEntityRef::Entity(spawned[2]),
            }
        );
    }
}
