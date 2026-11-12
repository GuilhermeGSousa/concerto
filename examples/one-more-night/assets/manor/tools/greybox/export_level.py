import hashlib
import heapq
import json
import math
import os
import sys
from collections import defaultdict

import bpy

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SPEC_PATH = os.path.join(ROOT, "design", "level", "level_spec.json")

LEVEL_COLLECTIONS = {"cellar": "L_cellar", "ground": "L_ground", "first": "L_first",
                     "attic": "L_attic", "grounds": "L_grounds"}
LISTEN_OUT = 0.5
NO_PASS_KINDS = {"hatch"}
RAMP_DOOR_KINDS = {"steps"}
NO_LISTEN_KINDS = {"hatch", "archway", "opening", "steps"}
PERSON_TAGS = {"stand", "lean", "pair", "work", "window", "stage", "seat", "bed", "listen"}
ON_FLOOR_TAGS = {"stand", "lean", "pair", "work", "window", "stage", "listen"}
SAMPLE = 0.2
OBST_INFLATE = 0.15
WAY_STEP = {"forecourt": 2.0, "garden": 2.0, "yard": 2.0}
WAY_REACH = 2.3
WAY_CLEAR = 0.3
PROP_REACH = 1.5
BEDROOMS = ["master", "g1", "g2", "g3", "g4", "g5"]
OUTDOOR_PAIR_SETS = {"terrace": 2, "garden": 2}
OBSTACLE_KINDS = {"furniture", "fireplace", "stair", "balustrade"}
OBSTACLE_NAMES = ("F06.fireplace", "dumbwaiter.shaft", "F05.cover")

ROUTES = [
    ("drawing -> dining via hall", "drawing", (5.5, 4.0), "dining", (23.0, 5.5), True, 19, 14),
    ("conservatory -> billiard via corridors", "conservatory", (-4.0, 11.0), "billiard", (24.5, 13.0), True, 31, 22),
    ("hall -> g1 via main stairs", "hall", (15.0, 4.0), "g1", (21.75, 4.0), True, 26, 21),
    ("servhall -> g2 via back stairs", "servhall", (39.5, 4.0), "g2", (27.0, 4.0), False, 29, 23),
    ("conservatory -> g1 (longest public)", "conservatory", (-4.0, 11.0), "g1", (21.75, 4.0), True, 38, 30),
    ("garden far corner -> hall", "garden", None, "hall", (13.0, 14.0), False, 28, 20),
    ("kitchen -> garage", "kitchen", (33.5, 4.0), "garage", (48.0, 22.0), False, 26, 19),
    ("servhall -> attic_staff", "servhall", (39.5, 4.0), "attic_staff", (21.75, 11.5), False, 38, 33),
]


def bbox(ob):
    ps = [ob.matrix_world @ v.co for v in ob.data.vertices]
    return (min(p.x for p in ps), min(p.y for p in ps), max(p.x for p in ps), max(p.y for p in ps),
            min(p.z for p in ps), max(p.z for p in ps))


