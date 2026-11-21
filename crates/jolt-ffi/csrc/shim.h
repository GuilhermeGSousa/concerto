#pragma once

/* Umbrella over the shim's C API, split by area into world.h, body.h,
 * ray.h, and shape_cast.h. The Rust declarations in src/lib.rs must mirror these headers
 * exactly. */

#include "body.h"
#include "ray.h"
#include "shape_cast.h"
#include "world.h"
#include "shape.h"