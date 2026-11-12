import math
import os
import sys

import bpy
from mathutils import Matrix, Vector

MODELS = {
    "sofa": dict(size=(1.75, 0.9, 0.92), tex=1024),
    "chair_salon": dict(size=(0.62, 0.62, 0.98), tex=1024),
    "chair_hall": dict(size=(0.5, 0.55, 0.95), tex=512),
    "armchair_leather": dict(size=(0.9, 0.88, 1.0), tex=1024),
    "console": dict(size=(1.4, 0.48, 0.85), tex=1024),
    "clock": dict(size=(0.62, 0.4, 2.1), tex=1024),
    "gong": dict(size=(0.92, 0.5, 1.2), tex=1024),
    "sidetable": dict(size=(0.46, 0.46, 0.6), tex=512),
    "chest": dict(size=(0.96, 0.52, 0.9), tex=1024),
    "coffee_table": dict(size=(1.0, 0.56, 0.45), turn=90, tex=512),
    "bookcase": dict(size=(1.5, 0.38, 2.2), tex=1024),
    "chandelier": dict(size=(1.1, 1.1, 1.1), origin="top", tex=1024, glow=4.0),
    "sconce": dict(size=(0.4, 0.25, 0.37), origin="hang", tex=512, glow=4.0),
    "fireplace_marble": dict(size=(1.8, 0.45, 1.45), origin="wall", tex=1024),
    "curtains": dict(size=(2.0, 0.3, 2.9), origin="wall", tex=1024, two_sided=True),
    "painting_landscape": dict(size=(1.2, 0.08, 0.9), origin="hang", tex=1024),
    "rug_turkey": dict(size=(1.6, 2.4, 0.015), tilt=-90, tex=1024, two_sided=True),
}


def bounds(me):
    lo = Vector((min(v.co[i] for v in me.vertices) for i in range(3)))
    hi = Vector((max(v.co[i] for v in me.vertices) for i in range(3)))
    return lo, hi


def fit(ob, spec):
    me = ob.data
    me.transform(ob.matrix_world)
    ob.matrix_world = Matrix.Identity(4)
    me.transform(Matrix.Rotation(math.radians(spec.get("turn", 0)), 4, "Z")
                 @ Matrix.Rotation(math.radians(spec.get("tilt", 0)), 4, "X"))
    lo, hi = bounds(me)
    size = hi - lo
    mid = (lo + hi) / 2
    origin = {
        "floor": Vector((mid.x, mid.y, lo.z)),
        "top": Vector((mid.x, mid.y, hi.z)),
        "wall": Vector((mid.x, hi.y, lo.z)),
        "hang": Vector((mid.x, hi.y, mid.z)),
    }[spec.get("origin", "floor")]
    w, d, h = spec["size"]
    me.transform(Matrix.Diagonal((w / size.x, d / size.y, h / size.z, 1.0))
                 @ Matrix.Translation(-origin))
    me.update()


def dress(ob, name, spec, raw_dir):
    mat = ob.data.materials[0]
    mat.name = f"lib_{name}"
    mat.use_backface_culling = not spec.get("two_sided", False)
    nodes = mat.node_tree.nodes
    for n, node in enumerate(nd for nd in nodes if nd.type == "TEX_IMAGE" and nd.image):
        img = node.image
        if img.size[0] > spec["tex"]:
            img.scale(spec["tex"], spec["tex"])
        img.name = f"lib_{name}_{n}"
        img.pack()
    glow = os.path.join(raw_dir, name, f"{name}_emissive.png")
    if spec.get("glow") and os.path.exists(glow):
        bsdf = next(nd for nd in nodes if nd.type == "BSDF_PRINCIPLED")
        tex = nodes.new("ShaderNodeTexImage")
        tex.image = bpy.data.images.load(glow)
        tex.image.scale(spec["tex"], spec["tex"])
        tex.image.name = f"lib_{name}_glow"
        tex.image.pack()
        mat.node_tree.links.new(tex.outputs["Color"], bsdf.inputs["Emission Color"])
        bsdf.inputs["Emission Strength"].default_value = spec["glow"]


def main(raw_dir, out_dir, only):
    os.makedirs(out_dir, exist_ok=True)
    done = []
    for name, spec in MODELS.items():
        src = os.path.join(raw_dir, name, f"{name}.glb")
        if not os.path.exists(src) or (only and name not in only):
            continue
        bpy.ops.wm.read_factory_settings(use_empty=True)
        bpy.ops.import_scene.gltf(filepath=src)
        ob = next(o for o in bpy.context.scene.objects if o.type == "MESH")
        for other in [o for o in bpy.context.scene.objects if o is not ob]:
            bpy.data.objects.remove(other)
        ob.name = ob.data.name = f"lib.{name}"
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
