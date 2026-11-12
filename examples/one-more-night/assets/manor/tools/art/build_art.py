import json
import math
import os
import sys

import bmesh
import bpy
from mathutils import Vector, geometry

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
import textures

TEX_DIR = os.path.join(ROOT, "textures")
LIB_DIR = os.path.join(ROOT, "library")
SPEC = os.path.join(ROOT, "design", "level", "level_spec.json")
ART = "Art"
FLOOR_Z = {"garden": -0.6, "shed": -0.6}
CEILING = {"hall": 7.2, "garage": 3.0, "shed": 2.4}
GLAZED = ("glazed", "glazed_double", "french_window")

MATERIALS = {
    "parquet": dict(color="parquet_color.jpg", normal="parquet_normal.jpg", rough="parquet_rough.jpg", tile=2.0),
    "boards": dict(color="boards_color.jpg", normal="boards_normal.jpg", rough="boards_rough.jpg", tile=2.0),
    "marble": dict(color="marble_checker_color.png", rough=0.16, tile=1.2, turn=45.0),
    "quarry": dict(color="quarry_color.jpg", normal="quarry_normal.jpg", rough="quarry_rough.jpg", tile=1.5),
    "flags": dict(color="flags_color.jpg", normal="flags_normal.jpg", rough="flags_rough.jpg", tile=2.0),
    "concrete": dict(color="concrete_color.jpg", normal="concrete_normal.jpg", rough="concrete_rough.jpg", tile=3.0),
    "gravel": dict(color="gravel_color.jpg", normal="gravel_normal.jpg", rough="gravel_rough.jpg", tile=2.0),
    "grass": dict(color="grass_color.jpg", normal="grass_normal.jpg", rough="grass_rough.jpg", tile=2.5),
    "brick": dict(color="brick_color.jpg", normal="brick_normal.jpg", rough="brick_rough.jpg", tile=1.6),
    "limewash": dict(color="limewash_color.jpg", normal="limewash_normal.jpg", rough="limewash_rough.jpg", tile=1.6),
    "tile_white": dict(color="tile_white_color.jpg", normal="tile_white_normal.jpg", rough=0.2, tile=1.0),
    "oak": dict(color="oak_dark_color.png", normal="oak_normal.jpg", rough="oak_rough.jpg", tile=1.6, upright=True),
    "paint_white": dict(color="paint_white_color.png", normal="plaster_normal.jpg", rough=0.45, tile=2.0),
    "paint_cream": dict(color="paint_cream_color.png", normal="plaster_normal.jpg", rough=0.4, tile=2.0),
    "paint_green": dict(color="paint_green_color.png", normal="plaster_normal.jpg", rough=0.35, tile=2.0),
    "plaster_cream": dict(color="plaster_cream_color.png", normal="plaster_normal.jpg", rough=0.8, tile=2.0),
    "ceiling": dict(color="ceiling_color.png", normal="plaster_normal.jpg", rough=0.9, tile=2.0),
    "plain": dict(flat=(0.55, 0.53, 0.50), rough=0.9),
    "glass": dict(flat=(0.012, 0.018, 0.035), rough=0.06, glow=(0.02, 0.035, 0.08), two_sided=True),
    "glazing": dict(flat=(0.03, 0.035, 0.045), rough=0.12, glow=(0.008, 0.014, 0.03)),
    "brass": dict(flat=(0.55, 0.42, 0.20), rough=0.35, metal=1.0),
}
for _paper in ("sage", "oxblood", "burgundy", "green", "rose", "blue", "yellow", "lilac", "peach"):
    MATERIALS[f"paper_{_paper}"] = dict(color=f"paper_{_paper}_color.png", rough=0.78, tile=0.42)


def public(paper, floor="boards", dado="oak", trim="oak", lamp="chandelier", hearth="fireplace_marble"):
    return dict(bands=[(0.9, dado), (None, paper)], floor=floor, trim=trim, skirting=0.2,
                panel_rows=[(0.28, 0.82)], panel_width=0.75, rail=True, cornice=True, curtains=True,
                lamp=lamp, hearth=hearth, leaf="oak", stair="oak")


def bedroom(paper):
    return public(paper, dado="paint_white", trim="paint_white", lamp="pendant_bowl", hearth="fireplace_plain")


def service(floor="quarry", walls=None):
    return dict(bands=walls or [(1.2, "paint_green"), (None, "paint_cream")], floor=floor, trim="paint_green",
                skirting=0.12, panel_rows=[], panel_width=1.0, rail=False, cornice=False, curtains=False,
                lamp="pendant_enamel", hearth="fireplace_plain", leaf="paint_green", stair="boards")


def outdoors(floor):
    return dict(bands=[(None, "brick")], floor=floor, trim="brick", skirting=0.0, panel_rows=[], panel_width=1.0,
                rail=False, cornice=False, curtains=False, lamp=None, hearth=None, leaf="oak", stair="flags",
                outdoor=True)


STYLE = {
    "hall": dict(public("plaster_cream", floor="marble"), bands=[(2.4, "oak"), (None, "plaster_cream")],
                 skirting=0.26, panel_rows=[(0.34, 1.02), (1.1, 2.3)], panel_width=0.7),
    "drawing": public("paper_sage", floor="parquet", dado="paint_white", trim="paint_white"),
    "dining": public("paper_oxblood"),
    "library": public("paper_green", floor="parquet"),
    "study": public("paper_burgundy"),
    "billiard": public("paper_green"),
    "corr_w": public("plaster_cream", floor="parquet", lamp="pendant_bowl"),
    "corr_e": public("plaster_cream", floor="parquet", lamp="pendant_bowl"),
    "conservatory": dict(public("paint_white", floor="quarry", dado="brick", trim="paint_white", lamp="pendant_bowl"),
                         panel_rows=[], rail=False, cornice=False, curtains=False),
    "gallery": public("plaster_cream", lamp="pendant_bowl"),
    "master": dict(bedroom("paper_rose"), lamp="chandelier"),
    "dressing": bedroom("paper_rose"),
    "g1": bedroom("paper_blue"),
    "g2": bedroom("paper_yellow"),
    "g3": bedroom("paper_lilac"),
    "g4": bedroom("paper_sage"),
    "g5": bedroom("paper_peach"),
    "bath": dict(bedroom("paint_white"), bands=[(1.5, "tile_white"), (None, "paint_white")], floor="marble",
                 panel_rows=[], curtains=False),
    "pantry": service(),
    "kitchen": service(),
    "servhall": service(),
    "passage": service(),
    "backstairs": service(floor="boards"),
    "housekeeper": service(floor="boards"),
    "lamproom": service(),
    "closet": service(floor="boards"),
    "attic_corr": service(floor="boards", walls=[(None, "paint_cream")]),
    "attic_maids": service(floor="boards", walls=[(None, "paint_cream")]),
    "attic_men": service(floor="boards", walls=[(None, "paint_cream")]),
    "attic_staff": service(floor="boards", walls=[(None, "paint_cream")]),
    "boxroom": service(floor="boards", walls=[(None, "paint_cream")]),
    "cellar": service(floor="flags", walls=[(None, "limewash")]),
    "cellar_pass": service(floor="flags", walls=[(None, "limewash")]),
    "garage": service(floor="concrete", walls=[(None, "limewash")]),
    "shed": service(floor="boards", walls=[(None, "limewash")]),
    "forecourt": outdoors("gravel"),
    "yard": outdoors("gravel"),
    "path": outdoors("gravel"),
    "terrace": outdoors("flags"),
    "garden": outdoors("grass"),
}
SERVICE = {rid for rid, style in STYLE.items() if style["lamp"] == "pendant_enamel"}
CLUB = ("library", "study", "billiard")

