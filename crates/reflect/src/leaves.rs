use concerto_color::Color;
use glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use uuid::Uuid;

/// Implements [`Reflect`](crate::Reflect) as a [`Leaf`](crate::Leaf) encoded through serde.
#[macro_export]
macro_rules! impl_reflect_leaf {
    ($($leaf:ty),* $(,)?) => {
        $(
            impl $crate::Leaf for $leaf {
                fn encode(&self) -> ::std::result::Result<::std::vec::Vec<u8>, $crate::LeafError> {
                    $crate::encode_leaf(self)
                }

                fn decode(&mut self, bytes: &[u8]) -> ::std::result::Result<(), $crate::LeafError> {
                    $crate::decode_leaf(self, bytes)
                }
            }

            impl $crate::Reflect for $leaf {
                fn reflect_ref(&self) -> $crate::ReflectRef<'_> {
                    $crate::ReflectRef::Leaf(self)
                }

                fn reflect_mut(&mut self) -> $crate::ReflectMut<'_> {
                    $crate::ReflectMut::Leaf(self)
                }
            }

            impl $crate::GetTypeRegistration for $leaf {
                fn get_type_registration() -> $crate::TypeRegistration {
                    $crate::TypeRegistration::of::<Self>()
                }
            }
        )*
    };
}

impl_reflect_leaf!(
    bool, u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, f32, f64, String, Vec2, Vec3, Vec4,
    Quat, Mat4, Color, Uuid,
);
