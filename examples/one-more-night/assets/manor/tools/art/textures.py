import os

import bpy
import numpy as np


def load(path):
    img = bpy.data.images.load(path)
    w, h = img.size
    px = np.empty(w * h * 4, dtype=np.float32)
    img.pixels.foreach_get(px)
    bpy.data.images.remove(img)
    return px.reshape(h, w, 4)[:, :, :3]


def save(path, rgb):
    h, w, _ = rgb.shape
    img = bpy.data.images.new("tmp_art_texture", w, h, alpha=False)
    px = np.ones((h, w, 4), dtype=np.float32)
    px[:, :, :3] = np.clip(rgb, 0.0, 1.0)
    img.pixels.foreach_set(px.ravel())
    img.filepath_raw = path
    img.file_format = "PNG"
    img.save()
    bpy.data.images.remove(img)


def tinted(src, colour, contrast):
    grey = src.mean(axis=2, keepdims=True)
    grey = (grey - grey.mean()) * contrast + 1.0
    return grey * np.array(colour, dtype=np.float32)


def damask(size, ground, figure):
    v, u = np.mgrid[0:size, 0:size].astype(np.float32) / size
    tau = 2.0 * np.pi
    lattice = np.cos(tau * u) + np.cos(tau * v)
    trellis = np.exp(-((lattice / 0.16) ** 2))
    ring = np.exp(-(((np.abs(lattice) - 1.05) / 0.12) ** 2))
    cu = (u + 0.5 * (lattice < 0)) % 1.0 - 0.5
    cv = (v + 0.5 * (lattice < 0)) % 1.0 - 0.5
    angle = np.arctan2(cv, cu)
    radius = np.hypot(cu, cv)
    petals = np.clip(1.0 - radius / (0.11 + 0.07 * np.cos(4.0 * angle) ** 2), 0.0, 1.0) ** 0.5
    leaves = np.exp(-(((radius - 0.27) / 0.035) ** 2)) * (np.cos(8.0 * angle) > 0.2)
    figure_mask = np.clip(trellis + 0.7 * ring + petals + 0.8 * leaves, 0.0, 1.0)[:, :, None]
    weave = 0.012 * np.sin(tau * 96.0 * u) * np.sin(tau * 96.0 * v)
    out = np.array(ground, dtype=np.float32) * (1.0 - figure_mask) + np.array(figure, dtype=np.float32) * figure_mask
    return out + weave[:, :, None]


def checker(marble, light, dark, grout):
    size = marble.shape[0]
    half = size // 2
    grey = marble.mean(axis=2, keepdims=True)
    veins = (grey - grey.mean()) / max(float(grey.std()), 1e-5)
    pale = np.array(light, dtype=np.float32) * (1.0 + 0.05 * veins)
    deep = np.array(dark, dtype=np.float32) * (1.0 - 0.35 * np.roll(veins, half // 2, axis=1))
    y, x = np.mgrid[0:size, 0:size]
    tiles = ((x // half) + (y // half)) % 2 == 0
    out = np.where(tiles[:, :, None], pale, deep)
    line = (x % half < 2) | (y % half < 2)
    out[line] = grout
    return out


def make(tex_dir):
    made = []

    def want(name):
        path = os.path.join(tex_dir, name)
        if os.path.exists(path):
            return None
        made.append(name)
        return path

    def src(name):
        return load(os.path.join(tex_dir, name))

    path = want("oak_dark_color.png")
    if path:
        save(path, tinted(src("oak_color.jpg"), (0.34, 0.2, 0.105), 1.4))
    path = want("paint_white_color.png")
    if path:
        save(path, tinted(src("plaster_color.jpg"), (0.80, 0.77, 0.69), 0.35))
    path = want("plaster_cream_color.png")
    if path:
        save(path, tinted(src("plaster_color.jpg"), (0.70, 0.62, 0.47), 0.5))
    path = want("ceiling_color.png")
    if path:
        save(path, tinted(src("plaster_color.jpg"), (0.83, 0.81, 0.75), 0.3))
    path = want("paper_sage_color.png")
    if path:
        save(path, damask(512, (0.49, 0.55, 0.45), (0.435, 0.5, 0.405)))
    papers = {
        "oxblood": ((0.36, 0.13, 0.12), (0.30, 0.10, 0.095)),
        "burgundy": ((0.33, 0.12, 0.16), (0.27, 0.095, 0.13)),
        "green": ((0.17, 0.27, 0.2), (0.14, 0.225, 0.165)),
        "rose": ((0.66, 0.5, 0.48), (0.6, 0.43, 0.42)),
        "blue": ((0.47, 0.55, 0.62), (0.41, 0.49, 0.57)),
        "yellow": ((0.72, 0.64, 0.42), (0.66, 0.58, 0.36)),
        "lilac": ((0.58, 0.52, 0.62), (0.52, 0.46, 0.57)),
        "peach": ((0.72, 0.56, 0.45), (0.66, 0.5, 0.39)),
    }
    for name, (ground, figure) in papers.items():
        path = want(f"paper_{name}_color.png")
        if path:
            save(path, damask(512, ground, figure))
    paints = {
        "paint_cream": ((0.74, 0.69, 0.55), 0.3),
        "paint_green": ((0.16, 0.25, 0.19), 0.3),
    }
    for name, (colour, contrast) in paints.items():
        path = want(f"{name}_color.png")
        if path:
            save(path, tinted(src("plaster_color.jpg"), colour, contrast))
    path = want("hedge_color.png")
    if path:
        save(path, tinted(src("grass_color.jpg"), (0.055, 0.11, 0.05), 2.2))
    path = want("velvet_red_color.png")
    if path:
        save(path, tinted(src("velvet_color.jpg"), (0.3, 0.055, 0.075), 1.0))
    path = want("velvet_green_color.png")
    if path:
        save(path, tinted(src("velvet_color.jpg"), (0.045, 0.16, 0.085), 1.0))
    path = want("marble_checker_color.png")
    if path:
        save(path, checker(src("marble_color.jpg"), (0.80, 0.78, 0.73), (0.06, 0.06, 0.065), (0.25, 0.24, 0.22)))
    return made
