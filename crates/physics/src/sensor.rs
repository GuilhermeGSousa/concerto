use concerto_ecs::{
    reflect::{ReflectComponent, ReflectSceneComponent},
    signal::{EntitySignal, Signal},
    CommandQueue, Component, Entity, ResMut,
};
use concerto_reflect::{Reflect, ReflectDefault};

use crate::backend::SensorContactKind;
use crate::physics_state::PhysicsState;

/// Makes the entity's `Collider` a trigger volume: it detects bodies without colliding with them.
#[derive(Component, Reflect, Default, Clone, Copy, Debug)]
#[reflect(Component, Default, SceneComponent)]
pub struct Sensor;

/// A body entered the sensor this signal is aimed at.
pub struct SensorEnter {
    pub other: Entity,
}

impl Signal for SensorEnter {}
impl EntitySignal for SensorEnter {}

/// A body left the sensor this signal is aimed at, or went away while inside it.
pub struct SensorExit {
    pub other: Entity,
}

impl Signal for SensorExit {}
impl EntitySignal for SensorExit {}

pub(crate) struct SensorContact {
    pub(crate) sensor: Entity,
    pub(crate) other: Entity,
    pub(crate) kind: SensorContactKind,
}

/// Triggers `SensorEnter` and `SensorExit` on the sensors for every contact gathered since the last run, in the order they happened.
pub fn report_sensor_contacts(mut state: ResMut<PhysicsState>, mut cmd: CommandQueue) {
    for contact in state.take_sensor_contacts() {
        let mut sensor = cmd.entity(contact.sensor);
        match contact.kind {
            SensorContactKind::Enter => sensor.trigger(SensorEnter {
                other: contact.other,
            }),
            SensorContactKind::Exit => sensor.trigger(SensorExit {
                other: contact.other,
            }),
        }
    }
}