FURNITURE = {
    "sofa": "sofa", "sidetable": "sidetable", "chest": "chest", "console": "console", "clock": "clock",
    "gong": "gong", "phone_table": "sidetable", "desk": "desk", "writing_table": "writing_table",
    "dressing_table": "dressing_table", "cupboard": "cupboard", "wardrobe": "wardrobe", "sbed": "bed_iron",
    "trunk": "trunk", "crate": "crate", "planter": "planter", "tub": "bathtub", "billiard": "billiard_table",
    "car": "car", "counter": "counter", "bench": "bench", "gbench": "garden_bench",
}
FIT_TOP = {"sidetable", "chest", "console", "coffee_table", "bookcase", "clock", "gong", "table_dining",
           "table_kitchen", "table_library", "desk", "writing_table", "cupboard", "wardrobe", "dresser",
           "wine_rack", "storage_rack", "trunk", "crate", "counter", "billiard_table", "bathtub"}


def model_for(room, kind, size):
    if kind == "chair":
        if room in SERVICE:
            return "chair_kitchen"
        if room == "drawing" or STYLE.get(room, {}).get("lamp") == "pendant_bowl" and room not in ("corr_w", "corr_e", "gallery", "conservatory"):
            return "chair_salon"
        if room in ("hall", "corr_w", "corr_e", "gallery"):
            return "chair_hall"
        return "chair_dining"
    if kind == "armchair":
        return "armchair_leather" if room in CLUB else "armchair_chintz"
    if kind == "table":
        if size.z < 0.6:
            return "coffee_table"
        if room in SERVICE:
            return "table_kitchen"
        return "table_dining" if max(size.x, size.y) > 2.4 else "table_library"
    if kind == "bookcase":
        return "bookcase"
    if kind == "shelves":
        return "dresser" if room in SERVICE else "bookcase"
    if kind == "rack":
        return "wine_rack" if room == "cellar" else "storage_rack"
    if kind == "bed":
        return "four_poster" if room == "master" else "bed"
    return FURNITURE.get(kind)


DRESS = {
    "drawing": [
        dict(model="sconce", at=(0.2, 1.5, 1.85), face="E"),
        dict(model="sconce", at=(0.2, 4.6, 1.85), face="E"),
        dict(model="painting_landscape", at=(1.1, 3.0, 2.2), face="E"),
        dict(model="painting_landscape", at=(8.5, 7.9, 1.8), face="S", size=(1.0, 0.07, 0.75)),
    ],
    "hall": [
        dict(model="painting_landscape", at=(11.1, 1.8, 1.9), face="E", size=(1.3, 0.08, 0.95)),
        dict(model="painting_landscape", at=(18.9, 1.8, 1.9), face="W", size=(1.3, 0.08, 0.95)),
        dict(model="sconce", at=(11.1, 12.0, 2.0), face="E"),
        dict(model="sconce", at=(11.1, 14.6, 2.0), face="E"),
    ],
}
STRETCH = {"table_dining", "table_kitchen", "table_library", "counter", "bench", "billiard_table", "car", "bed",
           "four_poster", "bed_iron", "desk", "dresser", "wine_rack", "storage_rack", "bookcase", "garden_bench"}
FACES = {"N": Vector((0, 1)), "S": Vector((0, -1)), "E": Vector((1, 0)), "W": Vector((-1, 0))}

SKIRTING = [(0.0, 0.0), (0.022, 0.0), (0.022, 0.82), (0.012, 0.9), (0.012, 0.97), (0.0, 1.0)]
RAIL = [(0.0, -0.035), (0.02, -0.03), (0.034, -0.01), (0.034, 0.012), (0.018, 0.03), (0.0, 0.035)]
CORNICE = [(0.0, -0.16), (0.02, -0.16), (0.03, -0.12), (0.07, -0.085), (0.1, -0.035), (0.14, -0.02), (0.14, 0.0)]
BALUSTER = [(0.0, 0.03), (0.07, 0.03), (0.1, 0.02), (0.16, 0.03), (0.3, 0.034), (0.44, 0.022), (0.72, 0.016),
            (0.84, 0.028), (0.9, 0.02), (0.96, 0.024)]


def out(points, ref):
    normal = geometry.normal(points)
    centre = sum(points, Vector()) / len(points)
    if normal.dot(centre - ref) < 0:
        return list(reversed(points))
    return list(points)


class Room:
    def __init__(self, rid, key, rects, z0, z1):
        self.id, self.key = rid, key
        self.rects = rects
        self.z0, self.z1 = z0, z1
        self.style = STYLE[rid]

    def rect_at(self, p):
        return max(self.rects, key=lambda r: min(p.x - r[0], r[2] - p.x, p.y - r[1], r[3] - p.y))

    def margin(self, p):
        x0, y0, x1, y1 = self.rect_at(p)
        return min(p.x - x0, x1 - p.x, p.y - y0, y1 - p.y)

    def holds(self, p):
        return self.z0 - 0.3 < p.z < self.z1 + 0.3 and self.margin(p) > -0.01


class Run:
    def __init__(self, room, normal, plane, lo, hi, z0, z1):
        self.room, self.normal, self.plane = room, normal, plane
        self.lo, self.hi, self.z0, self.z1 = lo, hi, z0, z1

    def point(self, along, z, out=0.0):
        if abs(self.normal.x) > 0.5:
            return Vector((self.plane + self.normal.x * out, along, z))
        return Vector((along, self.plane + self.normal.y * out, z))


