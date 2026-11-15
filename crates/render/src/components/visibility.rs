use concerto_ecs::{component::Component, reflect::ReflectSceneComponent};
use concerto_reflect::Reflect;

use crate::components::render_entity::SyncWithRenderWorld;

/// Keeps a mesh drawn in every view, whatever its bounds.
#[derive(Component, Reflect, Default, Clone, Copy)]
#[require(SyncWithRenderWorld)]
#[reflect(SceneComponent)]
pub struct NoFrustumCulling;

/// Keeps a mesh out of every shadow map.
#[derive(Component, Reflect, Default, Clone, Copy)]
#[require(SyncWithRenderWorld)]
#[reflect(SceneComponent)]
pub struct NotShadowCaster;

/// The layers an entity is drawn on, or the layers a camera draws: a camera draws a mesh when the two share a layer. An entity or camera without it is on layer 0, and children do not take their parent's.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[require(SyncWithRenderWorld)]
pub struct RenderLayers(u32);

impl RenderLayers {
    /// How many layers there are, numbered from 0.
    pub const COUNT: u8 = 32;
    /// Layer 0 alone, which is where everything without the component is.
    pub const DEFAULT: Self = Self(1);
    /// No layer: drawn by no camera, or a camera that draws nothing.
    pub const NONE: Self = Self(0);

    /// `layer` alone.
    pub const fn layer(layer: u8) -> Self {
        Self::NONE.with(layer)
    }

    /// These layers and `layer`; a layer past the last is ignored.
    pub const fn with(self, layer: u8) -> Self {
        if layer < Self::COUNT {
            Self(self.0 | (1 << layer))
        } else {
            self
        }
    }

    /// These layers without `layer`.
    pub const fn without(self, layer: u8) -> Self {
        if layer < Self::COUNT {
            Self(self.0 & !(1 << layer))
        } else {
            self
        }
    }

    /// Whether the two share a layer.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl Default for RenderLayers {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::RenderLayers;

    #[test]
    fn layers_are_shared_or_not_and_the_default_is_layer_zero() {
        let world = RenderLayers::default();
        let body = RenderLayers::layer(1);
        let other_eyes = RenderLayers::DEFAULT.with(1);
        let own_eyes = other_eyes.without(1);

        assert_eq!(world, RenderLayers::layer(0));
        assert!(world.intersects(own_eyes) && world.intersects(other_eyes));
        assert!(body.intersects(other_eyes));
        assert!(
            !body.intersects(own_eyes),
            "a camera without the layer skips it"
        );
    }
}
