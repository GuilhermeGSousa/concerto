import json
import struct
import sys

import bpy
import numpy as np
from mathutils import Matrix, Vector

SIDES = {"Left": "l", "Right": "r"}
FINGERS = {"Thumb": "thumb", "Index": "index", "Middle": "middle", "Ring": "ring", "Pinky": "pinky"}
UNSCALED = {"Head", "neck_01"}


def bone_map():
    names = {
        "Hips": "pelvis",
        "Spine": "spine_01",
        "Spine1": "spine_02",
        "Spine2": "spine_03",
        "Neck": "neck_01",
        "Head": "Head",
    }
    for side, s in SIDES.items():
        names[f"{side}Shoulder"] = f"clavicle_{s}"
        names[f"{side}Arm"] = f"upperarm_{s}"
        names[f"{side}ForeArm"] = f"lowerarm_{s}"
        names[f"{side}Hand"] = f"hand_{s}"
        names[f"{side}UpLeg"] = f"thigh_{s}"
        names[f"{side}Leg"] = f"calf_{s}"
        names[f"{side}Foot"] = f"foot_{s}"
        names[f"{side}ToeBase"] = f"ball_{s}"
        for finger, f in FINGERS.items():
            for i in range(1, 4):
                names[f"{side}Hand{finger}{i}"] = f"{f}_0{i}_{s}"
    return names


TAILS = {
    "pelvis": "spine_01",
    "spine_01": "spine_02",
    "spine_02": "spine_03",
    "spine_03": "neck_01",
    "neck_01": "Head",
}
for s in SIDES.values():
    TAILS.update(
        {
            f"clavicle_{s}": f"upperarm_{s}",
            f"upperarm_{s}": f"lowerarm_{s}",
            f"lowerarm_{s}": f"hand_{s}",
            f"hand_{s}": f"middle_01_{s}",
            f"thigh_{s}": f"calf_{s}",
            f"calf_{s}": f"foot_{s}",
            f"foot_{s}": f"ball_{s}",
        }
    )
    for f in FINGERS.values():
        TAILS[f"{f}_01_{s}"] = f"{f}_02_{s}"
        TAILS[f"{f}_02_{s}"] = f"{f}_03_{s}"


def heads(armature, rename):
    out = {}
    for bone in armature.data.bones:
        name = rename(bone.name)
        if name:
            out[name] = (
                armature.matrix_world @ bone.head_local,
                armature.matrix_world @ bone.tail_local,
            )
    return out


def segment(joints, name):
    head, tail = joints[name]
    if name in TAILS:
        tail = joints[TAILS[name]][0]
    return head, tail


def lateral(joints, name):
    side = name[-1]
    if name.startswith(("foot", "ball")):
        return Vector((0.0, 0.0, 1.0))
    if name[-2:] in ("_l", "_r") and not name.startswith(("thigh", "calf", "foot", "ball", "clavicle")):
        return joints[f"index_01_{side}"][0] - joints[f"pinky_01_{side}"][0]
    return Vector((0.0, -1.0, 0.0))


def frame(direction, reference):
    x = direction.normalized()
    z = x.cross(reference)
    if z.length < 1e-6:
        z = x.cross(Vector((0.0, 0.0, 1.0)))
    z.normalize()
    y = z.cross(x)
    return Matrix((x, y, z)).transposed()


def transform(source, target, name):
    sh, st = segment(source, name)
    th, tt = segment(target, name)
    fs = frame(st - sh, lateral(source, name))
    ft = frame(tt - th, lateral(target, name))
    stretch = 1.0
    if name not in UNSCALED:
        stretch = min(max((tt - th).length / max((st - sh).length, 1e-6), 0.8), 1.25)
    scale = Matrix.Diagonal((stretch, 1.0, 1.0))
    linear = ft @ scale @ fs.transposed()
    return Matrix.Translation(th) @ linear.to_4x4() @ Matrix.Translation(-sh)


