use std::path::Path;

use anyhow::Context;
use concerto_animation::{
    clip::AnimationClip,
    retarget::{ReferencePose, RetargetRig},
};
use concerto_ecs::{
    Component, Entity, Event, Query, Res, ResMut,
    command::CommandQueue,
    events::{event_reader::EventReader, event_writer::EventWriter},
    signal::{On, Signal},
};
use concerto_editor::{
    asset_editor::valid_asset_name,
    project::{AssetsWritten, ProjectState},
};
use concerto_foundation::assets::{
    Asset, AssetId,
    content::{decode_content_asset, read_asset_record, save_content_asset_into},
    handle::AssetHandle,
    registry::AssetRegistry,
};
use concerto_mcp::{PendingCall, ToolCalls, ToolError};
use concerto_mesh::Skeleton;
use concerto_ragdoll::{
    RagdollPart, RagdollShape,
    generate::{HumanoidChains, HumanoidSettings, generate_humanoid},
};
use schemars::JsonSchema;
use serde::Deserialize;

const DEFAULT_NAME: &str = "ragdoll";
const EXTENSION: &str = "gasset";

/// The names of the rig's chains that replace the defaults the generator looks for; a chain left out keeps its default.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, PartialEq)]
pub struct ChainNames {
    /// The spine chain; spine without it.
    pub spine: Option<String>,
    /// The neck chain, ending at the head; neck without it.
    pub neck: Option<String>,
    /// The left arm chain, from the clavicle to the hand; arm_l without it.
    pub arm_l: Option<String>,
    /// The right arm chain, from the clavicle to the hand; arm_r without it.
    pub arm_r: Option<String>,
    /// The left leg chain, from the thigh to the foot; leg_l without it.
    pub leg_l: Option<String>,
    /// The right leg chain, from the thigh to the foot; leg_r without it.
    pub leg_r: Option<String>,
}

impl ChainNames {
    fn over(&self, defaults: HumanoidChains) -> HumanoidChains {
        let pick = |named: &Option<String>, default| named.as_deref().map_or(default, Into::into);
        HumanoidChains {
            spine: pick(&self.spine, defaults.spine),
            neck: pick(&self.neck, defaults.neck),
            left_arm: pick(&self.arm_l, defaults.left_arm),
            right_arm: pick(&self.arm_r, defaults.right_arm),
            left_leg: pick(&self.leg_l, defaults.left_leg),
            right_leg: pick(&self.leg_r, defaults.right_leg),
        }
    }
}

/// Generates a humanoid `Ragdoll` from the retarget rig `rig` and saves it as a new asset.
///
/// A [`RagdollCreation`] says how it went. The Create ragdoll button and the
/// `create_ragdoll` tool both send it.
pub struct CreateRagdoll {
    /// The id of the retarget rig whose chains and skeleton the ragdoll is generated from.
    pub rig: AssetId,
    /// Where the new asset goes, such as content/hero/ragdoll.gasset; next to the rig, as ragdoll.gasset, without one.
    pub address: Option<String>,
    /// The character's mass in kg; 75 without it.
    pub total_mass: Option<f32>,
    /// The rig's chain names where they differ from the defaults.
    pub chains: ChainNames,
}
impl Signal for CreateRagdoll {}

/// A ragdoll that was generated and saved.
#[derive(Clone, Debug, PartialEq)]
pub struct CreatedRagdoll {
    /// The new asset's id.
    pub id: AssetId,
    /// The address the new asset was written to.
    pub address: String,
    /// The sum of the parts' masses, in kg.
    pub total_mass: f32,
    /// One line per part: its bone, shape with sizes, and mass.
    pub parts: Vec<String>,
}

/// What a `CreateRagdoll` came to: the new asset, or why nothing was written.
#[derive(Event, Clone, Debug, PartialEq)]
pub struct RagdollCreation {
    /// The rig the [`CreateRagdoll`] named.
    pub rig: AssetId,
    /// The saved ragdoll, or what to change so that one can be generated.
    pub result: Result<CreatedRagdoll, String>,
}

pub(crate) fn read_asset<A: Asset>(
    root: &Path,
    registry: &AssetRegistry,
    id: AssetId,
    what: &str,
) -> anyhow::Result<A> {
    let entry = registry
        .get(id)
        .with_context(|| format!("the {what} is not in the project"))?;
    let bytes = read_asset_record(root, entry)
        .with_context(|| format!("cannot read {}", entry.location()))?;
    let (header, value) = decode_content_asset::<A>(&bytes)?;
    anyhow::ensure!(
        header.asset_id == id,
        "{} holds another asset's id",
        entry.address
    );
    Ok(value)
}

