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

    /* Makes the body a sensor: jolt_body_create builds it kinematic, on the
     * SENSOR layer, never sleeping and active, whatever the other settings
     * say. */
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
     * unless static. The settings remain owned by the caller and can be
     * reused. */
    JoltBodyId jolt_body_create(JoltWorld *world, const JoltBodyCreationSettings *settings);

    /* Removes a body from the simulation and destroys it, waking any bodies that
     * were touching it (they would otherwise sleep in mid-air). The id is invalid
     * afterwards. */
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

    void jolt_body_add_force_at(JoltWorld *world,
                                JoltBodyId body,
                                float force[3],
                                float position[3]);

#ifdef __cplusplus
}
#endif
