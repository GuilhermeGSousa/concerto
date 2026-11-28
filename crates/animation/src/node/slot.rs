use concerto_ecs::Res;
use concerto_foundation::assets::handle::LiveLeaves;
use concerto_foundation::assets::{asset_store::AssetStore, handle::AssetHandle};
use concerto_name::{Name, name};
use concerto_reflect::{Reflect, ReflectDefault, TypeRegistry, ValueWarning};
use log::warn;
use uuid::Uuid;

use crate::{
    evaluation::{AnimationGraphContext, DriveContext},
    graph::AnimationNodeData,
    mask::{BoneMask, MaskResolution},
    message::{AnimationMessage, ClipEnded, MessageType},
    node::{
        AnimationNode, EditorAnimationNode, InputSlots, NodeState, PlaceholderNode,
        ReflectAnimationNode,
        layer::{blend_attributes, blend_joint},
    },
    pose::{AnimationFrame, EvaluatedFrame, FiredEvent, FramePool, RootMotion, WeightedMotion},
};

/// Asks every slot in the graph that has an entry named `entry` to play it from the start.
#[derive(Clone, Default, Reflect)]
#[reflect(Default)]
pub struct PlayInSlot {
    /// The name of the [`SlotEntry`] to play.
    pub entry: Name,
}

impl AnimationMessage for PlayInSlot {}

/// A node a [`SlotNode`] plays over its base when a message calls for it, fading in and out by its own times.
#[derive(Reflect)]
pub struct SlotEntry {
    /// The name a [`PlayInSlot`] message calls the entry by.
    pub name: Name,
    /// The node the entry plays, with the slot's base as its one input.
    pub node: AnimationNodeData,
    /// The mask whose bone weights limit where the entry applies; the whole body without one.
    pub mask: Option<AssetHandle<BoneMask>>,
    /// The seconds the entry takes to reach full weight; 0 is at once.
    pub fade_in: f32,
    /// The seconds the entry takes to leave once its node raises one of its `out` types; 0 is at once.
    pub fade_out: f32,
    /// The message types that start the entry when the player receives one.
    pub on: Vec<MessageType>,
    /// The message types that fade the entry out when its node raises one.
    ///
    /// A type matches any message of that type, whatever its fields: `ClipEnded` matches the end
    /// of any clip the entry's node plays.
    pub out: Vec<MessageType>,
}

impl Default for SlotEntry {
    fn default() -> Self {
        Self::new(Name::default(), PlaceholderNode {})
    }
}

impl SlotEntry {
    /// An entry named `name` playing `node` over the whole body, with no fades and no message type that starts it, ending when its node raises [`ClipEnded`].
    pub fn new<N: AnimationNode>(name: Name, node: N) -> Self {
        Self {
            name,
            node: AnimationNodeData::of(&node),
            mask: None,
            fade_in: 0.0,
            fade_out: 0.0,
            on: Vec::new(),
            out: vec![MessageType::of::<ClipEnded>()],
        }
    }

    /// Limits the entry to the bones of `mask`, by their weights.
    pub fn with_mask(mut self, mask: AssetHandle<BoneMask>) -> Self {
        self.mask = Some(mask);
        self
    }

    /// Sets the seconds the entry takes to reach full weight; 0 is at once.
    pub fn with_fade_in(mut self, fade_in: f32) -> Self {
        self.fade_in = fade_in;
        self
    }

    /// Sets the seconds the entry takes to leave once its node raises one of its `out` types; 0 is at once.
    pub fn with_fade_out(mut self, fade_out: f32) -> Self {
        self.fade_out = fade_out;
        self
    }

    /// Adds a message type that starts the entry.
    pub fn with_on(mut self, message: MessageType) -> Self {
        self.on.push(message);
        self
    }

    /// Adds a message type that, raised by the entry's node, fades the entry out.
    pub fn with_out(mut self, message: MessageType) -> Self {
        self.out.push(message);
        self
    }
}

