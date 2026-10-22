use std::sync::{Arc, Mutex, PoisonError};

use concerto_app::{App, edit_time::AtEditTime, schedule_groups::LateUpdate};
use concerto_ecs::{Query, Res, Resource};
use concerto_editor::{
    hosted::{HostRequest, HostRole, begin, finish_blocking},
    tools::{
        EditToolsPlugin,
        camera::{FlyCamera, SetCameraPose},
    },
};
use concerto_foundation::transform::Transform;
use concerto_render::{ExclusiveCamera, gpu::Gpu};
use glam::Vec3;

fn game(app: &mut App) {
    app.register_plugin(concerto::DefaultPlugins);
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Seen {
    position: Vec3,
    yaw: f32,
    pitch: f32,
    camera_at: Vec3,
    camera_facing: Vec3,
}

#[derive(Resource, Clone, Default)]
struct Record(Arc<Mutex<Option<Seen>>>);

fn record(
    fly: Res<FlyCamera>,
    exclusive: Res<ExclusiveCamera>,
    transforms: Query<&Transform>,
    record: Res<Record>,
) {
    let Some(transform) = transforms.get_entity(exclusive.0) else {
        return;
    };
    *record.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(Seen {
        position: fly.position,
        yaw: fly.yaw,
        pitch: fly.pitch,
        camera_at: transform.translation,
        camera_facing: transform.rotation * Vec3::NEG_Z,
    });
}

#[test]
fn a_camera_pose_places_the_editor_camera_on_the_next_step() {
    let Some(gpu) = pollster::block_on(Gpu::request(&Gpu::instance(), None)) else {
        return;
    };
    let seen = Record::default();
    let recorder = seen.clone();
    let pending = begin(
        HostRequest {
            gpu: Some(&gpu),
            recipe: game,
            size: (64, 48),
            scale_factor: 1.0,
            role: HostRole::Edit,
        },
        move |app| {
            app.register_plugin(EditToolsPlugin)
                .insert_resource(recorder)
                .add_system(LateUpdate, record.at_edit_time());
        },
        |_| {},
    );
    let mut hosted = finish_blocking(pending);
    hosted.step_for_test();
    let last = || {
        seen.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .unwrap()
    };
    assert_eq!(last().position, FlyCamera::default().position);

    let pose = SetCameraPose {
        position: Vec3::new(1.0, 2.0, 3.0),
        yaw: std::f32::consts::FRAC_PI_2,
        pitch: 0.0,
    };
    hosted.trigger_for_test(pose);
    hosted.step_for_test();

    let placed = last();
    assert_eq!(placed.position, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(
        (placed.yaw, placed.pitch),
        (std::f32::consts::FRAC_PI_2, 0.0)
    );
    assert_eq!(placed.camera_at, Vec3::new(1.0, 2.0, 3.0));
    assert!(
        (placed.camera_facing - Vec3::NEG_X).length() < 1e-4,
        "{placed:?}"
    );

    hosted.step_for_test();
    assert_eq!(last(), placed, "the pose holds");
}
