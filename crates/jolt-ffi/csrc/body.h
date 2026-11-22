#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "shape.h"
#include "world.h"

/* Positions and half-extents are xyz float triples; rotations are xyzw
 * quaternions. Plain float arrays are used instead of structs so the ABI is
 * independent of Jolt's 16-byte-aligned vector types. */

#ifdef __cplusplus
extern "C"
{
#endif

    /* A Jolt body id (JPH::BodyID::GetIndexAndSequenceNumber()). */
    typedef uint32_t JoltBodyId;

    /* Mirrors JPH::EMotionType. Static bodies live on the NON_MOVING collision
     * layer, kinematic and dynamic bodies on MOVING. */
    typedef uint32_t JoltMotionType;
    enum
    {
        JOLT_MOTION_TYPE_STATIC = 0,
        JOLT_MOTION_TYPE_KINEMATIC = 1,
        JOLT_MOTION_TYPE_DYNAMIC = 2,
    };

    /* Bitmask of the degrees of freedom a dynamic body may use (mirrors
     * JPH::EAllowedDOFs). A value of 0 is invalid: use a static body instead. */
    typedef uint32_t JoltAllowedDofs;
    enum
    {
        JOLT_ALLOWED_DOFS_TRANSLATION_X = 1 << 0,
        JOLT_ALLOWED_DOFS_TRANSLATION_Y = 1 << 1,
        JOLT_ALLOWED_DOFS_TRANSLATION_Z = 1 << 2,
        JOLT_ALLOWED_DOFS_ROTATION_X = 1 << 3,
        JOLT_ALLOWED_DOFS_ROTATION_Y = 1 << 4,
        JOLT_ALLOWED_DOFS_ROTATION_Z = 1 << 5,
        JOLT_ALLOWED_DOFS_ALL = 0x3f,
    };

    /* The recipe jolt_body_create builds a body from (wraps
     * JPH::BodyCreationSettings). Freshly created settings sit at the origin
     * with an identity rotation and static motion; a shape must be set before
     * the settings are used. */
    typedef struct JoltBodyCreationSettings JoltBodyCreationSettings;

    JoltBodyCreationSettings *jolt_body_creation_settings_create(void);
    void jolt_body_creation_settings_destroy(JoltBodyCreationSettings *settings);

    void jolt_body_creation_settings_set_position(JoltBodyCreationSettings *settings,
                                                  const float position[3]);
    void jolt_body_creation_settings_set_rotation(JoltBodyCreationSettings *settings,
                                                  const float rotation[4]);
    void jolt_body_creation_settings_set_motion_type(JoltBodyCreationSettings *settings,
                                                     JoltMotionType motion_type);

    /* Only meaningful for dynamic bodies (e.g. lock the rotation DOFs to keep
     * a player capsule upright). Defaults to all. */
    void jolt_body_creation_settings_set_allowed_dofs(JoltBodyCreationSettings *settings,
                                                      JoltAllowedDofs allowed_dofs);

    /* Scales gravity for the body: 1 by default, 0 for a body that moves only
     * by the velocities it is given. */
    void jolt_body_creation_settings_set_gravity_factor(JoltBodyCreationSettings *settings,
                                                        float gravity_factor);

    /* The collision layers the body is in and the ones it collides with, each
     * a mask of bits 0 to 13. Two bodies collide when each is in a layer the
     * other collides with. Defaults: in layer 0, colliding with every layer. */
    void jolt_body_creation_settings_set_layers(JoltBodyCreationSettings *settings,
                                                uint32_t memberships,
                                                uint32_t filter);

    /* Makes the body a kinematic, always active sensor. */
    void jolt_body_creation_settings_set_is_sensor(JoltBodyCreationSettings *settings,
                                                   bool is_sensor);

    /* Sets the shape the body is built from (see shape.h). The settings take
     * their own reference, so `shape` may be destroyed straight afterwards or
     * reused for any number of other bodies. */
    void jolt_body_creation_settings_set_shape(JoltBodyCreationSettings *settings,
                                               const JoltShape *shape);

    /* Offsets the shape's geometry relative to the body origin (e.g. lift a
     * capsule so the origin sits at its bottom). Composes with
     * jolt_body_creation_settings_set_shape in any call order. */
    void jolt_body_creation_settings_set_shape_offset(JoltBodyCreationSettings *settings,
                                                      const float offset[3]);

    /* Scales the shape's geometry, so one shape can back bodies of different
     * sizes (an entity's Transform scale). Applied inside the offset, which
     * therefore stays in unscaled body space.
     *
     * Non-uniform and negative scales are supported; a mirrored scale keeps
     * mesh triangles wound correctly. A component of zero is invalid and
     * leaves the shape unscaled. */
    void jolt_body_creation_settings_set_shape_scale(JoltBodyCreationSettings *settings,
                                                     const float scale[3]);

    /* Creates a body from `settings` and adds it to the simulation, active
     * unless static; a sensor is always kinematic and active. The settings
     * remain owned by the caller and can be reused. */
    JoltBodyId jolt_body_create(JoltWorld *world, const JoltBodyCreationSettings *settings);

    /* Removes a body from the simulation and destroys it, waking any bodies that
     * were touching it (they would otherwise sleep in mid-air) and forgetting its
     * sensor pairs. The id is invalid afterwards. */
    void jolt_body_destroy(JoltWorld *world, JoltBodyId body);

    typedef struct JoltSensorContact
    {
        JoltBodyId sensor;
        JoltBodyId other;
        uint8_t entered;
    } JoltSensorContact;

    uint32_t jolt_world_drain_sensor_contacts(JoltWorld *world,
                                              JoltSensorContact *out,
                                              uint32_t capacity);

    /* The body's current motion type; JOLT_MOTION_TYPE_STATIC for an id that is not in the world. */
    JoltMotionType jolt_body_get_motion_type(const JoltWorld *world, JoltBodyId body);

    /* Whether the body is being simulated: false for a static or sleeping body. */
    bool jolt_body_is_active(const JoltWorld *world, JoltBodyId body);

    typedef uint8_t JoltContactKind;
    enum
    {
        JOLT_CONTACT_START = 0,
        JOLT_CONTACT_TOUCH = 1,
        JOLT_CONTACT_END = 2,
    };

    /* A contact between two solid bodies of which at least one reports
     * contacts. START and END mark a pair beginning and ceasing to touch and
     * carry no data. TOUCH is sent in every collision step the pair is
     * simulated: `normal` points from body1 to body2, `impulse` is the
     * estimated normal impulse of that collision step. */
    typedef struct JoltContact
    {
        JoltBodyId body1;
        JoltBodyId body2;
        JoltContactKind kind;
        float point[3];
        float normal[3];
        float impulse;
    } JoltContact;

    /* Turns the reporting of `body`'s contacts with solid bodies on or off.
     * Must not be called while the world steps. */
    void jolt_body_set_contact_reporting(JoltWorld *world, JoltBodyId body, bool enabled);

    uint32_t jolt_world_drain_contacts(JoltWorld *world, JoltContact *out, uint32_t capacity);

    /* Puts the body in the layers `memberships` and makes it collide with the
     * layers `filter`, as jolt_body_creation_settings_set_layers does at
     * creation. With no memberships it collides with nothing and no query sees
     * it. */
    void jolt_body_set_layers(JoltWorld *world, JoltBodyId body, uint32_t memberships, uint32_t filter);

    void jolt_body_get_angular_velocity(const JoltWorld *world,
                                        JoltBodyId body,
                                        float out_velocity[3]);

    /* Sets the body's velocities so that it reaches `position` and `rotation`
     * (xyzw) after `delta_time`. Meant for a kinematic body, which then pushes
     * dynamic bodies out of its way. */
    void jolt_body_move_kinematic(JoltWorld *world,
                                  JoltBodyId body,
                                  const float position[3],
                                  const float rotation[4],
                                  float delta_time);

    /* Moves the body's origin at once, keeping its rotation and velocity. */
    void jolt_body_set_position(JoltWorld *world, JoltBodyId body, const float position[3]);

    typedef uint32_t JoltGroundState;
    enum
    {
        JOLT_GROUND_STATE_ON_GROUND = 0,
        JOLT_GROUND_STATE_ON_STEEP_GROUND = 1,
        JOLT_GROUND_STATE_IN_AIR = 2,
    };

    /* The closest ground found by jolt_body_probe_ground. All fields besides
     * `state` are only valid when state != IN_AIR; `velocity` is the ground
     * body's velocity at the contact point. */
    typedef struct JoltGroundProbeResult
    {
        JoltGroundState state;
        JoltBodyId body;
        float position[3];
        float normal[3];
        float velocity[3];
    } JoltGroundProbeResult;

    /* Collides `body`'s shape against the world (ignoring `body` itself) and
     * reports the most upward-facing contact within `max_separation` below the
     * shape. Contacts steeper than `max_slope_angle` (radians from horizontal)
     * report ON_STEEP_GROUND. */
    void jolt_body_probe_ground(const JoltWorld *world,
                                JoltBodyId body,
                                float max_separation,
                                float max_slope_angle,
                                JoltGroundProbeResult *out_result);

    /* Setting a non-zero velocity also wakes the body: SetLinearVelocity alone
     * leaves a sleeping body asleep. */
    void jolt_body_set_linear_velocity(JoltWorld *world,
                                       JoltBodyId body,
                                       const float velocity[3]);
    void jolt_body_get_linear_velocity(const JoltWorld *world,
                                       JoltBodyId body,
                                       float out_velocity[3]);

    /* Reads a body's world-space position (xyz) and rotation (xyzw). */
    void jolt_body_get_transform(const JoltWorld *world,
                                 JoltBodyId body,
                                 float out_position[3],
                                 float out_rotation[4]);

    void jolt_body_add_impulse(JoltWorld *world,
                               JoltBodyId body,
                               float impulse[3]);

    void jolt_body_add_impulse_at(JoltWorld *world,
                                  JoltBodyId body,
                                  float impulse[3],
                                  float position[3]);

    void jolt_body_add_force(JoltWorld *world,
                             JoltBodyId body,
                             float force[3]);

    /* Adds a torque, in N·m about the world axes, for the next step. */
    void jolt_body_add_torque(JoltWorld *world, JoltBodyId body, const float torque[3]);

    void jolt_body_add_force_at(JoltWorld *world,
                                JoltBodyId body,
                                float force[3],
                                float position[3]);

#ifdef __cplusplus
}
#endif
