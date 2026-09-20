use ecs::{Entity, Query, Res, ResMut, Resource, With};
use essential::{
    time::Time,
    transform::{GlobalTransform, Transform},
};
use glam::{Quat, Vec3};
use render::components::camera::Camera;

use crate::{
    main_camera::MainCamera,
    virtual_camera::{BlendIn, CameraPose, Ease, Lens, VirtualCamera},
};

/// Orders the live [`VirtualCamera`]s into a stack and drives the main camera
/// from whichever one is on top.
///
/// A camera joins the stack when the component is added and leaves when it is
/// removed or the entity is despawned — the component's lifecycle is the whole
/// mechanism, so nothing has to be polled. Position in the stack is `priority`,
/// with later arrivals stacked on top of earlier ones at the same priority, so a
/// cutscene camera takes over by existing with a higher priority and hands
/// control back by going away.
///
/// A camera can also sit in the stack without being eligible for control; the
/// live camera is the highest [`enabled`](StackEntry::enabled) entry. That flag
/// lives on the entry rather than the component precisely so that toggling it
/// goes through the director and needs no change detection.
#[derive(Resource, Default)]
pub struct CameraDirector {
    main_camera: Option<Entity>,
    stack: Vec<StackEntry>,
    state: DirectorState,
    blend: BlendIn,
}

