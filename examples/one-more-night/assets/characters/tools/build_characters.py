import math
import os

import bmesh
import bpy
import numpy as np
from mathutils import Vector

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RIG = os.path.join(HERE, "UAL1.glb")
EXPORT = os.path.join(HERE, "export")
CLIPS = ("Idle_Loop", "Walk_Loop", "Sitting_Idle_Loop", "Sitting_Enter", "Sitting_Exit")

SKIN = (0.78, 0.6, 0.5)
BLACK = (0.03, 0.03, 0.035)
WHITE = (0.88, 0.87, 0.83)
BRASS = (0.8, 0.6, 0.22)

LOOKS = {
    "lady": {
        "hair": (0.68, 0.67, 0.66),
        "coat": (0.36, 0.07, 0.11),
        "arms": SKIN,
        "forearms": WHITE,
        "hands": WHITE,
        "thighs": (0.36, 0.07, 0.11),
        "calves": WHITE,
        "shoes": (0.2, 0.04, 0.06),
        "skirt": (0.36, 0.07, 0.11),
        "bun": True,
        "brooch": True,
        "portrait": 1,
    },
    "gentleman": {
        "hair": (0.16, 0.1, 0.06),
        "coat": BLACK,
        "arms": BLACK,
        "forearms": BLACK,
        "hands": SKIN,
        "thighs": (0.1, 0.1, 0.11),
        "calves": (0.1, 0.1, 0.11),
        "shoes": BLACK,
        "tails": True,
        "shirt": True,
        "moustache": True,
        "portrait": 2,
    },
    "footman": {
        "hair": (0.5, 0.36, 0.18),
        "coat": (0.1, 0.24, 0.15),
        "arms": (0.1, 0.24, 0.15),
        "forearms": (0.1, 0.24, 0.15),
        "hands": WHITE,
        "thighs": (0.8, 0.74, 0.6),
        "calves": WHITE,
        "shoes": BLACK,
        "tails": True,
        "buttons": True,
        "collar": True,
        "portrait": 3,
    },
    "guest": {
        "hair": (0.32, 0.14, 0.08),
        "coat": (0.09, 0.13, 0.27),
        "arms": (0.09, 0.13, 0.27),
        "forearms": (0.09, 0.13, 0.27),
        "hands": SKIN,
        "thighs": (0.33, 0.33, 0.35),
        "calves": (0.33, 0.33, 0.35),
        "shoes": (0.22, 0.12, 0.06),
        "shirt": True,
    },
    "maid": {
        "hair": (0.24, 0.15, 0.09),
        "coat": BLACK,
        "arms": BLACK,
        "forearms": BLACK,
        "hands": SKIN,
        "thighs": BLACK,
        "calves": BLACK,
        "shoes": BLACK,
        "skirt": BLACK,
        "collar": True,
        "apron": True,
        "cap": True,
    },
    "colonel": {
        "hair": (0.8, 0.8, 0.78),
        "coat": (0.42, 0.36, 0.2),
        "arms": (0.42, 0.36, 0.2),
        "forearms": (0.42, 0.36, 0.2),
        "hands": SKIN,
        "thighs": (0.3, 0.2, 0.12),
        "calves": (0.3, 0.2, 0.12),
        "shoes": (0.22, 0.12, 0.06),
        "shirt": True,
        "moustache": True,
        "portrait": 4,
    },
    "debutante": {
        "hair": (0.85, 0.68, 0.3),
        "coat": (0.45, 0.72, 0.68),
        "arms": SKIN,
        "forearms": SKIN,
        "hands": SKIN,
        "thighs": (0.45, 0.72, 0.68),
        "calves": WHITE,
        "shoes": (0.8, 0.74, 0.6),
        "skirt": (0.45, 0.72, 0.68),
        "brooch": True,
        "portrait": 5,
    },
    "doctor": {
        "hair": (0.08, 0.07, 0.07),
        "coat": (0.42, 0.43, 0.46),
        "arms": (0.42, 0.43, 0.46),
        "forearms": (0.42, 0.43, 0.46),
        "hands": SKIN,
        "thighs": (0.42, 0.43, 0.46),
        "calves": (0.42, 0.43, 0.46),
        "shoes": BLACK,
        "shirt": True,
        "spectacles": True,
        "portrait": 6,
    },
    "butler": {
        "hair": (0.55, 0.55, 0.56),
        "coat": BLACK,
        "arms": BLACK,
        "forearms": BLACK,
        "hands": WHITE,
        "thighs": (0.3, 0.3, 0.32),
        "calves": (0.3, 0.3, 0.32),
        "shoes": BLACK,
        "tails": True,
        "shirt": True,
        "portrait": 7,
    },
    "cook": {
        "hair": (0.55, 0.2, 0.08),
        "coat": WHITE,
        "arms": WHITE,
        "forearms": SKIN,
        "hands": SKIN,
        "thighs": WHITE,
        "calves": BLACK,
        "shoes": BLACK,
        "skirt": (0.55, 0.62, 0.72),
        "apron": True,
        "cap": True,
        "portrait": 8,
    },
    "housekeeper": {
        "hair": (0.2, 0.18, 0.18),
        "coat": (0.2, 0.14, 0.26),
        "arms": (0.2, 0.14, 0.26),
        "forearms": (0.2, 0.14, 0.26),
        "hands": SKIN,
        "thighs": (0.2, 0.14, 0.26),
        "calves": BLACK,
        "shoes": BLACK,
        "skirt": (0.2, 0.14, 0.26),
        "collar": True,
        "bun": True,
        "brooch": True,
        "portrait": 9,
    },
    "chauffeur": {
        "hair": (0.3, 0.2, 0.1),
        "coat": (0.16, 0.2, 0.3),
        "arms": (0.16, 0.2, 0.3),
        "forearms": BLACK,
        "hands": BLACK,
        "thighs": (0.16, 0.2, 0.3),
        "calves": BLACK,
        "shoes": BLACK,
        "buttons": True,
        "cap": True,
        "cap_colour": (0.16, 0.2, 0.3),
        "peak": True,
        "portrait": 10,
    },
}