fn target_address(
    requested: Option<&str>,
    rig_address: &str,
    content_root: &str,
    root: &Path,
    registry: &AssetRegistry,
) -> Result<String, String> {
    let address = match requested {
        Some(address) => address.to_owned(),
        None => {
            let folder = rig_address
                .rsplit_once('/')
                .map_or("", |(folder, _)| folder);
            format!("{folder}/{DEFAULT_NAME}.{EXTENSION}")
        }
    };
    let inside = address
        .strip_prefix(content_root)
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(|| {
            format!("{address} is not inside {content_root}; pass an address such as {content_root}/hero/{DEFAULT_NAME}.{EXTENSION}")
        })?;
    let stem = inside
        .strip_suffix(&format!(".{EXTENSION}"))
        .ok_or_else(|| format!("{address} must end in .{EXTENSION}"))?;
    let valid = stem.split('/').all(|segment| {
        !segment.is_empty() && segment != "." && segment != ".." && !segment.contains('\\')
    });
    if !valid {
        return Err(format!("{address} is not a valid asset address"));
    }
    let name = stem.rsplit('/').next().unwrap_or(stem);
    valid_asset_name(name).map_err(|error| format!("{address}: {error}"))?;
    if registry.id_for_address(&address).is_some() || root.join(&address).exists() {
        return Err(format!(
            "{address} already exists; pass another `address` to keep it"
        ));
    }
    Ok(address)
}

fn shape_line(shape: &RagdollShape) -> String {
    match shape {
        RagdollShape::Capsule {
            half_height,
            radius,
        } => format!("capsule half_height {half_height:.3} radius {radius:.3}"),
        RagdollShape::Box { half_extents } => format!(
            "box half_extents [{:.3}, {:.3}, {:.3}]",
            half_extents.x, half_extents.y, half_extents.z
        ),
        RagdollShape::Sphere { radius } => format!("sphere radius {radius:.3}"),
    }
}

fn part_line(part: &RagdollPart) -> String {
    format!(
        "{} · {} · {:.2} kg",
        part.bone.0,
        shape_line(&part.shape),
        part.mass
    )
}

fn generate(
    request: &CreateRagdoll,
    root: &Path,
    content_root: &str,
    registry: &mut AssetRegistry,
) -> Result<CreatedRagdoll, String> {
    let entry = registry.get(request.rig).ok_or_else(|| {
        "That rig is not in the project; list_assets kind=RetargetRig shows the rigs.".to_owned()
    })?;
    if entry.kind == Skeleton::name() {
        return Err(format!(
            "{} is a Skeleton, and a ragdoll is generated from a RetargetRig of it. Create a rig for the skeleton first (create_asset kind=RetargetRig), set its root and chains, then pass the rig.",
            entry.address
        ));
    }
    if entry.kind != RetargetRig::name() {
        return Err(format!(
            "{} is a {}, not a RetargetRig; list_assets kind=RetargetRig shows the rigs.",
            entry.address, entry.kind
        ));
    }
    let rig_address = entry.address.clone();
    let rig = read_asset::<RetargetRig>(root, registry, request.rig, "rig")
        .map_err(|error| format!("{error:#}"))?;
    let skeleton_id = rig.skeleton.id();
    if skeleton_id.is_nil() {
        return Err(format!(
            "{rig_address} names no skeleton; open it and set its skeleton, then try again."
        ));
    }
    let skeleton = read_asset::<Skeleton>(root, registry, skeleton_id, "skeleton")
        .map_err(|error| format!("{error:#}"))?;
    let clip = match &rig.reference {
        ReferencePose::Rest => None,
        ReferencePose::ClipFrame { clip, .. } => Some(
            read_asset::<AnimationClip>(root, registry, clip.id(), "rig's reference clip")
                .map_err(|error| format!("{error:#}"))?,
        ),
    };
    let resolved = rig
        .resolve(&skeleton, clip.as_ref())
        .map_err(|error| format!("{rig_address} does not resolve against its skeleton: {error}"))?;
    let total_mass = request
        .total_mass
        .unwrap_or(HumanoidSettings::default().total_mass);
    if !(total_mass.is_finite() && total_mass > 0.0) {
        return Err(format!(
            "total_mass is {total_mass}; give the character's mass in kg, above 0."
        ));
    }
    let settings = HumanoidSettings::default()
        .with_total_mass(total_mass)
        .with_chains(request.chains.over(HumanoidChains::default()));
    let ragdoll = generate_humanoid(
        AssetHandle::weak(skeleton_id),
        &skeleton,
        &resolved,
        &[],
        &settings,
    )
    .map_err(|error| error.to_string())?;
    ragdoll
        .resolve(&skeleton)
        .map_err(|error| format!("the generated ragdoll does not resolve: {error}"))?;
    let address = target_address(
        request.address.as_deref(),
        &rig_address,
        content_root,
        root,
        registry,
    )?;
    let id = save_content_asset_into(&ragdoll, root, &address, registry)
        .map_err(|error| format!("Could not write {address}: {error:#}"))?;
    Ok(CreatedRagdoll {
        id,
        address,
        total_mass: ragdoll.parts.iter().map(|part| part.mass).sum(),
        parts: ragdoll.parts.iter().map(part_line).collect(),
    })
}

