use concerto_ecs::component::{Component, ComponentLifecycleCallback};
use concerto_ecs::reflect::{ReflectComponent, ReflectSceneComponent};
use concerto_reflect::{Reflect, ReflectDefault};
use concerto_render::{DepthOfField, components::camera::Camera};
use glam::{Quat, Vec3};

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
///
/// `priority` and `enabled` are edited in place, through reflection or
/// [`set_enabled`](Self::set_enabled); the director re-sorts its stack on its
/// next run.
#[derive(Reflect)]
#[reflect(Component, Default, SceneComponent)]
pub struct VirtualCamera {
    /// Editable in place: the director re-sorts the stack on its next run.
    priority: i32,
    /// Whether the camera is eligible to be live. Editable in place: the
    /// director picks the change up on its next run.
    ///
    /// Omitted from a stored scene means "eligible", the default: a camera
    /// that never mentions `enabled` still works.
    enabled: bool,
    /// `None` keeps whatever lens the main camera currently has. Free to edit —
    /// the director reads it every frame. Absent from a stored scene is `None`.
    pub lens: Option<Lens>,
    /// `None` keeps whatever focus the main camera currently has, or none. A camera that takes over from one without a focus opens its blur over the blend.
    pub focus: Option<DepthOfField>,
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
            focus: None,
        }
    }

    /// Gives the camera a focus, which the main camera takes while this one is live.
    pub fn with_focus(mut self, focus: DepthOfField) -> Self {
        self.focus = Some(focus);
        self
    }

    pub fn with_lens(mut self, lens: Lens) -> Self {
        self.lens = Some(lens);
        self
    }

    /// Joins the stack in its slot but passed over for control until
    /// [`set_enabled`](Self::set_enabled) turns it on — how a scene ships cutscene cameras that must not hijack the view
    /// the moment they load.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn priority(&self) -> i32 {
        self.priority
    }

    /// Whether this camera is eligible for control.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Takes the camera in or out of contention without disturbing its slot.
    /// The director applies it on its next run.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// Projection parameters, split out of [`Camera`] so they can be overridden and
/// blended independently of the render target and clear colour.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub struct Lens {
    pub fovy: f32,
    pub znear: f32,
    pub zfar: f32,
}

impl Default for Lens {
    fn default() -> Self {
        Self::from_camera(&Camera::default())
    }
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
    pub focus: Option<DepthOfField>,
}

impl CameraPose {
    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self {
            translation: self.translation.lerp(other.translation, t),
            rotation: self.rotation.slerp(other.rotation, t),
            lens: self.lens.lerp(other.lens, t),
            focus: match (self.focus, other.focus) {
                (Some(from), Some(to)) => Some(from.lerp(to, t)),
                (None, Some(to)) => Some(to.faded(t)),
                (Some(from), None) => Some(from.faded(1.0 - t)),
                (None, None) => None,
            },
        }
    }
}
