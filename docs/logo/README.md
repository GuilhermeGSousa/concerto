# Concerto's logo

The mark is a C drawn as two arcs around a dot: one player in front of an
ensemble.

| File | What it is |
|---|---|
| `concerto-mark.svg` | The mark alone, 64 x 64, in the violet gradient |
| `concerto-logo-dark.svg` | Mark and wordmark for dark backgrounds |
| `concerto-logo-light.svg` | Mark and wordmark for light backgrounds |
| `build.py` | Writes the three files above and the editor's brand face |

The wordmark is Inter SemiBold, converted to outlines so the files need no
font. The editor draws the mark from
`crates/editor/fonts/Concerto/ConcertoMark.ttf`, a face with that one glyph at
U+E000, in the theme's accent colour.

All of it is generated. To change the logo, edit the geometry and colours at
the top of `build.py` and run it from the repository root, with `fontTools`
installed:

```sh
python3 docs/logo/build.py
```
