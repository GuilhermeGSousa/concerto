use concerto_app::{plugins::Plugin, schedule_groups::LateUpdate};
use concerto_color::Color;
use concerto_debug_gizmos::{DebugGizmos, DebugGizmosPlugin};
use concerto_ecs::{IntoSystemConfig, Query, Res, Resource};
use concerto_foundation::transform::{GlobalTransform, systems::propagate_global_transforms};
use concerto_physics::{
    body::{BodyActivity, BodyId},
    collider::{Collider, ColliderOffset},
    contact::Contacts,
    physics_state::PhysicsState,
    ragdoll::RagdollId,
};
use concerto_ragdoll::BodySense;
use concerto_render::plugin::RenderPlugin;
use glam::Vec3;

use crate::{ragdoll::draw_ragdoll, shapes::LocalShape};

pub(crate) const STATIC_COLOR: Color = Color::rgba(0.55, 0.65, 0.85, 1.0);
pub(crate) const KINEMATIC_COLOR: Color = Color::rgba(0.2, 0.85, 1.0, 1.0);
pub(crate) const DYNAMIC_COLOR: Color = Color::rgba(0.3, 0.9, 0.35, 1.0);
pub(crate) const ASLEEP_COLOR: Color = Color::rgba(0.7, 0.45, 0.9, 1.0);
pub(crate) const SENSOR_COLOR: Color = Color::rgba(1.0, 0.55, 0.15, 1.0);
pub(crate) const CONTACT_COLOR: Color = Color::rgba(1.0, 0.25, 0.25, 1.0);
pub(crate) const CONTACT_CROSS: f32 = 0.08;
pub(crate) const CONTACT_NORMAL: f32 = 0.3;
pub(crate) const CENTRE_COLOR: Color = Color::rgba(1.0, 0.9, 0.2, 1.0);
pub(crate) const SUPPORT_COLOR: Color = Color::rgba(0.9, 0.9, 0.9, 1.0);
pub(crate) const BALANCED_COLOR: Color = Color::rgba(0.2, 1.0, 0.3, 1.0);
pub(crate) const OFF_BALANCE_COLOR: Color = Color::rgba(1.0, 0.2, 0.2, 1.0);
pub(crate) const VELOCITY_COLOR: Color = Color::rgba(0.3, 0.7, 1.0, 1.0);
pub(crate) const IMPACT_COLOR: Color = Color::rgba(1.0, 0.5, 0.1, 1.0);
pub(crate) const CENTRE_RADIUS: f32 = 0.05;
pub(crate) const CAPTURE_CROSS: f32 = 0.1;

/// Which physics layers are drawn: every collider, every ragdoll's parts and joint limits, contact points, and what each `BodySense` perceives.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct PhysicsGizmos {
    /// Draws every collider's shape, coloured by whether its body is static, kinematic, dynamic or
    /// asleep.
    pub colliders: bool,
    /// Draws every ragdoll's parts, joint frames, swing cones and twist ranges.
    pub ragdolls: bool,
    /// Draws a cross at each listed contact point and a line along its normal.
    pub contacts: bool,
    /// Draws each `BodySense`'s centre of mass, support outline, capture point, velocity and
    /// impacts.
    pub sensing: bool,
}

impl PhysicsGizmos {
    /// Every layer on.
    pub fn all() -> Self {
        Self {
            colliders: true,
            ragdolls: true,
            contacts: true,
            sensing: true,
        }
    }
}

/// Draws the layers of [`PhysicsGizmos`] in game time.
///
/// Register it after `DefaultPlugins`.
pub struct PhysicsGizmosPlugin;

impl Plugin for PhysicsGizmosPlugin {
    fn build(&self, app: &mut concerto_app::App) {
        app.insert_resource(PhysicsGizmos::default());
        if app.is_plugin_added::<RenderPlugin>() && !app.is_plugin_added::<DebugGizmosPlugin>() {
            app.register_plugin(DebugGizmosPlugin);
        }
        if app.is_plugin_added::<DebugGizmosPlugin>() {
            app.add_system(
                LateUpdate,
                (draw_colliders, draw_ragdolls, draw_contacts, draw_senses)
                    .after(propagate_global_transforms),
            );
        }
    }
}

/// The colour a body is drawn in: grey-blue when static, cyan when kinematic, green when dynamic, violet when asleep.
pub fn activity_color(activity: BodyActivity) -> Color {
    match activity {
        BodyActivity::Static => STATIC_COLOR,
        BodyActivity::Kinematic => KINEMATIC_COLOR,
        BodyActivity::Dynamic => DYNAMIC_COLOR,
        BodyActivity::Asleep => ASLEEP_COLOR,
    }
}