class Opening:
    def __init__(self, room, normal, line, lo, hi, z0, z1):
        self.room, self.normal, self.line = room, normal, line
        self.lo, self.hi, self.z0, self.z1 = lo, hi, z0, z1


class Build:
    def __init__(self, rooms):
        spec = json.load(open(SPEC))
        metrics = spec["metrics"]
        self.rooms = {}
        found = []
        for r in spec["rooms"]:
            if r["id"] not in STYLE or (rooms and r["id"] not in rooms):
                continue
            levels = {}
            for b in r["boxes"]:
                levels.setdefault(b.get("level", r["level"]), []).append((b["x0"], b["y0"], b["x1"], b["y1"]))
            for level, rects in levels.items():
                z0 = FLOOR_Z.get(r["id"], metrics["level_z"][level])
                height = CEILING.get(r["id"], metrics["ceiling"][level])
                key = r["id"] if len(levels) == 1 else f"{r['id']}_{level}"
                found.append(Room(r["id"], key, rects, z0, z0 + (12.0 if STYLE[r["id"]].get("outdoor") else height)))
        found.sort(key=lambda room: bool(room.style.get("outdoor")))
        for room in found:
            self.rooms[room.key] = room
        self.lamps = json.load(open(os.path.join(HERE, "lamps.json")))
        self.runs = []
        self.sills = []
        self.lintels = []
        self.solids = []
        self.ceilings = []
        self.materials = {}
        self.made = []
        coll = bpy.data.collections.get(ART)
        if coll is None:
            coll = bpy.data.collections.new(ART)
            bpy.context.scene.collection.children.link(coll)
        self.coll = coll

    def room_at(self, p):
        held = [room for room in self.rooms.values() if room.holds(p)]
        if not held:
            return None
        indoor = [room for room in held if not room.style.get("outdoor")]
        return max(indoor or held, key=lambda room: room.z0)

    def material(self, name):
        if name in self.materials:
            return self.materials[name]
        spec = MATERIALS[name]
        mat = bpy.data.materials.get(f"art_{name}") or bpy.data.materials.new(f"art_{name}")
        mat.use_nodes = True
        nodes, links = mat.node_tree.nodes, mat.node_tree.links
        nodes.clear()
        out = nodes.new("ShaderNodeOutputMaterial")
        bsdf = nodes.new("ShaderNodeBsdfPrincipled")
        links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])
        bsdf.inputs["Metallic"].default_value = spec.get("metal", 0.0)

        def image(file, data):
            tex = nodes.new("ShaderNodeTexImage")
            tex.image = bpy.data.images.load(os.path.join(TEX_DIR, file), check_existing=True)
            if data:
                tex.image.colorspace_settings.name = "Non-Color"
            return tex

        if "flat" in spec:
            bsdf.inputs["Base Color"].default_value = (*spec["flat"], 1.0)
        else:
            links.new(image(spec["color"], False).outputs["Color"], bsdf.inputs["Base Color"])
        rough = spec.get("rough", 0.6)
        if isinstance(rough, str):
            links.new(image(rough, True).outputs["Color"], bsdf.inputs["Roughness"])
        else:
            bsdf.inputs["Roughness"].default_value = rough
        if "normal" in spec:
            bump = nodes.new("ShaderNodeNormalMap")
            links.new(image(spec["normal"], True).outputs["Color"], bump.inputs["Color"])
            links.new(bump.outputs["Normal"], bsdf.inputs["Normal"])
        if "glow" in spec:
            bsdf.inputs["Emission Color"].default_value = (*spec["glow"], 1.0)
            bsdf.inputs["Emission Strength"].default_value = 1.0
        mat.use_backface_culling = not spec.get("two_sided", False)
        self.materials[name] = mat
        return mat

    def uv(self, name, p, n):
        spec = MATERIALS[name]
        tile = spec.get("tile", 1.0)
        if abs(n.z) > 0.5:
            u, v = p.x, p.y
            turn = math.radians(spec.get("turn", 0.0))
            if turn:
                u, v = u * math.cos(turn) - v * math.sin(turn), u * math.sin(turn) + v * math.cos(turn)
        elif abs(n.x) > abs(n.y):
            u, v = p.y, p.z
        else:
            u, v = p.x, p.z
        if spec.get("upright") and abs(n.z) <= 0.5:
            u, v = v, u
        return (u / tile, v / tile)

    def emit(self, name, parts, props=None):
        bm = bmesh.new()
        layer = bm.loops.layers.uv.new("UVMap")
        names = []
        for mat, quads in parts.items():
            if mat not in names:
                names.append(mat)
            for quad in quads:
                verts = [bm.verts.new(p) for p in quad]
                try:
                    face = bm.faces.new(verts)
                except ValueError:
                    continue
                face.material_index = names.index(mat)
        bm.normal_update()
        for face in bm.faces:
            mat = names[face.material_index]
            for loop in face.loops:
                loop[layer].uv = self.uv(mat, loop.vert.co, face.normal)
        return self.finish(name, bm, names, props)

    def finish(self, name, bm, names, props=None):
        me = bpy.data.meshes.new(name)
        bm.to_mesh(me)
        bm.free()
        for mat in names:
            me.materials.append(self.material(mat))
        ob = bpy.data.objects.new(name, me)
        ob["generated_art"] = True
        for key, value in (props or {}).items():
            ob[key] = value
        self.coll.objects.link(ob)
        self.made.append(name)
        return ob

    def planes(self):
        zs = set()
        for room in self.rooms.values():
            if room.style.get("outdoor"):
                continue
            for height, _ in room.style["bands"]:
                if height is not None:
                    zs.add(round(room.z0 + height, 3))
            zs.add(round(room.z1, 3))
            zs.add(round(room.z0, 3))
        for _, _, _, _, z in self.ceilings:
            zs.add(round(z, 3))
        return sorted(zs)

    def classify(self, face, fallback):
        c = face.calc_center_median()
        n = face.normal
        room = self.room_at(c + n * 0.06)
        if room is None:
            return (fallback if abs(n.z) < 0.5 else "plain"), None
        style = room.style
        on_line = room.margin(c) < 0.05
        if abs(n.z) > 0.5:
            if style.get("outdoor"):
                return (style["floor"] if n.z > 0 else "plain"), None
            if on_line:
                return style["trim"], ("sill" if n.z > 0 else "lintel", room)
            if n.z > 0:
                return (style["floor"] if abs(c.z - room.z0) < 0.05 else "plain"), None
            return "ceiling", None
        if style.get("outdoor"):
            return "brick", None
        if on_line:
            return style["trim"], None
        rise = c.z - room.z0
        for index, (height, mat) in enumerate(style["bands"]):
            if height is None or rise < height:
                return mat, ("wall", room, index)
        return "plain", None

    def shell(self, ob):
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        bm.transform(ob.matrix_world)
        for z in self.planes():
            bmesh.ops.bisect_plane(bm, geom=bm.verts[:] + bm.edges[:] + bm.faces[:],
                                   plane_co=(0, 0, z), plane_no=(0, 0, 1))
        bm.normal_update()
        layer = bm.loops.layers.uv.verify()
        names = []
        used = False
        for face in bm.faces:
            mat, info = self.classify(face, "brick" if ob.get("kind") == "exterior" else "plain")
            if mat not in names:
                names.append(mat)
            face.material_index = names.index(mat)
            for loop in face.loops:
                loop[layer].uv = self.uv(mat, loop.vert.co, face.normal)
            if mat != "plain":
                used = True
            if info is None:
                continue
            xs = [v.co.x for v in face.verts]
            ys = [v.co.y for v in face.verts]
            zs = [v.co.z for v in face.verts]
            if info[0] == "wall":
                n = Vector((round(face.normal.x), round(face.normal.y)))
                if abs(n.x) > 0.5:
                    self.runs.append(Run(info[1], n, xs[0], min(ys), max(ys), min(zs), max(zs)))
                else:
                    self.runs.append(Run(info[1], n, ys[0], min(xs), max(xs), min(zs), max(zs)))
                self.runs[-1].band = info[2]
            else:
                rec = (info[1], min(xs), max(xs), min(ys), max(ys), zs[0])
                (self.sills if info[0] == "sill" else self.lintels).append(rec)
        if not used:
            bm.free()
            return None
        return self.finish(f"art.{ob.name}", bm, names)

    def merged(self, runs):
        groups = {}
        for run in runs:
            key = (run.room.key, run.normal.x, run.normal.y, round(run.plane, 3), round(run.z0, 3), round(run.z1, 3),
                   run.band)
            groups.setdefault(key, []).append(run)
        out = []
        for group in groups.values():
            group.sort(key=lambda r: r.lo)
            cur = group[0]
            cur = Run(cur.room, cur.normal, cur.plane, cur.lo, cur.hi, cur.z0, cur.z1)
            cur.band = group[0].band
            for run in group[1:]:
                if run.lo <= cur.hi + 1e-3:
                    cur.hi = max(cur.hi, run.hi)
                else:
                    out.append(cur)
                    cur = Run(run.room, run.normal, run.plane, run.lo, run.hi, run.z0, run.z1)
                    cur.band = run.band
            out.append(cur)
        return out

    def free(self, room, p):
        if room.margin(p) < 0.21:
            return False
        return not any(x0 < p.x < x1 and y0 < p.y < y1 and z0 < p.z < z1 for x0, y0, x1, y1, z0, z1 in self.solids)

    def sweep(self, quads, run, z, profile, scale=1.0):
        reach = max(reach_out for reach_out, _ in profile)
        lo, hi = run.lo, run.hi
        if self.free(run.room, run.point(lo - 0.03, z, 0.03)):
            lo -= reach
        if self.free(run.room, run.point(hi + 0.03, z, 0.03)):
            hi += reach
        ups = [up * scale for _, up in profile]
        ref = run.point((lo + hi) / 2, z + (min(ups) + max(ups)) / 2, 0.0)
        for (o0, u0), (o1, u1) in zip(profile, profile[1:]):
            quads.append(out([run.point(lo, z + u0 * scale, o0), run.point(hi, z + u0 * scale, o0),
                              run.point(hi, z + u1 * scale, o1), run.point(lo, z + u1 * scale, o1)], ref))
        for along in (lo, hi):
            quads.append(out([run.point(along, z + up * scale, reach_out) for reach_out, up in profile], ref))

    def panel(self, quads, run, lo, hi, z0, z1):
        inset, raise_, bevel = 0.05, 0.014, 0.03
        a = [run.point(lo + inset, z0 + inset, 0.001), run.point(hi - inset, z0 + inset, 0.001),
             run.point(hi - inset, z1 - inset, 0.001), run.point(lo + inset, z1 - inset, 0.001)]
        b = [run.point(lo + inset + bevel, z0 + inset + bevel, raise_),
             run.point(hi - inset - bevel, z0 + inset + bevel, raise_),
             run.point(hi - inset - bevel, z1 - inset - bevel, raise_),
             run.point(lo + inset + bevel, z1 - inset - bevel, raise_)]
        ref = run.point((lo + hi) / 2, (z0 + z1) / 2, -0.05)
        for i in range(4):
            j = (i + 1) % 4
            quads.append(out([a[i], a[j], b[j], b[i]], ref))
        quads.append(out(b, ref))

    def trim(self, room):
        style = room.style
        mat = style["trim"]
        quads = []
        runs = self.merged([r for r in self.runs if r.room is room])
        tops = {round(room.z0 + h, 3) for h, _ in style["bands"] if h is not None}
        for run in runs:
            if run.hi - run.lo < 0.02:
                continue
            if abs(run.z0 - room.z0) < 0.01 and run.band == 0 and style["skirting"]:
                self.sweep(quads, run, run.z0, SKIRTING, style["skirting"])
            if style["rail"] and run.band == 0 and any(abs(run.z1 - top) < 0.01 for top in tops):
                self.sweep(quads, run, run.z1, RAIL)
            mid = run.point((run.lo + run.hi) / 2, run.z1 - 0.05, 0.1)
            if style["cornice"] and any(x0 < mid.x < x1 and y0 < mid.y < y1 and abs(z - run.z1) < 0.01
                                        for x0, y0, x1, y1, z in self.ceilings):
                self.sweep(quads, run, run.z1, CORNICE)
            if run.band != 0:
                continue
            width = run.hi - run.lo
            if width < 0.3:
                continue
            count = max(1, round(width / style["panel_width"]))
            for z0, z1 in style["panel_rows"]:
                if room.z0 + z0 < run.z0 - 0.01 or room.z0 + z1 > run.z1 + 0.01:
                    continue
                for i in range(count):
                    self.panel(quads, run, run.lo + width * i / count, run.lo + width * (i + 1) / count,
                               room.z0 + z0, room.z0 + z1)
        if quads:
            self.emit(f"art.{room.key}.trim_01", {mat: quads})

    def box(self, quads, lo, hi):
        x0, y0, z0 = lo
        x1, y1, z1 = hi
        v = [Vector(p) for p in ((x0, y0, z0), (x1, y0, z0), (x1, y1, z0), (x0, y1, z0),
                                 (x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1))]
        ref = (Vector(lo) + Vector(hi)) / 2
        for a, b, c, d in ((0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)):
            quads.append(out([v[a], v[b], v[c], v[d]], ref))

    def frame(self, quads, axis_x, line, lo, hi, z0, z1, side, half, width=0.14, depth=0.028):
        face = line + side * half
        near, far = sorted((face, face + side * depth))
        for a0, a1, b0, b1 in ((lo - width, lo, z0, z1 + width), (hi, hi + width, z0, z1 + width),
                               (lo, hi, z1, z1 + width)):
            if axis_x:
                self.box(quads, (a0, near, b0), (a1, far, b1))
            else:
                self.box(quads, (near, a0, b0), (far, a1, b1))

    def doors(self, room, half_of):
        quads = []
        for ob in bpy.data.objects:
            if ob.get("gb_kind") != "door" or room.id not in (ob.get("a"), ob.get("b")):
                continue
            if abs(ob.location.z - room.z0) > 0.1:
                continue
            x, y, z = ob.location
            w, h = ob["width"], ob["height"]
            axis_x = abs(math.sin(ob.rotation_euler.z)) < 0.5
            if axis_x:
                side = 1.0 if room.margin(Vector((x, y + 0.4, z + 1.0))) > 0 else -1.0
                self.frame(quads, True, y, x - w / 2, x + w / 2, room.z0, room.z0 + h, side, half_of(x, y, z))
            else:
                side = 1.0 if room.margin(Vector((x + 0.4, y, z + 1.0))) > 0 else -1.0
                self.frame(quads, False, x, y - w / 2, y + w / 2, room.z0, room.z0 + h, side, half_of(x, y, z))
        if quads:
            self.emit(f"art.{room.key}.architraves_01", {room.style["trim"]: quads})

    def openings(self, room):
        out = []
        for sill in self.sills:
            if sill[0] is not room or sill[5] < room.z0 + 0.2:
                continue
            _, x0, x1, y0, y1, z0 = sill
            tops = [l[5] for l in self.lintels
                    if l[0] is room and abs(l[1] - x0) < 0.01 and abs(l[2] - x1) < 0.01
                    and abs(l[3] - y0) < 0.01 and abs(l[4] - y1) < 0.01 and l[5] > z0]
            if not tops:
                continue
            if x1 - x0 > y1 - y0:
                line = (y0 + y1) / 2
                side = 1.0 if room.margin(Vector(((x0 + x1) / 2, line + 0.5, z0))) > 0 else -1.0
                out.append(Opening(room, Vector((0, side)), line, x0, x1, z0, min(tops)))
            else:
                line = (x0 + x1) / 2
                side = 1.0 if room.margin(Vector((line + 0.5, (y0 + y1) / 2, z0))) > 0 else -1.0
                out.append(Opening(room, Vector((side, 0)), line, y0, y1, z0, min(tops)))
            out[-1].half = (y1 - y0) / 2 if x1 - x0 > y1 - y0 else (x1 - x0) / 2
        return out

    def windows(self, room):
        glass, bars = [], []
        found = self.openings(room)
        for index, o in enumerate(found):
            axis_x = abs(o.normal.y) > 0.5

            def at(along, z, off):
                return Vector((along, o.line + off, z)) if axis_x else Vector((o.line + off, along, z))

            glass.append([at(o.lo, o.z0, 0), at(o.hi, o.z0, 0), at(o.hi, o.z1, 0), at(o.lo, o.z1, 0)])
            t = 0.022
            cuts = [(o.lo, o.lo + 0.05, o.z0, o.z1), (o.hi - 0.05, o.hi, o.z0, o.z1),
                    (o.lo, o.hi, o.z0, o.z0 + 0.05), (o.lo, o.hi, o.z1 - 0.05, o.z1)]
            mid = (o.z0 + o.z1) / 2
            cuts.append((o.lo, o.hi, mid - 0.03, mid + 0.03))
            columns = max(2, round((o.hi - o.lo) / 0.45))
            for i in range(1, columns):
                along = o.lo + (o.hi - o.lo) * i / columns
                cuts.append((along - t / 2, along + t / 2, o.z0, o.z1))
            rows = max(2, round((o.z1 - o.z0) / 0.55))
            for i in range(1, rows):
                z = o.z0 + (o.z1 - o.z0) * i / rows
                cuts.append((o.lo, o.hi, z - t / 2, z + t / 2))
            for a0, a1, z0, z1 in cuts:
                if axis_x:
                    self.box(bars, (a0, o.line - 0.03, z0), (a1, o.line + 0.03, z1))
                else:
                    self.box(bars, (o.line - 0.03, a0, z0), (o.line + 0.03, a1, z1))
            if o.z0 - room.z0 > 2.0 or not room.style["curtains"]:
                continue
            self.place("curtains", f"art.{room.key}.curtains_{index + 1:02d}",
                       at((o.lo + o.hi) / 2, room.z0, 0) + Vector((o.normal.x, o.normal.y, 0)) * (o.half + 0.01),
                       o.normal, size=(o.hi - o.lo + 0.8, 0.26, o.z1 + 0.32 - room.z0))
        if glass:
            self.emit(f"art.{room.key}.windows_01", {"glass": glass, "paint_white": bars})

    def library(self, model):
        me = bpy.data.meshes.get(f"lib.{model}")
        if me is not None:
            return me
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=os.path.join(LIB_DIR, f"{model}.glb"))
        for ob in set(bpy.data.objects) - before:
            if ob.type == "MESH":
                me = ob.data
            bpy.data.objects.remove(ob)
        me.name = f"lib.{model}"
        me.use_fake_user = True
        return me

    def place(self, model, name, at, facing, size=None, scale=None):
        me = self.library(model)
        ob = bpy.data.objects.new(name, me)
        ob["generated_art"] = True
        ob.location = at
        ob.rotation_euler = (0, 0, math.atan2(facing.x, -facing.y))
        if size is not None:
            lo = Vector((min(v.co[i] for v in me.vertices) for i in range(3)))
            hi = Vector((max(v.co[i] for v in me.vertices) for i in range(3)))
            scale = [size[i] / (hi[i] - lo[i]) for i in range(3)]
        if scale is not None:
            ob.scale = scale
        self.coll.objects.link(ob)
        self.made.append(name)
        return ob

    def facing(self, room, lo, hi, kind, tables):
        centre = Vector(((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, (lo.z + hi.z) / 2))
        x0, y0, x1, y1 = room.rect_at(centre)
        gaps = {"W": lo.x - x0, "E": x1 - hi.x, "S": lo.y - y0, "N": y1 - hi.y}
        for sx0, sy0, sx1, sy1, sz0, sz1 in self.solids:
            if not sz0 < centre.z < sz1:
                continue
            if lo.y < sy1 and hi.y > sy0:
                if 0 <= lo.x - sx1 < gaps["W"]:
                    gaps["W"] = lo.x - sx1
                if 0 <= sx0 - hi.x < gaps["E"]:
                    gaps["E"] = sx0 - hi.x
            if lo.x < sx1 and hi.x > sx0:
                if 0 <= lo.y - sy1 < gaps["S"]:
                    gaps["S"] = lo.y - sy1
                if 0 <= sy0 - hi.y < gaps["N"]:
                    gaps["N"] = sy0 - hi.y
        if hi.x - lo.x > hi.y - lo.y + 0.05:
            order = ("S", "N", "W", "E")
        elif hi.y - lo.y > hi.x - lo.x + 0.05:
            order = ("W", "E", "S", "N")
        else:
            order = ("S", "N", "W", "E")
        side = min(order, key=lambda k: (gaps[k] >= 0.45, gaps[k] if gaps[k] < 0.45 else 0, order.index(k)))
        if gaps[side] < 0.45 and not room.style.get("outdoor"):
            return -FACES[side], True
        flat = Vector((centre.x, centre.y))
        if kind in ("chair", "armchair", "sofa", "bench", "gbench") and tables:
            near = min(tables, key=lambda t: (t - flat).length)
            if (near - flat).length < 1.7:
                d = near - flat
                if abs(d.x) > abs(d.y):
                    return Vector((math.copysign(1, d.x), 0)), False
                return Vector((0, math.copysign(1, d.y))), False
        if hi.x - lo.x >= hi.y - lo.y:
            return Vector((0, -1)), False
        return Vector((1, 0)), False

    def furniture(self, room):
        boxes = []
        for ob in bpy.data.objects:
            if ob.get("room") != room.id or ob.get("gb_kind") not in ("furniture", "dressing"):
                continue
            if ob.name.startswith("art.") or ob.type != "MESH":
                continue
            cs = [ob.matrix_world @ Vector(c) for c in ob.bound_box]
            lo = Vector((min(c[i] for c in cs) for i in range(3)))
            hi = Vector((max(c[i] for c in cs) for i in range(3)))
            if not room.z0 - 0.3 < lo.z < room.z1:
                continue
            kind = ob.get("kind") or ob.name.rsplit(".", 1)[-1]
            if ob.name.startswith("furniture.bed."):
                kind = "sbed" if room.id in SERVICE else "bed"
            boxes.append((ob, kind, lo, hi))
        tables = [Vector(((lo.x + hi.x) / 2, (lo.y + hi.y) / 2)) for _, kind, lo, hi in boxes
                  if kind in ("sidetable", "table", "billiard")]
        for ob, kind, lo, hi in boxes:
            centre = Vector(((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, lo.z))
            if kind == "rug":
                long_x = hi.x - lo.x >= hi.y - lo.y
                span = sorted((hi.x - lo.x, hi.y - lo.y))
                self.place("rug_turkey", f"art.{ob.name}", centre + Vector((0, 0, 0.004)),
                           Vector((1, 0)) if long_x else Vector((0, -1)), size=(span[0], span[1], 0.012))
                continue
            model = model_for(room.id, kind, hi - lo)
            if model is None or not os.path.exists(os.path.join(LIB_DIR, f"{model}.glb")):
                self.missing.add(f"{room.id}:{kind}")
                continue
            face, against = self.facing(room, lo, hi, kind, tables)
            me = self.library(model)
            mlo = Vector((min(v.co[i] for v in me.vertices) for i in range(3)))
            mhi = Vector((max(v.co[i] for v in me.vertices) for i in range(3)))
            msize = mhi - mlo
            across = hi.x - lo.x if abs(face.y) > 0.5 else hi.y - lo.y
            deep = hi.y - lo.y if abs(face.y) > 0.5 else hi.x - lo.x
            limit = (0.6, 1.6) if model in STRETCH else (0.8, 1.25)
            sx = min(max(across / msize.x, limit[0]), limit[1])
            sy = min(max(deep / msize.y, limit[0]), limit[1])
            sz = (hi.z - lo.z) / msize.z if model in FIT_TOP else (sx + sy) / 2
            at = centre.copy()
            gap = {(0, -1): hi.y, (0, 1): lo.y, (1, 0): lo.x, (-1, 0): hi.x}[(round(face.x), round(face.y))]
            back = msize.y * sy / 2
            if against and abs(face.y) > 0.5:
                at.y = gap + face.y * back
            elif against:
                at.x = gap + face.x * back
            self.place(model, f"art.{ob.name}", at, face, scale=(sx, sy, sz))

    def fixtures(self, room):
        rods = []
        model = room.style["lamp"]
        sizes = {"chandelier": (0.85, 0.85, 0.95), "pendant_bowl": (0.42, 0.42, 0.7),
                 "pendant_enamel": (0.36, 0.36, 0.6)}
        index = 0
        for x, y, z in self.lamps.get(room.id, []):
            if model is None or not room.z0 - 0.1 < z < room.z1:
                continue
            if not os.path.exists(os.path.join(LIB_DIR, f"{model}.glb")):
                self.missing.add(f"{room.id}:{model}")
                continue
            if any(x0 < x < x1 and y0 < y < y1 and z0 < z + 1.5 and z1 > z - 2.0 for x0, y0, x1, y1, z0, z1 in self.stairs):
                continue
            ceilings = [c for x0, y0, x1, y1, c in self.ceilings if x0 < x < x1 and y0 < y < y1 and c > z]
            ceiling = min(ceilings) if ceilings else room.z1
            size = sizes[model]
            top = min(ceiling, z + size[2] + 0.25)
            index += 1
            self.place(model, f"art.{room.key}.{model}_{index:02d}", Vector((x, y, top)), Vector((0, -1)), size=size)
            if ceiling - top > 0.05:
                self.box(rods, (x - 0.012, y - 0.012, top - 0.02), (x + 0.012, y + 0.012, ceiling))
                self.box(rods, (x - 0.09, y - 0.09, ceiling - 0.04), (x + 0.09, y + 0.09, ceiling))
        if rods:
            self.emit(f"art.{room.key}.rods_01", {"brass": rods})
        counts = {}
        for item in DRESS.get(room.key, []):
            counts[item["model"]] = counts.get(item["model"], 0) + 1
            self.place(item["model"], f"art.{room.key}.{item['model']}_{counts[item['model']]:02d}",
                       Vector(item["at"]), FACES[item["face"]], size=item.get("size"))

    def fireplace(self, room):
        breast = bpy.data.objects.get(f"fireplace.{room.id}")
        model = room.style["hearth"]
        if breast is None or bpy.data.objects.get(f"firebox.{room.id}") is None or model is None:
            return
        if not os.path.exists(os.path.join(LIB_DIR, f"{model}.glb")):
            self.missing.add(f"{room.id}:{model}")
            return
        cs = [breast.matrix_world @ Vector(c) for c in breast.bound_box]
        lo = Vector((min(c[i] for c in cs) for i in range(3)))
        hi = Vector((max(c[i] for c in cs) for i in range(3)))
        if not room.z0 - 0.3 < lo.z < room.z1:
            return
        x0, y0, x1, y1 = room.rect_at((lo + hi) / 2)
        gaps = {"W": lo.x - x0, "E": x1 - hi.x, "S": lo.y - y0, "N": y1 - hi.y}
        face = -FACES[min(gaps, key=gaps.get)]
        at = Vector(((lo.x + hi.x) / 2, (lo.y + hi.y) / 2, room.z0))
        if abs(face.x) > 0.5:
            at.x = hi.x if face.x > 0 else lo.x
            width = hi.y - lo.y
        else:
            at.y = hi.y if face.y > 0 else lo.y
            width = hi.x - lo.x
        height = 1.45 if model == "fireplace_marble" else 1.25
        self.place(model, f"art.firebox.{room.id}", at, face, size=(min(width, 1.8), 0.42, height))

    def balustrade(self, ob):
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        bm.transform(ob.matrix_world)
        bm.normal_update()
        seen = set()
        quads = []
        for start in bm.faces:
            if start in seen:
                continue
            island, stack = [], [start]
            seen.add(start)
            while stack:
                face = stack.pop()
                island.append(face)
                for edge in face.edges:
                    for other in edge.link_faces:
                        if other not in seen:
                            seen.add(other)
                            stack.append(other)
            big = max(island, key=lambda f: f.calc_area())
            n = Vector((big.normal.x, big.normal.y, 0)).normalized()
            along = Vector((-n.y, n.x, 0))
            verts = {v for f in island for v in f.verts}
            mid = sum((v.co for v in verts), Vector()) / len(verts)
            ends = sorted(big.verts, key=lambda v: v.co.dot(along))
            a_lo, a_hi = sorted(ends[:2], key=lambda v: v.co.z)
            b_lo, b_hi = sorted(ends[2:], key=lambda v: v.co.z)
            centre = mid.dot(n)

            def on_axis(p):
                return p - n * (p.dot(n) - centre)

            p0, p1 = on_axis(a_lo.co), on_axis(b_lo.co)
            height = a_hi.co.z - a_lo.co.z
            length = (Vector((p1.x, p1.y, 0)) - Vector((p0.x, p0.y, 0))).length
            if length < 0.2:
                continue
            up = Vector((0, 0, 1))
            for z0, z1, half in ((height - 0.07, height, 0.04), (0.03, 0.08, 0.03)):
                a, b = p0 + up * z0, p1 + up * z0
                c, d = p0 + up * z1, p1 + up * z1
                s = n * half
                ref = (a + b + c + d) / 4
                for quad in ([a - s, b - s, d - s, c - s], [b + s, a + s, c + s, d + s],
                             [c - s, d - s, d + s, c + s], [a + s, b + s, b - s, a - s],
                             [a - s, c - s, c + s, a + s], [b + s, d + s, d - s, b - s]):
                    quads.append(out(quad, ref))
            count = max(1, round(length / 0.13))
            for i in range(count):
                base = p0.lerp(p1, (i + 0.5) / count) + up * 0.08
                span = height - 0.15
                rings = []
                for t, r in BALUSTER:
                    rings.append([base + up * (t * span) + (along * math.cos(k * math.pi / 3)
                                                             + n * math.sin(k * math.pi / 3)) * r
                                  for k in range(6)])
                for lower, upper in zip(rings, rings[1:]):
                    for k in range(6):
                        j = (k + 1) % 6
                        quad = [lower[k], lower[j], upper[j], upper[k]]
                        height_at = sum(q.z for q in quad) / 4
                        quads.append(out(quad, Vector((base.x, base.y, height_at))))
            for p in (p0, p1):
                s, t = n * 0.055, along * 0.055
                lo_c = [p - s - t, p + s - t, p + s + t, p - s + t]
                hi_c = [c + up * (height + 0.12) for c in lo_c]
                ref = p + up * (height / 2)
                for k in range(4):
                    j = (k + 1) % 4
                    quads.append(out([lo_c[k], lo_c[j], hi_c[j], hi_c[k]], ref))
                quads.append(out(hi_c, ref))
        bm.free()
        if quads:
            self.emit(f"art.{ob.name}", {"oak": quads})

    def stair(self, ob, mat):
        bm = bmesh.new()
        bm.from_mesh(ob.data)
        bm.transform(ob.matrix_world)
        bm.normal_update()
        layer = bm.loops.layers.uv.verify()
        for face in bm.faces:
            for loop in face.loops:
                loop[layer].uv = self.uv(mat, loop.vert.co, face.normal)
        self.finish(f"art.{ob.name}", bm, [mat])

    def glazed(self, ob):
        if "art_leaf_lo" not in ob:
            cs = [Vector(c) for c in ob.bound_box]
            ob["art_leaf_lo"] = [min(c[i] for c in cs) for i in range(3)]
            ob["art_leaf_hi"] = [max(c[i] for c in cs) for i in range(3)]
        lo, hi = Vector(ob["art_leaf_lo"]), Vector(ob["art_leaf_hi"])
        size = hi - lo
        thin = min(range(2), key=lambda i: size[i])
        wide = 1 - thin
        quads = []
        self.box(quads, lo, hi)
        cuts = [(0.0, 0.09, 0.0, 1.0), (0.91, 1.0, 0.0, 1.0), (0.0, 1.0, 0.0, 0.12), (0.0, 1.0, 0.95, 1.0),
                (0.47, 0.53, 0.0, 1.0)]
        cuts += [(0.0, 1.0, t - 0.012, t + 0.012) for t in (0.33, 0.54, 0.75)]
        for a0, a1, z0, z1 in cuts:
            b_lo, b_hi = lo.copy(), hi.copy()
            b_lo[thin] -= 0.012
            b_hi[thin] += 0.012
            b_lo[wide], b_hi[wide] = lo[wide] + size[wide] * a0, lo[wide] + size[wide] * a1
            b_lo.z, b_hi.z = lo.z + size.z * z0, lo.z + size.z * z1
            self.box(quads, b_lo, b_hi)
        bm = bmesh.new()
        for quad in quads:
            bm.faces.new([bm.verts.new(p) for p in quad])
        bm.to_mesh(ob.data)
        bm.free()
        ob.data.materials.clear()
        ob.data.materials.append(self.material("glazing"))

    def leaf(self, ob, mat):
        if "art_leaf_lo" not in ob:
            cs = [Vector(c) for c in ob.bound_box]
            ob["art_leaf_lo"] = [min(c[i] for c in cs) for i in range(3)]
            ob["art_leaf_hi"] = [max(c[i] for c in cs) for i in range(3)]
        lo, hi = Vector(ob["art_leaf_lo"]), Vector(ob["art_leaf_hi"])
        size = hi - lo
        thin = min(range(2), key=lambda i: size[i])
        wide = 1 - thin
        quads = []
        self.box(quads, lo, hi)
        rows = [(0.12, 0.42), (0.47, 0.72), (0.77, 0.95)]
        for sign, plane in ((-1.0, lo[thin]), (1.0, hi[thin])):
            normal = Vector((sign, 0)) if thin == 0 else Vector((0, sign))
            run = Run(None, normal, plane, lo[wide], hi[wide], lo.z, hi.z)
            columns = 2 if size[wide] > 0.7 else 1
            for r0, r1 in rows:
                for i in range(columns):
                    self.panel(quads, run, lo[wide] + size[wide] * i / columns + 0.04,
                               lo[wide] + size[wide] * (i + 1) / columns - 0.04,
                               lo.z + size.z * r0, lo.z + size.z * r1)
        bm = bmesh.new()
        layer = bm.loops.layers.uv.new("UVMap")
        for quad in quads:
            bm.faces.new([bm.verts.new(p) for p in quad])
        bm.normal_update()
        for face in bm.faces:
            for loop in face.loops:
                loop[layer].uv = self.uv(mat, loop.vert.co, face.normal)
        bm.to_mesh(ob.data)
        bm.free()
        ob.data.materials.clear()
        ob.data.materials.append(self.material(mat))


def clear():
    for ob in [o for o in bpy.data.objects if o.get("generated_art")]:
        bpy.data.objects.remove(ob)
    for me in [m for m in bpy.data.meshes if m.users == 0 and not m.use_fake_user]:
        bpy.data.meshes.remove(me)


def forget_library():
    for me in [m for m in bpy.data.meshes if m.name.startswith("lib.")]:
        bpy.data.meshes.remove(me)
    for mat in [m for m in bpy.data.materials if m.name.startswith("lib_")]:
        bpy.data.materials.remove(mat)
    for img in [i for i in bpy.data.images if i.name.startswith("lib_")]:
        bpy.data.images.remove(img)


def bounds_of(ob):
    cs = [ob.matrix_world @ Vector(c) for c in ob.bound_box]
    return (Vector((min(c[i] for c in cs) for i in range(3))), Vector((max(c[i] for c in cs) for i in range(3))))


def main(rooms=None, refresh=False):
    made_textures = textures.make(TEX_DIR)
    clear()
    if refresh:
        forget_library()
    build = Build(set(rooms) if rooms else None)
    build.missing = set()
    build.stairs = []
    ids = {room.id for room in build.rooms.values()}
    structural = []
    for ob in bpy.data.objects:
        kind = ob.get("gb_kind")
        if ob.type != "MESH" or ob.name.startswith("art."):
            continue
        names = set(str(ob.get("rooms", ob.get("room", ""))).split(","))
        lo, hi = bounds_of(ob)
        if kind == "ceiling":
            build.ceilings.append((lo.x, lo.y, hi.x, hi.y, lo.z))
        if kind == "stair":
            build.stairs.append((lo.x, lo.y, hi.x, hi.y, lo.z, hi.z))
        if not names & ids:
            continue
        if kind == "fireplace" and ob.name.startswith("fireplace."):
            build.solids.append((lo.x, lo.y, hi.x, hi.y, lo.z, hi.z))
            structural.append(ob)
        elif kind in ("wall", "floor", "ceiling"):
            structural.append(ob)
    walls = []
    for ob in structural:
        build.shell(ob)
        if ob.get("gb_kind") == "wall":
            lo, hi = bounds_of(ob)
            walls.append((lo, hi, min(hi.x - lo.x, hi.y - lo.y) / 2))

    def half_of(x, y, z):
        for lo, hi, value in walls:
            if lo.x - 0.01 <= x <= hi.x + 0.01 and lo.y - 0.01 <= y <= hi.y + 0.01 and lo.z - 0.3 <= z <= hi.z:
                return value
        return 0.1

    for room in build.rooms.values():
        build.furniture(room)
        if room.style.get("outdoor"):
            continue
        build.trim(room)
        build.doors(room, half_of)
        build.windows(room)
        build.fireplace(room)
        build.fixtures(room)
    for ob in list(bpy.data.objects):
        kind = ob.get("gb_kind")
        if ob.name.startswith("art.") or ob.type != "MESH" or kind not in ("balustrade", "stair", "doorleaf"):
            continue
        lo, hi = bounds_of(ob)
        room = build.room_at((lo + hi) / 2)
        if room is None:
            continue
        if kind == "balustrade":
            build.balustrade(ob)
        elif kind == "stair":
            build.stair(ob, room.style["stair"])
        else:
            door = bpy.data.objects.get(f"door.{ob.get('door')}")
            if door is not None and door.get("kind") in GLAZED:
                build.glazed(ob)
            else:
                service_door = door is not None and door.get("zone") == "service"
                build.leaf(ob, "paint_green" if service_door else "oak")
    tris = 0
    for name in build.made:
        me = bpy.data.objects[name].data
        me.calc_loop_triangles()
        tris += len(me.loop_triangles) if bpy.data.objects[name].type == "MESH" else 0
    return {"objects": len(build.made), "triangles": tris, "textures": made_textures,
            "missing": sorted(build.missing)}
