<script setup lang="ts">
import { computed, nextTick, ref, watch } from "vue";
import type { DocumentMode } from "../domain/main-document";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { useMarkdownPreview } from "../presentation/markdown-preview";
import { isIpcError } from "../domain/ipc";
import { readCheckoutFile } from "../lib/ipc";

const props = withDefaults(
  defineProps<{
    checkout: Checkout | null;
    /** Null while the terminal or the diff owns the panel; the loaded document stays put. */
    path: string | null;
    mode: DocumentMode;
    gitSnapshot: ActiveGitSnapshot;
    refreshRevision?: number;
    readingPosition?: { top: number; left: number };
  }>(),
  { refreshRevision: 0, readingPosition: () => ({ top: 0, left: 0 }) },
);
const emit = defineEmits<{
  updateMode: [mode: DocumentMode];
  readingPositionChanged: [position: { top: number; left: number }];
  openMarkdownLink: [path: string];
}>();

const content = ref("");
const contentState = ref<"idle" | "loading" | "ready" | "error">("idle");
const contentError = ref("");
const contentIdentity = ref<string | null>(null);
const fileViewport = ref<HTMLElement | null>(null);
const deleted = computed(
  () =>
    props.path !== null &&
    (props.gitSnapshot.status?.files.some((file) => file.path === props.path && file.status === "D") ?? false),
);
const available = computed(() => !deleted.value);
const readingPosition = ref(props.readingPosition);
const { markdownHtml, markdownPreviewState, markdownImageWarning, isMarkdownPath, load, clear } = useMarkdownPreview(
  () => props.checkout?.id ?? null,
);
const identity = computed(() => (props.path === null ? null : `${props.checkout?.id ?? ""}\0${props.path}`));
const isMarkdown = computed(() => props.path !== null && isMarkdownPath(props.path));
const sourceLines = computed(() => content.value.split(/\r?\n/));
const sourceLineNumbers = computed(() => sourceLines.value.map((_, index) => index + 1).join("\n"));
const compactSource = computed(() => sourceLines.value.length > 5000);
const highlightedLines = ref<readonly string[] | null>(null);
const highlightedSource = ref<string | null>(null);
const highlighting = ref(false);
let requestGeneration = 0;
let highlightGeneration = 0;
let loadedIdentity: string | null = null;

const highlightableSourceExtensions = new Set([
  "c",
  "cs",
  "css",
  "go",
  "gql",
  "graphql",
  "h",
  "htm",
  "html",
  "java",
  "js",
  "jsx",
  "json",
  "jsonc",
  "kt",
  "kts",
  "less",
  "md",
  "mdown",
  "markdown",
  "php",
  "py",
  "pyw",
  "rs",
  "scss",
  "sh",
  "sql",
  "swift",
  "toml",
  "ts",
  "tsx",
  "vue",
  "xml",
  "yaml",
  "yml",
  "zsh",
]);

function canHighlightSource(path: string): boolean {
  const extension = path.split(".").pop()?.toLowerCase();
  return extension !== undefined && highlightableSourceExtensions.has(extension);
}

function invalidateHighlight(clear = true) {
  highlightGeneration += 1;
  highlighting.value = false;
  if (clear) {
    highlightedLines.value = null;
    highlightedSource.value = null;
  }
}

function startHighlight(fileIdentity: string, source: string) {
  const path = props.path;
  if (path === null) return;
  const request = ++highlightGeneration;
  highlightedLines.value = null;
  highlightedSource.value = null;
  highlighting.value = canHighlightSource(path);
  if (!highlighting.value) return;

  void import("../lib/source-highlighter")
    .then(({ highlightSource }) => highlightSource(path, source))
    .then((lines) => {
      if (
        request !== highlightGeneration ||
        identity.value !== fileIdentity ||
        content.value !== source ||
        (props.mode === "view" && isMarkdown.value)
      )
        return;
      highlightedLines.value = lines;
      highlightedSource.value = lines === null ? null : source;
      highlighting.value = false;
    })
    .catch(() => {
      if (request === highlightGeneration && identity.value === fileIdentity && content.value === source) {
        highlighting.value = false;
      }
    });
}

function errorText(error: unknown): string {
  if (isIpcError(error)) {
    switch (error.code) {
      case "folder_missing":
        return "File or folder no longer exists.";
      case "permission_denied":
        return "Permission denied while reading this file.";
      case "file_too_large":
        return "This file is larger than the preview size limit.";
      case "binary_file":
        return "This file is binary or is not valid UTF-8.";
      case "path_outside_checkout":
        return "This path resolves outside the active checkout and cannot be opened.";
      case "invalid_path":
        return "This path is not a valid file in the checkout.";
      default:
        return error.message;
    }
  }
  return error instanceof Error ? error.message : String(error);
}

