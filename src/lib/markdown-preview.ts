import DOMPurify from "dompurify";
import matter from "gray-matter";
import MarkdownIt from "markdown-it";
import taskLists from "markdown-it-task-lists";
import { highlightCodeBlock, languageForFenceInfo } from "./source-highlighter";

export interface MarkdownImageReference {
  /** The original Markdown URL, passed to the checkout-scoped image command. */
  source: string;
  /** Normalized checkout-relative path used to deduplicate references. */
  path: string;
}

export interface MarkdownPreview {
  html: string;
  images: MarkdownImageReference[];
}

const markdownExtensions = new Set(["md", "markdown", "mdown", "mkd"]);
const SAFE_IMAGE_MIME_TYPES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);

// A document from a checkout is untrusted input, and every grammar costs a lazy chunk to load.
// These bounds keep a README that fences twenty languages from stalling the preview; whatever
// exceeds them stays plain text, which is what a fence with no known language renders as.
const MAX_HIGHLIGHTED_LANGUAGES = 8;
const MAX_HIGHLIGHTED_BLOCKS = 80;
const MAX_HIGHLIGHTED_BLOCK_LINES = 500;
const MAX_HIGHLIGHTED_LINES = 5000;
// Metadata is a header, not the document: past these it stops being a summary of the file and
// starts being the file's second half.
const MAX_FRONTMATTER_KEYS = 32;
const MAX_FRONTMATTER_VALUE_LENGTH = 300;

export function isMarkdownPath(path: string): boolean {
  const extension = path.split(".").pop()?.toLowerCase();
  return extension !== undefined && markdownExtensions.has(extension);
}

/**
 * Whether the document really opens with a `---` fenced block.
 *
 * The shape of the file decides this, before anything is parsed. A document that merely *starts*
 * with a rule is not front matter, and a parser left to guess reads the rest of it as a YAML
 * mapping and hands back an empty page: the heading the file opens with is the mapping's scalar.
 */
function opensFrontMatter(source: string): boolean {
  const lines = source.split(/\r?\n/);
  if (lines[0]?.trim() !== "---") return false;
  return lines.slice(1).some((line) => line.trim() === "---");
}

/**
 * What one front-matter value says, as the page shows it: text, never markup, and a key of
 * `null` says nothing rather than saying the word null. A nested block becomes one line, because
 * this is a header and not a second document.
 */
function frontMatterValue(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (Array.isArray(value)) return value.map(frontMatterValue).filter(Boolean).join(", ");
  if (typeof value === "object") {
    return Object.entries(value as Record<string, unknown>)
      .map(([key, nested]) => `${key}: ${frontMatterValue(nested)}`)
      .join(", ");
  }
  const text = String(value).replace(/\s+/g, " ").trim();
  return text.length > MAX_FRONTMATTER_VALUE_LENGTH ? `${text.slice(0, MAX_FRONTMATTER_VALUE_LENGTH)}…` : text;
}

/**
 * The metadata block GitHub puts above a document that carries some: a table with the keys across
 * the top and their values under them. It is built here rather than parsed out of a rendered
 * string so every value reaches the page escaped, and it goes through the same sanitizer as the
 * document it precedes.
 */
function frontMatterTable(data: Record<string, unknown>, escape: (text: string) => string): string {
  const keys = Object.keys(data).slice(0, MAX_FRONTMATTER_KEYS);
  if (keys.length === 0) return "";
  const header = keys.map((key) => `<th>${escape(key)}</th>`).join("");
  const values = keys.map((key) => `<td>${escape(frontMatterValue(data[key]))}</td>`).join("");
  return `<table class="markdown-frontmatter"><thead><tr>${header}</tr></thead><tbody><tr>${values}</tr></tbody></table>`;
}

