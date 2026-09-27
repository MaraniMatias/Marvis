/**
 * Generates the file-type icon table from Catppuccin's icon set, Mocha flavour.
 *
 * The drawings are Catppuccin's. They reach us packaged as a Zed extension, which is why there is
 * nothing to install and no package to depend on: the theme is a JSON file mapping names and
 * extensions to SVG files, and the SVGs are 16x16 single-stroke shapes whose colour is already
 * baked in per type. This reads one flavour and writes a TypeScript module holding just the rules
 * and the inner markup of the icons they reach, so nothing is fetched at runtime and the CSP is
 * untouched: the markup is rendered inline rather than through `img-src`.
 *
 * Usage: node scripts/generate-catppuccin-icons.mjs <path-to-icons-checkout>
 * The checkout is not vendored; see the header of the generated module for the source.
 */
import { readFileSync, writeFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { optimize } from "svgo";

const FLAVOUR = "Catppuccin Mocha";
const THEME = join("icon_themes", "catppuccin-icons.json");
const OUTPUT = join("src", "lib", "catppuccin-icons.ts");

/** The one shape every icon shares, so the wrapper supplies it instead of each icon. */
const VIEWBOX = "0 0 16 16";

const checkout = process.argv[2];
if (!checkout) {
  console.error("usage: node scripts/generate-catppuccin-icons.mjs <path-to-icons-checkout>");
  process.exit(1);
}

const theme = JSON.parse(readFileSync(join(checkout, THEME), "utf8")).themes.find(
  (candidate) => candidate.name === FLAVOUR,
);
if (!theme) {
  console.error(`the theme has no flavour named "${FLAVOUR}"`);
  process.exit(1);
}

/** `./icons/mocha/typescript.svg` -> `typescript`, the key `file_icons` is indexed by. */
function iconId(reference) {
  return reference.replace(/^\.\/icons\/[^/]+\//, "").replace(/\.svg$/, "");
}

const directory = {
  collapsed: iconId(theme.directory_icons.collapsed),
  expanded: iconId(theme.directory_icons.expanded),
};
const namedDirectories = Object.fromEntries(
  Object.entries(theme.named_directory_icons).map(([name, pair]) => [name, iconId(pair.collapsed)]),
);
const fileStems = theme.file_stems;
const fileSuffixes = theme.file_suffixes;
const referenced = new Set([
  directory.collapsed,
  directory.expanded,
  ...Object.values(namedDirectories),
  ...Object.values(fileStems),
  ...Object.values(fileSuffixes),
  "_file",
]);

/**
 * The inside of the `<svg>` element, with the wrapper dropped. Every icon but three is on the
 * same 16x16 box, so the viewBox is the renderer's job and repeating it 500-odd times would be a
 * third of the file; the three that declare their own get it back.
 *
 * `convertPathData` alone takes the markup from 255 kB to 185 kB. The fuller presets buy no
 * further compression but delete zero-length paths, and in these icons such a path is a
 * deliberate dot: a zero-length stroke under `stroke-linecap="round"`. The full preset drops one
 * of those from seven icons, so the list stays one plugin long and `painted` guards the result.
 */
function inner(icon) {
  const source = readFileSync(join(checkout, "icons", FLAVOUR.split(" ").pop().toLowerCase(), `${icon}.svg`), "utf8");
  const viewBox = source.match(/viewBox="([^"]+)"/)?.[1];
  if (!viewBox) throw new Error(`${icon} declares no viewBox`);
  if (viewBox !== VIEWBOX) viewBoxes[icon] = viewBox;
  const markup = strip(source);
  const optimised = strip(
    optimize(source, {
      multipass: true,
      plugins: [{ name: "convertPathData", params: { floatPrecision: 2 } }],
    }).data,
  );
  if (painted(optimised) !== painted(markup)) {
    throw new Error(`optimising ${icon} changed which shapes it paints`);
  }
  return optimised;
}

/** The markup without the box, since the renderer supplies that. */
function strip(svg) {
  return svg
    .replace(/^[\s\S]*?<svg[^>]*>/, "")
    .replace(/<\/svg>\s*$/, "")
    .replace(/\s+/g, " ")
    .trim();
}

/**
 * The paint an icon lays down, as a sorted multiset: which colours are stroked and filled, and how
 * each is filled. An icon that loses or gains one is no longer the shape the theme drew, whatever
 * happened to its path data. `stroke-width` is left out on purpose, since a path carrying a
 * `transform` has its scale folded into the geometry and the line weight written back out, which
 * is the same picture described differently.
 */
function painted(svg) {
  return [...svg.matchAll(/ (stroke|fill|fill-rule)="([^"]+)"/g)]
    .map((match) => `${match[1]}="${match[2]}"`)
    .sort()
    .join(" | ");
}

const viewBoxes = {};

const available = new Set(readdirSync(join(checkout, "icons", "mocha")).map((file) => file.replace(/\.svg$/, "")));
const missing = [...referenced].filter((icon) => !available.has(icon));
if (missing.length) {
  console.error(`the theme reaches ${missing.length} icons that are not in the flavour: ${missing.slice(0, 8)}`);
  process.exit(1);
}

const icons = Object.fromEntries([...referenced].sort().map((icon) => [icon, inner(icon)]));

const module = `// Generated by scripts/generate-catppuccin-icons.mjs. Do not edit.
//
// Catppuccin's drawings, flavour "${FLAVOUR}" (dark, 16x16), from
// https://github.com/catppuccin/zed-icons. The theme's rules plus the markup of the
// ${Object.keys(icons).length} icons they reach.

/** The viewBox every icon in this set shares; the renderer supplies it. */
export const CATPPUCCIN_ICON_VIEWBOX = "${VIEWBOX}";

/** The three icons that declare a box of their own instead of the shared one. */
export const CATPPUCCIN_ICON_VIEWBOX_OVERRIDE: Readonly<Record<string, string>> = ${JSON.stringify(viewBoxes)};

export const CATPPUCCIN_DIRECTORY_ICON = ${JSON.stringify(directory)} as const;

/** A directory with a name of its own, by its own name alone. */
export const CATPPUCCIN_NAMED_DIRECTORY: Readonly<Record<string, string>> = ${JSON.stringify(namedDirectories)};

/** Matched against the name before its first dot, so README.md finds \`readme\`. */
export const CATPPUCCIN_FILE_STEM: Readonly<Record<string, string>> = ${JSON.stringify(fileStems)};

/** Matched against everything after the last dot, by extension. */
export const CATPPUCCIN_FILE_SUFFIX: Readonly<Record<string, string>> = ${JSON.stringify(fileSuffixes)};

/** The markup inside the \`<svg>\` box, keyed by icon id. */
export const CATPPUCCIN_ICON_MARKUP: Readonly<Record<string, string>> = ${JSON.stringify(icons)};

const DEFAULT_FILE_ICON = "_file";

/**
 * The icon a name resolves to, in the order a name is asked: its own name before its dot, then
 * its extension, then nothing. Directories are not resolved here, since a directory answers with
 * either its named icon or the plain folder pair.
 */
export function catppuccinIconFor(name: string): string {
  const stem = name.slice(0, name.indexOf("."));
  if (stem !== name && stem in CATPPUCCIN_FILE_STEM) return CATPPUCCIN_FILE_STEM[stem];
  const dot = name.lastIndexOf(".");
  if (dot > 0) {
    const suffix = name.slice(dot + 1);
    if (suffix in CATPPUCCIN_FILE_SUFFIX) return CATPPUCCIN_FILE_SUFFIX[suffix];
  }
  return DEFAULT_FILE_ICON;
}
`;

writeFileSync(OUTPUT, module);
console.log(
  `wrote ${OUTPUT}: ${Object.keys(icons).length} icons, ` +
    `${Object.keys(namedDirectories).length} named directories, ` +
    `${Object.keys(fileSuffixes).length} suffixes, ` +
    `${(module.length / 1024).toFixed(0)} kB`,
);