def clear():
    for collection in (
        bpy.data.objects,
        bpy.data.meshes,
        bpy.data.materials,
        bpy.data.images,
        bpy.data.lights,
        bpy.data.cameras,
        bpy.data.armatures,
        bpy.data.actions,
    ):
        for block in list(collection):
            collection.remove(block)


class Figure:
    def __init__(self, name, armature, joints):
        self.name = name
        self.armature = armature
        self.joints = joints
        self.parts = []
        self.materials = {}

    def material(self, colour, metallic=0.0, roughness=0.75):
        key = (colour, metallic)
        if key not in self.materials:
            made = bpy.data.materials.new(f"{self.name}_{len(self.materials)}")
            made.use_nodes = True
            shader = made.node_tree.nodes["Principled BSDF"]
            shader.inputs["Base Color"].default_value = (*colour, 1.0)
            shader.inputs["Roughness"].default_value = roughness
            shader.inputs["Metallic"].default_value = metallic
            self.materials[key] = made
        return self.materials[key]

    def finish(self, build, colour, weights, metallic=0.0):
        mesh = bpy.data.meshes.new(f"{self.name}_part")
        build.to_mesh(mesh)
        build.free()
        mesh.materials.append(self.material(colour, metallic))
        obj = bpy.data.objects.new(f"{self.name}_part", mesh)
        bpy.context.scene.collection.objects.link(obj)
        for index, shares in enumerate(weights(mesh)):
            for bone, share in shares.items():
                group = obj.vertex_groups.get(bone) or obj.vertex_groups.new(name=bone)
                group.add([index], share, "REPLACE")
        self.parts.append(obj)

    def rigid(self, bone):
        return lambda mesh: [{bone: 1.0} for _ in mesh.vertices]

    def prism(self, start, end, start_radii, end_radii, colour, bone, sides=8):
        start, end = Vector(start), Vector(end)
        along = (end - start).normalized()
        reference = Vector((0, 1, 0)) if abs(along.z) > 0.7 else Vector((0, 0, 1))
        across = reference.cross(along).normalized()
        deep = along.cross(across)
        build = bmesh.new()
        rings = []
        for centre, radii in ((start, start_radii), (end, end_radii)):
            ring = []
            for side in range(sides):
                angle = 2 * math.pi * (side + 0.5) / sides
                ring.append(
                    build.verts.new(
                        centre
                        + across * math.cos(angle) * radii[0]
                        + deep * math.sin(angle) * radii[1]
                    )
                )
            rings.append(ring)
        for side in range(sides):
            after = (side + 1) % sides
            build.faces.new((rings[0][side], rings[0][after], rings[1][after], rings[1][side]))
        build.faces.new(list(reversed(rings[0])))
        build.faces.new(rings[1])
        self.finish(build, colour, self.rigid(bone))

    def box(self, centre, size, colour, bone, metallic=0.0):
        build = bmesh.new()
        bmesh.ops.create_cube(build, size=1.0)
        for vertex in build.verts:
            vertex.co = Vector(centre) + Vector(
                (vertex.co.x * size[0], vertex.co.y * size[1], vertex.co.z * size[2])
            )
        self.finish(build, colour, self.rigid(bone), metallic)

    def ball(self, centre, radii, colour, bone, metallic=0.0, segments=10):
        build = bmesh.new()
        bmesh.ops.create_uvsphere(build, u_segments=segments, v_segments=segments // 2 + 1, radius=1.0)
        for vertex in build.verts:
            vertex.co = Vector(centre) + Vector(
                (vertex.co.x * radii[0], vertex.co.y * radii[1], vertex.co.z * radii[2])
            )
        self.finish(build, colour, self.rigid(bone), metallic)

    def skirt(self, colour):
        levels = (
            (1.06, (0.16, 0.11), None),
            (0.56, (0.25, 0.21), "thigh"),
            (0.05, (0.31, 0.28), "calf"),
        )
        sides = 12
        build = bmesh.new()
        rings = []
        shares = []
        for height, radii, limb in levels:
            ring = []
            for side in range(sides):
                angle = 2 * math.pi * (side + 0.5) / sides
                x = math.cos(angle) * radii[0]
                ring.append(build.verts.new((x, math.sin(angle) * radii[1] + 0.01, height)))
                if limb is None:
                    shares.append({"pelvis": 1.0})
                else:
                    left = min(max(0.5 + x / (radii[0] * 1.2), 0.0), 1.0)
                    shares.append({f"{limb}_l": left * 0.7, f"{limb}_r": (1 - left) * 0.7, "pelvis": 0.3})
            rings.append(ring)
        for low, high in zip(rings, rings[1:]):
            for side in range(sides):
                after = (side + 1) % sides
                build.faces.new((low[side], high[side], high[after], low[after]))
        build.verts.ensure_lookup_table()
        self.finish(build, colour, lambda mesh: shares)

    def join(self):
        bpy.ops.object.select_all(action="DESELECT")
        for part in self.parts:
            part.select_set(True)
        bpy.context.view_layer.objects.active = self.parts[0]
        bpy.ops.object.join()
        body = self.parts[0]
        body.name = self.name
        body.data.name = self.name
        body.parent = self.armature
        modifier = body.modifiers.new("Armature", "ARMATURE")
        modifier.object = self.armature
        return body


def figure(name, look, armature, joints):
    made = Figure(name, armature, joints)
    at = joints
    coat = look["coat"]

    made.prism(at["pelvis"] - Vector((0, 0, 0.09)), at["pelvis"], (0.15, 0.1), (0.165, 0.105), look["thighs"], "pelvis")
    made.prism(at["pelvis"], at["spine_01"], (0.165, 0.105), (0.15, 0.1), coat, "pelvis")
    made.prism(at["spine_01"], at["spine_02"], (0.15, 0.1), (0.155, 0.105), coat, "spine_01")
    made.prism(at["spine_02"], at["spine_03"], (0.155, 0.105), (0.185, 0.118), coat, "spine_02")
    made.prism(at["spine_03"], at["neck_01"] - Vector((0, 0, 0.02)), (0.185, 0.118), (0.15, 0.095), coat, "spine_03")
    made.prism(at["neck_01"] - Vector((0, 0, 0.03)), at["Head"] + Vector((0, 0, 0.02)), (0.045, 0.045), (0.042, 0.042), SKIN, "neck_01")

    head = at["Head"] + Vector((0, -0.012, 0.105))
    made.ball(head, (0.088, 0.1, 0.112), SKIN, "Head")
    made.ball(head + Vector((0, 0.022, 0.026)), (0.094, 0.1, 0.108), look["hair"], "Head")
    made.box(head + Vector((0, -0.1, -0.012)), (0.024, 0.03, 0.036), SKIN, "Head")
    for side in (-1, 1):
        made.box(head + Vector((side * 0.034, -0.093, 0.016)), (0.016, 0.008, 0.012), BLACK, "Head")
    if look.get("moustache"):
        made.box(head + Vector((0, -0.094, -0.04)), (0.062, 0.014, 0.014), look["hair"], "Head")
    if look.get("bun"):
        made.ball(head + Vector((0, 0.1, 0.05)), (0.05, 0.045, 0.05), look["hair"], "Head")

    for side, sign in (("l", 1), ("r", -1)):
        shoulder, elbow, wrist = at[f"upperarm_{side}"], at[f"lowerarm_{side}"], at[f"hand_{side}"]
        made.ball(shoulder, (0.068, 0.068, 0.068), coat, f"upperarm_{side}", segments=8)
        made.prism(shoulder, elbow, (0.054, 0.054), (0.046, 0.046), look["arms"], f"upperarm_{side}")
        made.prism(elbow, wrist, (0.046, 0.046), (0.036, 0.036), look["forearms"], f"lowerarm_{side}")
        made.box(wrist + Vector((sign * 0.055, 0, 0)), (0.11, 0.075, 0.032), look["hands"], f"hand_{side}")
        hip, knee, ankle = at[f"thigh_{side}"], at[f"calf_{side}"], at[f"foot_{side}"]
        made.prism(hip, knee, (0.082, 0.082), (0.06, 0.06), look["thighs"], f"thigh_{side}")
        made.prism(knee, ankle, (0.058, 0.058), (0.042, 0.042), look["calves"], f"calf_{side}")
        made.box((ankle.x, ankle.y - 0.07, 0.045), (0.085, 0.25, 0.09), look["shoes"], f"foot_{side}")

    if look.get("skirt"):
        made.skirt(look["skirt"])
    if look.get("tails"):
        for side in (-1, 1):
            made.box((side * 0.062, 0.105, 0.74), (0.105, 0.03, 0.4), coat, "pelvis")
    chest = at["spine_03"]
    if look.get("shirt"):
        made.box((0, -0.112, chest.z + 0.03), (0.11, 0.03, 0.22), WHITE, "spine_03")
        made.box((0, -0.118, chest.z + 0.15), (0.075, 0.02, 0.03), BLACK, "spine_03")
    if look.get("collar"):
        made.box((0, -0.1, chest.z + 0.13), (0.09, 0.03, 0.05), WHITE, "spine_03")
    if look.get("buttons"):
        for row in range(3):
            for side in (-1, 1):
                made.ball((side * 0.05, -0.118, chest.z - 0.1 + row * 0.09), (0.014, 0.014, 0.014), BRASS, "spine_03" if row else "spine_02", metallic=1.0, segments=6)
    if look.get("apron"):
        made.box((0, -0.125, chest.z - 0.02), (0.2, 0.02, 0.26), WHITE, "spine_02")
        made.box((0, -0.19, 0.72), (0.3, 0.02, 0.5), WHITE, "pelvis")
    if look.get("cap"):
        made.ball(head + Vector((0, 0.02, 0.1)), (0.085, 0.085, 0.04), look.get("cap_colour", WHITE), "Head", segments=8)
    if look.get("peak"):
        made.box(head + Vector((0, -0.095, 0.082)), (0.13, 0.07, 0.012), BLACK, "Head")
    if look.get("spectacles"):
        for side in (-1, 1):
            made.box(head + Vector((side * 0.034, -0.104, 0.016)), (0.04, 0.006, 0.034), BRASS, "Head", metallic=1.0)
        made.box(head + Vector((0, -0.104, 0.022)), (0.03, 0.006, 0.008), BRASS, "Head", metallic=1.0)
    if look.get("brooch"):
        made.ball((0, -0.12, chest.z + 0.07), (0.02, 0.012, 0.02), BRASS, "spine_03", metallic=1.0, segments=6)
    return made.join()


def export(armature, body, name):
    bpy.ops.object.select_all(action="DESELECT")
    armature.select_set(True)
    body.select_set(True)
    bpy.context.view_layer.objects.active = armature
    bpy.ops.export_scene.gltf(
        filepath=os.path.join(EXPORT, name + ".glb"),
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_animations=True,
        export_animation_mode="NLA_TRACKS",
        export_skins=True,
    )


def lineup(bodies):
    scene = bpy.context.scene
    camera = bpy.data.objects.new("camera", bpy.data.cameras.new("camera"))
    camera.location = (0, -5.2, 1.0)
    camera.rotation_euler = (math.radians(89), 0, 0)
    camera.data.lens = 50
    scene.collection.objects.link(camera)
    scene.camera = camera
    for at, energy in (((2.5, -3.5, 3.0), 500), ((-3.0, -2.0, 2.0), 180)):
        lamp = bpy.data.objects.new("lamp", bpy.data.lights.new("lamp", "POINT"))
        lamp.data.energy = energy
        lamp.data.color = (1.0, 0.85, 0.7)
        lamp.location = at
        scene.collection.objects.link(lamp)
    scene.world = scene.world or bpy.data.worlds.new("world")
    scene.world.use_nodes = True
    scene.world.node_tree.nodes["Background"].inputs[0].default_value = (0.02, 0.025, 0.04, 1.0)
    scene.render.resolution_x = 380
    scene.render.resolution_y = 620
    camera.location = (0, -3.6, 0.95)
    shots = []
    for body in bodies:
        for other in bodies:
            other.hide_render = other is not body
        scene.render.filepath = os.path.join(HERE, f"lineup_{body.name}.png")
        bpy.ops.render.render(write_still=True)
        shot = bpy.data.images.load(scene.render.filepath)
        pixels = np.empty(380 * 620 * 4, dtype=np.float32)
        shot.pixels.foreach_get(pixels)
        shots.append(pixels.reshape(620, 380, 4))
        bpy.data.images.remove(shot)
        os.remove(scene.render.filepath)
    scene.render.resolution_x = 256
    scene.render.resolution_y = 256
    camera.location = (0, -1.05, 1.6)
    camera.rotation_euler = (math.radians(90), 0, 0)
    camera.data.lens = 85
    os.makedirs(os.path.join(HERE, "portraits"), exist_ok=True)
    for body in bodies:
        number = LOOKS[body.name].get("portrait")
        if number is None:
            continue
        for other in bodies:
            other.hide_render = other is not body
        scene.render.filepath = os.path.join(HERE, "portraits", f"portrait_{number:02}.png")
        bpy.ops.render.render(write_still=True)
    for body in bodies:
        body.hide_render = False
    sheet = np.concatenate(shots, axis=1)
    image = bpy.data.images.new("lineup", sheet.shape[1], sheet.shape[0])
    image.pixels.foreach_set(sheet.ravel())
    image.filepath_raw = os.path.join(HERE, "lineup.png")
    image.file_format = "PNG"
    image.save()
    bpy.data.images.remove(image)


def build():
    os.makedirs(EXPORT, exist_ok=True)
    clear()
    bpy.ops.import_scene.gltf(filepath=RIG)
    armature = next(obj for obj in bpy.data.objects if obj.type == "ARMATURE")
    for obj in [obj for obj in bpy.data.objects if obj.type == "MESH"]:
        bpy.data.objects.remove(obj, do_unlink=True)
    tracks = armature.animation_data.nla_tracks
    for track in [track for track in tracks if track.name not in CLIPS]:
        tracks.remove(track)
    for action in [action for action in bpy.data.actions if action.name not in CLIPS]:
        bpy.data.actions.remove(action)
    bpy.context.view_layer.update()
    joints = {bone.name: armature.matrix_world @ bone.head_local for bone in armature.data.bones}
    bodies = []
    for name, look in LOOKS.items():
        body = figure(name, look, armature, joints)
        export(armature, body, name)
        bodies.append(body)
    lineup(bodies)
    bpy.ops.wm.save_as_mainfile(filepath=os.path.join(HERE, "characters.blend"))
    return sorted(os.listdir(EXPORT))


BUILT = build()
