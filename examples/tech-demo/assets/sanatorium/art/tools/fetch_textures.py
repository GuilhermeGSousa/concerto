import json
import os
import sys
import urllib.request

SURFACES = {
    "snow_02": "snow",
    "snow_floor": "snow_packed",
    "dark_rock": "rock",
    "white_stucco": "stucco",
    "rustic_stone_wall": "stone",
    "dark_wooden_planks": "larch",
    "rusty_painted_metal": "metal",
    "roof_slates_02": "slate",
    "long_white_tiles": "wall_tile",
    "painted_plaster_wall": "wall_paint",
    "grey_plaster_02": "ceiling",
    "terrazzo_tiles": "terrazzo",
    "concrete_floor": "concrete",
    "old_wood_floor": "wood_floor",
    "red_brick": "brick",
}
MAPS = {"Diffuse": "color", "nor_gl": "normal", "Rough": "rough"}
AGENT = {"User-Agent": "concerto-art-fetch/1.0"}


def get(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers=AGENT)).read()


def main(out_dir):
    os.makedirs(out_dir, exist_ok=True)
    for asset, name in SURFACES.items():
        files = json.loads(get(f"https://api.polyhaven.com/files/{asset}"))
        for key, suffix in MAPS.items():
            path = os.path.join(out_dir, f"{name}_{suffix}.jpg")
            if os.path.exists(path) or key not in files:
                continue
            with open(path, "wb") as f:
                f.write(get(files[key]["1k"]["jpg"]["url"]))
            print(path)


if __name__ == "__main__":
    main(sys.argv[1])
