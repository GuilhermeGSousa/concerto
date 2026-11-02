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

/// How a visible character is reconciled with its capsule each frame; the methods are mutually exclusive.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default)]
pub enum CharacterDisplacement {
    Synchronization(Synchronization),
    Adjustment(Adjustment),
    Clamp(Clamp),
}

impl Default for CharacterDisplacement {
    fn default() -> Self {
        Self::Adjustment(Adjustment::default())
    }
}

/// Places the character and the capsule at a weighted average of the two each frame, clamped to 0..=1: 0 is code-driven, 1 is data-driven, and the speed between them depends on the frame rate.
#[derive(Reflect, Clone, Debug, PartialEq)]
#[reflect(Default)]
pub struct Synchronization {
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
    pub by_velocity: bool,
    pub position_halflife: f32,
    pub rotation_halflife: f32,
    pub position_max_ratio: f32,
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
    pub max_distance: f32,
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

/// Names a character's capsule, which holds its `CharacterSimulation`, and its skeleton, which holds its `SkeletonComponent` and `AnimationPlayer`; the game inserts it on the visible character.
#[derive(Component, Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Component, SceneComponent, Default)]
pub struct CharacterLink {
    pub capsule: SceneEntityRef,
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
    use std::any::TypeId;

    use concerto_app::{plugins::AssetManagerPlugin, App};
    use concerto_ecs::{
        entity::hierarchy::{ChildOf, Children},
        reflect::AppTypeRegistry,
    };
    use concerto_reflect::{apply_value, to_value, with_property_mut, Enum, PropertyPath};
    use concerto_scene::{capture::capture_scene, spawner::spawn_scene_in_world};

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

    fn assert_position(actual: Vec3, expected: Vec3) {
        assert!(
            actual.distance(expected) < EPS,
            "{actual:?} vs {expected:?}"
        );
    }

