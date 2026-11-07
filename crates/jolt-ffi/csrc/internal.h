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
#include <Jolt/Physics/PhysicsSystem.h>

#include <algorithm>
#include <mutex>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "body.h"

JPH_SUPPRESS_WARNINGS

// Object layers: statics only collide with dynamics; dynamics collide with
// everything.
namespace Layers
{
	constexpr JPH::ObjectLayer NON_MOVING = 0;
	constexpr JPH::ObjectLayer MOVING = 1;
	constexpr JPH::ObjectLayer SENSOR = 2;
	constexpr JPH::uint NUM_LAYERS = 3;
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
			return inObject2 == Layers::MOVING;
		case Layers::MOVING:
			return true;
		case Layers::SENSOR:
			return inObject2 == Layers::MOVING;
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
	explicit SensorContactListener(const std::unordered_set<JPH::BodyID> &sensors) : sensors(sensors) {}

	void OnContactAdded(const JPH::Body &inBody1,
						const JPH::Body &inBody2,
						const JPH::ContactManifold &,
						JPH::ContactSettings &) override
	{
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

	void OnContactRemoved(const JPH::SubShapeIDPair &inSubShapePair) override
	{
		JPH::BodyID body1 = inSubShapePair.GetBody1ID();
		JPH::BodyID body2 = inSubShapePair.GetBody2ID();
		if (sensors.count(body1) == 0 && sensors.count(body2) == 0)
			return;
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

	void Forget(JPH::BodyID body)
	{
		std::lock_guard<std::mutex> lock(mutex);
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
	const std::unordered_set<JPH::BodyID> &sensors;
	std::mutex mutex;
	std::unordered_map<SensorPair, uint32_t, SensorPairHash> counts;
	std::vector<JoltSensorContact> queued;
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
	SensorContactListener sensor_contact_listener{sensors};
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