/// A virtual camera's place in the stack.
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
    /// The entity holding the one [`Camera`] that renders to the window.
    /// `None` until [`spawn_main_camera`](crate::main_camera::spawn_main_camera) runs.
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

    /// Takes a camera in or out of contention without disturbing its slot.
    /// A no-op for a camera that is not in the stack.
    pub fn set_enabled(&mut self, vcam: Entity, enabled: bool) {
        let outgoing = self.live();

        let Some(index) = self.index_of(vcam) else {
            return;
        };
        self.stack[index].enabled = enabled;

        self.hand_over_from(outgoing);
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

    pub(crate) fn set_main_camera(&mut self, entity: Entity) {
        self.main_camera = Some(entity);
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
    main_cameras: Query<(&mut Transform, &mut Camera), With<MainCamera>>,
) {
    let Some(main_camera) = director.main_camera() else {
        return;
    };
    let Some((mut transform, mut camera)) = main_cameras.get_entity(main_camera) else {
        return;
    };

    let current_lens = Lens::from_camera(&camera);
    let delta = time.delta().as_secs_f32();

    let live_pose = director
        .live()
        .and_then(|live| sample_pose(live, &vcams, current_lens));

    let mut state = std::mem::replace(&mut director.state, DirectorState::Settled);
    let mut blend_finished = false;

    let pose = match &mut state {
        DirectorState::Settled => live_pose,
        DirectorState::Blending {
            from,
            elapsed,
            duration,
            ease,
        } => match (sample_pose(*from, &vcams, current_lens), live_pose) {
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
        transform.translation = pose.translation;
        transform.rotation = pose.rotation;
        transform.scale = Vec3::ONE;

        pose.lens.apply_to(&mut camera);
    }
}

fn sample_pose(
    vcam: Entity,
    vcams: &Query<(&VirtualCamera, &GlobalTransform)>,
    fallback_lens: Lens,
) -> Option<CameraPose> {
    let (vcam, global_transform) = vcams.get_entity(vcam)?;

    Some(CameraPose {
        translation: global_transform.translation(),
        rotation: normalize(global_transform.rotation()),
        lens: vcam.lens.unwrap_or(fallback_lens),
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
    use ecs::system::executor::single_thread::SingleThreadedExecutor;
    use ecs::system::schedule::Schedule;
    use ecs::world::World;
    use essential::{time::Time, transform::Transform};
    use glam::Vec3;
    use render::components::camera::Camera;

    use super::{CameraDirector, drive_main_camera};
    use crate::{
        main_camera::MainCamera,
        virtual_camera::{BlendIn, Lens, VirtualCamera},
    };

    struct Fixture {
        world: World,
        main_camera: ecs::Entity,
        schedule: ecs::system::schedule::CompiledSchedule,
    }

    impl Fixture {
        fn new() -> Self {
            let mut world = World::new();
            // Inserts the GlobalTransform the director reads poses from.
            world.register_component::<Transform>();
            // Joins/leaves the stack on spawn and despawn.
            world.register_component::<VirtualCamera>();
            world.insert_resource(Time::new());
            world.insert_resource(CameraDirector::default());

            let main_camera = world.spawn((MainCamera, Camera::default(), Transform::IDENTITY));
            world
                .get_resource_mut::<CameraDirector>()
                .unwrap()
                .set_main_camera(main_camera);

            let mut schedule = Schedule::new();
            schedule.add_system(drive_main_camera);
            let schedule = schedule.compile::<SingleThreadedExecutor>(&mut world);

            Self {
                world,
                main_camera,
                schedule,
            }
        }

        fn spawn_vcam(&mut self, vcam: VirtualCamera, at: Vec3) -> ecs::Entity {
            self.world.spawn((vcam, Transform::from_translation(at)))
        }

        /// The only way out of the stack short of despawning.
        fn remove_vcam(&mut self, vcam: ecs::Entity) {
            self.world.remove_component::<VirtualCamera>(vcam);
        }

        fn step(&mut self) -> Vec3 {
            // Advances `Time::delta` off the wall clock, so blends make progress.
            self.world.get_resource_mut::<Time>().unwrap().update();
            self.schedule.run(&mut self.world);
            self.world
                .get_component_for_entity::<Transform>(self.main_camera)
                .unwrap()
                .translation
        }

        fn director(&mut self) -> &mut CameraDirector {
            self.world.get_resource_mut::<CameraDirector>().unwrap()
        }

        fn stacked_cameras(&mut self) -> Vec<ecs::Entity> {
            self.director()
                .stack()
                .iter()
                .map(|entry| entry.camera)
                .collect()
        }
    }

    #[test]
    fn the_top_of_the_stack_drives_the_main_camera() {
        let mut fixture = Fixture::new();
        fixture.spawn_vcam(VirtualCamera::default(), Vec3::X);
        let high = fixture.spawn_vcam(VirtualCamera::new(10), Vec3::Y * 5.0);

        assert_eq!(fixture.step(), Vec3::Y * 5.0);
        assert_eq!(fixture.director().live(), Some(high));
    }

    #[test]
    fn equal_priority_cameras_stack_in_arrival_order() {
        let mut fixture = Fixture::new();
        let first = fixture.spawn_vcam(VirtualCamera::default(), Vec3::X);
        assert_eq!(fixture.step(), Vec3::X);

        let second = fixture.spawn_vcam(VirtualCamera::default(), Vec3::Z);
        assert_eq!(fixture.step(), Vec3::Z);
        assert_eq!(fixture.stacked_cameras(), vec![first, second]);
    }

    #[test]
    fn disabling_the_live_camera_falls_back_down_the_stack() {
        let mut fixture = Fixture::new();
        let low = fixture.spawn_vcam(VirtualCamera::default(), Vec3::X);
        let high = fixture.spawn_vcam(VirtualCamera::new(10), Vec3::Y * 5.0);

        assert_eq!(fixture.step(), Vec3::Y * 5.0);

        fixture.director().set_enabled(high, false);
        assert_eq!(fixture.step(), Vec3::X);
        assert_eq!(fixture.director().live(), Some(low));
        // Disabled, but still holding its slot.
        assert_eq!(fixture.stacked_cameras(), vec![low, high]);
    }

    #[test]
    fn re_enabling_a_camera_restores_its_place_in_the_order() {
        let mut fixture = Fixture::new();
        fixture.spawn_vcam(VirtualCamera::default(), Vec3::X);
        let high = fixture.spawn_vcam(VirtualCamera::new(10), Vec3::Y * 5.0);

        fixture.director().set_enabled(high, false);
        assert_eq!(fixture.step(), Vec3::X);

        fixture.director().set_enabled(high, true);
        assert_eq!(fixture.step(), Vec3::Y * 5.0);
    }

    #[test]
    fn a_camera_spawned_disabled_never_takes_over() {
        let mut fixture = Fixture::new();
        let low = fixture.spawn_vcam(VirtualCamera::default(), Vec3::X);
        let shot = fixture.spawn_vcam(VirtualCamera::new(100).disabled(), Vec3::Y * 50.0);

        assert_eq!(fixture.step(), Vec3::X);
        assert_eq!(fixture.director().live(), Some(low));
        assert!(!fixture.director().is_enabled(shot));
        // In the stack all the same, waiting on its cue.
        assert_eq!(fixture.stacked_cameras(), vec![low, shot]);
    }

    #[test]
    fn disabling_the_last_enabled_camera_leaves_the_director_settled() {
        let mut fixture = Fixture::new();
        fixture.director().set_blend(BlendIn::linear(0.25));

        let only = fixture.spawn_vcam(VirtualCamera::default(), Vec3::X);
        assert_eq!(fixture.step(), Vec3::X);

        // The outgoing camera still exists here, so a blend towards nothing
        // would never advance and never finish.
        fixture.director().set_enabled(only, false);
        for _ in 0..64 {
            fixture.step();
        }

        assert!(fixture.director().live().is_none());
        assert!(
            !fixture.director().is_blending(),
            "the director is still blending with nothing to blend towards"
        );
        // Holds the last pose it was given.
        assert_eq!(fixture.step(), Vec3::X);
    }

    #[test]
    fn removing_the_component_drops_the_live_camera_down_the_stack() {
        let mut fixture = Fixture::new();
        let low = fixture.spawn_vcam(VirtualCamera::default(), Vec3::X);
        let high = fixture.spawn_vcam(VirtualCamera::new(10), Vec3::Y * 5.0);

        assert_eq!(fixture.step(), Vec3::Y * 5.0);

        fixture.remove_vcam(high);
        assert_eq!(fixture.step(), Vec3::X);
        assert_eq!(fixture.stacked_cameras(), vec![low]);
    }

    #[test]
    fn re_adding_the_component_re_slots_a_camera_at_its_new_priority() {
        let mut fixture = Fixture::new();
        let low = fixture.spawn_vcam(VirtualCamera::default(), Vec3::X);
        let high = fixture.spawn_vcam(VirtualCamera::new(10), Vec3::Y * 5.0);

        assert_eq!(fixture.step(), Vec3::Y * 5.0);

        // Priority is fixed at insertion, so changing it means re-adding.
        fixture.remove_vcam(low);
        fixture.world.insert(VirtualCamera::new(20), low);

        assert_eq!(fixture.step(), Vec3::X);
        assert_eq!(fixture.stacked_cameras(), vec![high, low]);
    }

    #[test]
    fn despawning_the_live_camera_falls_back_down_the_stack() {
        let mut fixture = Fixture::new();
        let low = fixture.spawn_vcam(VirtualCamera::default(), Vec3::X);
        let high = fixture.spawn_vcam(VirtualCamera::new(10), Vec3::Y * 5.0);

        assert_eq!(fixture.step(), Vec3::Y * 5.0);

        fixture.world.despawn(high);
        assert_eq!(fixture.stacked_cameras(), vec![low]);
        assert_eq!(fixture.step(), Vec3::X);
    }

    #[test]
    fn a_blend_starts_at_the_outgoing_camera_instead_of_snapping() {
        let mut fixture = Fixture::new();
        fixture.director().set_blend(BlendIn::linear(10.0));
        fixture.spawn_vcam(VirtualCamera::default(), Vec3::Y * 5.0);
        assert_eq!(fixture.step(), Vec3::Y * 5.0);

        fixture.spawn_vcam(VirtualCamera::new(10), Vec3::X * 100.0);
        let blended = fixture.step();

        assert!(
            blended.distance(Vec3::Y * 5.0) < 1.0,
            "a 10s blend should barely have moved off the outgoing pose, got {blended}"
        );
        assert!(fixture.director().is_blending());
    }

    #[test]
    fn a_finished_blend_settles_on_the_stack_top() {
        let mut fixture = Fixture::new();
        // Short enough that one frame of wall-clock delta completes it.
        fixture
            .director()
            .set_blend(BlendIn::linear(f32::MIN_POSITIVE));
        fixture.spawn_vcam(VirtualCamera::default(), Vec3::Y * 5.0);
        fixture.step();

        let high = fixture.spawn_vcam(VirtualCamera::new(10), Vec3::X);
        assert_eq!(fixture.step(), Vec3::X);
        assert!(!fixture.director().is_blending());
        assert_eq!(fixture.director().live(), Some(high));
    }

    #[test]
    fn emptying_the_stack_leaves_the_director_settled() {
        let mut fixture = Fixture::new();
        fixture.director().set_blend(BlendIn::linear(0.25));

        let only = fixture.spawn_vcam(VirtualCamera::default(), Vec3::X);
        fixture.step();
        assert!(!fixture.director().is_blending());

        fixture.remove_vcam(only);
        for _ in 0..64 {
            fixture.step();
        }

        assert!(
            !fixture.director().is_blending(),
            "the director is still blending long after the stack emptied"
        );
    }

    #[test]
    fn a_virtual_camera_lens_overrides_the_main_camera_lens() {
        let mut fixture = Fixture::new();
        fixture.spawn_vcam(
            VirtualCamera::default().with_lens(Lens {
                fovy: 1.0,
                znear: 0.5,
                zfar: 500.0,
            }),
            Vec3::ZERO,
        );
        fixture.step();

        let camera = fixture
            .world
            .get_component_for_entity::<Camera>(fixture.main_camera)
            .unwrap();
        assert_eq!(camera.fovy, 1.0);
        assert_eq!(camera.zfar, 500.0);
    }
}
