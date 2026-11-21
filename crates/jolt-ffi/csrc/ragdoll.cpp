#include "internal.h"

#include <Jolt/Physics/Collision/Shape/BoxShape.h>
#include <Jolt/Physics/Collision/Shape/CapsuleShape.h>
#include <Jolt/Physics/Collision/Shape/RotatedTranslatedShape.h>
#include <Jolt/Physics/Collision/Shape/SphereShape.h>
#include <Jolt/Physics/Constraints/SwingTwistConstraint.h>
#include <Jolt/Physics/Ragdoll/Ragdoll.h>
#include <Jolt/Skeleton/Skeleton.h>
#include <Jolt/Skeleton/SkeletonPose.h>

#include <string>

#include "ragdoll.h"

struct JoltRagdollSettings
{
	JPH::Ref<JPH::RagdollSettings> settings;
};

struct JoltRagdoll
{
	JPH::Ref<JPH::Ragdoll> ragdoll;
	JPH::RefConst<JPH::RagdollSettings> settings;

	JPH::SwingTwistConstraint *Joint(uint32_t part) const
	{
		if (part >= ragdoll->GetBodyCount())
			return nullptr;
		int constraint = settings->GetConstraintIndexForBodyIndex(static_cast<int>(part));
		if (constraint < 0)
			return nullptr;
		return static_cast<JPH::SwingTwistConstraint *>(ragdoll->GetConstraint(constraint));
	}
};

namespace
{

	JPH::Vec3 ToVec3(const float v[3])
	{
		return JPH::Vec3(v[0], v[1], v[2]);
	}

	JPH::Quat ToQuat(const float q[4])
	{
		return JPH::Quat(q[0], q[1], q[2], q[3]).Normalized();
	}

	JPH::RefConst<JPH::Shape> CreatePartShape(const JoltRagdollPart &part)
	{
		JPH::ShapeSettings::ShapeResult result;
		switch (part.shape_kind)
		{
		case JOLT_PART_SHAPE_SPHERE:
			result = JPH::SphereShapeSettings(part.shape_size[0]).Create();
			break;
		case JOLT_PART_SHAPE_CAPSULE:
			result = JPH::CapsuleShapeSettings(part.shape_size[0], part.shape_size[1]).Create();
			break;
		case JOLT_PART_SHAPE_BOX:
		{
			JPH::Vec3 half_extents = ToVec3(part.shape_size);
			float convex_radius = std::min(JPH::cDefaultConvexRadius, half_extents.ReduceMin() * 0.5f);
			result = JPH::BoxShapeSettings(half_extents, convex_radius).Create();
			break;
		}
		default:
			return nullptr;
		}
		if (result.HasError())
			return nullptr;

		JPH::Vec3 position = ToVec3(part.shape_position);
		JPH::Quat rotation = ToQuat(part.shape_rotation);
		if (position.IsNearZero() && rotation.IsClose(JPH::Quat::sIdentity()))
			return result.Get();
		return new JPH::RotatedTranslatedShape(position, rotation, result.Get());
	}

	std::vector<JPH::Mat44> ToMatrices(const float *poses, uint32_t count)
	{
		std::vector<JPH::Mat44> matrices(count);
		for (uint32_t i = 0; i < count; ++i)
		{
			const float *pose = poses + i * 7;
			matrices[i] = JPH::Mat44::sRotationTranslation(ToQuat(pose + 3), ToVec3(pose));
		}
		return matrices;
	}

} // namespace

