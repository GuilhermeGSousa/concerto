use std::any::{Any, TypeId};

use concerto_ecs::{
    Entity, Query, Res,
    reflect::AppTypeRegistry,
    signal::{On, Signal},
};
use concerto_reflect::{
    PropertyVisitorMut, Reflect, ReflectDefault, ReflectedValue, TypeData, TypeRegistration,
    TypeRegistry, ValueWarning, apply_value, json_to_value,
};

use crate::player::AnimationPlayer;

/// A value an animation graph can receive, queued with `AnimationPlayer::send`; register it with `register_animation_message`.
pub trait AnimationMessage: Reflect + Clone + Send + Sync + 'static {}

/// Raised by a clip node on each frame its clip wraps, or, played once, stops.
#[derive(Clone, Default, Reflect)]
#[reflect(Default)]
pub struct ClipEnded;

impl AnimationMessage for ClipEnded {}

#[derive(Clone, Copy)]
pub(crate) struct ReflectAnimationMessage;

impl TypeData for ReflectAnimationMessage {}

/// The type paths of the message types registered with `register_animation_message`, in order.
pub fn animation_message_types(registry: &TypeRegistry) -> Vec<String> {
    let mut paths: Vec<String> = registry
        .iter()
        .filter(|registration| registration.data::<ReflectAnimationMessage>().is_some())
        .map(|registration| registration.type_path().to_owned())
        .collect();
    paths.sort();
    paths
}

/// Whether `message` is of a type registered with `register_animation_message`.
pub fn is_animation_message(registry: &TypeRegistry, message: &dyn Reflect) -> bool {
    registry
        .get((message as &dyn Any).type_id())
        .is_some_and(|registration| registration.data::<ReflectAnimationMessage>().is_some())
}

/// Reads JSON shaped as the inspector shows it as a message of the registered type `type_path`, or says why not; the type must be registered with `register_animation_message` and have a `Default`.
pub fn decode_animation_message(
    registry: &TypeRegistry,
    type_path: &str,
    json: &serde_json::Value,
) -> Result<ReflectedValue, String> {
    let registration = registry
        .get_by_name(type_path)
        .filter(|registration| registration.type_path() == type_path)
        .filter(|registration| registration.data::<ReflectAnimationMessage>().is_some())
        .ok_or_else(|| format!("{type_path} is not a registered animation message type"))?;
    if registration.data::<ReflectDefault>().is_none() {
        return Err(format!(
            "{type_path} has no Default, so a message cannot be built from JSON"
        ));
    }
    json_to_value(json, TypeRegistration::type_id(&registration), registry)
}

/// Builds the message of the registered type `type_path` from `stored`, as `decode_animation_message` read it; none when the type is not a registered message or the value does not fit.
pub fn build_animation_message(
    registry: &TypeRegistry,
    type_path: &str,
    stored: &ReflectedValue,
) -> Option<Box<dyn Reflect>> {
    let registration = registry
        .get_by_name(type_path)
        .filter(|registration| registration.type_path() == type_path)
        .filter(|registration| registration.data::<ReflectAnimationMessage>().is_some())?;
    let mut message = registration.data::<ReflectDefault>()?.default();
    apply_value(message.as_mut(), stored)
        .is_empty()
        .then_some(message)
}

/// Queues `message` on the `AnimationPlayer` of `character`, which does nothing without one; the message type must be registered with `register_animation_message`.
pub struct SendAnimationMessage<M: AnimationMessage> {
    pub character: Entity,
    pub message: M,
}

impl<M: AnimationMessage> Signal for SendAnimationMessage<M> {}

/// Queues the message of the registered type `type_path` built from the stored `value` on the `AnimationPlayer` of `character`; nothing happens without a player or when the type is not registered with `register_animation_message`. `decode_animation_message` reads the value.
pub struct SendReflectedMessage {
    pub character: Entity,
    pub type_path: String,
    pub value: ReflectedValue,
}

impl Signal for SendReflectedMessage {}

pub(crate) fn queue_reflected_message(
    on: On<SendReflectedMessage>,
    types: Res<AppTypeRegistry>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let sent = on.signal();
    let types = types.read();
    if let Some(mut player) = players.get_entity_mut(sent.character)
        && let Some(message) = build_animation_message(&types, &sent.type_path, &sent.value)
    {
        player.send_reflected(message, &types);
    }
}

pub(crate) fn queue_message<M: AnimationMessage>(
    on: On<SendAnimationMessage<M>>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let sent = on.signal();
    if let Some(mut player) = players.get_entity_mut(sent.character) {
        player.send(sent.message.clone());
    }
}

/// The type of a message a graph reacts to, stored by its type path and matched by its type once the graph is made live.
#[derive(Clone, Debug, Default, Reflect, serde::Serialize, serde::Deserialize)]
pub struct MessageType {
    path: String,
    #[reflect(skip)]
    #[serde(skip)]
    id: Option<TypeId>,
}

impl MessageType {
    /// The type of `M`.
    pub fn of<M: AnimationMessage>() -> Self {
        Self {
            path: std::any::type_name::<M>().to_owned(),
            id: Some(TypeId::of::<M>()),
        }
    }

    /// The type path it is stored by.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Whether `message` is of this type; never before the type is resolved.
    pub fn matches(&self, message: &dyn Reflect) -> bool {
        self.id == Some((message as &dyn Any).type_id())
    }
}

pub(crate) fn resolve_message_types(
    value: &mut dyn Reflect,
    registry: &TypeRegistry,
) -> Vec<ValueWarning> {
    let mut resolver = Resolver {
        registry,
        path: Vec::new(),
        warnings: Vec::new(),
    };
    resolver.visit(value);
    resolver.warnings
}

struct Resolver<'a> {
    registry: &'a TypeRegistry,
    path: Vec<String>,
    warnings: Vec<ValueWarning>,
}

impl Resolver<'_> {
    fn visit(&mut self, value: &mut dyn Reflect) {
        let Some(message) = value.downcast_mut::<MessageType>() else {
            value.visit_mut(self);
            return;
        };
        let registration = self
            .registry
            .get_by_name(&message.path)
            .filter(|registration| registration.type_path() == message.path);
        message.id = registration
            .filter(|registration| registration.data::<ReflectAnimationMessage>().is_some())
            .map(|registration| TypeRegistration::type_id(&registration));
        if message.id.is_none() {
            self.warnings.push(ValueWarning {
                path: self.path.join("."),
                reason: format!(
                    "message type {} is not registered with register_animation_message, so its trigger never fires",
                    message.path
                ),
            });
        }
    }
}

impl PropertyVisitorMut for Resolver<'_> {
    fn field(&mut self, name: &'static str, value: &mut dyn Reflect) {
        self.path.push(name.to_owned());
        self.visit(value);
        self.path.pop();
    }

    fn element(&mut self, index: usize, value: &mut dyn Reflect) {
        self.path.push(index.to_string());
        self.visit(value);
        self.path.pop();
    }
}
