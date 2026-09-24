use std::ops::Mul;

use concerto_ecs::component::scene::{SceneComponent, SceneSpawnContext};
use concerto_ecs::component::{Component, ComponentLifecycleCallback};
use concerto_ecs::entity::Entity;
use concerto_editable::Editable;
use glam::{Affine3A, Mat4, Quat, Vec3};

use crate::blend::Blendable;

pub mod systems;

#[derive(Clone, Blendable, Editable, serde::Serialize, serde::Deserialize)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
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

impl SceneComponent for Transform {
    fn apply(self, entity: Entity, ctx: &mut SceneSpawnContext<'_>) {
        ctx.insert(self, entity);
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    pub fn compute_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    pub fn from_translation_rotation(translation: Vec3, rotation: Quat) -> Self {
        Self {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }

    pub fn from_translation_rotation_scale(translation: Vec3, rotation: Quat, scale: Vec3) -> Self {
        Self {
            translation,
            rotation,
            scale,
        }
    }

    pub fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    pub fn from_rotation(rotation: Quat) -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation,
            scale: Vec3::ONE,
        }
    }

    pub fn from_scale(scale: Vec3) -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale,
        }
    }

    pub fn with_translation(mut self, translation: Vec3) -> Self {
        self.translation = translation;
        self
    }

    pub fn with_rotation(mut self, rotation: Quat) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn with_scale(mut self, scale: Vec3) -> Self {
        self.scale = scale;
        self
    }

    pub fn from_matrix(matrix: &[[f32; 4]; 4]) -> Self {
        let (scale, rotation, translation) =
            Mat4::from_cols_array_2d(matrix).to_scale_rotation_translation();
        Self {
            translation,
            rotation,
            scale,
        }
    }

    pub fn look_to(&mut self, direction: Vec3, up: Vec3) {
        self.rotation = Quat::look_to_rh(direction, up);
    }

    pub fn look_at(&mut self, center: Vec3, up: Vec3) {
        self.rotation = Quat::look_at_rh(self.translation, center, up)
    }

    pub fn local_x(&self) -> Vec3 {
        self.rotation * Vec3::X
    }

    pub fn local_y(&self) -> Vec3 {
        self.rotation * Vec3::Y
    }

    pub fn local_z(&self) -> Vec3 {
        self.rotation * Vec3::Z
    }

    pub fn up(&self) -> Vec3 {
        self.local_y()
    }

    pub fn down(&self) -> Vec3 {
        -self.up()
    }

    pub fn forward(&self) -> Vec3 {
        -self.local_z()
    }

    pub fn backward(&self) -> Vec3 {
        -self.forward()
    }

    pub fn right(&self) -> Vec3 {
        self.local_x()
    }

    pub fn left(&self) -> Vec3 {
        -self.right()
    }

    pub fn transform_point(&self, mut point: Vec3) -> Vec3 {
        point = self.scale * point;
        point = self.rotation * point;
        point += self.translation;
        point
    }

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

#[derive(Component)]
pub struct GlobalTransform(Affine3A);

impl GlobalTransform {
    pub fn new(transform: Mat4) -> Self {
        Self(Affine3A::from_mat4(transform))
    }

    pub fn matrix(&self) -> Mat4 {
        Mat4::from(self.0)
    }

    pub fn set_matrix(&mut self, transform: Mat4) {
        self.0 = Affine3A::from_mat4(transform)
    }

    pub fn translation(&self) -> Vec3 {
        self.0.translation.into()
    }

    pub fn rotation(&self) -> Quat {
        self.0.to_scale_rotation_translation().1
    }

    pub fn scale(&self) -> Vec3 {
        self.0.to_scale_rotation_translation().0
    }

    pub fn to_transform(&self) -> Transform {
        let (scale, rotation, translation) = self.0.to_scale_rotation_translation();
        Transform {
            translation,
            rotation,
            scale,
        }
    }

    pub fn to_raw(&self) -> GlobalTransformRaw {
        GlobalTransformRaw {
            matrix: self.matrix().to_cols_array_2d(),
        }
    }

    pub fn local_x(&self) -> Vec3 {
        self.rotation() * Vec3::X
    }

    pub fn local_y(&self) -> Vec3 {
        self.rotation() * Vec3::Y
    }

    pub fn local_z(&self) -> Vec3 {
        self.rotation() * Vec3::Z
    }

    pub fn up(&self) -> Vec3 {
        self.local_y()
    }

    pub fn down(&self) -> Vec3 {
        -self.up()
    }

    pub fn forward(&self) -> Vec3 {
        -self.local_z()
    }

    pub fn backward(&self) -> Vec3 {
        -self.forward()
    }

    pub fn right(&self) -> Vec3 {
        self.local_x()
    }

    pub fn left(&self) -> Vec3 {
        -self.right()
    }
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlobalTransformRaw {
    matrix: [[f32; 4]; 4],
}

#[cfg(test)]
mod editable_tests {
    use super::Transform;
    use concerto_editable::{with_property, Editable, PropertyPath, PropertyVisitor};
    use glam::{Quat, Vec3};
    use std::any::{Any, TypeId};

    #[test]
    fn transform_exposes_translation_rotation_and_scale() {
        struct Fields(Vec<(&'static str, TypeId)>);
        impl PropertyVisitor for Fields {
            fn field(&mut self, name: &'static str, value: &dyn Editable) {
                self.0.push((name, (value as &dyn Any).type_id()));
            }
        }
        let transform = Transform::from_translation_rotation_scale(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::from_rotation_y(90_f32.to_radians()),
            Vec3::ONE,
        );
        let mut fields = Fields(Vec::new());
        transform.visit(&mut fields);
        assert_eq!(
            fields.0,
            vec![
                ("translation", TypeId::of::<Vec3>()),
                ("rotation", TypeId::of::<Quat>()),
                ("scale", TypeId::of::<Vec3>())
            ]
        );
        with_property(
            &transform,
            &PropertyPath::new(["translation"]),
            &mut |value| {
                assert_eq!(
                    (value as &dyn Any).downcast_ref::<Vec3>(),
                    Some(&transform.translation)
                );
            },
        )
        .unwrap();
        with_property(&transform, &PropertyPath::new(["rotation"]), &mut |value| {
            assert_eq!(
                (value as &dyn Any).downcast_ref::<Quat>(),
                Some(&transform.rotation)
            );
        })
        .unwrap();
    }
}
