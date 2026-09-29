import { onBeforeUnmount, ref } from "vue";
import { readCheckoutMarkdownImage } from "../lib/ipc";
import type { DocumentOrigin } from "../domain/main-document";

const MAX_MARKDOWN_IMAGES = 12;
const MAX_MARKDOWN_IMAGE_BYTES = 8 * 1024 * 1024;
const MARKDOWN_EXTENSIONS = new Set(["md", "markdown", "mdown", "mkd"]);

export function isMarkdownPath(path: string): boolean {
  const extension = path.split(".").pop()?.toLowerCase();
  return extension !== undefined && MARKDOWN_EXTENSIONS.has(extension);
}

export function useMarkdownPreview(getCheckoutId: () => string | null, getOrigin: () => DocumentOrigin) {
  const markdownHtml = ref("");
  const markdownPreviewState = ref<"idle" | "loading" | "ready">("idle");
  const markdownImageWarning = ref(false);
  let generation = 0;
  let disposed = false;

  function isCurrent(requestGeneration: number, checkoutId: string): boolean {
    return !disposed && requestGeneration === generation && getCheckoutId() === checkoutId;
  }

  function invalidate() {
    generation += 1;
  }

  function clear() {
    invalidate();
    if (disposed) return;
    markdownHtml.value = "";
    markdownPreviewState.value = "idle";
    markdownImageWarning.value = false;
  }

  /**
   * Renders `source` as the preview of `path`, or leaves the page that is already up when
   * `keepVisible` says the file on screen is the one being re-read. The document is re-read twice
   * for every change anywhere in the checkout, and emptying the rendered page each time is a blink
   * the reader did nothing to cause.
   */
  async function load(checkoutId: string, path: string, source: string, keepVisible = false) {
    if (disposed) return;
    const requestGeneration = ++generation;
    const settled = keepVisible && markdownPreviewState.value === "ready";
    if (!settled) {
      markdownHtml.value = "";
      markdownPreviewState.value = "loading";
    }
    markdownImageWarning.value = false;
    if (!isMarkdownPath(path)) {
      markdownPreviewState.value = "idle";
      return;
    }
    const { attachMarkdownImages, renderMarkdownPreview } = await import("../lib/markdown-preview");
    if (!isCurrent(requestGeneration, checkoutId)) return;
    const preview = await renderMarkdownPreview(source, path);
    const paths = preview.images.map((image) => image.path);
    const loadedImages = new Map<string, { mimeType: string; dataBase64: string }>();
    if (getOrigin() === "review") {
      markdownHtml.value = attachMarkdownImages(preview.html, paths, loadedImages);
      markdownPreviewState.value = "ready";
      return;
    }
    markdownImageWarning.value = preview.images.length > MAX_MARKDOWN_IMAGES;

    let totalImageBytes = 0;
    for (const image of preview.images.slice(0, MAX_MARKDOWN_IMAGES)) {
      if (!isCurrent(requestGeneration, checkoutId)) return;
      try {
        const resource = await readCheckoutMarkdownImage(checkoutId, path, image.source);
        if (!isCurrent(requestGeneration, checkoutId)) return;
        if (resource.sizeBytes > MAX_MARKDOWN_IMAGE_BYTES - totalImageBytes) {
          markdownImageWarning.value = true;
          continue;
        }
        totalImageBytes += resource.sizeBytes;
        loadedImages.set(image.path, resource);
      } catch {
        if (!isCurrent(requestGeneration, checkoutId)) return;
        markdownImageWarning.value = true;
      }
    }

    if (!isCurrent(requestGeneration, checkoutId)) return;
    markdownHtml.value = attachMarkdownImages(preview.html, paths, loadedImages);
    markdownPreviewState.value = "ready";
  }

  onBeforeUnmount(() => {
    disposed = true;
    invalidate();
  });

  return {
    markdownHtml,
    markdownPreviewState,
    markdownImageWarning,
    isMarkdownPath,
    load,
    clear,
    invalidate,
  };
}