    fn assert_yaw(actual: Quat, yaw: f32) {
        let expected = Quat::from_rotation_y(yaw) * Vec3::Z;
        let forward = actual * Vec3::Z;
        assert!(
            forward.distance(expected) < EPS,
            "{forward:?} vs a yaw of {yaw}"
        );
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

    #[test]
    fn a_data_factor_outside_its_range_acts_as_its_nearest_end() {
        for (factor, end) in [(1.5, 1.0), (-0.5, 0.0)] {
            let displace = |factor| {
                synchronization(factor).displace(
                    0.016,
                    at(1.0, 2.0, 0.4),
                    at(3.0, 2.0, 0.9),
                    moved_by(Vec3::X, 0.1),
                )
            };
            let (outside, inside) = (displace(factor), displace(end));
            assert_position(outside.character.position, inside.character.position);
            assert_position(outside.pull, inside.pull);
            assert_eq!(outside.character.rotation, inside.character.rotation);
        }
    }

    #[test]
    fn a_negative_clamp_holds_the_character_on_the_capsule() {
        let negative = CharacterDisplacement::Clamp(Clamp {
            max_distance: -1.0,
            max_angle: -1.0,
        });

        let result = negative.displace(
            0.016,
            at(1.0, 2.0, 0.4),
            at(3.0, 2.0, 0.9),
            RootMotion::IDENTITY,
        );

        assert_position(result.character.position, Vec3::new(1.0, 0.0, 2.0));
        assert_yaw(result.character.rotation, 0.4);
    }

    #[test]
    fn a_negative_ratio_gives_the_character_no_correction() {
        let negative = CharacterDisplacement::Adjustment(Adjustment {
            by_velocity: true,
            position_max_ratio: -1.0,
            rotation_max_ratio: -1.0,
            ..Adjustment::default()
        });

        let result = negative.displace(
            0.016,
            at(0.0, 0.0, 0.0),
            at(1.0, 0.0, 0.0),
            moved_by(Vec3::X, 0.1),
        );

        assert_position(result.character.position, Vec3::new(2.0, 0.0, 0.0));
        assert_yaw(result.character.rotation, 0.1);
    }

    #[test]
    fn a_negative_halflife_closes_the_gap_at_once() {
        let negative = CharacterDisplacement::Adjustment(Adjustment {
            by_velocity: false,
            position_halflife: -1.0,
            rotation_halflife: -1.0,
            ..Adjustment::default()
        });

        let result = negative.displace(
            0.016,
            at(0.0, 0.0, 0.0),
            at(1.0, 0.0, 0.5),
            RootMotion::IDENTITY,
        );

        assert_position(result.character.position, Vec3::ZERO);
        assert_yaw(result.character.rotation, 0.0);
    }

    #[test]
    fn synchronization_at_zero_sits_on_the_capsule_and_pulls_nothing() {
        let result = synchronization(0.0).displace(
            0.016,
            at(1.0, 2.0, 0.4),
            at(3.0, 2.0, 0.9),
            moved_by(Vec3::X, 0.1),
        );
        assert_position(result.character.position, Vec3::new(1.0, 0.0, 2.0));
        assert_yaw(result.character.rotation, 0.4);
        assert_eq!(result.pull, Vec3::ZERO);
    }

    #[test]
    fn synchronization_at_one_follows_root_motion_and_pulls_the_capsule_to_it() {
        let result = synchronization(1.0).displace(
            0.016,
            at(1.0, 2.0, 0.4),
            at(3.0, 2.0, 0.9),
            moved_by(Vec3::X, 0.1),
        );
        assert_position(result.character.position, Vec3::new(4.0, 0.0, 2.0));
        assert_yaw(result.character.rotation, 1.0);
        assert_position(result.pull, Vec3::new(3.0, 0.0, 0.0));
    }

    #[test]
    fn synchronization_at_a_half_meets_halfway() {
        let result = synchronization(0.5).displace(
            0.016,
            at(1.0, 2.0, 0.4),
            at(3.0, 2.0, 0.9),
            moved_by(Vec3::X, 0.1),
        );
        assert_position(result.character.position, Vec3::new(2.5, 0.0, 2.0));
        assert_yaw(result.character.rotation, 0.7);
        assert_position(result.pull, Vec3::new(1.5, 0.0, 0.0));
    }

    #[test]
    fn adjustment_closes_half_the_gap_in_one_halflife() {
        let result = adjustment(false).displace(
            0.1,
            at(2.0, 0.0, 0.8),
            at(0.0, 0.0, 0.0),
            RootMotion::IDENTITY,
        );
        assert_position(result.character.position, Vec3::new(1.0, 0.0, 0.0));
        assert_yaw(result.character.rotation, 0.4);
        assert_eq!(result.pull, Vec3::ZERO);
    }

    #[test]
    fn adjustment_closes_the_gap_left_after_root_motion() {
        let result = adjustment(false).displace(
            0.1,
            at(2.0, 0.0, 0.8),
            at(0.0, 0.0, 0.0),
            moved_by(Vec3::X, 0.4),
        );
        assert_position(result.character.position, Vec3::new(1.5, 0.0, 0.0));
        assert_yaw(result.character.rotation, 0.6);
    }

    #[test]
    fn adjustment_by_velocity_leaves_a_standing_character_where_it_is() {
        let character = at(0.0, 0.0, 0.3);
        let result =
            adjustment(true).displace(0.1, at(2.0, 0.0, 1.5), character, RootMotion::IDENTITY);
        assert_eq!(result.character.position, character.position);
        assert_yaw(result.character.rotation, 0.3);
    }

    #[test]
    fn adjustment_by_velocity_moves_at_most_the_ratio_of_this_frames_delta() {
        let result = adjustment(true).displace(
            0.1,
            at(0.2, 2.0, 2.0),
            at(0.0, 0.0, 0.0),
            moved_by(Vec3::new(0.2, 0.0, 0.0), 0.1),
        );
        assert_position(result.character.position, Vec3::new(0.2, 0.0, 0.1));
        assert_yaw(result.character.rotation, 0.15);
    }

    #[test]
    fn adjustment_by_velocity_keeps_a_correction_smaller_than_the_ratio() {
        let result = adjustment(true).displace(
            0.1,
            at(0.2, 0.02, 0.12),
            at(0.0, 0.0, 0.0),
            moved_by(Vec3::new(0.2, 0.0, 0.0), 0.1),
        );
        assert_position(result.character.position, Vec3::new(0.2, 0.0, 0.01));
        assert_yaw(result.character.rotation, 0.11);
    }

    #[test]
    fn vertical_root_motion_gives_an_adjustment_no_room() {
        let delta = RootMotion {
            translation: Vec3::Y,
            rotation: Quat::IDENTITY,
        };
        let result = adjustment(true).displace(0.1, at(2.0, 0.0, 0.0), at(0.0, 0.0, 0.0), delta);
        assert_position(result.character.position, Vec3::ZERO);
    }

    #[test]
    fn clamp_leaves_a_near_character_alone() {
        let result = clamp().displace(
            0.016,
            at(1.0, 1.0, 0.2),
            at(1.1, 1.2, 0.3),
            moved_by(Vec3::new(0.1, 0.0, 0.0), 0.1),
        );
        assert_eq!(
            result.character.position,
            Vec3::new(1.1, 0.0, 1.2) + Vec3::new(0.1, 0.0, 0.0)
        );
        assert_yaw(result.character.rotation, 0.4);
        assert_eq!(result.pull, Vec3::ZERO);
    }

    #[test]
    fn clamp_pulls_a_far_character_to_max_distance_along_the_line() {
        let result = clamp().displace(
            0.016,
            at(1.0, 1.0, 0.0),
            at(4.0, 5.0, 0.0),
            RootMotion::IDENTITY,
        );
        assert_position(result.character.position, Vec3::new(1.3, 0.0, 1.4));
    }

    #[test]
    fn clamp_limits_yaw_to_max_angle_on_either_side() {
        let capsule = at(0.0, 0.0, 0.2);
        let left = clamp().displace(0.016, capsule, at(0.0, 0.0, 1.5), RootMotion::IDENTITY);
        let right = clamp().displace(0.016, capsule, at(0.0, 0.0, -1.5), RootMotion::IDENTITY);
        assert_yaw(left.character.rotation, 0.7);
        assert_yaw(right.character.rotation, -0.3);
    }

    #[test]
    fn clamp_measures_yaw_the_short_way_round() {
        let result = clamp().displace(
            0.016,
            at(0.0, 0.0, 3.0),
            at(0.0, 0.0, -3.0),
            RootMotion::IDENTITY,
        );
        assert_yaw(result.character.rotation, -3.0);
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

    #[test]
    fn only_the_yaw_of_a_turn_is_applied() {
        let delta = RootMotion {
            translation: Vec3::ZERO,
            rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(0.3),
        };
        let result =
            synchronization(1.0).displace(0.016, at(0.0, 0.0, 0.0), at(0.0, 0.0, 0.2), delta);
        assert_yaw(result.character.rotation, 0.7);
    }

    #[test]
    fn a_character_keeps_any_tilt_it_has() {
        let tilt = Quat::from_rotation_x(0.3);
        let character = Placement {
            position: Vec3::ZERO,
            rotation: tilt,
        };
        for method in [synchronization(0.5), adjustment(false), clamp()] {
            let result =
                method.displace(0.1, at(0.0, 0.0, 2.0), character, moved_by(Vec3::ZERO, 0.0));
            let up = result.character.rotation * Vec3::Y;
            assert!(
                (up.y - (tilt * Vec3::Y).y).abs() < EPS,
                "{method:?}: {up:?}"
            );
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
    fn a_variant_switch_gives_that_structs_defaults() {
        let mut value = CharacterDisplacement::default();
        assert!(value.set_variant("Synchronization"));
        assert_eq!(
            value,
            CharacterDisplacement::Synchronization(Synchronization { data_factor: 1.0 })
        );
        assert!(value.set_variant("Clamp"));
        assert_eq!(
            value,
            CharacterDisplacement::Clamp(Clamp {
                max_distance: 0.15,
                max_angle: FRAC_PI_2,
            })
        );
        assert!(value.set_variant("Adjustment"));
        assert_eq!(value, CharacterDisplacement::default());
    }

    #[test]
    fn the_plugin_registers_the_component_and_each_variants_struct() {
        let mut app = App::new();
        app.register_plugin(AssetManagerPlugin);
        app.register_plugin(MotionMatchingPlugin);
        let world = app.main_mut().world_mut();
        let registry = world.get_resource::<AppTypeRegistry>().unwrap();
        let id = TypeId::of::<CharacterDisplacement>();
        assert!(registry.get_type_data::<ReflectComponent>(id).is_some());
        assert!(registry.get_type_data::<ReflectDefault>(id).is_some());
        for id in [
            TypeId::of::<Synchronization>(),
            TypeId::of::<Adjustment>(),
            TypeId::of::<Clamp>(),
        ] {
            assert!(registry.get_type_data::<ReflectDefault>(id).is_some());
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
        let spawned = spawn_scene_in_world(world, &scene, parent).node_entities;

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

    fn written_through_path<T: Reflect + Copy>(
        value: &mut CharacterDisplacement,
        field: &'static str,
        written: T,
    ) {
        let path = PropertyPath::new(["0", field]);
        with_property_mut(value, &path, &mut |leaf| {
            *leaf.downcast_mut::<T>().unwrap() = written;
        })
        .unwrap();
    }

    #[test]
    fn each_variants_fields_are_reachable_by_a_property_path() {
        let [mut synchronization, mut adjustment, mut clamp] = variants();
        written_through_path(&mut synchronization, "data_factor", 0.1_f32);
        written_through_path(&mut adjustment, "by_velocity", true);
        written_through_path(&mut adjustment, "position_halflife", 0.2_f32);
        written_through_path(&mut adjustment, "rotation_halflife", 0.3_f32);
        written_through_path(&mut adjustment, "position_max_ratio", 0.4_f32);
        written_through_path(&mut adjustment, "rotation_max_ratio", 0.5_f32);
        written_through_path(&mut clamp, "max_distance", 0.6_f32);
        written_through_path(&mut clamp, "max_angle", 0.7_f32);
        assert_eq!(
            synchronization,
            CharacterDisplacement::Synchronization(Synchronization { data_factor: 0.1 })
        );
        assert_eq!(
            adjustment,
            CharacterDisplacement::Adjustment(Adjustment {
                by_velocity: true,
                position_halflife: 0.2,
                rotation_halflife: 0.3,
                position_max_ratio: 0.4,
                rotation_max_ratio: 0.5,
            })
        );
        assert_eq!(
            clamp,
            CharacterDisplacement::Clamp(Clamp {
                max_distance: 0.6,
                max_angle: 0.7,
            })
        );
    }
}
