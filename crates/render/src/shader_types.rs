use glam::{Mat4, Vec3, Vec4};

/// A WGSL `vec3<f32>` written through encase.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GpuVec3([f32; 3]);

/// A WGSL `vec4<f32>` written through encase.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GpuVec4([f32; 4]);

/// A WGSL `mat4x4<f32>` written through encase, column by column.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GpuMat4([[f32; 4]; 4]);

encase::impl_vector!(3, GpuVec3, f32; using AsRef AsMut From);
encase::impl_vector!(4, GpuVec4, f32; using AsRef AsMut From);
encase::impl_matrix!(4, 4, GpuMat4, f32; using AsRef AsMut From);

impl AsRef<[f32; 3]> for GpuVec3 {
    fn as_ref(&self) -> &[f32; 3] {
        &self.0
    }
}

impl AsMut<[f32; 3]> for GpuVec3 {
    fn as_mut(&mut self) -> &mut [f32; 3] {
        &mut self.0
    }
}

impl From<[f32; 3]> for GpuVec3 {
    fn from(value: [f32; 3]) -> Self {
        Self(value)
    }
}

impl From<Vec3> for GpuVec3 {
    fn from(value: Vec3) -> Self {
        Self(value.to_array())
    }
}

impl From<GpuVec3> for Vec3 {
    fn from(value: GpuVec3) -> Self {
        Vec3::from_array(value.0)
    }
}

impl AsRef<[f32; 4]> for GpuVec4 {
    fn as_ref(&self) -> &[f32; 4] {
        &self.0
    }
}

impl AsMut<[f32; 4]> for GpuVec4 {
    fn as_mut(&mut self) -> &mut [f32; 4] {
        &mut self.0
    }
}

impl From<[f32; 4]> for GpuVec4 {
    fn from(value: [f32; 4]) -> Self {
        Self(value)
    }
}

impl From<Vec4> for GpuVec4 {
    fn from(value: Vec4) -> Self {
        Self(value.to_array())
    }
}

impl From<GpuVec4> for Vec4 {
    fn from(value: GpuVec4) -> Self {
        Vec4::from_array(value.0)
    }
}

impl AsRef<[[f32; 4]; 4]> for GpuMat4 {
    fn as_ref(&self) -> &[[f32; 4]; 4] {
        &self.0
    }
}

impl AsMut<[[f32; 4]; 4]> for GpuMat4 {
    fn as_mut(&mut self) -> &mut [[f32; 4]; 4] {
        &mut self.0
    }
}

impl From<[[f32; 4]; 4]> for GpuMat4 {
    fn from(value: [[f32; 4]; 4]) -> Self {
        Self(value)
    }
}

impl From<Mat4> for GpuMat4 {
    fn from(value: Mat4) -> Self {
        Self(value.to_cols_array_2d())
    }
}

impl From<GpuMat4> for Mat4 {
    fn from(value: GpuMat4) -> Self {
        Mat4::from_cols_array_2d(&value.0)
    }
}