class Scene:
    def __init__(self):
        bpy.context.view_layer.update()
        self.spec = json.load(open(SPEC_PATH))
        obs = list(bpy.data.objects)
        self.rooms = {}
        for ob in obs:
            if ob.get("gb_kind") == "room":
                vs = [ob.matrix_world @ v.co for v in ob.data.vertices]
                parts = []
                for i in range(0, len(vs), 8):
                    c = vs[i:i + 8]
                    parts.append((min(p.x for p in c), min(p.y for p in c), max(p.x for p in c),
                                  max(p.y for p in c), min(p.z for p in c), max(p.z for p in c)))
                rid = ob.name[len("room."):]
                self.rooms[rid] = dict(id=rid, zone=ob["zone"], level=ob["level"], timeline=ob["timeline"],
                                       outdoor=bool(ob.get("outdoor")), parts=parts,
                                       area=sum((p[2] - p[0]) * (p[3] - p[1]) for p in parts))
        self.room_names = [o.name for o in obs if o.get("gb_kind") == "room"]
        self.inner = defaultdict(list)
        for a in self.rooms.values():
            for b in self.rooms.values():
                if a is b or b["area"] >= a["area"]:
                    continue
                if any(min(p[2], q[2]) - max(p[0], q[0]) > 1e-3 and min(p[3], q[3]) - max(p[1], q[1]) > 1e-3
                       and min(p[5], q[5]) - max(p[4], q[4]) > 1e-3 for p in a["parts"] for q in b["parts"]):
                    self.inner[a["id"]].append(b["id"])
        self.doors = []
        for ob in obs:
            if ob.get("gb_kind") == "door":
                m = ob.matrix_world.to_3x3()
                self.doors.append(dict(id=ob.name[len("door."):], a=ob["a"], b=ob["b"], kind=ob["kind"],
                                       zone=ob["zone"], default=ob["default"], key=ob.get("key", ""),
                                       feature=ob.get("feature", ""), leaves=ob.get("leaves", 0),
                                       pos=tuple(ob.matrix_world.translation), n=(m.col[1].x, m.col[1].y)))
        self.door_names = [o.name for o in obs if o.get("gb_kind") == "door"]
        self.anchors = []
        for ob in obs:
            if ob.get("gb_kind") == "anchor":
                self.anchors.append(dict(id=ob.name[len("anchor."):], pos=tuple(ob.matrix_world.translation),
                                         facing=math.degrees(ob.matrix_world.to_euler().z) % 360.0,
                                         tags=[t for t in ob["tags"].split(",") if t], prop_room=ob.get("room")))
        self.named = []
        for ob in obs:
            n = ob.name
            marker = (ob.type == "EMPTY" and n.startswith(("spawn.", "bed.", "dumbwaiter.", "acoustic."))) \
                or n == "bellboard" or n.startswith(("phone.", "bell."))
            if marker:
                f = float(ob["facing"]) if ob.get("facing") is not None else math.degrees(ob.matrix_world.to_euler().z)
                self.named.append(dict(id=n.replace(".", "_"), object=n, pos=tuple(ob.matrix_world.translation),
                                       facing=f % 360.0, tags=["bed"] if n.startswith("bed.") else []))
        self.obstacles = []
        for ob in obs:
            if ob.type != "MESH" or not ob.data.vertices:
                continue
            if ob.get("gb_kind") in OBSTACLE_KINDS or ob.name.startswith(OBSTACLE_NAMES):
                self.obstacles.append(bbox(ob))
        self.nav = []
        for ob in obs:
            if ob.get("gb_kind") != "nav":
                continue
            for poly in ob.data.polygons:
                vs = [ob.matrix_world @ ob.data.vertices[i].co for i in poly.vertices]
                self.nav.append(dict(x0=min(v.x for v in vs), y0=min(v.y for v in vs), x1=max(v.x for v in vs),
                                     y1=max(v.y for v in vs), vs=vs, stairs=bool(ob.get("stairs")),
                                     name=ob.name, landing="stair" in ob.keys() or ob.name.startswith("nav.landing")))
        self.features = {}
        root = bpy.data.collections.get("Features")
        for c in (root.children if root else []):
            self.features[c.name] = [o for o in c.objects]

    @staticmethod
    def plane_z(q, x, y):
        vs = q["vs"]
        zs = [v.z for v in vs]
        if max(zs) - min(zs) < 1e-6:
            return zs[0]
        west = [v.z for v in vs if abs(v.x - q["x0"]) < 1e-6]
        east = [v.z for v in vs if abs(v.x - q["x1"]) < 1e-6]
        if max(west) - min(west) < 1e-6 and max(east) - min(east) < 1e-6:
            return west[0] + (east[0] - west[0]) * (x - q["x0"]) / (q["x1"] - q["x0"])
        za = [v.z for v in vs if abs(v.y - q["y0"]) < 1e-6][0]
        zb = [v.z for v in vs if abs(v.y - q["y1"]) < 1e-6][0]
        return za + (zb - za) * (y - q["y0"]) / (q["y1"] - q["y0"])

    def floor_z(self, x, y, zref):
        best = None
        for q in self.nav:
            if q["x0"] - 1e-4 <= x <= q["x1"] + 1e-4 and q["y0"] - 1e-4 <= y <= q["y1"] + 1e-4:
                z = self.plane_z(q, x, y)
                if best is None or abs(z - zref) < abs(best - zref):
                    best = z
        return best

    def nav_distance(self, x, y, z):
        best = 1e9
        for q in self.nav:
            cx, cy = min(max(x, q["x0"]), q["x1"]), min(max(y, q["y0"]), q["y1"])
            fz = self.plane_z(q, cx, cy)
            if z - 2.5 <= fz <= z + 0.1:
                best = min(best, math.hypot(x - cx, y - cy))
        return best

    def rooms_at(self, x, y, z):
        out = []
        for r in self.rooms.values():
            for p in r["parts"]:
                if p[0] - 1e-4 <= x <= p[2] + 1e-4 and p[1] - 1e-4 <= y <= p[3] + 1e-4 and p[4] - 0.06 <= z <= p[5] + 0.06:
                    out.append((abs(z - p[4]) > 0.06, r["area"], r["id"]))
                    break
        return [rid for *_, rid in sorted(out)]

    def room_of(self, pos):
        c = self.rooms_at(*pos)
        return c[0] if c else None

    def in_room(self, room, x, y, z):
        r = self.rooms[room]
        if not any(p[0] - 1e-4 <= x <= p[2] + 1e-4 and p[1] - 1e-4 <= y <= p[3] + 1e-4 and p[4] - 0.1 <= z <= p[5] + 0.1
                   for p in r["parts"]):
            return False
        for inner in self.inner[room]:
            if any(p[0] < x < p[2] and p[1] < y < p[3] and p[4] - 0.1 <= z <= p[5] + 0.1
                   for p in self.rooms[inner]["parts"]):
                return False
        return True


