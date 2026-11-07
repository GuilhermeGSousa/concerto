#include "shape.h"

#include "internal.h"

#include <Jolt/Geometry/IndexedTriangle.h>
#include <Jolt/Math/Float3.h>
#include <Jolt/Physics/Collision/Shape/BoxShape.h>
#include <Jolt/Physics/Collision/Shape/CapsuleShape.h>
#include <Jolt/Physics/Collision/Shape/MeshShape.h>
#include <Jolt/Physics/Collision/Shape/SphereShape.h>

#include <algorithm>

extern "C"
{
    JoltShape *jolt_create_sphere_shape(float radius, float density)
    {
        JPH::SphereShape *shape = new JPH::SphereShape(radius);
        shape->SetDensity(density);
        return new JoltShape(shape);
    }

    JoltShape *jolt_create_box_shape(const float half_extents[3], float density)
    {
        JPH::Vec3 extents(half_extents[0], half_extents[1], half_extents[2]);
        float convex_radius = std::min(JPH::cDefaultConvexRadius, extents.ReduceMin());
        JPH::BoxShape *shape = new JPH::BoxShape(extents, convex_radius);
        shape->SetDensity(density);
        return new JoltShape(shape);
    }

    JoltShape *jolt_create_capsule_shape(float half_height, float radius, float density)
    {
        JPH::CapsuleShape *shape = new JPH::CapsuleShape(half_height, radius);
        shape->SetDensity(density);
        return new JoltShape(shape);
    }

    JoltShape *jolt_create_mesh_shape(const float *vertices,
                                      uint32_t vertex_count,
                                      const uint32_t *indices,
                                      uint32_t index_count)
    {
        JPH::VertexList vertex_list;
        vertex_list.reserve(vertex_count);
        for (uint32_t i = 0; i < vertex_count; ++i)
        {
            vertex_list.emplace_back(vertices[3 * i], vertices[3 * i + 1], vertices[3 * i + 2]);
        }

        // A trailing partial triangle would index past the buffer; drop it.
        JPH::IndexedTriangleList triangle_list;
        triangle_list.reserve(index_count / 3);
        for (uint32_t i = 0; i + 2 < index_count; i += 3)
        {
            triangle_list.emplace_back(indices[i], indices[i + 1], indices[i + 2]);
        }

        JPH::MeshShapeSettings mesh(std::move(vertex_list), std::move(triangle_list));
        JPH::Shape::ShapeResult result = mesh.Create();
        if (!result.IsValid())
        {
            return nullptr;
        }

        return new JoltShape(result.Get());
    }

    void jolt_shape_destroy(JoltShape *shape)
    {
        delete shape;
    }

    void jolt_shape_get_local_bounds(const JoltShape *shape,
                                     float out_min[3],
                                     float out_max[3])
    {
        JPH::AABox bounds = shape->shape->GetLocalBounds();
        out_min[0] = bounds.mMin.GetX();
        out_min[1] = bounds.mMin.GetY();
        out_min[2] = bounds.mMin.GetZ();
        out_max[0] = bounds.mMax.GetX();
        out_max[1] = bounds.mMax.GetY();
        out_max[2] = bounds.mMax.GetZ();
    }
}
