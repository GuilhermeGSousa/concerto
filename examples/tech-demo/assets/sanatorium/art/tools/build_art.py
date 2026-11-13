import math
import os
import re
import sys

import bpy
import numpy as np
from mathutils import Matrix, Vector
from mathutils.bvhtree import BVHTree

HERE = os.path.dirname(os.path.abspath(__file__))
ART = os.path.dirname(HERE)
SANATORIUM = os.path.dirname(ART)
TEX_DIR = os.path.join(ART, "textures")
LIBRARY = os.path.join(ART, "library")
OUT = os.path.join(ART, "export", "sanatorium_art.glb")

AREAS = [
    "EXT_Funicular",
    "EXT_Approach",
    "EXT_Balconies",
    "INT_Lodge",
    "INT_WardCorridor",
    "INT_BrotherWing",
    "INT_DirectorWing",
    "INT_Boiler",
    "INT_Service",
]

SURFACES = {
    "snow": dict(tex="snow", tile=4.0),
    "snow_packed": dict(tex="snow_packed", tile=3.0),
    "rock": dict(tex="rock", tile=8.0, tint=(0.8, 0.82, 0.88)),
    "stucco": dict(tex="stucco", tile=3.0, tint=(0.96, 0.93, 0.86)),
    "stone": dict(tex="stone", tile=2.5, tint=(0.85, 0.85, 0.85)),
    "brick": dict(tex="brick", tile=2.0, tint=(0.62, 0.55, 0.52)),
    "larch": dict(tex="larch", tile=2.0),
    "shutter": dict(tex="larch", tile=2.0, tint=(0.5, 0.75, 0.58)),
    "frame": dict(flat=(0.82, 0.81, 0.76), rough=0.45),
    "metal": dict(tex="metal", tile=2.0, metal=0.5),
    "slate": dict(tex="slate", tile=3.0),
    "wall_tile": dict(tex="wall_tile", tile=1.2),
    "wall_paint": dict(tex="wall_paint", tile=3.0, tint=(0.78, 0.88, 0.8)),
    "ceiling": dict(tex="stucco", tile=3.0, tint=(1.05, 1.05, 1.02)),
    "terrazzo": dict(tex="terrazzo", tile=2.0),
    "floor_wear": dict(tex="terrazzo", tile=2.0, tint=(0.7, 0.68, 0.64)),
    "concrete": dict(tex="concrete", tile=3.0, tint=(0.75, 0.73, 0.7)),
    "wood_floor": dict(tex="wood_floor", tile=2.0),
    "glass_dark": dict(flat=(0.02, 0.03, 0.05), rough=0.05),
    "glass_night": dict(flat=(0.03, 0.05, 0.09), rough=0.08),
    "glass_lit": dict(flat=(0.9, 0.62, 0.32), rough=0.3, glow=(1.0, 0.62, 0.3)),
    "lamp_glow": dict(flat=(1.0, 0.8, 0.5), rough=0.4, glow=(1.0, 0.75, 0.45)),
    "paper": dict(flat=(0.85, 0.82, 0.72), rough=0.9),
    "fabric": dict(flat=(0.32, 0.33, 0.36), rough=1.0),
    "conifer": dict(flat=(0.05, 0.1, 0.07), rough=0.9),
}

BY_MATERIAL = {
    "GB_Render": "stucco",
    "GB_Snow": "snow",
    "GB_SnowPacked": "snow_packed",
    "GB_Rock": "rock",
    "GB_Stone": "stone",
    "GB_Wood": "larch",
    "GB_Metal": "metal",
    "GB_GlassDark": "glass_dark",
    "GB_GlassLit": "glass_lit",
    "GB_Glass": "glass_night",
    "GB_LampGlow": "lamp_glow",
    "GB_Paper": "paper",
    "GB_Fabric": "fabric",
    "GB_Floor": "terrazzo",
    "GB_FloorDark": "concrete",
    "GB_FloorWear": "floor_wear",
    "GB_Ceiling": "ceiling",
    "GB_WallTile": "wall_tile",
    "GB_WallPaint": "wall_paint",
    "GB_Conifer": "conifer",
}

OVERRIDES = [
    (None, r"^Roof_", "GB_Render", "slate"),
    (None, r"^Roof_", "GB_Stone", "slate"),
    (None, r"^WallB_", "GB_Stone", "brick"),
    (None, r"WindowShut", "GB_Wood", "shutter"),
    (None, r"Window", "GB_Wood", "frame"),
    ("INT_DirectorWing", r"^Floor_Tile$", "GB_Floor", "wood_floor"),
]

