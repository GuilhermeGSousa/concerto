use std::f32::consts::FRAC_PI_2;

use glam::{Quat, Vec3};

use super::*;

const NEAR: f32 = 1e-3;

#[test]
fn a_shape_goes_through_the_simulation_bone() {
    let bone = Transform::from_translation_rotation(
        Vec3::new(1.0, 0.0, 2.0),
        Quat::from_rotation_y(FRAC_PI_2),
    );

    let ring = FeatureShape::Ring {
        center: Vec3::Z,
        radius: 0.5,
    };
    let FeatureShape::Ring { center, radius } = placed_shape(&bone, ring) else {
        panic!("a ring stays a ring");
    };
    assert!(center.distance(Vec3::new(2.0, 0.0, 2.0)) < NEAR, "{center}");
    assert_eq!(radius, 0.5);

    let arrow = FeatureShape::Arrow {
        start: Vec3::Z,
        end: Vec3::new(0.0, 1.0, 2.0),
    };
    let FeatureShape::Arrow { start, end } = placed_shape(&bone, arrow) else {
        panic!("an arrow stays an arrow");
    };
    assert!(start.distance(Vec3::new(2.0, 0.0, 2.0)) < NEAR, "{start}");
    assert!(end.distance(Vec3::new(3.0, 1.0, 2.0)) < NEAR, "{end}");
}

#[test]
fn the_picked_channel_has_its_full_colour_and_the_others_are_dimmed() {
    let full = channel_color(1, Some(1)).to_array();

    for dimmed in [channel_color(1, None), channel_color(1, Some(0))] {
        let dimmed = dimmed.to_array();
        for index in 0..3 {
            assert!((dimmed[index] - full[index] * 0.4).abs() < NEAR);
        }
        assert_eq!(dimmed[3], full[3]);
    }
}

#[test]
fn channels_take_their_colours_in_turn() {
    let full = |channel| channel_color(channel, Some(channel));

    assert_ne!(full(0), full(1));
    assert_eq!(full(CHANNEL_COLORS.len()), full(0));
}

#[test]
fn no_channel_uses_the_simulation_bones_colour() {
    assert!(!CHANNEL_COLORS.contains(&SIMULATION_COLOR));
}
