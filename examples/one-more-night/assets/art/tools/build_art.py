import math
import os

import bmesh
import bpy
import numpy as np

ART = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TEXTURES = os.path.join(ART, "textures")
EXPORT = os.path.join(ART, "export")
SIZE = 512
WALL = 3.0


def clear():
    for collection in (
        bpy.data.objects,
        bpy.data.meshes,
        bpy.data.materials,
        bpy.data.images,
        bpy.data.lights,
        bpy.data.cameras,
    ):
        for block in list(collection):
            collection.remove(block)


def noise(seed, octaves=(4, 8, 16, 32, 64)):
    rng = np.random.default_rng(seed)
    out = np.zeros((SIZE, SIZE))
    for index, cells in enumerate(octaves):
        grid = rng.random((cells, cells))
        at = np.arange(SIZE) * cells / SIZE
        low = np.floor(at).astype(int)
        blend = at - low
        blend = blend * blend * (3 - 2 * blend)
        high = (low + 1) % cells
        across = blend[None, :]
        down = blend[:, None]
        top = grid[np.ix_(low, low)] * (1 - across) + grid[np.ix_(low, high)] * across
        bottom = grid[np.ix_(high, low)] * (1 - across) + grid[np.ix_(high, high)] * across
        out += (top * (1 - down) + bottom * down) / (2**index)
    return (out - out.min()) / (out.max() - out.min())


def save(name, rgb):
    pixels = np.ones((SIZE, SIZE, 4), dtype=np.float32)
    pixels[..., :3] = np.clip(rgb, 0.0, 1.0)
    image = bpy.data.images.new(name, SIZE, SIZE)
    image.pixels.foreach_set(pixels[::-1].ravel())
    image.filepath_raw = os.path.join(TEXTURES, name + ".png")
    image.file_format = "PNG"
    image.save()
    return image


def tint(value, colour):
    return value[..., None] * np.array(colour)[None, None, :]


def wood_texture(name, colour, seed):
    grain = noise(seed, (64, 128, 256))[0, :]
    body = noise(seed + 1)
    value = 0.62 + 0.3 * grain[None, :] + 0.16 * body
    return save(name, tint(value, colour))


def parquet_texture(name, seed):
    rng = np.random.default_rng(seed)
    cell = SIZE // 4
    plank = cell // 4
    grain = noise(seed, (64, 128, 256))
    rgb = np.zeros((SIZE, SIZE, 3))
    for row in range(4):
        for column in range(4):
            upright = (row + column) % 2 == 0
            for strip in range(4):
                shade = 0.72 + 0.28 * rng.random()
                colour = np.array([0.42, 0.26, 0.13]) * shade
                y0, x0 = row * cell, column * cell
                if upright:
                    ys, xs = slice(y0, y0 + cell), slice(x0 + strip * plank, x0 + (strip + 1) * plank)
                    streak = grain[0, xs][None, :]
                else:
                    ys, xs = slice(y0 + strip * plank, y0 + (strip + 1) * plank), slice(x0, x0 + cell)
                    streak = grain[ys, 0][:, None]
                block = np.ones((ys.stop - ys.start, xs.stop - xs.start))
                rgb[ys, xs] = tint(block * (0.85 + 0.25 * streak), colour)
                rgb[ys.start : ys.start + 1, xs] *= 0.45
                rgb[ys, xs.start : xs.start + 1] *= 0.45
    return save(name, rgb)