KEEP_YAW = dict(turns=(0.0,))
REPLACE = {
    "Prop_Bed": dict(model="bed", fit="box"),
    "Prop_BedStripped": dict(model="bed_stripped", fit="box"),
    "Prop_Bench": dict(model="bench", fit="box"),
    "Prop_Boiler": dict(model="boiler", fit="height", pair=True),
    "Prop_Chair": dict(model="chair", fit="box"),
    "Prop_ChairPushedBack": dict(model="chair", fit="box"),
    "Prop_ChartRack": dict(model="chart_rack", fit="box"),
    "Prop_Clock": dict(model="wall_clock", fit="height"),
    "Prop_CoalHeap": dict(model="coal_heap", fit="box"),
    "Prop_Conifer": dict(model="spruce", fit="height", **KEEP_YAW),
    "Prop_Crate": dict(model="crate", fit="box"),
    "Prop_Crutch": dict(model="crutch", fit="height", **KEEP_YAW),
    "Prop_CureChairBalcony": dict(model="cure_chair", fit="box"),
    "Prop_CureChairTerrace": dict(model="cure_chair", fit="box"),
    "Prop_Curtain": dict(model="curtains", fit="box"),
    "Prop_Desk": dict(model="director_desk", fit="footprint"),
    "Prop_Desk_Secretary": dict(model="secretary_desk", fit="footprint"),
    "Prop_FilingCabinet": dict(model="filing_cabinet", fit="box"),
    "Prop_KeyBoard": dict(model="key_board", fit="height"),
    "Prop_LampPost": dict(model="lamp_post", fit="height", **KEEP_YAW),
    "Prop_Lantern": dict(model="lantern", fit="natural", **KEEP_YAW),
    "Prop_MealTrolley": dict(model="meal_trolley", fit="box"),
    "Prop_NurseCounter": dict(model="nurse_counter", fit="footprint"),
    "Prop_PatientLying": dict(model="patient_lying", fit="box"),
    "Prop_Pendant": dict(model="pendant", fit="box", top=True, **KEEP_YAW),
    "Prop_PorterDesk": dict(model="porter_desk", fit="footprint"),
    "Prop_Radiator": dict(model="radiator", fit="height"),
    "Prop_Revolver": dict(model="revolver", fit="natural"),
    "Prop_Schnapps": dict(model="schnapps", fit="height", **KEEP_YAW),
    "Prop_Signpost": dict(model="signpost", fit="height", **KEEP_YAW),
    "Prop_Sled": dict(model="sled", fit="box"),
    "Prop_SteamVent": dict(model="valve", fit="height"),
    "Prop_Valve": dict(model="valve", fit="height"),
    "Prop_Stove": dict(model="tiled_stove", fit="natural"),
    "Prop_Suitcase": dict(model="suitcase", fit="box"),
    "Prop_Trunk": dict(model="trunk", fit="box"),
    "Prop_Wayside": dict(model="wayside_shrine", fit="height", **KEEP_YAW),
    "Prop_Wheelchair": dict(model="wheelchair", fit="box"),
    "Prop_Workbench": dict(model="workbench", fit="footprint"),
    "Door_EntranceL": dict(model="door_oak_leaf", fit="box", **KEEP_YAW),
    "Door_EntranceR": dict(model="door_oak_leaf", fit="box", **KEEP_YAW),
    "Car": dict(model="funicular_car", fit="box", **KEEP_YAW),
}
DOOR = dict(model="door_leaf", fit="box", **KEEP_YAW)
DROP = {"Prop_Desktop", "Prop_Lamp"}

WARM = (1.0, 0.72, 0.45)
LIGHTS = {
    "Prop_Pendant": dict(at="bottom", dz=-0.1, power=5.0, reach=9.0, color=WARM),
    "Prop_LampPost": dict(at="top", dz=-0.35, power=8.0, reach=14.0, color=WARM),
    "Prop_Lantern": dict(at="bottom", dz=0.25, power=1.5, reach=6.0, color=(1.0, 0.65, 0.35)),
    "Prop_Lamp": dict(at="centre", dz=0.1, power=1.5, reach=5.0, color=WARM),
    "Prop_Stove": dict(at="bottom", dz=0.4, front=0.35, power=1.2, reach=6.0, color=(1.0, 0.5, 0.2)),
    "Prop_Boiler": dict(at="bottom", dz=0.6, front=0.6, power=6.0, reach=9.0, color=(1.0, 0.45, 0.15)),
}
SLOPED = {"Backdrop_Ridge"}
SNOW_ABOVE = 0.55
MOON_STRENGTH = 0.25
MOON_COLOR = (0.62, 0.72, 1.0)


