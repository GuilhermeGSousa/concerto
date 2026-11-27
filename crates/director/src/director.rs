use concerto_ecs::{Changed, Entity, Query, Res, ResMut, Resource, With, command::CommandQueue};
use concerto_foundation::{
    time::Time,
    transform::{GlobalTransform, Transform},
};
use concerto_render::{DepthOfField, components::camera::Camera};
use glam::{Mat4, Quat};

use crate::{
    main_camera::MainCamera,
    virtual_camera::{BlendIn, CameraPose, Ease, Lens, VirtualCamera},
};

/// Orders the live [`VirtualCamera`]s into a stack and drives the main camera
/// from whichever one is on top.
///
/// A camera joins the stack when the component is added and leaves when it is
/// removed or the entity is despawned. Position in the stack is `priority`, with
/// later arrivals stacked on top of earlier ones at the same priority, so a
/// cutscene camera takes over by existing with a higher priority and hands
/// control back by going away.
///
/// A camera can also sit in the stack without being eligible for control; the
/// live camera is the highest [`enabled`](StackEntry::enabled) entry. The
/// [`VirtualCamera`] component is the source of truth for `priority` and
/// `enabled`: the entry mirrors them, and [`sync_virtual_cameras`] re-sorts the
/// stack on the next run after either is edited, so [`live`](Self::live) and
/// [`is_live`](Self::is_live) follow an edit one frame later.
#[derive(Resource, Default)]
pub struct CameraDirector {
    main_camera: Option<Entity>,
    stack: Vec<StackEntry>,
    state: DirectorState,
    blend: BlendIn,
}

/// A virtual camera's place in the stack, mirrored from its component.
pub struct StackEntry {
    pub camera: Entity,
    pub priority: i32,
    pub enabled: bool,
}

/// The stack top is always what the view is heading for, so the only thing left
/// to track is whether it has arrived.
#[derive(Default)]
enum DirectorState {
    /// Driving the top of the stack directly, or nothing when it is empty.
    #[default]
    Settled,
    /// Catching up with the top from the camera that just lost control.
    Blending {
        from: Entity,
        elapsed: f32,
        duration: f32,
        ease: Ease,
    },
}

impl CameraDirector {
    /// The [`MainCamera`] being driven, the one with the lowest entity index.
    ///
    /// It is `None` while the level has none.
    pub fn main_camera(&self) -> Option<Entity> {
        self.main_camera
    }

    /// The highest enabled virtual camera in the stack — the one in control.
    pub fn live(&self) -> Option<Entity> {
        self.stack
            .iter()
            .rev()
            .find(|entry| entry.enabled)
            .map(|entry| entry.camera)
    }

    pub fn is_live(&self, vcam: Entity) -> bool {
        self.live() == Some(vcam)
    }

    /// Virtual cameras, lowest priority first. Includes disabled entries.
    pub fn stack(&self) -> &[StackEntry] {
        &self.stack
    }

    pub fn is_enabled(&self, vcam: Entity) -> bool {
        self.entry(vcam).is_some_and(|entry| entry.enabled)
    }

    /// Slots a camera in above everything of the same priority, so equal
    /// priorities run in arrival order. Called from [`VirtualCamera`]'s `on_add`.
    pub(crate) fn stack_insert(&mut self, camera: Entity, priority: i32, enabled: bool) {
        let outgoing = self.live();
        self.detach(camera);
        let index = self
            .stack
            .partition_point(|entry| entry.priority <= priority);
        self.stack.insert(
            index,
            StackEntry {
                camera,
                priority,
                enabled,
            },
        );
        self.hand_over_from(outgoing);
    }

    pub(crate) fn stack_sync(&mut self, camera: Entity, priority: i32, enabled: bool) {
        let Some(entry) = self.entry(camera) else {
            self.stack_insert(camera, priority, enabled);
            return;
        };
        if entry.priority != priority {
            self.stack_insert(camera, priority, enabled);
            return;
        }
        if entry.enabled == enabled {
            return;
        }

        let outgoing = self.live();
        if let Some(index) = self.index_of(camera) {
            self.stack[index].enabled = enabled;
        }
        self.hand_over_from(outgoing);
    }

    fn entry(&self, camera: Entity) -> Option<&StackEntry> {
        self.stack.iter().find(|entry| entry.camera == camera)
    }

    fn index_of(&self, camera: Entity) -> Option<usize> {
        self.stack.iter().position(|entry| entry.camera == camera)
    }

