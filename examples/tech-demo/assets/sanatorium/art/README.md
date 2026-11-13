# Sanatorium art

The models that dress the sanatorium graybox: an Alpine tuberculosis
sanatorium in the winter of 1928. Cold blue snow and night against warm
tungsten and amber lamps; white glazed tile, dark larch, cast iron and brass.
Every model is Hyper3D Rodin Gen-2.5 from a text prompt, realistic PBR with
base colour, normal and metallic-roughness maps.

| Path | What it is |
|---|---|
| `models.json` | Each model's Rodin generation id and the graybox pieces it stands for |
| `tools/library.py` | Fits a downloaded model to its size and writes it to `library/` |
| `library/` | One fitted `.glb` per model |

Nothing places these models in the level yet; the graybox is still what the
game draws.

## Fitting

```sh
flatpak run org.blender.Blender -b -P tools/library.py -- <downloads> library [name ...]
```

`<downloads>` holds `<name>/<name>.glb` as Rodin returned it, and
`<name>/<name>_emissive.png` for a model that glows. `MODELS` gives each model
either `size`, the box it is stretched to, or one of `width`, `depth` and
`height`, which scale it evenly. A model's front faces -Y, its width runs
along X, and its origin is on the floor at its centre unless `origin` says
`top` (pendants), `wall` (the back edge, for doors and curtains), `hang` (the
back face's centre, for things hung on a wall) or `centre`.

The graybox boxes are axis-aligned and some run their length along Y, such as
the director's desk and the porter's desk; whatever places the models turns
them to the box. Two models are smaller than their box on purpose: the boiler
is 3.4 m tall and 1.7 m wide, meant to stand in pairs across the 3.2 m boiler
plant, and the radiator is 0.73 m wide where its box is 1.15 m.

Downloads expire a week after Rodin returns them; a model's generation id in
`models.json` fetches it again from Hyper3D.
