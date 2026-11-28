use concerto_foundation::assets::{Asset, CreatableAsset, LoadableAsset, handle::AssetHandle};
use concerto_mesh::{JointName, ReflectSkeletonSource, Skeleton, SkeletonSource};
use concerto_reflect::Reflect;
use serde::{Deserialize, Serialize};

/// Bones an overlay is laid over, by branch, with the skeleton the editor offers bones from.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(SkeletonSource)]
pub struct BoneMask {
    /// The skeleton the editor offers bones from; the mask applies to any skeleton by bone name.
    pub preview_skeleton: AssetHandle<Skeleton>,
    /// The branches, each naming a bone.
    pub branches: Vec<BoneMaskBranch>,
}

impl BoneMask {
    /// A mask with no preview skeleton and no branches.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the skeleton the editor offers bones from.
    pub fn with_preview_skeleton(mut self, preview_skeleton: AssetHandle<Skeleton>) -> Self {
        self.preview_skeleton = preview_skeleton;
        self
    }

    /// Adds one branch, after the ones already there.
    pub fn with_branch(mut self, branch: BoneMaskBranch) -> Self {
        self.branches.push(branch);
        self
    }

    /// Resolves `mask` against `skeleton` into one weight per bone, in bone order, with `root` at weight 0.
    pub fn resolve_into(&self, skeleton: &Skeleton, root: Option<usize>, out: &mut MaskResolution) {
        let bones = skeleton.bone_count();
        out.weights.clear();
        out.weights.resize(bones, 0.0);
        out.missing.clear();
        out.names_root = false;
        out.entry_at.clear();
        out.entry_at.resize(bones, None);
        out.covering.clear();
        out.covering.resize(bones, None);
        out.inherited.clear();
        out.inherited.resize(bones, None);

        for (entry, branch) in self.branches.iter().enumerate() {
            if branch.bone.is_empty() {
                continue;
            }
            match skeleton.bone_index(branch.bone.0) {
                Some(bone) if bone < bones => {
                    out.entry_at[bone] = Some(entry as u32);
                    if Some(bone) == root {
                        out.names_root = true;
                    }
                }
                _ => {
                    if !out.missing.contains(&branch.bone) {
                        out.missing.push(branch.bone);
                    }
                }
            }
        }

        for bone in 0..bones {
            let from_parent = skeleton
                .parents
                .get(bone)
                .copied()
                .flatten()
                .map(usize::from)
                .filter(|parent| *parent < bone)
                .and_then(|parent| out.inherited[parent])
                .map(Cover::one_level_down);
            let (covering, inherited) = match out.entry_at[bone] {
                Some(entry) => {
                    let own = Cover { entry, levels: 0 };
                    let passed = if self.branches[entry as usize].descendants {
                        Some(own)
                    } else {
                        from_parent
                    };
                    (Some(own), passed)
                }
                None => (from_parent, from_parent),
            };
            out.covering[bone] = covering;
            out.inherited[bone] = inherited;

            if Some(bone) == root {
                continue;
            }
            if let Some(cover) = covering {
                let branch = &self.branches[cover.entry as usize];
                let weight = if branch.weight.is_nan() {
                    0.0
                } else {
                    branch.weight
                };
                let ramp = ((cover.levels as f32 + 1.0) / (branch.depth as f32 + 1.0)).min(1.0);
                out.weights[bone] = (weight * ramp).clamp(0.0, 1.0);
            }
        }
    }
}

impl Asset for BoneMask {
    const FORMAT_VERSION: u32 = 1;

    fn name() -> &'static str {
        "BoneMask"
    }
}

impl LoadableAsset for BoneMask {}

impl CreatableAsset for BoneMask {
    fn new_asset() -> Self {
        Self::new()
    }
}

impl SkeletonSource for BoneMask {
    fn skeleton(&self) -> Option<&AssetHandle<Skeleton>> {
        Some(&self.preview_skeleton)
    }
}

/// One entry of a mask: a bone, the weight it gives, how many levels the weight ramps in over, and whether it covers the bone's descendants.
#[derive(Clone, Debug, PartialEq, Reflect, Serialize, Deserialize)]
pub struct BoneMaskBranch {
    /// The bone the branch starts at, by name.
    pub bone: JointName,
    /// The weight the branch gives, clamped to 0 to 1 when resolved; NaN counts as 0.
    pub weight: f32,
    /// The number of levels below the bone over which the weight ramps up to full.
    ///
    /// At 0 the bone itself has the full weight; at `n` the bone has `1 / (n + 1)` of it and the
    /// bone `n` levels down has all of it.
    pub depth: u32,
    /// Whether the branch's weight passes down to the bone's descendants.
    pub descendants: bool,
}

impl Default for BoneMaskBranch {
    fn default() -> Self {
        Self {
            bone: JointName::default(),
            weight: 1.0,
            depth: 0,
            descendants: true,
        }
    }
}

