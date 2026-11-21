#pragma once

#include <stdbool.h>

#include "body.h"
#include "world.h"

#ifdef __cplusplus
extern "C"
{
#endif

	/* The closest hit of a raycast. */
	typedef struct JoltRayHit
	{
		/* The body that was hit. */
		JoltBodyId body;
		/* Hit distance as a fraction [0, 1] of the ray's direction vector. */
		float fraction;
		/* World-space surface normal at the hit point (xyz). */
		float normal[3];
	} JoltRayHit;

	/* Casts a ray from `origin` along `direction` (whose length is the maximum
	 * cast distance) against every body except the `ignore_count` ids at
	 * `ignore`, writing the closest hit to `out_hit`. Returns false — leaving
	 * `out_hit` untouched — if nothing was hit. */
	bool jolt_world_cast_ray(const JoltWorld *world,
							 const float origin[3],
							 const float direction[3],
							 const JoltBodyId *ignore,
							 uint32_t ignore_count,
							 JoltRayHit *out_hit);

#ifdef __cplusplus
}
#endif
