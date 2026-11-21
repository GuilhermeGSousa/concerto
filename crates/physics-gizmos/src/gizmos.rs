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

/// Which physics layers are drawn: every collider, every ragdoll's parts and joint limits, and contact points.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct PhysicsGizmos {
    pub colliders: bool,
    pub ragdolls: bool,
    pub contacts: bool,
}

impl PhysicsGizmos {
    /// Every layer on.
    pub fn all() -> Self {
        Self {
            colliders: true,
            ragdolls: true,
            contacts: true,
        }
    }
}

/// Draws the layers of `PhysicsGizmos` in game time; register it after `DefaultPlugins`.
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
                (draw_colliders, draw_ragdolls, draw_contacts).after(propagate_global_transforms),
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
