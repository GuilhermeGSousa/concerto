import math
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.cu2quPen import Cu2QuPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
INTER = ROOT / "crates/editor/fonts/Inter/Inter-SemiBold.ttf"
BRAND_FACE = ROOT / "crates/editor/fonts/Concerto/ConcertoMark.ttf"

CENTRE = 32.0
OUTER = {"radius": 25.0, "width": 6.0, "half_gap": 48.0}
INNER = {"radius": 15.5, "width": 5.0, "half_gap": 42.0}
DOT = 5.2
INNER_OPACITY = 0.72

LIGHT = "#b5abfc"
DEEP = "#7f6fd6"
ON_LIGHT = "#5d4fc0"
INK_ON_DARK = "#e9e9ed"
INK_ON_LIGHT = "#101220"

EM = 1024
ASCENT = 960
DESCENT = -64
GLYPH_SCALE = 832.0 / 56.0
GLYPH_CENTRE_X = 512.0
GLYPH_CENTRE_Y = 448.0
MARK_CODEPOINT = 0xE000


def point(radius, degrees):
    angle = math.radians(degrees)
    return (CENTRE + radius * math.cos(angle), CENTRE + radius * math.sin(angle))


def arc_to(pen, centre, radius, start, end):
    steps = max(1, math.ceil(abs(end - start) / 90.0))
    step = math.radians(end - start) / steps
    handle = 4.0 / 3.0 * math.tan(step / 4.0) * radius
    angle = math.radians(start)
    for _ in range(steps):
        following = angle + step
        pen.curveTo(
            (
                centre[0] + radius * math.cos(angle) - handle * math.sin(angle),
                centre[1] + radius * math.sin(angle) + handle * math.cos(angle),
            ),
            (
                centre[0] + radius * math.cos(following) + handle * math.sin(following),
                centre[1] + radius * math.sin(following) - handle * math.cos(following),
            ),
            (
                centre[0] + radius * math.cos(following),
                centre[1] + radius * math.sin(following),
            ),
        )
        angle = following


def draw_band(pen, band):
    radius = band["radius"]
    half = band["width"] / 2.0
    first = band["half_gap"]
    last = 360.0 - band["half_gap"]
    middle = (CENTRE, CENTRE)
    pen.moveTo(point(radius + half, first))
    arc_to(pen, middle, radius + half, first, last)
    arc_to(pen, point(radius, last), half, last, last + 180.0)
    arc_to(pen, middle, radius - half, last, first)
    arc_to(pen, point(radius, first), half, first + 180.0, first + 360.0)
    pen.closePath()


def draw_dot(pen):
    pen.moveTo((CENTRE + DOT, CENTRE))
    arc_to(pen, (CENTRE, CENTRE), DOT, 0.0, 360.0)
    pen.closePath()


def path_of(draw):
    pen = SVGPathPen(None, ntos=lambda value: f"{value:.2f}".rstrip("0").rstrip("."))
    draw(pen)
    return pen.getCommands()


def mark_body(fill):
    return (
        f'<path d="{path_of(lambda pen: draw_band(pen, OUTER))}" fill="{fill}"/>'
        f'<path d="{path_of(lambda pen: draw_band(pen, INNER))}" fill="{fill}" opacity="{INNER_OPACITY}"/>'
        f'<path d="{path_of(draw_dot)}" fill="{fill}"/>'
    )


def gradient():
    return (
        '<defs><linearGradient id="concerto" x1="0" y1="0" x2="1" y2="1">'
        f'<stop offset="0" stop-color="{LIGHT}"/><stop offset="1" stop-color="{DEEP}"/>'
        "</linearGradient></defs>"
    )


def mark_svg():
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="64" height="64">'
        f'<title>Concerto</title>{gradient()}{mark_body("url(#concerto)")}</svg>\n'
    )


def wordmark(size, tracking):
    font = TTFont(INTER)
    glyphs = font.getGlyphSet()
    cmap = font.getBestCmap()
    scale = size / font["head"].unitsPerEm
    pen = SVGPathPen(glyphs, ntos=lambda value: f"{value:.2f}".rstrip("0").rstrip("."))
    cursor = 0.0
    for letter in "Concerto":
        name = cmap[ord(letter)]
        glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, cursor, 0)))
        cursor += font["hmtx"][name][0] * scale + tracking * size
    return {"path": pen.getCommands(), "width": cursor - tracking * size}


def lockup_svg(mark_fill, ink, with_gradient):
    word = wordmark(46.0, -0.02)
    width = 64 + 16 + word["width"]
    defs = gradient() if with_gradient else ""
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width:.0f} 64" width="{width:.0f}" height="64">'
        f"<title>Concerto</title>{defs}{mark_body(mark_fill)}"
        f'<path transform="translate(80 48.5)" d="{word["path"]}" fill="{ink}"/></svg>\n'
    )


def brand_face():
    glyph = TTGlyphPen(None)
    quadratic = Cu2QuPen(glyph, 1.0, reverse_direction=True)
    placed = TransformPen(
        quadratic,
        (
            GLYPH_SCALE,
            0,
            0,
            -GLYPH_SCALE,
            GLYPH_CENTRE_X - CENTRE * GLYPH_SCALE,
            GLYPH_CENTRE_Y + CENTRE * GLYPH_SCALE,
        ),
    )
    draw_band(placed, OUTER)
    draw_band(placed, INNER)
    draw_dot(placed)
    builder = FontBuilder(EM, isTTF=True)
    builder.setupGlyphOrder([".notdef", "mark"])
    builder.setupCharacterMap({MARK_CODEPOINT: "mark"})
    builder.setupGlyf({".notdef": TTGlyphPen(None).glyph(), "mark": glyph.glyph()})
    builder.setupHorizontalMetrics({".notdef": (EM, 0), "mark": (EM, 96)})
    builder.setupHorizontalHeader(ascent=ASCENT, descent=DESCENT)
    builder.setupNameTable(
        {
            "familyName": "Concerto Mark",
            "styleName": "Regular",
            "uniqueFontIdentifier": "Concerto Mark Regular",
            "fullName": "Concerto Mark",
            "psName": "ConcertoMark-Regular",
        }
    )
    builder.setupOS2(sTypoAscender=ASCENT, sTypoDescender=DESCENT, usWinAscent=ASCENT, usWinDescent=-DESCENT)
    builder.setupPost()
    BRAND_FACE.parent.mkdir(parents=True, exist_ok=True)
    builder.save(BRAND_FACE)


def main():
    (HERE / "concerto-mark.svg").write_text(mark_svg())
    (HERE / "concerto-logo-dark.svg").write_text(lockup_svg("url(#concerto)", INK_ON_DARK, True))
    (HERE / "concerto-logo-light.svg").write_text(lockup_svg(ON_LIGHT, INK_ON_LIGHT, False))
    brand_face()


if __name__ == "__main__":
    main()
