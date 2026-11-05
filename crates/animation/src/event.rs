use concerto_ecs::{CommandQueue, Entity, signal::EntitySignal, signal::Signal};
use concerto_foundation::assets::handle::LiveLeaves;
use concerto_name::Name;
use concerto_reflect::{
    FromType, Reflect, ReflectedValue, TypeData, ValueWarning, apply_value_with,
};

/// A value a clip can carry and trigger when playback crosses its time.
pub trait ClipEvent: Reflect + Clone + Default + Send + Sync + 'static {}

/// The event imports write: a name such as `footstep_left`.
#[derive(Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(ClipEvent)]
pub struct AnimationEvent {
    pub name: Name,
}

impl ClipEvent for AnimationEvent {}

/// Triggered on the player's entity when playback crosses a `S` on a clip.
pub struct AnimationSignal<S: ClipEvent> {
    pub signal: S,
    pub weight: f32,
    pub time: f32,
    pub mirrored: bool,
}

impl<S: ClipEvent> Signal for AnimationSignal<S> {}

impl<S: ClipEvent> EntitySignal for AnimationSignal<S> {}

/// Where and how strongly a clip event fired.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fired {
    pub weight: f32,
    pub time: f32,
    pub mirrored: bool,
}

/// A clip event built from stored data, with what could not be applied to it.
pub struct BuiltClipEvent {
    pub value: Box<dyn Reflect>,
    pub warnings: Vec<ValueWarning>,
}

/// How a clip builds an event from stored data and triggers it on an entity.
#[derive(Clone, Copy)]
pub struct ReflectClipEvent {
    pub from_value: fn(&ReflectedValue, &mut LiveLeaves) -> BuiltClipEvent,
    pub trigger: fn(&dyn Reflect, Fired, Entity, &mut CommandQueue),
}

impl TypeData for ReflectClipEvent {}

impl<S: ClipEvent> FromType<S> for ReflectClipEvent {
    fn from_type() -> Self {
        Self {
            from_value: from_value::<S>,
            trigger: trigger::<S>,
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
