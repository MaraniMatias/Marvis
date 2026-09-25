<script setup lang="ts">
import DOMPurify from "dompurify";
import hljs from "highlight.js/lib/common";
import "highlight.js/styles/github-dark.css";
import { computed, defineAsyncComponent, nextTick, ref, watch } from "vue";
import type { MainDocument, MainDocumentMode } from "../domain/main-document";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { useMarkdownPreview } from "../presentation/markdown-preview";
import { isIpcError } from "../domain/ipc";
import { readCheckoutFile } from "../lib/ipc";

const FileDiff = defineAsyncComponent(() => import("./FileDiff.vue"));

const props = withDefaults(
  defineProps<{
    checkout: Checkout | null;
    document: MainDocument;
    gitSnapshot: ActiveGitSnapshot;
    active?: boolean;
    refreshRevision?: number;
    readingPosition?: { top: number; left: number };
    diffScrollTop?: number;
    zedAvailable?: boolean;
  }>(),
  {
    active: true,
    refreshRevision: 0,
    readingPosition: () => ({ top: 0, left: 0 }),
    diffScrollTop: 0,
    zedAvailable: false,
  },
);
const emit = defineEmits<{
  updateMode: [mode: MainDocumentMode];
  readingPositionChanged: [position: { top: number; left: number }];
  diffPositionChanged: [top: number];
  openMarkdownLink: [path: string];
  openInZed: [];
}>();

const content = ref("");
const contentState = ref<"idle" | "loading" | "ready" | "error">("idle");
const contentError = ref("");
const fileViewport = ref<HTMLElement | null>(null);
const wrapCode = ref(false);
const deleted = computed(
  () =>
    props.gitSnapshot.status?.files.some((file) => file.path === props.document.path && file.status === "D") ?? false,
);
const available = computed(() => !deleted.value);
const readingPosition = ref(props.readingPosition);
const { markdownHtml, markdownPreviewState, markdownImageWarning, isMarkdownPath, load, clear } = useMarkdownPreview(
  () => props.checkout?.id ?? null,
);
const identity = computed(() => `${props.document.checkoutId}\0${props.document.source}\0${props.document.path}`);
const isMarkdown = computed(() => isMarkdownPath(props.document.path));
const sourceLines = computed(() => content.value.split(/\r?\n/));
const sourceLineNumbers = computed(() => sourceLines.value.map((_, index) => index + 1).join("\n"));
const compactSource = computed(() => sourceLines.value.length > 5000);
const highlightedSource = computed(() => {
  const extension = props.document.path.split(".").pop()?.toLowerCase();
  const language = extension && hljs.getLanguage(extension) ? extension : undefined;
  if (!language) return null;
  const html = hljs.highlight(content.value, { language, ignoreIllegals: true }).value;
  const sanitized = DOMPurify.sanitize(html, {
    ALLOWED_TAGS: ["span"],
    ALLOWED_ATTR: ["class"],
  });
  return sanitized;
});
const highlightedLines = computed(() =>
  highlightedSource.value === null ? null : splitHighlightedLines(highlightedSource.value),
);
let requestGeneration = 0;
let loadedIdentity: string | null = null;

function splitHighlightedLines(html: string): string[] {
  const lines: string[] = [];
  const openSpans: string[] = [];
  const tagsAndBreaks = /<span class="[^"]*">|<\/span>|\r?\n/g;
  let line = "";
  let cursor = 0;
  for (const match of html.matchAll(tagsAndBreaks)) {
    const token = match[0];
    const index = match.index;
    line += html.slice(cursor, index);
    if (token === "\n" || token === "\r\n") {
      lines.push(`${line}${"</span>".repeat(openSpans.length)}`);
      line = openSpans.join("");
    } else if (token.startsWith("</")) {
      line += token;
      openSpans.pop();
    } else {
      line += token;
      openSpans.push(token);
    }
    cursor = index + token.length;
  }
  line += html.slice(cursor);
  lines.push(line);
  return lines;
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

