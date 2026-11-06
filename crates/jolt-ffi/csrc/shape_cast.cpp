#include "internal.h"

#include <Jolt/Physics/Body/BodyFilter.h>
#include <Jolt/Physics/Collision/CollisionCollectorImpl.h>
#include <Jolt/Physics/Collision/ShapeCast.h>
#include <Jolt/Physics/Collision/Shape/SphereShape.h>

#include "shape_cast.h"

extern "C"
{

	bool jolt_world_cast_sphere(const JoltWorld *world,
								const float origin[3],
								float radius,
								const float direction[3],
								const JoltBodyId *ignore,
								uint32_t ignore_count,
								JoltShapeHit *out_hit)
	{
		JPH::RefConst<JPH::Shape> sphere = new JPH::SphereShape(radius);
		JPH::RVec3 base_offset(origin[0], origin[1], origin[2]);
		JPH::RShapeCast cast = JPH::RShapeCast::sFromWorldTransform(
			sphere,
			JPH::Vec3::sReplicate(1.0f),
			JPH::RMat44::sTranslation(base_offset),
			JPH::Vec3(direction[0], direction[1], direction[2]));

		JPH::ShapeCastSettings settings;
		settings.mBackFaceModeTriangles = JPH::EBackFaceMode::CollideWithBackFaces;
		settings.mBackFaceModeConvex = JPH::EBackFaceMode::CollideWithBackFaces;
		settings.mReturnDeepestPoint = true;

		JPH::IgnoreMultipleBodiesFilter body_filter;
		body_filter.Reserve(ignore_count);
		for (uint32_t i = 0; i < ignore_count; ++i)
			body_filter.IgnoreBody(JPH::BodyID(ignore[i]));

		JPH::ClosestHitCollisionCollector<JPH::CastShapeCollector> collector;
		world->system.GetNarrowPhaseQuery().CastShape(cast, settings, base_offset, collector, {}, {}, body_filter);
		if (!collector.HadHit())
			return false;

		const JPH::ShapeCastResult &hit = collector.mHit;
		JPH::RVec3 point = base_offset + hit.mContactPointOn2;
		JPH::Vec3 normal = -hit.mPenetrationAxis.NormalizedOr(JPH::Vec3::sZero());

		out_hit->body = hit.mBodyID2.GetIndexAndSequenceNumber();
		out_hit->fraction = hit.mFraction;
		out_hit->point[0] = float(point.GetX());
		out_hit->point[1] = float(point.GetY());
		out_hit->point[2] = float(point.GetZ());
		out_hit->normal[0] = normal.GetX();
		out_hit->normal[1] = normal.GetY();
		out_hit->normal[2] = normal.GetZ();
		return true;
	}

} // extern "C"