pub(crate) fn create_ragdoll(
    on: On<CreateRagdoll>,
    mut project: ResMut<ProjectState>,
    mut registry: ResMut<AssetRegistry>,
    mut written: EventWriter<AssetsWritten>,
    mut outcome: EventWriter<RagdollCreation>,
) {
    let request = on.signal();
    let result = match project.project.as_ref() {
        _ if project.busy() => Err("Wait for the project to finish".to_owned()),
        None => Err("Open a project before generating a ragdoll.".to_owned()),
        Some(open) => {
            let (root, content_root) = (open.root.clone(), open.content_root.clone());
            generate(request, &root, &content_root, &mut registry)
        }
    };
    match &result {
        Ok(created) => {
            project.status = format!("Created {}", created.address);
            written.write(AssetsWritten {
                ids: vec![created.id],
            });
        }
        Err(message) => project.status = message.clone(),
    }
    outcome.write(RagdollCreation {
        rig: request.rig,
        result,
    });
}

/// Generates a humanoid ragdoll from a retarget rig and saves it as a new Ragdoll asset next to the rig, as the Create ragdoll button on a rig tab does, and answers its address and one line per part: bone, shape and mass. No skinned mesh is read, so part radii follow the bones' proportions. Tune the saved asset with open_asset and set_property.
#[derive(Deserialize, JsonSchema)]
pub struct CreateRagdollCall {
    /// The retarget rig, by address such as content/hero/rig.gasset or by id, as list_assets shows them.
    pub rig: String,
    /// Where to save the ragdoll, such as content/hero/ragdoll.gasset; ragdoll.gasset in the rig's folder without it. An address that is taken is refused.
    pub address: Option<String>,
    /// The character's mass in kg; 75 without it.
    pub total_mass: Option<f32>,
    /// The names of the rig's chains, for the ones not called spine, neck, arm_l, arm_r, leg_l and leg_r.
    pub chains: Option<ChainNames>,
}

#[derive(Component)]
pub(crate) struct CreatingRagdoll {
    rig: AssetId,
}

pub(crate) fn create_ragdoll_call(
    mut calls: ToolCalls<CreateRagdollCall>,
    registry: Res<AssetRegistry>,
    mut cmd: CommandQueue,
) {
    for (arguments, reply) in calls.read() {
        let Some(rig) = registry
            .id_for_location(&arguments.rig)
            .or_else(|| AssetId::from_simple_hex(&arguments.rig).ok())
            .filter(|id| registry.get(*id).is_some())
        else {
            reply.err(ToolError::new(
                "unknown_asset",
                format!(
                    "No asset `{}` in the project; list_assets kind=RetargetRig shows the rigs.",
                    arguments.rig
                ),
            ));
            continue;
        };
        cmd.trigger(CreateRagdoll {
            rig,
            address: arguments.address.clone(),
            total_mass: arguments.total_mass,
            chains: arguments.chains.clone().unwrap_or_default(),
        });
        cmd.spawn((
            PendingCall::new(reply, "the ragdoll to be generated"),
            CreatingRagdoll { rig },
        ));
    }
}

pub(crate) fn answer_creations(
    mut creations: EventReader<RagdollCreation>,
    waiting: Query<(Entity, &PendingCall, &CreatingRagdoll)>,
    mut cmd: CommandQueue,
) {
    for creation in creations.read() {
        let Some((entity, pending, _)) = waiting
            .iter()
            .find(|(_, _, creating)| creating.rig == creation.rig)
        else {
            continue;
        };
        match &creation.result {
            Ok(created) => {
                let mut text = format!(
                    "Created {} · Ragdoll · {}: {} parts, {:.1} kg. No skinned mesh was read, so part radii follow the bones' proportions. Open it with open_asset to tune it.",
                    created.address,
                    created.id.simple_hex(),
                    created.parts.len(),
                    created.total_mass
                );
                for line in &created.parts {
                    text.push('\n');
                    text.push_str(line);
                }
                pending.reply.ok(text);
            }
            Err(message) => pending
                .reply
                .err(ToolError::new("create_ragdoll_failed", message.clone())),
        }
        cmd.despawn(entity);
    }
}
