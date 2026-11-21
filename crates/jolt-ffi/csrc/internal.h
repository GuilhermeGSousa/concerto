#pragma once

// C++-internal definitions shared by the shim's translation units; not part
// of the C API. Every translation unit (Jolt + shim) must be compiled with
// the same JPH_* configuration defines; build.rs is the single source of
// truth for those.

#include <Jolt/Jolt.h>

#include <Jolt/Core/JobSystemThreadPool.h>
#include <Jolt/Core/TempAllocator.h>
#include <Jolt/Physics/Body/Body.h>
#include <Jolt/Physics/Body/BodyCreationSettings.h>
#include <Jolt/Physics/Collision/ContactListener.h>
#include <Jolt/Physics/Collision/EstimateCollisionResponse.h>
#include <Jolt/Physics/PhysicsSystem.h>

#include <algorithm>
#include <mutex>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "body.h"

JPH_SUPPRESS_WARNINGS

// Object layers: statics collide with dynamics and ragdoll parts; sensors
// only with dynamics; dynamics with everything; ragdoll parts with
// everything but sensors; ghosts with nothing.
namespace Layers
{
	constexpr JPH::ObjectLayer NON_MOVING = 0;
	constexpr JPH::ObjectLayer MOVING = 1;
	constexpr JPH::ObjectLayer SENSOR = 2;
	constexpr JPH::ObjectLayer RAGDOLL = 3;
	constexpr JPH::ObjectLayer GHOST = 4;
	constexpr JPH::uint NUM_LAYERS = 5;
} // namespace Layers

namespace BroadPhaseLayers
{
	constexpr JPH::BroadPhaseLayer NON_MOVING(0);
	constexpr JPH::BroadPhaseLayer MOVING(1);
	constexpr JPH::BroadPhaseLayer SENSOR(2);
	constexpr JPH::uint NUM_LAYERS = 3;
} // namespace BroadPhaseLayers

class ObjectLayerPairFilterImpl final : public JPH::ObjectLayerPairFilter
{
public:
	bool ShouldCollide(JPH::ObjectLayer inObject1, JPH::ObjectLayer inObject2) const override
	{
		switch (inObject1)
		{
		case Layers::NON_MOVING:
			return inObject2 == Layers::MOVING || inObject2 == Layers::RAGDOLL;
		case Layers::MOVING:
			return inObject2 != Layers::GHOST;
		case Layers::SENSOR:
			return inObject2 == Layers::MOVING;
		case Layers::RAGDOLL:
			return inObject2 != Layers::SENSOR && inObject2 != Layers::GHOST;
		case Layers::GHOST:
			return false;
		default:
			JPH_ASSERT(false);
			return false;
		}
	}
};

class BPLayerInterfaceImpl final : public JPH::BroadPhaseLayerInterface
{
public:
	BPLayerInterfaceImpl()
	{
		mObjectToBroadPhase[Layers::NON_MOVING] = BroadPhaseLayers::NON_MOVING;
		mObjectToBroadPhase[Layers::MOVING] = BroadPhaseLayers::MOVING;
		mObjectToBroadPhase[Layers::SENSOR] = BroadPhaseLayers::SENSOR;
		mObjectToBroadPhase[Layers::RAGDOLL] = BroadPhaseLayers::MOVING;
		mObjectToBroadPhase[Layers::GHOST] = BroadPhaseLayers::MOVING;
	}

	JPH::uint GetNumBroadPhaseLayers() const override
	{
		return BroadPhaseLayers::NUM_LAYERS;
	}

	JPH::BroadPhaseLayer GetBroadPhaseLayer(JPH::ObjectLayer inLayer) const override
	{
		JPH_ASSERT(inLayer < Layers::NUM_LAYERS);
		return mObjectToBroadPhase[inLayer];
	}

#if defined(JPH_EXTERNAL_PROFILE) || defined(JPH_PROFILE_ENABLED)
	const char *GetBroadPhaseLayerName(JPH::BroadPhaseLayer inLayer) const override
	{
		switch ((JPH::BroadPhaseLayer::Type)inLayer)
		{
		case (JPH::BroadPhaseLayer::Type)BroadPhaseLayers::NON_MOVING:
			return "NON_MOVING";
		case (JPH::BroadPhaseLayer::Type)BroadPhaseLayers::MOVING:
			return "MOVING";
		case (JPH::BroadPhaseLayer::Type)BroadPhaseLayers::SENSOR:
			return "SENSOR";
		default:
			JPH_ASSERT(false);
			return "INVALID";
		}
	}
#endif

private:
	JPH::BroadPhaseLayer mObjectToBroadPhase[Layers::NUM_LAYERS];
};

