#include "internal.h"

#include <Jolt/Core/Factory.h>
#include <Jolt/RegisterTypes.h>

#include <cstdarg>
#include <cstdio>
#include <mutex>

#include "world.h"

namespace
{

	void TraceImpl(const char *inFMT, ...)
	{
		va_list list;
		va_start(list, inFMT);
		char buffer[1024];
		vsnprintf(buffer, sizeof(buffer), inFMT, list);
		va_end(list);

		fprintf(stderr, "[jolt] %s\n", buffer);
	}

} // namespace

extern "C"
{

	void jolt_global_init(void)
	{
		static std::once_flag once;
		std::call_once(once, []
					   {
		JPH::RegisterDefaultAllocator();
		JPH::Trace = TraceImpl;
		JPH::Factory::sInstance = new JPH::Factory();
		JPH::RegisterTypes(); });
	}

	JoltWorld *jolt_world_create(uint32_t max_bodies,
								 uint32_t num_body_mutexes,
								 uint32_t max_body_pairs,
								 uint32_t max_contact_constraints)
	{
		jolt_global_init();

		JoltWorld *world = new JoltWorld();
		world->system.Init(max_bodies,
						   num_body_mutexes,
						   max_body_pairs,
						   max_contact_constraints,
						   world->broad_phase_layer_interface,
						   world->object_vs_broad_phase_layer_filter,
						   world->object_layer_pair_filter);
		world->system.SetContactListener(&world->sensor_contact_listener);
		return world;
	}

	void jolt_world_destroy(JoltWorld *world)
	{
		delete world;
	}

	JoltStepper *jolt_stepper_create(uint32_t temp_allocator_bytes)
	{
		jolt_global_init();
		return new JoltStepper(temp_allocator_bytes);
	}

	void jolt_stepper_destroy(JoltStepper *stepper)
	{
		delete stepper;
	}

	uint32_t jolt_world_step(JoltWorld *world,
							 JoltStepper *stepper,
							 float delta_time,
							 int collision_steps)
	{
		JPH::EPhysicsUpdateError error = world->system.Update(delta_time,
															  collision_steps,
															  &stepper->temp_allocator,
															  &stepper->job_system);
		return static_cast<uint32_t>(error);
	}

} // extern "C"
