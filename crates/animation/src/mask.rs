use concerto_foundation::assets::{Asset, CreatableAsset, LoadableAsset, handle::AssetHandle};
use concerto_mesh::{JointName, ReflectSkeletonSource, Skeleton, SkeletonSource};
use concerto_reflect::Reflect;
use serde::{Deserialize, Serialize};

/// Bones an overlay is laid over, by branch, with the skeleton the editor offers bones from.
#[derive(Clone, Debug, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(SkeletonSource)]
pub struct BoneMask {
    pub preview_skeleton: AssetHandle<Skeleton>,
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
    pub bone: JointName,
    pub weight: f32,
    pub depth: u32,
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
    pub weights: Vec<f32>,
    pub missing: Vec<JointName>,
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

/// Resolves `mask` against `skeleton` into one weight per bone, in bone order, with `root` at weight 0.
pub fn resolve_mask(
    mask: &BoneMask,
    skeleton: &Skeleton,
    root: Option<usize>,
    out: &mut MaskResolution,
) {
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

    for (entry, branch) in mask.branches.iter().enumerate() {
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
                let passed = if mask.branches[entry as usize].descendants {
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
            let branch = &mask.branches[cover.entry as usize];
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

#[cfg(test)]
mod tests {
    use concerto_mesh::{JointName, Skeleton, skeleton::JointPose};
    use concerto_name::Name;
    use glam::Mat4;
    use uuid::Uuid;

    use super::{BoneMask, BoneMaskBranch, MaskResolution, resolve_mask};

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
        resolve_mask(mask, &rig(), root, &mut out);
        out
    }

    fn assert_weights(actual: &[f32], expected: &[f32]) {
        assert_eq!(actual.len(), expected.len(), "{actual:?}");
        for (bone, (actual_weight, expected_weight)) in actual.iter().zip(expected).enumerate() {
            assert!(
                (actual_weight - expected_weight).abs() < 1e-6,
                "bone {bone}: {actual:?} != {expected:?}"
            );
        }
    }

    #[test]
    fn a_branch_at_depth_0_gives_its_weight_to_the_bone_and_every_descendant() {
        let mask = BoneMask::new().with_branch(BoneMaskBranch::new("spine2").with_weight(0.8));

        let resolved = resolve(&mask, None);

        assert_weights(
            &resolved.weights,
            &[0.0, 0.0, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8, 0.8],
        );
        assert!(resolved.missing.is_empty());
        assert!(!resolved.names_root);
    }

    #[test]
    fn a_branch_at_depth_2_ramps_in_over_three_levels_and_stays_full_below() {
        let mask = BoneMask::new().with_branch(BoneMaskBranch::new("spine1").with_depth(2));

        let resolved = resolve(&mask, None);

        assert_weights(
            &resolved.weights,
            &[0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        );
    }

    #[test]
    fn an_entry_without_descendants_changes_its_own_bone_alone() {
        let mask = BoneMask::new()
            .with_branch(BoneMaskBranch::new("spine1"))
            .with_branch(
                BoneMaskBranch::new("spine3")
                    .with_weight(0.25)
                    .with_descendants(false),
            );

        let resolved = resolve(&mask, None);

        assert_weights(
            &resolved.weights,
            &[0.0, 1.0, 1.0, 0.25, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        );
    }

    #[test]
    fn a_child_entry_overrides_its_ancestor_and_a_weight_0_child_cuts_its_branch_out() {
        let mask = BoneMask::new()
            .with_branch(BoneMaskBranch::new("spine1"))
            .with_branch(BoneMaskBranch::new("left_upper").with_weight(0.5))
            .with_branch(BoneMaskBranch::new("right_upper").with_weight(0.0));

        let resolved = resolve(&mask, None);

        assert_weights(
            &resolved.weights,
            &[0.0, 1.0, 1.0, 1.0, 0.5, 0.5, 0.5, 0.0, 0.0, 0.0],
        );
    }

    #[test]
    fn unknown_names_are_listed_empty_names_skipped_and_the_later_duplicate_wins() {
        let mask = BoneMask::new()
            .with_branch(BoneMaskBranch::new("tail"))
            .with_branch(BoneMaskBranch::default())
            .with_branch(BoneMaskBranch::new("spine3").with_weight(0.2))
            .with_branch(BoneMaskBranch::new("spine3").with_weight(0.6));

        let resolved = resolve(&mask, None);

        assert_eq!(resolved.missing, vec![JointName::from("tail")]);
        assert_weights(
            &resolved.weights,
            &[0.0, 0.0, 0.0, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6, 0.6],
        );
    }

    #[test]
    fn a_bone_whose_parent_is_not_below_it_has_no_parent() {
        let mut skeleton = skeleton_of(&["a", "b", "c", "d"], vec![None, None, None, Some(0)]);
        skeleton.parents = vec![None, Some(1), Some(3), Some(0)].into_boxed_slice();
        let mask = BoneMask::new().with_branch(BoneMaskBranch::new("a"));
        let mut resolved = MaskResolution::default();

        resolve_mask(&mask, &skeleton, None, &mut resolved);

        assert_weights(&resolved.weights, &[1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_nan_entry_weight_counts_as_0() {
        let mask = BoneMask::new().with_branch(BoneMaskBranch::new("spine1").with_weight(f32::NAN));

        let resolved = resolve(&mask, None);

        assert_weights(&resolved.weights, &[0.0; 10]);
    }

    #[test]
    fn the_root_has_weight_0_even_when_an_entry_names_it() {
        let mask = BoneMask::new().with_branch(BoneMaskBranch::new("root"));

        let resolved = resolve(&mask, Some(0));

        assert!(resolved.names_root);
        assert_weights(
            &resolved.weights,
            &[0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        );
    }

    #[test]
    fn weights_outside_0_to_1_are_clamped() {
        let mask = BoneMask::new()
            .with_branch(BoneMaskBranch::new("spine1").with_weight(2.0))
            .with_branch(BoneMaskBranch::new("left_upper").with_weight(-1.0))
            .with_branch(BoneMaskBranch::new("right_upper").with_weight(f32::INFINITY));

        let resolved = resolve(&mask, None);

        assert_weights(
            &resolved.weights,
            &[0.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        );
    }

    #[test]
    fn a_reused_buffer_resolves_the_same_and_shrinks_to_a_shorter_skeleton() {
        let mask = BoneMask::new()
            .with_branch(BoneMaskBranch::new("root"))
            .with_branch(BoneMaskBranch::new("tail"))
            .with_branch(BoneMaskBranch::new("spine2").with_depth(1));
        let mut resolved = MaskResolution::default();
        resolve_mask(&mask, &rig(), Some(0), &mut resolved);
        let first = resolved.weights.clone();

        resolve_mask(&mask, &rig(), Some(0), &mut resolved);

        assert_eq!(resolved.weights, first);
        assert_eq!(resolved.missing, vec![JointName::from("tail")]);
        assert!(resolved.names_root);

        let short = skeleton_of(&["a", "b"], vec![None, Some(0)]);
        let short_mask = BoneMask::new().with_branch(BoneMaskBranch::new("b").with_weight(0.5));

        resolve_mask(&short_mask, &short, None, &mut resolved);

        assert_weights(&resolved.weights, &[0.0, 0.5]);
        assert!(resolved.missing.is_empty());
        assert!(!resolved.names_root);
    }
}
