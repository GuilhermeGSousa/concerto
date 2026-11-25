use std::any::{Any, TypeId};

/// A typed per-bone value carried by an animation frame and blended with it.
pub trait PoseAttribute: Clone + Send + Sync + 'static {
    /// The value of a bone that no node set.
    fn neutral() -> Self;
    /// Blends `self`, carried at `self_weight`, with `other` at `other_weight`; the weights need not sum to 1.
    fn blend(&mut self, self_weight: f32, other: &Self, other_weight: f32);
}

trait Channel: Send + Sync {
    fn key(&self) -> TypeId;

    fn len(&self) -> usize;

    fn as_any(&self) -> &dyn Any;

    fn as_any_mut(&mut self) -> &mut dyn Any;

    fn empty(&self) -> Box<dyn Channel>;

    fn fill_neutral(&mut self, bone_count: usize);

    fn copy_from(&mut self, other: &dyn Channel);

    fn blend(&mut self, self_weight: f32, other: Option<&dyn Channel>, other_weight: f32);

    fn blend_bone(
        &mut self,
        bone: usize,
        self_weight: f32,
        other: Option<&dyn Channel>,
        other_weight: f32,
    );
}

struct Values<A>(Vec<A>);

impl<A: PoseAttribute> Values<A> {
    fn of(channel: Option<&dyn Channel>) -> Option<&Self> {
        channel.and_then(|channel| channel.as_any().downcast_ref::<Self>())
    }
}

impl<A: PoseAttribute> Channel for Values<A> {
    fn key(&self) -> TypeId {
        TypeId::of::<A>()
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn empty(&self) -> Box<dyn Channel> {
        Box::new(Values::<A>(Vec::new()))
    }

    fn fill_neutral(&mut self, bone_count: usize) {
        self.0.clear();
        self.0.resize(bone_count, A::neutral());
    }

    fn copy_from(&mut self, other: &dyn Channel) {
        if let Some(other) = Self::of(Some(other)) {
            self.0.clone_from(&other.0);
        }
    }

    fn blend(&mut self, self_weight: f32, other: Option<&dyn Channel>, other_weight: f32) {
        match Self::of(other) {
            Some(other) => {
                for (value, other) in self.0.iter_mut().zip(&other.0) {
                    value.blend(self_weight, other, other_weight);
                }
            }
            None => {
                let neutral = A::neutral();
                for value in &mut self.0 {
                    value.blend(self_weight, &neutral, other_weight);
                }
            }
        }
    }

    fn blend_bone(
        &mut self,
        bone: usize,
        self_weight: f32,
        other: Option<&dyn Channel>,
        other_weight: f32,
    ) {
        let Some(value) = self.0.get_mut(bone) else {
            return;
        };
        match Self::of(other).map(|other| other.0.get(bone)) {
            Some(Some(other)) => value.blend(self_weight, other, other_weight),
            Some(None) => {}
            None => value.blend(self_weight, &A::neutral(), other_weight),
        }
    }
}

/// The attributes of a frame: one channel per attribute type some node set, each one value per bone; an absent channel reads as neutral.
#[derive(Default)]
pub struct PoseAttributes {
    channels: Vec<Box<dyn Channel>>,
    spare: Vec<Box<dyn Channel>>,
}

impl PoseAttributes {
    /// The values of `A`, one per bone, or `None` when no node set `A`.
    pub fn channel<A: PoseAttribute>(&self) -> Option<&[A]> {
        Values::<A>::of(self.find(TypeId::of::<A>())).map(|values| values.0.as_slice())
    }

    /// The values of `A` for `bone_count` bones, to change; a channel no node set yet starts at neutral.
    pub fn channel_mut<A: PoseAttribute>(&mut self, bone_count: usize) -> &mut [A] {
        let index = match self.index_of(TypeId::of::<A>()) {
            Some(index) => index,
            None => self.open(TypeId::of::<A>(), bone_count, || {
                Box::new(Values::<A>(Vec::new()))
            }),
        };
        let values = &mut self.channels[index]
            .as_any_mut()
            .downcast_mut::<Values<A>>()
            .expect("a channel's key names its attribute type")
            .0;
        values.resize(bone_count, A::neutral());
        values
    }

    /// The value of `A` on `bone`, neutral when no node set it.
    pub fn get<A: PoseAttribute>(&self, bone: usize) -> A {
        self.channel::<A>()
            .and_then(|values| values.get(bone))
            .cloned()
            .unwrap_or_else(A::neutral)
    }

    /// Removes every channel, so every attribute reads neutral.
    pub fn clear(&mut self) {
        self.spare.append(&mut self.channels);
    }

    /// Overwrites these attributes with those of `other`.
    pub fn copy_from(&mut self, other: &PoseAttributes) {
        self.clear();
        for theirs in &other.channels {
            let index = self.open_like(theirs.as_ref());
            self.channels[index].copy_from(theirs.as_ref());
        }
    }

    /// Moves every bone toward `other` by `weight`, a channel absent on either side reading neutral.
    pub fn blend(&mut self, other: &PoseAttributes, weight: f32) {
        self.mix(1.0 - weight, other, weight);
    }

    /// Moves `bone` toward its value in `other` by `weight`, a channel absent on either side reading neutral.
    pub fn blend_bone(&mut self, other: &PoseAttributes, bone: usize, weight: f32) {
        for own in &mut self.channels {
            own.blend_bone(bone, 1.0 - weight, other.find(own.key()), weight);
        }
        for theirs in &other.channels {
            if self.index_of(theirs.key()).is_none() {
                let index = self.open_like(theirs.as_ref());
                self.channels[index].blend_bone(bone, 1.0 - weight, Some(theirs.as_ref()), weight);
            }
        }
    }

    /// Moves these attributes toward `destination`, keeping `decay` of their difference on every bone.
    pub fn decay_into(&mut self, destination: &PoseAttributes, decay: f32) {
        self.mix(decay, destination, 1.0 - decay);
    }

    fn mix(&mut self, self_weight: f32, other: &PoseAttributes, other_weight: f32) {
        for own in &mut self.channels {
            own.blend(self_weight, other.find(own.key()), other_weight);
        }
        for theirs in &other.channels {
            if self.index_of(theirs.key()).is_none() {
                let index = self.open_like(theirs.as_ref());
                self.channels[index].blend(self_weight, Some(theirs.as_ref()), other_weight);
            }
        }
    }

    fn find(&self, key: TypeId) -> Option<&dyn Channel> {
        self.channels
            .iter()
            .find(|channel| channel.key() == key)
            .map(|channel| channel.as_ref())
    }

    fn index_of(&self, key: TypeId) -> Option<usize> {
        self.channels
            .iter()
            .position(|channel| channel.key() == key)
    }

    fn open_like(&mut self, other: &dyn Channel) -> usize {
        self.open(other.key(), other.len(), || other.empty())
    }

    fn open(
        &mut self,
        key: TypeId,
        bone_count: usize,
        empty: impl FnOnce() -> Box<dyn Channel>,
    ) -> usize {
        let mut channel = match self.spare.iter().position(|channel| channel.key() == key) {
            Some(index) => self.spare.swap_remove(index),
            None => empty(),
        };
        channel.fill_neutral(bone_count);
        self.channels.push(channel);
        self.channels.len() - 1
    }
}
