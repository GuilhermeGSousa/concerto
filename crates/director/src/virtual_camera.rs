use ecs::Entity;
use ecs::component::{
    Component, ComponentLifecycleCallback,
    scene::{SceneComponent, SceneSpawnContext},
};
use glam::{Quat, Vec3};
use render::components::camera::Camera;
use serde::{Deserialize, Serialize};

use crate::director::CameraDirector;

/// A pose (and optionally lens) the main camera can adopt, without owning any
/// GPU resources.
///
/// The pose is the entity's own `GlobalTransform`, so a virtual camera can be
/// driven by anything that moves a transform: a follow rig, an animation clip,
/// parenting, or a camera authored in Blender.
///
/// Having the component *is* being in the [`CameraDirector`](crate::CameraDirector)'s
/// stack, sorted by [`priority`](Self::priority); the top of that stack is live.
/// Leave the stack by removing the component or despawning the entity.
#[derive(Serialize, Deserialize)]
pub struct VirtualCamera {
    /// Read-only: the director sorts the stack when the component is added, so
    /// an in-place edit would leave the stack out of order. Re-add the component
    /// to change it.
    priority: i32,
    /// Whether the camera is eligible to be live *when it joins the stack*. From
    /// then on the director's [`StackEntry`](crate::director::StackEntry) holds
    /// the flag — toggle it with
    /// [`CameraDirector::set_enabled`](crate::CameraDirector::set_enabled), not
    /// here, since an edit to this field is invisible to the director.
    ///
    /// Omitted from authored JSON means "eligible": a camera an artist never
    /// mentions `enabled` on still works.
    #[serde(default = "default_enabled")]
    enabled: bool,
    /// `None` keeps whatever lens the main camera currently has. Free to edit —
    /// the director reads it every frame. Absent from authored JSON is `None`.
    #[serde(default)]
    pub lens: Option<Lens>,
}

fn default_enabled() -> bool {
    true
}

/// Joining and leaving the director's stack is driven by the component's own
/// lifecycle, so spawning or despawning a camera is all it takes to enter or
/// leave the running order — including despawns, which no query can observe.
impl Component for VirtualCamera {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let Some(vcam) = world.get_component_for_entity::<VirtualCamera>(context.entity) else {
                return;
            };
            let priority = vcam.priority;
            let enabled = vcam.enabled;

            if let Some(director) = world.get_resource_mut::<CameraDirector>() {
                director.stack_insert(context.entity, priority, enabled);
            }
        })
    }


    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            if let Some(director) = world.get_resource_mut::<CameraDirector>() {
                director.stack_remove(context.entity);
            }
        })
    }
}

impl SceneComponent for VirtualCamera {
    fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        ctx.insert(self, entity);
    }
}

impl Default for VirtualCamera {
    fn default() -> Self {
        Self::new(0)
    }
}

impl VirtualCamera {
    /// Higher priority stacks above lower; equal priorities run in the order
    /// they were added.
    pub fn new(priority: i32) -> Self {
        Self {
            priority,
            enabled: true,
            lens: None,
        }
    }

    pub fn with_lens(mut self, lens: Lens) -> Self {
        self.lens = Some(lens);
        self
    }

    /// Joins the stack in its slot but passed over for control until
    /// [`CameraDirector::set_enabled`](crate::CameraDirector::set_enabled) turns
    /// it on — how a scene ships cutscene cameras that must not hijack the view
    /// the moment they load.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn priority(&self) -> i32 {
        self.priority
    }
}

/// Projection parameters, split out of [`Camera`] so they can be overridden and
/// blended independently of the render target and clear colour.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Lens {
    pub fovy: f32,
    pub znear: f32,
    pub zfar: f32,
}

impl Lens {
    pub fn from_camera(camera: &Camera) -> Self {
        Self {
            fovy: camera.fovy,
            znear: camera.znear,
            zfar: camera.zfar,
        }
    }

    /// Writes the lens onto a camera, leaving `aspect` alone — that belongs to
    /// the render crate, which syncs it from the surface.
    pub fn apply_to(&self, camera: &mut Camera) {
        camera.fovy = self.fovy;
        camera.znear = self.znear;
        camera.zfar = self.zfar;
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            fovy: self.fovy + (other.fovy - self.fovy) * t,
            znear: self.znear + (other.znear - self.znear) * t,
            zfar: self.zfar + (other.zfar - self.zfar) * t,
        }
    }
}