function safeRelativeImagePath(markdownPath: string, source: string): string | null {
  if (
    !source ||
    source.startsWith("/") ||
    source.startsWith("\\") ||
    source.includes("\\") ||
    /^[a-z][a-z\d+.-]*:/i.test(source)
  ) {
    return null;
  }
  try {
    const path = decodeURIComponent(source.split(/[?#]/, 1)[0] ?? "");
    if (!path || path.includes("\0")) return null;
    const parts = markdownPath.split("/").slice(0, -1);
    for (const part of path.split("/")) {
      if (!part || part === ".") continue;
      if (part === "..") {
        if (parts.length === 0) return null;
        parts.pop();
      } else {
        if (part === ".git" || (parts.length === 0 && /^[a-z][a-z\d+.-]*:/i.test(part))) return null;
        parts.push(part);
      }
    }
    return parts.length > 0 ? parts.join("/") : null;
  } catch {
    return null;
  }
}

function safeMarkdownLink(url: string): boolean {
  for (let index = 0; index < url.length; index += 1) {
    const character = url.charCodeAt(index);
    if (character <= 0x20 || character === 0x7f) return false;
  }
  if (url.startsWith("#")) return true;
  if (/^[a-z][a-z\d+.-]*:/i.test(url)) {
    return /^(https?|mailto|tel):/i.test(url);
  }
  return !url.startsWith("/") && !url.startsWith("\\") && !url.includes("\\");
}

/**
 * Swaps the plain body of every fenced block for its colored spans.
 *
 * This runs on the already-sanitized HTML, never before it: the Markdown allowlist has no
 * `style` and stays that way, so the only markup that can carry one is a Shiki fragment that
 * `highlightCodeBlock` sanitized on its own. Blocks are found by their `language-*` class in
 * the rendered document, which is what makes fences nested in blockquotes or lists work — those
 * never reach the top level of the token stream.
 */
async function highlightFencedCode(sanitizedHtml: string): Promise<string> {
  const document = new DOMParser().parseFromString(sanitizedHtml, "text/html");
  const blocks = Array.from(document.querySelectorAll("pre > code"));
  if (blocks.length === 0) return sanitizedHtml;

  const languages = new Set<ReturnType<typeof languageForFenceInfo>>();
  const pending: Promise<void>[] = [];
  let blocksLeft = MAX_HIGHLIGHTED_BLOCKS;
  let linesLeft = MAX_HIGHLIGHTED_LINES;

  for (const block of blocks) {
    const alias = /^language-(\S+)/.exec(block.getAttribute("class") ?? "")?.[1];
    const language = alias ? languageForFenceInfo(alias) : undefined;
    if (!language) continue;
    // A grammar already accepted costs no extra chunk, so only the first of each counts.
    if (!languages.has(language) && languages.size >= MAX_HIGHLIGHTED_LANGUAGES) continue;
    const code = block.textContent ?? "";
    const blockLines = code.split("\n").length;
    if (blocksLeft <= 0 || blockLines > MAX_HIGHLIGHTED_BLOCK_LINES || blockLines > linesLeft) continue;
    blocksLeft -= 1;
    linesLeft -= blockLines;
    languages.add(language);
    pending.push(
      highlightCodeBlock(language, code).then((highlighted) => {
        if (highlighted !== null) block.innerHTML = highlighted;
      }),
    );
  }

  await Promise.all(pending);
  return document.body.innerHTML;
}

/** The instance a plugin is handed. The default import is a value, so the type is its instance. */
type MarkdownParser = InstanceType<typeof MarkdownIt>;

/**
 * Drops the HTML comments a preview never shows, the way GitHub and every other renderer does.
 *
 * `html: false` is what keeps markup out of the page, and it is also why a comment is *visible*:
 * markdown-it never recognises the construct, so it escapes it and leaves `<!-- … -->` sitting in
 * the text. A merge request template is mostly comment, so that is a page of noise.
 *
 * Two rules get it back, and neither can reach code. The block rule is registered after `code` and
 * `fence`, so a fence or an indented block is consumed before it is offered the line, and the
 * inline rule takes the comment out of the text run it sits in — a code span is already a token of
 * its own by the time inline rules run, so `` `<!-- kept -->` `` keeps its comment.
 */
function dropHtmlComments(markdown: MarkdownParser): void {
  markdown.block.ruler.before(
    "paragraph",
    "marvis_html_comment",
    (state, startLine, endLine, silent) => {
      const start = state.bMarks[startLine] + state.tShift[startLine];
      // Four spaces of indent is a code block, whatever the line starts with.
      if (state.sCount[startLine] - state.blkIndent >= 4) return false;
      if (!state.src.startsWith("<!--", start)) return false;
      // A comment can run over as many lines as it likes, and the block ends where the document
      // does: a list item hands over an end line past its own content.
      const limit = Math.max(endLine, state.lineMax);
      let line = startLine;
      let closing = -1;
      for (; line < limit; line += 1) {
        const from = line === startLine ? start + 4 : state.bMarks[line] + state.tShift[line];
        const at = state.src.indexOf("-->", from);
        if (at >= 0 && at <= state.eMarks[line]) {
          closing = at;
          break;
        }
      }
      // `<!-- x --> words` is a line of text with a comment in it rather than a comment, so it
      // belongs to the inline rule below, which keeps the words.
      if (closing < 0 || state.src.slice(closing + 3, state.eMarks[line]).trim() !== "") return false;
      if (silent) return true;
      state.line = line + 1;
      return true;
    },
    { alt: ["paragraph", "reference", "blockquote", "list"] },
  );

  // `html_inline` is in the chain whatever `html` says: the rule itself declines, it is not
  // unregistered, so it is the one neighbour that is always there to sit in front of.
  markdown.inline.ruler.before("html_inline", "marvis_html_comment", (state, silent) => {
    if (!state.src.startsWith("<!--", state.pos)) return false;
    const end = state.src.indexOf("-->", state.pos + 4);
    // An `<!--` that is never closed is text, which is what GitHub leaves it as too.
    if (end < 0) return false;
    if (silent) return true;
    state.pos = end + 3;
    return true;
  });
}

export async function renderMarkdownPreview(source: string, markdownPath: string): Promise<MarkdownPreview> {
  const images: MarkdownImageReference[] = [];
  const imageIndexes = new Map<string, number>();
  const markdown = new MarkdownIt({
    html: false,
    linkify: true,
  }).use(taskLists, { enabled: false });
  dropHtmlComments(markdown);
  markdown.validateLink = safeMarkdownLink;
  markdown.renderer.rules.image = (tokens, index, options, env, renderer) => {
    const token = tokens[index];
    const value = token.attrGet("src");
    const sourcePath = typeof value === "string" ? value : "";
    const path = safeRelativeImagePath(markdownPath, sourcePath);
    if (!path) {
      token.attrSet("src", "");
      return renderer.renderToken(tokens, index, options);
    }

    let imageIndex = imageIndexes.get(path);
    if (imageIndex === undefined) {
      imageIndex = images.length;
      imageIndexes.set(path, imageIndex);
      images.push({ source: sourcePath, path });
    }
    token.attrSet("src", "");
    token.attrSet("data-marvis-image", String(imageIndex));
    return renderer.renderToken(tokens, index, options);
  };

  // Metadata only counts when the file really opens with a fenced block, and only when the block
  // parses: one that is not YAML was never metadata, so the document is rendered whole rather than
  // half-there, which is what a failed parse would leave behind.
  let body = source;
  let frontMatter = "";
  if (opensFrontMatter(source)) {
    try {
      const parsed = matter(source);
      const data = parsed.data as Record<string, unknown> | undefined;
      if (data && typeof data === "object" && !Array.isArray(data)) {
        frontMatter = frontMatterTable(data, markdown.utils.escapeHtml);
        if (frontMatter) body = parsed.content;
      }
    } catch {
      // Not YAML after all: the whole file is the document.
    }
  }

  const sanitized = DOMPurify.sanitize(frontMatter + markdown.render(body), {
    ALLOWED_TAGS: [
      "a",
      "blockquote",
      "br",
      "code",
      "del",
      "em",
      "h1",
      "h2",
      "h3",
      "h4",
      "h5",
      "h6",
      "hr",
      "img",
      "input",
      "li",
      "ol",
      "p",
      "pre",
      "s",
      "span",
      "strong",
      "table",
      "tbody",
      "td",
      "th",
      "thead",
      "tr",
      "ul",
    ],
    ALLOWED_ATTR: ["alt", "checked", "class", "data-marvis-image", "disabled", "href", "src", "title", "type"],
    ALLOW_DATA_ATTR: false,
  });
  // happy-dom (used by Vitest) unwraps table tags while sanitizing table sections.
  // Markdown raw HTML is disabled, so restoring this generated structural wrapper is safe.
  const restored = sanitized.includes("<table")
    ? sanitized
    : sanitized.replace(/(<thead>[\s\S]*?<\/thead>\s*<tbody>[\s\S]*?<\/tbody>)/g, "<table>$1</table>");
  return { html: await highlightFencedCode(restored), images };
}

export function attachMarkdownImages(
  sanitizedHtml: string,
  imagePaths: readonly string[],
  images: ReadonlyMap<string, { mimeType: string; dataBase64: string }>,
): string {
  const document = new DOMParser().parseFromString(sanitizedHtml, "text/html");
  for (const element of document.querySelectorAll("img[data-marvis-image]")) {
    const index = Number(element.getAttribute("data-marvis-image"));
    const path = Number.isInteger(index) ? imagePaths[index] : undefined;
    const image = path ? images.get(path) : undefined;
    if (image && SAFE_IMAGE_MIME_TYPES.has(image.mimeType) && /^[a-z\d+/]+=*$/i.test(image.dataBase64)) {
      element.setAttribute("src", `data:${image.mimeType};base64,${image.dataBase64}`);
    } else {
      element.removeAttribute("src");
    }
    element.removeAttribute("data-marvis-image");
  }
  return document.body.innerHTML;
}
