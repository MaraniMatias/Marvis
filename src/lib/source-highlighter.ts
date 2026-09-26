import DOMPurify from "dompurify";
import { createHighlighterCore } from "shiki/core";
import { createJavaScriptRegexEngine } from "shiki/engine/javascript";
import type typescript from "shiki/langs/typescript.mjs";
import githubDarkDefault from "shiki/themes/github-dark-default.mjs";

type SourceLanguage = (typeof typescript)[number];
type LanguageLoader = () => Promise<SourceLanguage[]>;
type LanguageDefinition = { name: string; load: LanguageLoader };

const languageDefinitions = {
  c: { name: "c", load: () => import("shiki/langs/c.mjs").then(({ default: language }) => language) },
  csharp: { name: "csharp", load: () => import("shiki/langs/csharp.mjs").then(({ default: language }) => language) },
  css: { name: "css", load: () => import("shiki/langs/css.mjs").then(({ default: language }) => language) },
  go: { name: "go", load: () => import("shiki/langs/go.mjs").then(({ default: language }) => language) },
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
  ["less", languageDefinitions.less],
  ["md", languageDefinitions.markdown],
  ["mdown", languageDefinitions.markdown],
  ["markdown", languageDefinitions.markdown],
  ["php", languageDefinitions.php],
  ["py", languageDefinitions.python],
  ["pyw", languageDefinitions.python],
  ["rs", languageDefinitions.rust],
  ["scss", languageDefinitions.scss],
  ["sh", languageDefinitions.shellscript],
  ["sql", languageDefinitions.sql],
  ["swift", languageDefinitions.swift],
  ["toml", languageDefinitions.toml],
  ["ts", languageDefinitions.typescript],
  ["tsx", languageDefinitions.tsx],
  ["vue", languageDefinitions.vue],
  ["xml", languageDefinitions.xml],
  ["yaml", languageDefinitions.yaml],
  ["yml", languageDefinitions.yaml],
  ["zsh", languageDefinitions.shellscript],
]);

const highlighters = new Map<string, ReturnType<typeof createHighlighterCore>>();
const highlightedSourceCache = new Map<string, Promise<readonly string[]>>();

const sanitizerOptions = {
  ALLOWED_TAGS: ["span"],
  ALLOWED_ATTR: ["class", "style"],
  ALLOW_DATA_ATTR: false,
};

function cacheKey(path: string, source: string): string {
  return JSON.stringify([path, source]);
}

function languageForPath(path: string) {
  const extension = path.split(".").pop()?.toLowerCase();
  return extension ? languageByExtension.get(extension) : undefined;
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

export function sanitizeHighlightedHtml(html: string): string[] {
  const sanitized = DOMPurify.sanitize(html, sanitizerOptions);
  const document = new DOMParser().parseFromString(`<div>${sanitized}</div>`, "text/html");
  const root = document.body.firstElementChild;
  if (!root) return [];
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
  return Array.from(root.querySelectorAll("span.line"), (line) => line.outerHTML);
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