/// How a virtual camera takes over from whatever was live before it.
#[derive(Clone, Copy)]
pub struct BlendIn {
    pub duration: f32,
    pub ease: Ease,
}

impl BlendIn {
    /// Snap instantly.
    pub const CUT: Self = Self {
        duration: 0.0,
        ease: Ease::Linear,
    };

    pub fn smooth(duration: f32) -> Self {
        Self {
            duration,
            ease: Ease::SmoothStep,
        }
    }

    pub fn linear(duration: f32) -> Self {
        Self {
            duration,
            ease: Ease::Linear,
        }
    }
}

impl Default for BlendIn {
    fn default() -> Self {
        Self::CUT
    }
}

#[derive(Clone, Copy, Default)]
#[repr(u8)]
pub enum Ease {
    Linear,
    #[default]
    SmoothStep,
    EaseIn,
    EaseOut,
}

impl Ease {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Ease::Linear => t,
            Ease::SmoothStep => t * t * (3.0 - 2.0 * t),
            Ease::EaseIn => t * t,
            Ease::EaseOut => t * (2.0 - t),
        }
    }
}

/// A resolved camera position in world space, ready to be written to the main
/// camera. Decoupled from any entity so a blend can outlive the virtual camera
/// it started from.
#[derive(Clone, Copy)]
pub struct CameraPose {
    pub translation: Vec3,
    pub rotation: Quat,
    pub lens: Lens,
}

impl CameraPose {
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t),
            lens: self.lens.lerp(other.lens, t),
        }
    }
}

#[cfg(test)]
mod tests {
    use ecs::system::executor::single_thread::SingleThreadedExecutor;
    use ecs::system::schedule::Schedule;
    use ecs::{CommandQueue, Component, Entity, Res, Resource, world::World};

    use super::VirtualCamera;
    use crate::director::CameraDirector;

    #[derive(Resource)]
    struct AuthoredComponent {
        node: Entity,
        json: &'static str,
    }

    fn insert_authored_camera(mut cmd: CommandQueue, authored: Res<AuthoredComponent>) {
        cmd.insert_from_json(
            VirtualCamera::name().to_string(),
            authored.json.to_string(),
            authored.node,
        );
    }

    /// Blender-authored components arrive as GLTF extras and are built through
    /// serde, which still reaches the private fields.
    fn spawn_from_json(json: &'static str) -> (World, Entity) {
        let mut world = World::new();
        world.register_component::<VirtualCamera>();
        world.register_component_type::<VirtualCamera>();
        world.insert_resource(CameraDirector::default());

        let node = world.spawn(());
        world.insert_resource(AuthoredComponent { node, json });

        let mut schedule = Schedule::new();
        schedule.add_system(insert_authored_camera);
        schedule
            .compile::<SingleThreadedExecutor>(&mut world)
            .run(&mut world);

        (world, node)
    }

    #[test]
    fn a_json_authored_camera_joins_the_stack_at_its_priority() {
        let (mut world, node) = spawn_from_json(r#"{ "priority": 7, "lens": null }"#);

        let vcam = world
            .get_component_for_entity::<VirtualCamera>(node)
            .expect("reflection should have inserted the component");
        assert_eq!(vcam.priority(), 7);

        let director = world.get_resource_mut::<CameraDirector>().unwrap();
        assert_eq!(director.stack()[0].priority, 7);
        // An author who never mentions `enabled` gets a camera that works.
        assert!(director.is_enabled(node));
        assert_eq!(director.live(), Some(node));
    }

    #[test]
    fn a_json_authored_camera_can_ship_disabled() {
        let (mut world, node) = spawn_from_json(r#"{ "priority": 7, "enabled": false }"#);

        assert!(
            world
                .get_component_for_entity::<VirtualCamera>(node)
                .is_some(),
            "reflection should have inserted the component before we check it shipped disabled"
        );

        let director = world.get_resource_mut::<CameraDirector>().unwrap();
        assert!(!director.is_enabled(node));
        assert!(director.live().is_none());
    }
}