async function loadFile(preservePosition = false) {
  const checkoutId = props.checkout?.id;
  const path = props.document.path;
  const fileIdentity = identity.value;
  if (!checkoutId || props.document.checkoutId !== checkoutId || props.document.mode === "diff") return;
  if (!available.value) {
    requestGeneration += 1;
    content.value = "";
    contentState.value = "error";
    contentError.value = deleted.value
      ? "This file was deleted; its previous contents are available in the diff."
      : "The current file is unavailable in this checkout.";
    return;
  }
  const request = ++requestGeneration;
  const previousPosition = preservePosition ? readingPosition.value : props.readingPosition;
  content.value = "";
  contentState.value = "loading";
  contentError.value = "";
  try {
    const result = await readCheckoutFile(checkoutId, path);
    if (request !== requestGeneration || props.checkout?.id !== checkoutId || identity.value !== fileIdentity) return;
    content.value = result.content;
    contentState.value = "ready";
    loadedIdentity = fileIdentity;
    await nextTick();
    if (request !== requestGeneration || props.checkout?.id !== checkoutId || identity.value !== fileIdentity) return;
    if (fileViewport.value) {
      fileViewport.value.scrollTop = previousPosition.top;
      fileViewport.value.scrollLeft = previousPosition.left;
    }
    readingPosition.value = previousPosition;
    if (props.document.mode === "view" && isMarkdown.value) {
      await load(checkoutId, path, result.content);
      await restoreMarkdownReadingPosition(previousPosition, request, fileIdentity, checkoutId);
    }
  } catch (error) {
    if (request !== requestGeneration || props.checkout?.id !== checkoutId || identity.value !== fileIdentity) return;
    contentError.value = errorText(error);
    contentState.value = "error";
    loadedIdentity = fileIdentity;
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
    request === requestGeneration &&
    props.checkout?.id === checkoutId &&
    identity.value === fileIdentity &&
    props.document.mode === "view";
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
  () => [identity.value, props.document.mode, available.value] as const,
  async ([fileIdentity, mode, isAvailable]) => {
    if (loadedIdentity !== fileIdentity) {
      requestGeneration += 1;
      loadedIdentity = null;
      content.value = "";
      contentState.value = "idle";
      contentError.value = "";
      clear();
      readingPosition.value = props.readingPosition;
      if (fileViewport.value) {
        fileViewport.value.scrollTop = 0;
        fileViewport.value.scrollLeft = 0;
      }
    }
    if (mode === "diff") {
      requestGeneration += 1;
      clear();
      return;
    }
    if (isAvailable === false) {
      requestGeneration += 1;
      content.value = "";
      contentState.value = "error";
      contentError.value = deleted.value
        ? "This file was deleted; its previous contents are available in the diff."
        : "The current file is unavailable in this checkout.";
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
        const fileIdentity = identity.value;
        const position = readingPosition.value;
        await load(checkoutId, props.document.path, content.value);
        await restoreMarkdownReadingPosition(position, request, fileIdentity, checkoutId);
      }
    } else clear();
  },
  { immediate: true, flush: "sync" },
);