/// Plays authored entries over its one input, the base, when messages call for them, like a montage slot.
///
/// An entry starts when the player receives a message of one of its `on` types or a
/// [`PlayInSlot`] naming it, and starts again from the beginning on the next one. It fades in,
/// plays until its node raises one of its `out` types, then fades out. Playing entries are
/// layered over the base in the order they started, by their fade weight times their mask, in
/// the pose, in the drive pass and in root motion. What an entry's node raises goes on up to the
/// slot's own containers.
#[derive(Default, Reflect)]
#[reflect(AnimationNode, Default)]
pub struct SlotNode {
    /// The entries the slot can play.
    pub entries: Vec<SlotEntry>,
}

impl SlotNode {
    /// A slot with no entries.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an entry, after the ones already there.
    pub fn with_entry(mut self, entry: SlotEntry) -> Self {
        self.entries.push(entry);
        self
    }

    fn start(&self, state: &mut SlotState, index: usize) {
        let (Some(entry), Some(entry_state)) =
            (self.entries.get(index), state.entries.get_mut(index))
        else {
            return;
        };
        let (Some(live), Some(node_state)) = (entry.node.live(), entry_state.node.as_mut()) else {
            return;
        };
        live.reset(node_state);
        entry_state.fading_out = false;
        if entry.fade_in <= 0.0 {
            entry_state.weight = 1.0;
        }
        state.playing.retain(|playing| *playing != index);
        state.playing.push(index);
    }
}

/// A [`SlotNode`]'s state: each entry's own node state and fade, and the entries playing in the order they started.
#[derive(Default)]
pub struct SlotState {
    entries: Vec<EntryState>,
    playing: Vec<usize>,
}

#[derive(Default)]
struct EntryState {
    node: Option<NodeState>,
    weight: f32,
    fading_out: bool,
    mask: EntryMask,
}

#[derive(Default)]
struct EntryMask {
    resolution: MaskResolution,
    warned_missing: bool,
    warned_root: bool,
}

impl EntryMask {
    fn resolve(
        &mut self,
        entry: &SlotEntry,
        context: &AnimationGraphContext<'_>,
        masks: &AssetStore<BoneMask>,
        bones: usize,
    ) -> bool {
        let Some(handle) = &entry.mask else {
            self.resolution.weights.clear();
            self.resolution.weights.resize(bones, 1.0);
            return true;
        };
        let (Some(asset), Some(skeleton)) = (masks.get(handle), context.skeleton()) else {
            return false;
        };
        asset.resolve_into(
            skeleton,
            context.root().map(|root| root.index),
            &mut self.resolution,
        );
        if !self.resolution.missing.is_empty() && !self.warned_missing {
            let names: Vec<&str> = self.resolution.missing.iter().map(|name| &**name).collect();
            warn!(
                "slot entry {} mask {:?} names bones the skeleton lacks: {}",
                entry.name.as_str(),
                handle.id(),
                names.join(", ")
            );
            self.warned_missing = true;
        }
        if self.resolution.names_root && !self.warned_root {
            warn!(
                "slot entry {} mask {:?} names the root bone, which keeps the base's joint",
                entry.name.as_str(),
                handle.id()
            );
            self.warned_root = true;
        }
        true
    }
}

fn fade_step(delta_time: f32, fade: f32) -> f32 {
    if fade > 0.0 { delta_time / fade } else { 1.0 }
}

impl AnimationNode for SlotNode {
    type State = SlotState;
    type CreateInput = ();
    type UpdateInput = ();
    type EvaluateInput = Res<'static, AssetStore<BoneMask>>;
    type DriveInput = Res<'static, AssetStore<BoneMask>>;