def wallpaper_texture(name, colour, accent, seed):
    ys, xs = np.mgrid[0:SIZE, 0:SIZE]
    stripe = np.where((xs // 64) % 2 == 0, 1.0, 0.86)
    value = stripe * (0.94 + 0.06 * noise(seed, (32, 64, 128)))
    rgb = tint(value, colour)
    dx = np.abs((xs % 128) - 64)
    dy = np.abs((ys % 128) - 64)
    motif = (dx + dy < 14) & (dx + dy > 6)
    rgb[motif] = np.array(accent)
    return save(name, rgb)


def plaster_texture(name, colour, seed):
    return save(name, tint(0.95 + 0.05 * noise(seed, (32, 64, 128, 256)), colour))


def tile_texture(name, seed):
    ys, xs = np.mgrid[0:SIZE, 0:SIZE]
    half = SIZE // 2
    dark = ((xs // half) + (ys // half)) % 2 == 0
    value = 0.9 + 0.1 * noise(seed)
    rgb = np.where(dark[..., None], tint(value, (0.1, 0.1, 0.11)), tint(value, (0.78, 0.74, 0.66)))
    grout = (xs % half < 3) | (ys % half < 3)
    rgb[grout] = np.array((0.3, 0.29, 0.27))
    return save(name, rgb)


def rug_texture(name):
    ys, xs = np.mgrid[0:SIZE, 0:SIZE]
    edge = np.minimum(np.minimum(xs, SIZE - 1 - xs), np.minimum(ys, SIZE - 1 - ys))
    rgb = tint(0.9 + 0.1 * noise(21), (0.42, 0.08, 0.09))
    rgb[(edge > 24) & (edge < 40)] = np.array((0.72, 0.55, 0.22))
    rgb[(edge > 52) & (edge < 58)] = np.array((0.72, 0.55, 0.22))
    rgb[edge <= 24] = np.array((0.16, 0.2, 0.3))
    return save(name, rgb)


def material(name, colour=(1, 1, 1), image=None, roughness=0.7, metallic=0.0, emission=None):
    made = bpy.data.materials.new(name)
    made.use_nodes = True
    shader = made.node_tree.nodes["Principled BSDF"]
    shader.inputs["Base Color"].default_value = (*colour, 1.0)
    shader.inputs["Roughness"].default_value = roughness
    shader.inputs["Metallic"].default_value = metallic
    if image is not None:
        node = made.node_tree.nodes.new("ShaderNodeTexImage")
        node.image = image
        made.node_tree.links.new(node.outputs["Color"], shader.inputs["Base Color"])
    if emission is not None:
        shader.inputs["Emission Color"].default_value = (*emission, 1.0)
        shader.inputs["Emission Strength"].default_value = 4.0
    return made


def unwrap(obj, tile, fit=False):
    mesh = obj.data
    layer = mesh.uv_layers.new(name="UVMap")
    origin = obj.location
    for polygon in mesh.polygons:
        normal = polygon.normal
        axis = max(range(3), key=lambda index: abs(normal[index]))
        for loop in polygon.loop_indices:
            vertex = mesh.vertices[mesh.loops[loop].vertex_index].co
            world = (vertex.x + origin.x, vertex.y + origin.y, vertex.z + origin.z)
            local = (vertex.x, vertex.y, vertex.z)
            point = local if fit else world
            if axis == 2:
                uv = (point[0], point[1])
            elif axis == 0:
                uv = (point[1], point[2])
            else:
                uv = (point[0], point[2])
            if fit:
                size = obj.dimensions
                spans = (size.x, size.y) if axis == 2 else ((size.y, size.z) if axis == 0 else (size.x, size.z))
                layer.data[loop].uv = (uv[0] / spans[0] + 0.5, uv[1] / spans[1] + 0.5)
            else:
                layer.data[loop].uv = (uv[0] / tile, uv[1] / tile)


def link(obj, group):
    group.objects.link(obj)
    return obj


def box(group, name, centre, size, mat, tile=1.0, fit=False):
    mesh = bpy.data.meshes.new(name)
    build = bmesh.new()
    bmesh.ops.create_cube(build, size=1.0)
    for vertex in build.verts:
        vertex.co.x *= size[0]
        vertex.co.y *= size[1]
        vertex.co.z *= size[2]
    build.to_mesh(mesh)
    build.free()
    obj = bpy.data.objects.new(name, mesh)
    obj.location = centre
    mesh.materials.append(mat)
    link(obj, group)
    bpy.context.view_layer.update()
    unwrap(obj, tile, fit)
    return obj


def span(group, name, low, high, mat, tile=1.0):
    centre = tuple((a + b) / 2 for a, b in zip(low, high))
    size = tuple(b - a for a, b in zip(low, high))
    return box(group, name, centre, size, mat, tile)


def round_shape(group, name, centre, radius, depth, mat, segments=16, sphere=False, axis="Z"):
    mesh = bpy.data.meshes.new(name)
    build = bmesh.new()
    if sphere:
        bmesh.ops.create_uvsphere(build, u_segments=segments, v_segments=segments // 2, radius=radius)
    else:
        bmesh.ops.create_cone(
            build, cap_ends=True, segments=segments, radius1=radius, radius2=radius, depth=depth
        )
    build.to_mesh(mesh)
    build.free()
    for polygon in mesh.polygons:
        polygon.use_smooth = True
    obj = bpy.data.objects.new(name, mesh)
    obj.location = centre
    if axis == "Y":
        obj.rotation_euler = (math.pi / 2, 0, 0)
    mesh.materials.append(mat)
    mesh.uv_layers.new(name="UVMap")
    return link(obj, group)


def collection(name):
    made = bpy.data.collections.new(name)
    bpy.context.scene.collection.children.link(made)
    return made


def export(group, name):
    bpy.ops.object.select_all(action="DESELECT")
    for obj in group.objects:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = group.objects[0]
    bpy.ops.export_scene.gltf(
        filepath=os.path.join(EXPORT, name + ".glb"),
        export_format="GLB",
        use_selection=True,
        export_apply=True,
        export_yup=True,
    )


def build():
    os.makedirs(TEXTURES, exist_ok=True)
    os.makedirs(EXPORT, exist_ok=True)
    clear()

    oak = material("oak", image=wood_texture("oak", (0.5, 0.3, 0.16), 1), roughness=0.55)
    mahogany = material("mahogany", image=wood_texture("mahogany", (0.3, 0.12, 0.08), 2), roughness=0.4)
    parquet = material("parquet", image=parquet_texture("parquet", 3), roughness=0.45)
    green = material(
        "wallpaper_green",
        image=wallpaper_texture("wallpaper_green", (0.16, 0.32, 0.23), (0.7, 0.56, 0.26), 4),
        roughness=0.9,
    )
    red = material(
        "wallpaper_red",
        image=wallpaper_texture("wallpaper_red", (0.42, 0.11, 0.12), (0.72, 0.56, 0.28), 5),
        roughness=0.9,
    )
    plaster = material("plaster", image=plaster_texture("plaster", (0.74, 0.7, 0.62), 6), roughness=0.95)
    ceiling = material("ceiling", image=plaster_texture("ceiling", (0.82, 0.8, 0.75), 7), roughness=0.95)
    tiles = material("tiles", image=tile_texture("tiles", 8), roughness=0.35)
    rug = material("rug", image=rug_texture("rug"), roughness=1.0)
    brass = material("brass", (0.83, 0.62, 0.24), roughness=0.3, metallic=1.0)
    iron = material("iron", (0.08, 0.08, 0.09), roughness=0.5, metallic=1.0)
    velvet = material("velvet", (0.45, 0.06, 0.08), roughness=1.0)
    linen = material("linen", (0.86, 0.84, 0.78), roughness=1.0)
    paper = material("paper", (0.88, 0.83, 0.7), roughness=0.9)
    porcelain = material("porcelain", (0.92, 0.92, 0.95), roughness=0.15)
    wicker = material("wicker", (0.6, 0.44, 0.24), roughness=0.9)
    dark = material("shadow", (0.02, 0.02, 0.025), roughness=1.0)
    glow = material("lamp_glow", (1.0, 0.85, 0.6), roughness=0.6, emission=(1.0, 0.72, 0.4))

    level = collection("level_test")
    t = 0.2
    span(level, "floor_drawing_room", (0, 0, -0.1), (6, 6, 0), parquet, 1.2)
    span(level, "floor_study", (6, 0, -0.1), (12, 6, 0), parquet, 1.2)
    span(level, "floor_corridor", (-t, 6, -0.1), (12 + t, 9 + t, 0), tiles, 1.0)
    span(level, "floor_apron", (-t, -t, -0.1), (12 + t, 0, 0), parquet, 1.2)
    span(level, "ceiling", (-t, -t, WALL), (12 + t, 9 + t, WALL + 0.1), ceiling, 2.0)
    span(level, "wall_south_a", (-t, -t, 0), (6, 0, WALL), green, 0.64)
    span(level, "wall_south_b", (6, -t, 0), (12 + t, 0, WALL), red, 0.64)
    span(level, "wall_west_a", (-t, 0, 0), (0, 6, WALL), green, 0.64)
    span(level, "wall_west_c", (-t, 6, 0), (0, 9 + t, WALL), plaster, 1.5)
    span(level, "wall_east_b", (12, 0, 0), (12 + t, 6, WALL), red, 0.64)
    span(level, "wall_east_c", (12, 6, 0), (12 + t, 9 + t, WALL), plaster, 1.5)
    span(level, "wall_north", (0, 9, 0), (12, 9 + t, WALL), plaster, 1.5)
    half = t / 2
    for side, low_x, high_x, paper_wall in (("a", 6 - half, 6, green), ("b", 6, 6 + half, red)):
        span(level, f"wall_rooms_south_{side}", (low_x, 0, 0), (high_x, 2.5, WALL), paper_wall, 0.64)
        span(level, f"wall_rooms_north_{side}", (low_x, 3.5, 0), (high_x, 6, WALL), paper_wall, 0.64)
        span(level, f"lintel_rooms_{side}", (low_x, 2.5, 2.2), (high_x, 3.5, WALL), paper_wall, 0.64)
    for low_x, high_x in ((0, 2.5), (3.5, 8.5), (9.5, 12)):
        for piece_low, piece_high in ((low_x, min(high_x, 6)), (max(low_x, 6), high_x)):
            if piece_high <= piece_low:
                continue
            paper_wall = green if piece_high <= 6 else red
            span(level, f"wall_corridor_room_{piece_low}", (piece_low, 6 - half, 0), (piece_high, 6, WALL), paper_wall, 0.64)
        span(level, f"wall_corridor_{low_x}", (low_x, 6, 0), (high_x, 6 + half, WALL), plaster, 1.5)
    for door_x, paper_wall in ((3, green), (9, red)):
        span(level, f"lintel_room_{door_x}", (door_x - 0.5, 6 - half, 2.2), (door_x + 0.5, 6, WALL), paper_wall, 0.64)
        span(level, f"lintel_corridor_{door_x}", (door_x - 0.5, 6, 2.2), (door_x + 0.5, 6 + half, WALL), plaster, 1.5)
    skirt = 0.14
    for name, low, high in (
        ("skirt_south", (0, 0, 0), (12, 0.03, skirt)),
        ("skirt_west", (0, 0, 0), (0.03, 6, skirt)),
        ("skirt_east", (11.97, 0, 0), (12, 6, skirt)),
        ("skirt_rooms_south", (0, 5.87, 0), (2.5, 5.9, skirt)),
        ("skirt_rooms_middle", (3.5, 5.87, 0), (8.5, 5.9, skirt)),
        ("skirt_rooms_north", (9.5, 5.87, 0), (12, 5.9, skirt)),
    ):
        span(level, name, low, high, mahogany, 0.8)
    box(level, "rug", (3.0, 2.15, 0.008), (4.0, 2.9, 0.015), rug, fit=True)
    for name, low, high in (
        ("table", (4.15, 0.9, 0.74), (5.05, 1.5, 0.8)),
        ("desk", (10.3, 0.9, 0.74), (11.3, 1.5, 0.8)),
        ("shelf", (11.1, 4.15, 0.74), (11.7, 5.05, 0.8)),
        ("key_shelf", (4.7, 8.4, 0.74), (5.3, 8.8, 0.8)),
    ):
        span(level, name + "_top", low, high, mahogany, 0.8)
        for corner_x in (low[0] + 0.05, high[0] - 0.05):
            for corner_y in (low[1] + 0.05, high[1] - 0.05):
                box(level, f"{name}_leg", (corner_x, corner_y, 0.37), (0.06, 0.06, 0.74), mahogany, 0.8)
    span(level, "basket_stand", (10.8, 8.1, 0.0), (11.6, 8.7, 0.4), oak, 0.8)
    for name, seat_x, back_x in (("chair_west", 1.5, 1.27), ("chair_east", 2.7, 2.93)):
        box(level, name + "_seat", (seat_x, 1.5, 0.41), (0.5, 0.5, 0.06), velvet, 1.0)
        box(level, name + "_frame", (seat_x, 1.5, 0.19), (0.44, 0.44, 0.38), mahogany, 0.8)
        box(level, name + "_back", (back_x, 1.5, 0.75), (0.05, 0.5, 0.7), mahogany, 0.8)
    for name, at in (("lamp_drawing_room", (3, 3)), ("lamp_study", (9, 3)), ("lamp_corridor", (6, 7.5))):
        round_shape(level, name + "_shade", (at[0], at[1], 2.72), 0.14, 0, glow, sphere=True)
        round_shape(level, name + "_rod", (at[0], at[1], 2.9), 0.012, 0.22, brass)
    export(level, "level_test")

    def kit(name, parts):
        group = collection("kit_" + name)
        parts(group)
        export(group, "kit_" + name)
        return group

    def door(group):
        box(group, "door_panel", (0, 0, 0), (1.0, 0.06, 2.2), oak, 0.9)
        for inset_z in (-0.55, 0.5):
            for face in (-0.035, 0.035):
                box(group, "door_inset", (0, face, inset_z), (0.7, 0.012, 0.8), mahogany, 0.9)
        for face in (-0.07, 0.07):
            round_shape(group, "door_knob", (0.4, face, -0.05), 0.04, 0, brass, sphere=True)

    def bell_pull(group):
        box(group, "pull_plate", (0, 0, 0.27), (0.08, 0.02, 0.06), brass)
        box(group, "pull_cord", (0, 0, 0.03), (0.035, 0.035, 0.44), velvet)
        box(group, "pull_tassel", (0, 0, -0.24), (0.07, 0.07, 0.12), brass)

    def bell_board(group):
        box(group, "board", (0, 0, 0), (1.2, 0.05, 0.6), mahogany, 0.8)
        for index in range(3):
            at_x = -0.4 + index * 0.4
            round_shape(group, "board_bell", (at_x, -0.05, 0.12), 0.06, 0, brass, sphere=True)
            box(group, "board_flag", (at_x, -0.03, -0.14), (0.14, 0.012, 0.1), paper)

    def gong(group):
        for post_x in (-0.3, 0.3):
            box(group, "gong_post", (post_x, 0, 0), (0.06, 0.06, 1.2), mahogany, 0.8)
            box(group, "gong_foot", (post_x, 0, -0.57), (0.1, 0.34, 0.06), mahogany, 0.8)
        box(group, "gong_bar", (0, 0, 0.57), (0.7, 0.06, 0.06), mahogany, 0.8)
        round_shape(group, "gong_disc", (0, 0, 0.12), 0.25, 0.025, brass, segments=24, axis="Y")

    def bed(group):
        box(group, "bed_frame", (0, 0, -0.12), (1.0, 1.6, 0.26), mahogany, 0.8)
        box(group, "bed_mattress", (0, 0.02, 0.08), (0.94, 1.5, 0.16), linen)
        box(group, "bed_blanket", (0, -0.3, 0.17), (0.96, 0.9, 0.03), velvet)
        box(group, "bed_pillow", (0, 0.55, 0.2), (0.6, 0.32, 0.08), linen)
        box(group, "bed_head", (0, 0.78, 0.15), (1.0, 0.05, 0.6), mahogany, 0.8)

    def lift(group):
        box(group, "lift_cabinet", (0, 0, 0), (0.7, 0.5, 1.0), oak, 0.8)
        box(group, "lift_opening", (0, -0.251, 0.1), (0.5, 0.01, 0.6), dark)
        box(group, "lift_sill", (0, -0.27, -0.22), (0.56, 0.05, 0.03), brass)
        round_shape(group, "lift_button", (0.29, -0.26, 0.3), 0.025, 0, brass, sphere=True)

    def prop_case(group):
        box(group, "case_base", (0, 0, -0.135), (0.5, 0.3, 0.08), mahogany, 0.6)
        box(group, "case_cushion", (0, 0, -0.08), (0.42, 0.22, 0.03), velvet)
        for corner_x in (-0.235, 0.235):
            for corner_y in (-0.135, 0.135):
                box(group, "case_post", (corner_x, corner_y, 0.03), (0.015, 0.015, 0.26), brass)
        box(group, "case_lid", (0, 0, 0.165), (0.5, 0.3, 0.02), brass)

    def prop_note(group):
        box(group, "note_paper", (0, 0, 0), (0.22, 0.3, 0.006), paper)
        for line in range(5):
            box(group, "note_ink", (0, 0.09 - line * 0.04, 0.004), (0.16, 0.006, 0.001), dark)

    def prop_basket(group):
        box(group, "basket_body", (0, 0, -0.03), (0.6, 0.45, 0.39), wicker)
        box(group, "basket_rim", (0, 0, 0.17), (0.64, 0.49, 0.03), oak, 0.6)
        box(group, "basket_linen", (0, 0, 0.2), (0.54, 0.39, 0.05), linen)

    def prop_key(group):
        round_shape(group, "key_bow", (-0.045, 0, 0), 0.025, 0.008, brass, segments=12)
        box(group, "key_shaft", (0.02, 0, 0), (0.1, 0.01, 0.008), brass)
        box(group, "key_bit", (0.06, -0.012, 0), (0.016, 0.02, 0.008), brass)

    def prop_cup(group):
        round_shape(group, "cup_saucer", (0, 0, -0.03), 0.07, 0.01, porcelain)
        round_shape(group, "cup_body", (0, 0, 0.005), 0.04, 0.06, porcelain)
        round_shape(group, "cup_tea", (0, 0, 0.036), 0.034, 0.002, mahogany)

    offset = 16.0
    for name, parts in (
        ("door", door),
        ("bell_pull", bell_pull),
        ("bell_board", bell_board),
        ("gong", gong),
        ("bed", bed),
        ("lift", lift),
        ("prop_case", prop_case),
        ("prop_note", prop_note),
        ("prop_basket", prop_basket),
        ("prop_key", prop_key),
        ("prop_cup", prop_cup),
    ):
        group = kit(name, parts)
        for obj in group.objects:
            obj.location.x += offset
        offset += 2.0

    bpy.ops.wm.save_as_mainfile(filepath=os.path.join(ART, "art.blend"))
    return sorted(os.listdir(EXPORT))


BUILT = build()
