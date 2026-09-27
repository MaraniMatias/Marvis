<script setup lang="ts">
/* eslint-disable vue/html-self-closing */
import { Check as CheckIcon, ChevronDown as ChevronDownIcon, Copy as CopyIcon } from "@lucide/vue";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { PopoverContent, PopoverRoot, PopoverTrigger } from "reka-ui";
import { computed, nextTick, onUnmounted, reactive, ref, watch } from "vue";
import type { DocumentMode } from "../domain/main-document";
import type { Checkout } from "../domain/workspace";
import type { ActiveGitSnapshot } from "../presentation/active-git-snapshot";
import { useMarkdownPreview } from "../presentation/markdown-preview";
import { isIpcError } from "../domain/ipc";
import { readCheckoutFile, writeCheckoutFile } from "../lib/ipc";
import type { SourceLanguageOption } from "../lib/source-languages";
import {
  PLAIN_TEXT,
  SELECTABLE_LANGUAGES,
  detectedLanguageName,
  languageExtensions,
  languageLabel,
} from "../lib/source-languages";
import { useToasts } from "../presentation/toasts";
import type { EditorView } from "@codemirror/view";

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
const originalContent = ref("");
const drafts = reactive(new Map<string, string>());
const draftExpectedContent = reactive(new Map<string, string>());
const contentState = ref<"idle" | "loading" | "ready" | "error">("idle");
const contentError = ref("");
const contentIdentity = ref<string | null>(null);
const fileViewport = ref<HTMLElement | null>(null);
const editorHost = ref<HTMLElement | null>(null);
const editorLoading = ref(false);
const saving = ref(false);
let editorView: EditorView | null = null;
let editorIdentity: string | null = null;
let editorLanguage: string | null = null;
let editorGeneration = 0;
let syncingEditor = false;
let restoringEditorPosition = false;
const { push: pushToast, pushCause: reportCause } = useToasts();
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

/**
 * What a file is read as, where the reader overrode what its extension said.
 * The choice belongs to a file and not to the panel: the next file starts from its own extension,
 * and the map dies with the window rather than travelling to the database.
 */
const languageOverrides = reactive(new Map<string, string>());
const languageKey = computed(() => (props.path === null ? null : `${props.checkout?.id ?? ""}\0${props.path}`));
const languageOverride = computed(() => {
  const key = languageKey.value;
  return key === null ? null : (languageOverrides.get(key) ?? null);
});
const languageOpen = ref(false);
const languageSearch = ref("");
const languageQuery = computed(() => languageSearch.value.trim().toLowerCase());
const detectedLanguage = computed(() => (props.path === null ? null : (detectedLanguageName(props.path) ?? null)));
const effectiveLanguage = computed(() => languageOverride.value ?? detectedLanguage.value);
const isDirty = computed(() => (identity.value === null ? false : drafts.has(identity.value)));
const currentDraft = computed(() => (identity.value === null ? undefined : drafts.get(identity.value)));

// A Markdown preview renders its own fences, so in View mode there is no grammar left to choose.
const showsSource = computed(() => !(props.mode === "view" && isMarkdown.value));

// The button says what is happening now: in Auto that is the detected grammar, and a file whose
// extension names none is worth saying out loud rather than showing a language that is not applied.
const languageButtonLabel = computed(() => {
  if (languageOverride.value !== null) {
    return languageOverride.value === PLAIN_TEXT ? "Texto plano" : languageLabel(languageOverride.value);
  }
  return detectedLanguage.value === null ? "Auto (sin resaltado)" : languageLabel(detectedLanguage.value);
});

interface LanguageRow {
  /** Null is Auto: the extension decides, and a file it says nothing about stays plain. */
  name: string | null;
  label: string;
  hint: string;
}

/** A search reads the written name, the grammar's own name, or the suffix a file wears. */
const matchingLanguages = computed<readonly SourceLanguageOption[]>(() => {
  const query = languageQuery.value;
  if (query === "") return SELECTABLE_LANGUAGES;
  return SELECTABLE_LANGUAGES.filter(
    (language) =>
      language.label.toLowerCase().includes(query) ||
      language.name.includes(query) ||
      languageExtensions(language.name).some((extension) => extension.includes(query)),
  );
});

