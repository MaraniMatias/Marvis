# Bundled fonts

Three files, under two family names, shipping the same typography as the two
patched TTF this replaced for 4.6MB of them.

`FiraCode-Regular.woff2` and `FiraCode-Bold.woff2` are
[Fira Code](https://github.com/tonsky/FiraCode) 6.2, taken from the `woff2/`
folder of the release zip unmodified. The license is `LICENSE-FiraCode.txt`
(SIL Open Font License 1.1) and it travels with the files, which is what that
license asks for.

`NerdSymbols.woff2` is the icon half of
[FiraCode Nerd Font Mono](https://github.com/ryanoasis/nerd-fonts) 3.2.1, made
from the same base Fira Code with the Nerd Fonts glyphs patched in. It is cut
down to the 10,093 glyphs used for icons: the private-use range, plus the ten
symbols patched outside it (`U+23FB-23FE`, `U+26A1`, `U+276C-276F`,
`U+2B58`). The outlines and advance widths are those of the patched font, so
the icons keep their terminal-cell width. The source glyph-license inventory
has not been verified; the Fira Code OFL notice does not establish the license
for separate patched glyphs.

They are bundled so terminal and editor typography is consistent regardless of
which fonts happen to be installed on the machine. The terminal waits for these
faces before opening xterm, so its initial grid and glyph atlas use the bundled
font rather than a fallback.

## Why the icons are a separate file

They used to be inside both text faces, which meant a face nobody reads was
downloaded and parsed twice. More than the weight, the icon half cannot be
replaced by an upstream Nerd Fonts symbols file: those draw on a 1em grid while
this cell is 0.6154em, so xterm measures every icon as nearly two cells wide.
The icons have to come from a font patched _to_ this cell, which is why this one
is a subset of the file it replaced rather than a download.

To reproduce the subset (the patched TTF is a build input and is not bundled):

```sh
python3 -m fontTools.subset FiraCodeNerdFontMono-Regular.ttf \
  --unicodes='U+23FB-23FE,U+26A1,U+276C-276F,U+2B58,U+E000-F8FF,U+F0000-FFFFD' \
  --name-IDs='*' --notdef-outline --layout-features='*' --drop-tables=FFTM,DSIG \
  --flavor=woff2 --output-file=NerdSymbols.woff2
```

To change the face, replace the files, keep the names, and update the
`@font-face` blocks in `src/marvis.css` and `TERMINAL_FONT_FAMILY` in
`src/lib/marvis-terminal.ts` together — the app has one face on purpose. The
icon family has to be named in every place the text one is, or a glyph in the
private use range falls back to a face of its own and draws at the wrong width.
