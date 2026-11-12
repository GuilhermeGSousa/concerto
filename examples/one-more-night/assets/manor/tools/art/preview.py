import json
import math
import os

import bpy
from mathutils import Vector

LAMPS = [tuple(p) for ps in json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "lamps.json"))).values()
         for p in ps]
HELPERS = ("room", "nav", "acoustic", "anchor")


def render(path, eye, target, lens=20.0, width=1280, height=720):
    sc = bpy.context.scene
    art = {o.name[4:] for o in bpy.data.objects if o.name.startswith("art.")}
    hidden = []
    for ob in bpy.data.objects:
        helper = ob.get("gb_kind") in HELPERS or ob.name in art
        if helper and not ob.hide_render:
            ob.hide_render = True
            hidden.append(ob)
    made = []
    cam = bpy.data.objects.new("preview_cam", bpy.data.cameras.new("preview_cam"))
    cam.data.lens = lens
    cam.data.clip_start = 0.05
    cam.location = eye
    cam.rotation_euler = (Vector(target) - Vector(eye)).to_track_quat("-Z", "Y").to_euler()
    sc.collection.objects.link(cam)
    made.append(cam)
    for x, y, z in LAMPS:
        if (Vector((x, y, z)) - Vector(eye)).length > 16.0:
            continue
        data = bpy.data.lights.new("preview_lamp", "POINT")
        data.energy = 300.0
        data.color = (1.0, 0.78, 0.5)
        data.use_shadow = False
        lamp = bpy.data.objects.new("preview_lamp", data)
        lamp.location = (x, y, z + 0.2)
        sc.collection.objects.link(lamp)
        made.append(lamp)
    moon = bpy.data.objects.new("preview_moon", bpy.data.lights.new("preview_moon", "SUN"))
    moon.data.energy = 0.25
    moon.data.color = (0.55, 0.65, 1.0)
    moon.rotation_euler = (math.radians(50), 0, math.radians(30))
    sc.collection.objects.link(moon)
    made.append(moon)
    world = bpy.data.worlds.new("preview_world")
    world.color = (0.01, 0.013, 0.02)
    keep = (sc.camera, sc.world, sc.render.engine, sc.render.resolution_x, sc.render.resolution_y,
            sc.render.filepath, sc.render.image_settings.file_format)
    sc.camera, sc.world = cam, world
    sc.render.engine = "BLENDER_EEVEE"
    sc.render.resolution_x, sc.render.resolution_y = width, height
    sc.render.filepath = path
    sc.render.image_settings.file_format = "JPEG"
    try:
        bpy.ops.render.render(write_still=True)
    finally:
        (sc.camera, sc.world, sc.render.engine, sc.render.resolution_x, sc.render.resolution_y,
         sc.render.filepath, sc.render.image_settings.file_format) = keep
        for ob in made:
            data = ob.data
            bpy.data.objects.remove(ob)
            if isinstance(data, bpy.types.Light):
                bpy.data.lights.remove(data)
            else:
                bpy.data.cameras.remove(data)
        bpy.data.worlds.remove(world)
        for ob in hidden:
            ob.hide_render = False
