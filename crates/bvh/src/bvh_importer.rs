use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use concerto_animation::clip::{AnimationChanelOutput, AnimationChannel, AnimationClip};
use concerto_asset_import::{ImportContext, ImportError, Importer, clip_names};
use concerto_ecs::component::scene::SceneEntityRef;
use concerto_foundation::assets::{AssetId, handle::AssetHandle};
use concerto_foundation::transform::Transform;
use concerto_mesh::{JointPose, Skeleton, SkeletonComponent};
use concerto_reflect::Reflect;
use concerto_scene::scene::{Scene, SceneNode};
use glam::{Mat4, Quat, Vec3};
use serde::Deserialize;
use uuid::Uuid;

use crate::parser::{self, Bvh, Channel};

const DEFAULT_SCALE: f32 = 0.01;

fn default_scale() -> f32 {
    DEFAULT_SCALE
}

/// Importer settings read from the `[settings]` table of a BVH source's sidecar.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BvhSettings {
    /// Multiplier applied to every offset and translation; defaults to 0.01 (centimetres to metres).
    #[serde(default = "default_scale")]
    pub scale: f32,
    /// Frames to keep as `[start, end)`; the whole file when absent.
    #[serde(default)]
    pub frame_range: Option<[usize; 2]>,
}

impl Default for BvhSettings {
    fn default() -> Self {
        Self {
            scale: DEFAULT_SCALE,
            frame_range: None,
        }
    }
}

impl BvhSettings {
    /// Reads and validates the settings table.
    pub fn from_table(table: &toml::Table) -> Result<Self, String> {
        let settings: BvhSettings = table.clone().try_into().map_err(|err| err.to_string())?;
        if !settings.scale.is_finite() || settings.scale <= 0.0 {
            return Err(format!(
                "scale must be finite and positive, got {}",
                settings.scale
            ));
        }
        Ok(settings)
    }
}

/// Imports BVH motion capture files.
pub struct BvhImporter;

impl Importer for BvhImporter {
    fn supported_extensions(&self) -> &'static [&'static str] {
        &["bvh"]
    }

    fn import(&self, source_path: &Path, ctx: &mut ImportContext) -> Result<(), ImportError> {
        let malformed = |message: String| ImportError::MalformedSource {
            source_path: source_path.to_path_buf(),
            message,
        };
        let text =
            std::fs::read_to_string(source_path).map_err(|err| ImportError::SourceUnreadable {
                source_path: source_path.to_path_buf(),
                message: err.to_string(),
            })?;
        let bvh = parser::parse(&text).map_err(|err| malformed(err.to_string()))?;
        let settings = BvhSettings::from_table(ctx.settings()).map_err(malformed)?;

        let frame_count = bvh.frames.len();
        let [start, end] = settings.frame_range.unwrap_or([0, frame_count]);
        if start >= end || end > frame_count {
            return Err(malformed(format!(
                "frame_range [{start}, {end}) must satisfy start < end <= {frame_count} frames"
            )));
        }
        if end - start < 2 {
            return Err(malformed(
                "the clip needs at least two frames after trimming".to_string(),
            ));
        }

        let bone_ids = bone_ids(&bvh);
        let skeleton = build_skeleton(&bvh, settings.scale, &bone_ids).map_err(malformed)?;
        let clip = build_clip(&bvh, settings.scale, start..end, &bone_ids);
        ctx.emit("skeleton/0", &skeleton)?;
        let stem = source_path.file_stem().and_then(|stem| stem.to_str());
        let clip_name = clip_names(&[stem]).remove(0);
        if clip_name != "0" {
            ctx.renamed_from(&format!("animation/{clip_name}"), "animation/0");
        }
        ctx.emit(&format!("animation/{clip_name}"), &clip)?;
        let scene = build_scene(&bvh, &skeleton, ctx.sub_asset_id("skeleton/0"))?;
        ctx.emit("scene", &scene)?;
        Ok(())
    }
}

fn bone_ids(bvh: &Bvh) -> Vec<Uuid> {
    let mut paths: Vec<String> = Vec::with_capacity(bvh.joints.len());
    for joint in &bvh.joints {
        let path = match joint.parent {
            Some(parent) => format!("{}/{}", paths[parent], joint.name),
            None => joint.name.clone(),
        };
        paths.push(path);
    }
    paths
        .iter()
        .map(|path| {
            let mut hasher = DefaultHasher::new();
            path.hash(&mut hasher);
            Uuid::from_u128(hasher.finish() as u128)
        })
        .collect()
}