const languageRows = computed<LanguageRow[]>(() => [
  // Auto and plain text are the two ways back out of a forced grammar, so the search never takes
  // them away: it filters the grammars, not the ways out.
  {
    name: null,
    label: "Auto",
    hint: detectedLanguage.value === null ? "sin resaltado" : languageLabel(detectedLanguage.value),
  },
  { name: PLAIN_TEXT, label: "Texto plano", hint: "" },
  ...matchingLanguages.value.map((language) => ({ name: language.name, label: language.label, hint: "" })),
]);

const languageIndex = ref(0);
const activeLanguageRow = computed(() => Math.min(languageIndex.value, Math.max(languageRows.value.length - 1, 0)));

watch(languageQuery, () => {
  languageIndex.value = 0;
});

function moveLanguageRow(step: number) {
  const total = languageRows.value.length;
  if (total > 0) languageIndex.value = (languageIndex.value + step + total) % total;
}

function chooseLanguage(row: LanguageRow) {
  const key = languageKey.value;
  if (key === null) return;
  if (row.name === null) languageOverrides.delete(key);
  else languageOverrides.set(key, row.name);
  languageOpen.value = false;
}

function chooseActiveLanguage() {
  const row = languageRows.value[activeLanguageRow.value];
  if (row) chooseLanguage(row);
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
  const language = effectiveLanguage.value;
  const request = ++highlightGeneration;
  highlightedLines.value = null;
  highlightedSource.value = null;
  highlighting.value = language !== null;
  if (language === null) return;

  void import("../lib/source-highlighter")
    .then(({ highlightSourceAs }) => highlightSourceAs(language, source))
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
      case "file_changed":
        return "The file changed on disk. Your draft was kept; reload it before saving.";
      default:
        return error.message;
    }
  }
  return error instanceof Error ? error.message : String(error);
}

function absoluteFilePath() {
  const checkoutPath = props.checkout?.canonicalPath;
  const path = props.path;
  if (!checkoutPath || path === null) return null;
  return `${checkoutPath.replace(/[\\/]+$/, "")}/${path}`;
}

async function copyFilePath() {
  const path = absoluteFilePath();
  if (path === null) return;
  try {
    await writeText(path);
    pushToast("File path copied.", "info");
  } catch (error) {
    reportCause(error);
  }
}

function disposeEditor() {
  editorGeneration += 1;
  editorView?.destroy();
  editorView = null;
  editorIdentity = null;
  editorLanguage = null;
  editorLoading.value = false;
}

function updateDraft(nextContent: string) {
  const fileIdentity = identity.value;
  if (syncingEditor || fileIdentity === null) return;
  content.value = nextContent;
  if (!drafts.has(fileIdentity)) draftExpectedContent.set(fileIdentity, originalContent.value);
  if (nextContent === originalContent.value) {
    drafts.delete(fileIdentity);
    draftExpectedContent.delete(fileIdentity);
  } else {
    drafts.set(fileIdentity, nextContent);
  }
}

function syncEditorContent(nextContent: string) {
  const view = editorView;
  if (!view || view.state.doc.toString() === nextContent) return;
  syncingEditor = true;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: nextContent } });
  syncingEditor = false;
}

function restoreSourcePosition(position: { top: number; left: number }) {
  restoringEditorPosition = true;
  readingPosition.value = position;
  if (editorView) {
    editorView.scrollDOM.scrollTop = position.top;
    editorView.scrollDOM.scrollLeft = position.left;
  }
  if (fileViewport.value) {
    fileViewport.value.scrollTop = position.top;
    fileViewport.value.scrollLeft = position.left;
  }
  restoringEditorPosition = false;
}

