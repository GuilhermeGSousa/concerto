import math
import os
import sys

import bpy
from mathutils import Matrix, Vector

MODELS = {
    "bed": dict(size=(0.9, 2.0, 1.0), tex=1024),
    "bed_stripped": dict(size=(0.9, 2.0, 1.0), tex=1024),
    "bench": dict(size=(2.0, 0.6, 0.9), tex=512),
    "boiler": dict(height=3.4, tex=2048),
    "chair": dict(size=(0.45, 0.48, 0.9), tex=512),
    "chart_rack": dict(size=(2.2, 0.12, 1.6), origin="wall", tex=1024),
    "coal_heap": dict(size=(2.4, 1.6, 1.0), tex=1024),
    "crate": dict(size=(1.0, 1.0, 1.0), tex=512),
    "crutch": dict(height=1.29, tex=512),
    "cure_chair": dict(size=(0.66, 1.75, 1.0), tex=1024),
    "curtains": dict(size=(1.4, 0.15, 2.6), origin="wall", tex=1024, two_sided=True),
    "director_desk": dict(width=1.7, tex=1024, glow=3.0),
    "door_entrance": dict(height=3.6, origin="wall", tex=1024),
    "door_room": dict(size=(1.0, 0.12, 2.1), origin="wall", tex=1024),
    "dumbwaiter": dict(height=1.6, origin="wall", tex=1024),
    "filing_cabinet": dict(size=(0.5, 0.7, 1.35), tex=512),
    "funicular_car": dict(size=(2.5, 7.6, 4.0), tex=2048),
    "key_board": dict(width=1.0, origin="hang", tex=1024),
    "lamp_post": dict(height=3.68, tex=1024, glow=6.0),
    "lantern": dict(height=0.4, tex=512, glow=6.0),
    "meal_trolley": dict(size=(0.96, 0.52, 1.04), turn=90, tex=512),
    "nurse_counter": dict(size=(2.5, 0.6, 1.09), turn=90, tex=1024),
    "patient_lying": dict(size=(0.5, 1.8, 0.4), tex=1024),
    "pendant": dict(size=(0.6, 0.6, 0.94), origin="top", tex=512, glow=6.0),
    "porter_desk": dict(size=(3.2, 1.0, 1.35), tex=1024),
    "radiator": dict(height=0.8, tex=512),
    "revolver": dict(depth=0.26, origin="centre", tex=1024),
    "schnapps": dict(height=0.3, tex=512),
    "secretary_desk": dict(size=(1.6, 0.75, 1.1), tex=1024),
    "signpost": dict(height=2.56, tex=512),
    "sled": dict(size=(0.63, 2.4, 0.48), tex=512),
    "spruce": dict(height=11.0, tex=1024, two_sided=True),
    "suitcase": dict(size=(0.68, 0.22, 0.5), tex=512),
    "tiled_stove": dict(height=2.0, tex=1024, glow=2.0),
    "trunk": dict(size=(1.0, 0.6, 0.6), tex=512),
    "valve": dict(height=0.6, tex=512),
    "wall_clock": dict(height=0.9, origin="hang", tex=512),
    "wayside_shrine": dict(height=2.5, tex=1024),
    "wheelchair": dict(size=(0.6, 0.9, 1.02), tex=1024),
    "workbench": dict(size=(2.0, 0.8, 1.1), tex=1024),
    "door_leaf": dict(source="door_room", cut=((0, 0.06, 0.94), (2, 0.0, 0.96)),
                      size=(1.0, 0.06, 2.1), origin="wall", tex=1024),
    "door_oak_leaf": dict(source="door_entrance", cut=((0, 0.06, 0.5), (2, 0.0, 0.95)),
                          size=(1.0, 0.07, 2.1), origin="wall", tex=1024),
}


BUDGET = 8000
TRIANGLES = {
    "spruce": 4000,
    "cure_chair": 3000,
    "filing_cabinet": 1500,
    "radiator": 1500,
    "door_leaf": 800,
    "door_oak_leaf": 800,
    "patient_lying": 3000,
    "wheelchair": 6000,
    "boiler": 15000,
    "crutch": 1200,
    "lamp_post": 2500,
    "bed": 5000,
    "bed_stripped": 5000,
    "funicular_car": 20000,
}


def triangles(me):
    return sum(len(p.vertices) - 2 for p in me.polygons)


def bounds(me):
    lo = Vector((min(v.co[i] for v in me.vertices) for i in range(3)))
    hi = Vector((max(v.co[i] for v in me.vertices) for i in range(3)))
    return lo, hi