fn build_skeleton(bvh: &Bvh, scale: f32, bone_ids: &[Uuid]) -> Result<Skeleton, String> {
    let mut parents = Vec::with_capacity(bvh.joints.len());
    let mut rest_pose = Vec::with_capacity(bvh.joints.len());
    let mut globals: Vec<Mat4> = Vec::with_capacity(bvh.joints.len());
    for joint in &bvh.joints {
        let parent = match joint.parent {
            Some(parent) => Some(u16::try_from(parent).map_err(|_| {
                format!(
                    "more than {} joints are not supported",
                    u16::MAX as usize + 1
                )
            })?),
            None => None,
        };
        parents.push(parent);
        let local = Mat4::from_translation(Vec3::from_array(joint.offset) * scale);
        globals.push(match joint.parent {
            Some(parent) => globals[parent] * local,
            None => local,
        });
        rest_pose.push(JointPose {
            translation: Vec3::from_array(joint.offset) * scale,
            ..JointPose::IDENTITY
        });
    }
    Skeleton::new(
        globals.iter().map(Mat4::inverse).collect(),
        parents,
        bvh.joints.iter().map(|joint| joint.name.clone()).collect(),
        rest_pose,
        bone_ids.to_vec(),
    )
    .map_err(|err| err.to_string())
}

fn push_node_component<T: Reflect>(node: &mut SceneNode, component: &T) -> Result<(), ImportError> {
    node.push_component(component)
        .map_err(|err| ImportError::SerializationFailed {
            sub_asset_name: "scene".to_string(),
            message: err.to_string(),
        })
}

fn build_scene(bvh: &Bvh, skeleton: &Skeleton, skeleton_id: AssetId) -> Result<Scene, ImportError> {
    let mut nodes: Vec<SceneNode> = Vec::with_capacity(bvh.joints.len());
    for (index, joint) in bvh.joints.iter().enumerate() {
        let mut node = SceneNode {
            name: joint.name.clone(),
            children: Vec::new(),
            components: Vec::new(),
        };
        let pose = &skeleton.rest_pose[index];
        push_node_component(
            &mut node,
            &Transform::from_translation_rotation_scale(
                pose.translation,
                pose.rotation,
                pose.scale,
            ),
        )?;
        if index == 0 {
            push_node_component(
                &mut node,
                &SkeletonComponent {
                    skeleton: AssetHandle::weak(skeleton_id),
                    bones: (0..bvh.joints.len()).map(SceneEntityRef::Index).collect(),
                    root: Some(SceneEntityRef::Index(0)),
                },
            )?;
        }
        nodes.push(node);
        if let Some(parent) = joint.parent {
            nodes[parent].children.push(index);
        }
    }
    Ok(Scene {
        nodes,
        referenced_assets: vec![skeleton_id],
    })
}

fn build_clip(
    bvh: &Bvh,
    scale: f32,
    frames: std::ops::Range<usize>,
    bone_ids: &[Uuid],
) -> AnimationClip {
    let times: Vec<f32> = (0..frames.len())
        .map(|frame| frame as f32 * bvh.frame_time)
        .collect();
    let mut clip = AnimationClip::default();
    clip.set_frame_rate(bvh.frame_time.recip());
    for (joint, id) in bvh.joints.iter().zip(bone_ids) {
        let has_position = joint.channels.iter().any(|channel| channel.is_position());
        let has_rotation = joint.channels.iter().any(|channel| !channel.is_position());

        if has_position {
            let rest = Vec3::from_array(joint.offset) * scale;
            let translations = bvh.frames[frames.clone()].iter().map(|row| {
                let mut translation = rest;
                for (index, channel) in joint.channels.iter().enumerate() {
                    let value = row[joint.channel_start + index] * scale;
                    match channel {
                        Channel::XPosition => translation.x = value,
                        Channel::YPosition => translation.y = value,
                        Channel::ZPosition => translation.z = value,
                        _ => {}
                    }
                }
                translation.to_array()
            });
            clip.add_channel(
                *id,
                AnimationChannel::new(
                    times.clone(),
                    AnimationChanelOutput::from_translation(translations),
                ),
            );
        }

        if has_rotation {
            let mut previous: Option<Quat> = None;
            let rotations = bvh.frames[frames.clone()].iter().map(|row| {
                let mut rotation = Quat::IDENTITY;
                for (index, channel) in joint.channels.iter().enumerate() {
                    let angle = row[joint.channel_start + index].to_radians();
                    match channel {
                        Channel::XRotation => rotation *= Quat::from_rotation_x(angle),
                        Channel::YRotation => rotation *= Quat::from_rotation_y(angle),
                        Channel::ZRotation => rotation *= Quat::from_rotation_z(angle),
                        _ => {}
                    }
                }
                let mut rotation = rotation.normalize();
                if previous.is_some_and(|previous| previous.dot(rotation) < 0.0) {
                    rotation = -rotation;
                }
                previous = Some(rotation);
                rotation.to_array()
            });
            clip.add_channel(
                *id,
                AnimationChannel::new(
                    times.clone(),
                    AnimationChanelOutput::from_rotation(rotations),
                ),
            );
        }
    }
    clip
}