class Graph:
    def __init__(self, sc):
        self.sc = sc
        self.nodes = {}
        self.adj = defaultdict(list)
        self.problems = []

    def add_node(self, nid, pos, room, kind):
        self.nodes[nid] = dict(pos=pos, room=room, kind=kind)

    def add_edge(self, a, b, stairs=False, door=None):
        pa, pb = self.nodes[a]["pos"], self.nodes[b]["pos"]
        d = math.dist(pa, pb) / (0.75 if stairs else 1.0)
        self.adj[a].append((b, d, door))
        self.adj[b].append((a, d, door))

    def visible(self, a, b):
        sc = self.sc
        na, nb = self.nodes[a], self.nodes[b]
        pa, pb = na["pos"], nb["pos"]
        dist = math.hypot(pa[0] - pb[0], pa[1] - pb[1])
        if abs(pa[2] - pb[2]) > 0.05 + 0.06 * dist:
            return False
        own = set()
        for p in (pa, pb):
            for i, o in enumerate(self.room_obst[na["room"]]):
                if o[0] - OBST_INFLATE <= p[0] <= o[2] + OBST_INFLATE and o[1] - OBST_INFLATE <= p[1] <= o[3] + OBST_INFLATE \
                        and o[4] < p[2] + 1.2 and o[5] > p[2] - 0.6:
                    own.add(i)
        n = max(1, int(dist / SAMPLE))
        for k in range(n + 1):
            t = k / n
            x, y, z = pa[0] + (pb[0] - pa[0]) * t, pa[1] + (pb[1] - pa[1]) * t, pa[2] + (pb[2] - pa[2]) * t
            if not sc.in_room(na["room"], x, y, z + 0.3):
                return False
            for i, o in enumerate(self.room_obst[na["room"]]):
                if i in own:
                    continue
                if o[0] - OBST_INFLATE < x < o[2] + OBST_INFLATE and o[1] - OBST_INFLATE < y < o[3] + OBST_INFLATE \
                        and o[4] < z + 1.2 and o[5] > z + 0.1:
                    return False
        return True

    def build(self):
        sc = self.sc
        for a in sc.anchors:
            if PERSON_TAGS & set(a["tags"]):
                x, y, z = a["pos"]
                if not ON_FLOOR_TAGS & set(a["tags"]) or "seat" in a["tags"]:
                    fz = sc.floor_z(x, y, z)
                    z = fz if fz is not None and z - 1.0 <= fz <= z + 0.01 else z
                self.add_node(a["id"], (x, y, z), a["room"], "anchor")
        self.listen = []
        for d in sc.doors:
            if d["kind"] in NO_PASS_KINDS or d["kind"] in RAMP_DOOR_KINDS:
                continue
            ids = []
            for side, sgn, room in (("a", 1, d["a"]), ("b", -1, d["b"])):
                x, y = d["pos"][0] + d["n"][0] * LISTEN_OUT * sgn, d["pos"][1] + d["n"][1] * LISTEN_OUT * sgn
                z = sc.floor_z(x, y, d["pos"][2])
                if z is None:
                    self.problems.append(f"door {d['id']}: no walkable floor on side {side}")
                    z = d["pos"][2]
                nid = f"listen_{d['id']}_{side}"
                self.add_node(nid, (x, y, z), room, "door")
                ids.append(nid)
                if d["kind"] not in NO_LISTEN_KINDS and d["leaves"]:
                    face = math.degrees(math.atan2(d["n"][0] * sgn, -d["n"][1] * sgn)) % 360.0
                    self.listen.append(dict(id=nid, pos=(x, y, z), facing=face, tags=["listen"], room=room))
            self.add_edge(ids[0], ids[1], door=d["id"])
        doored = {frozenset((d["a"], d["b"])) for d in sc.doors}
        outs = [r for r in sc.rooms.values() if r["outdoor"]]
        for i, ra in enumerate(outs):
            for rb in outs[i + 1:]:
                if frozenset((ra["id"], rb["id"])) in doored:
                    continue
                for p in ra["parts"]:
                    for q in rb["parts"]:
                        for axis in (0, 1):
                            o = 1 - axis
                            for edge_p, edge_q, sgn in ((p[axis + 2], q[axis], 1), (p[axis], q[axis + 2], -1)):
                                lo, hi = max(p[o], q[o]), min(p[o + 2], q[o + 2])
                                if abs(edge_p - edge_q) > 1e-4 or hi - lo < 0.9:
                                    continue
                                mid = (lo + hi) / 2
                                ids = []
                                for room, off in ((ra["id"], -sgn * LISTEN_OUT), (rb["id"], sgn * LISTEN_OUT)):
                                    xy = [0.0, 0.0]
                                    xy[axis], xy[o] = edge_p + off, mid
                                    z = sc.floor_z(xy[0], xy[1], 0.0) or 0.0
                                    nid = f"open_{ra['id']}_{rb['id']}_{room}"
                                    self.add_node(nid, (xy[0], xy[1], z), room, "open")
                                    ids.append(nid)
                                self.add_edge(ids[0], ids[1])
        ramp_ends, landings = [], []
        for n_, q in enumerate(sc.nav):
            if q["stairs"]:
                zs = sorted(v.z for v in q["vs"])
                lo = [v for v in q["vs"] if abs(v.z - zs[0]) < 1e-4]
                hi = [v for v in q["vs"] if abs(v.z - zs[-1]) < 1e-4]
                ids = []
                for tag, grp in (("lo", lo), ("hi", hi)):
                    pos = (sum(v.x for v in grp) / len(grp), sum(v.y for v in grp) / len(grp), grp[0].z)
                    nid = f"{q['name']}.{tag}"
                    self.add_node(nid, pos, sc.room_of(pos), "stair")
                    ramp_ends.append(nid)
                    ids.append(nid)
                self.add_edge(ids[0], ids[1], stairs=True)
            elif q["landing"]:
                pos = ((q["x0"] + q["x1"]) / 2, (q["y0"] + q["y1"]) / 2, q["vs"][0].z)
                nid = f"{q['name']}.mid"
                self.add_node(nid, pos, sc.room_of(pos), "stair")
                landings.append((nid, q))
        for nid, q in landings:
            for e in ramp_ends:
                p = self.nodes[e]["pos"]
                if q["x0"] - 0.35 <= p[0] <= q["x1"] + 0.35 and q["y0"] - 0.35 <= p[1] <= q["y1"] + 0.35 \
                        and abs(p[2] - q["vs"][0].z) < 0.06:
                    self.add_edge(nid, e)
        for room, r in sc.rooms.items():
            step = WAY_STEP.get(room, 1.0)
            n_ = 0
            for p in r["parts"]:
                x = p[0] + 0.5
                while x < p[2] - 0.2:
                    y = p[1] + 0.5
                    while y < p[3] - 0.2:
                        z = sc.floor_z(x, y, p[4])
                        if z is not None and p[4] - 0.06 <= z <= p[5] and sc.room_of((x, y, z)) == room \
                                and all(sc.in_room(room, x + dx, y + dy, z + 0.3)
                                        for dx, dy in ((WAY_CLEAR, 0), (-WAY_CLEAR, 0), (0, WAY_CLEAR), (0, -WAY_CLEAR))) \
                                and not any(o[0] - WAY_CLEAR < x < o[2] + WAY_CLEAR and o[1] - WAY_CLEAR < y < o[3] + WAY_CLEAR
                                            and o[4] < z + 1.2 and o[5] > z + 0.1 for o in sc.obstacles):
                            n_ += 1
                            self.add_node(f"way_{room}_{n_}", (x, y, z), room, "way")
                        y += step
                    x += step
        by_room = defaultdict(list)
        for nid, n in self.nodes.items():
            if n["room"] is None:
                self.problems.append(f"graph node {nid}: in no room")
                continue
            by_room[n["room"]].append(nid)
        self.room_obst = {}
        for room in by_room:
            parts = sc.rooms[room]["parts"]
            self.room_obst[room] = [o for o in sc.obstacles if any(
                min(o[2], p[2]) - max(o[0], p[0]) > -0.3 and min(o[3], p[3]) - max(o[1], p[1]) > -0.3
                and o[4] < p[5] and o[5] > p[4] for p in parts)]
        for room, ids in by_room.items():
            for i, a in enumerate(ids):
                for b in ids[i + 1:]:
                    if "way" in (self.nodes[a]["kind"], self.nodes[b]["kind"]):
                        pa, pb = self.nodes[a]["pos"], self.nodes[b]["pos"]
                        if math.hypot(pa[0] - pb[0], pa[1] - pb[1]) > WAY_REACH * WAY_STEP.get(room, 1.0):
                            continue
                    if self.visible(a, b):
                        self.add_edge(a, b)

    def dijkstra(self, src, allow=None):
        dist = {src: 0.0}
        heap = [(0.0, src)]
        while heap:
            d, u = heapq.heappop(heap)
            if d > dist.get(u, 1e18):
                continue
            for v, w, door in self.adj[u]:
                if allow is not None and door is not None and not allow(door):
                    continue
                nd = d + w
                if nd < dist.get(v, 1e18):
                    dist[v] = nd
                    heapq.heappush(heap, (nd, v))
        return dist

    def nearest(self, room, xy, standing=True):
        best = None
        for nid, n in self.nodes.items():
            if n["room"] != room or (standing and n["kind"] != "anchor"):
                continue
            if standing and not (ON_FLOOR_TAGS & set(self.sc.anchor_by_id[nid]["tags"])):
                continue
            d = math.hypot(n["pos"][0] - xy[0], n["pos"][1] - xy[1])
            if best is None or d < best[0]:
                best = (d, nid)
        return best[1] if best else None