def cut(ob, planes):
    import bmesh
    me = ob.data
    for axis, low, high in planes:
        lo, hi = bounds(me)
        span = hi[axis] - lo[axis]
        for at, keep_above in ((lo[axis] + low * span, True), (lo[axis] + high * span, False)):
            if (keep_above and low <= 0.0) or (not keep_above and high >= 1.0):
                continue
            bm = bmesh.new()
            bm.from_mesh(me)
            normal = Vector((0.0, 0.0, 0.0))
            normal[axis] = 1.0 if keep_above else -1.0
            point = Vector((0.0, 0.0, 0.0))
            point[axis] = at
            geom = bm.verts[:] + bm.edges[:] + bm.faces[:]
            result = bmesh.ops.bisect_plane(bm, geom=geom, plane_co=point, plane_no=normal, clear_inner=True)
            edges = [e for e in result["geom_cut"] if isinstance(e, bmesh.types.BMEdge)]
            if edges:
                bmesh.ops.holes_fill(bm, edges=edges, sides=0)
            bm.to_mesh(me)
            bm.free()
    me.update()


def scale_for(spec, size):
    if "size" in spec:
        w, d, h = spec["size"]
        return Matrix.Diagonal((w / size.x, d / size.y, h / size.z, 1.0))
    axis, length = next((i, spec[k]) for i, k in enumerate(("width", "depth", "height")) if k in spec)
    return Matrix.Scale(length / size[axis], 4)


def fit(ob, spec):
    me = ob.data
    me.transform(ob.matrix_world)
    ob.matrix_world = Matrix.Identity(4)
    me.transform(Matrix.Rotation(math.radians(spec.get("turn", 0)), 4, "Z"))
    lo, hi = bounds(me)
    mid = (lo + hi) / 2
    origin = {
        "floor": Vector((mid.x, mid.y, lo.z)),
        "top": Vector((mid.x, mid.y, hi.z)),
        "wall": Vector((mid.x, hi.y, lo.z)),
        "hang": Vector((mid.x, hi.y, mid.z)),
        "centre": mid,
    }[spec.get("origin", "floor")]
    me.transform(scale_for(spec, hi - lo) @ Matrix.Translation(-origin))
    me.update()


def dress(ob, name, spec, raw_dir):
    for n, mat in enumerate(ob.data.materials):
        mat.name = f"san_{name}_{n}"
        mat.use_backface_culling = not spec.get("two_sided", False)
        nodes = mat.node_tree.nodes
        for i, node in enumerate(nd for nd in nodes if nd.type == "TEX_IMAGE" and nd.image):
            img = node.image
            if img.size[0] > spec["tex"]:
                img.scale(spec["tex"], spec["tex"])
            img.name = f"san_{name}_{n}_{i}"
            img.pack()
        glow = os.path.join(raw_dir, name, f"{name}_emissive.png")
        if spec.get("glow") and os.path.exists(glow) and n == 0:
            bsdf = next(nd for nd in nodes if nd.type == "BSDF_PRINCIPLED")
            tex = nodes.new("ShaderNodeTexImage")
            tex.image = bpy.data.images.load(glow)
            tex.image.scale(spec["tex"], spec["tex"])
            tex.image.name = f"san_{name}_glow"
            tex.image.pack()
            mat.node_tree.links.new(tex.outputs["Color"], bsdf.inputs["Emission Color"])
            bsdf.inputs["Emission Strength"].default_value = spec["glow"]


def main(raw_dir, out_dir, only):
    os.makedirs(out_dir, exist_ok=True)
    done = []
    for name, spec in MODELS.items():
        source = spec.get("source", name)
        src = os.path.join(raw_dir, source, f"{source}.glb")
        if not os.path.exists(src) or (only and name not in only):
            continue
        bpy.ops.wm.read_factory_settings(use_empty=True)
        bpy.ops.import_scene.gltf(filepath=src)
        meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
        for other in [o for o in bpy.context.scene.objects if o.type != "MESH"]:
            bpy.data.objects.remove(other)
        bpy.ops.object.select_all(action="DESELECT")
        for o in meshes:
            o.select_set(True)
        bpy.context.view_layer.objects.active = meshes[0]
        if len(meshes) > 1:
            bpy.ops.object.join()
        ob = bpy.context.view_layer.objects.active
        ob.name = ob.data.name = f"san.{name}"
        if spec.get("cut"):
            cut(ob, spec["cut"])
        budget = TRIANGLES.get(name, BUDGET)
        if triangles(ob.data) > budget:
            thin = ob.modifiers.new("thin", "DECIMATE")
            thin.ratio = budget / triangles(ob.data)
            bpy.ops.object.modifier_apply(modifier=thin.name)
        fit(ob, spec)
        dress(ob, name, spec, raw_dir)
        ob.select_set(True)
        bpy.ops.export_scene.gltf(filepath=os.path.join(out_dir, f"{name}.glb"), export_format="GLB",
                                  use_selection=True, export_yup=True, export_apply=False,
                                  export_animations=False, export_cameras=False)
        done.append(name)
    print("library:", ", ".join(done))


if __name__ == "__main__":
    args = sys.argv[sys.argv.index("--") + 1:]
    main(args[0], args[1], set(args[2:]))
