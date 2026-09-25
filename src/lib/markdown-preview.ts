import DOMPurify from "dompurify";
import hljs from "highlight.js/lib/common";
import MarkdownIt from "markdown-it";
import taskLists from "markdown-it-task-lists";
import "highlight.js/styles/github-dark.css";

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

export function isMarkdownPath(path: string): boolean {
  const extension = path.split(".").pop()?.toLowerCase();
  return extension !== undefined && markdownExtensions.has(extension);
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

export function renderMarkdownPreview(source: string, markdownPath: string): MarkdownPreview {
  const images: MarkdownImageReference[] = [];
  const imageIndexes = new Map<string, number>();
  const markdown = new MarkdownIt({
    html: false,
    linkify: true,
    highlight(code, language) {
      if (language && hljs.getLanguage(language)) {
        return hljs.highlight(code, { language }).value;
      }
      return MarkdownIt().utils.escapeHtml(code);
    },
  }).use(taskLists, { enabled: false });
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

  const sanitized = DOMPurify.sanitize(markdown.render(source), {
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
  const html = sanitized.includes("<table")
    ? sanitized
    : sanitized.replace(/(<thead>[\s\S]*?<\/thead>\s*<tbody>[\s\S]*?<\/tbody>)/g, "<table>$1</table>");
  return { html, images };
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