impl BoneMaskBranch {
    /// A branch on `bone` at full weight, with no ramp, covering its descendants.
    pub fn new(bone: impl Into<JointName>) -> Self {
        Self {
            bone: bone.into(),
            ..Self::default()
        }
    }

    /// Sets the weight the branch gives.
    pub fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// Sets how many levels below the bone the weight ramps in over.
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    /// Sets whether the branch covers the bone's descendants.
    pub fn with_descendants(mut self, descendants: bool) -> Self {
        self.descendants = descendants;
        self
    }
}

/// A mask resolved against one skeleton, in buffers reused from frame to frame.
#[derive(Default)]
pub struct MaskResolution {
    /// The weight of each bone, in bone order, from 0 to 1.
    pub weights: Vec<f32>,
    /// The bones the mask names that the skeleton does not have.
    pub missing: Vec<JointName>,
    /// Whether the mask names the root bone, which always gets weight 0.
    pub names_root: bool,
    entry_at: Vec<Option<u32>>,
    covering: Vec<Option<Cover>>,
    inherited: Vec<Option<Cover>>,
}

#[derive(Clone, Copy)]
struct Cover {
    entry: u32,
    levels: u32,
}

impl Cover {
    fn one_level_down(self) -> Self {
        Self {
            entry: self.entry,
            levels: self.levels.saturating_add(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use concerto_mesh::{Skeleton, skeleton::JointPose};
    use concerto_name::Name;
    use glam::Mat4;
    use uuid::Uuid;

    use super::{BoneMask, BoneMaskBranch, MaskResolution};

    const NAMES: [&str; 10] = [
        "root",
        "spine1",
        "spine2",
        "spine3",
        "left_upper",
        "left_lower",
        "left_hand",
        "right_upper",
        "right_lower",
        "right_hand",
    ];

    fn skeleton_of(names: &[&str], parents: Vec<Option<u16>>) -> Skeleton {
        let count = names.len();
        Skeleton::new(
            vec![Mat4::IDENTITY; count],
            parents,
            names.iter().map(|name| Name::new(name)).collect(),
            vec![JointPose::IDENTITY; count],
            (0..count as u128).map(Uuid::from_u128).collect(),
        )
        .unwrap()
    }

    fn rig() -> Skeleton {
        skeleton_of(
            &NAMES,
            vec![
                None,
                Some(0),
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(3),
                Some(7),
                Some(8),
            ],
        )
    }

    fn resolve(mask: &BoneMask, root: Option<usize>) -> MaskResolution {
        let mut out = MaskResolution::default();
        mask.resolve_into(&rig(), root, &mut out);
        out
    }

    struct MaskCase {
        name: &'static str,
        mask: BoneMask,
        root: Option<usize>,
        weights: [f32; 10],
    }

    fn mask_case(name: &'static str, mask: BoneMask, weights: [f32; 10]) -> MaskCase {
        MaskCase {
            name,
            mask,
            root: None,
            weights,
        }
    }

    #[test]
    fn a_mask_resolves_to_one_weight_per_bone() {
        let third = 1.0 / 3.0;
        let cases = [
            mask_case(
                "a branch at depth 2 ramps in over three levels and stays full below",
                BoneMask::new().with_branch(BoneMaskBranch::new("spine1").with_depth(2)),
                [0.0, third, 2.0 * third, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            ),
            mask_case(
                "an entry without descendants changes its own bone alone",
                BoneMask::new()
                    .with_branch(BoneMaskBranch::new("spine1"))
                    .with_branch(
                        BoneMaskBranch::new("spine3")
                            .with_weight(0.25)
                            .with_descendants(false),
                    ),
                [0.0, 1.0, 1.0, 0.25, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            ),
            mask_case(
                "a child entry overrides its ancestor and a weight 0 child cuts its branch out",
                BoneMask::new()
                    .with_branch(BoneMaskBranch::new("spine1"))
                    .with_branch(BoneMaskBranch::new("left_upper").with_weight(0.5))
                    .with_branch(BoneMaskBranch::new("right_upper").with_weight(0.0)),
                [0.0, 1.0, 1.0, 1.0, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0],
            ),
            MaskCase {
                root: Some(0),
                ..mask_case(
                    "the root has weight 0 even when an entry names it",
                    BoneMask::new().with_branch(BoneMaskBranch::new("root")),
                    [0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
                )
            },
        ];

        for case in cases {
            let resolved = resolve(&case.mask, case.root);

            assert_eq!(resolved.weights.len(), case.weights.len(), "{}", case.name);
            for (bone, (actual, expected)) in resolved.weights.iter().zip(case.weights).enumerate()
            {
                assert!(
                    (actual - expected).abs() < 1e-6,
                    "{}: bone {bone} is {actual}, not {expected}",
                    case.name
                );
            }
        }
    }
}