extern "C"
{

	JoltRagdollSettings *jolt_ragdoll_settings_create(const JoltRagdollPart *parts,
													  uint32_t count,
													  float max_linear_velocity,
													  uint32_t memberships,
													  uint32_t filter,
													  uint32_t velocity_steps,
													  uint32_t position_steps)
	{
		jolt_global_init();
		if (count == 0)
			return nullptr;

		JPH::Ref<JPH::Skeleton> skeleton = new JPH::Skeleton();
		JPH::Ref<JPH::RagdollSettings> settings = new JPH::RagdollSettings();
		settings->mSkeleton = skeleton;
		settings->mParts.resize(count);

		for (uint32_t i = 0; i < count; ++i)
		{
			const JoltRagdollPart &source = parts[i];
			if (source.parent >= static_cast<int32_t>(i) || source.parent < -1 || (i == 0) != (source.parent == -1))
				return nullptr;
			skeleton->AddJoint(std::to_string(i), source.parent);

			JPH::RefConst<JPH::Shape> shape = CreatePartShape(source);
			if (shape == nullptr || !(source.mass > 0.0f))
				return nullptr;

			JPH::RagdollSettings::Part &part = settings->mParts[i];
			part.SetShape(shape);
			part.mPosition = JPH::RVec3(ToVec3(source.position));
			part.mRotation = ToQuat(source.rotation);
			part.mMotionType = JPH::EMotionType::Dynamic;
			part.mObjectLayer = Layers::Make(memberships, filter, false, false);
			part.mOverrideMassProperties = JPH::EOverrideMassProperties::CalculateInertia;
			part.mMassPropertiesOverride.mMass = source.mass;
			part.mMaxLinearVelocity = max_linear_velocity;
			part.mFriction = source.friction;

			if (source.parent < 0)
				continue;

			JPH::Vec3 twist_axis = ToVec3(source.twist_axis);
			JPH::Vec3 plane_axis = ToVec3(source.plane_axis);
			if (twist_axis.IsNearZero() || plane_axis.IsNearZero())
				return nullptr;
			twist_axis = twist_axis.Normalized();
			plane_axis = plane_axis - twist_axis * plane_axis.Dot(twist_axis);
			if (plane_axis.IsNearZero())
				return nullptr;
			plane_axis = plane_axis.Normalized();

			JPH::Ref<JPH::SwingTwistConstraintSettings> joint = new JPH::SwingTwistConstraintSettings();
			joint->mSpace = JPH::EConstraintSpace::WorldSpace;
			joint->mPosition1 = joint->mPosition2 = part.mPosition;
			joint->mTwistAxis1 = joint->mTwistAxis2 = twist_axis;
			joint->mPlaneAxis1 = joint->mPlaneAxis2 = plane_axis;
			joint->mNormalHalfConeAngle = source.normal_half_cone;
			joint->mPlaneHalfConeAngle = source.plane_half_cone;
			joint->mTwistMinAngle = source.twist_min;
			joint->mTwistMaxAngle = source.twist_max;
			joint->mMaxFrictionTorque = source.max_friction_torque;
			joint->mNumVelocityStepsOverride = velocity_steps;
			joint->mNumPositionStepsOverride = position_steps;
			JPH::MotorSettings motor(source.motor_frequency, source.motor_damping);
			motor.SetTorqueLimit(source.motor_torque_limit);
			joint->mSwingMotorSettings = motor;
			joint->mTwistMotorSettings = motor;
			part.mToParent = joint;
		}

		skeleton->CalculateParentJointIndices();
		if (!settings->Stabilize())
			return nullptr;
		settings->DisableParentChildCollisions();
		settings->CalculateBodyIndexToConstraintIndex();
		settings->CalculateConstraintIndexToBodyIdxPair();

		return new JoltRagdollSettings{settings};
	}

	void jolt_ragdoll_settings_destroy(JoltRagdollSettings *settings)
	{
		delete settings;
	}

	uint32_t jolt_ragdoll_settings_part_count(const JoltRagdollSettings *settings)
	{
		return static_cast<uint32_t>(settings->settings->mParts.size());
	}

	JoltRagdoll *jolt_ragdoll_create(JoltWorld *world,
									 const JoltRagdollSettings *settings,
									 uint32_t collision_group,
									 const float *poses)
	{
		JPH::Ref<JPH::Ragdoll> ragdoll = settings->settings->CreateRagdoll(collision_group, 0, &world->system);
		if (ragdoll == nullptr)
			return nullptr;

		std::vector<JPH::Mat44> matrices = ToMatrices(poses, static_cast<uint32_t>(ragdoll->GetBodyCount()));
		ragdoll->SetPose(JPH::RVec3::sZero(), matrices.data());
		ragdoll->AddToPhysicsSystem(JPH::EActivation::Activate);
		return new JoltRagdoll{ragdoll, settings->settings.GetPtr()};
	}

	void jolt_ragdoll_destroy(JoltWorld *world, JoltRagdoll *ragdoll)
	{
		JPH::BodyInterface &bi = world->system.GetBodyInterface();
		JPH::AABox bounds;
		for (JPH::BodyID id : ragdoll->ragdoll->GetBodyIDs())
		{
			JPH::BodyLockRead lock(world->system.GetBodyLockInterface(), id);
			if (lock.Succeeded())
				bounds.Encapsulate(lock.GetBody().GetWorldSpaceBounds());
		}

		ragdoll->ragdoll->RemoveFromPhysicsSystem();
		for (JPH::BodyID id : ragdoll->ragdoll->GetBodyIDs())
		{
			world->reporting.erase(id);
			world->sensor_contact_listener.Forget(id);
		}
		if (bounds.IsValid())
		{
			bounds.ExpandBy(JPH::Vec3::sReplicate(0.05f));
			bi.ActivateBodiesInAABox(bounds, JPH::BroadPhaseLayerFilter(), JPH::ObjectLayerFilter());
		}
		delete ragdoll;
	}

	uint32_t jolt_ragdoll_part_count(const JoltRagdoll *ragdoll)
	{
		return static_cast<uint32_t>(ragdoll->ragdoll->GetBodyCount());
	}

	JoltBodyId jolt_ragdoll_part_body(const JoltRagdoll *ragdoll, uint32_t part)
	{
		return ragdoll->ragdoll->GetBodyID(static_cast<int>(part)).GetIndexAndSequenceNumber();
	}

	void jolt_ragdoll_get_pose(const JoltWorld *world, const JoltRagdoll *ragdoll, float *out_poses)
	{
		const JPH::BodyInterface &bi = const_cast<JoltWorld *>(world)->system.GetBodyInterface();
		uint32_t count = static_cast<uint32_t>(ragdoll->ragdoll->GetBodyCount());
		for (uint32_t i = 0; i < count; ++i)
		{
			JPH::RVec3 position;
			JPH::Quat rotation;
			bi.GetPositionAndRotation(ragdoll->ragdoll->GetBodyID(static_cast<int>(i)), position, rotation);
			float *pose = out_poses + i * 7;
			pose[0] = static_cast<float>(position.GetX());
			pose[1] = static_cast<float>(position.GetY());
			pose[2] = static_cast<float>(position.GetZ());
			pose[3] = rotation.GetX();
			pose[4] = rotation.GetY();
			pose[5] = rotation.GetZ();
			pose[6] = rotation.GetW();
		}
	}

	void jolt_ragdoll_set_pose(JoltRagdoll *ragdoll, const float *poses)
	{
		std::vector<JPH::Mat44> matrices = ToMatrices(poses, static_cast<uint32_t>(ragdoll->ragdoll->GetBodyCount()));
		ragdoll->ragdoll->SetPose(JPH::RVec3::sZero(), matrices.data());
		ragdoll->ragdoll->Activate();
	}

	void jolt_ragdoll_drive_kinematic(JoltRagdoll *ragdoll, const float *poses, float delta_time)
	{
		std::vector<JPH::Mat44> matrices = ToMatrices(poses, static_cast<uint32_t>(ragdoll->ragdoll->GetBodyCount()));
		ragdoll->ragdoll->DriveToPoseUsingKinematics(JPH::RVec3::sZero(), matrices.data(), delta_time);
	}

	void jolt_ragdoll_drive_motors(JoltRagdoll *ragdoll, const float *poses)
	{
		uint32_t count = static_cast<uint32_t>(ragdoll->ragdoll->GetBodyCount());
		std::vector<JPH::Mat44> matrices = ToMatrices(poses, count);
		JPH::SkeletonPose pose;
		pose.SetSkeleton(ragdoll->settings->GetSkeleton());
		for (uint32_t i = 0; i < count; ++i)
			pose.GetJointMatrix(static_cast<int>(i)) = matrices[i];
		pose.CalculateJointStates();
		ragdoll->ragdoll->DriveToPoseUsingMotors(pose);
		ragdoll->ragdoll->Activate();
	}

	void jolt_ragdoll_set_joint_motor(JoltRagdoll *ragdoll,
									  uint32_t part,
									  bool enabled,
									  const float target_rotation[4],
									  bool by_stiffness,
									  float spring,
									  float damping,
									  float torque_limit)
	{
		JPH::SwingTwistConstraint *joint = ragdoll->Joint(part);
		if (joint == nullptr)
			return;
		if (enabled)
		{
			JPH::MotorSettings motor;
			motor.mSpringSettings = JPH::SpringSettings(by_stiffness ? JPH::ESpringMode::StiffnessAndDamping
																	 : JPH::ESpringMode::FrequencyAndDamping,
														spring,
														damping);
			motor.SetTorqueLimit(torque_limit);
			joint->GetSwingMotorSettings() = motor;
			joint->GetTwistMotorSettings() = motor;
			joint->SetSwingMotorState(JPH::EMotorState::Position);
			joint->SetTwistMotorState(JPH::EMotorState::Position);
			joint->SetTargetOrientationBS(ToQuat(target_rotation));
		}
		else
		{
			joint->SetSwingMotorState(JPH::EMotorState::Off);
			joint->SetTwistMotorState(JPH::EMotorState::Off);
		}
		ragdoll->ragdoll->Activate();
	}

	void jolt_ragdoll_set_part_motion_type(JoltWorld *world,
										   JoltRagdoll *ragdoll,
										   uint32_t part,
										   JoltMotionType motion_type)
	{
		if (part >= ragdoll->ragdoll->GetBodyCount() || motion_type == JOLT_MOTION_TYPE_STATIC)
			return;
		world->system.GetBodyInterface().SetMotionType(ragdoll->ragdoll->GetBodyID(static_cast<int>(part)),
													   static_cast<JPH::EMotionType>(motion_type),
													   JPH::EActivation::Activate);
	}

	void jolt_ragdoll_get_joint_motor_impulse(const JoltRagdoll *ragdoll,
											  uint32_t part,
											  float out_impulse[3])
	{
		JPH::Vec3 impulse = JPH::Vec3::sZero();
		const JPH::SwingTwistConstraint *joint = ragdoll->Joint(part);
		if (joint != nullptr && (joint->GetSwingMotorState() != JPH::EMotorState::Off ||
								 joint->GetTwistMotorState() != JPH::EMotorState::Off))
			impulse = joint->GetTotalLambdaMotor();
		out_impulse[0] = impulse.GetX();
		out_impulse[1] = impulse.GetY();
		out_impulse[2] = impulse.GetZ();
	}

} // extern "C"