def main(rodin_path, ual1_path, out_path):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=ual1_path)
    ual1 = next(o for o in bpy.context.scene.objects if o.type == "ARMATURE")
    ual1.name = "Armature"
    ual1.animation_data_clear()
    for obj in [o for o in bpy.context.scene.objects if o.type == "MESH"]:
        bpy.data.objects.remove(obj)
    target = heads(ual1, lambda n: n)

    bpy.ops.import_scene.gltf(filepath=rodin_path)
    rodin = next(o for o in bpy.context.scene.objects if o.type == "ARMATURE" and o != ual1)
    body = next(o for o in rodin.children if o.type == "MESH")
    for obj in [o for o in bpy.context.scene.objects if o.type == "MESH" and o != body]:
        bpy.data.objects.remove(obj)

    names = bone_map()
    raw = heads(rodin, names.get)
    corners = np.array([body.matrix_world @ Vector(c) for c in body.bound_box])
    floor = corners[:, 2].min()
    scale = target["Head"][0].z / (raw["Head"][0].z - floor)
    lift = Vector((0.0, 0.0, -floor))
    source = {k: ((h + lift) * scale, (t + lift) * scale) for k, (h, t) in raw.items()}

    mesh = body.data
    count = len(mesh.vertices)
    world = np.array([(body.matrix_world @ v.co + lift) * scale for v in mesh.vertices])
    homogeneous = np.hstack([world, np.ones((count, 1))])
    group_names = {g.index: names.get(g.name) for g in body.vertex_groups}
    matrices = {name: np.array(transform(source, target, name)) for name in source}

    moved = np.zeros((count, 3))
    total = np.zeros(count)
    for v in mesh.vertices:
        for g in v.groups:
            name = group_names.get(g.group)
            if name and g.weight > 0.0:
                moved[v.index] += g.weight * (matrices[name] @ homogeneous[v.index])[:3]
                total[v.index] += g.weight
    unweighted = total == 0.0
    total[unweighted] = 1.0
    moved /= total[:, None]
    moved[unweighted] = world[unweighted]

    body.parent = None
    body.matrix_world = Matrix.Identity(4)
    for v, co in zip(mesh.vertices, moved):
        v.co = co
    if mesh.has_custom_normals:
        bpy.context.view_layer.objects.active = body
        bpy.ops.mesh.customdata_custom_splitnormals_clear()
    mesh.shade_smooth()
    mesh.update()

    for group in list(body.vertex_groups):
        name = names.get(group.name)
        if name:
            group.name = name
        else:
            body.vertex_groups.remove(group)
    for modifier in list(body.modifiers):
        body.modifiers.remove(modifier)
    bpy.data.objects.remove(rodin)

    body.name = "Character"
    body.parent = ual1
    modifier = body.modifiers.new("Armature", "ARMATURE")
    modifier.object = ual1

    bpy.ops.object.select_all(action="DESELECT")
    ual1.select_set(True)
    body.select_set(True)
    bpy.context.view_layer.objects.active = ual1
    bpy.ops.export_scene.gltf(
        filepath=out_path,
        export_format="GLB",
        use_selection=True,
        export_animations=False,
        export_skins=True,
    )
    match_joint_order(out_path, ual1_path)
    return {"scale": scale, "vertices": count, "unweighted": int(unweighted.sum())}


def read_glb(path):
    data = open(path, "rb").read()
    json_length = struct.unpack("<I", data[12:16])[0]
    document = json.loads(data[20 : 20 + json_length])
    binary = bytearray(data[28 + json_length :])
    return document, binary


def write_glb(path, document, binary):
    text = json.dumps(document, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    binary += b"\0" * (-len(binary) % 4)
    total = 12 + 8 + len(text) + 8 + len(binary)
    with open(path, "wb") as out:
        out.write(struct.pack("<III", 0x46546C67, 2, total))
        out.write(struct.pack("<I4s", len(text), b"JSON") + text)
        out.write(struct.pack("<I4s", len(binary), b"BIN\0") + bytes(binary))


def joint_names(document):
    return [document["nodes"][i]["name"] for i in document["skins"][0]["joints"]]


def match_joint_order(path, reference_path):
    document, binary = read_glb(path)
    reference, _ = read_glb(reference_path)
    wanted = joint_names(reference)
    have = joint_names(document)
    if wanted == have:
        return
    skin = document["skins"][0]
    by_name = {name: node for name, node in zip(have, skin["joints"])}
    remap = [wanted.index(name) for name in have]
    skin["joints"] = [by_name[name] for name in wanted]

    matrices = document["accessors"][skin["inverseBindMatrices"]]
    view = document["bufferViews"][matrices["bufferView"]]
    start = view.get("byteOffset", 0) + matrices.get("byteOffset", 0)
    old = [bytes(binary[start + 64 * i : start + 64 * (i + 1)]) for i in range(len(have))]
    for i, block in enumerate(old):
        target = start + 64 * remap[i]
        binary[target : target + 64] = block

    formats = {5121: "B", 5123: "H"}
    for mesh in document["meshes"]:
        for primitive in mesh["primitives"]:
            for key, index in primitive["attributes"].items():
                if not key.startswith("JOINTS_"):
                    continue
                accessor = document["accessors"][index]
                kind = formats[accessor["componentType"]]
                size = struct.calcsize(kind)
                view = document["bufferViews"][accessor["bufferView"]]
                stride = view.get("byteStride", 4 * size)
                base = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
                for vertex in range(accessor["count"]):
                    at = base + vertex * stride
                    joints = struct.unpack_from(f"<4{kind}", binary, at)
                    struct.pack_into(f"<4{kind}", binary, at, *(remap[j] for j in joints))
    write_glb(path, document, binary)


if __name__ == "__main__":
    argv = sys.argv[sys.argv.index("--") + 1 :]
    print("FIT", main(*argv))
