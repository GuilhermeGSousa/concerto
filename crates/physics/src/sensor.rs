use concerto_ecs::{
    reflect::{ReflectComponent, ReflectSceneComponent},
    Component,
};
use concerto_reflect::{Reflect, ReflectDefault};

/// Makes the entity's `Collider` a trigger volume: it detects bodies without colliding with them.
#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component, Default, SceneComponent)]
pub struct Sensor;
