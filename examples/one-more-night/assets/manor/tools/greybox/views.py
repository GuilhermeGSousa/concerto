import math

import bpy
from mathutils import Euler, Vector

ORDER = ["cellar", "ground", "first", "attic"]
LEVEL_Z = {"cellar": -3.0, "ground": 0.0, "first": 3.8, "attic": 7.2}
EYE = 1.62
FRAMES = {
    "cellar": (31.5, 8.0, 22), "ground": (22.0, 13.0, 66),
    "first": (16.5, 8.0, 40), "attic": (26.0, 9.0, 20), "house": (17.0, 8.0, 46),
}


def _view3d():
    for win in bpy.context.window_manager.windows:
        for area in win.screen.areas:
            if area.type == "VIEW_3D":
                space = area.spaces.active
                return area, space, space.region_3d
    raise RuntimeError("no 3D viewport open")


def _style(space):
    sh = space.shading
    sh.type = "SOLID"
    sh.color_type = "MATERIAL"
    sh.light = "STUDIO"
    sh.show_cavity = True
    sh.cavity_type = "WORLD"
    sh.show_shadows = False
    space.overlay.show_overlays = False
    space.clip_start = 0.05
    space.clip_end = 500


def _visibility(level, hide_ceilings):
    for ob in bpy.data.objects:
        if not ob.get("generated"):
            continue
        if ob.get("gb_kind") == "room":
            ob.hide_set(True)
            continue
        lvl = ob.get("level")
        hide = lvl in ORDER and ORDER.index(lvl) > ORDER.index(level)
        if hide_ceilings and ob.get("gb_kind") == "ceiling" and lvl == level:
            hide = True
        ob.hide_set(hide)


def plan(level, frame=None):
    area, space, r3d = _view3d()
    _style(space)
    _visibility(level, hide_ceilings=True)
    cx, cy, dist = FRAMES[frame or level]
    r3d.view_perspective = "ORTHO"
    r3d.view_rotation = Euler((0, 0, 0)).to_quaternion()
    r3d.view_location = Vector((cx, cy, LEVEL_Z[level]))
    r3d.view_distance = dist
    area.tag_redraw()


def eye(x, y, level, yaw_deg, lens=35, floor=None, rooms=False):
    area, space, r3d = _view3d()
    _style(space)
    reset(rooms=rooms)
    z = (LEVEL_Z[level] if floor is None else floor) + EYE
    rot = Euler((math.radians(90), 0, math.radians(yaw_deg))).to_quaternion()
    d = 0.05
    r3d.view_perspective = "PERSP"
    space.lens = lens
    r3d.view_rotation = rot
    r3d.view_distance = d
    r3d.view_location = Vector((x, y, z)) + rot @ Vector((0, 0, -d))
    area.tag_redraw()


def maximise(on=True):
    win = bpy.context.window_manager.windows[0]
    area, space, r3d = _view3d()
    is_full = len(win.screen.areas) == 1
    if on != is_full:
        with bpy.context.temp_override(window=win, area=area):
            bpy.ops.screen.screen_full_area(use_hide_panels=True)
    for a in win.screen.areas:
        if a.type == "VIEW_3D":
            a.spaces.active.show_region_toolbar = not on
            a.spaces.active.show_region_header = not on
            a.spaces.active.show_region_ui = False


def clear_sections():
    for ob in list(bpy.data.objects):
        if ob.get("gb_temp"):
            me = ob.data
            bpy.data.objects.remove(ob)
            if me and me.users == 0:
                bpy.data.meshes.remove(me)


def section(level, cut=1.0, frame=None):
    import bmesh
    clear_sections()
    plan(level, frame)
    z = LEVEL_Z[level] + cut
    coll = bpy.context.scene.collection
    for ob in list(bpy.data.objects):
        if not ob.get("generated") or ob.get("gb_temp"):
            continue
        if ob.get("level") != level:
            continue
        if ob.get("gb_kind") == "wall":
            bm = bmesh.new()
            bm.from_mesh(ob.data)
            geom = bm.verts[:] + bm.edges[:] + bm.faces[:]
            bmesh.ops.bisect_plane(bm, geom=geom, plane_co=(0, 0, z), plane_no=(0, 0, 1),
                                   clear_outer=True)
            bmesh.ops.holes_fill(bm, edges=bm.edges[:], sides=0)
            me = bpy.data.meshes.new("_section." + ob.name)
            bm.to_mesh(me)
            bm.free()
            for m in ob.data.materials:
                me.materials.append(m)
            tmp = bpy.data.objects.new("_section." + ob.name, me)
            tmp["generated"] = True
            tmp["gb_temp"] = True
            coll.objects.link(tmp)
            ob.hide_set(True)
        elif ob.get("gb_kind") == "ceiling":
            ob.hide_set(True)
    _, _, r3d = _view3d()
    r3d.view_location.z = z


def volumes(level=None, frame=None):
    area, space, r3d = _view3d()
    _style(space)
    clear_sections()
    for ob in bpy.data.objects:
        if not ob.get("generated"):
            continue
        if ob.get("gb_kind") != "room":
            ob.hide_set(True)
            continue
        lvl = ob.get("level")
        show = level is None or lvl == level or (level == "ground" and lvl == "grounds")
        ob.display_type = "SOLID"
        ob.hide_set(not show)
    if level is None:
        r3d.view_perspective = "PERSP"
        space.lens = 35
        r3d.view_rotation = Euler((math.radians(58), 0, math.radians(35))).to_quaternion()
        r3d.view_location = Vector((20, 12, 2))
        r3d.view_distance = 48
    else:
        cx, cy, dist = FRAMES[frame or level]
        r3d.view_perspective = "ORTHO"
        r3d.view_rotation = Euler((0, 0, 0)).to_quaternion()
        r3d.view_location = Vector((cx, cy, LEVEL_Z[level]))
        r3d.view_distance = dist
    area.tag_redraw()


def swing(open_=True):
    for ob in bpy.data.objects:
        if ob.get("gb_kind") != "doorleaf":
            continue
        is_open = bool(ob.get("gb_preview_open"))
        if open_ == is_open:
            continue
        delta = math.radians(ob["open_angle"] * ob["swing"]) * (1 if open_ else -1)
        ob.rotation_euler.z += delta
        ob["gb_preview_open"] = open_


def reset(rooms=False):
    clear_sections()
    swing(False)
    for ob in bpy.data.objects:
        if not ob.get("generated"):
            continue
        if ob.get("gb_kind") == "room":
            ob.display_type = "WIRE"
            ob.hide_set(not rooms)
        else:
            ob.hide_set(False)


def feature(fid, on=True):
    scene = bpy.context.scene
    scene[fid] = bool(on)
    scene.update_tag()
    bpy.context.view_layer.update()
    for win in bpy.context.window_manager.windows:
        for area in win.screen.areas:
            area.tag_redraw()