async function ensureEditor(fileIdentity: string) {
  await nextTick();
  if (identity.value !== fileIdentity || props.mode !== "code" || contentState.value !== "ready") return;
  const host = editorHost.value;
  if (!host) return;
  if (editorView && editorIdentity === fileIdentity && editorLanguage === effectiveLanguage.value) {
    syncEditorContent(content.value);
    return;
  }
  disposeEditor();
  const generation = editorGeneration;
  editorLoading.value = true;
  let editorInitializing = true;
  try {
    const { createCodeEditor } = await import("../lib/code-editor");
    await nextTick();
    if (
      generation !== editorGeneration ||
      identity.value !== fileIdentity ||
      props.mode !== "code" ||
      contentState.value !== "ready" ||
      editorHost.value !== host
    )
      return;
    editorView = createCodeEditor({
      parent: host,
      content: content.value,
      language: effectiveLanguage.value,
      readingPosition: readingPosition.value,
      onChange: updateDraft,
      onScroll: (position) => {
        if (restoringEditorPosition || editorInitializing) return;
        readingPosition.value = position;
        emit("readingPositionChanged", position);
      },
    });
    editorIdentity = fileIdentity;
    editorLanguage = effectiveLanguage.value;
    await nextTick();
    window.setTimeout(() => {
      if (generation === editorGeneration) editorInitializing = false;
    }, 0);
  } catch (error) {
    if (generation === editorGeneration && identity.value === fileIdentity) reportCause(error);
  } finally {
    if (generation === editorGeneration) editorLoading.value = false;
  }
}

async function saveDraft() {
  const checkoutId = props.checkout?.id;
  const path = props.path;
  const fileIdentity = identity.value;
  const draft = currentDraft.value;
  if (saving.value || !checkoutId || path === null || fileIdentity === null || draft === undefined) return;
  const expectedContent = draftExpectedContent.get(fileIdentity) ?? originalContent.value;
  saving.value = true;
  try {
    await writeCheckoutFile(checkoutId, path, draft, expectedContent);
    if (identity.value === fileIdentity && props.checkout?.id === checkoutId && props.path === path) {
      originalContent.value = draft;
      // The editor remains live while the write is in flight. Do not discard a newer edit that
      // arrived after the request started; it is now based on the content we just persisted.
      if (drafts.get(fileIdentity) === draft) {
        content.value = draft;
        drafts.delete(fileIdentity);
        draftExpectedContent.delete(fileIdentity);
        syncEditorContent(draft);
      } else {
        draftExpectedContent.set(fileIdentity, draft);
      }
      pushToast("File saved.", "info");
    }
  } catch (error) {
    reportCause(error);
  } finally {
    saving.value = false;
  }
}

