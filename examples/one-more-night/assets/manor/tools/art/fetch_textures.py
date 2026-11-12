import json
import os
import sys
import urllib.request

SURFACES = {
    "herringbone_parquet": "parquet",
    "marble_01": "marble",
    "dark_wood": "oak",
    "painted_plaster_wall": "plaster",
    "wood_floor": "boards",
}
MAPS = {"Diffuse": "color", "nor_gl": "normal", "Rough": "rough"}
AGENT = {"User-Agent": "manor-art-fetch/1.0"}


def get(url):
    return urllib.request.urlopen(urllib.request.Request(url, headers=AGENT)).read()


def main(out_dir):
    os.makedirs(out_dir, exist_ok=True)
    for asset, name in SURFACES.items():
        files = json.loads(get(f"https://api.polyhaven.com/files/{asset}"))
        for key, suffix in MAPS.items():
            path = os.path.join(out_dir, f"{name}_{suffix}.jpg")
            if os.path.exists(path):
                continue
            with open(path, "wb") as f:
                f.write(get(files[key]["1k"]["jpg"]["url"]))
            print(path)


if __name__ == "__main__":
    main(sys.argv[1])