/// Draws the shape of every `Collider` in the colour of its body's activity while `PhysicsGizmos::colliders` is on.
pub fn draw_colliders(
    colliders: Query<(
        &Collider,
        &GlobalTransform,
        Option<&ColliderOffset>,
        Option<&BodyId>,
    )>,
    state: Option<Res<PhysicsState>>,
    shown: Res<PhysicsGizmos>,
    mut gizmos: DebugGizmos,
) {
    if !shown.colliders {
        return;
    }
    for (collider, transform, offset, body) in colliders.iter() {
        draw_collider(
            &mut gizmos,
            collider,
            transform,
            offset,
            body.zip(state.as_deref()),
        );
    }
}

/// Draws every ragdoll's parts, joint frames, swing cones and twist ranges while `PhysicsGizmos::ragdolls` is on.
pub fn draw_ragdolls(
    state: Option<Res<PhysicsState>>,
    shown: Res<PhysicsGizmos>,
    mut gizmos: DebugGizmos,
) {
    let (true, Some(state)) = (shown.ragdolls, state) else {
        return;
    };
    for ragdoll in state.ragdolls() {
        draw_ragdoll_of(&mut gizmos, &state, ragdoll);
    }
}

/// Draws a cross at each contact point and a line along its normal while `PhysicsGizmos::contacts` is on.
pub fn draw_contacts(
    contacts: Query<&Contacts>,
    shown: Res<PhysicsGizmos>,
    mut gizmos: DebugGizmos,
) {
    if !shown.contacts {
        return;
    }
    for contacts in contacts.iter() {
        draw_contact_list(&mut gizmos, contacts);
    }
}

/// Draws each `BodySense`'s centre of mass, support outline, capture point, velocity and impacts while `PhysicsGizmos::sensing` is on.
pub fn draw_senses(senses: Query<&BodySense>, shown: Res<PhysicsGizmos>, mut gizmos: DebugGizmos) {
    if !shown.sensing {
        return;
    }
    for sense in senses.iter() {
        draw_sense(&mut gizmos, sense);
    }
}

pub(crate) fn draw_collider(
    gizmos: &mut DebugGizmos,
    collider: &Collider,
    transform: &GlobalTransform,
    offset: Option<&ColliderOffset>,
    body: Option<(&BodyId, &PhysicsState)>,
) {
    let color = body.map_or(STATIC_COLOR, |(body, state)| {
        activity_color(state.body_activity(*body))
    });
    let offset = offset.map_or(Vec3::ZERO, |offset| offset.0);
    LocalShape::of(collider.shape()).draw_at(gizmos, transform, offset, color);
}

pub(crate) fn draw_ragdoll_of(gizmos: &mut DebugGizmos, state: &PhysicsState, ragdoll: RagdollId) {
    let Some(settings) = state.ragdoll_settings(ragdoll) else {
        return;
    };
    let mut pose = Vec::new();
    state.ragdoll_pose(ragdoll, &mut pose);
    let colors: Vec<Color> = state
        .ragdoll_bodies(ragdoll)
        .iter()
        .map(|body| activity_color(state.body_activity(*body)))
        .collect();
    draw_ragdoll(gizmos, settings, &pose, &colors);
}

pub(crate) fn draw_contact_list(gizmos: &mut DebugGizmos, contacts: &Contacts) {
    for contact in contacts.iter() {
        gizmos.cross(contact.point, CONTACT_CROSS, CONTACT_COLOR);
        gizmos.line(
            contact.point,
            contact.point + contact.normal * CONTACT_NORMAL,
            CONTACT_COLOR,
        );
    }
}

pub(crate) fn draw_sense(gizmos: &mut DebugGizmos, sense: &BodySense) {
    let centre = sense.centre.position;
    let floor = Vec3::new(centre.x, sense.ground, centre.z);
    gizmos.sphere(centre, CENTRE_RADIUS, CENTRE_COLOR);
    gizmos.line(centre, floor, CENTRE_COLOR);
    if sense.support.len() >= 2 {
        let points = sense.support.iter().copied();
        gizmos.linestrip(points.chain(sense.support.first().copied()), SUPPORT_COLOR);
    }
    let capture = if sense.balanced() {
        BALANCED_COLOR
    } else {
        OFF_BALANCE_COLOR
    };
    gizmos.cross(sense.capture_point, CAPTURE_CROSS, capture);
    gizmos.line(floor, sense.capture_point, capture);
    gizmos.arrow(centre, centre + sense.centre.velocity, VELOCITY_COLOR);
    for impact in [sense.chest_impact, sense.head_impact]
        .into_iter()
        .flatten()
    {
        gizmos.cross(impact.point, CONTACT_CROSS, IMPACT_COLOR);
        gizmos.line(
            impact.point,
            impact.point + impact.normal * CONTACT_NORMAL,
            IMPACT_COLOR,
        );
    }
}
