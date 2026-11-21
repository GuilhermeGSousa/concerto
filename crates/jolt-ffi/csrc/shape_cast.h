#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "body.h"
#include "world.h"

#ifdef __cplusplus
extern "C"
{
#endif

	typedef struct JoltShapeHit
	{
		JoltBodyId body;
		float fraction;
		float point[3];
		float normal[3];
	} JoltShapeHit;

	bool jolt_world_cast_sphere(const JoltWorld *world,
								const float origin[3],
								float radius,
								const float direction[3],
								const JoltBodyId *ignore,
								uint32_t ignore_count,
								JoltShapeHit *out_hit);

#ifdef __cplusplus
}
#endif
