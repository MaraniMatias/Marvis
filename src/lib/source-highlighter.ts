import DOMPurify from "dompurify";
import { createHighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import type typescript from "shiki/langs/typescript.mjs";
import githubDarkDefault from "shiki/themes/github-dark-default.mjs";

type SourceLanguage = (typeof typescript)[number];
type LanguageLoader = () => Promise<SourceLanguage[]>;
type LanguageDefinition = { name: string; load: LanguageLoader };

const languageDefinitions = {
  ada: { name: "ada", load: () => import("shiki/langs/ada.mjs").then(({ default: language }) => language) },
  apache: { name: "apache", load: () => import("shiki/langs/apache.mjs").then(({ default: language }) => language) },
  asciidoc: {
    name: "asciidoc",
    load: () => import("shiki/langs/asciidoc.mjs").then(({ default: language }) => language),
  },
  astro: { name: "astro", load: () => import("shiki/langs/astro.mjs").then(({ default: language }) => language) },
  bibtex: {
    name: "bibtex",
    load: () => import("shiki/langs/bibtex.mjs").then(({ default: language }) => language),
  },
  blade: { name: "blade", load: () => import("shiki/langs/blade.mjs").then(({ default: language }) => language) },
  c: { name: "c", load: () => import("shiki/langs/c.mjs").then(({ default: language }) => language) },
  csharp: { name: "csharp", load: () => import("shiki/langs/csharp.mjs").then(({ default: language }) => language) },
  clojure: {
    name: "clojure",
    load: () => import("shiki/langs/clojure.mjs").then(({ default: language }) => language),
  },
  cmake: { name: "cmake", load: () => import("shiki/langs/cmake.mjs").then(({ default: language }) => language) },
  commonLisp: {
    name: "common-lisp",
    load: () => import("shiki/langs/lisp.mjs").then(({ default: language }) => language),
  },
  cpp: { name: "cpp", load: () => import("shiki/langs/cpp.mjs").then(({ default: language }) => language) },
  css: { name: "css", load: () => import("shiki/langs/css.mjs").then(({ default: language }) => language) },
  csv: { name: "csv", load: () => import("shiki/langs/csv.mjs").then(({ default: language }) => language) },
  dart: { name: "dart", load: () => import("shiki/langs/dart.mjs").then(({ default: language }) => language) },
  diff: { name: "diff", load: () => import("shiki/langs/diff.mjs").then(({ default: language }) => language) },
  docker: { name: "docker", load: () => import("shiki/langs/docker.mjs").then(({ default: language }) => language) },
  elixir: { name: "elixir", load: () => import("shiki/langs/elixir.mjs").then(({ default: language }) => language) },
  erlang: { name: "erlang", load: () => import("shiki/langs/erlang.mjs").then(({ default: language }) => language) },
  fish: { name: "fish", load: () => import("shiki/langs/fish.mjs").then(({ default: language }) => language) },
  fsharp: { name: "fsharp", load: () => import("shiki/langs/fsharp.mjs").then(({ default: language }) => language) },
  gdscript: {
    name: "gdscript",
    load: () => import("shiki/langs/gdscript.mjs").then(({ default: language }) => language),
  },
  glsl: { name: "glsl", load: () => import("shiki/langs/glsl.mjs").then(({ default: language }) => language) },
  go: { name: "go", load: () => import("shiki/langs/go.mjs").then(({ default: language }) => language) },
  groovy: { name: "groovy", load: () => import("shiki/langs/groovy.mjs").then(({ default: language }) => language) },
  haml: { name: "haml", load: () => import("shiki/langs/haml.mjs").then(({ default: language }) => language) },
  handlebars: {
    name: "handlebars",
    load: () => import("shiki/langs/handlebars.mjs").then(({ default: language }) => language),
  },
  haskell: { name: "haskell", load: () => import("shiki/langs/haskell.mjs").then(({ default: language }) => language) },
  hcl: { name: "hcl", load: () => import("shiki/langs/hcl.mjs").then(({ default: language }) => language) },
  hlsl: { name: "hlsl", load: () => import("shiki/langs/hlsl.mjs").then(({ default: language }) => language) },
  http: { name: "http", load: () => import("shiki/langs/http.mjs").then(({ default: language }) => language) },
  ini: { name: "ini", load: () => import("shiki/langs/ini.mjs").then(({ default: language }) => language) },
  jinja: { name: "jinja", load: () => import("shiki/langs/jinja.mjs").then(({ default: language }) => language) },
  json5: { name: "json5", load: () => import("shiki/langs/json5.mjs").then(({ default: language }) => language) },
  jsonl: { name: "jsonl", load: () => import("shiki/langs/jsonl.mjs").then(({ default: language }) => language) },
  julia: { name: "julia", load: () => import("shiki/langs/julia.mjs").then(({ default: language }) => language) },
  just: { name: "just", load: () => import("shiki/langs/justfile.mjs").then(({ default: language }) => language) },
  latex: { name: "latex", load: () => import("shiki/langs/latex.mjs").then(({ default: language }) => language) },
  liquid: { name: "liquid", load: () => import("shiki/langs/liquid.mjs").then(({ default: language }) => language) },
  log: { name: "log", load: () => import("shiki/langs/log.mjs").then(({ default: language }) => language) },
  lua: { name: "lua", load: () => import("shiki/langs/lua.mjs").then(({ default: language }) => language) },
  make: { name: "make", load: () => import("shiki/langs/make.mjs").then(({ default: language }) => language) },
  matlab: { name: "matlab", load: () => import("shiki/langs/matlab.mjs").then(({ default: language }) => language) },
  mermaid: { name: "mermaid", load: () => import("shiki/langs/mermaid.mjs").then(({ default: language }) => language) },
  nginx: { name: "nginx", load: () => import("shiki/langs/nginx.mjs").then(({ default: language }) => language) },
  nix: { name: "nix", load: () => import("shiki/langs/nix.mjs").then(({ default: language }) => language) },
  nushell: { name: "nushell", load: () => import("shiki/langs/nushell.mjs").then(({ default: language }) => language) },
  objectiveC: {
    name: "objective-c",
    load: () => import("shiki/langs/objective-c.mjs").then(({ default: language }) => language),
  },
  ocaml: { name: "ocaml", load: () => import("shiki/langs/ocaml.mjs").then(({ default: language }) => language) },
  org: { name: "org", load: () => import("shiki/langs/org.mjs").then(({ default: language }) => language) },
  pascal: { name: "pascal", load: () => import("shiki/langs/pascal.mjs").then(({ default: language }) => language) },
  perl: { name: "perl", load: () => import("shiki/langs/perl.mjs").then(({ default: language }) => language) },
  powershell: {
    name: "powershell",
    load: () => import("shiki/langs/powershell.mjs").then(({ default: language }) => language),
  },
  prisma: { name: "prisma", load: () => import("shiki/langs/prisma.mjs").then(({ default: language }) => language) },
  prolog: { name: "prolog", load: () => import("shiki/langs/prolog.mjs").then(({ default: language }) => language) },
  proto: { name: "proto", load: () => import("shiki/langs/proto.mjs").then(({ default: language }) => language) },
  pug: { name: "pug", load: () => import("shiki/langs/pug.mjs").then(({ default: language }) => language) },
  qml: { name: "qml", load: () => import("shiki/langs/qml.mjs").then(({ default: language }) => language) },
  r: { name: "r", load: () => import("shiki/langs/r.mjs").then(({ default: language }) => language) },
  racket: { name: "racket", load: () => import("shiki/langs/racket.mjs").then(({ default: language }) => language) },
  regexp: { name: "regexp", load: () => import("shiki/langs/regex.mjs").then(({ default: language }) => language) },
  ruby: { name: "ruby", load: () => import("shiki/langs/ruby.mjs").then(({ default: language }) => language) },
  sass: { name: "sass", load: () => import("shiki/langs/sass.mjs").then(({ default: language }) => language) },
  scala: { name: "scala", load: () => import("shiki/langs/scala.mjs").then(({ default: language }) => language) },
  scheme: { name: "scheme", load: () => import("shiki/langs/scheme.mjs").then(({ default: language }) => language) },
  shellsession: {
    name: "shellsession",
    load: () => import("shiki/langs/shellsession.mjs").then(({ default: language }) => language),
  },
  smalltalk: {
    name: "smalltalk",
    load: () => import("shiki/langs/smalltalk.mjs").then(({ default: language }) => language),
  },
  solidity: {
    name: "solidity",
    load: () => import("shiki/langs/solidity.mjs").then(({ default: language }) => language),
  },
  sshConfig: {
    name: "ssh-config",
    load: () => import("shiki/langs/ssh-config.mjs").then(({ default: language }) => language),
  },
  stylus: { name: "stylus", load: () => import("shiki/langs/stylus.mjs").then(({ default: language }) => language) },
  svelte: { name: "svelte", load: () => import("shiki/langs/svelte.mjs").then(({ default: language }) => language) },
  systemVerilog: {
    name: "system-verilog",
    load: () => import("shiki/langs/system-verilog.mjs").then(({ default: language }) => language),
  },
  tcl: { name: "tcl", load: () => import("shiki/langs/tcl.mjs").then(({ default: language }) => language) },
  terraform: {
    name: "terraform",
    load: () => import("shiki/langs/terraform.mjs").then(({ default: language }) => language),
  },
  twig: { name: "twig", load: () => import("shiki/langs/twig.mjs").then(({ default: language }) => language) },
  typst: { name: "typst", load: () => import("shiki/langs/typst.mjs").then(({ default: language }) => language) },
  vala: { name: "vala", load: () => import("shiki/langs/vala.mjs").then(({ default: language }) => language) },
  vb: { name: "vb", load: () => import("shiki/langs/vb.mjs").then(({ default: language }) => language) },
  verilog: { name: "verilog", load: () => import("shiki/langs/verilog.mjs").then(({ default: language }) => language) },
  vhdl: { name: "vhdl", load: () => import("shiki/langs/vhdl.mjs").then(({ default: language }) => language) },
  viml: { name: "viml", load: () => import("shiki/langs/vimscript.mjs").then(({ default: language }) => language) },
  wasm: { name: "wasm", load: () => import("shiki/langs/wasm.mjs").then(({ default: language }) => language) },
  wgsl: { name: "wgsl", load: () => import("shiki/langs/wgsl.mjs").then(({ default: language }) => language) },
  zig: { name: "zig", load: () => import("shiki/langs/zig.mjs").then(({ default: language }) => language) },
  graphql: {
    name: "graphql",
    load: () => import("shiki/langs/graphql.mjs").then(({ default: language }) => language),
  },
  html: { name: "html", load: () => import("shiki/langs/html.mjs").then(({ default: language }) => language) },
  java: { name: "java", load: () => import("shiki/langs/java.mjs").then(({ default: language }) => language) },
  javascript: {
    name: "javascript",
    load: () => import("shiki/langs/javascript.mjs").then(({ default: language }) => language),
  },
  json: { name: "json", load: () => import("shiki/langs/json.mjs").then(({ default: language }) => language) },
  jsonc: { name: "jsonc", load: () => import("shiki/langs/jsonc.mjs").then(({ default: language }) => language) },
  kotlin: { name: "kotlin", load: () => import("shiki/langs/kotlin.mjs").then(({ default: language }) => language) },
  less: { name: "less", load: () => import("shiki/langs/less.mjs").then(({ default: language }) => language) },
  markdown: {
    name: "markdown",
    load: () => import("shiki/langs/markdown.mjs").then(({ default: language }) => language),
  },
  php: { name: "php", load: () => import("shiki/langs/php.mjs").then(({ default: language }) => language) },
  python: { name: "python", load: () => import("shiki/langs/python.mjs").then(({ default: language }) => language) },
  rust: { name: "rust", load: () => import("shiki/langs/rust.mjs").then(({ default: language }) => language) },
  scss: { name: "scss", load: () => import("shiki/langs/scss.mjs").then(({ default: language }) => language) },
  shellscript: {
    name: "shellscript",
    load: () => import("shiki/langs/shellscript.mjs").then(({ default: language }) => language),
  },
  sql: { name: "sql", load: () => import("shiki/langs/sql.mjs").then(({ default: language }) => language) },
  swift: { name: "swift", load: () => import("shiki/langs/swift.mjs").then(({ default: language }) => language) },
  toml: { name: "toml", load: () => import("shiki/langs/toml.mjs").then(({ default: language }) => language) },
  tsx: { name: "tsx", load: () => import("shiki/langs/tsx.mjs").then(({ default: language }) => language) },
  typescript: {
    name: "typescript",
    load: () => import("shiki/langs/typescript.mjs").then(({ default: language }) => language),
  },
  vue: { name: "vue", load: () => import("shiki/langs/vue.mjs").then(({ default: language }) => language) },
  xml: { name: "xml", load: () => import("shiki/langs/xml.mjs").then(({ default: language }) => language) },
  yaml: { name: "yaml", load: () => import("shiki/langs/yaml.mjs").then(({ default: language }) => language) },
} satisfies Record<string, LanguageDefinition>;

const languageByExtension = new Map<string, LanguageDefinition>([
  ["c", languageDefinitions.c],
  ["cs", languageDefinitions.csharp],
  ["css", languageDefinitions.css],
  ["go", languageDefinitions.go],
  ["gql", languageDefinitions.graphql],
  ["graphql", languageDefinitions.graphql],
  ["h", languageDefinitions.c],
  ["htm", languageDefinitions.html],
  ["html", languageDefinitions.html],
  ["java", languageDefinitions.java],
  ["js", languageDefinitions.javascript],
  ["jsx", languageDefinitions.javascript],
  ["json", languageDefinitions.json],
  ["jsonc", languageDefinitions.jsonc],
  ["kt", languageDefinitions.kotlin],
  ["kts", languageDefinitions.kotlin],
  ["adb", languageDefinitions.ada],
  ["adoc", languageDefinitions.asciidoc],
  ["asciidoc", languageDefinitions.asciidoc],
  ["bash", languageDefinitions.shellscript],
  ["bib", languageDefinitions.bibtex],
  ["c", languageDefinitions.c],
  ["cc", languageDefinitions.cpp],
  ["cjs", languageDefinitions.javascript],
  ["clj", languageDefinitions.clojure],
  ["cljs", languageDefinitions.clojure],
  ["cmake", languageDefinitions.cmake],
  ["cpp", languageDefinitions.cpp],
  ["cs", languageDefinitions.csharp],
  ["css", languageDefinitions.css],
  ["csv", languageDefinitions.csv],
  ["cxx", languageDefinitions.cpp],
  ["dart", languageDefinitions.dart],
  ["diff", languageDefinitions.diff],
  ["dockerfile", languageDefinitions.docker],
  ["edn", languageDefinitions.clojure],
  ["erl", languageDefinitions.erlang],
  ["ex", languageDefinitions.elixir],
  ["exs", languageDefinitions.elixir],
  ["fish", languageDefinitions.fish],
  ["fs", languageDefinitions.fsharp],
  ["fsx", languageDefinitions.fsharp],
  ["gd", languageDefinitions.gdscript],
  ["glsl", languageDefinitions.glsl],
  ["gql", languageDefinitions.graphql],
  ["graphql", languageDefinitions.graphql],
  ["h", languageDefinitions.c],
  ["hh", languageDefinitions.cpp],
  ["hpp", languageDefinitions.cpp],
  ["htm", languageDefinitions.html],
  ["html", languageDefinitions.html],
  ["hxx", languageDefinitions.cpp],
  ["ini", languageDefinitions.ini],
  ["java", languageDefinitions.java],
  ["jl", languageDefinitions.julia],
  ["js", languageDefinitions.javascript],
  ["json", languageDefinitions.json],
  ["json5", languageDefinitions.json5],
  ["jsonc", languageDefinitions.jsonc],
  ["jsonl", languageDefinitions.jsonl],
  ["jsx", languageDefinitions.javascript],
  ["kt", languageDefinitions.kotlin],
  ["kts", languageDefinitions.kotlin],
  ["less", languageDefinitions.less],
  ["lisp", languageDefinitions.commonLisp],
  ["log", languageDefinitions.log],
  ["lua", languageDefinitions.lua],
  ["m", languageDefinitions.objectiveC],
  ["make", languageDefinitions.make],
  ["makefile", languageDefinitions.make],
  ["md", languageDefinitions.markdown],
  ["mdown", languageDefinitions.markdown],
  ["markdown", languageDefinitions.markdown],
  ["mjs", languageDefinitions.javascript],
  ["mk", languageDefinitions.make],
  ["ml", languageDefinitions.ocaml],
  ["mmd", languageDefinitions.mermaid],
  ["mts", languageDefinitions.typescript],
  ["nu", languageDefinitions.nushell],
  ["objc", languageDefinitions.objectiveC],
  ["patch", languageDefinitions.diff],
  ["php", languageDefinitions.php],
  ["pl", languageDefinitions.perl],
  ["prisma", languageDefinitions.prisma],
  ["proto", languageDefinitions.proto],
  ["ps1", languageDefinitions.powershell],
  ["py", languageDefinitions.python],
  ["pyw", languageDefinitions.python],
  ["r", languageDefinitions.r],
  ["rb", languageDefinitions.ruby],
  ["rs", languageDefinitions.rust],
  ["sass", languageDefinitions.sass],
  ["scala", languageDefinitions.scala],
  ["scss", languageDefinitions.scss],
  ["sh", languageDefinitions.shellscript],
  ["sol", languageDefinitions.solidity],
  ["sql", languageDefinitions.sql],
  ["styl", languageDefinitions.stylus],
  ["sv", languageDefinitions.systemVerilog],
  ["svelte", languageDefinitions.svelte],
  ["swift", languageDefinitions.swift],
  ["tex", languageDefinitions.latex],
  ["tf", languageDefinitions.terraform],
  ["toml", languageDefinitions.toml],
  ["ts", languageDefinitions.typescript],
  ["tsx", languageDefinitions.tsx],
  ["v", languageDefinitions.verilog],
  ["vb", languageDefinitions.vb],
  ["vhd", languageDefinitions.vhdl],
  ["vhdl", languageDefinitions.vhdl],
  ["vim", languageDefinitions.viml],
  ["vue", languageDefinitions.vue],
  ["wat", languageDefinitions.wasm],
  ["wgsl", languageDefinitions.wgsl],
  ["xml", languageDefinitions.xml],
  ["yaml", languageDefinitions.yaml],
  ["yml", languageDefinitions.yaml],
  ["zig", languageDefinitions.zig],
  ["zsh", languageDefinitions.shellscript],
]);

const highlighters = new Map<string, ReturnType<typeof createHighlighterCore>>();
const highlightedSourceCache = new Map<string, Promise<readonly string[]>>();
const highlightedBlockCache = new Map<string, Promise<string | null>>();
const MAX_CACHED_BLOCKS = 256;

const sanitizerOptions = {
  ALLOWED_TAGS: ["span"],
  ALLOWED_ATTR: ["class", "style"],
  ALLOW_DATA_ATTR: false,
};

/**
 * The only way a Markdown fence reaches a grammar. The info string is attacker-controlled — it
 * comes from the file being previewed — so it is looked up in this allowlist and never turned
 * into a module path. An unknown alias means no highlighting, not a fallback guess.
 */
const fenceLanguageAliases = {
  ada: "ada",
  adb: "ada",
  apache: "apache",
  asciidoc: "asciidoc",
  adoc: "asciidoc",
  astro: "astro",
  httpd: "apache",
  matlab: "matlab",
  org: "org",
  pascal: "pascal",
  prolog: "prolog",
  bash: "shellscript",
  bat: "shellscript",
  bibtex: "bibtex",
  blade: "blade",
  c: "c",
  "c#": "csharp",
  "c++": "cpp",
  cc: "cpp",
  cjs: "javascript",
  clj: "clojure",
  cljs: "clojure",
  clojure: "clojure",
  cmake: "cmake",
  cmd: "shellscript",
  codeowners: "ini",
  console: "shellsession",
  cpp: "cpp",
  cs: "csharp",
  csharp: "csharp",
  css: "css",
  csv: "csv",
  cxx: "cpp",
  dart: "dart",
  diff: "diff",
  docker: "docker",
  dockerfile: "docker",
  editorconfig: "ini",
  edn: "clojure",
  elisp: "commonLisp",
  elixir: "elixir",
  emacs: "commonLisp",
  erl: "erlang",
  erlang: "erlang",
  ex: "elixir",
  exs: "elixir",
  fish: "fish",
  fs: "fsharp",
  fsharp: "fsharp",
  gd: "gdscript",
  gdscript: "gdscript",
  glsl: "glsl",
  go: "go",
  golang: "go",
  graphql: "graphql",
  groovy: "groovy",
  h: "c",
  haml: "haml",
  handlebars: "handlebars",
  haskell: "haskell",
  hbs: "handlebars",
  hcl: "hcl",
  hh: "cpp",
  hlsl: "hlsl",
  hpp: "cpp",
  hs: "haskell",
  htm: "html",
  html: "html",
  http: "http",
  hxx: "cpp",
  ini: "ini",
  java: "java",
  jinja: "jinja",
  jinja2: "jinja",
  jl: "julia",
  js: "javascript",
  json: "json",
  json5: "json5",
  jsonc: "jsonc",
  jsonl: "jsonl",
  jsx: "javascript",
  julia: "julia",
  just: "just",
  justfile: "just",
  kt: "kotlin",
  kotlin: "kotlin",
  kts: "kotlin",
  latex: "latex",
  less: "less",
  liquid: "liquid",
  lisp: "commonLisp",
  log: "log",
  lua: "lua",
  m: "objectiveC",
  make: "make",
  makefile: "make",
  markdown: "markdown",
  md: "markdown",
  mdx: "markdown",
  mermaid: "mermaid",
  mmd: "mermaid",
  mjs: "javascript",
  ml: "ocaml",
  nginx: "nginx",
  nim: "javascript",
  nix: "nix",
  nushell: "nushell",
  nu: "nushell",
  objc: "objectiveC",
  "objective-c": "objectiveC",
  ocaml: "ocaml",
  patch: "diff",
  perl: "perl",
  php: "php",
  powershell: "powershell",
  prisma: "prisma",
  properties: "ini",
  proto: "proto",
  protobuf: "proto",
  ps: "powershell",
  ps1: "powershell",
  pug: "pug",
  pwsh: "powershell",
  py: "python",
  python: "python",
  qml: "qml",
  r: "r",
  racket: "racket",
  rb: "ruby",
  regex: "regexp",
  regexp: "regexp",
  ruby: "ruby",
  rust: "rust",
  sass: "sass",
  scala: "scala",
  scheme: "scheme",
  scss: "scss",
  sh: "shellscript",
  shell: "shellscript",
  "shell-session": "shellsession",
  smalltalk: "smalltalk",
  sol: "solidity",
  solidity: "solidity",
  sql: "sql",
  sshconfig: "sshConfig",
  styl: "stylus",
  stylus: "stylus",
  svelte: "svelte",
  sv: "systemVerilog",
  swift: "swift",
  systemd: "ini",
  systemverilog: "systemVerilog",
  tcl: "tcl",
  terraform: "terraform",
  tex: "latex",
  tf: "terraform",
  tfvars: "terraform",
  toml: "toml",
  ts: "typescript",
  tsx: "tsx",
  twig: "twig",
  typ: "typst",
  typescript: "typescript",
  typst: "typst",
  v: "verilog",
  vala: "vala",
  vb: "vb",
  verilog: "verilog",
  vhdl: "vhdl",
  vim: "viml",
  vimscript: "viml",
  vue: "vue",
  wat: "wasm",
  wasm: "wasm",
  wgsl: "wgsl",
  xml: "xml",
  yaml: "yaml",
  yml: "yaml",
  zig: "zig",
  zsh: "shellscript",
} satisfies Record<string, keyof typeof languageDefinitions>;

const fenceLanguageByAlias = new Map<string, LanguageDefinition>(
  Object.entries(fenceLanguageAliases).map(([alias, key]) => [alias, languageDefinitions[key]]),
);

function cacheKey(path: string, source: string): string {
  return JSON.stringify([path, source]);
}

function languageForPath(path: string) {
  const extension = path.split(".").pop()?.toLowerCase();
  return extension ? languageByExtension.get(extension) : undefined;
}

/** The grammar a Markdown fence asks for, or undefined when its info string names no known one. */
export function languageForFenceInfo(info: string): LanguageDefinition | undefined {
  const name = info.trim().split(/\s+/, 1)[0]?.toLowerCase();
  return name ? fenceLanguageByAlias.get(name) : undefined;
}

function highlighterFor(language: LanguageDefinition) {
  const cached = highlighters.get(language.name);
  if (cached) return cached;
  const request = language.load().then((langs) =>
    createHighlighterCore({
      langs,
      themes: [githubDarkDefault],
      engine: createJavaScriptRegexEngine(),
    }),
  );
  highlighters.set(language.name, request);
  return request;
}

export function canHighlightSourcePath(path: string): boolean {
  return languageForPath(path) !== undefined;
}

/** Sanitizes one Shiki render down to nested spans carrying nothing but a class and a color. */
function sanitizeShikiFragment(html: string): Element | null {
  const sanitized = DOMPurify.sanitize(html, sanitizerOptions);
  const document = new DOMParser().parseFromString(`<div>${sanitized}</div>`, "text/html");
  const root = document.body.firstElementChild;
  if (!root) return null;
  for (const element of Array.from(root.querySelectorAll("script, style, iframe, object, embed, img, svg, math"))) {
    element.remove();
  }
  for (const element of Array.from(root.querySelectorAll("*"))) {
    if (element.tagName === "SPAN") continue;
    const parent = element.parentNode;
    if (!parent) continue;
    while (element.firstChild) parent.insertBefore(element.firstChild, element);
    element.remove();
  }
  for (const span of Array.from(root.querySelectorAll("span"))) {
    for (const attribute of Array.from(span.attributes)) {
      if (attribute.name !== "class" && attribute.name !== "style") span.removeAttribute(attribute.name);
    }
    const style = span.getAttribute("style");
    if (style && /(?:url|expression|javascript)\s*\(|[<>]/i.test(style)) span.removeAttribute("style");
  }
  return root;
}

export function sanitizeHighlightedHtml(html: string): string[] {
  const root = sanitizeShikiFragment(html);
  if (!root) return [];
  return Array.from(root.querySelectorAll("span.line"), (line) => line.outerHTML);
}

/**
 * The colored body of one code block, without the `<pre>` Shiki wraps it in: the Markdown
 * renderer owns that element, so only the spans are returned. `null` leaves the caller's
 * plain text in place, which is also what a grammar that fails to load produces.
 */
export function highlightCodeBlock(language: LanguageDefinition, code: string): Promise<string | null> {
  const key = JSON.stringify([language.name, code]);
  const cached = highlightedBlockCache.get(key);
  if (cached) return cached;
  if (highlightedBlockCache.size >= MAX_CACHED_BLOCKS) {
    const oldest = highlightedBlockCache.keys().next().value;
    if (oldest !== undefined) highlightedBlockCache.delete(oldest);
  }
  const request = highlighterFor(language)
    .then((instance) => instance.codeToHtml(code, { lang: language.name, theme: "github-dark-default" }))
    .then((html) => sanitizeShikiFragment(html)?.innerHTML || null)
    .catch(() => null);
  highlightedBlockCache.set(key, request);
  return request;
}

export function highlightSource(path: string, source: string): Promise<readonly string[] | null> {
  const language = languageForPath(path);
  if (!language) return Promise.resolve(null);

  const key = cacheKey(path, source);
  const cached = highlightedSourceCache.get(key);
  if (cached) return cached;

  const request = highlighterFor(language)
    .then((instance) => instance.codeToHtml(source, { lang: language.name, theme: "github-dark-default" }))
    .then(sanitizeHighlightedHtml)
    .then((lines) => {
      if (lines.length === 0) throw new Error("Shiki returned no source lines");
      return lines;
    });
  highlightedSourceCache.set(key, request);
  void request.catch(() => highlightedSourceCache.delete(key));
  return request;
}