    /// Called from [`VirtualCamera`]'s removal and despawn callbacks.
    pub(crate) fn stack_remove(&mut self, camera: Entity) {
        let outgoing = self.live();
        self.detach(camera);
        self.hand_over_from(outgoing);
    }

    fn detach(&mut self, camera: Entity) {
        if let Some(index) = self.index_of(camera) {
            self.stack.remove(index);
        }
    }

    /// Starts a blend when a stack change moved control to a different camera.
    /// Only the stack can change who is live, so this is the only place a blend
    /// begins.
    fn hand_over_from(&mut self, outgoing: Option<Entity>) {
        if self.live() == outgoing {
            return;
        }

        // Nothing left to blend towards: hold the last pose rather than starting
        // a blend that can never advance.
        if self.live().is_none() {
            self.state = DirectorState::Settled;
            return;
        }

        self.state = match outgoing {
            Some(from) if self.blend.duration > 0.0 => DirectorState::Blending {
                from,
                elapsed: 0.0,
                duration: self.blend.duration,
                ease: self.blend.ease,
            },
            _ => DirectorState::Settled,
        };
    }

    /// True while the view is still catching up with the top of the stack.
    pub fn is_blending(&self) -> bool {
        matches!(self.state, DirectorState::Blending { .. })
    }

    /// How the view catches up when control changes hands. Defaults to a cut.
    pub fn blend(&self) -> BlendIn {
        self.blend
    }

    pub fn set_blend(&mut self, blend: BlendIn) {
        self.blend = blend;
    }

    pub(crate) fn set_main_camera(&mut self, entity: Option<Entity>) {
        self.main_camera = entity;
    }
}

/// Re-syncs the stack from every [`VirtualCamera`] whose component changed.
pub fn sync_virtual_cameras(
    mut director: ResMut<CameraDirector>,
    vcams: Query<(Entity, &VirtualCamera), Changed<VirtualCamera>>,
) {
    for (entity, vcam) in vcams.iter() {
        director.stack_sync(entity, vcam.priority(), vcam.is_enabled());
    }
}