class ObjectVsBroadPhaseLayerFilterImpl final : public JPH::ObjectVsBroadPhaseLayerFilter
{
public:
	bool ShouldCollide(JPH::ObjectLayer inLayer1, JPH::BroadPhaseLayer inLayer2) const override
	{
		switch (inLayer1)
		{
		case Layers::NON_MOVING:
			return inLayer2 == BroadPhaseLayers::MOVING;
		case Layers::MOVING:
			return true;
		case Layers::SENSOR:
			return inLayer2 == BroadPhaseLayers::MOVING;
		case Layers::RAGDOLL:
			return inLayer2 != BroadPhaseLayers::SENSOR;
		case Layers::GHOST:
			return false;
		default:
			JPH_ASSERT(false);
			return false;
		}
	}
};

class NonSensorBroadPhaseLayerFilter final : public JPH::BroadPhaseLayerFilter
{
public:
	explicit NonSensorBroadPhaseLayerFilter(const JPH::BroadPhaseLayerFilter &inner) : inner(inner) {}

	bool ShouldCollide(JPH::BroadPhaseLayer inLayer) const override
	{
		return inLayer != BroadPhaseLayers::SENSOR && inner.ShouldCollide(inLayer);
	}

private:
	const JPH::BroadPhaseLayerFilter &inner;
};

class NonSensorObjectLayerFilter final : public JPH::ObjectLayerFilter
{
public:
	explicit NonSensorObjectLayerFilter(const JPH::ObjectLayerFilter &inner) : inner(inner) {}

	bool ShouldCollide(JPH::ObjectLayer inLayer) const override
	{
		return inLayer != Layers::SENSOR && inner.ShouldCollide(inLayer);
	}

private:
	const JPH::ObjectLayerFilter &inner;
};

// Passes solid layers a scene query asked for: ragdoll parts are skipped,
// included or the only thing seen.
class QueryObjectLayerFilter final : public JPH::ObjectLayerFilter
{
public:
	explicit QueryObjectLayerFilter(JoltRagdollQuery ragdolls) : ragdolls(ragdolls) {}

	bool ShouldCollide(JPH::ObjectLayer inLayer) const override
	{
		if (inLayer == Layers::SENSOR || inLayer == Layers::GHOST)
			return false;
		if (ragdolls == JOLT_RAGDOLL_QUERY_SKIP)
			return inLayer != Layers::RAGDOLL;
		if (ragdolls == JOLT_RAGDOLL_QUERY_ONLY)
			return inLayer == Layers::RAGDOLL;
		return true;
	}

private:
	JoltRagdollQuery ragdolls;
};

struct SensorPair
{
	JPH::BodyID sensor;
	JPH::BodyID other;

	bool operator==(const SensorPair &rhs) const
	{
		return sensor == rhs.sensor && other == rhs.other;
	}
};

struct SensorPairHash
{
	size_t operator()(const SensorPair &pair) const
	{
		uint64_t key = (uint64_t(pair.sensor.GetIndexAndSequenceNumber()) << 32) |
					   pair.other.GetIndexAndSequenceNumber();
		return std::hash<uint64_t>()(key);
	}
};

class SensorContactListener final : public JPH::ContactListener
{
public:
	SensorContactListener(const std::unordered_set<JPH::BodyID> &sensors,
						  const std::unordered_set<JPH::BodyID> &reporting,
						  const std::unordered_map<JPH::BodyID, JPH::BodyID> &ragdoll_ignores)
		: sensors(sensors), reporting(reporting), ragdoll_ignores(ragdoll_ignores)
	{
	}

	JPH::ValidateResult OnContactValidate(const JPH::Body &inBody1,
										  const JPH::Body &inBody2,
										  JPH::RVec3Arg,
										  const JPH::CollideShapeResult &) override
	{
		if (Ignores(inBody1, inBody2) || Ignores(inBody2, inBody1))
			return JPH::ValidateResult::RejectAllContactsForThisBodyPair;
		return JPH::ValidateResult::AcceptAllContactsForThisBodyPair;
	}

	void OnContactAdded(const JPH::Body &inBody1,
						const JPH::Body &inBody2,
						const JPH::ContactManifold &inManifold,
						JPH::ContactSettings &ioSettings) override
	{
		if (!inBody1.IsSensor() && !inBody2.IsSensor())
		{
			ReportSolid(inBody1, inBody2, inManifold, ioSettings, true);
			return;
		}
		if (inBody1.IsSensor() == inBody2.IsSensor())
			return;
		SensorPair pair = inBody1.IsSensor() ? SensorPair{inBody1.GetID(), inBody2.GetID()}
											 : SensorPair{inBody2.GetID(), inBody1.GetID()};
		std::lock_guard<std::mutex> lock(mutex);
		if (counts[pair]++ == 0)
			queued.push_back(JoltSensorContact{pair.sensor.GetIndexAndSequenceNumber(),
											   pair.other.GetIndexAndSequenceNumber(),
											   1});
	}

