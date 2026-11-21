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
    pub anchor: Name,
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
    pub links: Vec<NavLink>,
}

/// A link from the point that owns it to `to`, walkable both ways unless `one_way`.
#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq)]
#[reflect(Default)]
pub struct NavLink {
    pub to: SceneEntityRef,
    pub one_way: bool,
}
