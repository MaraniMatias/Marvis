# Bundled fonts

`FiraCodeNerdFontMono-Regular.ttf` and `FiraCodeNerdFontMono-Bold.ttf` are
[FiraCode Nerd Font Mono](https://github.com/ryanoasis/nerd-fonts) builds with
Nerd Fonts glyph patches. The source tree includes the Fira Code license,
`LICENSE-FiraCode.txt` (SIL Open Font License 1.1), and the production build
copies it beside the font assets. The upstream glyph-set license inventory is
not present here; the Fira Code notice does not claim to cover separate glyph
licenses.

They are bundled so terminal and editor typography is consistent regardless of
which fonts happen to be installed on the machine. The terminal waits for these
faces before opening xterm, so its initial grid and glyph atlas use the bundled
font rather than a fallback.

Only the two weights the app declares are kept. FiraCode Nerd Font Mono ships no
italic, so there is no italic face to copy, and the other weights are not asked
for: `font-synthesis: none` in `src/style.css` means a weight that has no face
comes back as the one that does, honestly, rather than faked.

To change the face, replace both files, keep the names, and update the
`@font-face` block in `src/marvis.css` and `TERMINAL_FONT_FAMILY` in
`src/lib/marvis-terminal.ts` together — the app has one face on purpose.