    fn make_live(&mut self, registry: &TypeRegistry, leaves: &mut LiveLeaves) -> Vec<ValueWarning> {
        let mut warnings = Vec::new();
        for entry in &mut self.entries {
            let label = format!("slot entry {}", entry.name.as_str());
            warnings.extend(entry.node.make_live(registry, leaves, &label));
        }
        warnings
    }

    fn create_state(&self, _: &(), context: &AnimationGraphContext<'_>) -> SlotState {
        SlotState {
            entries: self
                .entries
                .iter()
                .map(|entry| EntryState {
                    node: entry
                        .node
                        .live()
                        .and_then(|node| node.create_state(context)),
                    ..EntryState::default()
                })
                .collect(),
            playing: Vec::new(),
        }
    }

    fn update(
        &self,
        state: &mut SlotState,
        delta_time: f32,
        _: &(),
        context: &mut AnimationGraphContext<'_>,
    ) {
        for (index, entry) in self.entries.iter().enumerate() {
            let called = context.messages().any(|message| {
                entry.on.iter().any(|on| on.matches(message))
                    || message
                        .downcast_ref::<PlayInSlot>()
                        .is_some_and(|play| play.entry == entry.name)
            });
            if called {
                self.start(state, index);
            }
        }
        let SlotState { entries, playing } = state;
        playing.retain(|&index| {
            let (Some(entry), Some(entry_state)) =
                (self.entries.get(index), entries.get_mut(index))
            else {
                return false;
            };
            let (Some(live), Some(node_state)) = (entry.node.live(), entry_state.node.as_mut())
            else {
                return false;
            };
            let mut walk = context.child();
            live.update(node_state, delta_time, &mut walk);
            let mut ended = false;
            for message in walk.into_raised() {
                ended |= entry.out.iter().any(|out| out.matches(message.message()));
                context.pass_up(message);
            }
            if ended {
                entry_state.fading_out = true;
            }
            if entry_state.fading_out {
                entry_state.weight -= fade_step(delta_time, entry.fade_out);
            } else {
                entry_state.weight += fade_step(delta_time, entry.fade_in);
            }
            entry_state.weight = entry_state.weight.clamp(0.0, 1.0);
            let stopped = entry_state.fading_out && entry_state.weight <= 0.0;
            if stopped {
                entry_state.fading_out = false;
            }
            !stopped
        });
    }

    fn evaluate(
        &self,
        state: &mut SlotState,
        masks: &Res<AssetStore<BoneMask>>,
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        evaluated_inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        let Some(base) = evaluated_inputs.first() else {
            return;
        };
        output.copy_from(&base.frame);
        let SlotState { entries, playing } = state;
        for &index in playing.iter() {
            let (Some(entry), Some(entry_state)) =
                (self.entries.get(index), entries.get_mut(index))
            else {
                continue;
            };
            let (Some(live), Some(node_state)) = (entry.node.live(), entry_state.node.as_mut())
            else {
                continue;
            };
            let fade = entry_state.weight;
            if fade <= 0.0
                || !entry_state
                    .mask
                    .resolve(entry, context, masks, output.len())
            {
                continue;
            }
            let mut layer = pool.acquire();
            live.evaluate(
                node_state,
                context,
                bone_ids,
                &evaluated_inputs[..1],
                pool,
                &mut layer,
            );
            for event in layer.events() {
                output.push_event(FiredEvent {
                    weight: event.weight * fade,
                    ..*event
                });
            }
            for (bone, mask_weight) in entry_state.mask.resolution.weights.iter().enumerate() {
                let weight = mask_weight * fade;
                if weight > 0.0
                    && let (Some(joint), Some(target)) = (output.joint_mut(bone), layer.joint(bone))
                {
                    blend_joint(joint, target, weight);
                }
            }
            blend_attributes(&entry_state.mask.resolution.weights, fade, &layer, output);
            pool.release(layer);
        }
    }

