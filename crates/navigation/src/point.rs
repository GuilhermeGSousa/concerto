use concerto_ecs::{
    Component,
    component::scene::SceneEntityRef,
    reflect::{ReflectComponent, ReflectSceneComponent},
};
use concerto_foundation::transform::Transform;
use concerto_name::Name;
use concerto_reflect::{Reflect, ReflectDefault};

/// A point NPCs can walk to and through; its anchor names it to game data.
#[derive(Component, Reflect, Clone, Debug, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(Transform)]
pub struct NavPoint {
    /// The name game data refers to the point by; empty for none.
    pub anchor: Name,
    /// Whether agents may route to and through the point.
    pub enabled: bool,
}

impl Default for NavPoint {
    fn default() -> Self {
        Self {
            anchor: Name::default(),
            enabled: true,
        }
    }
}

/// The points this point links to.
#[derive(Component, Reflect, Clone, Debug, Default, PartialEq)]
#[reflect(Component, Default, SceneComponent)]
#[require(NavPoint)]
pub struct NavLinks {
    /// The links, each to another point.
    pub links: Vec<NavLink>,
}

/// A link from the point that owns it to `to`, walkable both ways unless `one_way`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Default)]
pub struct NavLink {
    /// The point linked to.
    pub to: SceneEntityRef,
    /// Whether the link is walkable only from the owning point to `to`.
    pub one_way: bool,
}