	void OnContactPersisted(const JPH::Body &inBody1,
							const JPH::Body &inBody2,
							const JPH::ContactManifold &inManifold,
							JPH::ContactSettings &ioSettings) override
	{
		if (!inBody1.IsSensor() && !inBody2.IsSensor())
			ReportSolid(inBody1, inBody2, inManifold, ioSettings, false);
	}

	void OnContactRemoved(const JPH::SubShapeIDPair &inSubShapePair) override
	{
		JPH::BodyID body1 = inSubShapePair.GetBody1ID();
		JPH::BodyID body2 = inSubShapePair.GetBody2ID();
		if (sensors.count(body1) == 0 && sensors.count(body2) == 0)
		{
			if (reporting.count(body1) == 0 && reporting.count(body2) == 0)
				return;
			std::lock_guard<std::mutex> lock(mutex);
			auto found = solid_counts.find(SensorPair{body1, body2});
			if (found == solid_counts.end())
				found = solid_counts.find(SensorPair{body2, body1});
			if (found == solid_counts.end())
				return;
			if (--found->second == 0)
			{
				solid_counts.erase(found);
				JoltContact contact{};
				contact.body1 = body1.GetIndexAndSequenceNumber();
				contact.body2 = body2.GetIndexAndSequenceNumber();
				contact.kind = JOLT_CONTACT_END;
				contacts.push_back(contact);
			}
			return;
		}
		std::lock_guard<std::mutex> lock(mutex);
		auto found = counts.find(SensorPair{body1, body2});
		if (found == counts.end())
			found = counts.find(SensorPair{body2, body1});
		if (found == counts.end())
			return;
		if (--found->second == 0)
		{
			queued.push_back(JoltSensorContact{found->first.sensor.GetIndexAndSequenceNumber(),
											   found->first.other.GetIndexAndSequenceNumber(),
											   0});
			counts.erase(found);
		}
	}

	uint32_t Drain(JoltSensorContact *out, uint32_t capacity)
	{
		std::lock_guard<std::mutex> lock(mutex);
		uint32_t count = static_cast<uint32_t>(std::min<size_t>(capacity, queued.size()));
		std::copy(queued.begin(), queued.begin() + count, out);
		queued.erase(queued.begin(), queued.begin() + count);
		return count;
	}

	uint32_t DrainContacts(JoltContact *out, uint32_t capacity)
	{
		std::lock_guard<std::mutex> lock(mutex);
		uint32_t count = static_cast<uint32_t>(std::min<size_t>(capacity, contacts.size()));
		std::copy(contacts.begin(), contacts.begin() + count, out);
		contacts.erase(contacts.begin(), contacts.begin() + count);
		return count;
	}

	void ForgetSolid(JPH::BodyID body)
	{
		std::lock_guard<std::mutex> lock(mutex);
		for (auto it = solid_counts.begin(); it != solid_counts.end();)
		{
			bool other_reports = reporting.count(it->first.sensor == body ? it->first.other : it->first.sensor) != 0;
			if ((it->first.sensor == body || it->first.other == body) && !other_reports)
				it = solid_counts.erase(it);
			else
				++it;
		}
	}

	void Forget(JPH::BodyID body)
	{
		std::lock_guard<std::mutex> lock(mutex);
		for (auto it = solid_counts.begin(); it != solid_counts.end();)
		{
			if (it->first.sensor == body || it->first.other == body)
				it = solid_counts.erase(it);
			else
				++it;
		}
		JoltBodyId solid_id = body.GetIndexAndSequenceNumber();
		contacts.erase(std::remove_if(contacts.begin(),
									  contacts.end(),
									  [solid_id](const JoltContact &contact)
									  { return contact.body1 == solid_id || contact.body2 == solid_id; }),
					   contacts.end());
		for (auto it = counts.begin(); it != counts.end();)
		{
			if (it->first.sensor == body || it->first.other == body)
				it = counts.erase(it);
			else
				++it;
		}
		JoltBodyId id = body.GetIndexAndSequenceNumber();
		queued.erase(std::remove_if(queued.begin(),
									queued.end(),
									[id](const JoltSensorContact &contact)
									{ return contact.sensor == id || contact.other == id; }),
					 queued.end());
	}

private:
	bool Ignores(const JPH::Body &part, const JPH::Body &other) const
	{
		if (part.GetObjectLayer() != Layers::RAGDOLL)
			return false;
		auto found = ragdoll_ignores.find(part.GetID());
		return found != ragdoll_ignores.end() && found->second == other.GetID();
	}