watch(
  () => [props.gitSnapshot.statusEventRevision, props.gitSnapshot.statusEventCheckoutId] as const,
  ([, eventCheckoutId]) => {
    if (
      props.document.mode !== "diff" &&
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
    if (revision !== previous && props.checkout?.id === props.document.checkoutId && props.document.mode !== "diff") {
      void loadFile(true);
    }
  },
);

function setFileMode() {
  emit("updateMode", isMarkdown.value ? "view" : "code");
}

function onFileScroll(event: Event) {
  const viewport = event.currentTarget as HTMLElement;
  readingPosition.value = { top: viewport.scrollTop, left: viewport.scrollLeft };
  emit("readingPositionChanged", readingPosition.value);
}

function onMarkdownLink(event: MouseEvent) {
  const target = event.target;
  if (!(target instanceof Element)) return;
  const link = target.closest("a[href]");
  const href = link?.getAttribute("href");
  if (!href || href.startsWith("#") || /^[a-z][a-z\d+.-]*:/i.test(href) || href.startsWith("/") || href.includes("\\"))
    return;
  try {
    const base = props.document.path.split("/").slice(0, -1);
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
    <header class="flex h-11 shrink-0 items-center justify-between gap-3 border-b border-white/8 px-4">
      <span class="min-w-0 truncate font-mono text-[11px] text-zinc-400" :title="document.path">{{
        document.path
      }}</span>
      <div role="group" aria-label="Document mode" class="flex shrink-0 items-center gap-1">
        <button
          type="button"
          :disabled="!zedAvailable"
          title="Open in Zed"
          aria-label="Open file in Zed"
          class="rounded px-2 py-1 text-[11px] text-zinc-500 hover:bg-white/8 hover:text-zinc-200 disabled:cursor-not-allowed disabled:opacity-40"
          @click="$emit('openInZed')"
        >
          ↗ Zed
        </button>
        <button
          v-if="document.mode !== 'diff'"
          type="button"
          :aria-pressed="wrapCode"
          :disabled="compactSource"
          :title="
            compactSource
              ? 'Wrapping is unavailable for files with more than 5,000 lines'
              : 'Toggle source line wrapping'
          "
          class="rounded px-2 py-1 text-[11px] text-zinc-500 hover:bg-white/8 hover:text-zinc-200 disabled:cursor-not-allowed disabled:opacity-40"
          @click="wrapCode = !wrapCode"
        >
          Wrap
        </button>
        <template v-if="document.source === 'change'">
          <button
            type="button"
            :aria-pressed="document.mode === 'diff'"
            class="rounded px-2.5 py-1 text-[11px]"
            :class="document.mode === 'diff' ? 'bg-white/10 text-zinc-100' : 'text-zinc-500 hover:text-zinc-300'"
            @click="$emit('updateMode', 'diff')"
          >
            Diff
          </button>
          <button
            v-if="!isMarkdown"
            type="button"
            :aria-pressed="document.mode !== 'diff'"
            :disabled="available === false"
            class="rounded px-2.5 py-1 text-[11px] disabled:cursor-not-allowed disabled:opacity-40"
            :class="document.mode !== 'diff' ? 'bg-white/10 text-zinc-100' : 'text-zinc-500 hover:text-zinc-300'"
            @click="setFileMode"
          >
            File
          </button>
        </template>
        <template v-if="isMarkdown">
          <button
            type="button"
            :aria-pressed="document.mode === 'view'"
            :disabled="document.source === 'change' && available === false"
            class="rounded px-2.5 py-1 text-[11px]"
            :class="
              document.mode === 'view'
                ? 'bg-white/10 text-zinc-100'
                : 'text-zinc-500 hover:text-zinc-300 disabled:opacity-40'
            "
            @click="$emit('updateMode', 'view')"
          >
            View
          </button>
          <button
            type="button"
            :aria-pressed="document.mode === 'code'"
            :disabled="document.source === 'change' && available === false"
            class="rounded px-2.5 py-1 text-[11px]"
            :class="
              document.mode === 'code'
                ? 'bg-white/10 text-zinc-100'
                : 'text-zinc-500 hover:text-zinc-300 disabled:opacity-40'
            "
            @click="$emit('updateMode', 'code')"
          >
            Code
          </button>
        </template>
        <span v-else-if="document.mode !== 'diff'" class="rounded bg-white/6 px-2.5 py-1 text-[11px] text-zinc-500">
          Code
        </span>
      </div>
    </header>
    <FileDiff
      v-if="document.source === 'change' && document.mode === 'diff' && checkout"
      :key="`${document.checkoutId}:${document.path}`"
      class="min-h-0 flex-1"
      :checkout="checkout"
      :git-snapshot="gitSnapshot"
      :path="document.path"
      :active="active"
      :scroll-top="diffScrollTop"
      @scroll-position-changed="$emit('diffPositionChanged', $event)"
    />
    <section
      v-else
      ref="fileViewport"
      class="min-h-0 flex-1 overflow-auto"
      aria-label="File contents"
      @scroll="onFileScroll"
    >
      <p v-if="available === false" role="status" class="p-5 text-sm text-amber-300">
        {{
          deleted
            ? "This file was deleted; its previous contents are available in the diff."
            : "The current file is unavailable in this checkout."
        }}
      </p>
      <p v-else-if="contentState === 'loading'" role="status" class="p-5 text-sm text-zinc-400">Loading file…</p>
      <p v-else-if="contentState === 'error'" role="alert" class="p-5 text-sm text-amber-300">{{ contentError }}</p>
      <p v-else-if="contentState === 'ready' && content.length === 0" role="status" class="p-5 text-sm text-zinc-500">
        This file is empty.
      </p>
      <template v-else-if="contentState === 'ready'">
        <template v-if="document.mode === 'view' && isMarkdown">
          <p v-if="markdownPreviewState === 'loading'" role="status" class="px-5 pt-4 text-xs text-zinc-500">
            Loading relative images…
          </p>
          <p v-if="markdownImageWarning" role="status" class="px-5 pt-4 text-xs text-amber-300">
            Some Markdown images were missing, unsupported, or over the preview limits.
          </p>
          <!-- eslint-disable vue/no-v-html -- Content is generated and DOMPurify-sanitized in markdown-preview.ts. -->
          <article
            v-if="markdownPreviewState === 'ready'"
            class="markdown-preview p-5 text-sm"
            @click="onMarkdownLink"
            v-html="markdownHtml"
          />
          <!-- eslint-enable vue/no-v-html -->
        </template>
        <div
          v-else-if="compactSource"
          class="flex py-3 font-mono text-[13px] leading-5 text-zinc-300"
          aria-label="Source code"
        >
          <pre class="source-line-number" aria-hidden="true">{{ sourceLineNumbers }}</pre>
          <pre class="source-code min-w-max whitespace-pre">
            <!-- eslint-disable vue/no-v-html -- Code is generated by highlight.js and DOMPurify-sanitized. -->
            <code v-if="highlightedSource !== null" class="hljs" v-html="highlightedSource" />
            <!-- eslint-enable vue/no-v-html -->
            <code v-else>{{ content }}</code>
          </pre>
        </div>
        <div
          v-else
          class="py-3 font-mono text-[13px] leading-5 text-zinc-300"
          :class="wrapCode ? 'w-full min-w-0' : 'min-w-max'"
          aria-label="Source code"
        >
          <div
            v-for="(line, index) in sourceLines"
            :key="index"
            class="flex min-h-5"
            :class="wrapCode ? 'whitespace-pre-wrap break-all' : 'whitespace-pre'"
          >
            <span class="source-line-number">{{ index + 1 }}</span>
            <!-- eslint-disable vue/no-v-html -- Line fragments come from one sanitized highlight.js render. -->
            <code
              v-if="highlightedLines !== null"
              class="hljs px-3"
              :class="wrapCode ? 'min-w-0 whitespace-pre-wrap break-all' : 'min-w-max'"
              v-html="highlightedLines[index]"
            />
            <!-- eslint-enable vue/no-v-html -->
            <code v-else class="px-3" :class="wrapCode ? 'min-w-0 whitespace-pre-wrap break-all' : 'min-w-max'">{{
              line
            }}</code>
          </div>
        </div>
      </template>
    </section>
  </main>
</template>

<style scoped>
.markdown-preview :deep(h1),
.markdown-preview :deep(h2),
.markdown-preview :deep(h3) {
  margin: 1.25rem 0 0.6rem;
  color: #e4e4e7;
  font-weight: 650;
}

.markdown-preview :deep(h1) {
  font-size: 1.35rem;
}

.markdown-preview :deep(h2) {
  font-size: 1.15rem;
}

.markdown-preview :deep(p),
.markdown-preview :deep(ul),
.markdown-preview :deep(ol),
.markdown-preview :deep(blockquote) {
  margin: 0.65rem 0;
  color: #d4d4d8;
}

.markdown-preview :deep(ul),
.markdown-preview :deep(ol) {
  padding-left: 1.4rem;
  list-style: revert;
}

.markdown-preview :deep(a) {
  color: #93c5fd;
  text-decoration: underline;
}

.markdown-preview :deep(blockquote) {
  border-left: 2px solid #52525b;
  padding-left: 0.75rem;
}

.markdown-preview :deep(table) {
  width: 100%;
  border-collapse: collapse;
  margin: 0.9rem 0;
}

.markdown-preview :deep(th),
.markdown-preview :deep(td) {
  border: 1px solid #3f3f46;
  padding: 0.35rem 0.5rem;
  text-align: left;
}

.markdown-preview :deep(pre) {
  overflow: auto;
  margin: 0.75rem 0;
  border-radius: 0.375rem;
  padding: 0.75rem;
  font-size: 0.75rem;
}

.markdown-preview :deep(code:not(pre code)) {
  border-radius: 0.2rem;
  background: #27272a;
  padding: 0.1rem 0.25rem;
  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  font-size: 0.85em;
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
  background: var(--surface-document);
  padding-right: 0.75rem;
  color: #52525b;
  text-align: right;
  white-space: pre;
}

.source-code {
  margin: 0;
  padding-left: 0.75rem;
}
</style>
