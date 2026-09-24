use std::ops::{Deref, DerefMut};

use concerto_ecs::events::Event;

#[derive(Event)]
pub struct WindowEvent(winit::event::WindowEvent);

impl WindowEvent {
    pub fn new(event: winit::event::WindowEvent) -> Self {
        Self(event)
    }
}

impl Deref for WindowEvent {
    type Target = winit::event::WindowEvent;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for WindowEvent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