def camel(s):
    return "".join(w.capitalize() for w in str(s).split("_"))


def f2(v):
    return f"{v:.2f}".replace("-0.00", "0.00")


def q(s):
    return '"' + s + '"'


def opt(s):
    return f"Some({q(s)})" if s else "None"


def lst(items):
    return "[" + ", ".join(items) + "]"


def anchor_minimums(sc, anchors):
    rules = sc.spec["anchor_rules"]
    by_room = defaultdict(list)
    for a in anchors:
        by_room[a["room"]].append(a)

    def tally(room):
        c = defaultdict(int)
        sets = set()
        for a in by_room[room]:
            for t in a["tags"]:
                c[t] += 1
            if "pair" in a["tags"] and a["id"][-2:] in ("_a", "_b"):
                sets.add(a["id"][:-2])
        c["pair_sets"] = len(sets)
        return c
    bad = []

    def need(room, what, have, want):
        if have < want:
            bad.append(f"{room}: {have} {what}, needs {want}")
    for grp in ("small", "medium", "large"):
        for room in rules[grp]["rooms"]:
            c = tally(room)
            need(room, "anchors", len(by_room[room]), rules[grp]["min"])
            for k, v in rules[grp]["of_which"].items():
                need(room, k, c[k], v)
    for room in rules["corridors"]["rooms"]:
        parts = sc.rooms[room]["parts"]
        length = max(max(p[2] - p[0], p[3] - p[1]) for p in parts)
        have = sum(1 for a in by_room[room] if {"stand", "lean"} & set(a["tags"]))
        need(room, "stand/lean", have, math.ceil(length * rules["corridors"]["per_metre"] - 1e-6))
    for room, n in rules["outdoors"].items():
        need(room, "anchors", len(by_room[room]), n)
        need(room, "pair_sets", tally(room)["pair_sets"], OUTDOOR_PAIR_SETS.get(room, 0))
    for room in BEDROOMS:
        c = tally(room)
        for k, v in (("bed", 1), ("seat", 1), ("drawer", 1), ("surface", 2)):
            need(room, k, c[k], v)
    need("drawing", "stage", sum(1 for a in anchors if "stage" in a["tags"]), 12)
    return bad


