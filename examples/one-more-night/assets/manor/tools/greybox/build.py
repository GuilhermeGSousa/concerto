import json
import os
from collections import defaultdict

import bmesh
import bpy

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
SPEC_PATH = os.path.join(ROOT, "design", "level", "level_spec.json")

STEPS = {1, 2, 3, 4, 5, 6, 7, 8}

GRID = 0.5
EPS = 1e-6
SLAB = 0.2
OUTDOOR_SLAB_BOTTOM = -1.0
MAIN_BLOCK = (0.0, 0.0, 30.0, 16.0)
LEVELS = ["cellar", "ground", "first", "attic"]
COLLECTIONS = {"cellar": "L_cellar", "ground": "L_ground", "first": "L_first",
               "attic": "L_attic", "grounds": "L_grounds", "nav": "Nav", "features": "Features"}

FLOOR_Z_OVERRIDE = {"garden": -0.6, "shed": -0.6}
CEILING_OVERRIDE = {("garage", "ground"): 3.0, ("shed", "ground"): 2.4,
                    ("hall", "first"): 3.4}
PATH_RAMP = (24.0, 42.0, -0.6, 0.0)
OPEN_EDGES = {("first", frozenset({"hall", "gallery"}))}
DOOR_WIDTH_OVERRIDE = {"d_drawing_cons": 1.6}
SKIP_OPENING_KINDS = {"steps", "gate"}
HATCH_SILL = 0.9

WINDOW = {"ground": (1.2, 0.9, 2.7), "first": (1.2, 0.9, 2.5),
          "attic": (0.8, 0.8, 1.6), "grounds": (1.0, 0.9, 2.1)}
GLAZED_ROOMS = {"conservatory": (1.6, 0.6, 2.8, 2.0)}
WINDOW_SPACING = 3.5
WINDOW_MARGIN = 0.6
NO_WINDOW_ROOMS = {"backstairs"}
NO_WINDOW_LEVELS = {"cellar"}

NO_LEAF_KINDS = {"archway", "steps", "opening"}
TWO_LEAF_KINDS = {"double", "glazed_double", "french_window", "garage"}
GLAZED_KINDS = {"glazed", "glazed_double", "french_window"}
DOUBLE_ACTING_KINDS = {"baize"}
LEAF_T = 0.05
LEAF_GAP = 0.01
FREE_WALL_T = 0.3
OPEN_ANGLE = 90.0

MAIN_STAIR_ARRIVAL_Y = 11.5
BAL_T = 0.06
BAL_H_STAIR = 0.9
BAL_H_FLOOR = 1.0
STAIR_WAIST = 0.15

EAR = 1.5
PHONES = {"phone.hall": ("hall", "ground", (11.35, 13.5, 0.80)),
          "phone.study": ("study", "ground", (10.55, 13.5, 0.76))}
TUBE_ENDS = {"master": ("first", (5.6, 7.85, 1.5)),
             "pantry": ("ground", (29.85, 2.0, 1.5))}
ACOUSTIC = {
    "F01": ("dining", "ground", (26.4, 4.0), 4.0, "pantry", "ground", (28.0, 4.0), False),
    "F02": ("hall", "ground", (15.0, 4.0), 4.0, "gallery", "first", (15.0, 8.6), False),
    "F06": ("study", "ground", (9.0, 13.0), 3.7, "g4", "first", (10.0, 14.8), False),
    "F11": ("study", "ground", (9.0, 13.0), 3.7, "terrace", "ground", (8.0, 16.8), False),
}
HATCH_BOLT_SIDE = "b"

FIREPLACES = {
    "drawing": ("ground", "x", 0.0, 3.0, +1),
    "study":   ("ground", "y", 16.0, 10.0, -1),
    "master":  ("first", "x", 0.0, 3.0, +1),
    "g1":      ("first", "x", 19.0, 4.0, +1),
    "g2":      ("first", "x", 30.0, 3.5, -1),
    "g3":      ("first", "x", 0.0, 13.0, +1),
    "g4":      ("first", "y", 16.0, 10.0, -1),
    "g5":      ("first", "x", 24.5, 13.0, -1),
}
BELL_PULLS = {
    "drawing": ("ground", "x", 0.0, 4.1, +1), "master": ("first", "x", 0.0, 4.1, +1),
    "g1": ("first", "x", 19.0, 5.1, +1), "g2": ("first", "x", 30.0, 4.6, -1),
    "g3": ("first", "x", 0.0, 14.1, +1), "g4": ("first", "y", 16.0, 8.9, -1),
    "g5": ("first", "x", 24.5, 14.1, -1), "study": ("ground", "y", 16.0, 9.0, -1),
    "dining": ("ground", "y", 8.0, 21.0, -1), "library": ("ground", "x", 7.0, 11.2, -1),
    "billiard": ("ground", "x", 19.0, 13.0, +1), "conservatory": ("ground", "x", 0.0, 12.0, -1),
    "dressing": ("first", "x", 11.0, 4.0, -1),
}
BELL_BOARD = ("servhall", "ground", "x", 37.0, 6.2, +1)
DUMBWAITER_PART_T = 0.2
DUMBWAITER_HATCH = (0.6, 0.9, 1.5)
DUMBWAITER_STOPS = {
    "kitchen": ("ground", "x", 30.0, 7.55, +1, "kitchen"),
    "pantry": ("ground", "x", 29.1, 7.55, -1, "pantry"),
    "gallery": ("first", "y", 8.0, 29.55, +1, "gallery"),
}
BEDS = {
    "g5": (19.1, 12.8, 21.1, 14.2, 0.55, "x0"),
    "attic_staff": (22.5, 12.4, 24.4, 13.3, 0.50, "x1"),
}
SPAWNS = {"spawn.guest": ("g5", "first", (21.7, 13.5), -90.0),
          "spawn.staff": ("servhall", "ground", (38.3, 6.2), 90.0)}
BED_MARKERS = {"bed.guest": "g5", "bed.staff": "attic_staff"}

GARDEN_WALL = (0.4, 2.4)
HEDGE = (0.6, 1.4)
PARAPET = (0.3, 0.9)
TERRACE_STEPS = (3.0, 0.15, 0.35)
GARDEN_POND = (3.0, 24.25, 7.0, 26.75)
GARDEN_GRAVEL = [(12.0, 21.0, 14.0, 35.6),
                 (-5.8, 29.0, 23.8, 31.0),
                 (11.0, 28.0, 15.0, 32.0),
                 (0.0, 31.0, 1.2, 33.6)]
GARDEN_SUNDIAL = (13.0, 30.0)
GARDEN_BENCHES = [(8.0, 35.55, 180.0), (18.0, 35.55, 180.0),
                  (5.0, 23.4, 0.0), (-5.55, 25.0, -90.0)]
GARDEN_COLD_FRAMES = (19.5, 34.8, 23.5, 35.7, 0.5)

PAIR_HALF = 0.6
DOOR_CLEAR = 1.0
PERSON_TAGS = {"stand", "lean", "pair", "work", "window", "stage"}
PERSON_MARGIN = 0.2
HEADROOM = 1.9
BEDROOMS = ["master", "g1", "g2", "g3", "g4", "g5"]
OUTDOOR_PAIR_SETS = {"terrace": 2, "garden": 2}

OUTDOOR_VOLUME_HEIGHT = 4.0
TIMELINE = {"corr_w": "hall", "corr_e": "hall", "passage": "servhall",
            "backstairs": "servhall", "cellar_pass": "servhall"}

COLOURS = {
    "gb_floor": (0.78, 0.74, 0.66, 1), "gb_ground": (0.45, 0.55, 0.40, 1),
    "gb_ceiling": (0.70, 0.70, 0.72, 1), "gb_wall_ext": (0.22, 0.22, 0.26, 1),
    "gb_wall_int": (0.45, 0.30, 0.25, 1), "gb_nav": (0.2, 0.6, 1.0, 1),
    "gb_zone_public": (0.95, 0.75, 0.25, 1), "gb_zone_private": (0.80, 0.30, 0.45, 1),
    "gb_zone_service": (0.30, 0.55, 0.85, 1), "gb_zone_open": (0.45, 0.80, 0.45, 1),
    "gb_door": (0.42, 0.24, 0.12, 1), "gb_door_glazed": (0.55, 0.75, 0.85, 1),
    "gb_stair": (0.60, 0.52, 0.42, 1), "gb_balustrade": (0.30, 0.20, 0.12, 1),
    "gb_fireplace": (0.70, 0.66, 0.60, 1), "gb_firebox": (0.08, 0.07, 0.07, 1),
    "gb_brass": (0.80, 0.62, 0.20, 1), "gb_curtain": (0.45, 0.08, 0.12, 1),
    "gb_glass": (0.70, 0.88, 0.95, 1), "gb_bookcase": (0.35, 0.20, 0.10, 1),
    "gb_garden_wall": (0.55, 0.32, 0.25, 1), "gb_hedge": (0.18, 0.35, 0.16, 1),
    "gb_parapet": (0.78, 0.76, 0.70, 1), "gb_gravel": (0.72, 0.68, 0.58, 1),
    "gb_water": (0.20, 0.40, 0.50, 1),
    "gb_furniture": (0.55, 0.38, 0.22, 1), "gb_upholstery": (0.35, 0.50, 0.45, 1),
    "gb_rug": (0.50, 0.20, 0.20, 1),
}