function unavailableText() {
  return deleted.value
    ? "This file was deleted in this checkout."
    : "The current file is unavailable in this checkout.";
}

async function loadFile(preservePosition = false) {
  const checkoutId = props.checkout?.id;
  const path = props.path;
  const fileIdentity = identity.value;
  if (!checkoutId || path === null || fileIdentity === null) return;
  if (!available.value) {
    requestGeneration += 1;
    invalidateHighlight();
    content.value = "";
    contentIdentity.value = fileIdentity;
    contentState.value = "error";
    contentError.value = unavailableText();
    return;
  }
  const request = ++requestGeneration;
  const previousPosition = preservePosition ? readingPosition.value : props.readingPosition;
  contentState.value = "loading";
  contentError.value = "";
  try {
    const result = await readCheckoutFile(checkoutId, path);
    if (request !== requestGeneration || props.checkout?.id !== checkoutId || identity.value !== fileIdentity) return;
    content.value = result.content;
    contentIdentity.value = fileIdentity;
    contentState.value = "ready";
    loadedIdentity = fileIdentity;
    if (props.mode === "view" && isMarkdown.value) {
      invalidateHighlight();
    } else {
      startHighlight(fileIdentity, result.content);
    }
    await nextTick();
    if (request !== requestGeneration || props.checkout?.id !== checkoutId || identity.value !== fileIdentity) return;
    if (fileViewport.value) {
      fileViewport.value.scrollTop = previousPosition.top;
      fileViewport.value.scrollLeft = previousPosition.left;
    }
    readingPosition.value = previousPosition;
    if (props.mode === "view" && isMarkdown.value) {
      await load(checkoutId, path, result.content);
      await restoreMarkdownReadingPosition(previousPosition, request, fileIdentity, checkoutId);
    }
  } catch (error) {
    if (request !== requestGeneration || props.checkout?.id !== checkoutId || identity.value !== fileIdentity) return;
    content.value = "";
    contentIdentity.value = fileIdentity;
    contentError.value = errorText(error);
    contentState.value = "error";
    loadedIdentity = fileIdentity;
    invalidateHighlight();
    clear();
  }
}

async function restoreMarkdownReadingPosition(
  position: { top: number; left: number },
  request: number,
  fileIdentity: string,
  checkoutId: string,
) {
  const isCurrent = () =>
    request === requestGeneration && props.checkout?.id === checkoutId && identity.value === fileIdentity;
  await nextTick();
  if (!isCurrent()) return;
  const images = fileViewport.value?.querySelectorAll("img") ?? [];
  await Promise.all(Array.from(images, (image) => image.decode?.().catch(() => undefined) ?? Promise.resolve()));
  await nextTick();
  if (isCurrent() && fileViewport.value) {
    fileViewport.value.scrollTop = position.top;
    fileViewport.value.scrollLeft = position.left;
    readingPosition.value = position;
  }
}

watch(
  () => [identity.value, props.mode, available.value] as const,
  async ([fileIdentity, mode, isAvailable]) => {
    // Another view owns the main panel: what is loaded here stays loaded, and E.3 does not
    // apply because nothing is being selected.
    if (fileIdentity === null) return;
    if (loadedIdentity !== fileIdentity) {
      requestGeneration += 1;
      invalidateHighlight(false);
      loadedIdentity = null;
      contentState.value = contentIdentity.value === null ? "idle" : "loading";
      contentError.value = "";
      clear();
      readingPosition.value = props.readingPosition;
      if (fileViewport.value) {
        fileViewport.value.scrollTop = 0;
        fileViewport.value.scrollLeft = 0;
      }
    }
    if (isAvailable === false) {
      requestGeneration += 1;
      invalidateHighlight();
      content.value = "";
      contentIdentity.value = fileIdentity;
      contentState.value = "error";
      contentError.value = unavailableText();
      clear();
      return;
    }
    if (contentState.value !== "ready" || loadedIdentity !== fileIdentity) {
      await loadFile();
      return;
    }
    if (mode === "view" && isMarkdown.value && contentState.value === "ready") {
      const checkoutId = props.checkout?.id;
      if (checkoutId) {
        const request = requestGeneration;
        const position = readingPosition.value;
        await load(checkoutId, props.path!, content.value);
        await restoreMarkdownReadingPosition(position, request, fileIdentity, checkoutId);
      }
    } else {
      if (contentState.value === "ready") startHighlight(fileIdentity, content.value);
      clear();
    }
  },
  { immediate: true, flush: "sync" },
);

