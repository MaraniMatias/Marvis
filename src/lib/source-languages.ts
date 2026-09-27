/**
 * What a document can be highlighted as, with no Shiki in it.
 *
 * `source-highlighter.ts` imports `shiki/core` at module scope, so the document toolbar cannot
 * import anything from it without dragging the highlighter into the first chunk: the toolbar needs
 * the detected language and the list of choices synchronously, long before anyone asks a
 * highlighter for anything. It owns the extension allowlist and the labels, while
 * `source-highlighter.ts` keeps the grammar loaders and resolves every name through that allowlist.
 */

export interface SourceLanguageOption {
  name: string;
  label: string;
}

/**
 * Marvis' own "no grammar". Shiki v4 ships no plaintext language, so this name is in no allowlist
 * and resolves to no highlighting at all, which is how a plain file has always rendered.
 */
export const PLAIN_TEXT = "plaintext";

/** The grammars the toolbar offers, by the name Shiki knows each one by. */
export const SELECTABLE_LANGUAGES: readonly SourceLanguageOption[] = [
  { name: "c", label: "C" },
  { name: "cpp", label: "C++" },
  { name: "csharp", label: "C#" },
  { name: "css", label: "CSS" },
  { name: "diff", label: "Diff" },
  { name: "docker", label: "Dockerfile" },
  { name: "go", label: "Go" },
  { name: "graphql", label: "GraphQL" },
  { name: "html", label: "HTML" },
  { name: "ini", label: "INI" },
  { name: "java", label: "Java" },
  { name: "javascript", label: "JavaScript" },
  { name: "json", label: "JSON" },
  { name: "json5", label: "JSON5" },
  { name: "jsonc", label: "JSON with Comments" },
  { name: "kotlin", label: "Kotlin" },
  { name: "less", label: "Less" },
  { name: "log", label: "Log" },
  { name: "lua", label: "Lua" },
  { name: "make", label: "Makefile" },
  { name: "markdown", label: "Markdown" },
  { name: "nginx", label: "Nginx" },
  { name: "php", label: "PHP" },
  { name: "powershell", label: "PowerShell" },
  { name: "python", label: "Python" },
  { name: "ruby", label: "Ruby" },
  { name: "rust", label: "Rust" },
  { name: "scss", label: "SCSS" },
  { name: "shellscript", label: "Shell" },
  { name: "sql", label: "SQL" },
  { name: "swift", label: "Swift" },
  { name: "toml", label: "TOML" },
  { name: "tsx", label: "TSX" },
  { name: "typescript", label: "TypeScript" },
  { name: "vue", label: "Vue" },
  { name: "xml", label: "XML" },
  { name: "yaml", label: "YAML" },
];

/**
 * The file extensions the highlighter answers for, mapped to a grammar name. It moved here from
 * `source-highlighter.ts` so the same table decides both what the toolbar detects and what the
 * highlighter loads. An extension that names no grammar is not a mistake: it means the file
 * renders plain until the reader forces one from the toolbar.
 */
export const LANGUAGE_BY_EXTENSION: ReadonlyMap<string, string> = new Map<string, string>([
  ["c", "c"],
  ["cs", "csharp"],
  ["css", "css"],
  ["go", "go"],
  ["gql", "graphql"],
  ["graphql", "graphql"],
  ["h", "c"],
  ["htm", "html"],
  ["html", "html"],
  ["java", "java"],
  ["js", "javascript"],
  ["jsx", "javascript"],
  ["json", "json"],
  ["jsonc", "jsonc"],
  ["kt", "kotlin"],
  ["kts", "kotlin"],
  ["adb", "ada"],
  ["adoc", "asciidoc"],
  ["asciidoc", "asciidoc"],
  ["bash", "shellscript"],
  ["bib", "bibtex"],
  ["cc", "cpp"],
  ["cjs", "javascript"],
  ["clj", "clojure"],
  ["cljs", "clojure"],
  ["cmake", "cmake"],
  ["cpp", "cpp"],
  ["csv", "csv"],
  ["cxx", "cpp"],
  ["dart", "dart"],
  ["diff", "diff"],
  ["dockerfile", "docker"],
  ["edn", "clojure"],
  ["erl", "erlang"],
  ["ex", "elixir"],
  ["exs", "elixir"],
  ["fish", "fish"],
  ["fs", "fsharp"],
  ["fsx", "fsharp"],
  ["gd", "gdscript"],
  ["glsl", "glsl"],
  ["hh", "cpp"],
  ["hpp", "cpp"],
  ["hxx", "cpp"],
  ["ini", "ini"],
  ["jl", "julia"],
  ["json5", "json5"],
  ["jsonl", "jsonl"],
  ["less", "less"],
  ["lisp", "common-lisp"],
  ["log", "log"],
  ["lua", "lua"],
  ["m", "objective-c"],
  ["make", "make"],
  ["makefile", "make"],
  ["md", "markdown"],
  ["mdown", "markdown"],
  ["markdown", "markdown"],
  ["mjs", "javascript"],
  ["mk", "make"],
  ["ml", "ocaml"],
  ["mmd", "mermaid"],
  ["mts", "typescript"],
  ["nu", "nushell"],
  ["objc", "objective-c"],
  ["patch", "diff"],
  ["php", "php"],
  ["pl", "perl"],
  ["prisma", "prisma"],
  ["proto", "proto"],
  ["ps1", "powershell"],
  ["py", "python"],
  ["pyw", "python"],
  ["r", "r"],
  ["rb", "ruby"],
  ["rs", "rust"],
  ["sass", "sass"],
  ["scala", "scala"],
  ["scss", "scss"],
  ["sh", "shellscript"],
  ["sol", "solidity"],
  ["sql", "sql"],
  ["styl", "stylus"],
  ["sv", "system-verilog"],
  ["svelte", "svelte"],
  ["swift", "swift"],
  ["tex", "latex"],
  ["tf", "terraform"],
  ["toml", "toml"],
  ["ts", "typescript"],
  ["tsx", "tsx"],
  ["v", "verilog"],
  ["vb", "vb"],
  ["vhd", "vhdl"],
  ["vhdl", "vhdl"],
  ["vim", "viml"],
  ["vue", "vue"],
  ["wat", "wasm"],
  ["wgsl", "wgsl"],
  ["xml", "xml"],
  ["yaml", "yaml"],
  ["yml", "yaml"],
  ["zig", "zig"],
  ["zsh", "shellscript"],
]);

const labelByName = new Map(SELECTABLE_LANGUAGES.map((language) => [language.name, language.label]));
const extensionsByName = new Map<string, string[]>();
for (const [extension, name] of LANGUAGE_BY_EXTENSION) {
  const owned = extensionsByName.get(name);
  if (owned) owned.push(extension);
  else extensionsByName.set(name, [extension]);
}

/** The grammar a path's extension asks for, or undefined when it asks for none. */
export function detectedLanguageName(path: string): string | undefined {
  const extension = path.split(".").pop()?.toLowerCase();
  return extension === undefined ? undefined : LANGUAGE_BY_EXTENSION.get(extension);
}

/** What the toolbar writes down: the written name when it has one, title case when it does not. */
export function languageLabel(name: string): string {
  return labelByName.get(name) ?? titleCase(name);
}

/** Every extension that already asks for `name`, so the search finds it by the suffix it wears. */
export function languageExtensions(name: string): readonly string[] {
  return extensionsByName.get(name) ?? [];
}

function titleCase(name: string): string {
  return name
    .split(/[-_]/)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}