def run(out_dir=None, export_glb=True):
    sc = Scene()
    lint = []

    unresolved, mismatched = [], []
    for a in sc.anchors:
        a["room"] = sc.room_of(a["pos"])
        if a["room"] is None:
            unresolved.append(a["id"])
            a["room"] = a["prop_room"]
        elif a["prop_room"] and a["room"] != a["prop_room"]:
            mismatched.append(f"{a['id']} (in {a['room']}, tagged {a['prop_room']})")
    for n in sc.named:
        n["room"] = sc.room_of(n["pos"])
    sc.anchor_by_id = {a["id"]: a for a in sc.anchors}

    g = Graph(sc)
    g.build()
    for l in g.listen:
        sc.anchor_by_id[l["id"]] = l
    all_anchors = sc.anchors + g.listen

    dup = []
    for label, names in (("room", [r for r in sc.room_names]), ("door", sc.door_names),
                         ("anchor", [a["id"] for a in all_anchors] + [n["id"] for n in sc.named])):
        seen = set()
        for n in names:
            base = n.rsplit(".", 1)[0] if n[-4:-3] == "." and n[-3:].isdigit() else n
            if base in seen or base != n:
                dup.append(f"{label} {n}")
            seen.add(base)
    lint.append(("ids are unique", dup))
    bad = []
    for d in sc.doors:
        for side, sgn, room in (("a", 1, d["a"]), ("b", -1, d["b"])):
            if room not in sc.rooms:
                bad.append(f"{d['id']}: unknown room {room}")
                continue
            x, y = d["pos"][0] + d["n"][0] * 0.3 * sgn, d["pos"][1] + d["n"][1] * 0.3 * sgn
            zs = [d["pos"][2] + 0.5, sc.rooms[room]["parts"][0][4] + 0.5]
            if not any(sc.in_room(room, x, y, z) or room in sc.rooms_at(x, y, z) for z in zs):
                bad.append(f"{d['id']}: side {side} is not in {room}")
    lint.append(("every door joins two volumes", bad))

    lint.append(("every anchor resolves to one room", unresolved + mismatched))
    bad = []
    for a in all_anchors:
        x, y, z = a["pos"]
        if ON_FLOOR_TAGS & set(a["tags"]) and "seat" not in a["tags"]:
            fz = sc.floor_z(x, y, z)
            if fz is None or abs(fz - z) > 0.06:
                bad.append(a["id"])
        elif sc.nav_distance(x, y, z) > PROP_REACH:
            bad.append(a["id"])
    lint.append(("every anchor stands on (or within reach of) walkable floor", bad))

    lint.append(("each room meets its anchor minimum", anchor_minimums(sc, sc.anchors)))

    door_zone = {d["id"]: d for d in sc.doors}
    spawn = {n["object"]: n for n in sc.named if n["object"].startswith("spawn.")}
    bad = []
    reach = {}
    for who, key in (("staff", "spawn.staff"), ("guest", "spawn.guest")):
        s = spawn.get(key)
        if not s or not s["room"]:
            bad.append(f"{key} missing")
            continue
        start = g.nearest(s["room"], s["pos"][:2])
        if who == "staff":
            dist = g.dijkstra(start)
            want = set(sc.rooms)
        else:
            home = s["room"]
            dist = g.dijkstra(start, allow=lambda d: door_zone[d]["zone"] == "public" or home in (door_zone[d]["a"], door_zone[d]["b"]))
            want = {r for r, v in sc.rooms.items() if v["zone"] in ("public", "open")}
        got = {g.nodes[n]["room"] for n in dist}
        reach[who] = got
        bad += [f"{who} cannot reach {r}" for r in sorted(want - got)]
    lint.append(("the Staff reach every room, the Guest every public room", bad))

    first = next(iter(g.nodes))
    comp = g.dijkstra(first)
    lint.append(("the anchor graph is connected", g.problems + sorted(
        n for n, v in g.nodes.items() if n not in comp and v["kind"] != "way")))

    bad = []
    fdoors = defaultdict(list)
    for d in sc.doors:
        if d["feature"]:
            fdoors[d["feature"]].append(d["id"])
    links = []
    names = {o.name for o in bpy.data.objects}
    for f in sc.spec["features"]:
        fid = f["id"]
        objs = sc.features.get(fid, [])
        if not objs:
            bad.append(f"{fid}: no objects")
        for d in f.get("doors", []):
            if d not in fdoors[fid]:
                bad.append(f"{fid}: door {d} missing")
        if "acoustic" in f:
            s, t = f"acoustic.{fid}.src", f"acoustic.{fid}.dst"
            if s not in names or t not in names:
                bad.append(f"{fid}: acoustic ends missing")
            else:
                so = bpy.data.objects[s]
                links.append(dict(feature=fid, src=s, dst=t, radius=float(so.get("radius", 0.0)),
                                  two_way=bool(so.get("two_way")), area=so.get("area")))
        if fid not in bpy.context.scene.keys():
            bad.append(f"{fid}: no scene flag")
    lint.append(("every feature has its objects", bad))

    sp = sc.spec["metrics"]["speeds"]
    lim = sc.spec["walk_limits"]
    budget, bad = [], []
    for label, ra, pa, rb, pb, indoor_public, spec_m, spec_s in ROUTES:
        to = g.nearest(rb, pb)
        dist_to = g.dijkstra(to) if to else {}
        if pa is None:
            cands = [n for n, v in g.nodes.items() if v["room"] == ra and v["kind"] == "anchor"
                     and ON_FLOOR_TAGS & set(sc.anchor_by_id[n]["tags"]) and n in dist_to]
            frm = max(cands, key=lambda n: dist_to[n]) if cands else None
        else:
            frm = g.nearest(ra, pa)
        limit = lim["indoor_public_max_s"] if indoor_public else lim["any_max_s"]
        if frm is None or to is None or frm not in dist_to:
            budget.append(dict(route=label, frm=frm, to=to, metres=None, seconds=None, limit=limit, ok=False,
                               spec_m=spec_m, spec_s=spec_s))
            bad.append(f"{label}: no path")
            continue
        eff = dist_to[frm]
        secs = eff / sp["walk"]
        ok = secs <= limit + 1e-6
        budget.append(dict(route=label, frm=frm, to=to, metres=round(eff, 1), seconds=round(secs, 1),
                           limit=limit, ok=ok, spec_m=spec_m, spec_s=spec_s))
        if not ok:
            bad.append(f"{label}: {secs:.1f} s > {limit} s")
    lint.append(("every walk-budget route is within its limit", bad))

    persons = [a["id"] for a in sc.anchors if a["id"] in g.nodes]
    travel_full = {}
    for a in persons:
        d = g.dijkstra(a)
        for b in persons:
            if a < b and b in d:
                travel_full[(a, b)] = d[b] / sp["npc"]
    reps = {}
    by_room = defaultdict(list)
    for a in sc.anchors:
        if a["id"] in g.nodes and ON_FLOOR_TAGS & set(a["tags"]):
            by_room[a["room"]].append(a)
    for room, lst_ in by_room.items():
        cx = sum(a["pos"][0] for a in lst_) / len(lst_)
        cy = sum(a["pos"][1] for a in lst_) / len(lst_)
        reps[room] = min(lst_, key=lambda a: math.hypot(a["pos"][0] - cx, a["pos"][1] - cy))["id"]
    rep_ids = sorted(reps.values())

    ok = all(not ids for _, ids in lint)
    report = dict(ok=ok, lint=[dict(check=c, result="PASS" if not ids else "FAIL", ids=ids) for c, ids in lint],
                  walk_budget=budget, counts=dict(rooms=len(sc.rooms), doors=len(sc.doors), anchors=len(sc.anchors),
                                                  listen=len(g.listen), named=len(sc.named), graph_nodes=len(g.nodes),
                                                  graph_edges=sum(len(v) for v in g.adj.values()) // 2,
                                                  travel_pairs=len(travel_full)),
                  representatives=reps)

    if out_dir:
        os.makedirs(out_dir, exist_ok=True)
        L = ["LevelManifest(", "    version: 1,", "    rooms: ["]
        for r in sorted(sc.rooms.values(), key=lambda r: r["id"]):
            bounds = sorted({(p[0], p[1], p[2], p[3]) for p in r["parts"]})
            bs = lst(f"(({f2(b[0])}, {f2(b[1])}), ({f2(b[2])}, {f2(b[3])}))" for b in bounds)
            L.append(f"        Room(id: {q(r['id'])}, level: {camel(r['level'])}, zone: {camel(r['zone'])}, "
                     f"timeline: {q(r['timeline'])}, bounds: {bs}),")
        L += ["    ],", "    doors: ["]
        for d in sorted(sc.doors, key=lambda d: d["id"]):
            L.append(f"        Door(id: {q(d['id'])}, a: {q(d['a'])}, b: {q(d['b'])}, kind: {camel(d['kind'])}, "
                     f"zone: {camel(d['zone'])}, default: {camel(d['default'])}, key: {opt(d['key'])}, "
                     f"feature: {opt(d['feature'])}),")
        L += ["    ],", "    anchors: ["]
        for a in sorted(all_anchors + sc.named, key=lambda a: a["id"]):
            p = a["pos"]
            L.append(f"        Anchor(id: {q(a['id'])}, room: {q(a['room'] or '')}, pos: ({f2(p[0])}, {f2(p[1])}, {f2(p[2])}), "
                     f"facing: {a['facing']:.1f}, tags: {lst(camel(t) for t in a['tags'])}),")
        L += ["    ],", "    features: ["]
        for f in sc.spec["features"]:
            fid = f["id"]
            objs = sc.features.get(fid, [])
            on = sorted(o.name for o in objs if o.get("feature_state") == "on")
            off = sorted(o.name for o in objs if o.get("feature_state") == "off")
            lk = [x for l in links if l["feature"] == fid for x in (l["src"], l["dst"])]
            L.append(f"        Feature(id: {q(fid)}, doors: {lst(q(d) for d in sorted(fdoors[fid]))}, "
                     f"links: {lst(q(x) for x in lk)}, on: {lst(q(x) for x in on)}, off: {lst(q(x) for x in off)}),")
        L += ["    ],", "    links: ["]
        for l in links:
            area = "None"
            if l["area"]:
                v = [float(x) for x in l["area"].split(",")]
                area = f"Some((({f2(v[0])}, {f2(v[1])}), ({f2(v[2])}, {f2(v[3])})))"
            L.append(f"        AcousticLink(feature: {q(l['feature'])}, src: {q(l['src'])}, dst: {q(l['dst'])}, "
                     f"radius: {l['radius']:.1f}, two_way: {'true' if l['two_way'] else 'false'}, area: {area}),")
        L += ["    ],", "    travel: ["]
        for i, a in enumerate(rep_ids):
            for b in rep_ids[i + 1:]:
                t = travel_full.get((a, b)) or travel_full.get((b, a))
                if t is not None:
                    L.append(f"        Travel(from: {q(a)}, to: {q(b)}, seconds: {t:.1f}),")
        L += ["    ],", "    ways: ["]
        for nid in sorted(g.nodes):
            p = g.nodes[nid]["pos"]
            L.append(f"        Way(id: {q(nid)}, pos: ({f2(p[0])}, {f2(p[1])}, {f2(p[2])})),")
        L += ["    ],", "    paths: ["]
        for a in sorted(g.adj):
            for b in sorted({b for b, _, _ in g.adj[a] if a < b}):
                L.append(f"        Path(a: {q(a)}, b: {q(b)}),")
        L += ["    ],", ")", ""]
        text = "\n".join(L)
        mpath = os.path.join(out_dir, "level_manifest.ron")
        with open(mpath, "w") as fh:
            fh.write(text)
        report["manifest"] = mpath
        report["manifest_sha256"] = hashlib.sha256(text.encode()).hexdigest()
        with open(os.path.join(out_dir, "travel_full.csv"), "w") as fh:
            fh.write("from,to,seconds\n")
            for (a, b), t in sorted(travel_full.items()):
                fh.write(f"{a},{b},{t:.1f}\n")
        lines = [f"{'PASS' if not ids else 'FAIL'}  {c}" + ("" if not ids else "\n      " + "\n      ".join(ids))
                 for c, ids in lint]
        lines.append("")
        lines.append("Walk budget (player 1.4 m/s, stairs x0.75; metres are stair-weighted)")
        for b in budget:
            lines.append(f"{'PASS' if b['ok'] else 'FAIL'}  {b['route']}: {b['metres']} m, {b['seconds']} s "
                         f"(limit {b['limit']} s; spec {b['spec_m']} m, {b['spec_s']} s)  {b['frm']} -> {b['to']}")
        lines.append("")
        lines.append(f"manifest sha256 {report['manifest_sha256']}")
        with open(os.path.join(out_dir, "level_lint.txt"), "w") as fh:
            fh.write("\n".join(lines) + "\n")
        report["lint_text"] = "\n".join(lines)
        if export_glb:
            report["glb"] = export_glbs(sc, out_dir)
    return report