watch(
  () => [props.gitSnapshot.statusEventRevision, props.gitSnapshot.statusEventCheckoutId] as const,
  ([, eventCheckoutId]) => {
    if (
      props.path !== null &&
      props.checkout?.id === eventCheckoutId &&
      props.gitSnapshot.checkoutId === eventCheckoutId
    ) {
      void loadFile(true);
    }
  },
);

watch(
  () => props.refreshRevision,
  (revision, previous) => {
    if (revision !== previous && props.checkout?.id && props.path !== null) void loadFile(true);
  },
);

function onFileScroll(event: Event) {
  const viewport = event.currentTarget as HTMLElement;
  readingPosition.value = { top: viewport.scrollTop, left: viewport.scrollLeft };
  emit("readingPositionChanged", readingPosition.value);
}

function onMarkdownLink(event: MouseEvent) {
  const target = event.target;
  if (!(target instanceof Element) || props.path === null) return;
  const link = target.closest("a[href]");
  const href = link?.getAttribute("href");
  if (!href || href.startsWith("#") || /^[a-z][a-z\d+.-]*:/i.test(href) || href.startsWith("/") || href.includes("\\"))
    return;
  try {
    const base = props.path.split("/").slice(0, -1);
    for (const part of decodeURIComponent(href.split(/[?#]/, 1)[0] ?? "").split("/")) {
      if (!part || part === ".") continue;
      if (part === "..") {
        if (!base.length) return;
        base.pop();
      } else base.push(part);
    }
    const path = base.join("/");
    if (!path || path.includes("\0") || !isMarkdownPath(path)) return;
    event.preventDefault();
    emit("openMarkdownLink", path);
  } catch {
    // Ignore malformed encoded relative paths.
  }
}
</script>

<template>
  <main class="document-pane flex min-h-0 flex-1 flex-col">
    <header class="document-toolbar flex h-10 shrink-0 items-center justify-between gap-3 border-b px-3">
      <span class="min-w-0 truncate text-[11px] text-(--marvis-text-dim)" :title="path ?? undefined">{{
        path ?? ""
      }}</span>
      <div
        v-if="isMarkdown"
        role="group"
        aria-label="Document mode"
        class="document-mode-control flex shrink-0 items-center gap-0.5"
      >
        <button
          type="button"
          :aria-pressed="mode === 'view'"
          class="document-mode-button rounded-sm px-2.5 py-1 text-[11px]"
          @click="$emit('updateMode', 'view')"
        >
          View
        </button>
        <button
          type="button"
          :aria-pressed="mode === 'code'"
          class="document-mode-button rounded-sm px-2.5 py-1 text-[11px]"
          @click="$emit('updateMode', 'code')"
        >
          Code
        </button>
      </div>
    </header>
    <section ref="fileViewport" class="min-h-0 flex-1 overflow-auto" aria-label="File contents" @scroll="onFileScroll">
      <p v-if="available === false" role="status" class="pane-state text-sm">
        {{ unavailableText() }}
      </p>
      <!-- E.3: the previous document is not what stays on screen while a new one is read. -->
      <p
        v-else-if="contentState === 'loading' && contentIdentity !== identity"
        role="status"
        class="pane-state text-sm"
      >
        Loading file…
      </p>
      <p v-else-if="contentState === 'error'" role="alert" class="pane-state text-sm">
        {{ contentError }}
      </p>
      <p v-else-if="contentState === 'ready' && content.length === 0" role="status" class="pane-state text-sm">
        This file is empty.
      </p>
      <p v-else-if="highlighting" role="status" class="pane-state text-sm">Highlighting source…</p>
      <template v-else-if="contentState === 'ready' || (contentState === 'loading' && contentIdentity === identity)">
        <template v-if="mode === 'view' && isMarkdown">
          <p
            v-if="markdownPreviewState === 'loading'"
            role="status"
            class="px-3 pt-3 text-xs text-(--marvis-text-faint)"
          >
            Rendering preview…
          </p>
          <p v-if="markdownImageWarning" role="status" class="px-3 pt-3 text-xs text-(--marvis-red)">
            Some Markdown images were missing, unsupported, or over the preview limits.
          </p>
          <!-- eslint-disable vue/no-v-html -- Content is generated and DOMPurify-sanitized in markdown-preview.ts. -->
          <article
            v-if="markdownPreviewState === 'ready'"
            class="markdown-preview p-3 text-sm"
            @click="onMarkdownLink"
            v-html="markdownHtml"
          />
          <!-- eslint-enable vue/no-v-html -->
        </template>
        <div
          v-else-if="compactSource"
          class="flex py-2 font-mono text-[13px] leading-5 text-(--marvis-text)"
          aria-label="Source code"
        >
          <pre class="source-line-number" aria-hidden="true">{{ sourceLineNumbers }}</pre>
          <pre class="source-code min-w-max whitespace-pre">
            <!-- eslint-disable vue/no-v-html -- Code is generated by Shiki and DOMPurify-sanitized. -->
            <code v-if="highlightedLines !== null" class="shiki" v-html="highlightedLines.join('\n')" />
            <!-- eslint-enable vue/no-v-html -->
            <code v-else>{{ content }}</code>
          </pre>
        </div>
        <div
          v-else
          class="min-w-max py-2 font-mono text-[13px] leading-5 text-(--marvis-text)"
          aria-label="Source code"
        >
          <div v-for="(line, index) in sourceLines" :key="index" class="flex min-h-5 whitespace-pre">
            <span class="source-line-number">{{ index + 1 }}</span>
            <!-- eslint-disable vue/no-v-html -- Line fragments come from one sanitized Shiki render. -->
            <code v-if="highlightedLines !== null" class="shiki min-w-max px-3" v-html="highlightedLines[index]" />
            <!-- eslint-enable vue/no-v-html -->
            <code v-else class="min-w-max px-3">{{ line }}</code>
          </div>
        </div>
      </template>
    </section>
  </main>
</template>

<style scoped>
.markdown-preview {
  max-width: 78ch;
  margin: 0 auto;
  padding: 2rem 1.5rem 3rem;
  color: var(--marvis-text);
  font-size: 0.875rem;
  line-height: 1.75;
}

.markdown-preview :deep(h1),
.markdown-preview :deep(h2),
.markdown-preview :deep(h3) {
  margin: 2rem 0 0.65rem;
  color: var(--marvis-text);
  font-weight: 650;
  line-height: 1.3;
}

.markdown-preview :deep(h1) {
  margin-top: 0;
  font-size: 1.5rem;
}

.markdown-preview :deep(h2) {
  font-size: 1.15rem;
}

.markdown-preview :deep(p),
.markdown-preview :deep(ul),
.markdown-preview :deep(ol),
.markdown-preview :deep(blockquote) {
  margin: 0.9rem 0;
  color: var(--marvis-text);
}

.markdown-preview :deep(ul),
.markdown-preview :deep(ol) {
  padding-left: 1.4rem;
  list-style: revert;
}

.markdown-preview :deep(a) {
  color: var(--marvis-accent);
  text-decoration: underline;
}

.markdown-preview :deep(blockquote) {
  border-left: 2px solid var(--marvis-border);
  padding-left: 0.75rem;
  color: var(--marvis-text-secondary);
}

.markdown-preview :deep(table) {
  width: max-content;
  max-width: 100%;
  border-collapse: collapse;
  margin: 1.25rem 0;
}

.markdown-preview :deep(th),
.markdown-preview :deep(td) {
  border: 1px solid var(--marvis-border);
  padding: 0.45rem 0.65rem;
  text-align: left;
}

.markdown-preview :deep(pre) {
  overflow: auto;
  margin: 1.1rem 0;
  border: 1px solid var(--marvis-border);
  border-radius: var(--marvis-radius);
  background: var(--marvis-bg-1);
  padding: 0.85rem 1rem;
  font-size: 0.75rem;
  line-height: 1.65;
}

.markdown-preview :deep(code:not(pre code)) {
  border-radius: 2px;
  background: var(--marvis-bg-2);
  padding: 0.1rem 0.25rem;
  font-family: var(--marvis-font);
  font-size: 0.85em;
}

/* Shiki's own <pre> is dropped so this rule owns the block surface, which also drops the base
   color it carried. Unstyled runs of a code block have no span of their own, so the text color
   has to come from here or those runs fall back to the dimmer document color. */
.markdown-preview :deep(pre code) {
  color: #e6edf3;
  font-family: var(--marvis-font);
}

.markdown-preview :deep(img) {
  max-width: 100%;
  height: auto;
}

.source-line-number {
  position: sticky;
  left: 0;
  margin: 0;
  width: 3rem;
  flex-shrink: 0;
  user-select: none;
  background: var(--marvis-bg-0);
  border-right: 1px solid var(--marvis-border);
  padding-right: 0.75rem;
  color: var(--marvis-text-faint);
  text-align: right;
  white-space: pre;
}

.source-code {
  margin: 0;
  padding-left: 0.75rem;
}
</style>
