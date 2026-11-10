import math

KINDS = {
    "chair": (0.6, 0.6, 0.45, dict(seats=1, back=1)),
    "armchair": (0.8, 0.8, 0.45, dict(seats=1, hide=1, back=1)),
    "sofa": (1.8, 0.8, 0.45, dict(seats=2, hide=1, back=1)),
    "gbench": (1.6, 0.5, 0.45, dict(seats=2, back=1)),
    "bench": (3.0, 0.4, 0.45, dict(seats=3)),
    "table": (1.2, 0.8, 0.76, dict(surf=1)),
    "sidetable": (0.5, 0.5, 0.6, dict(surf=1)),
    "console": (1.4, 0.45, 0.85, dict(surf=2, drawer=1)),
    "desk": (1.4, 0.7, 0.76, dict(surf=1, drawer=1, work=1)),
    "writing_table": (1.2, 0.7, 0.76, dict(surf=1, drawer=1)),
    "dressing_table": (1.0, 0.5, 0.76, dict(surf=1, drawer=1)),
    "chest": (1.0, 0.5, 0.9, dict(surf=1, drawer=1)),
    "cupboard": (1.2, 0.5, 1.8, dict(drawer=1, drawer_z=1.0)),
    "wardrobe": (1.2, 0.6, 2.0, dict(drawer=1, drawer_z=0.4, hide=1, hide_z=1.4)),
    "bookcase": (1.5, 0.35, 2.2, dict(surf=1, surf_z=1.2, hide=1, hide_z=1.5)),
    "shelves": (1.5, 0.4, 1.8, dict(surf=2, surf_z=1.0)),
    "bed": (1.4, 2.0, 0.55, dict(bed=1, hide=1, back=1)),
    "sbed": (0.9, 1.9, 0.5, dict(bed=1, back=1)),
    "counter": (1.6, 0.6, 0.9, dict(surf=2, work=1)),
    "rack": (1.8, 0.5, 1.8, dict(surf=1, surf_z=1.0)),
    "trunk": (0.9, 0.5, 0.5, dict(surf=1, drawer=1, drawer_z=0.25)),
    "planter": (0.6, 0.6, 0.8, dict(surf=1)),
    "clock": (0.5, 0.35, 2.1, dict(hide=1, hide_z=1.2)),
    "gong": (0.8, 0.4, 1.2, dict()),
    "tub": (1.7, 0.75, 0.6, dict(surf=1, hide=1, hide_z=0.1)),
    "car": (1.8, 4.4, 1.5, dict(surf=1, surf_z=1.0, drawer=1, drawer_z=0.8, hide=1, hide_z=0.5)),
    "billiard": (3.6, 1.8, 0.85, dict(surf=2)),
    "crate": (1.0, 1.0, 0.8, dict(surf=1, hide=1)),
}
SEAT_KINDS = {"chair", "armchair", "sofa", "gbench", "bench", "bed", "sbed"}


def W(kind, side, at, w=None, d=None, h=None, box=0, **o):
    return dict(t="wall", kind=kind, side=side, at=at, w=w, d=d, h=h, box=box, o=o)


def F(kind, x, y, face="S", w=None, d=None, h=None, **o):
    return dict(t="free", kind=kind, x=x, y=y, face=face, w=w, d=d, h=h, o=o)


def PAIR(x, y, axis, lvl=None):
    return dict(t="pair", x=x, y=y, axis=axis, lvl=lvl)


def PSEAT(x, y, axis, table=True):
    return dict(t="pseat", x=x, y=y, axis=axis, table=table)


def A(tags, x, y, facing, z=0.0, lvl=None, name=None):
    return dict(t="anchor", tags=tags, x=x, y=y, facing=facing, z=z, lvl=lvl, name=name)


def PIC(side, at, box=0, z=1.6, tags="wall"):
    return dict(t="pic", side=side, at=at, box=box, z=z, tags=tags)


def WIN(side, n=0, box=0):
    return dict(t="win", side=side, n=n, box=box)


