use std::ops::Mul;

use concerto_ecs::component::{Component, ComponentLifecycleCallback};
use concerto_ecs::reflect::{ReflectComponent, ReflectSceneComponent};
use concerto_reflect::{Reflect, ReflectDefault};
use glam::{Affine3A, Mat4, Quat, Vec3};

use crate::blend::Blendable;

/// The systems that write [`GlobalTransform`] from [`Transform`] down the hierarchy.
pub mod systems;

/// An entity's position, rotation and scale relative to its parent, or to the world when it has
/// none.
///
/// Adding it adds a [`GlobalTransform`], which the transform systems keep up to date; removing it
/// removes that.
#[derive(Clone, Blendable, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(Component, Default, SceneComponent)]
pub struct Transform {
    /// The position relative to the parent.
    pub translation: Vec3,
    /// The rotation relative to the parent.
    pub rotation: Quat,
    /// The scale along each local axis; 1.0 is unscaled.
    pub scale: Vec3,
}

impl Component for Transform {
    fn on_add() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            let global_transform = GlobalTransform::new(
                world
                    .get_component_for_entity::<Transform>(context.entity)
                    .unwrap()
                    .compute_matrix(),
            );

            world.insert(global_transform, context.entity);
        })
    }

    fn on_remove() -> Option<ComponentLifecycleCallback> {
        Some(|mut world, context| {
            world.remove_component::<GlobalTransform>(context.entity);
        })
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// No translation or rotation, and a scale of 1.0.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Returns the matrix that scales, then rotates, then translates.
    pub fn compute_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Creates a transform with a scale of 1.0.
    pub fn from_translation_rotation(translation: Vec3, rotation: Quat) -> Self {
        Self {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }

    /// Creates a transform from all three parts.
    pub fn from_translation_rotation_scale(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            translation,
            rotation,
            scale,
        }
    }

    /// Creates a transform at `translation` with no rotation and a scale of 1.0.
    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// Creates a transform with `rotation`, at the origin with a scale of 1.0.
    pub fn from_rotation(rotation: Quat) -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation,
            scale: Vec3::ONE,
        }
    }

    /// Creates a transform with `scale`, at the origin with no rotation.
    pub fn from_scale(scale: Vec3) -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale,
        }
    }

    /// Returns the transform with its translation replaced.
    pub fn with_translation(mut self, translation: Vec3) -> Self {
        self.translation = translation;
        self
    }

    /// Returns the transform with its rotation replaced.
    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    /// Returns the transform with its scale replaced.
    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    /// Decomposes a column-major matrix, which must hold no shear, into a transform.
    pub fn from_matrix(matrix: &[[f32; 4]; 4]) -> Self {
        let (scale, rotation, translation) =
            Mat4::from_cols_array_2d(matrix).to_scale_rotation_translation();
        Self {
            translation,
            rotation,
            scale,
        }
    }

    /// Rotates the transform so its forward axis, -Z, points along `direction`, with its up axis as
    /// close to `up` as it can be.
    pub fn look_to(&mut self, direction: Vec3, up: Vec3) {
        self.rotation = glam::camera::rh::view::look_to_quat(direction, up);
    }

    /// Rotates the transform so its forward axis, -Z, points from its translation toward `center`,
    /// with its up axis as close to `up` as it can be.
    pub fn look_at(&mut self, center: Vec3, up: Vec3) {
        self.rotation = glam::camera::rh::view::look_at_quat(self.translation, center, up)
    }

    /// Returns the local X axis in the parent's space.
    pub fn local_x(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    /// Returns the local Y axis in the parent's space.
    pub fn local_y(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    /// Returns the local Z axis in the parent's space.
    pub fn local_z(&self) -> Vec3 {
        self.rotation * Vec3::Z
    }

    /// Returns the up direction, local +Y, in the parent's space.
    pub fn up(&self) -> Vec3 {
        self.local_y()
    }

    /// Returns the down direction, local -Y, in the parent's space.
    pub fn down(&self) -> Vec3 {
        -self.up()
    }

    /// Returns the forward direction, local -Z, in the parent's space.
    pub fn forward(&self) -> Vec3 {
        -self.local_z()
    }

    /// Returns the backward direction, local +Z, in the parent's space.
    pub fn backward(&self) -> Vec3 {
        -self.forward()
    }

    /// Returns the right direction, local +X, in the parent's space.
    pub fn right(&self) -> Vec3 {
        self.local_x()
    }

    /// Returns the left direction, local -X, in the parent's space.
    pub fn left(&self) -> Vec3 {
        -self.right()
    }

    /// Returns `point` taken from this transform's space into its parent's: scaled, rotated, then
    /// translated.
    pub fn transform_point(&self, mut point: Vec3) -> Vec3 {
        point = self.scale * point;
        point = self.rotation * point;
        point += self.translation;
        point
    }

    /// Returns `other` placed in this transform's space: the composition of `self` applied after
    /// `other`, with scales multiplied per axis.
    pub fn mul_transform(&self, other: Transform) -> Transform {
        let translation = self.transform_point(other.translation);
        let rotation = self.rotation * other.rotation;
        let scale = self.scale * other.scale;

        Transform {
            translation,
            rotation,
            scale,
        }
    }
}

impl Mul<Transform> for Transform {
    type Output = Transform;

    fn mul(self, rhs: Transform) -> Self::Output {
        self.mul_transform(rhs)
    }
}

/// An entity's transform in world space, written by the transform systems from its [`Transform`]
/// and its ancestors'.
///
/// Read it; write the [`Transform`] instead.
#[derive(Component, PartialEq)]
pub struct GlobalTransform(Affine3A);

impl GlobalTransform {
    /// Creates a global transform from an affine matrix.
    pub fn new(transform: Mat4) -> Self {
        Self(Affine3A::from_mat4(transform))
    }

    /// Returns the world-space matrix.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from(self.0)
    }

    /// Replaces the world-space matrix, which must be affine.
    pub fn set_matrix(&mut self, transform: Mat4) {
        self.0 = Affine3A::from_mat4(transform)
    }

    /// Returns the world-space position.
    pub fn translation(&self) -> Vec3 {
        self.0.translation.into()
    }

    /// Returns the world-space rotation, with any scale removed.
    pub fn rotation(&self) -> Quat {
        self.0.to_scale_rotation_translation().1
    }

    /// Returns the world-space scale along each local axis.
    pub fn scale(&self) -> Vec3 {
        self.0.to_scale_rotation_translation().0
    }

    /// Decomposes the world-space matrix into a [`Transform`].
    pub fn to_transform(&self) -> Transform {
        let (scale, rotation, translation) = self.0.to_scale_rotation_translation();
        Transform {
            translation,
            rotation,
            scale,
        }
    }

    /// Returns the matrix in the layout the GPU reads.
    pub fn to_raw(&self) -> GlobalTransformRaw {
        GlobalTransformRaw {
            matrix: self.matrix().to_cols_array_2d(),
        }
    }

    /// Returns the local X axis in world space, without scale.
    pub fn local_x(&self) -> Vec3 {
        self.rotation() * Vec3::X
    }

    /// Returns the local Y axis in world space, without scale.
    pub fn local_y(&self) -> Vec3 {
        self.rotation() * Vec3::Y
    }

    /// Returns the local Z axis in world space, without scale.
    pub fn local_z(&self) -> Vec3 {
        self.rotation() * Vec3::Z
    }

    /// Returns the up direction, local +Y, in world space.
    pub fn up(&self) -> Vec3 {
        self.local_y()
    }

    /// Returns the down direction, local -Y, in world space.
    pub fn down(&self) -> Vec3 {
        -self.up()
    }

    /// Returns the forward direction, local -Z, in world space.
    pub fn forward(&self) -> Vec3 {
        -self.local_z()
    }

    /// Returns the backward direction, local +Z, in world space.
    pub fn backward(&self) -> Vec3 {
        -self.forward()
    }

    /// Returns the right direction, local +X, in world space.
    pub fn right(&self) -> Vec3 {
        self.local_x()
    }

    /// Returns the left direction, local -X, in world space.
    pub fn left(&self) -> Vec3 {
        -self.right()
    }
}

/// A [`GlobalTransform`]'s matrix as a column-major `[[f32; 4]; 4]`, for upload to the GPU.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlobalTransformRaw {
    matrix: [[f32; 4]; 4],
}