def kind_of(name):
    area = next((a for a in AREAS if name.startswith(a + "_")), None)
    rest = name[len(area) + 1 :] if area else name
    rest = re.sub(r"\.\d+$", "", rest)
    rest = re.sub(r"_\d+$", "", rest)
    return area, rest


def base_material(name):
    return re.sub(r"\.\d+$", "", name)


def surface_for(area, kind, material):
    for o_area, pattern, gb, surface in OVERRIDES:
        if (o_area is None or o_area == area) and gb == material and re.search(pattern, kind):
            return surface
    return BY_MATERIAL.get(material, "stucco")


class Materials:
    def __init__(self):
        self.made = {}

    def tinted(self, tex, tint):
        if tint is None:
            return os.path.join(TEX_DIR, f"{tex}_color.jpg")
        key = "_".join(f"{c:.2f}" for c in tint)
        path = os.path.join(TEX_DIR, f"{tex}_tint_{key}_color.jpg")
        if not os.path.exists(path):
            image = bpy.data.images.load(os.path.join(TEX_DIR, f"{tex}_color.jpg"))
            pixels = np.array(image.pixels[:]).reshape(-1, 4)
            pixels[:, :3] = np.clip(pixels[:, :3] * np.array(tint), 0.0, 1.0)
            out = bpy.data.images.new(f"tint_{tex}_{key}", image.size[0], image.size[1])
            out.pixels = pixels.ravel()
            out.filepath_raw = path
            out.file_format = "JPEG"
            out.save()
            bpy.data.images.remove(out)
            bpy.data.images.remove(image)
        return path

    def get(self, name):
        if name in self.made:
            return self.made[name]
        spec = SURFACES[name]
        mat = bpy.data.materials.new(f"san_{name}")
        mat.use_nodes = True
        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        nodes.clear()
        out = nodes.new("ShaderNodeOutputMaterial")
        bsdf = nodes.new("ShaderNodeBsdfPrincipled")
        links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])
        bsdf.inputs["Metallic"].default_value = spec.get("metal", 0.0)

        def image(path, data):
            tex = nodes.new("ShaderNodeTexImage")
            tex.image = bpy.data.images.load(path, check_existing=True)
            if data:
                tex.image.colorspace_settings.name = "Non-Color"
            return tex

        if "flat" in spec:
            bsdf.inputs["Base Color"].default_value = (*spec["flat"], 1.0)
            bsdf.inputs["Roughness"].default_value = spec.get("rough", 0.6)
        else:
            tex = spec["tex"]
            links.new(image(self.tinted(tex, spec.get("tint")), False).outputs["Color"], bsdf.inputs["Base Color"])
            links.new(image(os.path.join(TEX_DIR, f"{tex}_rough.jpg"), True).outputs["Color"], bsdf.inputs["Roughness"])
            bump = nodes.new("ShaderNodeNormalMap")
            links.new(image(os.path.join(TEX_DIR, f"{tex}_normal.jpg"), True).outputs["Color"], bump.inputs["Color"])
            links.new(bump.outputs["Normal"], bsdf.inputs["Normal"])
        if "glow" in spec:
            bsdf.inputs["Emission Color"].default_value = (*spec["glow"], 1.0)
            bsdf.inputs["Emission Strength"].default_value = 1.0
        self.made[name] = mat
        return mat


def world_uvs(ob, tiles):
    me = ob.data
    while me.uv_layers:
        me.uv_layers.remove(me.uv_layers[0])
    layer = me.uv_layers.new(name="UVMap")
    mw = ob.matrix_world
    normal_matrix = mw.to_3x3().inverted().transposed()
    for poly in me.polygons:
        n = (normal_matrix @ poly.normal).normalized()
        tile = tiles[poly.material_index] if poly.material_index < len(tiles) else 2.0
        for li in poly.loop_indices:
            p = mw @ me.vertices[me.loops[li].vertex_index].co
            if abs(n.z) > 0.5:
                u, v = p.x, p.y
            elif abs(n.x) > abs(n.y):
                u, v = p.y, p.z
            else:
                u, v = p.x, p.z
            layer.data[li].uv = (u / tile, v / tile)


