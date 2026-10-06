#!/usr/bin/env python3
"""Strip TrueType hinting out of the bundled woff2 faces.

Every bundled face shipped Fira Code's full hinting bytecode: a 3.6KB `fpgm`,
a `prep`, a `cvt `, and a `gasp` asking for grid-fitting at every ppem with no
large-ppem escape hatch. CoreText, which macOS uses, ignores that bytecode
entirely, so the app already looked right there. FreeType, which Linux uses,
executes it, and at 1x a 14px cell is exactly where snapping stems to the pixel
grid shows: uneven stem weight between glyphs, and under fontconfig's default
`hintmetrics=hintfull`, advances rounded to whole pixels so the terminal cell
drifts from the macOS one.

Removing the bytecode leaves both platforms rasterizing the same unhinted
outlines -- which is what macOS already drew, and what `src/marvis.css` says
bundling these faces is for. Outlines, advance widths and the GSUB/GPOS
features Fira Code's ligatures depend on are left untouched.

Inputs are the prepared woff2 files described in `src/assets/fonts/README.md`:
the Fira Code 6.2 release `woff2/` folder, and the already-subset
NerdSymbols.woff2. Re-running this on an already-stripped file rewrites the
same bytes, so it doubles as the check that the committed fonts are hint-free.

Note that emptying a glyph's `program` is not enough: `_g_l_y_f.py` decides
whether to emit an instruction block from `hasattr(glyph, "program")`, so the
attribute has to go. `removeHinting()` is what fontTools' own subsetter uses
for `--no-hinting`, and it only removes the instructions when the glyph still
holds its unexpanded `data` -- so this script never touches `getGlyphSet()` or
any other expanding accessor before trimming.

`recalcBBoxes=False` is what keeps the outlines bit-for-bit. With it on, `glyf`
expands every glyph and renormalizes the bounds, and in NerdSymbols 787 icons
whose cached `xMin` sits one unit away from their real leftmost point come back
moved by that unit. With it off, `glyf.compile` writes each trimmed `data`
verbatim, so the only bytes that change are the instruction bytes that `trim`
splices out.

The committed binaries were produced with fontTools 4.62.1. A different version
can compress differently, and the sha256 pins in `scripts/artifacts.test.mjs`
are what will report that.

Usage:
    python3 scripts/build-fonts.py FiraCode-Regular.woff2 FiraCode-Bold.woff2 ...
"""

import io
import sys
from pathlib import Path

from fontTools.pens.recordingPen import RecordingPen
from fontTools.ttLib import TTFont

HINT_TABLES = ("fpgm", "prep", "cvt ", "gasp")
# `head.flags` bit 2: "instructions may depend on pointsize".
POINTSIZE_FLAG = 1 << 2


def glyphs_with_bytecode(font):
    """Glyphs carrying instructions. Expands `font`, so never call this on a face being edited."""
    return [
        name
        for name, glyph in font["glyf"].glyphs.items()
        if (program := getattr(glyph, "program", None)) is not None and program.bytecode
    ]


def outlines(font):
    """Every glyph's advance plus its full drawing, composites included."""
    glyph_set = font.getGlyphSet()
    drawn = {}
    for name in font.getGlyphOrder():
        pen = RecordingPen()
        glyph_set[name].draw(pen)
        drawn[name] = (glyph_set[name].width, pen.value)
    return drawn


def layout_features(font, tag):
    if tag not in font:
        return None
    table = font[tag]
    table.ensureDecompiled()
    return sorted(record.FeatureTag for record in table.table.FeatureList.FeatureRecord)


def fingerprint(font):
    """Everything this transform must leave alone."""
    return {
        "glyphOrder": font.getGlyphOrder(),
        "numGlyphs": font["maxp"].numGlyphs,
        "unitsPerEm": font["head"].unitsPerEm,
        "hmtx": [font["hmtx"][name] for name in font.getGlyphOrder()],
        "outlines": outlines(font),
        "cmap": sorted(font.getBestCmap().items()),
        "names": [font["name"].getDebugName(name_id) for name_id in range(18)],
        "italicAngle": font["post"].italicAngle,
        "widthClass": font["OS/2"].usWidthClass,
        "GSUB": layout_features(font, "GSUB"),
        "GPOS": layout_features(font, "GPOS"),
    }


def assert_unhinted(font):
    leftover = glyphs_with_bytecode(font)
    assert not leftover, f"glyphs still carry hinting bytecode: {leftover[:5]}"
    present = [tag for tag in HINT_TABLES if tag in font]
    assert not present, f"hinting tables survived: {present}"
    assert not font["head"].flags & POINTSIZE_FLAG, (
        "head.flags still advertises pointsize-dependent instructions"
    )


def strip(path):
    raw = Path(path).read_bytes()
    source = TTFont(io.BytesIO(raw))
    expected = fingerprint(source)
    cleared = len(glyphs_with_bytecode(source))

    target = TTFont(io.BytesIO(raw), recalcTimestamp=False, recalcBBoxes=False)
    for glyph in target["glyf"].glyphs.values():
        glyph.removeHinting()
    for tag in HINT_TABLES:
        if tag in target:
            del target[tag]
    target["head"].flags &= ~POINTSIZE_FLAG

    output = io.BytesIO()
    target.flavor = "woff2"
    target.save(output)
    written = output.getvalue()

    # Verify the bytes that are about to be committed, not the in-memory face.
    result = TTFont(io.BytesIO(written))
    assert result.flavor == "woff2", f"{path}: lost its woff2 flavor"
    assert_unhinted(result)
    assert fingerprint(result) == expected, (
        f"{path}: transform touched something it must not"
    )

    Path(path).write_bytes(written)
    return cleared, len(written)


def main(argv):
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    for path in argv:
        cleared, size = strip(path)
        print(f"{path}: stripped hinting from {cleared} glyphs -> {size} bytes")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