function cancelDraft() {
  const fileIdentity = identity.value;
  if (fileIdentity === null || !drafts.has(fileIdentity)) return;
  drafts.delete(fileIdentity);
  draftExpectedContent.delete(fileIdentity);
  content.value = originalContent.value;
  syncEditorContent(content.value);
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
    const draft = drafts.get(fileIdentity);
    if (draft !== undefined) {
      requestGeneration += 1;
      content.value = draft;
      contentIdentity.value = fileIdentity;
      contentState.value = "ready";
      contentError.value = unavailableText();
      loadedIdentity = null;
      return;
    }
    disposeEditor();
    requestGeneration += 1;
    invalidateHighlight();
    content.value = "";
    originalContent.value = "";
    contentIdentity.value = fileIdentity;
    contentState.value = "error";
    contentError.value = unavailableText();
    return;
  }
  const request = ++requestGeneration;
  const previousPosition = preservePosition ? readingPosition.value : props.readingPosition;
  // Whatever is on screen already belongs to this same document, so a re-read leaves it where it
  // is: the checkout watcher re-reads the open file twice for every change anywhere in the
  // workdir, and dropping back to "loading" — or to a blank page where the reason it cannot be
  // read was — until the read lands is what blinks.
  const refreshing = contentIdentity.value === fileIdentity;
  if (!refreshing) {
    contentState.value = "loading";
    contentError.value = "";
  }
  try {
    const result = await readCheckoutFile(checkoutId, path);
    if (request !== requestGeneration || props.checkout?.id !== checkoutId || identity.value !== fileIdentity) return;
    const draft = drafts.get(fileIdentity);
    // The bytes are usually the ones already on screen, because whatever changed was somewhere
    // else in the checkout. Then there is nothing to repaint, re-highlight or re-render, and the
    // scroll position is still where the reader put it. A draft is different: its baseline must
    // still be updated for Cancel, while the draft itself stays on screen.
    if (refreshing && contentState.value === "ready" && draft === undefined && result.content === content.value) {
      originalContent.value = result.content;
      return;
    }
    if (draft !== undefined && draft === result.content) {
      drafts.delete(fileIdentity);
      draftExpectedContent.delete(fileIdentity);
    }
    originalContent.value = result.content;
    content.value = drafts.get(fileIdentity) ?? result.content;
    contentIdentity.value = fileIdentity;
    contentState.value = "ready";
    loadedIdentity = fileIdentity;
    if (props.mode === "view" && isMarkdown.value) {
      invalidateHighlight();
    } else if (props.mode === "code") {
      // Keep the existing sanitized source render available as a fallback while the editor chunk
      // loads. CodeMirror owns the visible Code view once it is ready.
      startHighlight(fileIdentity, content.value);
    } else {
      startHighlight(fileIdentity, content.value);
    }
    await nextTick();
    if (request !== requestGeneration || props.checkout?.id !== checkoutId || identity.value !== fileIdentity) return;
    if (fileViewport.value) {
      fileViewport.value.scrollTop = previousPosition.top;
      fileViewport.value.scrollLeft = previousPosition.left;
    }
    restoreSourcePosition(previousPosition);
    if (props.mode === "code") {
      void ensureEditor(fileIdentity).then(async () => {
        await nextTick();
        if (identity.value === fileIdentity) restoreSourcePosition(previousPosition);
      });
    }
    if (props.mode === "view" && isMarkdown.value) {
      await load(checkoutId, path, result.content, refreshing);
      await restoreMarkdownReadingPosition(previousPosition, request, fileIdentity, checkoutId);
    }
  } catch (error) {
    if (request !== requestGeneration || props.checkout?.id !== checkoutId || identity.value !== fileIdentity) return;
    const draft = drafts.get(fileIdentity);
    if (draft !== undefined) {
      content.value = draft;
      contentIdentity.value = fileIdentity;
      contentState.value = "ready";
      contentError.value = errorText(error);
      loadedIdentity = fileIdentity;
      reportCause(error);
      return;
    }
    content.value = "";
    originalContent.value = "";
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
    } else if (mode === "code") {
      if (contentState.value === "ready") startHighlight(fileIdentity, content.value);
      clear();
      if (contentState.value === "ready") void ensureEditor(fileIdentity);
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

// Choosing a grammar is the one thing about a reading that changes without the file or the mode
// changing, so the source is read again. The Markdown preview owns its own fences and ignores this.
watch(effectiveLanguage, () => {
  if (contentState.value !== "ready" || identity.value === null) return;
  if (props.mode === "code") {
    startHighlight(identity.value, content.value);
    void ensureEditor(identity.value);
  } else if (showsSource.value) startHighlight(identity.value, content.value);
});

watch(
  () => [identity.value, props.mode] as const,
  ([fileIdentity, mode]) => {
    if (fileIdentity === null || mode !== "code") {
      disposeEditor();
      return;
    }
    if (contentState.value === "ready") void ensureEditor(fileIdentity);
  },
);

onUnmounted(disposeEditor);

function onFileScroll(event: Event) {
  const viewport = event.currentTarget as HTMLElement;
  readingPosition.value = { top: viewport.scrollTop, left: viewport.scrollLeft };
  emit("readingPositionChanged", readingPosition.value);
}

watch(
  content,
  async () => {
    if (props.mode !== "code" || contentState.value !== "ready") return;
    await nextTick();
    if (fileViewport.value) {
      fileViewport.value.scrollTop = readingPosition.value.top;
      fileViewport.value.scrollLeft = readingPosition.value.left;
    }
  },
  { flush: "post" },
);

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
  <main class="document-pane relative flex min-h-0 flex-1 flex-col">
    <header class="document-toolbar flex h-10 shrink-0 items-center justify-between gap-3 border-b px-3">
      <div class="flex min-w-0 items-center gap-1.5">
        <span class="min-w-0 truncate text-[11px] text-(--marvis-text-dim)" :title="path ?? undefined">{{
          path ?? ""
        }}</span>
        <button
          v-if="path !== null"
          type="button"
          aria-label="Copy file path"
          title="Copy absolute file path"
          class="toolbar-icon-button shrink-0"
          @click="copyFilePath"
        >
          <CopyIcon class="icon-xs" aria-hidden="true" />
        </button>
      </div>
      <!-- Both controls sit on the same row, at the same height, on the side the empty path text
           pushes them to. The mode group is Markdown's alone, so the language one is its sibling
           rather than another segment inside it. -->
      <div class="flex shrink-0 items-center gap-1.5">
        <!-- The pill is a plain div and not the PopoverRoot: reka's PopperRoot renders only its
             slot with inheritAttrs off, so a label and a surface set on it are dropped. -->
        <div
          v-if="showsSource"
          role="group"
          aria-label="Highlight language"
          class="document-mode-control flex shrink-0 items-center"
        >
          <PopoverRoot v-model:open="languageOpen">
            <PopoverTrigger
              data-testid="language-trigger"
              :title="`Resaltar como ${languageButtonLabel}. Solo afecta la vista Code.`"
              class="document-mode-button flex items-center gap-1 rounded-sm px-2 py-1 text-[11px]"
            >
              {{ languageButtonLabel }}
              <ChevronDownIcon class="icon-xs shrink-0 text-(--marvis-text-faint)" aria-hidden="true" />
            </PopoverTrigger>
            <PopoverContent
              side="bottom"
              align="end"
              :side-offset="4"
              class="surface-popover flex w-56 flex-col gap-1 rounded p-1 text-[11px] text-(--marvis-text)"
            >
              <input
                v-model="languageSearch"
                type="search"
                aria-label="Search highlight languages"
                placeholder="Buscar…"
                class="window-search min-w-0 appearance-none bg-transparent p-0 text-(--marvis-text-secondary) placeholder:text-(--marvis-text-faint)"
                @keydown.down.prevent="moveLanguageRow(1)"
                @keydown.up.prevent="moveLanguageRow(-1)"
                @keydown.enter.prevent="chooseActiveLanguage"
              />
              <div role="listbox" aria-label="Grammar" class="flex max-h-60 flex-col overflow-auto">
                <button
                  v-for="(row, index) in languageRows"
                  :key="row.name ?? 'auto'"
                  type="button"
                  role="option"
                  :aria-selected="row.name === languageOverride"
                  :title="row.label"
                  class="flex items-center gap-1.5 rounded-sm px-2 py-1 text-left hover:bg-(--marvis-border)"
                  :class="index === activeLanguageRow ? 'bg-(--marvis-border)' : ''"
                  @click="chooseLanguage(row)"
                >
                  <span class="min-w-0 flex-1 truncate">{{ row.label }}</span>
                  <span v-if="row.hint" class="shrink-0 text-(--marvis-text-faint)">{{ row.hint }}</span>
                  <CheckIcon v-if="row.name === languageOverride" class="icon-xs shrink-0" aria-hidden="true" />
                </button>
                <p v-if="!matchingLanguages.length" class="px-2 py-1 text-(--marvis-text-faint)">
                  No language matches "{{ languageSearch }}".
                </p>
              </div>
            </PopoverContent>
          </PopoverRoot>
        </div>
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
      <p
        v-else-if="contentState === 'ready' && content.length === 0 && mode !== 'code'"
        role="status"
        class="pane-state text-sm"
      >
        This file is empty.
      </p>
      <p v-else-if="highlighting && mode !== 'code'" role="status" class="pane-state text-sm">Highlighting source…</p>
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
        <div v-else-if="mode === 'code'" ref="editorHost" class="code-editor-host" aria-label="Source code">
          <p v-if="editorLoading" role="status" class="pane-state text-sm">Loading editor…</p>
          <!-- Keep the existing Shiki output available as a read-only fallback while CM6 loads and
               for the same source rendering used by the preview's language cache. -->
          <pre v-if="compactSource" class="source-line-number source-compatibility">{{ sourceLineNumbers }}</pre>
          <template v-else>
            <span v-for="index in sourceLines.length" :key="index" class="source-line-number source-compatibility">{{
              index
            }}</span>
          </template>
          <!-- eslint-disable vue/no-v-html -- Shiki output is DOMPurify-sanitized. -->
          <code
            v-if="highlightedLines !== null"
            class="shiki source-compatibility"
            v-html="highlightedLines.join('\n')"
          />
          <!-- eslint-enable vue/no-v-html -->
          <code v-else class="source-compatibility">{{ content }}</code>
        </div>
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
      <div
        v-if="isDirty && mode === 'code'"
        class="absolute bottom-4 right-4 z-10 flex items-center gap-2 rounded border border-(--marvis-border) bg-(--marvis-bg-1) p-1.5 shadow-lg"
        aria-label="Unsaved changes"
      >
        <button
          type="button"
          aria-label="Cancel"
          class="document-action-button"
          :disabled="saving"
          @click="cancelDraft"
        >
          Cancel
        </button>
        <button
          type="button"
          aria-label="Save"
          class="document-action-button document-action-primary"
          :disabled="saving"
          @click="saveDraft"
        >
          {{ saving ? "Saving…" : "Save" }}
        </button>
      </div>
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

/* The metadata a document opens with, in GitHub's shape: a table with the keys across the top and
   their values under them. One row of keys over one row of values is unreadable in a panel this
   narrow, so the table is a two-column grid with its rows dissolved into it, which lands the keys
   and their values on the same line, one pair at a time. */
.markdown-preview :deep(table.markdown-frontmatter) {
  display: grid;
  grid-template-columns: minmax(4rem, 9rem) 1fr;
  width: 100%;
  max-width: 100%;
  margin: 0 0 1.5rem;
  border: 1px solid var(--marvis-border);
  border-radius: var(--marvis-radius);
}

.markdown-preview :deep(table.markdown-frontmatter > thead),
.markdown-preview :deep(table.markdown-frontmatter > tbody),
.markdown-preview :deep(table.markdown-frontmatter > thead > tr),
.markdown-preview :deep(table.markdown-frontmatter > tbody > tr) {
  display: contents;
}

.markdown-preview :deep(table.markdown-frontmatter th),
.markdown-preview :deep(table.markdown-frontmatter td) {
  border: none;
  border-bottom: 1px solid var(--marvis-border);
  padding: 0.35rem 0.6rem;
  font-weight: 400;
  vertical-align: top;
}

.markdown-preview :deep(table.markdown-frontmatter th) {
  color: var(--marvis-text-secondary);
  font-size: 0.85em;
}

/* The last pair sits on the box's own edge, so it carries no rule under it. */
.markdown-preview :deep(table.markdown-frontmatter > thead > tr > th:last-child),
.markdown-preview :deep(table.markdown-frontmatter > tbody > tr > td:last-child) {
  border-bottom: none;
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

.source-compatibility {
  display: none;
}

.code-editor-host {
  min-height: 100%;
  height: 100%;
}

.code-editor-host :deep(.cm-editor) {
  min-height: 100%;
  height: 100%;
  background: var(--marvis-bg-0);
  color: var(--marvis-text);
  font-family: var(--marvis-font);
  font-size: 13px;
}

.code-editor-host :deep(.cm-scroller) {
  font-family: var(--marvis-font);
  line-height: 1.55;
}

.code-editor-host :deep(.cm-gutters) {
  background: var(--marvis-bg-0);
  border-right: 1px solid var(--marvis-border);
  color: var(--marvis-text-faint);
}

.code-editor-host :deep(.cm-activeLineGutter),
.code-editor-host :deep(.cm-activeLine) {
  background: color-mix(in srgb, var(--marvis-border) 35%, transparent);
}

.toolbar-icon-button,
.document-action-button {
  border-radius: var(--marvis-radius);
  color: var(--marvis-text-secondary);
}

.toolbar-icon-button {
  display: inline-flex;
  padding: 0.2rem;
}

.toolbar-icon-button:hover,
.document-action-button:hover:not(:disabled) {
  background: var(--marvis-border);
  color: var(--marvis-text);
}

.document-action-button {
  padding: 0.3rem 0.65rem;
  font-size: 0.75rem;
}

.document-action-primary {
  background: var(--marvis-accent);
  color: var(--marvis-bg-0);
}

.document-action-button:disabled {
  cursor: not-allowed;
  opacity: 0.55;
}
</style>