def dress_surface(ob, area, kind, materials):
    if ob.data.users > 1:
        ob.data = ob.data.copy()
    tiles = []
    for slot in ob.material_slots:
        gb = base_material(slot.material.name) if slot.material else "GB_Render"
        surface = surface_for(area, kind, gb)
        slot.material = materials.get(surface)
        tiles.append(SURFACES[surface].get("tile", 2.0))
    if kind in SLOPED:
        ob.data.materials.append(materials.get("snow"))
        tiles.append(SURFACES["snow"]["tile"])
        snow = len(ob.data.materials) - 1
        normal_matrix = ob.matrix_world.to_3x3().inverted().transposed()
        for poly in ob.data.polygons:
            if (normal_matrix @ poly.normal).normalized().z > SNOW_ABOVE:
                poly.material_index = snow
    world_uvs(ob, tiles)


def load_library(names):
    meshes = {}
    for name in sorted(names):
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=os.path.join(LIBRARY, f"{name}.glb"))
        new = [o for o in bpy.data.objects if o not in before]
        mesh_ob = next(o for o in new if o.type == "MESH")
        mesh_ob.data.transform(mesh_ob.matrix_world)
        meshes[name] = mesh_ob.data
        for o in new:
            bpy.data.objects.remove(o)
    return meshes


def mesh_bounds(me):
    co = np.empty(len(me.vertices) * 3)
    me.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    return Vector(co.min(axis=0)), Vector(co.max(axis=0))


def box_frame(ob):
    mw = ob.matrix_world
    yaw = math.atan2(mw[1][0], mw[0][0])
    corners = [mw @ Vector(c) for c in ob.bound_box]
    lo = Vector([min(c[i] for c in corners) for i in range(3)])
    hi = Vector([max(c[i] for c in corners) for i in range(3)])
    local_lo = Vector([min(c[i] for c in ob.bound_box) for i in range(3)])
    local_hi = Vector([max(c[i] for c in ob.bound_box) for i in range(3)])
    centre = mw @ ((local_lo + local_hi) / 2)
    return dict(yaw=yaw, dims=ob.dimensions.copy(), centre=centre, bottom=lo.z, top=hi.z)


def turns_for(spec, model_size, dims):
    if "turns" in spec:
        return list(spec["turns"])
    model_x = model_size.x >= model_size.y * 1.15
    model_y = model_size.y >= model_size.x * 1.15
    box_x = dims.x >= dims.y * 1.15
    box_y = dims.y >= dims.x * 1.15
    if (model_x and box_y) or (model_y and box_x):
        return [math.pi / 2, -math.pi / 2]
    if model_x or model_y or box_x or box_y:
        return [0.0, math.pi]
    return [0.0, math.pi, math.pi / 2, -math.pi / 2]


def choose_turn(turns, frame, tree):
    if len(turns) == 1 or tree is None:
        return turns[0]
    origin = frame["centre"].copy()
    origin.z = frame["bottom"] + min(frame["dims"].z * 0.5, 1.0)
    walls, exits = [], []
    for index, turn in enumerate(turns):
        angle = frame["yaw"] + turn
        back = Vector((-math.sin(angle), math.cos(angle), 0.0))
        half = frame["dims"].y / 2 if abs(math.sin(turn)) < 0.5 else frame["dims"].x / 2
        location, normal, _, distance = tree.ray_cast(origin, back, 4.0)
        if location is None:
            walls.append((4.0, index, turn))
        elif normal.dot(back) > 0.0:
            exits.append((-distance, index, turn))
        else:
            walls.append((distance - half, index, turn))
    if exits:
        return min(exits)[2]
    return min(walls)[2]


def scale_for(fit, model_size, dims, turn):
    sideways = abs(math.sin(turn)) > 0.5
    bx, by = (dims.y, dims.x) if sideways else (dims.x, dims.y)
    sx, sy, sz = bx / model_size.x, by / model_size.y, dims.z / model_size.z
    if fit == "box":
        return Vector((sx, sy, sz))
    if fit == "footprint":
        return Vector((sx, sy, (sx + sy) / 2))
    if fit == "height":
        return Vector((sz, sz, sz))
    return Vector((1.0, 1.0, 1.0))


def place(name, mesh, frame, spec, turn, collection, shift=0.0):
    lo, hi = mesh_bounds(mesh)
    size = hi - lo
    scale = scale_for(spec["fit"], size, frame["dims"], turn)
    angle = frame["yaw"] + turn
    rotation = Matrix.Rotation(angle, 4, "Z")
    along = Matrix.Rotation(frame["yaw"], 3, "Z") @ Vector((shift, 0.0, 0.0))
    ob = bpy.data.objects.new(name, mesh)
    collection.objects.link(ob)
    ob.matrix_world = Matrix.Translation(frame["centre"] + along) @ rotation @ Matrix.Diagonal((*scale, 1.0))
    corners = [ob.matrix_world @ Vector(c) for c in ob.bound_box]
    centre = sum(corners, Vector()) / 8
    target_z = frame["top"] if spec.get("top") else frame["bottom"]
    model_z = max(c.z for c in corners) if spec.get("top") else min(c.z for c in corners)
    offset = Vector((frame["centre"].x + along.x - centre.x, frame["centre"].y + along.y - centre.y, target_z - model_z))
    ob.matrix_world = Matrix.Translation(offset) @ ob.matrix_world
    return ob