def RUG(x, y, w, d):
    return dict(t="rug", x=x, y=y, w=w, d=d)


def OBST(x0, y0, x1, y1, lvl=None):
    return dict(t="obst", rect=(x0, y0, x1, y1), lvl=lvl)


def finale_stage():
    cx, cy, r = 2.2, 3.0, 2.6
    out = []
    for i in range(10):
        a = math.radians(-70 + 140 * i / 9)
        x, y = cx + r * math.cos(a), cy + r * math.sin(a)
        facing = math.degrees(math.atan2(cy - y, cx - x)) - 90.0
        out.append(A("stage", round(x, 2), round(y, 2), round(facing, 1), name=f"drawing_stage_{i + 1:02d}"))
    out.append(A("stage", 1.7, 1.9, "E", name="drawing_stage_p1"))
    out.append(A("stage", 1.7, 4.1, "E", name="drawing_stage_p2"))
    return out


LAYOUT = {
    "hall": [
        OBST(15.5, 11.2, 19.0, 16.0), OBST(11.1, 13.25, 11.6, 13.75),
        W("console", "W", 1.8), W("console", "E", 1.8),
        W("clock", "W", 6.5), W("gong", "E", 6.5),
        W("chair", "S", 12.3, hide=1), W("chair", "S", 17.7),
        RUG(15.0, 4.0, 3.0, 2.0),
        PAIR(13.0, 6.5, "x"), PAIR(17.0, 6.5, "x"), PAIR(13.2, 11.5, "y"),
        A("stand", 16.2, 10.6, "S"), PIC("W", 7.5), PIC("E", 7.5),
    ],
    "drawing": [
        W("sofa", "N", 3.0), W("sidetable", "N", 4.6), W("chest", "N", 8.5),
        W("sofa", "E", 1.6), F("table", 9.3, 1.6, "W", w=1.0, d=0.6, h=0.45, surf=2),
        W("chest", "S", 7.5), W("bookcase", "S", 9.8),
        PSEAT(7.5, 5.5, "x"), PSEAT(7.5, 2.4, "x"), PAIR(5.9, 3.9, "y"),
        RUG(3.2, 3.0, 2.4, 2.0),
        A("lean", 0.7, 1.7, "E"), WIN("S"), PIC("N", 1.6),
    ] + finale_stage(),
    "dining": [
        F("table", 23.0, 3.6, "N", w=4.0, d=1.2, h=0.76, surf=3),
        F("chair", 21.5, 4.55, "S"), F("chair", 22.5, 4.55, "S"),
        F("chair", 23.5, 4.55, "S"), F("chair", 24.5, 4.55, "S"),
        F("chair", 21.5, 2.65, "N"), F("chair", 22.5, 2.65, "N"),
        F("chair", 23.5, 2.65, "N"), F("chair", 24.5, 2.65, "N"),
        F("chair", 20.55, 3.6, "E"), F("chair", 25.45, 3.6, "W"),
        W("chest", "S", 25.0, w=1.8, surf=2, work=1), W("sidetable", "E", 4.0, w=0.9, d=0.45, h=0.85),
        W("chest", "N", 25.3, hide=1),
        RUG(23.0, 3.6, 4.6, 2.6),
        PAIR(20.3, 1.3, "x"), PAIR(25.3, 5.9, "y"), PAIR(20.6, 6.6, "x"),
        WIN("S"), PIC("W", 6.5),
    ],
    "corr_w": [
        A("lean", 1.6, 9.6, "S"), A("lean", 5.0, 9.6, "S"),
        A("lean", 7.5, 8.4, "N"), A("lean", 10.3, 8.4, "N"), PIC("S", 3.0),
    ],
    "corr_e": [
        A("lean", 20.5, 9.6, "S"), A("lean", 21.5, 8.4, "N"),
        A("lean", 26.0, 9.6, "S"), A("lean", 26.3, 8.4, "N"), PIC("N", 27.5),
    ],
    "library": [
        W("bookcase", "W", 11.3), W("bookcase", "W", 13.0), W("bookcase", "W", 14.7, hide=0),
        W("bookcase", "N", 1.3, hide=0), W("bookcase", "N", 5.6),
        W("bookcase", "S", 1.4), W("chest", "S", 5.6),
        W("bookcase", "E", 14.6, hide=0),
        PSEAT(3.5, 13.0, "x"), PAIR(5.2, 12.0, "y"),
        A("stand,window", 3.5, 14.7, "N"),
    ],
    "study": [
        OBST(10.2, 12.8, 10.9, 14.2),
        A("surface", 10.55, 13.05, "W", z=0.76), A("surface", 10.55, 14.0, "W", z=0.76),
        A("drawer", 10.25, 13.2, "W", z=0.5),
        F("chair", 9.7, 13.5, "E", seat_tags="seat,work"),
        A("drawer", 10.85, 11.5, "W", z=1.4),
        W("chest", "E", 11.5), W("bookcase", "W", 11.2),
        W("armchair", "W", 14.6), WIN("N"),
    ],
    "billiard": [
        F("billiard", 22.3, 13.0, "N"),
        A("lean", 22.3, 11.75, "N"), A("lean", 22.3, 14.25, "S"),
        W("shelves", "S", 21.0, w=0.8, d=0.2, surf=1), W("chest", "S", 27.5, hide=1),
        W("chest", "N", 28.0), W("sidetable", "N", 19.7),
        W("sofa", "E", 13.0),
        PSEAT(27.0, 13.0, "x"), PAIR(25.2, 12.0, "y"), PAIR(26.5, 15.0, "x"),
        WIN("N"),
    ],
    "conservatory": [
        PSEAT(-5.5, 9.0, "x"), PSEAT(-2.5, 12.5, "y"), PSEAT(-6.0, 13.0, "x"),
        W("planter", "W", 7.5, hide=1), W("planter", "W", 11.0), W("planter", "W", 14.8, hide=1),
        W("planter", "S", -4.0),
        W("chest", "E", 14.5), W("chest", "E", 10.8),
        W("sofa", "S", -6.3), WIN("W", 1), WIN("N"),
    ],
    "pantry": [
        W("counter", "W", 4.0, d=0.5), W("counter", "S", 28.5, w=1.4, surf=1),
        W("cupboard", "E", 5.0, w=0.8, d=0.3), W("cupboard", "E", 3.5, hide=1, hide_z=1.4),
    ],
    "kitchen": [
        W("counter", "W", 3.5, w=1.8, d=0.7, surf=1), W("counter", "S", 32.0, w=1.4, surf=1),
        W("chest", "S", 35.0, w=1.6, hide=1), W("cupboard", "E", 6.5, hide=1, hide_z=1.4),
        F("table", 33.5, 4.0, "N", w=2.4, d=1.1, h=0.8, surf=3), F("chair", 33.5, 5.0, "S"),
        PAIR(31.5, 6.5, "x"), PAIR(35.5, 6.3, "y"),
    ],
    "servhall": [
        F("table", 40.0, 3.0, "E", w=3.6, d=1.1, h=0.76, surf=3),
        F("bench", 39.0, 3.0, "E"), F("bench", 41.0, 3.0, "W"),
        W("chest", "S", 40.5, w=1.6, hide=1),
        PAIR(40.9, 6.6, "x"), PAIR(38.0, 1.9, "y"),
        A("work", 37.7, 6.2, "W"), PIC("W", 2.0),
    ],
    "passage": [
        A("lean", 32.3, 8.4, "N"), A("lean", 36.5, 8.4, "N"),
        A("lean", 37.3, 9.6, "S"), A("lean", 40.6, 8.4, "N"),
    ],
    "backstairs": [
        A("stand", 32.2, 10.6, "W", lvl="cellar"), A("stand", 32.2, 10.6, "W", lvl="ground"),
        A("stand", 32.2, 9.0, "W", lvl="first"), A("stand", 32.2, 9.0, "W", lvl="attic"),
    ],
    "housekeeper": [
        W("desk", "W", 13.0, work=0), F("chair", 34.25, 13.0, "W", seat_tags="seat,work"),
        W("cupboard", "E", 12.0, w=1.4, hide=1, hide_z=1.4), W("cupboard", "E", 14.3),
        W("shelves", "N", 35.0), W("armchair", "S", 36.3),
    ],
    "lamproom": [
        W("counter", "N", 39.5, w=2.4, surf=3),
        W("shelves", "W", 13.0, h=1.0, surf=1, surf_z=1.0, hide=1, hide_z=0.3),
        W("cupboard", "E", 14.0), PIC("E", 12.0, tags="wall,hide"),
    ],
    "gallery": [
        OBST(15.44, 11.44, 19.0, 16.0),
        A("lean", 1.3, 8.4, "N"), A("lean", 5.5, 9.6, "S"), A("lean", 6.3, 8.4, "N"),
        A("lean", 10.2, 9.6, "S"),
        A("lean", 13.0, 8.5, "S"), A("lean,window", 15.0, 8.5, "S"), A("lean", 17.0, 8.5, "S"),
        A("lean", 20.0, 9.6, "S"), A("lean", 23.8, 8.4, "N"), A("lean", 23.8, 9.6, "S"),
        A("lean", 27.3, 8.4, "N"), A("lean", 27.2, 9.6, "S"),
        W("console", "W", 13.0, box=1), W("chair", "N", 12.2, box=1),
        PAIR(13.3, 11.2, "x"), A("stand", 12.6, 14.2, "E"), PIC("N", 14.0),
    ],
    "master": [
        W("bed", "S", 3.5, w=1.6), W("sidetable", "S", 2.3), W("sidetable", "S", 4.7),
        W("wardrobe", "N", 1.2),
        W("dressing_table", "W", 6.0, w=1.2), F("chair", 1.15, 6.0, "W"),
        W("writing_table", "E", 1.8), F("chair", 5.75, 1.8, "E"),
        F("armchair", 2.0, 4.3, "W"),
        PAIR(5.3, 6.2, "x"), PAIR(5.2, 3.4, "y"),
        A("lean", 0.8, 1.7, "E"),
    ],
    "dressing": [
        W("wardrobe", "E", 1.5), W("wardrobe", "E", 6.0),
        W("chest", "S", 9.0),
        W("dressing_table", "W", 1.8), F("chair", 8.05, 1.8, "W"),
        W("shelves", "W", 6.3), PIC("N", 7.8),
    ],
    "g1": [
        W("bed", "S", 21.6), W("sidetable", "S", 20.5), W("sidetable", "S", 22.7),
        W("wardrobe", "N", 20.0),
        W("dressing_table", "N", 23.5), F("chair", 23.5, 6.95, "N"),
        W("writing_table", "E", 1.6), F("chair", 23.25, 1.6, "E"),
        F("armchair", 20.9, 4.0, "W"),
        PAIR(22.3, 5.5, "x"), PAIR(22.6, 3.2, "y"),
        A("lean", 19.8, 2.6, "E"),
    ],
    "g2": [
        W("bed", "S", 27.2), W("sidetable", "S", 26.1), W("sidetable", "S", 28.3),
        W("wardrobe", "N", 27.8),
        W("dressing_table", "W", 6.2), F("chair", 25.55, 6.2, "W"),
        W("writing_table", "W", 1.8), F("chair", 25.75, 1.8, "W"),
        F("armchair", 28.2, 3.5, "E"),
        PAIR(27.0, 5.6, "x"), PAIR(26.5, 3.3, "y"),
        A("lean", 29.2, 5.3, "W"),
    ],
    "g3": [
        W("bed", "N", 2.8), W("sidetable", "N", 1.7), W("sidetable", "N", 3.9),
        W("wardrobe", "S", 4.6),
        W("dressing_table", "S", 1.0), F("chair", 1.0, 11.05, "S"),
        W("writing_table", "E", 11.6), F("chair", 4.25, 11.6, "E"),
        F("armchair", 1.9, 13.0, "W"),
        PAIR(3.3, 12.3, "x"), PAIR(4.6, 14.6, "y"),
        A("lean", 0.8, 14.9, "E"),
    ],
    "g4": [
        W("bed", "E", 12.5), W("sidetable", "E", 11.4), W("sidetable", "E", 13.6),
        W("wardrobe", "S", 6.4),
        W("dressing_table", "N", 7.6), F("chair", 7.6, 14.95, "N"),
        W("writing_table", "W", 14.6), F("chair", 6.75, 14.6, "W"),
        F("armchair", 9.7, 14.25, "N"),
        PAIR(7.6, 12.0, "y"), PAIR(9.9, 10.8, "x"),
        A("lean", 8.6, 15.2, "S"),
    ],
    "g5": [
        OBST(19.1, 12.8, 21.1, 14.2),
        A("bed", 20.1, 13.5, "E", z=0.55), A("hide", 19.5, 13.5, "E", z=0.55),
        W("sidetable", "W", 12.4), W("sidetable", "W", 14.6),
        W("wardrobe", "S", 19.8),
        W("dressing_table", "N", 20.5), F("chair", 20.5, 14.95, "N"),
        W("writing_table", "S", 23.5), F("chair", 23.5, 11.25, "S"),
        F("armchair", 22.9, 13.0, "E"),
        PAIR(22.6, 15.0, "x"), PAIR(21.7, 11.9, "x"),
        A("lean", 23.9, 14.9, "W"),
    ],
    "bath": [
        W("tub", "W", 14.0), W("chest", "E", 12.0, w=0.9), W("chest", "N", 26.2, w=0.8),
        W("chair", "E", 14.3), PIC("W", 11.5),
    ],
    "closet": [
        W("shelves", "W", 13.5, w=2.4, hide=1, hide_z=1.4), W("counter", "N", 28.5, w=1.2, surf=1),
        W("cupboard", "E", 14.0), W("shelves", "E", 12.0, w=1.0, h=0.5, surf=1, surf_z=0.5),
    ],
    "attic_corr": [
        A("stand", 19.8, 9.0, "E"), A("lean", 24.5, 8.4, "N"),
        A("lean", 24.5, 9.6, "S"), A("lean", 28.6, 8.4, "N"),
    ],
    "attic_maids": [
        W("sbed", "W", 4.5, hide=1), W("sbed", "E", 4.5),
        W("chest", "S", 21.75), W("chest", "N", 20.0, w=0.8),
        W("shelves", "N", 23.5, w=1.0, h=1.5), W("chair", "W", 6.8),
        A("stand", 21.75, 6.5, "S"),
    ],
    "attic_men": [
        W("sbed", "W", 4.5, hide=1), W("sbed", "E", 4.5),
        W("chest", "S", 27.25), W("chest", "N", 25.5, w=0.8),
        W("shelves", "N", 29.0, w=1.0, h=1.5), W("chair", "W", 6.8),
        A("stand", 27.25, 6.5, "S"),
    ],
    "attic_staff": [
        OBST(22.5, 12.4, 24.4, 13.3),
        A("bed", 23.45, 12.85, "W", z=0.5), A("hide", 24.0, 12.85, "W", z=0.5),
        W("chest", "N", 20.5), W("chest", "W", 11.0, w=0.8),
        W("sidetable", "E", 11.0), W("chair", "S", 23.3),
        A("stand", 21.0, 11.3, "N"),
    ],
    "boxroom": [
        W("trunk", "N", 25.5), W("trunk", "N", 27.0), W("trunk", "N", 28.5),
        W("trunk", "W", 12.0), W("shelves", "E", 12.0, h=1.2, hide=1, hide_z=0.4),
        A("stand", 27.25, 11.4, "N"),
    ],
    "cellar_pass": [A("stand", 32.5, 9.4, "W")],
    "cellar": [
        W("rack", "S", 28.5, hide=1, hide_z=0.6), W("rack", "S", 30.5), W("rack", "S", 32.5),
        W("rack", "S", 34.5, hide=1, hide_z=0.6),
        W("rack", "W", 3.0), W("rack", "W", 5.5), W("rack", "E", 3.0), W("rack", "E", 5.5),
        F("table", 32.0, 4.0, "N", w=1.2, d=1.2, h=0.8, surf=2),
        W("chest", "N", 28.5), W("chest", "N", 35.5),
        PAIR(30.0, 4.0, "y"), PAIR(34.0, 4.0, "y"), PAIR(33.6, 6.6, "x"),
    ],
    "garage": [
        F("car", 48.0, 21.8, "S"),
        W("counter", "N", 47.0, w=2.4, d=0.7, surf=3, drawer=1),
        W("cupboard", "E", 24.0, hide=1, hide_z=1.4), W("shelves", "E", 21.0),
        W("armchair", "W", 25.0), W("sidetable", "W", 24.2),
        PAIR(45.5, 20.3, "y"), PAIR(50.5, 19.5, "y"), PAIR(50.3, 22.5, "y"),
    ],
    "shed": [
        W("counter", "W", 33.0, w=2.0), W("shelves", "N", -2.0, hide=1, hide_z=1.4),
        W("chest", "S", -2.5), PIC("S", -1.0),
    ],
    "terrace": [
        W("gbench", "S", 17.0), W("gbench", "S", 28.0), W("gbench", "S", -6.5),
        PSEAT(20.5, 18.0, "x"), PSEAT(-1.5, 18.0, "x"),
        F("planter", 11.1, 19.38, "S"), F("planter", 14.9, 19.38, "S"),
        PAIR(9.5, 18.6, "x"),
        A("stand,window", 8.0, 17.0, "S"),
        A("lean", 5.0, 19.3, "N"), A("lean", 18.0, 19.3, "N"), A("lean", 26.0, 19.3, "N"),
    ],
    "garden": [
        OBST(3.0, 24.25, 7.0, 26.75), OBST(7.2, 35.3, 8.8, 35.8), OBST(17.2, 35.3, 18.8, 35.8),
        OBST(4.2, 23.15, 5.8, 23.65), OBST(-5.8, 24.2, -5.3, 25.8),
        OBST(12.75, 29.75, 13.25, 30.25), OBST(19.5, 34.8, 23.5, 35.7),
        A("seat", 7.6, 35.55, "S", z=0.45), A("seat", 8.4, 35.55, "S", z=0.45),
        A("seat", 17.6, 35.55, "S", z=0.45), A("seat", 18.4, 35.55, "S", z=0.45),
        A("seat", 4.6, 23.4, "N", z=0.45), A("seat", 5.4, 23.4, "N", z=0.45),
        A("seat", -5.55, 24.6, "E", z=0.45), A("seat", -5.55, 25.4, "E", z=0.45),
        A("surface", 13.0, 30.0, "S", z=0.94),
        A("surface", 20.5, 35.25, "S", z=0.5), A("surface", 22.5, 35.25, "S", z=0.5),
        A("hide", 21.5, 35.25, "S", z=0.2), A("hide", 5.0, 25.5, "N", z=0.25),
        PAIR(13.0, 26.0, "y"), PAIR(18.0, 30.0, "x"), PAIR(2.0, 30.0, "x"),
        A("stand", 22.5, 33.9, "S"), A("stand", -5.0, 35.0, "E"),
    ],
    "yard": [
        F("crate", 50.9, 3.0, "W"),
        PAIR(45.5, 9.0, "y"),
        A("lean", 42.6, 5.0, "E"), A("lean", 42.6, 13.0, "E"),
        A("stand", 47.0, 16.3, "N"), A("stand", 50.5, 12.0, "W"),
    ],
    "forecourt": [
        A("stand", 15.0, -1.6, "N"), PAIR(12.5, -3.0, "x"), PAIR(30.0, -4.0, "x"),
        A("stand", 0.0, -5.0, "N"), A("stand", 45.0, -4.0, "E"), A("lean", 20.0, -0.7, "S"),
    ],
    "path": [A("stand", 30.0, 30.0, "E"), A("stand", 38.0, 30.0, "W"), A("stand", 43.0, 25.0, "S")],
}
