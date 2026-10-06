use concerto_ecs::{
    component::bundle::IntoBundle,
    signal::{
        Signal,
        listener::{IntoListener, Listener},
    },
};

use super::{InteractionSpec, Interactive, Layout, Shape, Themed, Typography};
use crate::{
    interaction::UIClick, material::UIMaterial, node::UINode, text::UIText, theme::UITheme,
};

/// An element with a signal listener attached.
pub struct Listening<E, S: Signal> {
    element: E,
    listener: Listener<S>,
}

impl<E, S: Signal> Listening<E, S> {
    pub(super) fn new<M>(element: E, listener: impl IntoListener<S, M>) -> Self {
        Self {
            element,
            listener: listener.into_listener(),
        }
    }
}

/// Signal listeners for interactive elements; attach them after the element's own modifiers.
pub trait Listen: IntoBundle {
    /// Runs `listener` when this element receives its signal; one listener per signal type.
    fn on<S: Signal, M>(self, listener: impl IntoListener<S, M>) -> Listening<Self, S> {
        Listening::new(self, listener)
    }

    /// Runs `listener` when a button is released over this element after pressing it.
    fn on_click<M>(self, listener: impl IntoListener<UIClick, M>) -> Listening<Self, UIClick> {
        self.on(listener)
    }
}

impl<E: Listen, S: Signal> Listen for Listening<E, S> {}

impl<E: IntoBundle, S: Signal> IntoBundle for Listening<E, S> {
    type Bundle = (E::Bundle, Listener<S>);

    fn into_bundle(self) -> Self::Bundle {
        (self.element.into_bundle(), self.listener)
    }
}

impl<E: Themed, S: Signal> Themed for Listening<E, S> {
    fn theme(&self) -> &UITheme {
        self.element.theme()
    }
}

impl<E: Layout, S: Signal> Layout for Listening<E, S> {
    fn node_mut(&mut self) -> &mut UINode {
        self.element.node_mut()
    }
}

impl<E: Typography, S: Signal> Typography for Listening<E, S> {
    fn text_mut(&mut self) -> &mut UIText {
        self.element.text_mut()
    }
}

impl<E: Shape, S: Signal> Shape for Listening<E, S> {
    fn material_mut(&mut self) -> &mut UIMaterial {
        self.element.material_mut()
    }
}

impl<E: Interactive, S: Signal> Interactive for Listening<E, S> {
    fn interaction_mut(&mut self) -> &mut InteractionSpec {
        self.element.interaction_mut()
    }
}