class Spec:
    def __init__(self, path):
        with open(path) as f:
            self.d = json.load(f)
        m = self.d["metrics"]
        self.lz = m["level_z"]
        self.ceil = m["ceiling"]
        self.eaves = m["attic_eaves"]
        self.wall_t = m["walls"]
        self.openings = m["openings_wh"]
        self.rooms = {r["id"]: r for r in self.d["rooms"]}
        self.doors = self.d["doors"]
        dev = self.d["devices"]
        self.hall_void = tuple(dev["hall_void"])
        bs = tuple(dev["back_stairs"]["footprint"])
        ms = dev["main_stairs"]["footprint"]
        self.stairwell = (ms[0], MAIN_STAIR_ARRIVAL_Y, ms[2], ms[3])
        self.vertical_open = [(self.hall_void, "ground", "first"),
                              (self.stairwell, "ground", "first"),
                              (bs, "cellar", "ground"), (bs, "ground", "first"),
                              (bs, "first", "attic")]
        self.nav_exclude = [("ground", self.stairwell),
                            ("cellar", (bs[0], 11.0, bs[2], bs[3]))]
        self.window_exclude = [("ground", tuple(ms))]
        for lvl, axis, c, s_, into in FIREPLACES.values():
            r = (c - 0.5, s_ - 0.8, c + 0.5, s_ + 0.8) if axis == "x" else (s_ - 0.8, c - 0.5, s_ + 0.8, c + 0.5)
            self.window_exclude.append((lvl, r))
        x0, y0, x1, y1 = self.d["devices"]["dumbwaiter"]["shaft"]
        ht = DUMBWAITER_PART_T / 2
        self.shaft = (x0 - ht, y0 - ht, round(x1 + 0.1, 3), round(y1 + 0.1, 3))
        self.vertical_open.append((self.shaft, "ground", "first"))
        self.nav_exclude.append(("ground", self.shaft))
        st = next(d for d in self.doors if d["kind"] == "steps")
        sw = TERRACE_STEPS[0]
        self.steps_rect = (st["at"][0] - sw / 2, st["at"][1], st["at"][0] + sw / 2, st["at"][1] + 1.0)
        self.nav_exclude.append(("ground", self.steps_rect))
        px0, py0, px1, py1 = GARDEN_POND
        self.nav_exclude.append(("ground", (px0, py0 - 0.25, px1, py1 + 0.25)))
        self.runs = []
        self.boxes = defaultdict(list)
        for r in self.d["rooms"]:
            for b in r["boxes"]:
                rect = (b["x0"], b["y0"], b["x1"], b["y1"])
                self.boxes[b["level"]].append((r["id"], rect, "room"))
        self.boxes["first"].append(("hall", self.hall_void, "room"))
        self.boxes["attic"].append(("loft", MAIN_BLOCK, "envelope"))
        xs = [v for bl in self.boxes.values() for _, r, _ in bl for v in (r[0], r[2])]
        ys = [v for bl in self.boxes.values() for _, r, _ in bl for v in (r[1], r[3])]
        self.x0, self.y0 = min(xs), min(ys)
        self.nx = int(round((max(xs) - self.x0) / GRID))
        self.ny = int(round((max(ys) - self.y0) / GRID))
        self.owner = {lvl: self._rasterise(lvl) for lvl in LEVELS}
        self.door_run = {}
        self.windows_forced = []
        for f in self.d["features"]:
            w = f.get("acoustic", {}).get("window_at")
            if w:
                self.windows_forced.append(("ground", w[0], w[1], f["id"]))

    def _rasterise(self, lvl):
        grid = {}
        for i in range(self.nx):
            for j in range(self.ny):
                cx, cy = self.cell_centre(i, j)
                best = None
                for oid, (x0, y0, x1, y1), kind in self.boxes[lvl]:
                    if x0 < cx < x1 and y0 < cy < y1:
                        area = (x1 - x0) * (y1 - y0)
                        rank = (0 if kind == "room" else 1, area)
                        if best is None or rank < best[0]:
                            best = (rank, oid)
                if best:
                    grid[(i, j)] = best[1]
        return grid

    def cell_centre(self, i, j):
        return self.x0 + (i + 0.5) * GRID, self.y0 + (j + 0.5) * GRID

    def kind(self, oid):
        if oid is None:
            return "none"
        if oid == "loft":
            return "envelope"
        return "outdoor" if self.rooms[oid].get("outdoor") else "indoor"

    def coll_level(self, rid, lvl):
        return "grounds" if self.rooms[rid]["level"] == "grounds" else lvl

    def next_level(self, lvl):
        i = LEVELS.index(lvl)
        return LEVELS[i + 1] if i + 1 < len(LEVELS) else None

    def _open(self, lvl, x, y, up):
        for (x0, y0, x1, y1), lo, hi in self.vertical_open:
            if (lo if up else hi) == lvl and x0 < x < x1 and y0 < y < y1:
                return True
        return False

    def open_up(self, lvl, x, y):
        return self._open(lvl, x, y, True)

    def open_down(self, lvl, x, y):
        return self._open(lvl, x, y, False)

    def owner_at(self, lvl, x, y):
        i = int((x - self.x0) // GRID)
        j = int((y - self.y0) // GRID)
        return self.owner.get(lvl, {}).get((i, j))

    def floor_top(self, rid, lvl, x, y):
        if rid == "path":
            xa, xb, za, zb = PATH_RAMP
            t = min(1.0, max(0.0, (x - xa) / (xb - xa)))
            return za + (zb - za) * t
        return FLOOR_Z_OVERRIDE.get(rid, self.lz[lvl])

    def ceil_rel(self, rid, lvl, x, y):
        if (rid, lvl) in CEILING_OVERRIDE:
            return CEILING_OVERRIDE[(rid, lvl)]
        if lvl == "attic" and rid not in ("attic_corr", "backstairs"):
            corr = [r for o, r, _ in self.boxes["attic"] if o == "attic_corr"][0]
            box = [r for o, r, _ in self.boxes["attic"] if o == rid][0]
            hi, lo = self.ceil["attic"], self.eaves
            if box[3] == corr[1]:
                return lo + (hi - lo) * (y - box[1]) / (box[3] - box[1])
            if box[1] == corr[3]:
                return hi - (hi - lo) * (y - box[1]) / (box[3] - box[1])
        return self.ceil[lvl]

    def ceil_under(self, rid, lvl, x, y):
        return self.floor_top(rid, lvl, x, y) + self.ceil_rel(rid, lvl, x, y)

    def ceil_top(self, rid, lvl, x, y, cx, cy):
        nxt = self.next_level(lvl)
        if self.open_up(lvl, cx, cy):
            return self.lz[nxt] - SLAB
        under = self.ceil_under(rid, lvl, x, y)
        top = under + SLAB
        if nxt and self.kind(self.owner_at(nxt, cx, cy)) in ("indoor", "envelope"):
            top = max(top, self.lz[nxt] - SLAB)
        return top

    def ceil_above_flag(self, lvl, cx, cy):
        nxt = self.next_level(lvl)
        return bool(nxt and self.kind(self.owner_at(nxt, cx, cy)) in ("indoor", "envelope"))


class Run:
    def __init__(self, lvl, orient, c, s0, s1, a, b, wtype, t, bottom, top0, top1):
        self.lvl, self.orient, self.c = lvl, orient, c
        self.s0, self.s1 = s0, s1
        self.l0, self.l1 = s0, s1
        self.a, self.b, self.wtype, self.t = a, b, wtype, t
        self.bottom = bottom
        self.base_s, self.base_top = s0, top0
        self.slope = (top1 - top0) / (s1 - s0)
        self.openings = []

    def top(self, s):
        return self.base_top + self.slope * (s - self.base_s)

    def point(self, s):
        return (s, self.c) if self.orient == "x" else (self.c, s)

    def indoor_rooms(self, spec):
        return [o for o in (self.a, self.b) if spec.kind(o) == "indoor"]


def wall_type(spec, lvl, a, b):
    if a == b:
        return None
    ka, kb = spec.kind(a), spec.kind(b)
    if "indoor" not in (ka, kb):
        return None
    if (lvl, frozenset({a, b})) in OPEN_EDGES:
        return "open"
    if ka == kb == "indoor":
        return "interior"
    other = kb if ka == "indoor" else ka
    return "interior" if other == "envelope" else "exterior"


def raw_pieces(spec, lvl):
    own = spec.owner[lvl]
    pieces = []

    def side_top(oid, x, y, cx, cy):
        if spec.kind(oid) != "indoor":
            return None
        return spec.ceil_top(oid, lvl, x, y, cx, cy)

    def side_bottom(oid, x, y):
        if spec.kind(oid) != "indoor":
            return None
        return spec.floor_top(oid, lvl, x, y) - SLAB

    for i in range(spec.nx + 1):
        x = spec.x0 + i * GRID
        for j in range(spec.ny):
            a, b = own.get((i - 1, j)), own.get((i, j))
            wt = wall_type(spec, lvl, a, b)
            if not wt:
                continue
            ya, yb = spec.y0 + j * GRID, spec.y0 + (j + 1) * GRID
            cyc = (ya + yb) / 2
            tops0 = [t for t in (side_top(a, x, ya, x - GRID / 2, cyc), side_top(b, x, ya, x + GRID / 2, cyc)) if t is not None]
            tops1 = [t for t in (side_top(a, x, yb, x - GRID / 2, cyc), side_top(b, x, yb, x + GRID / 2, cyc)) if t is not None]
            bots = [v for v in (side_bottom(a, x, cyc), side_bottom(b, x, cyc)) if v is not None]
            pieces.append(dict(orient="y", c=x, s0=ya, s1=yb, a=a, b=b, wtype=wt,
                               bottom=min(bots), top0=max(tops0), top1=max(tops1)))
    for j in range(spec.ny + 1):
        y = spec.y0 + j * GRID
        for i in range(spec.nx):
            a, b = own.get((i, j - 1)), own.get((i, j))
            wt = wall_type(spec, lvl, a, b)
            if not wt:
                continue
            xa, xb = spec.x0 + i * GRID, spec.x0 + (i + 1) * GRID
            cxc = (xa + xb) / 2
            tops0 = [t for t in (side_top(a, xa, y, cxc, y - GRID / 2), side_top(b, xa, y, cxc, y + GRID / 2)) if t is not None]
            tops1 = [t for t in (side_top(a, xb, y, cxc, y - GRID / 2), side_top(b, xb, y, cxc, y + GRID / 2)) if t is not None]
            bots = [v for v in (side_bottom(a, cxc, y), side_bottom(b, cxc, y)) if v is not None]
            pieces.append(dict(orient="x", c=y, s0=xa, s1=xb, a=a, b=b, wtype=wt,
                               bottom=min(bots), top0=max(tops0), top1=max(tops1)))
    return pieces


def build_runs(spec):
    raw = {lvl: raw_pieces(spec, lvl) for lvl in LEVELS}
    for lvl in LEVELS:
        nxt = spec.next_level(lvl)
        if not nxt:
            continue
        above = {(p["orient"], round(p["c"], 3), round(p["s0"], 3)): p["bottom"] for p in raw[nxt]}
        for p in raw[lvl]:
            k = (p["orient"], round(p["c"], 3), round(p["s0"], 3))
            if k in above:
                p["top0"] = min(p["top0"], above[k])
                p["top1"] = min(p["top1"], above[k])
    runs = []
    for lvl in LEVELS:
        ps = sorted(raw[lvl], key=lambda p: (p["orient"], p["c"], p["s0"]))
        cur = None
        for p in ps:
            if cur and cur["orient"] == p["orient"] and abs(cur["c"] - p["c"]) < EPS \
                    and abs(cur["s1"] - p["s0"]) < EPS and cur["a"] == p["a"] and cur["b"] == p["b"] \
                    and abs(cur["bottom"] - p["bottom"]) < EPS and abs(cur["top1"] - p["top0"]) < 1e-4:
                slope_c = (cur["top1"] - cur["top0"]) / (cur["s1"] - cur["s0"])
                slope_p = (p["top1"] - p["top0"]) / (p["s1"] - p["s0"])
                if abs(slope_c - slope_p) < 1e-4:
                    cur["s1"], cur["top1"] = p["s1"], p["top1"]
                    continue
            if cur:
                runs.append((lvl, cur))
            cur = dict(p)
        if cur:
            runs.append((lvl, cur))
    out = []
    for lvl, p in runs:
        t = spec.wall_t["interior" if p["wtype"] == "open" else p["wtype"]]
        out.append(Run(lvl, p["orient"], p["c"], p["s0"], p["s1"], p["a"], p["b"],
                       p["wtype"], t, p["bottom"], p["top0"], p["top1"]))
    adjust_junctions(out)
    return out


def adjust_junctions(runs):
    ends = defaultdict(list)
    for r in runs:
        for which, s in ((0, r.l0), (1, r.l1)):
            x, y = r.point(s)
            ends[(r.lvl, round(x, 3), round(y, 3))].append((r, which))
    for key, lst in ends.items():
        xs = [r for r, _ in lst if r.orient == "x"]
        ys = [r for r, _ in lst if r.orient == "y"]
        for r, which in lst:
            sign = -1 if which == 0 else 1
            if r.orient == "x":
                if len(xs) > 1 or not ys:
                    continue
                d = max(o.t for o in ys) / 2
            else:
                if not xs:
                    continue
                d = -min(o.t for o in xs) / 2
            if which == 0:
                r.s0 += sign * d
            else:
                r.s1 += sign * d


def attach_openings(spec, runs, report):
    by_line = defaultdict(list)
    for r in runs:
        by_line[(r.lvl, r.orient, round(r.c, 3))].append(r)
    for d in spec.doors:
        if d["kind"] in SKIP_OPENING_KINDS:
            report["skipped_openings"].append(d["id"])
            continue
        x, y = d["at"]
        lvl = d["level"]
        want = {d["a"], d["b"]}
        found = None
        for orient, c, s in (("y", x, y), ("x", y, x)):
            for r in by_line.get((lvl, orient, round(c, 3)), []):
                if {r.a, r.b} == want and r.l0 - EPS <= s <= r.l1 + EPS:
                    found = (r, s)
        if not found:
            report["errors"].append(f"door {d['id']}: no wall run between {sorted(want)} at {d['at']} on {lvl}")
            continue
        r, s = found
        spec.door_run[d["id"]] = r
        w, h = spec.openings[d["kind"]]
        w = DOOR_WIDTH_OVERRIDE.get(d["id"], w)
        floor = spec.floor_top(d["a"], lvl, x, y)
        o0, o1 = s - w / 2, s + w / 2
        if o0 < r.s0 - EPS or o1 > r.s1 + EPS:
            report["warnings"].append(f"door {d['id']}: opening {o0:.2f}-{o1:.2f} clipped to wall {r.s0:.2f}-{r.s1:.2f}")
            o0, o1 = max(o0, r.s0), min(o1, r.s1)
        if d["kind"] == "hatch":
            r.openings.append((o0, o1, floor + HATCH_SILL, floor + HATCH_SILL + h, "door:" + d["id"]))
        else:
            r.openings.append((o0, o1, None, floor + h, "door:" + d["id"]))
        report["doors_cut"] += 1


def attach_open_edges(spec, runs):
    for r in runs:
        if r.wtype != "open":
            continue
        mx, my = r.point((r.l0 + r.l1) / 2)
        head = min(spec.ceil_under(o, r.lvl, mx, my) for o in r.indoor_rooms(spec))
        r.openings.append((r.s0, r.s1, None, head, "open"))


def run_at(spec, lvl, axis, c, s):
    orient = "y" if axis == "x" else "x"
    for r in spec.runs:
        if r.lvl == lvl and r.orient == orient and abs(r.c - c) < EPS and r.l0 - EPS <= s <= r.l1 + EPS:
            return r
    return None


def attach_device_openings(spec, runs, report):
    spec.runs = runs
    w, sill, head = DUMBWAITER_HATCH
    for stop, (lvl, axis, c, s, into, room) in DUMBWAITER_STOPS.items():
        r = run_at(spec, lvl, axis, c, s)
        if r is None:
            continue
        floor = spec.lz[lvl]
        r.openings.append((s - w / 2, s + w / 2, floor + sill, floor + head, f"dumbwaiter:{stop}"))


def attach_windows(spec, runs, report):
    for r in runs:
        if r.wtype != "exterior" or r.lvl in NO_WINDOW_LEVELS:
            continue
        rooms = r.indoor_rooms(spec)
        if len(rooms) != 1 or rooms[0] in NO_WINDOW_ROOMS:
            continue
        rid = rooms[0]
        clvl = spec.coll_level(rid, r.lvl)
        if rid in GLAZED_ROOMS:
            w, sill, head, spacing = GLAZED_ROOMS[rid]
        else:
            (w, sill, head), spacing = WINDOW[clvl], WINDOW_SPACING
        mx, my = r.point((r.l0 + r.l1) / 2)
        floor = spec.floor_top(rid, r.lvl, mx, my)
        forced = []
        for flvl, fx, fy, fid in spec.windows_forced:
            fs, fc = (fx, fy) if r.orient == "x" else (fy, fx)
            if flvl == r.lvl and abs(fc - r.c) < EPS and r.l0 < fs < r.l1:
                forced.append((fs, fid))
        centres = []
        if forced:
            centres = [(fs, "window:" + fid) for fs, fid in forced]
        else:
            free = [(r.l0 + WINDOW_MARGIN, r.l1 - WINDOW_MARGIN)]
            blocks = [(o[0], o[1]) for o in r.openings]
            for lvl, (ex0, ey0, ex1, ey1) in spec.window_exclude:
                if lvl != r.lvl:
                    continue
                if r.orient == "x" and ey0 - EPS <= r.c <= ey1 + EPS:
                    blocks.append((ex0, ex1))
                elif r.orient == "y" and ex0 - EPS <= r.c <= ex1 + EPS:
                    blocks.append((ey0, ey1))
            for o0, o1 in blocks:
                nxt = []
                for a, b in free:
                    lo, hi = o0 - WINDOW_MARGIN, o1 + WINDOW_MARGIN
                    if hi <= a or lo >= b:
                        nxt.append((a, b))
                        continue
                    if lo > a:
                        nxt.append((a, lo))
                    if hi < b:
                        nxt.append((hi, b))
                free = nxt
            for a, b in free:
                length = b - a
                if length < w:
                    continue
                n = max(1, int(length // spacing))
                for k in range(n):
                    centres.append((a + length * (k + 0.5) / n, "window"))
        for s, tag in centres:
            zhi = floor + head
            if zhi > r.top(s) - 0.1:
                report["warnings"].append(f"window on {rid} at s={s:.2f} too tall for wall, skipped")
                continue
            r.openings.append((s - w / 2, s + w / 2, floor + sill, zhi, tag))
            report["windows"] += 1


class MeshAccum:
    def __init__(self):
        self.verts, self.faces = [], []

    def prism(self, xy, zb, zt):
        n = len(self.verts)
        for (x, y), z in zip(xy, zb):
            self.verts.append((x, y, z))
        for (x, y), z in zip(xy, zt):
            self.verts.append((x, y, z))
        self.faces += [(n, n + 3, n + 2, n + 1), (n + 4, n + 5, n + 6, n + 7),
                       (n, n + 1, n + 5, n + 4), (n + 1, n + 2, n + 6, n + 5),
                       (n + 2, n + 3, n + 7, n + 6), (n + 3, n, n + 4, n + 7)]

    def quad(self, xy, z):
        n = len(self.verts)
        for (x, y), zz in zip(xy, z):
            self.verts.append((x, y, zz))
        self.faces.append((n, n + 1, n + 2, n + 3))

    def empty(self):
        return not self.faces


def material(name):
    m = bpy.data.materials.get(name)
    if m is None:
        m = bpy.data.materials.new(name)
    m.diffuse_color = COLOURS[name]
    return m


def collection(name):
    c = bpy.data.collections.get(name)
    if c is None:
        c = bpy.data.collections.new(name)
        bpy.context.scene.collection.children.link(c)
    return c


def make_object(name, acc, coll, mat, props, recalc=True):
    me = bpy.data.meshes.new(name)
    me.from_pydata(acc.verts, [], acc.faces)
    if recalc:
        bm = bmesh.new()
        bm.from_mesh(me)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        bm.to_mesh(me)
        bm.free()
    me.update()
    me.materials.append(mat)
    ob = bpy.data.objects.new(name, me)
    coll.objects.link(ob)
    ob["generated"] = True
    for k, v in props.items():
        ob[k] = v
    return ob


def rect_xy(x0, y0, x1, y1):
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]


def merge_rects(cells):
    cells = set(cells)
    rects = []
    for j, i in sorted((j, i) for i, j in cells):
        if (i, j) not in cells:
            continue
        i1 = i
        while (i1 + 1, j) in cells:
            i1 += 1
        j1 = j
        while all((k, j1 + 1) in cells for k in range(i, i1 + 1)):
            j1 += 1
        for jj in range(j, j1 + 1):
            for ii in range(i, i1 + 1):
                cells.discard((ii, jj))
        rects.append((i, j, i1 + 1, j1 + 1))
    return rects


def clear_generated():
    n = 0
    for ob in list(bpy.data.objects):
        if ob.get("generated"):
            data = ob.data
            bpy.data.objects.remove(ob)
            if isinstance(data, bpy.types.Mesh) and data.users == 0:
                bpy.data.meshes.remove(data)
            n += 1
    return n


def build_slabs(spec, report):
    for lvl in LEVELS:
        floors, ceilings = defaultdict(set), defaultdict(set)
        for (i, j), oid in spec.owner[lvl].items():
            k = spec.kind(oid)
            if k not in ("indoor", "outdoor"):
                continue
            cx, cy = spec.cell_centre(i, j)
            seg = 0 if oid != "path" or cx < PATH_RAMP[1] else 1
            if not spec.open_down(lvl, cx, cy):
                floors[(oid, seg)].add((i, j))
            if k == "indoor" and not spec.open_up(lvl, cx, cy):
                ceilings[(oid, spec.ceil_above_flag(lvl, cx, cy))].add((i, j))

        per_obj = defaultdict(MeshAccum)
        for (oid, _), cells in floors.items():
            outdoor = spec.kind(oid) == "outdoor"
            for i0, j0, i1, j1 in merge_rects(cells):
                x0, y0 = spec.x0 + i0 * GRID, spec.y0 + j0 * GRID
                x1, y1 = spec.x0 + i1 * GRID, spec.y0 + j1 * GRID
                xy = rect_xy(x0, y0, x1, y1)
                zt = [spec.floor_top(oid, lvl, x, y) for x, y in xy]
                zb = [OUTDOOR_SLAB_BOTTOM] * 4 if outdoor else [z - SLAB for z in zt]
                per_obj[("floor", oid)].prism(xy, zb, zt)
            nav_cells = {c for c in cells if not any(
                lv == lvl and r[0] < spec.cell_centre(*c)[0] < r[2] and r[1] < spec.cell_centre(*c)[1] < r[3]
                for lv, r in spec.nav_exclude)}
            for i0, j0, i1, j1 in merge_rects(nav_cells):
                xy = rect_xy(spec.x0 + i0 * GRID, spec.y0 + j0 * GRID,
                             spec.x0 + i1 * GRID, spec.y0 + j1 * GRID)
                per_obj[("nav", oid)].quad(xy, [spec.floor_top(oid, lvl, x, y) for x, y in xy])
        for (oid, _), cells in ceilings.items():
            for i0, j0, i1, j1 in merge_rects(cells):
                x0, y0 = spec.x0 + i0 * GRID, spec.y0 + j0 * GRID
                x1, y1 = spec.x0 + i1 * GRID, spec.y0 + j1 * GRID
                xy = rect_xy(x0, y0, x1, y1)
                cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
                zb = [spec.ceil_under(oid, lvl, x, y) for x, y in xy]
                zt = [spec.ceil_top(oid, lvl, x, y, cx, cy) for x, y in xy]
                per_obj[("ceiling", oid)].prism(xy, zb, zt)

        for (what, oid), acc in per_obj.items():
            clvl = spec.coll_level(oid, lvl)
            outdoor = spec.kind(oid) == "outdoor"
            name = f"{what}.{clvl}.{oid}" if clvl == "grounds" else f"{what}.{lvl}.{oid}"
            if what == "nav":
                make_object(name, acc, collection(COLLECTIONS["nav"]), material("gb_nav"),
                            {"gb_kind": "nav", "level": lvl, "room": oid}, recalc=False)
            else:
                mat = material("gb_ground" if outdoor else ("gb_floor" if what == "floor" else "gb_ceiling"))
                make_object(name, acc, collection(COLLECTIONS[clvl]), mat,
                            {"gb_kind": what, "level": lvl, "room": oid})
            report["objects"][what] += 1


def wall_pieces(run):
    out = []
    cursor = run.s0
    for o0, o1, zlo, zhi, _ in sorted(run.openings):
        if o0 > cursor + 1e-4:
            out.append((cursor, o0, run.bottom, run.bottom, run.top(cursor), run.top(o0)))
        if zlo is not None and zlo > run.bottom + 1e-4:
            out.append((o0, o1, run.bottom, run.bottom, zlo, zlo))
        if run.top(o0) > zhi + 1e-4 or run.top(o1) > zhi + 1e-4:
            out.append((o0, o1, zhi, zhi, run.top(o0), run.top(o1)))
        cursor = max(cursor, o1)
    if run.s1 > cursor + 1e-4:
        out.append((cursor, run.s1, run.bottom, run.bottom, run.top(cursor), run.top(run.s1)))
    return out


def build_walls(spec, report, make=True):
    runs = build_runs(spec)
    attach_openings(spec, runs, report)
    attach_open_edges(spec, runs)
    attach_device_openings(spec, runs, report)
    attach_windows(spec, runs, report)
    spec.runs = runs
    if not make:
        return runs
    counters = defaultdict(int)
    for r in sorted(runs, key=lambda r: (LEVELS.index(r.lvl), r.orient, r.c, r.l0)):
        rooms = r.indoor_rooms(spec)
        clvl = r.lvl
        if all(spec.rooms[x]["level"] == "grounds" for x in rooms):
            clvl = "grounds"
        acc = MeshAccum()
        h = r.t / 2
        for s0, s1, zb0, zb1, zt0, zt1 in wall_pieces(r):
            if s1 - s0 < 1e-4:
                continue
            if r.orient == "x":
                xy = [(s0, r.c - h), (s1, r.c - h), (s1, r.c + h), (s0, r.c + h)]
                zb, zt = [zb0, zb1, zb1, zb0], [zt0, zt1, zt1, zt0]
            else:
                xy = [(r.c - h, s0), (r.c + h, s0), (r.c + h, s1), (r.c - h, s1)]
                zb, zt = [zb0, zb0, zb1, zb1], [zt0, zt0, zt1, zt1]
            acc.prism(xy, zb, zt)
        if acc.empty():
            continue
        counters[clvl] += 1
        name = f"wall.{clvl}.{counters[clvl]:03d}"
        doors = [t[5:] for *_, t in r.openings if t.startswith("door:")]
        make_object(name, acc, collection(COLLECTIONS[clvl]),
                    material("gb_wall_ext" if r.wtype == "exterior" else "gb_wall_int"),
                    {"gb_kind": "wall", "kind": "header" if r.wtype == "open" else r.wtype,
                     "level": r.lvl,
                     "rooms": ",".join(x for x in (r.a, r.b) if x),
                     "doors": ",".join(doors),
                     "windows": sum(1 for *_, t in r.openings if t.startswith("window"))})
        report["objects"]["wall"] += 1
    report["runs"] = len(runs)
    for r in runs:
        ops = sorted(r.openings)
        for (a0, a1, *_ , ta), (b0, b1, *_, tb) in zip(ops, ops[1:]):
            if b0 < a1 - 1e-4:
                report["errors"].append(f"overlapping openings {ta} and {tb} on {r.lvl} {r.orient}={r.c}")
    return runs


def build_volumes(spec, report):
    shells = defaultdict(MeshAccum)
    bboxes = defaultdict(list)
    for lvl in LEVELS:
        for rid, (x0, y0, x1, y1), kind in spec.boxes[lvl]:
            if kind != "room":
                continue
            outdoor = spec.kind(rid) == "outdoor"
            i0, i1 = round((x0 - spec.x0) / GRID), round((x1 - spec.x0) / GRID)
            j0, j1 = round((y0 - spec.y0) / GRID), round((y1 - spec.y0) / GRID)
            groups = defaultdict(set)
            for i in range(i0, i1):
                for j in range(j0, j1):
                    cx, cy = spec.cell_centre(i, j)
                    up = (not outdoor) and spec.open_up(lvl, cx, cy)
                    seg = 0 if rid != "path" or cx < PATH_RAMP[1] else 1
                    groups[(up, seg)].add((i, j))
            for (up, _), cells in groups.items():
                for a0, b0, a1, b1 in merge_rects(cells):
                    rx0, ry0 = spec.x0 + a0 * GRID, spec.y0 + b0 * GRID
                    rx1, ry1 = spec.x0 + a1 * GRID, spec.y0 + b1 * GRID
                    xy = rect_xy(rx0, ry0, rx1, ry1)
                    zb = [spec.floor_top(rid, lvl, x, y) for x, y in xy]
                    if outdoor:
                        zt = [z + OUTDOOR_VOLUME_HEIGHT for z in zb]
                    elif up:
                        zt = [spec.lz[spec.next_level(lvl)]] * 4
                    else:
                        zt = [spec.ceil_under(rid, lvl, x, y) for x, y in xy]
                    shells[rid].prism(xy, zb, zt)
                    bboxes[rid].append((rx0, ry0, min(zb), rx1, ry1, max(zt)))

    nested = {(r["id"], r["nested_in"]) for r in spec.d["rooms"] if r.get("nested_in")}
    for rid, room in spec.rooms.items():
        acc = shells.get(rid)
        if not acc or acc.empty():
            report["errors"].append(f"room {rid}: no volume")
            continue
        clvl = "grounds" if room["level"] == "grounds" else room["level"]
        ob = make_object(f"room.{rid}", acc, collection(COLLECTIONS[clvl]),
                         material(f"gb_zone_{room['zone']}"),
                         {"gb_kind": "room", "room": rid, "zone": room["zone"],
                          "level": room["level"], "timeline": TIMELINE.get(rid, rid),
                          "outdoor": bool(room.get("outdoor"))})
        ob.display_type = "WIRE"
        ob.hide_render = True
        report["objects"]["room"] += 1

    ids = sorted(bboxes)
    for n, a in enumerate(ids):
        for b in ids[n + 1:]:
            if (a, b) in nested or (b, a) in nested:
                continue
            for p in bboxes[a]:
                for q in bboxes[b]:
                    dx = min(p[3], q[3]) - max(p[0], q[0])
                    dy = min(p[4], q[4]) - max(p[1], q[1])
                    dz = min(p[5], q[5]) - max(p[2], q[2])
                    if dx > 1e-4 and dy > 1e-4 and dz > 1e-4:
                        report["errors"].append(f"volumes {a} and {b} overlap")
    report["rooms_expected"] = len(spec.rooms)


def door_geometry(spec, d):
    x, y = d["at"]
    lvl = d["level"]
    a_boxes = [r for o, r, k in spec.boxes[lvl] if o == d["a"]]
    b_boxes = [r for o, r, k in spec.boxes[lvl] if o == d["b"]]
    for orient in ("y", "x"):
        c, s = (x, y) if orient == "y" else (y, x)
        for ra in a_boxes:
            lo, hi = (ra[1], ra[3]) if orient == "y" else (ra[0], ra[2])
            near, far = (ra[0], ra[2]) if orient == "y" else (ra[1], ra[3])
            if not (lo - EPS <= s <= hi + EPS):
                continue
            if abs(near - c) < EPS or abs(far - c) < EPS:
                centre = (near + far) / 2
                nb = [rb for rb in b_boxes
                      if abs(((rb[0] if orient == "y" else rb[1]) - c)) < EPS
                      or abs(((rb[2] if orient == "y" else rb[3]) - c)) < EPS
                      or (rb[0] < x < rb[2] and rb[1] < y < rb[3])]
                if not nb:
                    continue
                sign = 1.0 if centre > c else -1.0
                normal = (sign, 0.0) if orient == "y" else (0.0, sign)
                return orient, c, s, normal
    return None


def build_doors(spec, report):
    import math
    from mathutils import Euler
    for d in spec.doors:
        geo = door_geometry(spec, d)
        if not geo:
            report["errors"].append(f"door {d['id']}: cannot place on a shared edge")
            continue
        orient, c, s, (nx, ny) = geo
        x, y = d["at"]
        lvl = d["level"]
        kind = d["kind"]
        floor = spec.floor_top(d["a"], lvl, x, y)
        both_grounds = all(spec.rooms[r]["level"] == "grounds" for r in (d["a"], d["b"]))
        coll = collection(COLLECTIONS["grounds" if both_grounds else lvl])
        w, h = spec.openings.get(kind, (1.0, 2.1))
        w = DOOR_WIDTH_OVERRIDE.get(d["id"], w)

        emp = bpy.data.objects.new(f"door.{d['id']}", None)
        emp.empty_display_type = "ARROWS"
        emp.empty_display_size = 0.5
        emp.location = (x, y, floor)
        emp.rotation_euler = Euler((0, 0, math.atan2(ny, nx) - math.pi / 2))
        coll.objects.link(emp)
        props = {"generated": True, "gb_kind": "door", "kind": kind, "zone": d["zone"],
                 "default": d["default"], "key": d.get("key", ""), "feature": d.get("feature", ""),
                 "a": d["a"], "b": d["b"], "level": lvl, "width": w, "height": h,
                 "leaves": 0 if kind in NO_LEAF_KINDS else (2 if kind in TWO_LEAF_KINDS else 1)}
        if d.get("note"):
            props["note"] = d["note"]
        for k, v in props.items():
            emp[k] = v
        report["objects"]["door"] += 1
        if kind in NO_LEAF_KINDS:
            continue

        run = spec.door_run.get(d["id"])
        t = run.t if run else (GARDEN_WALL[0] if kind == "gate" else FREE_WALL_T)
        zlo = floor + (HATCH_SILL if kind == "hatch" else 0.0)
        sdir = (1.0, 0.0) if orient == "x" else (0.0, 1.0)
        offset = t / 2 - LEAF_T / 2
        two = kind in TWO_LEAF_KINDS
        leaf_w = (w - 3 * LEAF_GAP) / 2 if two else w - 2 * LEAF_GAP
        hinges = [(s - w / 2 + LEAF_GAP, 1.0)]
        if two:
            hinges.append((s + w / 2 - LEAF_GAP, -1.0))
        mat = material("gb_door_glazed" if kind in GLAZED_KINDS else "gb_door")
        for n, (hs, along) in enumerate(hinges, start=1):
            hx = (hs if orient == "x" else c) + nx * offset
            hy = (c if orient == "x" else hs) + ny * offset
            dx, dy = sdir[0] * along, sdir[1] * along
            acc = MeshAccum()
            acc.prism(rect_xy(0.0, -LEAF_T / 2, leaf_w, LEAF_T / 2), [LEAF_GAP] * 4, [h - LEAF_GAP] * 4)
            name = f"doorleaf.{d['id']}" + ("" if n == 1 else f".{n}")
            left = (-dy, dx)
            swing = 1 if left[0] * nx + left[1] * ny > 0 else -1
            ob = make_object(name, acc, coll, mat,
                             {"gb_kind": "doorleaf", "door": d["id"], "level": lvl, "leaf": n,
                              "swing": swing, "open_angle": OPEN_ANGLE,
                              "double_acting": kind in DOUBLE_ACTING_KINDS})
            ob.location = (hx, hy, zlo)
            ob.rotation_euler = Euler((0, 0, math.atan2(dy, dx)))
            report["objects"]["doorleaf"] += 1


def stair_flight(acc, lane, s0, dirn, n, z0, r, g, solid_to=None):
    xa, xb = lane
    for k in range(1, n):
        a, b = s0 + dirn * (k - 1) * g, s0 + dirn * k * g
        top = z0 + k * r
        bot = solid_to if solid_to is not None else z0 + (k - 1) * r - STAIR_WAIST
        acc.prism(rect_xy(xa, min(a, b), xb, max(a, b)), [bot] * 4, [top] * 4)


def stair_ramp(acc, lane, s0, dirn, n, z0, r, g):
    xa, xb = lane
    ya, za = s0 - dirn * g, z0
    yb, zb = s0 + dirn * (n - 1) * g, z0 + n * r
    lo, hi = (ya, za), (yb, zb)
    if ya > yb:
        lo, hi = hi, lo
    acc.quad(rect_xy(xa, lo[0], xb, hi[0]), [lo[1], lo[1], hi[1], hi[1]])


def bar_y(acc, x0, x1, y0, y1, zb0, zb1, h):
    acc.prism(rect_xy(x0, y0, x1, y1), [zb0, zb0, zb1, zb1], [zb0 + h, zb0 + h, zb1 + h, zb1 + h])


def bar_x(acc, x0, x1, y0, y1, zb, h):
    acc.prism(rect_xy(x0, y0, x1, y1), [zb] * 4, [zb + h] * 4)


def build_stairs(spec, report):
    dev = spec.d["devices"]
    lz = spec.lz
    ext, inner = spec.wall_t["exterior"] / 2, spec.wall_t["interior"] / 2
    objs = []

    ms = dev["main_stairs"]
    fx0, fy0, fx1, fy1 = ms["footprint"]
    g, w = ms["going"], ms["width"]
    H = lz["first"] - lz["ground"]
    N = round(H / ms["rise"])
    r = H / N
    n1, n2 = (N + 1) // 2, N // 2
    arrive = MAIN_STAIR_ARRIVAL_Y
    land_s = arrive + (n2 - 1) * g
    start = land_s - (n1 - 1) * g
    north = fy1 - ext
    east = fx1 - inner
    lane_w, lane_e = (fx0, fx0 + w), (east - w, east)
    z_land = n1 * r
    if start < fy0 or north - land_s < w:
        report["errors"].append("main stair does not fit its footprint")
    vis, nav1, navl, nav2 = MeshAccum(), MeshAccum(), MeshAccum(), MeshAccum()
    stair_flight(vis, lane_w, start, +1, n1, 0.0, r, g, solid_to=0.0)
    vis.prism(rect_xy(lane_w[0], land_s, lane_e[1], north), [0.0] * 4, [z_land] * 4)
    stair_flight(vis, lane_e, land_s, -1, n2, z_land, r, g, solid_to=0.0)
    stair_ramp(nav1, lane_w, start, +1, n1, 0.0, r, g)
    navl.quad(rect_xy(lane_w[0], land_s, lane_e[1], north), [z_land] * 4)
    stair_ramp(nav2, lane_e, land_s, -1, n2, z_land, r, g)
    sp = {"room": "hall", "stair": "main", "risers": N, "rise": round(r, 4), "going": g}
    objs.append(("stair.main", vis, "ground", "gb_stair", dict(sp, level="ground")))
    for nm, acc, extra in (("flight1", nav1, {"stairs": True}), ("landing", navl, {"stairs": False}),
                           ("flight2", nav2, {"stairs": True})):
        objs.append((f"nav.stair.main.{nm}", acc, "nav", "gb_nav", dict(sp, level="ground", **extra)))

    bal = MeshAccum()
    line1 = lambda y: r + (y - start) / g * r
    line2 = lambda y: z_land + r + (land_s - y) / g * r
    hb = BAL_H_STAIR + 0.2
    bar_y(bal, lane_w[0], lane_w[0] + BAL_T, start, land_s, line1(start) - 0.2, line1(land_s) - 0.2, hb)
    bar_y(bal, lane_w[0], lane_w[0] + BAL_T, land_s, north, z_land - 0.2, z_land - 0.2, hb)
    bar_y(bal, lane_w[1] - BAL_T, lane_w[1], start, land_s, line1(start) - 0.2, line1(land_s) - 0.2, hb)
    bar_y(bal, lane_e[0], lane_e[0] + BAL_T, arrive, land_s, line2(arrive) - 0.2, line2(land_s) - 0.2, hb)
    bar_x(bal, lane_w[1] - BAL_T, lane_e[0] + BAL_T, land_s, land_s + BAL_T, z_land - 0.2, hb)
    objs.append(("balustrade.main_stair", bal, "ground", "gb_balustrade",
                 {"room": "hall", "level": "ground"}))
    well = MeshAccum()
    zf = lz["first"]
    bar_x(well, fx0 - BAL_T, lane_e[0], arrive - BAL_T, arrive, zf, BAL_H_FLOOR)
    bar_y(well, fx0 - BAL_T, fx0, arrive, north, zf, zf, BAL_H_FLOOR)
    objs.append(("balustrade.main_stairwell", well, "first", "gb_balustrade",
                 {"room": "gallery", "level": "first"}))

    hv = spec.hall_void
    over = MeshAccum()
    bar_x(over, hv[0] + inner, hv[2] - inner, hv[3], hv[3] + BAL_T + 0.02, zf, BAL_H_FLOOR)
    objs.append(("balustrade.hall_gallery", over, "first", "gb_balustrade",
                 {"room": "gallery", "level": "first"}))

    bs = dev["back_stairs"]
    bx0, by0, bx1, by1 = bs["footprint"]
    g, w = bs["going"], bs["width"]
    lane_w = (bx0 + ext, bx0 + ext + w)
    lane_e = (bx1 - ext - w, bx1 - ext)
    land_y = by0 + inner + w
    north = by1 - ext
    storeys = [("cellar", "ground"), ("ground", "first"), ("first", "attic")]
    for lo, hi in storeys:
        z0 = lz[lo]
        H = lz[hi] - z0
        N = round(H / bs["rise"])
        r = H / N
        if N % 2:
            report["errors"].append(f"back stair {lo}->{hi}: odd riser count {N}")
        na = nb = N // 2
        half_y = land_y + (na - 1) * g
        z_half = z0 + na * r
        if north - half_y < w:
            report["errors"].append(f"back stair {lo}->{hi}: half-landing too shallow")
        solid = z0 if lo == "cellar" else None
        vis, na_, nh, nb_ = MeshAccum(), MeshAccum(), MeshAccum(), MeshAccum()
        stair_flight(vis, lane_w, land_y, +1, na, z0, r, g, solid_to=solid)
        vis.prism(rect_xy(lane_w[0], half_y, lane_e[1], north),
                  [z0 if solid is not None else z_half - SLAB] * 4, [z_half] * 4)
        stair_flight(vis, lane_e, half_y, -1, nb, z_half, r, g, solid_to=solid)
        top = lz[hi] + BAL_H_FLOOR if hi == "attic" else lz[hi] - SLAB
        vis.prism(rect_xy(lane_w[1], land_y, lane_e[0], half_y), [z0 - SLAB] * 4, [top] * 4)
        stair_ramp(na_, lane_w, land_y, +1, na, z0, r, g)
        nh.quad(rect_xy(lane_w[0], half_y, lane_e[1], north), [z_half] * 4)
        stair_ramp(nb_, lane_e, half_y, -1, nb, z_half, r, g)
        sp = {"room": "backstairs", "stair": "back", "risers": N, "rise": round(r, 4), "going": g}
        objs.append((f"stair.back.{lo}", vis, lo, "gb_stair", dict(sp, level=lo)))
        objs.append((f"nav.stair.back.{lo}.a", na_, "nav", "gb_nav", dict(sp, level=lo, stairs=True)))
        objs.append((f"nav.stair.back.{lo}.half", nh, "nav", "gb_nav", dict(sp, level=lo, stairs=False)))
        objs.append((f"nav.stair.back.{lo}.b", nb_, "nav", "gb_nav", dict(sp, level=lo, stairs=True)))
        landing, nl = MeshAccum(), MeshAccum()
        landing.prism(rect_xy(bx0, by0, bx1, land_y), [lz[hi] - SLAB] * 4, [lz[hi]] * 4)
        nl.quad(rect_xy(bx0, by0, bx1, land_y), [lz[hi]] * 4)
        objs.append((f"landing.back.{hi}", landing, hi, "gb_floor", {"room": "backstairs", "level": hi}))
        objs.append((f"nav.landing.back.{hi}", nl, "nav", "gb_nav",
                     {"room": "backstairs", "level": hi, "stairs": False}))
    att = MeshAccum()
    bar_x(att, lane_w[0], lane_w[1], land_y - BAL_T, land_y, lz["attic"], BAL_H_FLOOR)
    objs.append(("balustrade.back_attic", att, "attic", "gb_balustrade",
                 {"room": "backstairs", "level": "attic"}))

    for name, acc, cl, mat, props in objs:
        kind = name.split(".")[0]
        is_nav = cl == "nav"
        make_object(name, acc, collection(COLLECTIONS[cl]), material(mat),
                    dict(props, gb_kind="nav" if is_nav else kind), recalc=not is_nav)
        report["objects"]["nav" if is_nav else kind] += 1


def drive_state(ob, fid, state):
    scene = bpy.context.scene
    for path in ("hide_viewport", "hide_render"):
        ob.driver_remove(path)
        drv = ob.driver_add(path).driver
        drv.type = "SCRIPTED"
        var = drv.variables.new()
        var.name = "v"
        var.type = "SINGLE_PROP"
        var.targets[0].id_type = "SCENE"
        var.targets[0].id = scene
        var.targets[0].data_path = f'["{fid}"]'
        drv.expression = "1 - v" if state == "on" else "v"


def wall_rect(axis, u0, u1, v0, v1):
    u0, u1 = sorted((u0, u1))
    v0, v1 = sorted((v0, v1))
    return rect_xy(v0, u0, v1, u1) if axis == "x" else rect_xy(u0, v0, u1, v1)


def wall_face(spec, lvl, axis, c, s, into):
    r = run_at(spec, lvl, axis, c, s)
    return c + into * (r.t / 2 if r else spec.wall_t["interior"] / 2)


def fireplace(acc_breast, acc_box, axis, face, s, z0, height, into):
    yb = face + into * 0.4
    acc_breast.prism(wall_rect(axis, s - 0.8, s + 0.8, face, yb), [z0] * 4, [z0 + height] * 4)
    acc_breast.prism(wall_rect(axis, s - 0.75, s + 0.75, yb, yb + into * 0.5), [z0] * 4, [z0 + 0.05] * 4)
    acc_breast.prism(wall_rect(axis, s - 0.9, s + 0.9, yb, yb + into * 0.15), [z0 + 1.1] * 4, [z0 + 1.16] * 4)
    acc_box.prism(wall_rect(axis, s - 0.4, s + 0.4, yb, yb + into * 0.01), [z0] * 4, [z0 + 0.8] * 4)


def build_fireplace(spec, room, acc_b, acc_f):
    lvl, axis, c, s, into = FIREPLACES[room]
    face = wall_face(spec, lvl, axis, c, s, into)
    px, py = (face, s) if axis == "x" else (s, face)
    fireplace(acc_b, acc_f, axis, face, s, spec.lz[lvl], spec.ceil_rel(room, lvl, px, py), into)


def build_features(spec, report):
    import math
    from mathutils import Euler, Matrix
    scene = bpy.context.scene
    feats = {f["id"]: f for f in spec.d["features"]}
    root = collection(COLLECTIONS["features"])
    fcoll = {}
    for fid, f in feats.items():
        c = bpy.data.collections.get(fid)
        if c is None:
            c = bpy.data.collections.new(fid)
            root.children.link(c)
        fcoll[fid] = c
        if fid not in scene.keys():
            scene[fid] = True
        ui = scene.id_properties_ui(fid)
        ui.update(description=f"{f['name']}. Off: {f['off']}")

    def link(ob, fid, state="both"):
        if ob.name not in fcoll[fid].objects:
            fcoll[fid].objects.link(ob)
        ob["feature"] = fid
        ob["feature_state"] = state
        if state in ("on", "off"):
            drive_state(ob, fid, state)

    def new(name, acc, lvl, mat, fid, state, props=None):
        clvl = lvl
        ob = make_object(name, acc, collection(COLLECTIONS[clvl]), material(mat),
                         dict({"gb_kind": "feature", "level": lvl if lvl != "grounds" else "ground"},
                              **(props or {})))
        link(ob, fid, state)
        report["objects"]["feature"] += 1
        return ob

    def acoustic(fid, src, dst, radius, two_way):
        for end, (room, lvl, xyz) in (("src", src), ("dst", dst)):
            emp = bpy.data.objects.new(f"acoustic.{fid}.{end}", None)
            emp.empty_display_type = "SPHERE" if end == "src" else "SINGLE_ARROW"
            emp.empty_display_size = radius if end == "src" else 0.4
            emp.location = xyz
            collection(COLLECTIONS[spec.coll_level(room, lvl)]).objects.link(emp)
            emp["generated"] = True
            emp["gb_kind"] = "acoustic"
            emp["room"] = room
            emp["level"] = lvl
            emp["two_way"] = two_way
            if end == "src":
                emp["radius"] = radius
            link(emp, fid, "on")
            report["objects"]["acoustic"] += 1

    def floor_xyz(room, lvl, xy, dz=EAR):
        return (xy[0], xy[1], spec.floor_top(room, lvl, xy[0], xy[1]) + dz)

    for d in spec.doors:
        if d.get("feature"):
            for ob in bpy.data.objects:
                if ob.name == f"door.{d['id']}" or ob.get("door") == d["id"] and ob.get("gb_kind") == "doorleaf":
                    link(ob, d["feature"], "both")
    for fid, f in feats.items():
        for rid in f.get("rooms", []):
            for ob in bpy.data.objects:
                if not ob.get("generated"):
                    continue
                own = ob.get("room") == rid and ob.get("gb_kind") in ("room", "floor", "ceiling", "nav")
                wall = ob.get("gb_kind") == "wall" and rid in ob.get("rooms", "").split(",") and \
                    all(x == rid or spec.kind(x) != "indoor" for x in ob.get("rooms", "").split(","))
                if own or wall:
                    link(ob, fid, "both")

    for fid, (sr, sl, sxy, rad, dr, dl, dxy, two) in ACOUSTIC.items():
        acoustic(fid, (sr, sl, floor_xyz(sr, sl, sxy)), (dr, dl, floor_xyz(dr, dl, dxy)), rad, two)
    if "F02" in ACOUSTIC:
        hv = feats["F02"]["acoustic"].get("src_area")
        bpy.data.objects["acoustic.F02.src"]["area"] = ",".join(str(v) for v in hv)
        link(bpy.data.objects["balustrade.hall_gallery"], "F02", "both")
    (hr, hl, hp), (sr, sl, sp) = PHONES["phone.hall"], PHONES["phone.study"]
    acoustic("F07", (hr, hl, hp), (sr, sl, sp), 1.0, True)
    ends = {}
    for side, (lvl, (x, y, z)) in TUBE_ENDS.items():
        room = "master" if side == "master" else "pantry"
        z = spec.lz[lvl] + z
        ends[side] = (room, lvl, (x, y, z))
        into = (0, -1) if side == "master" else (-1, 0)
        mp, cap = MeshAccum(), MeshAccum()
        if into[1]:
            mp.prism(rect_xy(x - 0.05, y - 0.08, x + 0.05, y + 0.05), [z - 0.05] * 4, [z + 0.05] * 4)
            cap.prism(rect_xy(x - 0.07, y - 0.11, x + 0.07, y - 0.08), [z - 0.07] * 4, [z + 0.07] * 4)
        else:
            mp.prism(rect_xy(x - 0.08, y - 0.05, x + 0.05, y + 0.05), [z - 0.05] * 4, [z + 0.05] * 4)
            cap.prism(rect_xy(x - 0.11, y - 0.07, x - 0.08, y + 0.07), [z - 0.07] * 4, [z + 0.07] * 4)
        new(f"F08.mouthpiece.{side}", mp, lvl, "gb_brass", "F08", "both", {"room": room})
        new(f"F08.cap.{side}", cap, lvl, "gb_brass", "F08", "off", {"room": room})
    acoustic("F08", ends["master"], ends["pantry"], 1.0, True)

    h = next(d for d in spec.doors if d["id"] == "h_serving")
    hx, hy = h["at"]
    run = spec.door_run["h_serving"]
    zf = spec.floor_top(h["a"], "ground", hx, hy) + HATCH_SILL
    bolt = MeshAccum()
    face = hx + run.t / 2
    bolt.prism(rect_xy(face, hy + 0.15, face + 0.04, hy + 0.35), [zf + 0.25] * 4, [zf + 0.33] * 4)
    new("F01.bolt", bolt, "ground", "gb_brass", "F01", "off", {"room": "pantry"})

    leaf = bpy.data.objects.get("doorleaf.d_jib")
    jd = next(d for d in spec.doors if d["id"] == "d_jib")
    jx, jy = jd["at"]
    jrun = spec.door_run["d_jib"]
    jw, jh = spec.openings["concealed"]
    lib_face = jx - jrun.t / 2
    cover = MeshAccum()
    ox, oy, oz = leaf.location if leaf else (0.0, 0.0, 0.0)
    cover.prism(rect_xy(lib_face - 0.35 - ox, jy - jw / 2 - oy, lib_face - ox, jy + jw / 2 - oy),
                [0.01 - oz] * 4, [jh + 0.1 - oz] * 4)
    cov = new("F05.cover", cover, "ground", "gb_bookcase", "F05", "both", {"room": "library"})
    cov.location = (ox, oy, oz)
    if leaf:
        cov.parent = leaf
        cov.matrix_parent_inverse = (Matrix.Translation(leaf.location) @
                                     leaf.rotation_euler.to_matrix().to_4x4()).inverted()
        link(leaf, "F05", "on")
    infill = MeshAccum()
    infill.prism(rect_xy(jx, jy - jw / 2, jx + jrun.t / 2, jy + jw / 2), [0.0] * 4, [jh] * 4)
    new("F05.infill", infill, "ground", "gb_wall_int", "F05", "off", {"room": "study"})

    for room in ("study", "g4"):
        br, fb = MeshAccum(), MeshAccum()
        lvl = FIREPLACES[room][0]
        build_fireplace(spec, room, br, fb)
        new(f"F06.fireplace.{room}", br, lvl, "gb_fireplace", "F06", "both", {"room": room})
        new(f"F06.firebox.{room}", fb, lvl, "gb_firebox", "F06", "both", {"room": room})

    wx, wy = next(f["acoustic"]["window_at"] for f in spec.d["features"] if f["id"] == "F11")
    ww, wsill, whead = WINDOW["ground"]
    face = wy - spec.wall_t["exterior"] / 2
    sash, curt = MeshAccum(), MeshAccum()
    sash.prism(rect_xy(wx - ww / 2, wy - 0.02, wx + ww / 2, wy + 0.02), [wsill] * 4, [whead] * 4)
    curt.prism(rect_xy(wx - ww / 2 - 0.2, face - 0.12, wx + ww / 2 + 0.2, face - 0.06), [wsill - 0.1] * 4, [whead + 0.2] * 4)
    new("F11.sash", sash, "ground", "gb_glass", "F11", "off", {"room": "study"})
    new("F11.curtains", curt, "ground", "gb_curtain", "F11", "off", {"room": "study"})

    for fid, c in fcoll.items():
        report.setdefault("features", {})[fid] = len(c.objects)
        if not c.objects:
            report["errors"].append(f"feature {fid}: no objects")


def make_empty(name, loc, facing_deg, coll, props, display="ARROWS", size=0.4):
    import math
    from mathutils import Euler
    emp = bpy.data.objects.new(name, None)
    emp.empty_display_type = display
    emp.empty_display_size = size
    emp.location = loc
    emp.rotation_euler = Euler((0, 0, math.radians(facing_deg)))
    coll.objects.link(emp)
    emp["generated"] = True
    for k, v in props.items():
        emp[k] = v
    return emp


def facing_of(axis, into):
    if axis == "x":
        return -90.0 if into > 0 else 90.0
    return 0.0 if into > 0 else 180.0


def build_devices(spec, report):
    objs = []

    def put(name, acc, lvl, mat, props, origin=(0.0, 0.0, 0.0)):
        if origin != (0.0, 0.0, 0.0):
            acc.verts = [(x - origin[0], y - origin[1], z - origin[2]) for x, y, z in acc.verts]
        ob = make_object(name, acc, collection(COLLECTIONS[lvl]), material(mat),
                         dict({"gb_kind": "device", "level": lvl}, **props))
        ob.location = origin
        report["objects"]["device"] += 1
        return ob

    for room, (lvl, *_r) in FIREPLACES.items():
        if room in ("study", "g4"):
            continue
        br, fb = MeshAccum(), MeshAccum()
        build_fireplace(spec, room, br, fb)
        put(f"fireplace.{room}", br, lvl, "gb_fireplace", {"room": room, "gb_kind": "fireplace"})
        put(f"firebox.{room}", fb, lvl, "gb_firebox", {"room": room, "gb_kind": "fireplace"})

    for room, (lvl, axis, c, s, into) in BELL_PULLS.items():
        face = wall_face(spec, lvl, axis, c, s, into)
        z0 = spec.lz[lvl]
        px, py = (face, s) if axis == "x" else (s, face)
        top = z0 + spec.ceil_rel(room, lvl, px, py) - 0.1
        v = face + into * 0.05
        acc = MeshAccum()
        acc.prism(wall_rect(axis, s - 0.015, s + 0.015, v - 0.015, v + 0.015), [z0 + 1.1] * 4, [top] * 4)
        acc.prism(wall_rect(axis, s - 0.03, s + 0.03, v - 0.03, v + 0.03), [z0 + 0.95] * 4, [z0 + 1.1] * 4)
        hx, hy = (v, s) if axis == "x" else (s, v)
        ob = put(f"bell.{room}", acc, lvl, "gb_brass", {"room": room}, origin=(hx, hy, z0 + 1.0))
        ob.rotation_euler.z = 0.0
        ob["facing"] = facing_of(axis, into)

    room, lvl, axis, c, s, into = BELL_BOARD
    face = wall_face(spec, lvl, axis, c, s, into)
    acc = MeshAccum()
    acc.prism(wall_rect(axis, s - 0.6, s + 0.6, face, face + into * 0.08), [1.6] * 4, [2.4] * 4)
    bx, by = (face + into * 0.04, s) if axis == "x" else (s, face + into * 0.04)
    ob = put("bellboard", acc, lvl, "gb_brass", {"room": room}, origin=(bx, by, 2.0))
    ob["facing"] = facing_of(axis, into)

    tables = {"phone.hall": ("furniture.hall.phone_table", (11.1, 13.25, 11.6, 13.75)),
              "phone.study": ("furniture.study.desk", (10.2, 12.8, 10.9, 14.2))}
    for name, (room, lvl, (x, y, z)) in PHONES.items():
        tname, (tx0, ty0, tx1, ty1) = tables[name]
        t = MeshAccum()
        t.prism(rect_xy(tx0, ty0, tx1, ty1), [0.0] * 4, [z] * 4)
        put(tname, t, lvl, "gb_bookcase", {"room": room, "gb_kind": "furniture", "surface": z})
        ph = MeshAccum()
        ph.prism(rect_xy(x - 0.11, y - 0.09, x + 0.11, y + 0.09), [z] * 4, [z + 0.08] * 4)
        ph.prism(rect_xy(x - 0.12, y - 0.03, x + 0.12, y + 0.03), [z + 0.08] * 4, [z + 0.12] * 4)
        put(name, ph, lvl, "gb_firebox", {"room": room}, origin=(x, y, z))

    sx0, sy0, sx1, sy1 = spec.shaft
    t = DUMBWAITER_PART_T
    w, sill, head = DUMBWAITER_HATCH
    in_x0, in_y0 = sx0 + t, sy0 + t
    in_x1 = 30.0 - spec.wall_t["interior"] / 2
    in_y1 = 8.0 - spec.wall_t["interior"] / 2
    stop_s = DUMBWAITER_STOPS["pantry"][3]
    acc = MeshAccum()
    for lvl in ("ground", "first"):
        zb, zt = spec.lz[lvl] - SLAB, spec.lz[spec.next_level(lvl)] - SLAB if lvl == "ground" else spec.lz[lvl] + spec.ceil["first"]
        acc.prism(rect_xy(sx0, sy0, in_x1, sy0 + t), [zb] * 4, [zt] * 4)
        if lvl == "ground":
            h0, h1 = stop_s - w / 2, stop_s + w / 2
            fz = spec.lz[lvl]
            acc.prism(rect_xy(sx0, sy0 + t, sx0 + t, h0), [zb] * 4, [zt] * 4)
            acc.prism(rect_xy(sx0, h1, sx0 + t, in_y1), [zb] * 4, [zt] * 4)
            acc.prism(rect_xy(sx0, h0, sx0 + t, h1), [zb] * 4, [fz + sill] * 4)
            acc.prism(rect_xy(sx0, h0, sx0 + t, h1), [fz + head] * 4, [zt] * 4)
        else:
            acc.prism(rect_xy(sx0, sy0 + t, sx0 + t, in_y1), [zb] * 4, [zt] * 4)
    put("dumbwaiter.shaft", acc, "ground", "gb_wall_int", {"room": "pantry", "gb_kind": "device"})
    car = MeshAccum()
    car.prism(rect_xy(in_x0 + 0.05, in_y0 + 0.05, in_x1 - 0.05, in_y1 - 0.05), [sill] * 4, [head] * 4)
    put("dumbwaiter.car", car, "ground", "gb_bookcase", {"room": "pantry"})
    for stop, (lvl, axis, c, s, into, room) in DUMBWAITER_STOPS.items():
        if stop == "pantry":
            face = sx0
        else:
            face = wall_face(spec, lvl, axis, c, s, into)
        loc = (face, s, spec.lz[lvl] + sill) if axis == "x" else (s, face, spec.lz[lvl] + sill)
        make_empty(f"dumbwaiter.{stop}", loc, facing_of(axis, into), collection(COLLECTIONS[lvl]),
                   {"gb_kind": "device", "room": room, "level": lvl})
        report["objects"]["device"] += 1

    for room, (x0, y0, x1, y1, top, head_end) in BEDS.items():
        lvl = spec.rooms[room]["level"]
        z = spec.lz[lvl]
        acc = MeshAccum()
        acc.prism(rect_xy(x0, y0, x1, y1), [z] * 4, [z + top] * 4)
        hb = {"x0": (x0, y0, x0 + 0.08, y1), "x1": (x1 - 0.08, y0, x1, y1),
              "y0": (x0, y0, x1, y0 + 0.08), "y1": (x0, y1 - 0.08, x1, y1)}[head_end]
        acc.prism(rect_xy(*hb), [z] * 4, [z + top + 0.45] * 4)
        put(f"furniture.bed.{room}", acc, lvl, "gb_bookcase", {"room": room, "gb_kind": "furniture"})
    for name, room in BED_MARKERS.items():
        x0, y0, x1, y1, top, head_end = BEDS[room]
        lvl = spec.rooms[room]["level"]
        foot = {"x0": -90.0, "x1": 90.0, "y0": 0.0, "y1": 180.0}[head_end]
        make_empty(name, ((x0 + x1) / 2, (y0 + y1) / 2, spec.lz[lvl] + top), foot,
                   collection(COLLECTIONS[lvl]), {"gb_kind": "device", "room": room, "level": lvl})
        report["objects"]["device"] += 1
    for name, (room, lvl, (x, y), facing) in SPAWNS.items():
        make_empty(name, (x, y, spec.lz[lvl]), facing, collection(COLLECTIONS[lvl]),
                   {"gb_kind": "device", "room": room, "level": lvl})
        report["objects"]["device"] += 1


def boundary_type(a, b):
    pair = {a, b}
    other = lambda r: b if a == r else a
    if pair == {"garden", "terrace"}:
        return "parapet", "terrace"
    if "garden" in pair:
        return ("garden_wall", None) if other("garden") in (None, "path") else None
    if "terrace" in pair:
        return ("parapet", "terrace") if other("terrace") is None else None
    for r in ("forecourt", "yard", "path"):
        if r in pair and other(r) is None:
            return "hedge", None
    return None


def build_outdoors(spec, report):
    own = spec.owner["ground"]
    dims = {"garden_wall": GARDEN_WALL, "hedge": HEDGE, "parapet": PARAPET}
    pieces = []

    def heights(kind, room, x, y):
        f = spec.floor_top(room, "ground", x, y)
        t, h = dims[kind]
        bottom = OUTDOOR_SLAB_BOTTOM if kind == "garden_wall" else f - SLAB
        return bottom, f + h

    for orient in ("y", "x"):
        n_lines = spec.nx + 1 if orient == "y" else spec.ny + 1
        n_cells = spec.ny if orient == "y" else spec.nx
        for i in range(n_lines):
            c = (spec.x0 if orient == "y" else spec.y0) + i * GRID
            for j in range(n_cells):
                if orient == "y":
                    a, b = own.get((i - 1, j)), own.get((i, j))
                    s0 = spec.y0 + j * GRID
                else:
                    a, b = own.get((j, i - 1)), own.get((j, i))
                    s0 = spec.x0 + j * GRID
                bt = boundary_type(a, b)
                if not bt:
                    continue
                kind, side_room = bt
                s1 = s0 + GRID
                room = side_room or (a if spec.kind(a) == "outdoor" else b)
                if kind == "garden_wall":
                    room = "garden"
                side = 0 if side_room is None else (-1 if a == side_room else 1)
                pt = (lambda s: (c, s)) if orient == "y" else (lambda s: (s, c))
                zb0, zt0 = heights(kind, room, *pt(s0))
                zb1, zt1 = heights(kind, room, *pt(s1))
                pieces.append(dict(orient=orient, c=c, s0=s0, s1=s1, kind=kind, side=side,
                                   zb0=zb0, zb1=zb1, zt0=zt0, zt1=zt1))
    runs = []
    for p in sorted(pieces, key=lambda p: (p["kind"], p["orient"], p["c"], p["s0"])):
        q = runs[-1] if runs else None
        if q and q["kind"] == p["kind"] and q["orient"] == p["orient"] and abs(q["c"] - p["c"]) < EPS \
                and abs(q["s1"] - p["s0"]) < EPS and q["side"] == p["side"] \
                and abs(q["zt1"] - p["zt0"]) < 1e-4 and abs(q["zb1"] - p["zb0"]) < 1e-4:
            sq = (q["zt1"] - q["zt0"]) / (q["s1"] - q["s0"])
            sp = (p["zt1"] - p["zt0"]) / (p["s1"] - p["s0"])
            if abs(sq - sp) < 1e-4:
                q["s1"], q["zt1"], q["zb1"] = p["s1"], p["zt1"], p["zb1"]
                continue
        runs.append(dict(p, openings=[]))

    cut = 0
    for d in spec.doors:
        if d["kind"] not in SKIP_OPENING_KINDS:
            continue
        x, y = d["at"]
        if d["kind"] == "gate":
            w, h = spec.openings["gate"]
            kinds, zlo, zhi = ("garden_wall",), spec.floor_top("garden", "ground", x, y), None
            zhi = zlo + h
        else:
            w, kinds, zlo, zhi = TERRACE_STEPS[0], ("parapet",), None, None
        for r in runs:
            if r["kind"] not in kinds:
                continue
            c, s = (x, y) if r["orient"] == "y" else (y, x)
            if abs(r["c"] - c) < EPS and r["s0"] - EPS <= s <= r["s1"] + EPS:
                r["openings"].append((s - w / 2, s + w / 2, zlo, zhi))
                cut += 1
                break
        else:
            report["errors"].append(f"outdoor opening {d['id']}: no boundary run")
    report["outdoor_openings"] = cut

    accs = defaultdict(MeshAccum)
    for r in runs:
        t = dims[r["kind"]][0]
        s0, s1 = r["s0"], r["s1"]
        if r["kind"] != "parapet":
            s0, s1 = s0 - t / 2, s1 + t / 2
        L = r["s1"] - r["s0"]
        lerp = lambda a0, a1, s: a0 + (a1 - a0) * (s - r["s0"]) / L
        zb = lambda s: lerp(r["zb0"], r["zb1"], min(max(s, r["s0"]), r["s1"]))
        zt = lambda s: lerp(r["zt0"], r["zt1"], min(max(s, r["s0"]), r["s1"]))
        off = (-t / 2, t / 2) if r["side"] == 0 else ((-t, 0.0) if r["side"] < 0 else (0.0, t))
        boxes = []
        cur = s0
        for o0, o1, zlo, zhi in sorted(r["openings"]):
            if o0 > cur:
                boxes.append((cur, o0, None, None))
            if zlo is not None:
                boxes.append((o0, o1, None, zlo))
            if zhi is not None:
                boxes.append((o0, o1, zhi, None))
            cur = o1
        if s1 > cur:
            boxes.append((cur, s1, None, None))
        for a0, a1, lo, hi in boxes:
            b0, b1 = (zb(a0), zb(a1)) if lo is None else (lo, lo)
            t0, t1 = (zt(a0), zt(a1)) if hi is None else (hi, hi)
            if r["orient"] == "x":
                xy = [(a0, r["c"] + off[0]), (a1, r["c"] + off[0]), (a1, r["c"] + off[1]), (a0, r["c"] + off[1])]
                zbs, zts = [b0, b1, b1, b0], [t0, t1, t1, t0]
            else:
                xy = [(r["c"] + off[0], a0), (r["c"] + off[1], a0), (r["c"] + off[1], a1), (r["c"] + off[0], a1)]
                zbs, zts = [b0, b0, b1, b1], [t0, t0, t1, t1]
            accs[r["kind"]].prism(xy, zbs, zts)
    names = {"garden_wall": ("outdoor.garden_wall", "gb_garden_wall"),
             "hedge": ("outdoor.hedge", "gb_hedge"), "parapet": ("outdoor.terrace_parapet", "gb_parapet")}
    for kind, acc in accs.items():
        nm, mat = names[kind]
        make_object(nm, acc, collection(COLLECTIONS["grounds"]), material(mat),
                    {"gb_kind": "boundary", "kind": kind, "level": "ground",
                     "height": dims[kind][1], "thickness": dims[kind][0]})
        report["objects"]["boundary"] += 1

    sw, rise, going = TERRACE_STEPS
    x0, y0, x1, _ = spec.steps_rect
    zg = spec.floor_top("garden", "ground", 0, 0)
    n = round((spec.floor_top("terrace", "ground", 0, 0) - zg) / rise)
    foot = y0 + (n - 1) * going
    vis, nav = MeshAccum(), MeshAccum()
    stair_flight(vis, (x0, x1), foot, -1, n, zg, rise, going, solid_to=OUTDOOR_SLAB_BOTTOM)
    stair_ramp(nav, (x0, x1), foot, -1, n, zg, rise, going)
    make_object("steps.terrace_garden", vis, collection(COLLECTIONS["grounds"]), material("gb_parapet"),
                {"gb_kind": "stair", "stair": "terrace", "room": "garden", "level": "ground",
                 "risers": n, "rise": rise, "going": going})
    make_object("nav.steps.terrace_garden", nav, collection(COLLECTIONS["nav"]), material("gb_nav"),
                {"gb_kind": "nav", "stair": "terrace", "room": "garden", "level": "ground", "stairs": True},
                recalc=False)
    report["objects"]["stair"] += 1
    report["objects"]["nav"] += 1

    def put(name, acc, mat, props):
        make_object(name, acc, collection(COLLECTIONS["grounds"]), material(mat),
                    dict({"gb_kind": "furniture", "room": "garden", "level": "ground"}, **props))
        report["objects"]["garden"] += 1

    gr = MeshAccum()
    for gx0, gy0, gx1, gy1 in GARDEN_GRAVEL:
        gr.quad(rect_xy(gx0, gy0, gx1, gy1), [zg + 0.005] * 4)
    put("garden.gravel", gr, "gb_gravel", {"gb_kind": "dressing"})
    sx, sy = GARDEN_SUNDIAL
    sd = MeshAccum()
    sd.prism(rect_xy(sx - 0.15, sy - 0.15, sx + 0.15, sy + 0.15), [zg] * 4, [zg + 0.9] * 4)
    sd.prism(rect_xy(sx - 0.25, sy - 0.25, sx + 0.25, sy + 0.25), [zg + 0.9] * 4, [zg + 0.94] * 4)
    put("garden.sundial", sd, "gb_parapet", {"surface": zg + 0.94})
    px0, py0, px1, py1 = GARDEN_POND
    pd, water = MeshAccum(), MeshAccum()
    k = 0.3
    for bx in ((px0, py0, px1, py0 + k), (px0, py1 - k, px1, py1),
               (px0, py0 + k, px0 + k, py1 - k), (px1 - k, py0 + k, px1, py1 - k)):
        pd.prism(rect_xy(*bx), [zg] * 4, [zg + 0.4] * 4)
    water.quad(rect_xy(px0 + k, py0 + k, px1 - k, py1 - k), [zg + 0.25] * 4)
    put("garden.pond", pd, "gb_parapet", {"seat_height": zg + 0.4})
    put("garden.pond_water", water, "gb_water", {"gb_kind": "dressing"})
    import math
    for n_, (bx, by, facing) in enumerate(GARDEN_BENCHES, start=1):
        b = MeshAccum()
        fa = math.radians(facing)
        fx, fy = -math.sin(fa), math.cos(fa)
        ax, ay = fy, -fx
        def quad_pts(u0, u1, v0, v1):
            return [(bx + ax * u + fx * v, by + ay * u + fy * v) for u, v in ((u0, v0), (u1, v0), (u1, v1), (u0, v1))]
        def box(u0, u1, v0, v1, z0, z1):
            pts = quad_pts(u0, u1, v0, v1)
            area = sum(pts[i][0] * pts[(i + 1) % 4][1] - pts[(i + 1) % 4][0] * pts[i][1] for i in range(4))
            if area < 0:
                pts = pts[::-1]
            b.prism(pts, [z0] * 4, [z1] * 4)
        box(-0.8, 0.8, -0.25, 0.25, zg, zg + 0.45)
        box(-0.8, 0.8, -0.25, -0.17, zg + 0.45, zg + 0.9)
        put(f"garden.bench.{n_}", b, "gb_bookcase", {"seat_height": zg + 0.45, "facing": facing})
    cx0, cy0, cx1, cy1, ch = GARDEN_COLD_FRAMES
    cf = MeshAccum()
    cf.prism(rect_xy(cx0, cy0, cx1, cy1), [zg] * 4, [zg + ch] * 4)
    put("garden.cold_frames", cf, "gb_glass", {"surface": zg + ch})


FACING = {"N": 0.0, "W": 90.0, "S": 180.0, "E": -90.0}
DIRV = {"N": (0.0, 1.0), "S": (0.0, -1.0), "E": (1.0, 0.0), "W": (-1.0, 0.0)}


def load_layout():
    path = os.path.join(HERE, "layout.py")
    ns = {}
    exec(compile(open(path).read(), path, "exec"), ns)
    return ns


def build_anchors(spec, report):
    import math
    ns = load_layout()
    LAYOUT, KINDS, SEAT_KINDS = ns["LAYOUT"], ns["KINDS"], ns["SEAT_KINDS"]
    errors = report["errors"]
    obstacles = defaultdict(list)
    anchors = []
    pieces = []

    for d in spec.doors:
        if d["kind"] in ("hatch", "steps"):
            continue
        geo = door_geometry(spec, d)
        if not geo:
            continue
        orient, c, s, _ = geo
        w = DOOR_WIDTH_OVERRIDE.get(d["id"], spec.openings[d["kind"]][0]) / 2 + 0.1
        depth = 1.5 if d["kind"] == "garage" else DOOR_CLEAR
        r = (c - depth, s - w, c + depth, s + w) if orient == "y" else (s - w, c - depth, s + w, c + depth)
        obstacles[d["level"]].append((r, "door " + d["id"], "door"))
    for room, (lvl, axis, c, s, into) in FIREPLACES.items():
        face = wall_face(spec, lvl, axis, c, s, into)
        lo, hi = sorted((face, face + into * 0.9))
        r = (lo, s - 0.9, hi, s + 0.9) if axis == "x" else (s - 0.9, lo, s + 0.9, hi)
        obstacles[lvl].append((r, "fireplace " + room, "solid"))
    obstacles["ground"].append((spec.steps_rect, "terrace steps", "solid"))
    for lvl in ("ground", "first"):
        obstacles[lvl].append((spec.shaft, "dumbwaiter shaft", "solid"))

    def room_level(room, lvl=None):
        return lvl or spec.rooms[room]["boxes"][0]["level"]

    def box_of(room, i):
        b = spec.rooms[room]["boxes"][i]
        return (b["x0"], b["y0"], b["x1"], b["y1"]), b["level"]

    def side_frame(room, side, at, box):
        (x0, y0, x1, y1), lvl = box_of(room, box)
        axis, c, into = {"N": ("y", y1, -1), "S": ("y", y0, 1), "E": ("x", x1, -1), "W": ("x", x0, 1)}[side]
        face = wall_face(spec, lvl, axis, c, at, into)
        if axis == "y":
            return (at, face), (0.0, float(into)), lvl
        return (face, at), (float(into), 0.0), lvl

    def add_anchor(room, lvl, tags, x, y, dz, facing, name=None, pair=None):
        deg = FACING[facing] if isinstance(facing, str) else float(facing)
        anchors.append(dict(room=room, lvl=lvl, tags=tags, x=x, y=y, dz=dz, deg=deg, name=name, pair=pair))

    def vec_deg(n):
        return math.degrees(math.atan2(-n[0], n[1]))

    def add_piece(room, lvl, kind, O, n, w, d, h, o, name_hint=None):
        a = (n[1], -n[0])
        pt = lambda u, v: (O[0] + a[0] * u + n[0] * v, O[1] + a[1] * u + n[1] * v)
        cs = [pt(-w / 2, 0), pt(w / 2, 0), pt(w / 2, d), pt(-w / 2, d)]
        rect = (min(c[0] for c in cs), min(c[1] for c in cs), max(c[0] for c in cs), max(c[1] for c in cs))
        rec = dict(KINDS[kind][3])
        rec.update(o)
        pieces.append(dict(room=room, lvl=lvl, kind=kind, rect=rect, h=h, back=rec.get("back"),
                           O=O, n=n, w=w, d=d))
        deg = vec_deg(n)
        k = rec.get("surf", 0)
        for i in range(k):
            x, y = pt(w * ((i + 0.5) / k - 0.5), d / 2)
            add_anchor(room, lvl, "surface", x, y, rec.get("surf_z", h), deg)
        for i in range(rec.get("drawer", 0)):
            x, y = pt(w / 4 if (k or rec.get("hide")) else 0.0, d - 0.05)
            add_anchor(room, lvl, "drawer", x, y, rec.get("drawer_z", min(0.5, h * 0.6)), deg)
        for i in range(rec.get("hide", 0)):
            x, y = pt(-w / 4, d / 2)
            add_anchor(room, lvl, "hide", x, y, rec.get("hide_z", h), deg)
        k = rec.get("seats", 0)
        for i in range(k):
            x, y = pt(w * ((i + 0.5) / k - 0.5), d / 2)
            add_anchor(room, lvl, rec.get("seat_tags", "seat"), x, y, h, deg, pair=rec.get("pair"))
        if rec.get("work"):
            x, y = pt(0.0, d + 0.45)
            add_anchor(room, lvl, "work", x, y, 0.0, deg + 180.0)
        if rec.get("bed"):
            x, y = pt(0.0, d / 2)
            add_anchor(room, lvl, "bed", x, y, h, deg)

    pair_n = defaultdict(int)
    rugs = []
    for room, items in LAYOUT.items():
        if room not in spec.rooms:
            errors.append(f"layout: unknown room {room}")
            continue
        for it in items:
            t = it["t"]
            if t == "obst":
                obstacles[room_level(room, it["lvl"])].append((it["rect"], f"{room} fixture", "solid"))
            elif t in ("wall", "free"):
                dw, dd, dh, _ = KINDS[it["kind"]]
                w, d, h = it["w"] or dw, it["d"] or dd, it["h"] or dh
                if t == "wall":
                    O, n, lvl = side_frame(room, it["side"], it["at"], it["box"])
                    O = (O[0] + n[0] * 0.02, O[1] + n[1] * 0.02)
                else:
                    n = DIRV[it["face"]]
                    O = (it["x"] - n[0] * d / 2, it["y"] - n[1] * d / 2)
                    lvl = room_level(room)
                add_piece(room, lvl, it["kind"], O, n, w, d, h, it["o"])
            elif t == "pair":
                lvl = room_level(room, it["lvl"])
                pair_n[room] += 1
                ax = (1.0, 0.0) if it["axis"] == "x" else (0.0, 1.0)
                for sfx, sgn in (("a", -1), ("b", 1)):
                    add_anchor(room, lvl, "stand,pair", it["x"] + ax[0] * PAIR_HALF * sgn,
                               it["y"] + ax[1] * PAIR_HALF * sgn, 0.0, vec_deg((-ax[0] * sgn, -ax[1] * sgn)),
                               name=f"{room}_p{pair_n[room]:02d}_{sfx}")
            elif t == "pseat":
                lvl = room_level(room)
                pair_n[room] += 1
                ax = (1.0, 0.0) if it["axis"] == "x" else (0.0, 1.0)
                dw, dd, dh, _ = KINDS["chair"]
                for sfx, sgn in (("a", -1), ("b", 1)):
                    n = (-ax[0] * sgn, -ax[1] * sgn)
                    cx, cy = it["x"] + ax[0] * PAIR_HALF * sgn, it["y"] + ax[1] * PAIR_HALF * sgn
                    add_piece(room, lvl, "chair", (cx - n[0] * dd / 2, cy - n[1] * dd / 2), n, dw, dd, dh,
                              dict(seats=0))
                    add_anchor(room, lvl, "seat,pair", cx, cy, dh, vec_deg(n),
                               name=f"{room}_p{pair_n[room]:02d}_{sfx}")
                if it["table"]:
                    add_piece(room, lvl, "sidetable", (it["x"], it["y"] - 0.25), (0.0, 1.0), 0.5, 0.5, 0.6, {})
            elif t == "anchor":
                add_anchor(room, room_level(room, it["lvl"]), it["tags"], it["x"], it["y"], it["z"],
                           it["facing"], name=it["name"])
            elif t == "pic":
                O, n, lvl = side_frame(room, it["side"], it["at"], it["box"])
                add_anchor(room, lvl, it["tags"], O[0] + n[0] * 0.03, O[1] + n[1] * 0.03, it["z"], vec_deg(n))
            elif t == "win":
                (x0, y0, x1, y1), lvl = box_of(room, it["box"])
                axis, c, into = {"N": ("y", y1, -1), "S": ("y", y0, 1), "E": ("x", x1, -1), "W": ("x", x0, 1)}[it["side"]]
                lo, hi = (x0, x1) if axis == "y" else (y0, y1)
                wins = sorted((o[0] + o[1]) / 2 for r in spec.runs
                              if r.lvl == lvl and r.orient == ("x" if axis == "y" else "y")
                              and abs(r.c - c) < EPS for o in r.openings
                              if o[4].startswith("window") and lo < (o[0] + o[1]) / 2 < hi)
                if it["n"] >= len(wins):
                    errors.append(f"layout: {room} has no window {it['n']} on its {it['side']} wall")
                    continue
                s = wins[it["n"]]
                face = wall_face(spec, lvl, axis, c, s, into)
                px, py = (s, face + into * 0.6) if axis == "y" else (face + into * 0.6, s)
                add_anchor(room, lvl, "stand,window", px, py, 0.0,
                           vec_deg((0.0, -into) if axis == "y" else (-into, 0.0)))
            elif t == "rug":
                rugs.append((room, room_level(room), it))
                add_anchor(room, room_level(room), "hide", it["x"], it["y"], 0.0, 0.0)

    def overlap(a, b, m=0.0):
        return min(a[2], b[2]) - max(a[0], b[0]) > 1e-3 - m and min(a[3], b[3]) - max(a[1], b[1]) > 1e-3 - m

    for i, p in enumerate(pieces):
        r, lvl, room = p["rect"], p["lvl"], p["room"]
        label = f"{room} {p['kind']} at ({(r[0] + r[2]) / 2:.2f}, {(r[1] + r[3]) / 2:.2f})"
        for cx, cy in ((r[0] + 0.02, r[1] + 0.02), (r[2] - 0.02, r[1] + 0.02), (r[2] - 0.02, r[3] - 0.02), (r[0] + 0.02, r[3] - 0.02)):
            own = spec.owner_at(lvl, cx, cy)
            if own != room and not (room, own) in {(r_["id"], r_.get("nested_in")) for r_ in spec.d["rooms"]}:
                errors.append(f"furniture {label}: corner in {own}")
                break
        for orect, olabel, _ in obstacles[lvl]:
            if overlap(r, orect):
                errors.append(f"furniture {label}: overlaps {olabel}")
        for q in pieces[i + 1:]:
            if q["lvl"] == lvl and overlap(r, q["rect"]):
                errors.append(f"furniture {label}: overlaps {q['room']} {q['kind']}")
        (fx, fy) = ((r[0] + r[2]) / 2, (r[1] + r[3]) / 2)
        if spec.kind(room) == "indoor" and p["h"] > spec.ceil_rel(room, lvl, fx, fy) - 0.02:
            errors.append(f"furniture {label}: taller than the ceiling there")

    counter = defaultdict(int)
    names = set()
    for a in anchors:
        if not a["name"]:
            counter[a["room"]] += 1
            a["name"] = f"{a['room']}_{counter[a['room']]:02d}"
        if a["name"] in names:
            errors.append(f"anchor {a['name']}: duplicate id")
        names.add(a["name"])
        tags = set(a["tags"].split(","))
        bad = tags - set(spec.d["anchor_rules"]["tags"])
        if bad:
            errors.append(f"anchor {a['name']}: unknown tags {sorted(bad)}")
        lvl, room = a["lvl"], a["room"]
        own = spec.owner_at(lvl, a["x"], a["y"])
        if own != room and (room, own) not in {(r_["id"], r_.get("nested_in")) for r_ in spec.d["rooms"]}:
            errors.append(f"anchor {a['name']}: lies in {own}")
        a["z"] = spec.floor_top(room, lvl, a["x"], a["y"]) + a["dz"]
        if tags & PERSON_TAGS and "seat" not in tags:
            pt = (a["x"], a["y"], a["x"], a["y"])
            for orect, olabel, okind in obstacles[lvl]:
                if okind == "solid" and overlap(pt, orect, PERSON_MARGIN):
                    errors.append(f"anchor {a['name']}: inside {olabel}")
            for q in pieces:
                if q["lvl"] == lvl and overlap(pt, q["rect"], PERSON_MARGIN):
                    errors.append(f"anchor {a['name']}: inside {q['room']} {q['kind']}")
            if spec.kind(room) == "indoor" and spec.ceil_rel(room, lvl, a["x"], a["y"]) < HEADROOM:
                errors.append(f"anchor {a['name']}: headroom {spec.ceil_rel(room, lvl, a['x'], a['y']):.2f}")

    by_room = defaultdict(list)
    for a in anchors:
        by_room[a["room"]].append(a)
    def tally(room):
        c = defaultdict(int)
        sets = set()
        for a in by_room[room]:
            for tg in a["tags"].split(","):
                c[tg] += 1
            if "_p" in a["name"] and a["name"][-2:] in ("_a", "_b"):
                sets.add(a["name"][:-2])
        c["pair_sets"] = len(sets)
        return c
    rules = spec.d["anchor_rules"]
    def need(room, what, have, want):
        if have < want:
            errors.append(f"anchors {room}: {have} {what}, needs {want}")
    for grp in ("small", "medium", "large"):
        for room in rules[grp]["rooms"]:
            c = tally(room)
            need(room, "anchors", len(by_room[room]), rules[grp]["min"])
            for k, v in rules[grp]["of_which"].items():
                need(room, k, c[k], v)
    for room in rules["corridors"]["rooms"]:
        length = max(max(b["x1"] - b["x0"], b["y1"] - b["y0"]) for b in spec.rooms[room]["boxes"])
        c = sum(1 for a in by_room[room] if {"stand", "lean"} & set(a["tags"].split(",")))
        need(room, "stand/lean", c, math.ceil(length * rules["corridors"]["per_metre"] - 1e-6))
    for room, n in rules["outdoors"].items():
        need(room, "anchors", len(by_room[room]), n)
        need(room, "pair_sets", tally(room)["pair_sets"], OUTDOOR_PAIR_SETS.get(room, 0))
    for room in BEDROOMS:
        c = tally(room)
        for k, v in (("bed", 1), ("seat", 1), ("drawer", 1), ("surface", 2)):
            need(room, k, c[k], v)
    stage = [a for a in anchors if "stage" in a["tags"].split(",")]
    need("drawing", "stage", len(stage), 12)

    kcount = defaultdict(int)
    for p in pieces:
        room, lvl, kind = p["room"], p["lvl"], p["kind"]
        kcount[(room, kind)] += 1
        z = spec.floor_top(room, lvl, (p["rect"][0] + p["rect"][2]) / 2, (p["rect"][1] + p["rect"][3]) / 2)
        acc = MeshAccum()
        acc.prism(rect_xy(*p["rect"]), [z] * 4, [z + p["h"]] * 4)
        if p["back"]:
            O, n, w = p["O"], p["n"], p["w"]
            a = (n[1], -n[0])
            cs = [(O[0] + a[0] * u + n[0] * v, O[1] + a[1] * u + n[1] * v) for u, v in ((-w / 2, 0), (w / 2, 0), (w / 2, 0.08), (-w / 2, 0.08))]
            br = (min(c[0] for c in cs), min(c[1] for c in cs), max(c[0] for c in cs), max(c[1] for c in cs))
            acc.prism(rect_xy(*br), [z + p["h"]] * 4, [z + p["h"] + 0.45] * 4)
        clvl = spec.coll_level(room, lvl)
        make_object(f"furniture.{room}.{kind}_{kcount[(room, kind)]:02d}", acc, collection(COLLECTIONS[clvl]),
                    material("gb_upholstery" if kind in SEAT_KINDS else "gb_furniture"),
                    {"gb_kind": "furniture", "kind": kind, "room": room, "level": lvl, "top": p["h"]})
        report["objects"]["furniture"] += 1
    for n_, (room, lvl, it) in enumerate(rugs, start=1):
        z = spec.floor_top(room, lvl, it["x"], it["y"]) + 0.005
        acc = MeshAccum()
        acc.quad(rect_xy(it["x"] - it["w"] / 2, it["y"] - it["d"] / 2, it["x"] + it["w"] / 2, it["y"] + it["d"] / 2), [z] * 4)
        make_object(f"furniture.{room}.rug", acc, collection(COLLECTIONS[spec.coll_level(room, lvl)]),
                    material("gb_rug"), {"gb_kind": "dressing", "room": room, "level": lvl}, recalc=False)
        report["objects"]["furniture"] += 1
    for a in anchors:
        emp = make_empty(f"anchor.{a['name']}", (a["x"], a["y"], a["z"]), a["deg"],
                         collection(COLLECTIONS[spec.coll_level(a["room"], a["lvl"])]),
                         {"gb_kind": "anchor", "tags": a["tags"], "room": a["room"], "level": a["lvl"]},
                         display="SINGLE_ARROW", size=0.3)
        report["objects"]["anchor"] += 1
    tags_total = defaultdict(int)
    for a in anchors:
        for tg in a["tags"].split(","):
            tags_total[tg] += 1
    report["anchors"] = {"total": len(anchors), "by_tag": dict(tags_total),
                         "by_room": {r: len(v) for r, v in sorted(by_room.items())}}


def main():
    spec = Spec(SPEC_PATH)
    report = {"removed": clear_generated(), "errors": [], "warnings": [],
              "skipped_openings": [], "doors_cut": 0, "windows": 0,
              "objects": defaultdict(int)}
    for name in COLLECTIONS.values():
        collection(name)
    if 1 in STEPS:
        build_slabs(spec, report)
    build_walls(spec, report, make=1 in STEPS)
    if 2 in STEPS:
        build_volumes(spec, report)
    if 3 in STEPS:
        build_doors(spec, report)
    if 4 in STEPS:
        build_stairs(spec, report)
    if 5 in STEPS:
        build_features(spec, report)
    if 6 in STEPS:
        build_devices(spec, report)
    if 7 in STEPS:
        build_outdoors(spec, report)
    if 8 in STEPS:
        build_anchors(spec, report)
    nav = bpy.context.view_layer.layer_collection.children.get(COLLECTIONS["nav"])
    if nav:
        nav.hide_viewport = True
    report["objects"] = dict(report["objects"])
    expected = sum(1 for d in spec.doors if d["kind"] not in SKIP_OPENING_KINDS)
    report["doors_expected"] = expected
    report["ok"] = not report["errors"] and report["doors_cut"] == expected and \
        report["objects"].get("room", 0) == len(spec.rooms) and \
        report["objects"].get("door", 0) == len(spec.doors)
    return report


if __name__ == "__main__":
    result = main()
    print(json.dumps(result, indent=1))
