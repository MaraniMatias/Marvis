import DOMPurify from "dompurify";
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

/** One key of the metadata block, already flattened to what the page shows beside it. */
interface FrontMatterEntry {
  key: string;
  value: string;
}

/**
 * The `---` fenced block the document opens with and the body behind it, or null when the document
 * opens with something else.
 *
 * The shape of the file decides this, before anything is read out of it. A document that merely
 * *starts* with a rule is not front matter, and a reader left to guess takes the rest of the file
 * for metadata and hands back an empty page: the heading the file opens with is one of its values.
 * Cutting the body here is what keeps the fence and what is read out of it from disagreeing.
 */
function frontMatterBlock(source: string): { block: string[]; body: string } | null {
  const lines = source.split(/\r?\n/);
  if (lines[0]?.trim() !== "---") return null;
  const closing = lines.findIndex((line, index) => index > 0 && line.trim() === "---");
  if (closing < 0) return null;
  return { block: lines.slice(1, closing), body: lines.slice(closing + 1).join("\n") };
}

/** The text a scalar carries, without the quotes it may be written in. */
function unquote(value: string): string {
  const text = value.trim();
  const quote = text[0];
  if ((quote === '"' || quote === "'") && text.length > 1 && text.endsWith(quote)) {
    return text.slice(1, -1).replace(/\\(["'\\])/g, "$1");
  }
  return text;
}

/** `[a, b]` is the one sequence YAML writes on one line; anything else is left as it was written. */
function inlineList(value: string): string {
  const inner = value.trim();
  if (!inner.startsWith("[") || !inner.endsWith("]")) return unquote(inner);
  return inner
    .slice(1, -1)
    .split(",")
    .map((item) => unquote(item))
    .filter(Boolean)
    .join(", ");
}

/** `|` and `>` are the two ways a value is written across the lines under it. */
const BLOCK_SCALAR = /^[|>][-+]?$/;

/** The indentation that makes a line part of the key above it rather than a key of its own. */
function indentOf(line: string): number {
  return line.length - line.trimStart().length;
}

/**
 * Every key of the block, with the value beside it as one line of text.
 *
 * The nesting is flattened into the key (`permission.task.*`) rather than parsed, because what this
 * is for is a header the reader reads, not a value they can look up. A block with no key in it is
 * not metadata and returns null, so the document is rendered whole.
 */
function readFrontMatter(block: readonly string[]): FrontMatterEntry[] | null {
  const entries: FrontMatterEntry[] = [];
  const parents: { indent: number; key: string }[] = [];
  let foundKey = false;

  for (let index = 0; index < block.length; index += 1) {
    const line = block[index];
    if (line.trim() === "" || line.trimStart().startsWith("#")) continue;
    const indent = indentOf(line);
    const text = line.trim();

    // A sequence item belongs to the key above it, and joins whatever that key already says.
    if (text.startsWith("- ") || text === "-") {
      const owner = [...parents].reverse().find((parent) => parent.indent < indent);
      const item = unquote(text.slice(1));
      if (!owner || !item) continue;
      const entry = entries.find((candidate) => candidate.key === owner.key);
      if (!entry) continue;
      entry.value = entry.value ? `${entry.value}, ${item}` : item;
      continue;
    }

    const separator = text.indexOf(":");
    if (separator < 1) {
      // A line that is neither a key nor a sequence item belongs to a value written across lines.
      const owner = [...parents].reverse().find((parent) => parent.indent < indent);
      const entry = owner ? entries.find((candidate) => candidate.key === owner.key) : undefined;
      if (entry?.value) {
        entry.value = BLOCK_SCALAR.test(entry.value) ? text : `${entry.value} ${text}`;
        continue;
      }
      // Nothing above it wanted it, so this line is not part of a mapping.
      if (indent === 0) return null;
      continue;
    }

    const name = unquote(text.slice(0, separator));
    if (name === "") {
      if (indent === 0) return null;
      continue;
    }
    while (parents.length > 0 && parents[parents.length - 1].indent >= indent) parents.pop();
    // The nearest enclosing key already spells out the ones above it, so it is the only prefix.
    const key = parents.length > 0 ? `${parents[parents.length - 1].key}.${name}` : name;
    parents.push({ indent, key });
    foundKey = true;
    // A key written twice is one key, and the second one is what it says: a header that showed the
    // same key twice would say the file has two values where it has one.
    const value = inlineList(text.slice(separator + 1));
    const existing = entries.find((candidate) => candidate.key === key);
    if (existing) existing.value = value;
    else entries.push({ key, value });
  }

  if (!foundKey || entries.length === 0) return null;
  // A key that is only there to hold others has nothing of its own to show, and the children
  // already carry its name. A key with no value and no children does show: that it is set to
  // nothing is part of what the file says.
  const holders = new Set<string>();
  for (const entry of entries) {
    const parts = entry.key.split(".");
    for (let part = 1; part < parts.length; part += 1) holders.add(parts.slice(0, part).join("."));
  }
  return entries.filter((entry) => !(holders.has(entry.key) && entry.value === "")).slice(0, MAX_FRONTMATTER_KEYS);
}

/** What one value says on the page: one line, and no longer than a header can carry. */
function frontMatterLine(value: string): string {
  const text = value.replace(/\s+/g, " ").trim();
  return text.length > MAX_FRONTMATTER_VALUE_LENGTH ? `${text.slice(0, MAX_FRONTMATTER_VALUE_LENGTH)}…` : text;
}

/**
 * The metadata block GitHub puts above a document that carries some, as the key/value pairs it is.
 * It is built here rather than parsed out of a rendered string so every value reaches the page
 * escaped, and it goes through the same sanitizer as the document it precedes.
 *
 * Exported for the one thing no other test can see: the class the layout in DocumentPane.vue hangs
 * on. A sanitizer is free to drop the attributes of the element it is handed, and the test DOM does
 * exactly that, so this is asserted on the markup before the sanitizer rather than after it.
 */
export function frontMatterList(entries: readonly FrontMatterEntry[], escape: (text: string) => string): string {
  if (entries.length === 0) return "";
  const pairs = entries
    .map((entry) => `<dt>${escape(entry.key)}</dt><dd>${escape(frontMatterLine(entry.value))}</dd>`)
    .join("");
  return `<dl class="markdown-frontmatter">${pairs}</dl>`;
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

  // Metadata only counts when the file really opens with a fenced block, and only when what is
  // inside it is a mapping: anything else was never metadata, so the document is rendered whole
  // rather than half-there, which is what a failed read would leave behind.
  let body = source;
  let frontMatter = "";
  const fenced = frontMatterBlock(source);
  if (fenced) {
    const entries = readFrontMatter(fenced.block);
    if (entries) {
      frontMatter = frontMatterList(entries, markdown.utils.escapeHtml);
      if (frontMatter) body = fenced.body;
    }
  }

  const sanitized = DOMPurify.sanitize(frontMatter + markdown.render(body), {
    ALLOWED_TAGS: [
      "a",
      "blockquote",
      "br",
      "code",
      "dd",
      "del",
      "dl",
      "dt",
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