    fn root_motion(
        &self,
        state: &mut SlotState,
        _: &(),
        context: &AnimationGraphContext<'_>,
        bone_ids: &[Uuid],
        inputs: &[WeightedMotion],
    ) -> RootMotion {
        let mut motion = inputs
            .first()
            .map_or(RootMotion::IDENTITY, |input| input.motion);
        let SlotState { entries, playing } = state;
        for &index in playing.iter() {
            let (Some(entry), Some(entry_state)) =
                (self.entries.get(index), entries.get_mut(index))
            else {
                continue;
            };
            let (Some(live), Some(node_state)) = (entry.node.live(), entry_state.node.as_mut())
            else {
                continue;
            };
            if entry.mask.is_some() {
                continue;
            }
            let entry_motion = live.root_motion(
                node_state,
                context,
                bone_ids,
                &inputs[..inputs.len().min(1)],
            );
            if entry_state.weight > 0.0 {
                motion = motion.blend(entry_motion, entry_state.weight);
            }
        }
        motion
    }

    fn drive(
        &self,
        state: &mut SlotState,
        masks: &Res<AssetStore<BoneMask>>,
        context: &mut DriveContext<'_>,
        bone_ids: &[Uuid],
        inputs: &[EvaluatedFrame],
        pool: &mut FramePool,
        output: &mut AnimationFrame,
    ) {
        let Some(base) = inputs.first() else {
            return;
        };
        output.attributes_mut().copy_from(base.frame.attributes());
        let SlotState { entries, playing } = state;
        for &index in playing.iter() {
            let (Some(entry), Some(entry_state)) =
                (self.entries.get(index), entries.get_mut(index))
            else {
                continue;
            };
            let (Some(live), Some(node_state)) = (entry.node.live(), entry_state.node.as_mut())
            else {
                continue;
            };
            let fade = entry_state.weight;
            if fade <= 0.0
                || !entry_state
                    .mask
                    .resolve(entry, context.graph, masks, output.len())
            {
                continue;
            }
            let mut layer = pool.acquire();
            let mut entry_context = context.scaled(fade);
            live.drive(
                node_state,
                &mut entry_context,
                bone_ids,
                &inputs[..1],
                pool,
                &mut layer,
            );
            context.pass_up(entry_context);
            blend_attributes(&entry_state.mask.resolution.weights, fade, &layer, output);
            pool.release(layer);
        }
    }

    fn drive_shares(&self, _: &SlotState, _: &DriveContext<'_>, shares: &mut [f32]) {
        for (slot, share) in shares.iter_mut().enumerate() {
            *share = if slot == 0 { 1.0 } else { 0.0 };
        }
    }

    fn reset(&self, state: &mut SlotState) {
        for (entry, entry_state) in self.entries.iter().zip(&mut state.entries) {
            if let (Some(live), Some(node_state)) = (entry.node.live(), entry_state.node.as_mut()) {
                live.reset(node_state);
            }
            entry_state.weight = 0.0;
            entry_state.fading_out = false;
        }
        state.playing.clear();
    }
}

impl EditorAnimationNode for SlotNode {
    fn input_slots(&self) -> InputSlots {
        InputSlots::Fixed(vec![name!("base")])
    }

    fn describe(&self, state: &SlotState) -> Option<String> {
        let playing: Vec<String> = state
            .playing
            .iter()
            .filter_map(|&index| {
                let entry = self.entries.get(index)?;
                let entry_state = state.entries.get(index)?;
                let mut line = format!("{} {:.2}", entry.name.as_str(), entry_state.weight);
                let described = entry
                    .node
                    .live()
                    .zip(entry_state.node.as_ref())
                    .and_then(|(live, node_state)| live.describe(node_state));
                if let Some(described) = described {
                    line.push_str(" · ");
                    line.push_str(&described);
                }
                Some(line)
            })
            .collect();
        Some(if playing.is_empty() {
            "no entry playing".to_owned()
        } else {
            format!("playing {}", playing.join(", "))
        })
    }
}