def add_light(name, frame, spec, collection, turn=0.0):
    data = bpy.data.lights.new(name, "POINT")
    data.energy = spec["power"] * 4.0 * math.pi
    data.color = spec["color"]
    data.shadow_soft_size = 0.1
    data.use_custom_distance = True
    data.cutoff_distance = spec["reach"]
    ob = bpy.data.objects.new(name, data)
    collection.objects.link(ob)
    z = {"bottom": frame["bottom"], "top": frame["top"], "centre": frame["centre"].z}[spec["at"]] + spec["dz"]
    angle = frame["yaw"] + turn
    front = Vector((math.sin(angle), -math.cos(angle), 0.0))
    half = frame["dims"].y / 2 if abs(math.sin(turn)) < 0.5 else frame["dims"].x / 2
    reach = half + spec["front"] if "front" in spec else 0.0
    ob.location = (frame["centre"].x + front.x * reach, frame["centre"].y + front.y * reach, z)


def add_moon(collection):
    data = bpy.data.lights.new("Moon", "SUN")
    data.energy = MOON_STRENGTH
    data.color = MOON_COLOR
    ob = bpy.data.objects.new("Moon", data)
    collection.objects.link(ob)
    ob.rotation_euler = (math.radians(50.0), 0.0, math.radians(35.0))


def main(only=None):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    for area in AREAS:
        bpy.ops.import_scene.gltf(filepath=os.path.join(SANATORIUM, f"{area}.glb"))
    for ob in list(bpy.context.scene.objects):
        if ob.type != "MESH":
            bpy.data.objects.remove(ob)

    meshes = list(bpy.context.scene.objects)
    plan = []
    architecture = []
    for ob in meshes:
        area, kind = kind_of(ob.name)
        if kind in DROP:
            plan.append((ob, area, kind, None))
        elif kind in REPLACE or kind.startswith("Door_"):
            plan.append((ob, area, kind, REPLACE.get(kind, DOOR)))
        else:
            architecture.append((ob, area, kind))

    depsgraph = bpy.context.evaluated_depsgraph_get()
    verts, polys = [], []
    for ob, _, _ in architecture:
        base = len(verts)
        verts.extend(ob.matrix_world @ v.co for v in ob.data.vertices)
        polys.extend([base + i for i in p.vertices] for p in ob.data.polygons)
    tree = BVHTree.FromPolygons(verts, polys) if polys else None

    library = load_library({spec["model"] for _, _, _, spec in plan if spec})
    collection = bpy.context.scene.collection
    materials = Materials()
    placed, lights, missing = 0, 0, set()
    for ob, area, kind, spec in plan:
        frame = box_frame(ob)
        name = ob.name
        bpy.data.objects.remove(ob)
        mesh = library.get(spec["model"]) if spec else None
        turn = 0.0
        if mesh is not None:
            lo, hi = mesh_bounds(mesh)
            turn = choose_turn(turns_for(spec, hi - lo, frame["dims"]), frame, tree)
        if kind in LIGHTS:
            add_light(f"Light_{name}", frame, LIGHTS[kind], collection, turn)
            lights += 1
        if spec is None:
            continue
        if mesh is None:
            missing.add(spec["model"])
            continue
        if spec.get("pair"):
            quarter = frame["dims"].x / 4
            place(f"{name}_A", mesh, frame, spec, turn, collection, -quarter)
            place(f"{name}_B", mesh, frame, spec, turn, collection, quarter)
            placed += 2
        else:
            place(name, mesh, frame, spec, turn, collection)
            placed += 1

    for ob, area, kind in architecture:
        dress_surface(ob, area, kind, materials)
    add_moon(collection)

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.export_scene.gltf(
        filepath=OUT,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_lights=True,
        export_animations=False,
        export_cameras=False,
        export_tangents=True,
        export_image_format="JPEG",
        export_jpeg_quality=88,
    )
    return dict(placed=placed, lights=lights, surfaces=len(architecture), missing=sorted(missing))


if __name__ == "__main__":
    print("ART", main())
