use std::any::{Any, TypeId};

use concerto_ecs::{
    Entity, Query,
    signal::{On, Signal},
};
use concerto_reflect::{
    FromType, PropertyVisitorMut, Reflect, TypeData, TypeRegistration, TypeRegistry, ValueWarning,
};

use crate::player::AnimationPlayer;

/// A value an animation graph can receive, queued with `AnimationPlayer::send`; mark it `#[reflect(AnimationMessage)]`.
pub trait AnimationMessage: Reflect + Clone + Send + Sync + 'static {}

/// Says a registered type is an [`AnimationMessage`], so `OnMessage` triggers stored with its path fire.
#[derive(Clone, Copy)]
pub struct ReflectAnimationMessage;

impl TypeData for ReflectAnimationMessage {}

impl<M: AnimationMessage> FromType<M> for ReflectAnimationMessage {
    fn from_type() -> Self {
        Self
    }
}

/// Queues `message` on the `AnimationPlayer` of `character`, which does nothing without one; the message type must be registered with `register_animation_message`.
pub struct SendAnimationMessage<M: AnimationMessage> {
    pub character: Entity,
    pub message: M,
}

impl<M: AnimationMessage> Signal for SendAnimationMessage<M> {}

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
            .as_ref()
            .filter(|registration| registration.data::<ReflectAnimationMessage>().is_some())
            .map(|registration| TypeRegistration::type_id(registration));
        let problem = match (&registration, message.id) {
            (None, _) => "is not registered",
            (Some(_), None) => "is not marked #[reflect(AnimationMessage)]",
            (Some(_), Some(_)) => return,
        };
        self.warnings.push(ValueWarning {
            path: self.path.join("."),
            reason: format!(
                "message type {} {problem}, so its trigger never fires",
                message.path
            ),
        });
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