	void ReportSolid(const JPH::Body &inBody1,
					 const JPH::Body &inBody2,
					 const JPH::ContactManifold &inManifold,
					 const JPH::ContactSettings &inSettings,
					 bool added)
	{
		if (reporting.count(inBody1.GetID()) == 0 && reporting.count(inBody2.GetID()) == 0)
			return;

		JPH::CollisionEstimationResult estimate;
		JPH::EstimateCollisionResponse(inBody1,
									   inBody2,
									   inManifold,
									   estimate,
									   inSettings.mCombinedFriction,
									   inSettings.mCombinedRestitution,
									   1.0f,
									   1);

		JoltContact contact{};
		contact.body1 = inBody1.GetID().GetIndexAndSequenceNumber();
		contact.body2 = inBody2.GetID().GetIndexAndSequenceNumber();
		contact.kind = JOLT_CONTACT_TOUCH;
		JPH::RVec3 point = JPH::RVec3::sZero();
		JPH::uint points = inManifold.mRelativeContactPointsOn1.size();
		for (JPH::uint i = 0; i < points; ++i)
		{
			point += inManifold.GetWorldSpaceContactPointOn1(i);
			contact.impulse += estimate.mImpulses[i].mContactImpulse;
		}
		if (points > 0)
			point /= static_cast<float>(points);
		contact.point[0] = static_cast<float>(point.GetX());
		contact.point[1] = static_cast<float>(point.GetY());
		contact.point[2] = static_cast<float>(point.GetZ());
		contact.normal[0] = inManifold.mWorldSpaceNormal.GetX();
		contact.normal[1] = inManifold.mWorldSpaceNormal.GetY();
		contact.normal[2] = inManifold.mWorldSpaceNormal.GetZ();

		std::lock_guard<std::mutex> lock(mutex);
		SensorPair pair{inBody1.GetID(), inBody2.GetID()};
		auto found = solid_counts.find(pair);
		bool started = found == solid_counts.end();
		if (started)
			solid_counts[pair] = 1;
		else if (added)
			++found->second;
		if (started)
		{
			JoltContact start = contact;
			start.kind = JOLT_CONTACT_START;
			contacts.push_back(start);
		}
		contacts.push_back(contact);
	}

	const std::unordered_set<JPH::BodyID> &sensors;
	const std::unordered_set<JPH::BodyID> &reporting;
	const std::unordered_map<JPH::BodyID, JPH::BodyID> &ragdoll_ignores;
	std::mutex mutex;
	std::unordered_map<SensorPair, uint32_t, SensorPairHash> counts;
	std::vector<JoltSensorContact> queued;
	std::unordered_map<SensorPair, uint32_t, SensorPairHash> solid_counts;
	std::vector<JoltContact> contacts;
};

// The layer interfaces are declared before the system: PhysicsSystem::Init
// stores references to them, so they must be destroyed after it (members are
// destroyed in reverse declaration order).
struct JoltWorld
{
	BPLayerInterfaceImpl broad_phase_layer_interface;
	ObjectVsBroadPhaseLayerFilterImpl object_vs_broad_phase_layer_filter;
	ObjectLayerPairFilterImpl object_layer_pair_filter;
	std::unordered_set<JPH::BodyID> sensors;
	std::unordered_set<JPH::BodyID> reporting;
	std::unordered_map<JPH::BodyID, JPH::BodyID> ragdoll_ignores;
	SensorContactListener sensor_contact_listener{sensors, reporting, ragdoll_ignores};
	JPH::PhysicsSystem system;
};

struct JoltBodyCreationSettings
{
	JPH::BodyCreationSettings settings;
	// Applied by jolt_body_create by wrapping the final shape in a
	// RotatedTranslatedShape, so it composes with the shape setters in any
	// call order.
	JPH::Vec3 shape_offset = JPH::Vec3::sZero();
	// Applied inside the offset wrap, so the offset stays in body space
	// rather than being scaled along with the geometry.
	JPH::Vec3 shape_scale = JPH::Vec3::sReplicate(1.0f);
};

// Holds a reference rather than owning the allocation: Jolt shapes are
// refcounted and every body created from one keeps its own reference, so
// destroying the handle must not free a shape still in use.
struct JoltShape
{
	JPH::RefConst<JPH::Shape> shape;

	explicit JoltShape(const JPH::Shape *new_shape) : shape(new_shape) {}
};

struct JoltStepper
{
	JPH::TempAllocatorImpl temp_allocator;
	JPH::JobSystemThreadPool job_system;

	explicit JoltStepper(uint32_t temp_allocator_bytes)
		: temp_allocator(temp_allocator_bytes),
		  // -1 worker threads = hardware concurrency - 1.
		  job_system(JPH::cMaxPhysicsJobs, JPH::cMaxPhysicsBarriers, -1)
	{
	}
};
