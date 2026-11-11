use std::path::{Path, PathBuf};

use concerto_foundation::assets::content::check_encoding;
use concerto_foundation::assets::Asset;
use concerto_foundation::transform::Transform;
use concerto_reflect::{apply_value, to_value, ReflectedValue};
use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Asset, Serialize, Deserialize)]
struct GlamLeaves {
    transform: ReflectedValue,
    vec3: ReflectedValue,
    quat: ReflectedValue,
    mat4: ReflectedValue,
}

fn encoding_dir() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/encoding")).to_path_buf()
}

fn transform() -> Transform {
    Transform {
        translation: Vec3::new(1.5, -2.25, 3.0),
        rotation: Quat::from_xyzw(0.1, 0.2, 0.3, 0.927_362),
        scale: Vec3::new(0.5, 2.0, 4.0),
    }
}

fn vec3() -> Vec3 {
    Vec3::new(-7.0, 0.125, 1e6)
}

fn quat() -> Quat {
    Quat::from_xyzw(-0.5, 0.5, -0.5, 0.5)
}

fn mat4() -> Mat4 {
    Mat4::from_cols_array(&std::array::from_fn(|index| index as f32 * 0.5 - 3.0))
}

#[test]
fn reflected_glam_leaves_keep_their_encoding() {
    let leaves = GlamLeaves {
        transform: to_value(&transform()).unwrap(),
        vec3: to_value(&vec3()).unwrap(),
        quat: to_value(&quat()).unwrap(),
        mat4: to_value(&mat4()).unwrap(),
    };

    check_encoding(&leaves, &encoding_dir());

    let fixture = std::fs::read(encoding_dir().join("GlamLeaves.v1.bin")).unwrap();
    let stored: GlamLeaves = bincode::deserialize(&fixture).unwrap();

    let mut read_transform = Transform::default();
    assert!(apply_value(&mut read_transform, &stored.transform).is_empty());
    assert_eq!(read_transform.translation, transform().translation);
    assert_eq!(read_transform.rotation, transform().rotation);
    assert_eq!(read_transform.scale, transform().scale);

    let mut read_vec3 = Vec3::ZERO;
    assert!(apply_value(&mut read_vec3, &stored.vec3).is_empty());
    assert_eq!(read_vec3, vec3());

    let mut read_quat = Quat::IDENTITY;
    assert!(apply_value(&mut read_quat, &stored.quat).is_empty());
    assert_eq!(read_quat, quat());

    let mut read_mat4 = Mat4::IDENTITY;
    assert!(apply_value(&mut read_mat4, &stored.mat4).is_empty());
    assert_eq!(read_mat4, mat4());
}
