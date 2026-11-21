#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "body.h"
#include "world.h"

#ifdef __cplusplus
extern "C"
{
#endif

    /* Owns a reference to a JPH::RagdollSettings: the parts of one ragdoll and
     * the constraints between them. Immutable once created, so any number of
     * ragdolls on any thread may be created from it. */
    typedef struct JoltRagdollSettings JoltRagdollSettings;

    /* Owns a JPH::Ragdoll that is in a world. */
    typedef struct JoltRagdoll JoltRagdoll;

    typedef uint32_t JoltPartShape;
    enum
    {
        /* shape_size[0] is the radius. */
        JOLT_PART_SHAPE_SPHERE = 0,
        /* shape_size[0] is the half height of the cylinder along Y, shape_size[1] the radius. */
        JOLT_PART_SHAPE_CAPSULE = 1,
        /* shape_size is the half extents. */
        JOLT_PART_SHAPE_BOX = 2,
    };

    /* One body of a ragdoll and the swing-twist joint to its parent. Poses and
     * axes are in the space of the reference pose, the one every part is
     * described in. The joint sits at the part's position. */
    typedef struct JoltRagdollPart
    {
        /* Index of the parent part, which must come earlier, or -1 for the root. */
        int32_t parent;
        float position[3];
        float rotation[4];
        JoltPartShape shape_kind;
        float shape_size[3];
        /* The shape's pose relative to the part. */
        float shape_position[3];
        float shape_rotation[4];
        float mass;
        float twist_axis[3];
        float plane_axis[3];
        float normal_half_cone;
        float plane_half_cone;
        float twist_min;
        float twist_max;
        float max_friction_torque;
        float motor_frequency;
        float motor_damping;
        float motor_torque_limit;
    } JoltRagdollPart;

    /* Builds the settings, stabilizes their masses and disables collisions
     * between each part and its parent. Returns null when a parent index is
     * not an earlier part, a shape is degenerate, or stabilizing fails. No
     * part ever moves faster than `max_linear_velocity`, in m/s. */
    JoltRagdollSettings *jolt_ragdoll_settings_create(const JoltRagdollPart *parts,
                                                      uint32_t count,
                                                      float max_linear_velocity);
    void jolt_ragdoll_settings_destroy(JoltRagdollSettings *settings);
    uint32_t jolt_ragdoll_settings_part_count(const JoltRagdollSettings *settings);

    /* Creates a ragdoll, places each part at its world pose in `poses` (seven
     * floats a part: position xyz, rotation xyzw) and adds it to the world,
     * active. `collision_group` must be unique among the world's ragdolls.
     * Returns null when the world is out of bodies. */
    JoltRagdoll *jolt_ragdoll_create(JoltWorld *world,
                                     const JoltRagdollSettings *settings,
                                     uint32_t collision_group,
                                     const float *poses);

    /* Removes the ragdoll's bodies and constraints from the world and destroys
     * them, waking what touched them. */
    void jolt_ragdoll_destroy(JoltWorld *world, JoltRagdoll *ragdoll);

    uint32_t jolt_ragdoll_part_count(const JoltRagdoll *ragdoll);
    JoltBodyId jolt_ragdoll_part_body(const JoltRagdoll *ragdoll, uint32_t part);

    /* Reads every part's world pose into `out_poses`, seven floats a part. */
    void jolt_ragdoll_get_pose(const JoltWorld *world, const JoltRagdoll *ragdoll, float *out_poses);

    /* Moves every part to its world pose in `poses` at once, keeping velocities. */
    void jolt_ragdoll_set_pose(JoltRagdoll *ragdoll, const float *poses);

    /* Sets each part's velocity so that it reaches its world pose in `poses`
     * after `delta_time`. Meant for kinematic parts, which then push dynamic
     * bodies out of the way. */
    void jolt_ragdoll_drive_kinematic(JoltRagdoll *ragdoll, const float *poses, float delta_time);

    /* Turns every joint's motor on and aims it at the rotation the part has
     * relative to its parent in `poses`. */
    void jolt_ragdoll_drive_motors(JoltRagdoll *ragdoll, const float *poses);

    /* Turns the motor of the joint between `part` and its parent off, or on
     * towards `target_rotation`, the part's rotation relative to its parent,
     * with the given spring and torque limit. Does nothing for the root. */
    void jolt_ragdoll_set_joint_motor(JoltRagdoll *ragdoll,
                                      uint32_t part,
                                      bool enabled,
                                      const float target_rotation[4],
                                      float frequency,
                                      float damping,
                                      float torque_limit);

    /* Makes one part kinematic or dynamic, keeping its velocity. */
    void jolt_ragdoll_set_part_motion_type(JoltWorld *world,
                                           JoltRagdoll *ragdoll,
                                           uint32_t part,
                                           JoltMotionType motion_type);

    /* The angular impulse the joint's motor applied in the last collision
     * step, about the joint's three axes. Zero for the root. */
    void jolt_ragdoll_get_joint_motor_impulse(const JoltRagdoll *ragdoll,
                                              uint32_t part,
                                              float out_impulse[3]);

#ifdef __cplusplus
}
#endif
