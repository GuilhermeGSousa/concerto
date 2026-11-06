#include "internal.h"

#include <Jolt/Physics/Body/BodyFilter.h>
#include <Jolt/Physics/Body/BodyLock.h>
#include <Jolt/Physics/Collision/CastResult.h>
#include <Jolt/Physics/Collision/RayCast.h>

#include "ray.h"

extern "C"
{

	bool jolt_world_cast_ray(const JoltWorld *world,
							 const float origin[3],
							 const float direction[3],
							 const JoltBodyId *ignore,
							 uint32_t ignore_count,
							 JoltRayHit *out_hit)
	{
		JPH::RRayCast ray(JPH::RVec3(origin[0], origin[1], origin[2]),
						  JPH::Vec3(direction[0], direction[1], direction[2]));

		JPH::IgnoreMultipleBodiesFilter body_filter;
		body_filter.Reserve(ignore_count);
		for (uint32_t i = 0; i < ignore_count; ++i)
			body_filter.IgnoreBody(JPH::BodyID(ignore[i]));

		JPH::RayCastResult hit;
		if (!world->system.GetNarrowPhaseQuery().CastRay(ray, hit, {}, {}, body_filter))
			return false;

		out_hit->body = hit.mBodyID.GetIndexAndSequenceNumber();
		out_hit->fraction = hit.mFraction;

		// The surface normal requires shape access, which needs the body lock
		// (the recommended pattern from NarrowPhaseQuery::CastRay's docs).
		JPH::BodyLockRead lock(world->system.GetBodyLockInterface(), hit.mBodyID);
		if (lock.Succeeded())
		{
			JPH::Vec3 normal = lock.GetBody().GetWorldSpaceSurfaceNormal(
				hit.mSubShapeID2, ray.GetPointOnRay(hit.mFraction));
			out_hit->normal[0] = normal.GetX();
			out_hit->normal[1] = normal.GetY();
			out_hit->normal[2] = normal.GetZ();
		}
		else
		{
			out_hit->normal[0] = 0.0f;
			out_hit->normal[1] = 0.0f;
			out_hit->normal[2] = 0.0f;
		}

		return true;
	}

} // extern "C"