/// Writes the pose of the live virtual camera onto the main camera.
///
/// Runs in `Update`, so the `Transform` it writes is propagated to
/// `GlobalTransform` later the same frame and uploaded by the render extract
/// after that. The virtual-camera pose it reads comes from the *previous*
/// frame's propagation, which costs one frame of camera lag.
pub fn drive_main_camera(
    mut director: ResMut<CameraDirector>,
    time: Res<Time>,
    vcams: Query<(&VirtualCamera, &GlobalTransform)>,
    mut main_cameras: Query<
        (
            &mut Transform,
            &GlobalTransform,
            &mut Camera,
            Option<&mut DepthOfField>,
        ),
        With<MainCamera>,
    >,
    mut cmd: CommandQueue,
) {
    let Some(main_camera) = director.main_camera() else {
        return;
    };
    let Some((mut transform, global, mut camera, mut focus)) =
        main_cameras.get_entity_mut(main_camera)
    else {
        return;
    };

    let current = Fallback {
        lens: Lens::from_camera(&camera),
        focus: focus.as_ref().map(|focus| ***focus),
    };
    let delta = time.delta().as_secs_f32();

    let live_pose = director
        .live()
        .and_then(|live| sample_pose(live, &vcams, current));

    let mut state = std::mem::replace(&mut director.state, DirectorState::Settled);
    let mut blend_finished = false;

    let pose = match &mut state {
        DirectorState::Settled => live_pose,
        DirectorState::Blending {
            from,
            elapsed,
            duration,
            ease,
        } => match (sample_pose(*from, &vcams, current), live_pose) {
            (Some(from_pose), Some(live_pose)) => {
                *elapsed += delta;
                let t = ease.apply(*elapsed / *duration);
                blend_finished = *elapsed >= *duration;
                Some(from_pose.lerp(live_pose, t))
            }
            // Outgoing camera despawned mid-blend: nothing left to blend from.
            (None, live_pose) => {
                blend_finished = true;
                live_pose
            }
            (Some(_), None) => None,
        },
    };

    if blend_finished {
        state = DirectorState::Settled;
    }
    director.state = state;

    if let Some(pose) = pose {
        let world = Mat4::from_rotation_translation(pose.rotation, pose.translation);
        let local = transform.compute_matrix() * global.matrix().inverse() * world;
        let (scale, rotation, translation) = local.to_scale_rotation_translation();
        transform.translation = translation;
        transform.rotation = normalize(rotation);
        transform.scale = scale;

        pose.lens.apply_to(&mut camera);
        if let Some(wanted) = pose.focus {
            match focus.as_mut() {
                Some(current) if ***current != wanted => ***current = wanted,
                Some(_) => {}
                None => cmd.insert(wanted, main_camera),
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Fallback {
    lens: Lens,
    focus: Option<DepthOfField>,
}

fn sample_pose(
    vcam: Entity,
    vcams: &Query<(&VirtualCamera, &GlobalTransform)>,
    fallback: Fallback,
) -> Option<CameraPose> {
    let (vcam, global_transform) = vcams.get_entity(vcam)?;

    Some(CameraPose {
        translation: global_transform.translation(),
        rotation: normalize(global_transform.rotation()),
        lens: vcam.lens.unwrap_or(fallback.lens),
        focus: vcam.focus.or(fallback.focus),
    })
}

fn normalize(rotation: Quat) -> Quat {
    if rotation.is_finite() && rotation.length_squared() > f32::EPSILON {
        rotation.normalize()
    } else {
        Quat::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use concerto_ecs::IntoSystemConfig;
    use concerto_ecs::reflect::{AppTypeRegistry, ReflectComponent};
    use concerto_ecs::system::executor::single_thread::SingleThreadedExecutor;
    use concerto_ecs::system::schedule::Schedule;
    use concerto_ecs::world::World;
    use concerto_foundation::{time::Time, transform::Transform};
    use concerto_reflect::{apply_value, to_value};
    use concerto_render::{DepthOfField, components::camera::Camera};
    use glam::Vec3;

    use super::{CameraDirector, drive_main_camera, sync_virtual_cameras};
    use concerto_foundation::transform::systems::update_simple_entities;

    use crate::{
        main_camera::{MainCamera, adopt_main_camera},
        virtual_camera::{BlendIn, Lens, VirtualCamera},
    };

    struct Fixture {
        world: World,
        main_camera: concerto_ecs::Entity,
        schedule: concerto_ecs::system::schedule::CompiledSchedule,
    }

    impl Fixture {
        fn new() -> Self {
            let mut world = World::new();
            // Inserts the GlobalTransform the director reads poses from.
            world.register_component::<Transform>();
            // Joins/leaves the stack on spawn and despawn.
            world.register_component::<VirtualCamera>();
            world.register_type::<VirtualCamera>();
            world.insert_resource(Time::new());
            world.insert_resource(CameraDirector::default());

            let main_camera = world.spawn((MainCamera, Camera::default(), Transform::IDENTITY));

            let mut schedule = Schedule::new();
            schedule.add_system(drive_main_camera);
            schedule.add_system(sync_virtual_cameras.before(drive_main_camera));
            schedule.add_system(adopt_main_camera.before(drive_main_camera));
            schedule.add_system(update_simple_entities.after(drive_main_camera));
            let schedule = schedule.compile::<SingleThreadedExecutor>(&mut world);

            Self {
                world,
                main_camera,
                schedule,
            }
        }

        fn spawn_vcam(&mut self, vcam: VirtualCamera, at: Vec3) -> concerto_ecs::Entity {
            self.world.spawn((vcam, Transform::from_translation(at)))
        }

        /// The only way out of the stack short of despawning.
        fn remove_vcam(&mut self, vcam: concerto_ecs::Entity) {
            self.world.remove_component::<VirtualCamera>(vcam);
        }

        fn step(&mut self) -> Vec3 {
            // Advances `Time::delta` off the wall clock, so blends make progress.
            self.world.get_resource_mut::<Time>().unwrap().update();
            self.schedule.run(&mut self.world);
            self.world.tick();
            self.world
                .get_component_for_entity::<Transform>(self.main_camera)
                .unwrap()
                .translation
        }

        fn set_enabled(&mut self, vcam: concerto_ecs::Entity, enabled: bool) {
            self.world
                .get_component_for_entity_mut::<VirtualCamera>(vcam)
                .unwrap()
                .set_enabled(enabled);
        }

        fn edit_vcam(&mut self, vcam: concerto_ecs::Entity, replacement: &VirtualCamera) {
            let stored = to_value(replacement).unwrap();
            let reflect = self
                .world
                .get_resource::<AppTypeRegistry>()
                .unwrap()
                .get_type_data::<ReflectComponent>(TypeId::of::<VirtualCamera>())
                .expect("a virtual camera is an editable component");
            assert!((reflect.edit)(&mut self.world, vcam, &mut |value| {
                apply_value(value, &stored);
            }));
        }

        fn focus(&self) -> Option<DepthOfField> {
            self.world
                .get_component_for_entity::<DepthOfField>(self.main_camera)
                .copied()
        }

        fn director(&mut self) -> &mut CameraDirector {
            self.world.get_resource_mut::<CameraDirector>().unwrap()
        }

        fn stacked_cameras(&mut self) -> Vec<concerto_ecs::Entity> {
            self.director()
                .stack()
                .iter()
                .map(|entry| entry.camera)
                .collect()
        }
    }

    #[test]
    fn the_highest_enabled_camera_drives_the_main_camera_as_cameras_come_go_and_change() {
        let mut fixture = Fixture::new();
        let low = fixture.spawn_vcam(
            VirtualCamera::default().with_lens(Lens {
                fovy: 1.0,
                znear: 0.5,
                zfar: 500.0,
            }),
            Vec3::X,
        );
        assert_eq!(fixture.step(), Vec3::X);
        let camera = fixture
            .world
            .get_component_for_entity::<Camera>(fixture.main_camera)
            .unwrap();
        assert_eq!(camera.fovy, 1.0);
        assert_eq!(camera.zfar, 500.0);

        let high = fixture.spawn_vcam(VirtualCamera::new(10), Vec3::Y * 5.0);
        let shot = fixture.spawn_vcam(VirtualCamera::new(100).disabled(), Vec3::Y * 50.0);
        assert_eq!(fixture.step(), Vec3::Y * 5.0);
        assert_eq!(fixture.director().live(), Some(high));

        let same = fixture.spawn_vcam(VirtualCamera::new(10), Vec3::Z);
        assert_eq!(fixture.step(), Vec3::Z);
        fixture.world.despawn(same);
        assert_eq!(fixture.step(), Vec3::Y * 5.0);

        fixture.set_enabled(high, false);
        assert_eq!(fixture.step(), Vec3::X);
        fixture.set_enabled(high, true);
        assert_eq!(fixture.step(), Vec3::Y * 5.0);

        fixture.edit_vcam(low, &VirtualCamera::new(20));
        assert_eq!(fixture.step(), Vec3::X);
        assert_eq!(fixture.stacked_cameras(), vec![high, low, shot]);

        fixture.set_enabled(shot, true);
        assert_eq!(fixture.step(), Vec3::Y * 50.0);

        fixture.remove_vcam(shot);
        assert_eq!(fixture.step(), Vec3::X);
        assert_eq!(fixture.director().live(), Some(low));
    }

    #[test]
    fn a_takeover_blends_from_the_outgoing_camera_and_settles_on_the_new_one() {
        let mut fixture = Fixture::new();
        fixture.director().set_blend(BlendIn::linear(10.0));
        fixture.spawn_vcam(VirtualCamera::default(), Vec3::Y * 5.0);
        assert_eq!(fixture.step(), Vec3::Y * 5.0);

        let far_focus = DepthOfField::at(8.0);
        fixture.spawn_vcam(
            VirtualCamera::new(10).with_focus(far_focus),
            Vec3::X * 100.0,
        );
        let blended = fixture.step();

        assert!(blended.distance(Vec3::Y * 5.0) < 1.0, "{blended}");
        assert!(fixture.director().is_blending());
        fixture.step();
        let opening = fixture.focus().expect("the main camera takes the focus");
        assert_eq!(opening.focus_distance, 8.0);
        assert!(
            opening.max_blur < far_focus.max_blur * 0.5,
            "a focus taken from a camera without one opens its blur over the blend: {}",
            opening.max_blur
        );

        fixture
            .director()
            .set_blend(BlendIn::linear(f32::MIN_POSITIVE));
        let near_focus = DepthOfField::at(2.0);
        let top = fixture.spawn_vcam(VirtualCamera::new(20).with_focus(near_focus), Vec3::Z);
        assert_eq!(fixture.step(), Vec3::Z);
        fixture.step();
        assert_eq!(fixture.focus(), Some(near_focus));
        assert!(!fixture.director().is_blending());
        assert_eq!(fixture.director().live(), Some(top));
    }
}