def export_glbs(sc, out_dir):
    vl = bpy.context.view_layer
    grounds_rooms = {r for r, v in sc.rooms.items() if v["level"] == "grounds"}
    sets = {}
    for lvl, cname in LEVEL_COLLECTIONS.items():
        coll = bpy.data.collections.get(cname)
        sets[lvl] = list(coll.all_objects) if coll else []
    nav = bpy.data.collections.get("Nav")
    for ob in (nav.objects if nav else []):
        lvl = "grounds" if ob.get("room") in grounds_rooms else ob.get("level", "ground")
        sets[lvl].append(ob)
    every = {ob for obs in sets.values() for ob in obs}
    saved, muted = {}, []
    lc_nav = vl.layer_collection.children.get("Nav")
    nav_hidden = lc_nav.hide_viewport if lc_nav else None
    if lc_nav:
        lc_nav.hide_viewport = False
    for ob in every:
        saved[ob.name] = (ob.hide_get(), ob.hide_viewport)
        if ob.animation_data:
            for fc in ob.animation_data.drivers:
                if fc.data_path in ("hide_viewport", "hide_render") and not fc.mute:
                    fc.mute = True
                    muted.append(fc)
        ob.hide_viewport = False
        ob.hide_set(False)
    vl.update()
    out = {}
    try:
        for lvl, obs in sets.items():
            for ob in bpy.data.objects:
                ob.select_set(False)
            n = 0
            for ob in obs:
                try:
                    ob.select_set(True)
                    n += 1
                except RuntimeError:
                    pass
            path = os.path.join(out_dir, f"manor_{lvl}.glb")
            bpy.ops.export_scene.gltf(filepath=path, export_format="GLB", use_selection=True,
                                      export_extras=True, export_yup=True, export_apply=False,
                                      export_animations=False, export_cameras=False)
            out[lvl] = dict(path=path, objects=n, bytes=os.path.getsize(path))
    finally:
        for ob in bpy.data.objects:
            ob.select_set(False)
        for fc in muted:
            fc.mute = False
        for ob in every:
            h, hv = saved[ob.name]
            ob.hide_viewport = hv
            ob.hide_set(h)
        if lc_nav:
            lc_nav.hide_viewport = nav_hidden
        bpy.context.scene.update_tag()
        vl.update()
    return out


def main(out_dir=None, export_glb=True):
    return run(out_dir, export_glb)


if __name__ == "__main__":
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    out = os.path.abspath(argv[0]) if argv and not argv[0].startswith("--") else os.path.join(ROOT, "export")
    rep = main(out, "--manifest-only" not in argv)
    print(rep.get("lint_text", ""))
    for lvl, g_ in rep.get("glb", {}).items():
        print(f"wrote {g_['path']} ({g_['objects']} objects, {g_['bytes']} bytes)")
    print("LINT", "PASS" if rep["ok"] else "FAIL")
    if bpy.app.background and not rep["ok"]:
        sys.exit(1)
