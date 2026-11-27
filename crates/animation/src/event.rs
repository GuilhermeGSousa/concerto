use concerto_ecs::{CommandQueue, Entity, signal::EntitySignal, signal::Signal};
use concerto_foundation::assets::handle::LiveLeaves;
use concerto_name::Name;
use concerto_reflect::{
    FromType, Reflect, ReflectDefault, ReflectedValue, TypeData, ValueWarning, apply_value_with,
};

use crate::message::AnimationMessage;

/// A value a clip can carry and trigger when playback crosses its time.
pub trait ClipEvent: Reflect + Clone + Default + Send + Sync + 'static {}

/// The event imports write: a name such as `footstep_left`.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(ClipEvent, Default)]
pub struct AnimationEvent {
    /// The name the import gave the event.
    pub name: Name,
}

impl ClipEvent for AnimationEvent {}

impl Signal for AnimationEvent {}

impl EntitySignal for AnimationEvent {}

impl AnimationMessage for AnimationEvent {}

/// Triggered on the player's entity when playback crosses a `S` on a clip.
pub struct AnimationSignal<S: ClipEvent> {
    /// The event value on the clip.
    pub signal: S,
    /// The weight the clip had in the evaluated pose, from 0 to 1.
    pub weight: f32,
    /// The key's time on the clip, in seconds.
    pub time: f32,
    /// Whether the pose was mirrored, so a left-side event plays on the right.
    pub mirrored: bool,
}

impl<S: ClipEvent> Signal for AnimationSignal<S> {}

impl<S: ClipEvent> EntitySignal for AnimationSignal<S> {}

/// Where and how strongly a clip event fired.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fired {
    /// The weight the clip had in the evaluated pose, from 0 to 1.
    pub weight: f32,
    /// The key's time on the clip, in seconds.
    pub time: f32,
    /// Whether the pose was mirrored when the key was crossed.
    pub mirrored: bool,
}

/// A clip event built from stored data, with what could not be applied to it.
pub struct BuiltClipEvent {
    /// The event, of the type the stored data named.
    pub value: Box<dyn Reflect>,
    /// The stored fields that did not apply to the event.
    pub warnings: Vec<ValueWarning>,
}

/// How a clip builds an event from stored data and triggers it on an entity.
#[derive(Clone, Copy)]
pub struct ReflectClipEvent {
    /// Builds the event from its stored fields, resolving the asset handles in them.
    pub from_value: fn(&ReflectedValue, &mut LiveLeaves) -> BuiltClipEvent,
    /// Triggers an [`AnimationSignal`] carrying the event on the entity.
    pub trigger: fn(&dyn Reflect, Fired, Entity, &mut CommandQueue),
    /// Clones the event into a box, as a clip node raises it; `None` when it is another type.
    pub boxed: fn(&dyn Reflect) -> Option<Box<dyn Reflect>>,
}

impl TypeData for ReflectClipEvent {}

impl<S: ClipEvent> FromType<S> for ReflectClipEvent {
    fn from_type() -> Self {
        Self {
            from_value: from_value::<S>,
            trigger: trigger::<S>,
            boxed: boxed::<S>,
        }
    }
}

fn from_value<S: ClipEvent>(stored: &ReflectedValue, leaves: &mut LiveLeaves) -> BuiltClipEvent {
    let mut event = S::default();
    let warnings = apply_value_with(&mut event, stored, leaves);
    BuiltClipEvent {
        value: Box::new(event),
        warnings,
    }
}

fn boxed<S: ClipEvent>(value: &dyn Reflect) -> Option<Box<dyn Reflect>> {
    let event = value.downcast_ref::<S>()?;
    Some(Box::new(event.clone()))
}

fn trigger<S: ClipEvent>(
    value: &dyn Reflect,
    fired: Fired,
    entity: Entity,
    cmd: &mut CommandQueue,
) {
    let Some(signal) = value.downcast_ref::<S>() else {
        return;
    };
    cmd.entity(entity).trigger(AnimationSignal {
        signal: signal.clone(),
        weight: fired.weight,
        time: fired.time,
        mirrored: fired.mirrored,
    });
}
